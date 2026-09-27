//! Word-level tokenisation with byte offsets.
//!
//! Every downstream stage works on tokens rather than raw strings, because a
//! flag has to point at an exact span of the final text for the host app to
//! highlight it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    /// Letters, possibly with internal apostrophes, hyphens, dots or underscores
    /// so that `don't`, `.env`, `MAX_RETRIES` and `state-of-the-art` survive.
    Word,
    /// Anything whose normalised form contains a digit: `15`, `$50`, `3.5`, `v0.1`.
    Number,
    /// Standalone punctuation and symbols.
    Punct,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    /// The token exactly as it appears in the source text.
    pub text: String,
    /// Lowercased, with typographic apostrophes folded to ASCII.
    pub norm: String,
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

impl Token {
    pub fn is_punct(&self) -> bool {
        self.kind == TokenKind::Punct
    }

    pub fn is_number(&self) -> bool {
        self.kind == TokenKind::Number
    }
}

/// Characters that may appear *inside* a word without splitting it.
fn is_inner(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '-' | '_' | '.' | '/' | '@' | '+')
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// A comma inside a number, as in `1,000`.
///
/// Only between two digits, so `hello, world` is still two tokens. The check
/// looks backwards at the text consumed so far and forwards at what follows.
fn is_digit_separator(c: char, text: &str, idx: usize, end: usize) -> bool {
    if c != ',' {
        return false;
    }
    let prev_is_digit = text[..end]
        .chars()
        .next_back()
        .is_some_and(|p| p.is_ascii_digit());
    let next_is_digit = text[idx + c.len_utf8()..]
        .chars()
        .next()
        .is_some_and(|n| n.is_ascii_digit());
    prev_is_digit && next_is_digit
}

/// Characters that may *start* a word when a word character follows, so that
/// `.env`, `$50`, `#channel` and `@handle` survive as single tokens.
fn is_leading(c: char) -> bool {
    matches!(c, '.' | '@' | '#' | '_' | '$')
}

/// Folds typographic apostrophes so `don’t` and `don't` compare equal.
pub fn normalize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '\u{2019}' || c == '\u{02BC}' {
                '\''
            } else {
                c
            }
        })
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Splits text into tokens, preserving byte offsets into the original string.
///
/// Inner punctuation is kept only when flanked by word characters, so a trailing
/// full stop ends up as its own [`TokenKind::Punct`] token rather than being
/// glued onto the last word.
pub fn tokenize(text: &str) -> Vec<Token> {
    let bytes_len = text.len();
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some(&(start, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }

        if !is_word_char(c) {
            let after = start + c.len_utf8();
            let next_is_word = text[after..].chars().next().is_some_and(is_word_char);
            if !(is_leading(c) && next_is_word) {
                let raw = &text[start..after];
                tokens.push(Token {
                    text: raw.to_string(),
                    norm: normalize(raw),
                    start,
                    end: after,
                    kind: TokenKind::Punct,
                });
                chars.next();
                continue;
            }
            // Leading sigil: consume it, then fall through to the word scan.
            chars.next();
        }

        // Walk forward while we are on word characters, or on inner punctuation
        // that is immediately followed by another word character.
        let mut end = start;
        while let Some(&(idx, ch)) = chars.peek() {
            if is_word_char(ch) {
                end = idx + ch.len_utf8();
                chars.next();
                continue;
            }
            if is_inner(ch) || is_digit_separator(ch, text, idx, end) {
                let after = idx + ch.len_utf8();
                let next_is_word = text[after..].chars().next().is_some_and(is_word_char);
                if next_is_word {
                    end = after;
                    chars.next();
                    continue;
                }
            }
            break;
        }

        // A percent sign belongs to the number in front of it: `20%` is not the
        // same quantity as `20`.
        if let Some(&(idx, ch)) = chars.peek()
            && ch == '%'
            && text[..end]
                .chars()
                .next_back()
                .is_some_and(|p| p.is_ascii_digit())
        {
            end = idx + ch.len_utf8();
            chars.next();
        }

        debug_assert!(end <= bytes_len);
        let raw = &text[start..end];
        let norm = normalize(raw);
        let kind = if norm.chars().any(|c| c.is_ascii_digit()) {
            TokenKind::Number
        } else {
            TokenKind::Word
        };
        tokens.push(Token {
            text: raw.to_string(),
            norm,
            start,
            end,
            kind,
        });
    }

    tokens
}

/// Tokens with punctuation removed, for comparisons that shouldn't care about it.
pub fn content_tokens(tokens: &[Token]) -> Vec<&Token> {
    tokens.iter().filter(|t| !t.is_punct()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_contractions_whole() {
        let t = tokenize("don't merge this");
        assert_eq!(
            t.iter().map(|t| t.norm.as_str()).collect::<Vec<_>>(),
            ["don't", "merge", "this"]
        );
    }

    #[test]
    fn folds_typographic_apostrophe() {
        assert_eq!(tokenize("don\u{2019}t")[0].norm, "don't");
    }

    #[test]
    fn splits_trailing_punctuation() {
        let t = tokenize("ship it.");
        assert_eq!(t.len(), 3);
        assert_eq!(t[2].kind, TokenKind::Punct);
    }

    #[test]
    fn keeps_identifiers_whole() {
        let t = tokenize("set MAX_RETRIES in .env to 15");
        let norms: Vec<_> = t.iter().map(|t| t.norm.as_str()).collect();
        assert!(norms.contains(&"max_retries"));
        assert!(norms.contains(&".env"));
        assert_eq!(t.last().unwrap().kind, TokenKind::Number);
    }

    #[test]
    fn a_percent_sign_belongs_to_its_number() {
        let t = tokenize("scale to 20% today");
        assert_eq!(t[2].norm, "20%");
        assert_eq!(t[2].kind, TokenKind::Number);
    }

    #[test]
    fn a_stray_percent_sign_is_punctuation() {
        let t = tokenize("100 % sure");
        assert!(
            t.iter()
                .any(|token| token.kind == TokenKind::Punct && token.norm == "%")
        );
    }

    #[test]
    fn digit_separators_stay_inside_the_number() {
        let t = tokenize("send 1,000 of them");
        assert_eq!(t[1].norm, "1,000");
        assert_eq!(t[1].kind, TokenKind::Number);
    }

    #[test]
    fn a_comma_between_words_still_splits() {
        let t = tokenize("hello, world");
        assert_eq!(t.len(), 3);
        assert_eq!(t[1].kind, TokenKind::Punct);
    }

    #[test]
    fn offsets_round_trip() {
        let text = "don't merge this before Friday.";
        for token in tokenize(text) {
            assert_eq!(&text[token.start..token.end], token.text);
        }
    }
}
