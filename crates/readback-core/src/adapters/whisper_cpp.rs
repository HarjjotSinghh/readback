//! Adapter for whisper.cpp's `--output-json-full`.
//!
//! whisper.cpp reports per-*token* probabilities, and its tokens are subwords:
//! `never` may arrive as `" nev"` + `"er"`. Tokens are merged back into words on
//! leading whitespace, and a word's confidence is the minimum of its tokens,
//! since one shaky piece makes the whole word shaky.

use crate::error::{AdapterError, Result};
use crate::types::{Hypothesis, Transcript, Word};
use serde_json::Value;

const ENGINE: &str = "whisper.cpp";

/// whisper.cpp reports offsets in milliseconds.
fn offset_ms(value: &Value, key: &str) -> Option<u32> {
    value.get("offsets")?.get(key)?.as_u64().map(|v| v as u32)
}

fn is_special(text: &str) -> bool {
    text.starts_with("[_") || (text.starts_with('<') && text.ends_with('>'))
}

fn push_token(words: &mut Vec<Word>, token: &Value) {
    let Some(raw) = token.get("text").and_then(Value::as_str) else {
        return;
    };
    if is_special(raw) {
        return;
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }

    let probability = token.get("p").and_then(Value::as_f64).map(|p| p as f32);
    let start = offset_ms(token, "from");
    let end = offset_ms(token, "to");
    let starts_word = raw.starts_with(char::is_whitespace) || words.is_empty();

    if starts_word {
        let mut word = Word::new(trimmed);
        word.confidence = probability;
        word.start_ms = start;
        word.end_ms = end;
        words.push(word);
        return;
    }

    // Continuation of the previous word.
    let word = words.last_mut().expect("words is non-empty");
    word.text.push_str(trimmed);
    word.end_ms = end.or(word.end_ms);
    word.confidence = match (word.confidence, probability) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
}

pub fn from_json(json: &str) -> Result<Transcript> {
    let value: Value = serde_json::from_str(json)?;
    let segments =
        value
            .get("transcription")
            .and_then(Value::as_array)
            .ok_or(AdapterError::MissingField {
                engine: ENGINE,
                field: "transcription",
            })?;

    let mut words: Vec<Word> = Vec::new();
    let mut text = String::new();

    for segment in segments {
        if let Some(t) = segment.get("text").and_then(Value::as_str) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t.trim());
        }
        if let Some(tokens) = segment.get("tokens").and_then(Value::as_array) {
            for token in tokens {
                push_token(&mut words, token);
            }
        }
    }

    if text.is_empty() && words.is_empty() {
        return Err(AdapterError::Empty { engine: ENGINE });
    }
    if text.is_empty() {
        text = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
    }

    Ok(Transcript {
        primary: Hypothesis {
            text,
            words,
            ..Default::default()
        }
        .with_provider(ENGINE),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"transcription":[{
        "text":" never merge",
        "tokens":[
            {"text":"[_BEG_]","p":0.9},
            {"text":" nev","p":0.51,"offsets":{"from":100,"to":300}},
            {"text":"er","p":0.42,"offsets":{"from":300,"to":420}},
            {"text":" merge","p":0.97,"offsets":{"from":420,"to":800}}
        ]}]}"#;

    #[test]
    fn merges_subword_tokens() {
        let t = from_json(SAMPLE).unwrap();
        assert_eq!(t.primary.words.len(), 2);
        assert_eq!(t.primary.words[0].text, "never");
    }

    #[test]
    fn a_word_is_only_as_confident_as_its_weakest_token() {
        let t = from_json(SAMPLE).unwrap();
        assert_eq!(t.primary.words[0].confidence, Some(0.42));
        assert_eq!(t.primary.words[0].end_ms, Some(420));
    }

    #[test]
    fn drops_special_tokens() {
        assert!(
            !from_json(SAMPLE)
                .unwrap()
                .primary
                .words
                .iter()
                .any(|w| w.text.contains("_BEG_"))
        );
    }
}
