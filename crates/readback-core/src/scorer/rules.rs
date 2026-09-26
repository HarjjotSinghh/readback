//! The default scorer: lexicon lookups plus imperative detection. No model, no
//! dependencies, microseconds.

use super::StakesScorer;
use crate::lexicon::Lexicon;
use crate::tokenize::Token;

/// Weights for the deterministic scorer. Tune these rather than forking it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RulesWeights {
    /// Applied to every utterance, so nothing is ever exactly zero risk.
    pub base: f32,
    /// Multiplier on the highest-weighted semantic class present.
    pub semantic: f32,
    /// Added when the utterance reads as an instruction.
    pub imperative: f32,
    /// Added when the instruction is destructive.
    pub destructive: f32,
}

impl Default for RulesWeights {
    fn default() -> Self {
        Self {
            base: 0.05,
            semantic: 0.7,
            imperative: 0.15,
            destructive: 0.2,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RulesScorer {
    weights: RulesWeights,
}

impl RulesScorer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_weights(weights: RulesWeights) -> Self {
        Self { weights }
    }
}

impl StakesScorer for RulesScorer {
    fn name(&self) -> &str {
        "rules"
    }

    fn score(&self, _text: &str, tokens: &[Token], lexicon: &Lexicon) -> f32 {
        let w = self.weights;
        let mut score = w.base;

        let heaviest = tokens
            .iter()
            .filter_map(|t| lexicon.classify(t))
            .map(|c| c.stakes_weight())
            .fold(0.0_f32, f32::max);
        score += heaviest * w.semantic;

        if tokens.iter().any(|t| lexicon.is_imperative(&t.norm)) {
            score += w.imperative;
        }
        if tokens.iter().any(|t| lexicon.is_destructive(&t.norm)) {
            score += w.destructive;
        }

        score.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::tokenize;

    fn score(text: &str) -> f32 {
        RulesScorer::new().score(text, &tokenize(text), &Lexicon::default())
    }

    #[test]
    fn small_talk_scores_near_zero() {
        assert!(score("lol sounds good") < 0.2);
    }

    #[test]
    fn a_negated_instruction_scores_high() {
        assert!(score("never merge this before Friday") > 0.8);
    }

    #[test]
    fn a_destructive_instruction_maxes_out() {
        assert_eq!(score("don't delete the old staging tables"), 1.0);
    }

    #[test]
    fn a_plain_remark_outranks_small_talk_but_not_an_order() {
        let remark = score("the build finished around Friday");
        assert!(remark > score("lol sounds good"));
        assert!(remark < score("never merge this before Friday"));
    }
}
