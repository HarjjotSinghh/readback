//! Acoustic suspicion: where the recogniser itself was shaky.
//!
//! This stage never decides anything on its own. It answers "how much should we
//! doubt this?", which the policy stage multiplies against how much a wrong word
//! would cost.

use crate::align::{EditKind, align, token_span_or_point};
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
}

impl Default for SuspicionConfig {
    fn default() -> Self {
        Self {
            low_confidence: 0.55,
            max_no_speech: 0.6,
            max_compression_ratio: 2.4,
            min_avg_logprob: -1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SuspicionOutcome {
    /// `0.0..=1.0`. Zero when the engine reported no confidence data at all.
    pub score: f32,
    pub flags: Vec<Flag>,
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
pub fn assess(transcript: &Transcript, text: &str, cfg: &SuspicionConfig) -> SuspicionOutcome {
    let mut flags = Vec::new();
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
            let risk = (1.0 - confidence).clamp(0.0, 1.0);
            score = score.max(risk);
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

    SuspicionOutcome {
        score: score.clamp(0.0, 1.0),
        flags,
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
            assess(&t, "merge this", &SuspicionConfig::default()).score,
            0.0
        );
    }

    #[test]
    fn flags_a_shaky_word_and_points_at_it() {
        let t = transcript_with(vec![
            Word::new("never").with_confidence(0.42),
            Word::new("merge").with_confidence(0.97),
        ]);
        let out = assess(&t, "never merge", &SuspicionConfig::default());
        assert_eq!(out.flags.len(), 1);
        assert_eq!(out.flags[0].span.slice("never merge"), "never");
        assert!((out.score - 0.58).abs() < 1e-5);
    }

    #[test]
    fn confident_words_are_left_alone() {
        let t = transcript_with(vec![Word::new("ship").with_confidence(0.99)]);
        assert!(
            assess(&t, "ship", &SuspicionConfig::default())
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
        let out = assess(&t, "thank you for watching", &SuspicionConfig::default());
        assert_eq!(out.flags[0].kind, FlagKind::HallucinationSignal);
    }

    #[test]
    fn engine_disagreement_raises_suspicion() {
        let mut t = Transcript::from_text("merge this change");
        t.alternatives = vec![Hypothesis::from_text("don't merge this change")];
        assert!(assess(&t, "merge this change", &SuspicionConfig::default()).score > 0.0);
    }
}
