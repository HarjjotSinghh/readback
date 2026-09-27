//! Omission detection: speech the recogniser never turned into a word.
//!
//! This is the one failure class text can never catch. "Merge a change like
//! this" is a perfectly grammatical sentence, so no language model can tell
//! that "never" used to be in front of it. The audio can: if voice-activity
//! detection says someone was speaking for 250 ms and no transcribed word is
//! aligned to that stretch, a word went missing there.
//!
//! It is probabilistic and it will produce false positives in noisy rooms. It
//! is reported as evidence, never as a correction.

use crate::tokenize::tokenize;
use crate::types::{AudioEvidence, Flag, FlagKind, Span, SpeechRegion, Word};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OmissionConfig {
    /// Uncovered speech shorter than this is ignored. Below roughly 150 ms a
    /// gap is more likely to be a breath or an alignment wobble than a word.
    pub min_gap_ms: u32,
    /// Word timings are stretched by this much on each side before looking for
    /// gaps, since engines clip the edges of a word.
    pub padding_ms: u32,
}

impl Default for OmissionConfig {
    fn default() -> Self {
        Self {
            min_gap_ms: 180,
            padding_ms: 60,
        }
    }
}

/// One stretch of speech with nothing transcribed over it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    pub start_ms: u32,
    pub end_ms: u32,
    /// Index of the first word that begins after the gap, when there is one.
    pub before_word: Option<usize>,
}

impl Gap {
    pub fn duration_ms(&self) -> u32 {
        self.end_ms.saturating_sub(self.start_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OmissionOutcome {
    /// `0.0..=1.0`, rising with the length of the longest gap.
    pub score: f32,
    pub gaps: Vec<Gap>,
    pub flags: Vec<Flag>,
}

/// Word intervals, padded and merged, sorted by start time.
fn covered_intervals(words: &[Word], padding_ms: u32) -> Vec<(u32, u32)> {
    let mut intervals: Vec<(u32, u32)> = words
        .iter()
        .filter_map(|w| Some((w.start_ms?, w.end_ms?)))
        .map(|(a, b)| (a.saturating_sub(padding_ms), b.saturating_add(padding_ms)))
        .collect();
    intervals.sort_unstable();

    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(intervals.len());
    for (start, end) in intervals {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// Subtracts the covered intervals from one speech region.
fn uncovered(region: SpeechRegion, covered: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut holes = Vec::new();
    let mut cursor = region.start_ms;

    for &(start, end) in covered {
        if end <= cursor {
            continue;
        }
        if start >= region.end_ms {
            break;
        }
        if start > cursor {
            holes.push((cursor, start.min(region.end_ms)));
        }
        cursor = cursor.max(end);
        if cursor >= region.end_ms {
            return holes;
        }
    }
    if cursor < region.end_ms {
        holes.push((cursor, region.end_ms));
    }
    holes
}

/// The first word starting at or after `at_ms`.
fn word_after(words: &[Word], at_ms: u32) -> Option<usize> {
    words
        .iter()
        .enumerate()
        .filter(|(_, w)| w.start_ms.is_some_and(|s| s >= at_ms))
        .min_by_key(|(_, w)| w.start_ms.unwrap_or(u32::MAX))
        .map(|(i, _)| i)
}

/// Byte offset in `text` where the missing word most likely belonged.
///
/// Words are matched by position rather than by time, since `text` may be the
/// polished rewrite. When nothing matches, the flag anchors at the end.
fn anchor(text: &str, words: &[Word], before_word: Option<usize>) -> Span {
    let tokens = tokenize(text);
    let Some(index) = before_word else {
        return Span::new(text.len(), text.len());
    };
    // Count how many non-punctuation tokens precede the target word, then find
    // that token in the output text.
    let target = words[..index]
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .count();
    let content: Vec<&crate::tokenize::Token> = tokens.iter().filter(|t| !t.is_punct()).collect();
    match content.get(target) {
        Some(token) => Span::new(token.start, token.start),
        None => Span::new(text.len(), text.len()),
    }
}

/// Finds speech that no word covers.
pub fn detect(
    audio: &AudioEvidence,
    words: &[Word],
    text: &str,
    cfg: &OmissionConfig,
) -> OmissionOutcome {
    if audio.speech.is_empty() {
        return OmissionOutcome::default();
    }
    let covered = covered_intervals(words, cfg.padding_ms);
    if covered.is_empty() {
        // No timings at all: the host gave us regions we cannot check against,
        // so say nothing rather than flagging the whole clip.
        return OmissionOutcome::default();
    }

    let mut gaps = Vec::new();
    for region in &audio.speech {
        for (start, end) in uncovered(*region, &covered) {
            if end.saturating_sub(start) < cfg.min_gap_ms {
                continue;
            }
            gaps.push(Gap {
                start_ms: start,
                end_ms: end,
                before_word: word_after(words, end),
            });
        }
    }

    let longest = gaps.iter().map(Gap::duration_ms).max().unwrap_or(0);
    // A gap at the detection threshold is weak evidence; roughly a second of
    // unexplained speech is about as suspicious as this stage ever gets.
    let score = if gaps.is_empty() {
        0.0
    } else {
        ((longest as f32 - cfg.min_gap_ms as f32) / 800.0 + 0.35).clamp(0.0, 0.95)
    };

    let flags = gaps
        .iter()
        .map(|gap| Flag {
            kind: FlagKind::PossibleOmission,
            severity: FlagKind::PossibleOmission.severity(),
            span: anchor(text, words, gap.before_word),
            evidence: format!(
                "{} ms of speech at {:.2}s has no transcribed word aligned to it",
                gap.duration_ms(),
                gap.start_ms as f32 / 1000.0
            ),
            suggestion: None,
        })
        .collect();

    OmissionOutcome { score, gaps, flags }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Vec<Word> {
        vec![
            Word::new("merge").with_timing(1000, 1400),
            Word::new("a").with_timing(1400, 1500),
            Word::new("change").with_timing(1500, 1900),
        ]
    }

    #[test]
    fn silence_produces_no_gaps() {
        let audio = AudioEvidence::from_regions([(1000, 1900)]);
        let out = detect(
            &audio,
            &words(),
            "merge a change",
            &OmissionConfig::default(),
        );
        assert!(out.gaps.is_empty());
        assert_eq!(out.score, 0.0);
    }

    #[test]
    fn speech_before_the_first_word_is_a_gap() {
        // 400 ms of speech where "never" used to be.
        let audio = AudioEvidence::from_regions([(500, 1900)]);
        let out = detect(
            &audio,
            &words(),
            "merge a change",
            &OmissionConfig::default(),
        );
        assert_eq!(out.gaps.len(), 1);
        assert_eq!(out.gaps[0].before_word, Some(0));
        assert!(out.score > 0.3);
    }

    #[test]
    fn the_flag_anchors_where_the_word_belonged() {
        let audio = AudioEvidence::from_regions([(500, 1900)]);
        let out = detect(
            &audio,
            &words(),
            "merge a change",
            &OmissionConfig::default(),
        );
        assert_eq!(out.flags[0].span, Span::new(0, 0));
    }

    #[test]
    fn a_gap_between_words_anchors_at_the_following_word() {
        let mut w = words();
        w[2] = Word::new("change").with_timing(2400, 2800);
        let audio = AudioEvidence::from_regions([(1000, 2800)]);
        let out = detect(&audio, &w, "merge a change", &OmissionConfig::default());
        assert_eq!(out.gaps.len(), 1);
        assert_eq!(out.flags[0].span.slice("merge a changex"), "");
        assert_eq!(out.flags[0].span, Span::new(8, 8));
    }

    #[test]
    fn short_gaps_are_breaths_not_words() {
        let audio = AudioEvidence::from_regions([(900, 1900)]);
        let out = detect(
            &audio,
            &words(),
            "merge a change",
            &OmissionConfig::default(),
        );
        assert!(
            out.gaps.is_empty(),
            "100 ms is below the floor and padding covers it"
        );
    }

    #[test]
    fn without_timings_the_stage_stays_quiet() {
        let audio = AudioEvidence::from_regions([(0, 5000)]);
        let untimed = vec![Word::new("merge"), Word::new("this")];
        assert!(
            detect(&audio, &untimed, "merge this", &OmissionConfig::default())
                .gaps
                .is_empty()
        );
    }
}
