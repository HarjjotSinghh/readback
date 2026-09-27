//! The Cleanup Guard.
//!
//! Most dictation stacks run an LLM "polish" pass over the raw transcript. That
//! pass is fluent, confident, and perfectly willing to delete the word `not`.
//! This stage aligns the raw transcript against the polished text and reverts
//! any edit that removed a protected token, keeping the rest of the polish.
//!
//! It is deterministic and needs no model.

use crate::align::{Edit, EditKind, align};
use crate::lexicon::{Lexicon, SemanticClass};
use crate::tokenize::{Token, tokenize};
use crate::types::{Flag, Span};
use std::collections::HashMap;
use std::ops::Range;

/// Result of running the guard over a raw/cleaned pair.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardOutcome {
    /// The text to use downstream: the polish, minus any meaning-changing edit.
    pub text: String,
    pub flags: Vec<Flag>,
    /// True when at least one span was restored from the raw transcript.
    pub reverted: bool,
}

#[derive(Debug)]
struct Emitted {
    text: String,
    is_punct: bool,
}

#[derive(Debug)]
struct PendingFlag {
    class: SemanticClass,
    substituted: bool,
    out_range: Range<usize>,
    lost: Vec<String>,
    replacement: Option<String>,
}

/// The identity a protected token is tracked by.
///
/// Numbers are keyed by canonical value, so a cleanup step rewriting `three` as
/// `3` is not mistaken for one rewriting `fifteen` as `fifty`. Everything else
/// is keyed by its normalised spelling.
fn loss_key(token: &Token, lex: &Lexicon) -> String {
    if lex.classify(token) == Some(SemanticClass::Number)
        && let Some(canonical) = crate::number::canonical(token)
    {
        return canonical;
    }
    token.norm.clone()
}

/// Counts protected tokens by the identity they are tracked under.
fn protected_counts(tokens: &[Token], lex: &Lexicon) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for token in tokens.iter().filter(|t| lex.is_protected(t)) {
        *counts.entry(loss_key(token, lex)).or_insert(0) += 1;
    }
    counts
}

/// Protected tokens in `range` that the cleanup genuinely lost.
///
/// A token that merely moved elsewhere in the sentence is not lost, so the
/// budget is computed from whole-utterance counts rather than per-span
/// membership. Each reported loss consumes one unit of that budget.
fn lost_in_range(
    raw: &[Token],
    range: &Range<usize>,
    lex: &Lexicon,
    budget: &mut HashMap<String, usize>,
) -> Vec<(usize, SemanticClass)> {
    let mut lost = Vec::new();
    for (offset, token) in raw[range.clone()].iter().enumerate() {
        let Some(class) = lex.classify(token) else {
            continue;
        };
        let Some(remaining) = budget.get_mut(&loss_key(token, lex)) else {
            continue;
        };
        if *remaining == 0 {
            continue;
        }
        *remaining -= 1;
        lost.push((range.start + offset, class));
    }
    lost
}

/// Drops punctuation, which the polish step tends to pull into an edit.
fn content_tokens(tokens: &[Token]) -> Vec<&Token> {
    tokens.iter().filter(|t| !t.is_punct()).collect()
}

fn push_tokens(out: &mut Vec<Emitted>, tokens: &[Token]) {
    out.extend(tokens.iter().map(|t| Emitted {
        text: t.text.clone(),
        is_punct: t.is_punct(),
    }));
}

/// Punctuation that hugs the previous token instead of taking a space.
fn hugs_left(text: &str) -> bool {
    matches!(
        text,
        "," | "." | ";" | ":" | "!" | "?" | ")" | "]" | "}" | "%" | "'" | "\u{2019}" | "\""
    )
}

fn hugs_right(text: &str) -> bool {
    matches!(text, "(" | "[" | "{" | "$" | "#" | "@")
}

/// Joins emitted tokens back into text, returning each token's byte span.
fn render(out: &[Emitted]) -> (String, Vec<Span>) {
    let mut text = String::new();
    let mut spans = Vec::with_capacity(out.len());
    let mut open_quote = false;

    for (i, tok) in out.iter().enumerate() {
        let mut space = i > 0;
        if i > 0 {
            let prev = &out[i - 1];
            if tok.is_punct && hugs_left(&tok.text) {
                space = false;
            }
            if prev.is_punct && hugs_right(&prev.text) {
                space = false;
            }
            if tok.text == "\"" {
                // Alternate: opening quote hugs right, closing quote hugs left.
                space = !open_quote;
            }
            if prev.text == "\"" && !open_quote {
                space = false;
            }
        }
        if tok.text == "\"" {
            open_quote = !open_quote;
        }
        if space {
            text.push(' ');
        }
        let start = text.len();
        text.push_str(&tok.text);
        spans.push(Span::new(start, text.len()));
    }

    (text, spans)
}

fn evidence_for(lost: &[String], replacement: Option<&String>) -> String {
    let words = lost.join("\", \"");
    match replacement {
        Some(rep) => {
            format!("cleanup rewrote \"{words}\" as \"{rep}\"; restored from the raw transcript")
        }
        None => format!("cleanup dropped \"{words}\"; restored from the raw transcript"),
    }
}

/// Runs the guard over a raw transcript and its cleaned rewrite.
///
/// When the cleanup changed nothing protected, `cleaned` is returned byte for
/// byte and `reverted` is false.
pub fn check_cleanup(raw_text: &str, cleaned_text: &str, lex: &Lexicon) -> GuardOutcome {
    let raw = tokenize(raw_text);
    let cleaned = tokenize(cleaned_text);
    let edits = align(&raw, &cleaned);

    let raw_counts = protected_counts(&raw, lex);
    let cleaned_counts = protected_counts(&cleaned, lex);
    let mut budget: HashMap<String, usize> = raw_counts
        .into_iter()
        .filter_map(|(norm, raw_n)| {
            let kept = cleaned_counts.get(&norm).copied().unwrap_or(0);
            raw_n
                .checked_sub(kept)
                .filter(|n| *n > 0)
                .map(|n| (norm, n))
        })
        .collect();

    let mut out: Vec<Emitted> = Vec::with_capacity(cleaned.len());
    let mut pending: Vec<PendingFlag> = Vec::new();
    let mut reverted = false;

    for Edit { kind, left, right } in edits {
        match kind {
            EditKind::Equal | EditKind::Insert => push_tokens(&mut out, &cleaned[right]),
            EditKind::Delete | EditKind::Replace => {
                // `20 percent` becoming `20%` is a re-spelling, not a change.
                // Punctuation is dropped first, since the polish step tends to
                // pull a full stop into the same edit.
                let renumbered = kind == EditKind::Replace
                    && crate::number::same_phrase(
                        &content_tokens(&raw[left.clone()]),
                        &content_tokens(&cleaned[right.clone()]),
                    );
                if renumbered {
                    push_tokens(&mut out, &cleaned[right]);
                    continue;
                }

                let lost = lost_in_range(&raw, &left, lex, &mut budget);
                if lost.is_empty() {
                    push_tokens(&mut out, &cleaned[right]);
                    continue;
                }

                // Restore the raw span verbatim. Reverting the whole span rather
                // than splicing single words keeps the sentence grammatical.
                let start = out.len();
                push_tokens(&mut out, &raw[left.clone()]);
                let flagged_end = out.len();
                reverted = true;

                // Trailing punctuation belongs to the polish, not to the
                // meaning, so it survives the revert. Without this, reverting
                // the last span of a sentence eats its full stop.
                let kept_punct = cleaned[right.clone()]
                    .iter()
                    .rev()
                    .take_while(|t| t.is_punct())
                    .count();
                if kept_punct > 0 {
                    push_tokens(&mut out, &cleaned[right.end - kept_punct..right.end]);
                }

                let class = lost
                    .iter()
                    .map(|(_, c)| *c)
                    .max_by(|a, b| {
                        a.removal_flag()
                            .severity()
                            .cmp(&b.removal_flag().severity())
                    })
                    .expect("lost is non-empty");
                let lost_words: Vec<String> =
                    lost.iter().map(|(i, _)| raw[*i].text.clone()).collect();
                // Quote the rewrite exactly as it was written, minus the
                // trailing punctuation that was kept rather than replaced.
                let quoted = right.start..right.end - kept_punct;
                let replacement = (kind == EditKind::Replace && !quoted.is_empty()).then(|| {
                    let from = cleaned[quoted.start].start;
                    let to = cleaned[quoted.end - 1].end;
                    cleaned_text[from..to].trim().to_string()
                });
                let substituted = kind == EditKind::Replace
                    && cleaned[right.clone()]
                        .iter()
                        .any(|t| lex.classify(t) == Some(class));

                pending.push(PendingFlag {
                    class,
                    substituted,
                    out_range: start..flagged_end,
                    lost: lost_words,
                    replacement,
                });
            }
        }
    }

    if !reverted {
        // Nothing protected was touched: hand back the polish untouched.
        return GuardOutcome {
            text: cleaned_text.to_string(),
            flags: Vec::new(),
            reverted: false,
        };
    }

    let (text, spans) = render(&out);
    let flags = pending
        .into_iter()
        .map(|p| {
            let kind = if p.substituted {
                p.class.substitution_flag()
            } else {
                p.class.removal_flag()
            };
            let span = Span::new(
                spans[p.out_range.start].start,
                spans[p.out_range.end - 1].end,
            );
            Flag {
                kind,
                severity: kind.severity(),
                span,
                evidence: evidence_for(&p.lost, p.replacement.as_ref()),
                suggestion: Some(span.slice(&text).to_string()),
            }
        })
        .collect();

    GuardOutcome {
        text,
        flags,
        reverted: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexicon::Locale;
    use crate::types::FlagKind;

    #[test]
    fn restores_a_dropped_negation() {
        let lex = Lexicon::default();
        let out = check_cleanup(
            "never merge a change like this",
            "Merge a change like this.",
            &lex,
        );
        assert!(out.reverted);
        assert!(out.text.to_lowercase().contains("never"));
        assert_eq!(out.flags[0].kind, FlagKind::DroppedNegation);
    }

    #[test]
    fn keeps_harmless_polish_byte_for_byte() {
        let lex = Lexicon::default();
        let cleaned = "We need to deploy it tomorrow.";
        let out = check_cleanup(
            "um so basically we uh need to deploy it tomorrow",
            cleaned,
            &lex,
        );
        assert!(!out.reverted);
        assert_eq!(out.text, cleaned);
        assert!(out.flags.is_empty());
    }

    #[test]
    fn catches_an_inverted_instruction() {
        let lex = Lexicon::default();
        let out = check_cleanup(
            "we should not merge this until Bobby checks it",
            "We should merge this once Bobby checks it.",
            &lex,
        );
        assert!(out.reverted);
        assert!(
            out.flags
                .iter()
                .any(|f| f.kind == FlagKind::DroppedNegation)
        );
    }

    #[test]
    fn a_moved_word_is_not_a_loss() {
        let lex = Lexicon::default();
        let out = check_cleanup(
            "merge this not before Friday",
            "Do not merge this before Friday.",
            &lex,
        );
        assert!(!out.reverted, "text was: {}", out.text);
    }

    #[test]
    fn guards_numbers() {
        let lex = Lexicon::default();
        let out = check_cleanup("set max retries to 15", "Set max retries to 50.", &lex);
        assert!(out.reverted);
        assert!(out.text.contains("15"));
        assert_eq!(out.flags[0].kind, FlagKind::ChangedNumber);
    }

    #[test]
    fn rewriting_a_number_is_not_changing_it() {
        let lex = Lexicon::default();
        for (raw, cleaned) in [
            (
                "the meeting got moved to three",
                "The meeting got moved to 3.",
            ),
            ("send 1000 of them", "Send 1,000 of them."),
            ("set retries to fifteen", "Set retries to 15."),
        ] {
            let out = check_cleanup(raw, cleaned, &lex);
            assert!(
                !out.reverted,
                "{raw:?} -> {cleaned:?} was reverted to {:?}",
                out.text
            );
        }
    }

    #[test]
    fn a_spelled_unit_becoming_a_symbol_is_not_a_change() {
        let lex = Lexicon::default();
        let out = check_cleanup("scale it to 20 percent", "Scale it to 20%.", &lex);
        assert!(!out.reverted, "text was: {}", out.text);
    }

    #[test]
    fn changing_a_number_is_still_caught() {
        let lex = Lexicon::default();
        for (raw, cleaned) in [
            ("set retries to fifteen", "Set retries to 50."),
            ("refund $50", "Refund 50."),
            ("scale to 20%", "Scale to 20."),
        ] {
            let out = check_cleanup(raw, cleaned, &lex);
            assert!(out.reverted, "{raw:?} -> {cleaned:?} slipped through");
            assert_eq!(out.flags[0].kind, FlagKind::ChangedNumber);
        }
    }

    #[test]
    fn guards_environments() {
        let lex = Lexicon::default();
        let out = check_cleanup("deploy this to staging", "Deploy this to production.", &lex);
        assert!(out.reverted);
        assert_eq!(out.flags[0].kind, FlagKind::ChangedEnvironment);
    }

    #[test]
    fn reverting_the_last_span_keeps_the_full_stop() {
        let lex = Lexicon::default();
        let out = check_cleanup("deploy this to staging", "Deploy this to production.", &lex);
        assert!(out.text.ends_with("staging."), "text was: {}", out.text);
        assert_eq!(out.flags[0].span.slice(&out.text), "staging");
    }

    #[test]
    fn evidence_quotes_the_rewrite_as_written() {
        let lex = Lexicon::default();
        let out = check_cleanup("deploy this to staging", "Deploy this to production.", &lex);
        assert!(
            out.flags[0].evidence.contains("\"production\""),
            "{}",
            out.flags[0].evidence
        );
    }

    #[test]
    fn guards_hinglish_negation() {
        let lex = Lexicon::new(&[Locale::En, Locale::Hinglish]);
        let out = check_cleanup("abhi mat bhejo", "Bhejo abhi.", &lex);
        assert!(out.reverted, "text was: {}", out.text);
    }

    #[test]
    fn guards_user_vocabulary() {
        let mut lex = Lexicon::default();
        lex.protect(["Harpawan"]);
        let out = check_cleanup("ask Harpawan about it", "Ask Harpreet about it.", &lex);
        assert!(out.reverted);
        assert_eq!(out.flags[0].kind, FlagKind::ChangedProtectedTerm);
    }

    #[test]
    fn flag_spans_point_at_restored_text() {
        let lex = Lexicon::default();
        let out = check_cleanup("do not deploy this", "Deploy this.", &lex);
        let flag = &out.flags[0];
        assert!(flag.span.slice(&out.text).to_lowercase().contains("not"));
    }
}
