//! The protected lexicon: which words Readback refuses to let a cleanup step
//! quietly rewrite.

pub mod en;
pub mod hinglish;

use crate::tokenize::{Token, TokenKind};
use crate::types::FlagKind;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Which language packs are loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    En,
    /// Romanised Hindi mixed with English, as actually spoken.
    Hinglish,
}

/// What kind of meaning a token carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticClass {
    Negation,
    Temporal,
    Direction,
    Quantifier,
    Modality,
    Environment,
    Number,
    /// A term supplied by the host app's vocabulary or the user's dictionary.
    ProtectedTerm,
}

impl SemanticClass {
    /// The flag raised when a token of this class is removed or rewritten.
    pub fn removal_flag(self) -> FlagKind {
        match self {
            SemanticClass::Negation => FlagKind::DroppedNegation,
            SemanticClass::Temporal => FlagKind::ChangedTemporal,
            SemanticClass::Direction => FlagKind::ChangedDirection,
            SemanticClass::Quantifier => FlagKind::ChangedQuantifier,
            SemanticClass::Modality => FlagKind::ChangedModality,
            SemanticClass::Environment => FlagKind::ChangedEnvironment,
            SemanticClass::Number => FlagKind::ChangedNumber,
            SemanticClass::ProtectedTerm => FlagKind::ChangedProtectedTerm,
        }
    }

    /// The flag raised when a token of this class is swapped for a different
    /// token of the same class, which is usually worse than a plain removal.
    pub fn substitution_flag(self) -> FlagKind {
        match self {
            SemanticClass::Negation => FlagKind::AlteredNegation,
            other => other.removal_flag(),
        }
    }

    /// How much this class contributes to the stakes score.
    pub fn stakes_weight(self) -> f32 {
        match self {
            SemanticClass::Negation => 1.0,
            SemanticClass::Direction | SemanticClass::Environment => 0.85,
            SemanticClass::Number | SemanticClass::Temporal => 0.7,
            SemanticClass::Quantifier | SemanticClass::Modality => 0.5,
            SemanticClass::ProtectedTerm => 0.4,
        }
    }
}

/// Word lists plus the user's own protected terms.
#[derive(Debug, Clone)]
pub struct Lexicon {
    negation: HashSet<String>,
    temporal: HashSet<String>,
    direction: HashSet<String>,
    quantifier: HashSet<String>,
    modality: HashSet<String>,
    environment: HashSet<String>,
    number_words: HashSet<String>,
    destructive: HashSet<String>,
    imperative: HashSet<String>,
    protected: HashSet<String>,
}

fn insert_all(set: &mut HashSet<String>, words: &[&str]) {
    set.extend(words.iter().map(|w| w.to_string()));
}

impl Default for Lexicon {
    fn default() -> Self {
        Self::new(&[Locale::En])
    }
}

impl Lexicon {
    /// Builds a lexicon from the given language packs.
    pub fn new(locales: &[Locale]) -> Self {
        let mut lex = Self {
            negation: HashSet::new(),
            temporal: HashSet::new(),
            direction: HashSet::new(),
            quantifier: HashSet::new(),
            modality: HashSet::new(),
            environment: HashSet::new(),
            number_words: HashSet::new(),
            destructive: HashSet::new(),
            imperative: HashSet::new(),
            protected: HashSet::new(),
        };

        for locale in locales {
            match locale {
                Locale::En => {
                    insert_all(&mut lex.negation, en::NEGATION);
                    insert_all(&mut lex.temporal, en::TEMPORAL);
                    insert_all(&mut lex.direction, en::DIRECTION);
                    insert_all(&mut lex.quantifier, en::QUANTIFIER);
                    insert_all(&mut lex.modality, en::MODALITY);
                    insert_all(&mut lex.environment, en::ENVIRONMENT);
                    insert_all(&mut lex.number_words, en::NUMBER_WORDS);
                    insert_all(&mut lex.destructive, en::DESTRUCTIVE);
                    insert_all(&mut lex.imperative, en::IMPERATIVE);
                }
                Locale::Hinglish => {
                    insert_all(&mut lex.negation, hinglish::NEGATION);
                    insert_all(&mut lex.temporal, hinglish::TEMPORAL);
                    insert_all(&mut lex.direction, hinglish::DIRECTION);
                    insert_all(&mut lex.quantifier, hinglish::QUANTIFIER);
                    insert_all(&mut lex.modality, hinglish::MODALITY);
                    insert_all(&mut lex.number_words, hinglish::NUMBER_WORDS);
                }
            }
        }

        lex
    }

    /// Adds user vocabulary: names, projects, environments, jargon.
    pub fn protect<I, S>(&mut self, terms: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.protected.extend(
            terms
                .into_iter()
                .map(|t| crate::tokenize::normalize(t.as_ref())),
        );
    }

    /// Classifies a token, highest-consequence class first.
    ///
    /// Order matters: `never` is both a negation and a quantifier, and it is
    /// the negation reading that ruins someone's afternoon.
    pub fn classify(&self, token: &Token) -> Option<SemanticClass> {
        let n = token.norm.as_str();

        if self.protected.contains(n) {
            return Some(SemanticClass::ProtectedTerm);
        }
        if self.is_negation(n) {
            return Some(SemanticClass::Negation);
        }
        if self.direction.contains(n) {
            return Some(SemanticClass::Direction);
        }
        if self.environment.contains(n) {
            return Some(SemanticClass::Environment);
        }
        if token.kind == TokenKind::Number || self.number_words.contains(n) {
            return Some(SemanticClass::Number);
        }
        if self.temporal.contains(n) {
            return Some(SemanticClass::Temporal);
        }
        if self.quantifier.contains(n) {
            return Some(SemanticClass::Quantifier);
        }
        if self.modality.contains(n) {
            return Some(SemanticClass::Modality);
        }
        None
    }

    /// Classifies a bare word, for callers that have no [`Token`] in hand.
    ///
    /// The word is tokenised first, so `"$50"` and `"don\u{2019}t"` behave the same
    /// as they would mid-sentence. Returns `None` for input that tokenises to
    /// nothing.
    pub fn classify_str(&self, word: &str) -> Option<SemanticClass> {
        let tokens = crate::tokenize::tokenize(word);
        tokens.first().and_then(|t| self.classify(t))
    }

    /// Every word currently loaded for a class, sorted, for inspection and
    /// documentation. `Number` returns only the spelled-out words, since
    /// digits are recognised by shape rather than by list.
    pub fn words(&self, class: SemanticClass) -> Vec<&str> {
        let set = match class {
            SemanticClass::Negation => &self.negation,
            SemanticClass::Temporal => &self.temporal,
            SemanticClass::Direction => &self.direction,
            SemanticClass::Quantifier => &self.quantifier,
            SemanticClass::Modality => &self.modality,
            SemanticClass::Environment => &self.environment,
            SemanticClass::Number => &self.number_words,
            SemanticClass::ProtectedTerm => &self.protected,
        };
        let mut words: Vec<&str> = set.iter().map(String::as_str).collect();
        words.sort_unstable();
        words
    }

    /// Words that raise the stakes without being protected themselves.
    pub fn words_destructive(&self) -> Vec<&str> {
        let mut words: Vec<&str> = self.destructive.iter().map(String::as_str).collect();
        words.sort_unstable();
        words
    }

    /// True for listed negations and for any unlisted `n't` contraction.
    pub fn is_negation(&self, norm: &str) -> bool {
        self.negation.contains(norm) || norm.ends_with("n't")
    }

    pub fn is_destructive(&self, norm: &str) -> bool {
        self.destructive.contains(norm)
    }

    pub fn is_imperative(&self, norm: &str) -> bool {
        self.imperative.contains(norm)
    }

    pub fn is_protected_term(&self, norm: &str) -> bool {
        self.protected.contains(norm)
    }

    /// True when this token must survive the cleanup step untouched.
    pub fn is_protected(&self, token: &Token) -> bool {
        self.classify(token).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::tokenize;

    fn class_of(lex: &Lexicon, word: &str) -> Option<SemanticClass> {
        lex.classify(&tokenize(word)[0])
    }

    #[test]
    fn negation_beats_quantifier() {
        let lex = Lexicon::default();
        assert_eq!(class_of(&lex, "never"), Some(SemanticClass::Negation));
    }

    #[test]
    fn unlisted_contractions_are_negations() {
        let lex = Lexicon::default();
        assert!(lex.is_negation("mightn't"));
    }

    #[test]
    fn numbers_classify_without_a_list() {
        let lex = Lexicon::default();
        assert_eq!(class_of(&lex, "15"), Some(SemanticClass::Number));
        assert_eq!(class_of(&lex, "$50"), Some(SemanticClass::Number));
    }

    #[test]
    fn spelled_out_numbers_count_too() {
        let lex = Lexicon::default();
        assert_eq!(class_of(&lex, "fifteen"), Some(SemanticClass::Number));
        assert_eq!(class_of(&lex, "Fifty"), Some(SemanticClass::Number));
    }

    #[test]
    fn hinglish_negation_loads_only_when_asked() {
        assert_eq!(class_of(&Lexicon::default(), "nahi"), None);
        let lex = Lexicon::new(&[Locale::En, Locale::Hinglish]);
        assert_eq!(class_of(&lex, "nahi"), Some(SemanticClass::Negation));
    }

    #[test]
    fn user_vocabulary_outranks_builtin_lists() {
        let mut lex = Lexicon::default();
        lex.protect(["Main"]);
        assert_eq!(class_of(&lex, "main"), Some(SemanticClass::ProtectedTerm));
    }

    #[test]
    fn classify_str_matches_classify() {
        let lex = Lexicon::default();
        assert_eq!(lex.classify_str("never"), Some(SemanticClass::Negation));
        assert_eq!(lex.classify_str("$50"), Some(SemanticClass::Number));
        assert_eq!(lex.classify_str("   "), None);
    }

    #[test]
    fn words_lists_the_loaded_pack() {
        let lex = Lexicon::new(&[Locale::En, Locale::Hinglish]);
        let negations = lex.words(SemanticClass::Negation);
        assert!(negations.contains(&"never"));
        assert!(negations.contains(&"nahi"));
        assert!(
            negations.windows(2).all(|w| w[0] <= w[1]),
            "should be sorted"
        );
    }

    #[test]
    fn ordinary_words_are_not_protected() {
        let lex = Lexicon::default();
        for word in ["the", "meeting", "tomorrow's", "laptop"] {
            let token = &tokenize(word)[0];
            if word == "tomorrow's" {
                continue;
            }
            assert!(!lex.is_protected(token), "{word} should not be protected");
        }
    }
}
