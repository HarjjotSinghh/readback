//! Canonical numeric values, so that rewriting a number is told apart from
//! renumbering it.
//!
//! A cleanup step that turns "three" into "3" has changed nothing. One that
//! turns "fifteen" into "fifty" has changed a great deal. Both look like a
//! substitution to a string comparison, so numbers are reduced to a canonical
//! key and compared by value.
//!
//! The key keeps the *kind* of number as well as the value, because dropping a
//! currency symbol is itself a change: `$50` and `50` do not compare equal.

use crate::tokenize::Token;
use std::collections::HashMap;
use std::sync::OnceLock;

/// English number words with an unambiguous value.
const EN_WORDS: &[(&str, f64)] = &[
    ("zero", 0.0),
    ("one", 1.0),
    ("two", 2.0),
    ("three", 3.0),
    ("four", 4.0),
    ("five", 5.0),
    ("six", 6.0),
    ("seven", 7.0),
    ("eight", 8.0),
    ("nine", 9.0),
    ("ten", 10.0),
    ("eleven", 11.0),
    ("twelve", 12.0),
    ("thirteen", 13.0),
    ("fourteen", 14.0),
    ("fifteen", 15.0),
    ("sixteen", 16.0),
    ("seventeen", 17.0),
    ("eighteen", 18.0),
    ("nineteen", 19.0),
    ("twenty", 20.0),
    ("thirty", 30.0),
    ("forty", 40.0),
    ("fifty", 50.0),
    ("sixty", 60.0),
    ("seventy", 70.0),
    ("eighty", 80.0),
    ("ninety", 90.0),
    ("hundred", 100.0),
    ("thousand", 1_000.0),
    ("million", 1_000_000.0),
    ("billion", 1_000_000_000.0),
    ("dozen", 12.0),
];

/// Romanised Hindi numerals. Spellings that collide with common English words
/// are deliberately absent; see `lexicon::hinglish`.
const HINGLISH_WORDS: &[(&str, f64)] = &[
    ("ek", 1.0),
    ("paanch", 5.0),
    ("panch", 5.0),
    ("chhe", 6.0),
    ("che", 6.0),
    ("saat", 7.0),
    ("aath", 8.0),
    ("nau", 9.0),
    ("das", 10.0),
    ("gyarah", 11.0),
    ("barah", 12.0),
    ("pachas", 50.0),
    ("sau", 100.0),
    ("hazaar", 1_000.0),
    ("hazar", 1_000.0),
    ("lakh", 100_000.0),
    ("crore", 10_000_000.0),
    ("karod", 10_000_000.0),
];

fn word_values() -> &'static HashMap<&'static str, f64> {
    static VALUES: OnceLock<HashMap<&'static str, f64>> = OnceLock::new();
    VALUES.get_or_init(|| EN_WORDS.iter().chain(HINGLISH_WORDS).copied().collect())
}

/// What kind of number this is. Two numbers only compare equal when both the
/// kind and the value match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Currency,
    Percent,
    Ordinal,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Kind::Plain => "num",
            Kind::Currency => "cur",
            Kind::Percent => "pct",
            Kind::Ordinal => "ord",
        }
    }
}

/// Formats a value without trailing zeros, so `1.50` and `1.5` agree.
fn format_value(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let mut text = format!("{value:.6}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// Parses a token whose normalised form contains digits.
fn from_digits(norm: &str) -> Option<(Kind, f64)> {
    let kind = if norm.starts_with('$') || norm.ends_with("usd") || norm.starts_with('\u{20b9}') {
        Kind::Currency
    } else if norm.ends_with('%') {
        Kind::Percent
    } else if norm.ends_with("st")
        || norm.ends_with("nd")
        || norm.ends_with("rd")
        || norm.ends_with("th")
    {
        Kind::Ordinal
    } else {
        Kind::Plain
    };

    // Keep digits, one decimal point and a leading minus; drop separators.
    let mut digits = String::new();
    let mut seen_dot = false;
    for (i, c) in norm.char_indices() {
        match c {
            '-' if i == 0 => digits.push(c),
            '0'..='9' => digits.push(c),
            '.' if !seen_dot && !digits.is_empty() => {
                seen_dot = true;
                digits.push(c);
            }
            ',' | '_' | ' ' => {}
            _ => {}
        }
    }
    // A trailing decimal point came from a full stop, not from the number.
    let trimmed = digits.trim_end_matches('.');
    let value: f64 = trimmed.parse().ok()?;
    Some((kind, value))
}

/// A comparison key for a numeric token, or `None` when the token is not a
/// number this module recognises.
///
/// ```
/// use readback_core::number::canonical;
/// use readback_core::tokenize::tokenize;
///
/// let three = canonical(&tokenize("three")[0]);
/// let digit = canonical(&tokenize("3")[0]);
/// assert_eq!(three, digit);
/// assert_ne!(canonical(&tokenize("$50")[0]), canonical(&tokenize("50")[0]));
/// ```
pub fn canonical(token: &Token) -> Option<String> {
    let norm = token.norm.as_str();

    if norm.chars().any(|c| c.is_ascii_digit()) {
        let (kind, value) = from_digits(norm)?;
        return Some(format!("{}:{}", kind.prefix(), format_value(value)));
    }

    let value = *word_values().get(norm)?;
    Some(format!("{}:{}", Kind::Plain.prefix(), format_value(value)))
}

/// Units that a bare number can be written with instead of a symbol.
const UNITS: &[(&str, Kind)] = &[
    ("percent", Kind::Percent),
    ("pct", Kind::Percent),
    ("dollars", Kind::Currency),
    ("dollar", Kind::Currency),
    ("usd", Kind::Currency),
    ("rupees", Kind::Currency),
    ("rupee", Kind::Currency),
    ("inr", Kind::Currency),
];

/// A comparison key for a run of tokens, so `20 percent` and `20%` agree.
///
/// Handles a bare number followed by a spelled-out unit. Longer compounds such
/// as `twenty five` are not reduced; they fall back to token-by-token
/// comparison, which is conservative rather than wrong.
///
/// ```
/// use readback_core::number::canonical_phrase;
/// use readback_core::tokenize::tokenize;
///
/// let spelled = tokenize("20 percent");
/// let symbol = tokenize("20%");
/// assert_eq!(
///     canonical_phrase(&spelled.iter().collect::<Vec<_>>()),
///     canonical_phrase(&symbol.iter().collect::<Vec<_>>()),
/// );
/// ```
pub fn canonical_phrase(tokens: &[&Token]) -> Option<String> {
    match tokens {
        [single] => canonical(single),
        [value, unit] => {
            let (_, kind) = UNITS.iter().find(|(name, _)| *name == unit.norm)?;
            let key = canonical(value)?;
            let number = key.strip_prefix("num:")?;
            Some(format!("{}:{number}", kind.prefix()))
        }
        _ => None,
    }
}

/// True when two runs of tokens name the same quantity.
pub fn same_phrase(left: &[&Token], right: &[&Token]) -> bool {
    match (canonical_phrase(left), canonical_phrase(right)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// True when two tokens name the same number in the same form.
pub fn same_value(a: &Token, b: &Token) -> bool {
    match (canonical(a), canonical(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::tokenize;

    fn key(text: &str) -> Option<String> {
        canonical(&tokenize(text)[0])
    }

    fn phrase(text: &str) -> Option<String> {
        canonical_phrase(&tokenize(text).iter().collect::<Vec<_>>())
    }

    #[test]
    fn words_and_digits_agree() {
        assert_eq!(key("three"), key("3"));
        assert_eq!(key("fifteen"), key("15"));
        assert_eq!(key("Fifty"), key("50"));
    }

    #[test]
    fn different_numbers_stay_different() {
        assert_ne!(key("fifteen"), key("fifty"));
        assert_ne!(key("15"), key("50"));
    }

    #[test]
    fn currency_is_part_of_the_key() {
        assert_ne!(key("$50"), key("50"));
        assert_eq!(key("$50"), key("$50"));
    }

    #[test]
    fn percentages_are_their_own_kind() {
        assert_ne!(key("20%"), key("20"));
    }

    #[test]
    fn ordinals_are_their_own_kind() {
        assert_ne!(key("3rd"), key("3"));
        assert_eq!(key("3rd"), key("3rd"));
    }

    #[test]
    fn separators_and_trailing_zeros_are_ignored() {
        assert_eq!(key("1,000"), key("1000"));
        assert_eq!(key("1.50"), key("1.5"));
    }

    #[test]
    fn a_trailing_full_stop_is_not_a_decimal_point() {
        // "…to 15." tokenises the stop separately, but be safe either way.
        assert_eq!(key("15"), Some("num:15".to_string()));
    }

    #[test]
    fn hinglish_numerals_are_recognised() {
        assert_eq!(key("pachas"), key("50"));
        assert_eq!(key("ek"), key("1"));
    }

    #[test]
    fn english_words_that_are_also_hindi_numerals_are_not_numbers() {
        // "do merge this" must not read as "2 merge this".
        assert_eq!(key("do"), None);
        assert_eq!(key("teen"), None);
        assert_eq!(key("char"), None);
        assert_eq!(key("bees"), None);
    }

    #[test]
    fn a_spelled_unit_matches_its_symbol() {
        assert_eq!(phrase("20 percent"), phrase("20%"));
        assert_eq!(phrase("50 dollars"), phrase("$50"));
    }

    #[test]
    fn a_unit_still_distinguishes_the_amount() {
        assert_ne!(phrase("20 percent"), phrase("50%"));
        assert_ne!(phrase("50 dollars"), phrase("50"));
    }

    #[test]
    fn longer_compounds_are_left_alone() {
        assert_eq!(phrase("twenty five"), None);
    }

    #[test]
    fn ordinary_words_are_not_numbers() {
        assert_eq!(key("merge"), None);
        assert_eq!(key("quarter"), None, "ambiguous: a quarter past, or 0.25");
    }
}
