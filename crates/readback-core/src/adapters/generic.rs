//! Fallback adapter for engines with no confidence data, and for hand-built
//! input. With no per-word signal, Readback leans entirely on the Cleanup Guard
//! and the stakes scorer.

use crate::error::{AdapterError, Result};
use crate::types::{Hypothesis, Transcript};
use serde_json::Value;

/// Wraps a plain string.
pub fn from_text(text: &str) -> Transcript {
    Transcript::from_text(text)
}

/// Builds a transcript from a best guess plus competing candidates.
pub fn from_candidates(primary: &str, alternatives: &[&str]) -> Transcript {
    Transcript {
        primary: Hypothesis::from_text(primary),
        alternatives: alternatives
            .iter()
            .map(|t| Hypothesis::from_text(*t))
            .collect(),
        ..Default::default()
    }
}

/// Reads `{"text": "..."}`, optionally with an `alternatives` array of strings
/// or objects carrying their own `text`.
pub fn from_json(json: &str) -> Result<Transcript> {
    let value: Value = serde_json::from_str(json)?;
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .ok_or(AdapterError::MissingField {
            engine: "generic",
            field: "text",
        })?;

    let alternatives = value
        .get("alternatives")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .or_else(|| item.get("text").and_then(Value::as_str).map(str::to_string))
                })
                .map(Hypothesis::from_text)
                .collect()
        })
        .unwrap_or_default();

    Ok(Transcript {
        primary: Hypothesis::from_text(text),
        alternatives,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_and_alternatives() {
        let t = from_json(r#"{"text":"merge it","alternatives":["don't merge it"]}"#).unwrap();
        assert_eq!(t.primary.text, "merge it");
        assert_eq!(t.alternatives[0].text, "don't merge it");
    }

    #[test]
    fn missing_text_is_an_error() {
        assert!(from_json(r#"{"segments":[]}"#).is_err());
    }
}
