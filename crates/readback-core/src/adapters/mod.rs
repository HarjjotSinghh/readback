//! Turning engine-specific output into one [`Transcript`](crate::types::Transcript).
//!
//! Every recogniser reports confidence differently, or not at all. Readback
//! degrades to the later stages when an engine gives it nothing, so an adapter
//! is allowed to return a transcript with no per-word data.

pub mod faster_whisper;
pub mod generic;
pub mod parakeet;
pub mod whisper_cpp;

use crate::types::Word;
use serde_json::Value;

/// Reads a confidence-like field under any of the usual names.
///
/// NeMo calls it `confidence`, faster-whisper calls it `probability`,
/// whisper.cpp calls it `p`, and cloud APIs generally say `score`.
pub(crate) fn confidence_of(value: &Value) -> Option<f32> {
    ["confidence", "probability", "p", "score"]
        .iter()
        .find_map(|key| value.get(key).and_then(Value::as_f64))
        .map(|c| c as f32)
}

/// Reads a word's surface form under any of the usual names.
pub(crate) fn word_text_of(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    ["word", "text", "token"]
        .iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .map(str::to_string)
}

/// Seconds or milliseconds in, milliseconds out.
///
/// Engines are split on the unit, so anything below 1000 is read as seconds.
pub(crate) fn to_ms(value: Option<f64>) -> Option<u32> {
    value.map(|v| {
        if v < 1000.0 {
            (v * 1000.0).round() as u32
        } else {
            v.round() as u32
        }
    })
}

pub(crate) fn word_from(value: &Value) -> Option<Word> {
    let text = word_text_of(value)?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    let mut word = Word::new(text);
    word.confidence = confidence_of(value);
    word.start_ms = to_ms(value.get("start").and_then(Value::as_f64));
    word.end_ms = to_ms(value.get("end").and_then(Value::as_f64));
    Some(word)
}
