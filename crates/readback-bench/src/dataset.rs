//! CriticalSpeechBench: cases where one wrong word changes the instruction.
//!
//! Every case records what was actually said (`reference`), what the recogniser
//! produced (`raw`), and optionally what an LLM cleanup step rewrote it into
//! (`cleaned`). A stack's output is judged against `reference`.
//!
//! **v0 is a text-level benchmark.** The recogniser errors are written by hand
//! rather than produced by running audio through a model, so it measures how a
//! reliability layer responds to a given error, not how often that error
//! happens. Audio fixtures are the obvious next step and would make the numbers
//! comparable across engines.

use anyhow::{Context as _, Result};
use readback_core::{SpeechRegion, Word};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The dataset shipped with the crate, so the runner works with no arguments.
pub const BUNDLED: &str = include_str!("../data/critical-speech-bench-v0.jsonl");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub id: String,
    /// Grouping for the per-category breakdown: `negation`, `environment`, …
    pub category: String,
    /// What the speaker actually said.
    pub reference: String,
    /// What the recogniser produced.
    pub raw: String,
    /// What an LLM cleanup step rewrote `raw` into, when the case has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleaned: Option<String>,
    /// Per-word evidence, for cases about acoustic confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<Word>>,
    /// Voice-activity regions, for cases about dropped words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech: Option<Vec<SpeechRegion>>,
    /// Destination app, for cases that depend on routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Terms the user would have in their dictionary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vocabulary: Vec<String>,
    /// True when the stack got it right, or got it harmlessly wrong. These are
    /// the controls: a reliability layer that holds them is unusable.
    #[serde(default)]
    pub benign: bool,
}

impl Case {
    /// What the stack would have pasted without Readback in the loop.
    pub fn baseline_text(&self) -> &str {
        self.cleaned.as_deref().unwrap_or(&self.raw)
    }
}

fn parse(contents: &str, source: &str) -> Result<Vec<Case>> {
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with("//"))
        .map(|(i, line)| {
            serde_json::from_str::<Case>(line).with_context(|| format!("{source}: line {}", i + 1))
        })
        .collect()
}

/// Loads the dataset bundled with this crate.
pub fn bundled() -> Result<Vec<Case>> {
    parse(BUNDLED, "bundled dataset")
}

/// Loads a JSONL dataset from disk.
pub fn load(path: &Path) -> Result<Vec<Case>> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&contents, &path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_dataset_parses() {
        let cases = bundled().expect("bundled dataset should parse");
        assert!(
            cases.len() >= 40,
            "expected a useful number of cases, got {}",
            cases.len()
        );
    }

    #[test]
    fn case_ids_are_unique() {
        let cases = bundled().unwrap();
        let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate case ids");
    }

    #[test]
    fn controls_are_present_in_useful_numbers() {
        let cases = bundled().unwrap();
        let benign = cases.iter().filter(|c| c.benign).count();
        assert!(
            benign * 4 >= cases.len(),
            "at least a quarter of cases should be controls, got {benign} of {}",
            cases.len()
        );
    }

    #[test]
    fn baseline_prefers_the_cleanup_output() {
        let case = Case {
            id: "x".into(),
            category: "negation".into(),
            reference: "never merge".into(),
            raw: "never merge".into(),
            cleaned: Some("Merge.".into()),
            words: None,
            speech: None,
            app: None,
            vocabulary: Vec::new(),
            benign: false,
        };
        assert_eq!(case.baseline_text(), "Merge.");
    }
}
