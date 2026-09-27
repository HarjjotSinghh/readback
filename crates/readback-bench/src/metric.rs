//! The Critical Semantic Error Rate.
//!
//! Word error rate treats dropping "um" and dropping "not" as the same mistake,
//! which is why a stack can post a good WER and still ruin someone's afternoon.
//! CSER weights each error by how much the word it touched carries meaning, so
//! a dropped negation costs fifty times a dropped filler.
//!
//! The weights below are a deliberate, documented choice rather than a derived
//! constant. They are meant to be argued with; that is the point of publishing
//! them.

use readback_core::align::{EditKind, align};
use readback_core::lexicon::{Lexicon, SemanticClass};
use readback_core::number;
use readback_core::tokenize::{Token, tokenize};
use serde::{Deserialize, Serialize};

/// Any error at or above this weight changed what the sentence instructs.
pub const MEANING_FLIP_WEIGHT: f32 = 4.0;

const FILLERS: &[&str] = &[
    "um",
    "uh",
    "er",
    "ah",
    "hmm",
    "like",
    "basically",
    "actually",
    "literally",
    "yeah",
    "okay",
    "so",
    "well",
    "right",
    "you",
    "know",
    "i",
    "mean",
    "kinda",
    "sorta",
    "just",
];

const ARTICLES: &[&str] = &["a", "an", "the"];

/// What kind of word an error touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    Punctuation,
    Filler,
    Article,
    Ordinary,
    ProperNoun,
    Number,
    Temporal,
    Modality,
    Quantifier,
    Direction,
    Environment,
    Negation,
}

impl ErrorClass {
    /// Cost of getting a word of this class wrong, relative to an ordinary word.
    pub fn weight(self) -> f32 {
        match self {
            ErrorClass::Punctuation => 0.1,
            ErrorClass::Filler => 0.2,
            ErrorClass::Article => 0.3,
            ErrorClass::Ordinary => 1.0,
            ErrorClass::ProperNoun => 2.0,
            ErrorClass::Number | ErrorClass::Temporal => 3.0,
            ErrorClass::Modality | ErrorClass::Quantifier => 3.0,
            ErrorClass::Direction | ErrorClass::Environment => 4.0,
            ErrorClass::Negation => 5.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ErrorClass::Punctuation => "punctuation",
            ErrorClass::Filler => "filler",
            ErrorClass::Article => "article",
            ErrorClass::Ordinary => "ordinary",
            ErrorClass::ProperNoun => "proper-noun",
            ErrorClass::Number => "number",
            ErrorClass::Temporal => "temporal",
            ErrorClass::Modality => "modality",
            ErrorClass::Quantifier => "quantifier",
            ErrorClass::Direction => "direction",
            ErrorClass::Environment => "environment",
            ErrorClass::Negation => "negation",
        }
    }

    /// True when an error of this class changes what the sentence instructs.
    pub fn flips_meaning(self) -> bool {
        self.weight() >= MEANING_FLIP_WEIGHT
    }
}

pub fn classify(token: &Token, lexicon: &Lexicon) -> ErrorClass {
    if token.is_punct() {
        return ErrorClass::Punctuation;
    }
    if let Some(class) = lexicon.classify(token) {
        return match class {
            SemanticClass::Negation => ErrorClass::Negation,
            SemanticClass::Direction => ErrorClass::Direction,
            SemanticClass::Environment => ErrorClass::Environment,
            SemanticClass::Number => ErrorClass::Number,
            SemanticClass::Temporal => ErrorClass::Temporal,
            SemanticClass::Quantifier => ErrorClass::Quantifier,
            SemanticClass::Modality => ErrorClass::Modality,
            SemanticClass::ProtectedTerm => ErrorClass::ProperNoun,
        };
    }
    if FILLERS.contains(&token.norm.as_str()) {
        return ErrorClass::Filler;
    }
    if ARTICLES.contains(&token.norm.as_str()) {
        return ErrorClass::Article;
    }
    // A capitalised word the lexicon does not know is probably a name.
    if token.text.chars().next().is_some_and(char::is_uppercase) {
        return ErrorClass::ProperNoun;
    }
    ErrorClass::Ordinary
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    Substitution,
    Deletion,
    Insertion,
}

/// One difference between what was said and what was written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticError {
    pub kind: ErrorKind,
    pub class: ErrorClass,
    pub weight: f32,
    pub reference: String,
    pub hypothesis: String,
}

impl SemanticError {
    pub fn flips_meaning(&self) -> bool {
        self.class.flips_meaning()
    }
}

/// Scores for one utterance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scores {
    /// Classic word error rate over content tokens, for comparison.
    pub wer: f32,
    /// Critical Semantic Error Rate: error weight over reference weight.
    pub cser: f32,
    /// True when at least one error was heavy enough to change the instruction.
    pub meaning_flip: bool,
    pub errors: Vec<SemanticError>,
}

fn content(tokens: &[Token]) -> Vec<&Token> {
    tokens.iter().filter(|t| !t.is_punct()).collect()
}

fn text_of(tokens: &[&Token]) -> String {
    tokens
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Compares what was said against what the stack produced.
///
/// Punctuation is excluded: it is not what this benchmark is about, and
/// including it would let a stack game the number by dropping commas.
pub fn score(reference: &str, hypothesis: &str, lexicon: &Lexicon) -> Scores {
    let ref_tokens = tokenize(reference);
    let hyp_tokens = tokenize(hypothesis);
    let reference_content: Vec<Token> = content(&ref_tokens).into_iter().cloned().collect();
    let hypothesis_content: Vec<Token> = content(&hyp_tokens).into_iter().cloned().collect();

    let mut errors = Vec::new();
    let mut edit_count = 0usize;

    for edit in align(&reference_content, &hypothesis_content) {
        match edit.kind {
            EditKind::Equal => continue,
            EditKind::Delete => {
                edit_count += edit.left.len();
                for token in &reference_content[edit.left.clone()] {
                    let class = classify(token, lexicon);
                    errors.push(SemanticError {
                        kind: ErrorKind::Deletion,
                        class,
                        weight: class.weight(),
                        reference: token.text.clone(),
                        hypothesis: String::new(),
                    });
                }
            }
            EditKind::Insert => {
                edit_count += edit.right.len();
                for token in &hypothesis_content[edit.right.clone()] {
                    let class = classify(token, lexicon);
                    errors.push(SemanticError {
                        kind: ErrorKind::Insertion,
                        class,
                        weight: class.weight(),
                        reference: String::new(),
                        hypothesis: token.text.clone(),
                    });
                }
            }
            EditKind::Replace => {
                edit_count += edit.left.len().max(edit.right.len());
                let left: Vec<&Token> = reference_content[edit.left.clone()].iter().collect();
                let right: Vec<&Token> = hypothesis_content[edit.right.clone()].iter().collect();

                // Writing "three" as "3" is a word error but not a semantic
                // one. WER still counts it above, which is exactly the contrast
                // this metric exists to draw.
                let renumbered = number::same_phrase(&left, &right)
                    || (left.len() == right.len()
                        && left
                            .iter()
                            .zip(right.iter())
                            .all(|(a, b)| number::same_value(a, b)));
                if renumbered {
                    continue;
                }
                // A substitution costs whichever side carries more meaning: a
                // negation replaced by an ordinary word is still a negation error.
                let class = left
                    .iter()
                    .chain(right.iter())
                    .map(|t| classify(t, lexicon))
                    .max_by(|a, b| a.weight().total_cmp(&b.weight()))
                    .unwrap_or(ErrorClass::Ordinary);
                errors.push(SemanticError {
                    kind: ErrorKind::Substitution,
                    class,
                    weight: class.weight(),
                    reference: text_of(&left),
                    hypothesis: text_of(&right),
                });
            }
        }
    }

    let reference_weight: f32 = reference_content
        .iter()
        .map(|t| classify(t, lexicon).weight())
        .sum::<f32>()
        .max(f32::EPSILON);
    let error_weight: f32 = errors.iter().map(|e| e.weight).sum();
    let denominator = reference_content.len().max(1) as f32;

    Scores {
        wer: edit_count as f32 / denominator,
        cser: error_weight / reference_weight,
        meaning_flip: errors.iter().any(SemanticError::flips_meaning),
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex() -> Lexicon {
        Lexicon::default()
    }

    #[test]
    fn identical_text_scores_zero() {
        let s = score("merge this change", "merge this change", &lex());
        assert_eq!(s.wer, 0.0);
        assert_eq!(s.cser, 0.0);
        assert!(!s.meaning_flip);
    }

    #[test]
    fn a_dropped_negation_flips_meaning() {
        let s = score(
            "never merge a change like this",
            "merge a change like this",
            &lex(),
        );
        assert!(s.meaning_flip);
        assert_eq!(s.errors[0].class, ErrorClass::Negation);
    }

    #[test]
    fn a_dropped_filler_does_not() {
        let s = score(
            "um merge a change like this",
            "merge a change like this",
            &lex(),
        );
        assert!(!s.meaning_flip);
        assert!(s.cser < 0.05, "cser was {}", s.cser);
    }

    #[test]
    fn wer_cannot_tell_them_apart_but_cser_can() {
        let filler = score("um merge this change", "merge this change", &lex());
        let negation = score("not merge this change", "merge this change", &lex());
        assert_eq!(filler.wer, negation.wer, "both are one deleted word");
        assert!(
            negation.cser > filler.cser * 10.0,
            "cser must separate them"
        );
    }

    #[test]
    fn an_environment_swap_flips_meaning() {
        let s = score(
            "deploy this to staging",
            "deploy this to production",
            &lex(),
        );
        assert!(s.meaning_flip);
        assert_eq!(s.errors[0].class, ErrorClass::Environment);
    }

    #[test]
    fn a_number_swap_is_heavy_but_not_a_flip() {
        let s = score("set retries to fifteen", "set retries to fifty", &lex());
        assert!(!s.meaning_flip);
        assert_eq!(s.errors[0].class, ErrorClass::Number);
        assert!(s.cser > 0.2);
    }

    #[test]
    fn renumbering_costs_nothing_semantically() {
        let s = score(
            "the meeting got moved to three",
            "The meeting got moved to 3.",
            &lex(),
        );
        assert_eq!(s.cser, 0.0, "the value did not change");
        assert!(
            s.wer > 0.0,
            "WER still counts it, which is the point of the contrast"
        );
        assert!(!s.meaning_flip);
    }

    #[test]
    fn renumbering_does_not_hide_a_real_change() {
        let s = score("set retries to fifteen", "set retries to 50", &lex());
        assert!(s.cser > 0.0);
        assert_eq!(s.errors[0].class, ErrorClass::Number);
    }

    #[test]
    fn dropping_a_currency_symbol_still_counts() {
        let s = score("refund $50 today", "refund 50 today", &lex());
        assert!(s.cser > 0.0, "$50 and 50 are not the same amount");
    }

    #[test]
    fn punctuation_is_not_scored() {
        let s = score("ship it", "ship it.", &lex());
        assert_eq!(s.wer, 0.0);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn an_unknown_capitalised_word_counts_as_a_name() {
        let s = score("ask Harpawan about it", "ask Harpreet about it", &lex());
        assert_eq!(s.errors[0].class, ErrorClass::ProperNoun);
    }
}
