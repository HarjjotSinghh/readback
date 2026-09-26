//! Stakes scoring: how much a single wrong word would cost here.
//!
//! Readback ships a deterministic rules scorer and nothing else. The trait
//! exists so a local decision model (Laya via ONNX) or a hosted one (Jev) can
//! be dropped in without the core depending on either.

pub mod rules;

use crate::lexicon::Lexicon;
use crate::tokenize::Token;

pub use rules::RulesScorer;

/// Answers one typed question: does this utterance contain an instruction,
/// negation, number or commitment where one wrong word flips the meaning?
pub trait StakesScorer: Send + Sync {
    /// Short identifier recorded in [`crate::types::Provenance`].
    fn name(&self) -> &str;

    /// Returns `0.0..=1.0`. Implementations must be deterministic for a given
    /// input or the policy stage becomes impossible to debug.
    fn score(&self, text: &str, tokens: &[Token], lexicon: &Lexicon) -> f32;
}
