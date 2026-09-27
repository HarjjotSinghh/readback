//! Loading transcripts and configuration from files, stdin and flags.

use crate::cli::{ConfigArgs, Engine, InputArgs};
use anyhow::{Context as _, Result, bail};
use readback_core::config::Config;
use readback_core::lexicon::Locale;
use readback_core::policy::Policy;
use readback_core::{Context, Transcript, adapters};
use std::io::Read;
use std::path::Path;

/// Reads a file, or stdin when the path is `-`.
pub fn read_source(path: &Path) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        return Ok(buf);
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Guesses the engine from the shape of the payload.
///
/// Each engine has one unmistakable top-level key, so this is a lookup rather
/// than a heuristic. Anything that is not JSON is treated as plain text.
fn detect(payload: &str) -> Engine {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return Engine::Text;
    };
    if value.get("transcription").is_some() {
        return Engine::WhisperCpp;
    }
    if value.get("segments").is_some() || value.is_array() {
        return Engine::FasterWhisper;
    }
    if value.get("hypotheses").is_some() {
        return Engine::Parakeet;
    }
    if value.get("text").is_some() {
        return Engine::Generic;
    }
    Engine::Text
}

fn parse_with(engine: Engine, payload: &str) -> Result<Transcript> {
    let parsed = match engine {
        Engine::Text => return Ok(adapters::generic::from_text(payload.trim())),
        Engine::FasterWhisper => adapters::faster_whisper::from_json(payload),
        Engine::WhisperCpp => adapters::whisper_cpp::from_json(payload),
        Engine::Parakeet => adapters::parakeet::from_json(payload),
        Engine::Generic => adapters::generic::from_json(payload),
        Engine::Auto => unreachable!("auto is resolved before parsing"),
    };
    parsed.map_err(Into::into)
}

/// The engine actually used, for reporting.
pub struct LoadedTranscript {
    pub transcript: Transcript,
    pub engine: Engine,
}

pub fn load_transcript(args: &InputArgs) -> Result<LoadedTranscript> {
    if let Some(text) = &args.text {
        return Ok(LoadedTranscript {
            transcript: adapters::generic::from_text(text),
            engine: Engine::Text,
        });
    }

    let Some(path) = &args.raw else {
        bail!("no transcript given: pass --raw <FILE> or --text <STRING>");
    };
    let payload = read_source(path)?;
    if payload.trim().is_empty() {
        bail!("transcript input was empty");
    }

    let engine = match args.from {
        Engine::Auto => detect(&payload),
        explicit => explicit,
    };
    let transcript = parse_with(engine, &payload)
        .with_context(|| format!("parsing transcript as {engine:?}"))?;
    Ok(LoadedTranscript { transcript, engine })
}

pub fn load_cleaned(args: &InputArgs) -> Result<Option<String>> {
    if let Some(text) = &args.cleaned_text {
        return Ok(Some(text.clone()));
    }
    match &args.cleaned {
        Some(path) => Ok(Some(read_source(path)?.trim().to_string())),
        None => Ok(None),
    }
}

fn parse_locale(name: &str) -> Result<Locale> {
    match name.to_lowercase().as_str() {
        "en" | "english" => Ok(Locale::En),
        "hinglish" | "hi" => Ok(Locale::Hinglish),
        other => bail!("unknown locale {other:?}; expected `en` or `hinglish`"),
    }
}

/// Builds a [`Config`] from a file, then applies the flags on top.
pub fn build_config(args: &ConfigArgs) -> Result<Config> {
    let mut config = match &args.config {
        Some(path) => serde_json::from_str(&read_source(path)?)
            .with_context(|| format!("parsing config {}", path.display()))?,
        None => Config::default(),
    };

    if args.recommended {
        config.policy = Policy::recommended();
    }
    if !args.locale.is_empty() {
        config.locales = args
            .locale
            .iter()
            .map(|l| parse_locale(l))
            .collect::<Result<Vec<_>>>()?;
    }
    config.vocabulary.extend(args.vocab.iter().cloned());
    if let Some(path) = &args.vocab_file {
        config.vocabulary.extend(
            read_source(path)?
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string),
        );
    }

    Ok(config)
}

pub fn build_context(args: &ConfigArgs) -> Context {
    Context {
        app: args.app.clone(),
        vocabulary: Vec::new(),
    }
}
