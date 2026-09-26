//! Token-level alignment between two versions of the same utterance.
//!
//! Used to compare the raw transcript against a cleaned-up rewrite, and to
//! compare competing hypotheses from different engines.

use crate::tokenize::Token;
use similar::{Algorithm, DiffOp, capture_diff_slices};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// Both sides agree.
    Equal,
    /// Present on the left, gone on the right.
    Delete,
    /// Absent on the left, added on the right.
    Insert,
    /// Both sides have tokens here, and they differ.
    Replace,
}

/// One aligned region between the two token sequences.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub kind: EditKind,
    pub left: Range<usize>,
    pub right: Range<usize>,
}

impl Edit {
    pub fn is_equal(&self) -> bool {
        self.kind == EditKind::Equal
    }
}

/// Aligns two token sequences on their normalised forms.
///
/// Punctuation participates in the alignment so that byte spans stay exact,
/// but callers are expected to ignore punctuation-only edits.
pub fn align(left: &[Token], right: &[Token]) -> Vec<Edit> {
    let l: Vec<&str> = left.iter().map(|t| t.norm.as_str()).collect();
    let r: Vec<&str> = right.iter().map(|t| t.norm.as_str()).collect();

    capture_diff_slices(Algorithm::Myers, &l, &r)
        .into_iter()
        .map(|op| match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => Edit {
                kind: EditKind::Equal,
                left: old_index..old_index + len,
                right: new_index..new_index + len,
            },
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => Edit {
                kind: EditKind::Delete,
                left: old_index..old_index + old_len,
                right: new_index..new_index,
            },
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => Edit {
                kind: EditKind::Insert,
                left: old_index..old_index,
                right: new_index..new_index + new_len,
            },
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => Edit {
                kind: EditKind::Replace,
                left: old_index..old_index + old_len,
                right: new_index..new_index + new_len,
            },
        })
        .collect()
}

/// Byte span covering a range of tokens in the text they came from.
///
/// Returns `None` for an empty token range, since there is nothing to point at.
pub fn token_span(tokens: &[Token], range: &Range<usize>) -> Option<crate::types::Span> {
    let first = tokens.get(range.start)?;
    let last = tokens.get(range.end.checked_sub(1)?)?;
    Some(crate::types::Span::new(first.start, last.end))
}

/// Byte span for a token range, falling back to a zero-width span at the
/// position where the missing tokens should have been.
pub fn token_span_or_point(tokens: &[Token], range: &Range<usize>) -> crate::types::Span {
    if let Some(span) = token_span(tokens, range) {
        return span;
    }
    // Empty range: anchor just after the preceding token, or at the start.
    let at = range
        .start
        .checked_sub(1)
        .and_then(|i| tokens.get(i))
        .map(|t| t.end)
        .unwrap_or(0);
    crate::types::Span::new(at, at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::tokenize;

    #[test]
    fn detects_a_deleted_word() {
        let left = tokenize("never merge this");
        let right = tokenize("merge this");
        let edits = align(&left, &right);
        let deleted: Vec<_> = edits
            .iter()
            .filter(|e| e.kind == EditKind::Delete)
            .collect();
        assert_eq!(deleted.len(), 1);
        assert_eq!(left[deleted[0].left.clone()][0].norm, "never");
    }

    #[test]
    fn detects_a_replacement() {
        let left = tokenize("deploy to staging");
        let right = tokenize("deploy to production");
        let edits = align(&left, &right);
        assert!(edits.iter().any(|e| e.kind == EditKind::Replace));
    }

    #[test]
    fn identical_text_yields_only_equal() {
        let t = tokenize("ship it on Friday");
        assert!(align(&t, &t).iter().all(Edit::is_equal));
    }

    #[test]
    fn spans_point_at_the_right_bytes() {
        let text = "never merge this";
        let tokens = tokenize(text);
        let span = token_span(&tokens, &(0..1)).unwrap();
        assert_eq!(span.slice(text), "never");
    }
}
