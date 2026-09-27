//! Selective re-decoding: asking again about the part that looked wrong.
//!
//! Every other stage works on what the recogniser already said. This one goes
//! back to the audio. When a span looks shaky, the host is asked to decode just
//! that slice again — with a bigger model, a wider beam, or phrase biasing —
//! and the answers are compared.
//!
//! It exists for the failure nothing else catches: a confidently wrong word
//! that reads perfectly. `do merge that branch` is fluent, unremarkable, and
//! the exact opposite of `don't merge that branch`. No lexicon, confidence
//! score or language model can see that from the text. A second decode of the
//! 400 ms where `don't` was spoken can.
//!
//! Readback still never touches audio. The host owns the clip and the decoder;
//! this module decides *which* slices are worth the cost and what a
//! disagreement means.

use crate::lexicon::{Lexicon, SemanticClass};
use crate::tokenize::{Token, tokenize};
use crate::types::{Flag, FlagKind, Span};
use serde::{Deserialize, Serialize};

/// A slice of the clip, in milliseconds from its start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSpan {
    pub start_ms: u32,
    pub end_ms: u32,
}

impl AudioSpan {
    pub fn new(start_ms: u32, end_ms: u32) -> Self {
        Self { start_ms, end_ms }
    }

    pub fn duration_ms(&self) -> u32 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    /// Widens the slice, since a word's reported boundaries clip its edges and
    /// a decoder needs context either side to do better than the first pass.
    pub fn padded(&self, padding_ms: u32) -> Self {
        Self {
            start_ms: self.start_ms.saturating_sub(padding_ms),
            end_ms: self.end_ms.saturating_add(padding_ms),
        }
    }
}

/// What the host is being asked to decode again.
#[derive(Debug, Clone, PartialEq)]
pub struct RedecodeRequest {
    pub span: AudioSpan,
    /// What the first pass produced for this slice, for reference. A host may
    /// pass it to a decoder as a prompt, or ignore it.
    pub original: String,
}

/// A second opinion on a slice of audio.
///
/// Implementations are expected to be slow — a second model over a short slice
/// — which is why only flagged spans reach here. Returning an empty list means
/// "no opinion" and costs nothing.
pub trait Redecoder: Send + Sync {
    /// Candidate transcripts for the slice, best first.
    fn redecode(&self, request: &RedecodeRequest) -> Vec<String>;

    /// Recorded in provenance so a verdict can be traced.
    fn name(&self) -> &str {
        "redecoder"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RedecodeConfig {
    /// Widening applied to each slice before decoding.
    pub padding_ms: u32,
    /// Cap on slices per utterance. Re-decoding is the expensive path and must
    /// not turn a 250 ms interaction into a several-second one.
    pub max_spans: usize,
    /// Slices shorter than this are widened to it; a decoder given 80 ms of
    /// audio has nothing to work with.
    pub min_span_ms: u32,
    /// Share of the original's words a candidate must also contain before its
    /// disagreement is believed.
    pub min_overlap: f32,
    /// Stakes the *utterance* must reach before any of it is decoded again.
    ///
    /// This gates on the sentence, not on the span. Gating on span risk was
    /// tried and fails badly: measured on whisper.cpp tiny.en it cut the
    /// expensive path from 64% of clips to 34%, and catching collapsed from
    /// 55.6% to 11.1%. The catches were coming from spans the cheap stages
    /// rated *low* risk — when an engine drops a word outright, what remains
    /// often looks perfectly confident, which is exactly why a second decode is
    /// the only thing that can find it.
    ///
    /// Gating on sentence stakes instead fares better but still trades
    /// catching away roughly in proportion to what it saves:
    ///
    /// | `min_stakes` | clips re-decoded | caught |
    /// |---|---:|---:|
    /// | 0.0 (off) | 64% | 55.6% |
    /// | 0.3 | 48% | 33.3% |
    /// | 0.5 | 45% | 33.3% |
    ///
    /// **Defaults to off**, because there is no free lunch here and the
    /// measured behaviour is worth preserving. Raise it when latency matters
    /// more than catching, and know what it costs.
    pub min_stakes: f32,
}

impl Default for RedecodeConfig {
    fn default() -> Self {
        Self {
            padding_ms: 300,
            max_spans: 2,
            min_span_ms: 400,
            min_overlap: 0.5,
            min_stakes: 0.0,
        }
    }
}

/// What a second decode said about one slice.
#[derive(Debug, Clone, PartialEq)]
pub struct SpanVerdict {
    pub span: AudioSpan,
    pub original: String,
    pub candidate: String,
    /// The protected word the second decode found that the first pass missed.
    pub introduced: String,
    pub class: SemanticClass,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RedecodeOutcome {
    pub flags: Vec<Flag>,
    pub verdicts: Vec<SpanVerdict>,
    pub spans_decoded: usize,
}

fn protected_norms(tokens: &[Token], lexicon: &Lexicon) -> Vec<(String, SemanticClass)> {
    tokens
        .iter()
        .filter_map(|t| lexicon.classify(t).map(|c| (t.norm.clone(), c)))
        .collect()
}

/// Share of the original's content words that also appear in the candidate.
///
/// A second decode of a padded slice is working with less context than the
/// first pass had, and will happily return something unrelated. Overlap is how
/// we tell "the same words, plus one the first pass missed" from "a different
/// guess entirely".
fn overlap(original: &[Token], candidate: &[Token]) -> f32 {
    let content: Vec<&Token> = original.iter().filter(|t| !t.is_punct()).collect();
    if content.is_empty() {
        return 0.0;
    }
    let shared = content
        .iter()
        .filter(|t| candidate.iter().any(|c| c.norm == t.norm))
        .count();
    shared as f32 / content.len() as f32
}

/// Compares a second decode against the first pass for the same slice.
///
/// Two conditions, both necessary. The candidate must be decoding the *same
/// words* — measured by overlap — and it must contain a word that can flip an
/// instruction which the first pass did not have.
///
/// Only negations, direction verbs and environments qualify. A recovered date
/// or name from a context-starved slice is far more likely to be the decoder
/// guessing than a real recovery, and charging the user's attention for it is
/// what makes this stage unusable.
///
/// Only one direction counts as evidence: a word appearing in the candidate
/// that the original lacks. The reverse is far weaker, since a short slice
/// drops words routinely.
pub fn compare(
    original: &str,
    candidates: &[String],
    lexicon: &Lexicon,
    min_overlap: f32,
) -> Option<(String, String, SemanticClass)> {
    let original_tokens = tokenize(original);
    let had: Vec<String> = protected_norms(&original_tokens, lexicon)
        .into_iter()
        .map(|(norm, _)| norm)
        .collect();

    for candidate in candidates {
        let candidate_tokens = tokenize(candidate);
        if overlap(&original_tokens, &candidate_tokens) < min_overlap {
            continue;
        }

        let introduced = protected_norms(&candidate_tokens, lexicon)
            .into_iter()
            .find(|(norm, class)| !had.contains(norm) && flips_an_instruction(*class));

        if let Some((norm, class)) = introduced {
            return Some((candidate.clone(), norm, class));
        }
    }
    None
}

/// Classes where recovering a missed word changes what the sentence instructs.
fn flips_an_instruction(class: SemanticClass) -> bool {
    matches!(
        class,
        SemanticClass::Negation | SemanticClass::Direction | SemanticClass::Environment
    )
}

/// Severity for a disagreement about a word of this class.
fn severity_for(class: SemanticClass) -> crate::types::Severity {
    use crate::types::Severity;
    match class {
        SemanticClass::Negation | SemanticClass::Direction | SemanticClass::Environment => {
            Severity::Critical
        }
        _ => Severity::High,
    }
}

/// Re-decodes the given slices and reports any disagreement.
///
/// `anchor` is the byte offset in the final text that each slice corresponds
/// to, so a flag can point at something the host can highlight.
pub fn run(
    decoder: &dyn Redecoder,
    slices: &[(AudioSpan, String, Span)],
    lexicon: &Lexicon,
    cfg: &RedecodeConfig,
) -> RedecodeOutcome {
    let mut outcome = RedecodeOutcome::default();

    for (span, original, anchor) in slices.iter().take(cfg.max_spans) {
        // Spans arrive already padded by the caller, which is the only place
        // that knows which words the padding pulled in — and `original` has to
        // describe exactly that audio for the overlap check to mean anything.
        let mut widened = *span;
        if widened.duration_ms() < cfg.min_span_ms {
            let shortfall = cfg.min_span_ms - widened.duration_ms();
            widened = widened.padded(shortfall / 2 + 1);
        }

        let request = RedecodeRequest {
            span: widened,
            original: original.clone(),
        };
        let candidates = decoder.redecode(&request);
        outcome.spans_decoded += 1;
        if candidates.is_empty() {
            continue;
        }

        let Some((candidate, introduced, class)) =
            compare(original, &candidates, lexicon, cfg.min_overlap)
        else {
            continue;
        };

        outcome.flags.push(Flag {
            kind: FlagKind::RedecodeDisagreement,
            severity: severity_for(class),
            span: *anchor,
            evidence: format!(
                "a second decode of {:.2}s-{:.2}s heard \"{candidate}\", which contains \
                 \"{introduced}\"; the first pass did not",
                widened.start_ms as f32 / 1000.0,
                widened.end_ms as f32 / 1000.0,
            ),
            suggestion: Some(candidate.clone()),
        });
        outcome.verdicts.push(SpanVerdict {
            span: widened,
            original: original.clone(),
            candidate,
            introduced,
            class,
        });
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Canned(Vec<String>);

    impl Redecoder for Canned {
        fn redecode(&self, _request: &RedecodeRequest) -> Vec<String> {
            self.0.clone()
        }
        fn name(&self) -> &str {
            "canned"
        }
    }

    fn slices() -> Vec<(AudioSpan, String, Span)> {
        vec![(
            AudioSpan::new(1000, 1400),
            "do merge".to_string(),
            Span::new(0, 2),
        )]
    }

    #[test]
    fn a_recovered_negation_is_critical() {
        let decoder = Canned(vec!["don't merge".into()]);
        let out = run(
            &decoder,
            &slices(),
            &Lexicon::default(),
            &RedecodeConfig::default(),
        );

        assert_eq!(out.flags.len(), 1);
        assert_eq!(out.flags[0].kind, FlagKind::RedecodeDisagreement);
        assert_eq!(out.flags[0].severity, crate::types::Severity::Critical);
        assert_eq!(out.verdicts[0].introduced, "don't");
    }

    #[test]
    fn agreement_costs_nothing() {
        let decoder = Canned(vec!["do merge".into()]);
        let out = run(
            &decoder,
            &slices(),
            &Lexicon::default(),
            &RedecodeConfig::default(),
        );
        assert!(out.flags.is_empty());
        assert_eq!(out.spans_decoded, 1);
    }

    #[test]
    fn a_decoder_with_no_opinion_is_free() {
        let decoder = Canned(vec![]);
        let out = run(
            &decoder,
            &slices(),
            &Lexicon::default(),
            &RedecodeConfig::default(),
        );
        assert!(out.flags.is_empty());
    }

    #[test]
    fn an_unrelated_candidate_is_not_evidence() {
        // Same slice, completely different words: the decoder is guessing.
        let slices = vec![(
            AudioSpan::new(1000, 1400),
            "do merge".to_string(),
            Span::new(0, 2),
        )];
        let decoder = Canned(vec!["never mind the weather".into()]);
        let out = run(
            &decoder,
            &slices,
            &Lexicon::default(),
            &RedecodeConfig::default(),
        );
        assert!(out.flags.is_empty(), "overlap is too low to believe this");
    }

    #[test]
    fn a_recovered_name_is_not_worth_the_users_attention() {
        let mut lexicon = Lexicon::default();
        lexicon.protect(["Bobby"]);
        let slices = vec![(
            AudioSpan::new(1000, 1400),
            "ask about it".to_string(),
            Span::new(0, 3),
        )];
        let decoder = Canned(vec!["ask Bobby about it".into()]);
        let out = run(&decoder, &slices, &lexicon, &RedecodeConfig::default());
        assert!(
            out.flags.is_empty(),
            "a context-starved slice inventing a name is not a recovery"
        );
    }

    #[test]
    fn only_newly_introduced_words_count() {
        // The candidate drops a word the first pass had. A short slice decoded
        // out of context does that routinely, so it is not evidence.
        let slices = vec![(
            AudioSpan::new(1000, 1400),
            "don't merge".to_string(),
            Span::new(0, 5),
        )];
        let decoder = Canned(vec!["merge".into()]);
        let out = run(
            &decoder,
            &slices,
            &Lexicon::default(),
            &RedecodeConfig::default(),
        );
        assert!(out.flags.is_empty());
    }

    #[test]
    fn the_gate_is_off_by_default() {
        assert_eq!(RedecodeConfig::default().min_stakes, 0.0);
    }

    #[test]
    fn the_expensive_path_is_capped() {
        let many: Vec<_> = (0..10)
            .map(|i| {
                (
                    AudioSpan::new(i * 500, i * 500 + 400),
                    "do".to_string(),
                    Span::new(0, 2),
                )
            })
            .collect();
        let decoder = Canned(vec!["don't".into()]);
        let out = run(
            &decoder,
            &many,
            &Lexicon::default(),
            &RedecodeConfig::default(),
        );
        assert_eq!(out.spans_decoded, 2, "max_spans must bound the cost");
    }

    #[test]
    fn short_slices_are_widened_enough_to_decode() {
        let cfg = RedecodeConfig::default();
        let span = AudioSpan::new(1000, 1080).padded(cfg.padding_ms);
        assert!(
            span.duration_ms() >= 600,
            "padding alone should already help"
        );
    }

    #[test]
    fn padding_does_not_underflow_at_the_start_of_a_clip() {
        assert_eq!(AudioSpan::new(50, 300).padded(300).start_ms, 0);
    }
}
