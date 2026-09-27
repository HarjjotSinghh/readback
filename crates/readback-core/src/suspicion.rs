//! Acoustic suspicion: where the recogniser itself was shaky.
//!
//! This stage never decides anything on its own. It answers "how much should we
//! doubt this?", which the policy stage multiplies against how much a wrong word
//! would cost.

use crate::align::{EditKind, align, token_span_or_point};
use crate::lexicon::Lexicon;
use crate::tokenize::{Token, tokenize};
use crate::types::{Flag, FlagKind, Severity, Span, Transcript, Word};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Thresholds for the decoder-level hallucination tells.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SuspicionConfig {
    /// Word confidence at or below this is worth flagging.
    pub low_confidence: f32,
    /// `no_speech_prob` above this means the decoder was writing over silence.
    pub max_no_speech: f32,
    /// Whisper's compression ratio above this means repetitive output.
    pub max_compression_ratio: f32,
    /// Mean token log-probability below this means a struggling decode.
    pub min_avg_logprob: f32,
    /// Weighted word risk below this is not reported as a flag.
    ///
    /// The score still rises, so policy can act on it; what this suppresses is
    /// telling the user to look at a word that does not matter.
    pub min_word_risk: f32,
}

impl Default for SuspicionConfig {
    fn default() -> Self {
        Self {
            low_confidence: 0.55,
            max_no_speech: 0.6,
            max_compression_ratio: 2.4,
            min_avg_logprob: -1.0,
            min_word_risk: 0.15,
        }
    }
}

/// A word worth asking about again, with the byte span it occupies in the
/// output text so a later flag can point at it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    /// Index into the transcript's word list.
    pub word: usize,
    /// Weighted doubt, highest first when sorted.
    pub risk: f32,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SuspicionOutcome {
    /// `0.0..=1.0`. Zero when the engine reported no confidence data at all.
    pub score: f32,
    pub flags: Vec<Flag>,
    /// Words the recogniser was unsure about that carry enough meaning to be
    /// worth a second decode, worst first.
    pub candidates: Vec<Candidate>,
}

/// Maps each input word onto a byte span in `text`.
///
/// The text handed to this stage may be the polished rewrite, so words are
/// aligned rather than assumed to line up positionally.
fn word_spans(words: &[Word], text: &str) -> Vec<Span> {
    let out_tokens = tokenize(text);
    let word_tokens: Vec<Token> = words
        .iter()
        .map(|w| {
            let mut t = tokenize(&w.text);
            t.pop().unwrap_or(Token {
                text: w.text.clone(),
                norm: crate::tokenize::normalize(&w.text),
                start: 0,
                end: w.text.len(),
                kind: crate::tokenize::TokenKind::Word,
            })
        })
        .collect();

    let mut spans = vec![Span::new(0, 0); words.len()];
    for edit in align(&word_tokens, &out_tokens) {
        match edit.kind {
            EditKind::Equal => {
                for (offset, i) in edit.left.clone().enumerate() {
                    let j = edit.right.start + offset;
                    spans[i] = token_span_or_point(&out_tokens, &(j..j + 1));
                }
            }
            _ => {
                // The word was dropped or rewritten downstream: anchor the flag
                // at whatever replaced it, or at the seam if nothing did.
                let span = token_span_or_point(&out_tokens, &edit.right);
                for i in edit.left.clone() {
                    spans[i] = span;
                }
            }
        }
    }
    spans
}

/// True when any trigram of normalised tokens repeats three times or more,
/// which is how Whisper-family loops usually present.
fn has_repetition_loop(tokens: &[Token]) -> bool {
    if tokens.len() < 9 {
        return false;
    }
    let mut seen: HashMap<[&str; 3], usize> = HashMap::new();
    for window in tokens.windows(3) {
        let key = [
            window[0].norm.as_str(),
            window[1].norm.as_str(),
            window[2].norm.as_str(),
        ];
        let count = seen.entry(key).or_insert(0);
        *count += 1;
        if *count >= 3 {
            return true;
        }
    }
    false
}

/// The lexical weight of a recognised word, tokenising it first so that
/// punctuation attached by the engine does not change the answer.
fn word_weight(text: &str, lexicon: &Lexicon) -> f32 {
    tokenize(text)
        .iter()
        .filter(|t| !t.is_punct())
        .map(|t| lexicon.lexical_weight(t))
        .fold(0.0_f32, f32::max)
}

fn signal_flag(evidence: String) -> Flag {
    Flag {
        kind: FlagKind::HallucinationSignal,
        severity: FlagKind::HallucinationSignal.severity(),
        span: Span::new(0, 0),
        evidence,
        suggestion: None,
    }
}

/// Scores how much to doubt this transcript, and flags the specific words.
///
/// A word's contribution is scaled by what that word carries. The recogniser
/// being unsure about `the` is not evidence that the sentence changed meaning,
/// even when the sentence is about production; being unsure about `never` is.
pub fn assess(
    transcript: &Transcript,
    text: &str,
    cfg: &SuspicionConfig,
    lexicon: &Lexicon,
) -> SuspicionOutcome {
    let mut flags = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut score: f32 = 0.0;

    let words = &transcript.primary.words;
    let scored: Vec<(usize, f32)> = words
        .iter()
        .enumerate()
        .filter_map(|(i, w)| w.confidence.map(|c| (i, c)))
        .collect();

    if !scored.is_empty() {
        let spans = word_spans(words, text);
        for (i, confidence) in scored {
            if confidence > cfg.low_confidence {
                continue;
            }
            let weight = word_weight(&words[i].text, lexicon);
            let risk = ((1.0 - confidence) * weight).clamp(0.0, 1.0);
            score = score.max(risk);
            candidates.push(Candidate {
                word: i,
                risk,
                span: spans[i],
            });

            // Below this the word is not worth the user's attention even though
            // the recogniser was unsure: reporting it is what causes flag
            // fatigue, and a flag nobody reads protects nobody.
            if risk < cfg.min_word_risk {
                continue;
            }
            flags.push(Flag {
                kind: FlagKind::LowConfidence,
                severity: FlagKind::LowConfidence.severity(),
                span: spans[i],
                evidence: format!(
                    "\"{}\" was recognised with {:.0}% confidence",
                    words[i].text,
                    confidence * 100.0
                ),
                suggestion: None,
            });
        }
    }

    let s = &transcript.signals;
    if let Some(no_speech) = s.no_speech_prob
        && no_speech > cfg.max_no_speech
    {
        score = score.max(no_speech);
        flags.push(signal_flag(format!(
            "decoder reported {:.0}% probability that this was silence",
            no_speech * 100.0
        )));
    }
    if let Some(ratio) = s.compression_ratio
        && ratio > cfg.max_compression_ratio
    {
        score = score.max(0.6);
        flags.push(signal_flag(format!(
            "output compresses {ratio:.1}x, which usually means the decoder repeated itself"
        )));
    }
    if let Some(logprob) = s.avg_logprob
        && logprob < cfg.min_avg_logprob
    {
        score = score.max(0.6);
        flags.push(signal_flag(format!(
            "mean token log-probability was {logprob:.2}, below the {:.2} floor",
            cfg.min_avg_logprob
        )));
    }
    if has_repetition_loop(&tokenize(text)) {
        score = score.max(0.7);
        flags.push(signal_flag(
            "the same three-word sequence repeats several times".into(),
        ));
    }

    // Disagreement between engines is itself evidence, even before re-decoding.
    if let Some(alt) = transcript.alternatives.first() {
        let primary_tokens = tokenize(&transcript.primary.text);
        let alt_tokens = tokenize(&alt.text);
        let edits = align(&primary_tokens, &alt_tokens);
        let changed: usize = edits
            .iter()
            .filter(|e| !e.is_equal())
            .map(|e| e.left.len().max(e.right.len()))
            .sum();
        let total = primary_tokens.len().max(1);
        let disagreement = (changed as f32 / total as f32).clamp(0.0, 1.0);
        if disagreement > 0.0 {
            score = score.max(disagreement * 0.8);
        }
    }

    candidates.sort_by(|a, b| b.risk.total_cmp(&a.risk));
    SuspicionOutcome {
        score: score.clamp(0.0, 1.0),
        flags,
        candidates,
    }
}

/// Convenience for callers that only want the number.
pub fn severity_ceiling(flags: &[Flag]) -> Severity {
    flags
        .iter()
        .map(|f| f.severity)
        .max()
        .unwrap_or(Severity::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DecodeSignals, Hypothesis};

    fn transcript_with(words: Vec<Word>) -> Transcript {
        Transcript::from_words(words)
    }

    #[test]
    fn no_confidence_data_means_no_suspicion() {
        let t = Transcript::from_text("merge this");
        assert_eq!(
            assess(
                &t,
                "merge this",
                &SuspicionConfig::default(),
                &Lexicon::default()
            )
            .score,
            0.0
        );
    }

    #[test]
    fn flags_a_shaky_word_and_points_at_it() {
        let t = transcript_with(vec![
            Word::new("never").with_confidence(0.42),
            Word::new("merge").with_confidence(0.97),
        ]);
        let out = assess(
            &t,
            "never merge",
            &SuspicionConfig::default(),
            &Lexicon::default(),
        );
        assert_eq!(out.flags.len(), 1);
        assert_eq!(out.flags[0].span.slice("never merge"), "never");
        // "never" is a negation, so it carries full weight: 0.58 * 1.0.
        assert!((out.score - 0.58).abs() < 1e-5);
    }

    #[test]
    fn a_shaky_function_word_is_not_worth_reporting() {
        // The same confidence on "the" as on "never" above.
        let t = transcript_with(vec![
            Word::new("the").with_confidence(0.42),
            Word::new("build").with_confidence(0.97),
        ]);
        let out = assess(
            &t,
            "the build",
            &SuspicionConfig::default(),
            &Lexicon::default(),
        );
        assert!(
            out.flags.is_empty(),
            "flagging this is what causes flag fatigue"
        );
        assert!(out.score < 0.1, "score was {}", out.score);
    }

    #[test]
    fn a_shaky_environment_still_counts() {
        let t = transcript_with(vec![
            Word::new("deploy").with_confidence(0.98),
            Word::new("production").with_confidence(0.42),
        ]);
        let out = assess(
            &t,
            "deploy production",
            &SuspicionConfig::default(),
            &Lexicon::default(),
        );
        assert_eq!(out.flags.len(), 1);
        assert!(out.score > 0.45, "score was {}", out.score);
    }

    #[test]
    fn candidates_are_ranked_by_weighted_doubt() {
        let t = transcript_with(vec![
            Word::new("the").with_confidence(0.40),
            Word::new("production").with_confidence(0.45),
        ]);
        let out = assess(
            &t,
            "the production",
            &SuspicionConfig::default(),
            &Lexicon::default(),
        );
        assert_eq!(out.candidates.len(), 2);
        assert_eq!(out.candidates[0].word, 1, "the protected word comes first");
        assert!(out.candidates[0].risk > out.candidates[1].risk);
    }

    #[test]
    fn confident_words_are_left_alone() {
        let t = transcript_with(vec![Word::new("ship").with_confidence(0.99)]);
        assert!(
            assess(&t, "ship", &SuspicionConfig::default(), &Lexicon::default())
                .flags
                .is_empty()
        );
    }

    #[test]
    fn silence_raises_a_hallucination_signal() {
        let mut t = Transcript::from_text("thank you for watching");
        t.signals = DecodeSignals {
            no_speech_prob: Some(0.92),
            ..Default::default()
        };
        let out = assess(
            &t,
            "thank you for watching",
            &SuspicionConfig::default(),
            &Lexicon::default(),
        );
        assert_eq!(out.flags[0].kind, FlagKind::HallucinationSignal);
    }

    #[test]
    fn engine_disagreement_raises_suspicion() {
        let mut t = Transcript::from_text("merge this change");
        t.alternatives = vec![Hypothesis::from_text("don't merge this change")];
        assert!(
            assess(
                &t,
                "merge this change",
                &SuspicionConfig::default(),
                &Lexicon::default()
            )
            .score
                > 0.0
        );
    }
}
