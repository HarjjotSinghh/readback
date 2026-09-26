//! Adapter for faster-whisper, whose segments carry both word probabilities and
//! the decoder tells Readback cares about.
//!
//! Expected shape (the usual `[segment._asdict() for segment in segments]`
//! dump, either bare or under a `segments` key):
//!
//! ```json
//! { "segments": [ {
//!     "text": " never merge this",
//!     "avg_logprob": -0.21, "no_speech_prob": 0.01, "compression_ratio": 1.3,
//!     "words": [ { "word": " never", "start": 0.1, "end": 0.4, "probability": 0.42 } ]
//! } ] }
//! ```

use super::word_from;
use crate::error::{AdapterError, Result};
use crate::types::{DecodeSignals, Hypothesis, Transcript, Word};
use serde_json::Value;

const ENGINE: &str = "faster-whisper";

fn segments_of(value: &Value) -> Option<&Vec<Value>> {
    value
        .get("segments")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())
}

pub fn from_json(json: &str) -> Result<Transcript> {
    let value: Value = serde_json::from_str(json)?;
    let segments = segments_of(&value).ok_or(AdapterError::MissingField {
        engine: ENGINE,
        field: "segments",
    })?;

    let mut words: Vec<Word> = Vec::new();
    let mut text = String::new();
    // Decoder tells are per segment; the worst one describes the utterance.
    let mut avg_logprob: Option<f32> = None;
    let mut no_speech: Option<f32> = None;
    let mut compression: Option<f32> = None;

    for segment in segments {
        if let Some(t) = segment.get("text").and_then(Value::as_str) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t.trim());
        }
        if let Some(list) = segment.get("words").and_then(Value::as_array) {
            words.extend(list.iter().filter_map(word_from));
        }
        if let Some(v) = segment.get("avg_logprob").and_then(Value::as_f64) {
            avg_logprob = Some(avg_logprob.map_or(v as f32, |cur: f32| cur.min(v as f32)));
        }
        if let Some(v) = segment.get("no_speech_prob").and_then(Value::as_f64) {
            no_speech = Some(no_speech.map_or(v as f32, |cur: f32| cur.max(v as f32)));
        }
        if let Some(v) = segment.get("compression_ratio").and_then(Value::as_f64) {
            compression = Some(compression.map_or(v as f32, |cur: f32| cur.max(v as f32)));
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
        alternatives: Vec::new(),
        signals: DecodeSignals {
            avg_logprob,
            no_speech_prob: no_speech,
            compression_ratio: compression,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"segments":[{
        "text":" never merge this","avg_logprob":-0.21,"no_speech_prob":0.02,"compression_ratio":1.3,
        "words":[
            {"word":" never","start":0.10,"end":0.42,"probability":0.42},
            {"word":" merge","start":0.42,"end":0.80,"probability":0.97},
            {"word":" this","start":0.80,"end":1.00,"probability":0.99}
        ]}]}"#;

    #[test]
    fn reads_words_and_signals() {
        let t = from_json(SAMPLE).unwrap();
        assert_eq!(t.primary.text, "never merge this");
        assert_eq!(t.primary.words.len(), 3);
        assert_eq!(t.primary.words[0].confidence, Some(0.42));
        assert_eq!(t.primary.words[0].start_ms, Some(100));
        assert_eq!(t.signals.no_speech_prob, Some(0.02));
    }

    #[test]
    fn accepts_a_bare_segment_array() {
        let bare = r#"[{"text":"ship it","words":[]}]"#;
        assert_eq!(from_json(bare).unwrap().primary.text, "ship it");
    }
}
