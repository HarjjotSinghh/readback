//! Adapter for NVIDIA Parakeet / NeMo output.
//!
//! NeMo exposes word-level confidence and, when asked, N-best hypotheses. Both
//! the flat shape and the `hypotheses` array are accepted, since the exact
//! wrapper depends on how the caller invoked the model.
//!
//! ```json
//! { "text": "merge this", "words": [ { "word": "merge", "confidence": 0.61 } ],
//!   "hypotheses": [ { "text": "don't merge this", "score": 0.39 } ] }
//! ```

use super::word_from;
use crate::error::{AdapterError, Result};
use crate::types::{Hypothesis, Transcript, Word};
use serde_json::Value;

const ENGINE: &str = "parakeet";

fn hypothesis_from(value: &Value) -> Option<Hypothesis> {
    let text = value
        .get("text")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if text.is_empty() {
        return None;
    }
    let words: Vec<Word> = value
        .get("words")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(word_from).collect())
        .unwrap_or_default();
    let score = ["score", "confidence"]
        .iter()
        .find_map(|k| value.get(k).and_then(Value::as_f64))
        .map(|s| s as f32);
    Some(Hypothesis {
        text,
        provider: Some(ENGINE.to_string()),
        score,
        words,
    })
}

pub fn from_json(json: &str) -> Result<Transcript> {
    let value: Value = serde_json::from_str(json)?;

    let listed: Vec<Hypothesis> = value
        .get("hypotheses")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(hypothesis_from).collect())
        .unwrap_or_default();

    // A flat `text` field is the best guess when present; otherwise the first
    // listed hypothesis is, since NeMo returns them best first.
    let mut candidates = Vec::new();
    if let Some(flat) = hypothesis_from(&value) {
        candidates.push(flat);
    }
    candidates.extend(listed);

    let mut candidates = candidates.into_iter();
    let primary = candidates
        .next()
        .ok_or(AdapterError::Empty { engine: ENGINE })?;
    let alternatives: Vec<Hypothesis> = candidates.filter(|alt| alt.text != primary.text).collect();

    Ok(Transcript {
        primary,
        alternatives,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_words_and_nbest() {
        let t = from_json(
            r#"{"text":"merge this","words":[{"word":"merge","confidence":0.61}],
                "hypotheses":[{"text":"don't merge this","score":0.39}]}"#,
        )
        .unwrap();
        assert_eq!(t.primary.text, "merge this");
        assert_eq!(t.primary.words[0].confidence, Some(0.61));
        assert_eq!(t.alternatives[0].text, "don't merge this");
    }

    #[test]
    fn falls_back_to_the_first_hypothesis() {
        let t = from_json(r#"{"hypotheses":[{"text":"ship it","score":0.8}]}"#).unwrap();
        assert_eq!(t.primary.text, "ship it");
        assert!(t.alternatives.is_empty());
    }

    #[test]
    fn a_duplicate_hypothesis_is_not_an_alternative() {
        let t = from_json(r#"{"text":"ship it","hypotheses":[{"text":"ship it"}]}"#).unwrap();
        assert!(t.alternatives.is_empty());
    }
}
