//! End-to-end tests for the audio harness.
//!
//! A stub engine stands in for a real ASR: it is handed a clip and prints a
//! canned transcript, which is enough to prove the harness runs a command,
//! parses its output, scores it against the reference and routes the verdict.

use readback_bench::audio::{self, AsrCommand, AudioCase};
use readback_bench::runner;
use readback_core::config::Config;
use readback_core::{Action, Locale};
use std::path::{Path, PathBuf};

/// A directory with two clips and a stub engine that transcribes them wrongly.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("readback-audio-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("clips")).unwrap();
        Self { dir }
    }

    /// Writes a 16 kHz mono WAV: silence, then a tone, then silence.
    fn clip(&self, id: &str, speech_ms: u32) -> &Self {
        let rate = 16_000u32;
        let lead = rate / 4; // 250 ms of silence
        let speech = rate * speech_ms / 1000;
        let samples: Vec<i16> = (0..lead)
            .map(|_| 0i16)
            .chain((0..speech).map(|i| ((i as f32 * 0.2).sin() * 8000.0) as i16))
            .chain((0..lead).map(|_| 0i16))
            .collect();

        let data_len = samples.len() as u32 * 2;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + data_len).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes()); // PCM
        wav.extend(1u16.to_le_bytes()); // mono
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 2).to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data_len.to_le_bytes());
        for sample in samples {
            wav.extend(sample.to_le_bytes());
        }

        std::fs::write(self.dir.join("clips").join(format!("{id}.wav")), wav).unwrap();
        self
    }

    /// A stub engine: prints the transcript mapped to the clip's basename.
    fn engine(&self, mapping: &[(&str, &str)]) -> String {
        let mut script = String::from("#!/bin/sh\ncase \"$(basename \"$1\" .wav)\" in\n");
        for (id, text) in mapping {
            script.push_str(&format!("  {id}) printf '%s' '{text}' ;;\n"));
        }
        script.push_str("  *) printf '' ;;\nesac\n");

        let path = self.dir.join("stub-asr.sh");
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        format!("sh {} {{wav}}", path.display())
    }

    fn case(&self, id: &str, reference: &str, benign: bool) -> AudioCase {
        AudioCase {
            id: id.to_string(),
            category: "negation".to_string(),
            reference: reference.to_string(),
            wav: Path::new("clips").join(format!("{id}.wav")),
            app: None,
            vocabulary: Vec::new(),
            benign,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn config() -> Config {
    Config::recommended().with_locales([Locale::En, Locale::Hinglish])
}

#[test]
fn a_dropped_negation_from_the_engine_is_measured() {
    let f = Fixture::new("dropped");
    f.clip("neg", 900);
    let asr = AsrCommand::new(
        f.engine(&[("neg", r#"{"text":"merge a change like this"}"#)]),
        Some("stub".into()),
    )
    .unwrap();

    let case = f.case("neg", "never merge a change like this", false);
    let result = audio::run_case(&case, &f.dir, &asr, None, None, false, &config()).unwrap();

    assert_eq!(result.heard, "merge a change like this");
    assert!(
        result.case.baseline.meaning_flip,
        "the engine changed the instruction"
    );
    assert_eq!(result.engine, "stub");
    assert!(result.clip_ms > 0);
}

#[test]
fn a_correct_transcript_scores_clean_and_passes() {
    let f = Fixture::new("clean");
    f.clip("ok", 700);
    let asr = AsrCommand::new(
        f.engine(&[("ok", r#"{"text":"ship it when CI passes"}"#)]),
        None,
    )
    .unwrap();

    let case = f.case("ok", "ship it when CI passes", true);
    let result = audio::run_case(&case, &f.dir, &asr, None, None, false, &config()).unwrap();

    assert_eq!(result.case.readback.cser, 0.0);
    assert_eq!(result.case.action, Action::Pass);
    assert!(!result.case.baseline.meaning_flip);
}

#[test]
fn vad_supplies_speech_regions_from_the_clip() {
    let f = Fixture::new("vad");
    f.clip("neg", 900);
    let asr = AsrCommand::new(f.engine(&[("neg", r#"{"text":"merge this"}"#)]), None).unwrap();
    let case = f.case("neg", "never merge this", false);

    let without = audio::run_case(&case, &f.dir, &asr, None, None, false, &config()).unwrap();
    let with = audio::run_case(&case, &f.dir, &asr, None, None, true, &config()).unwrap();

    assert_eq!(without.speech_regions, 0);
    assert!(
        with.speech_regions > 0,
        "the tone should register as speech"
    );
}

#[test]
fn a_cleanup_command_is_run_over_the_transcript() {
    let f = Fixture::new("cleanup");
    f.clip("neg", 800);
    let asr = AsrCommand::new(
        f.engine(&[("neg", r#"{"text":"we should not merge this"}"#)]),
        None,
    )
    .unwrap();
    // A cleanup step that helpfully deletes the negation, as they do.
    let cleanup =
        readback_bench::audio::CleanupCommand::new("printf '%s' {text} | sed 's/ not//'").unwrap();

    let case = f.case("neg", "we should not merge this", false);
    let result =
        audio::run_case(&case, &f.dir, &asr, Some(&cleanup), None, false, &config()).unwrap();

    assert_eq!(result.case.baseline_text, "we should merge this");
    assert!(
        result.case.readback_text.contains("not"),
        "the guard should have put it back"
    );
    assert_ne!(result.case.action, Action::Pass);
}

#[test]
fn a_missing_clip_is_reported_clearly() {
    let f = Fixture::new("missing");
    let asr = AsrCommand::new(f.engine(&[]), None).unwrap();
    let case = f.case("nope", "anything", false);

    let error = audio::run_case(&case, &f.dir, &asr, None, None, false, &config()).unwrap_err();
    assert!(error.to_string().contains("clip not found"), "{error}");
}

#[test]
fn a_failing_engine_is_reported_clearly() {
    let f = Fixture::new("failing");
    f.clip("neg", 500);
    let asr = AsrCommand::new("sh -c 'echo boom >&2; exit 1' {wav}", None).unwrap();
    let case = f.case("neg", "anything", false);

    let error = audio::run_case(&case, &f.dir, &asr, None, None, false, &config()).unwrap_err();
    assert!(error.to_string().contains("boom"), "{error}");
}

#[test]
fn results_aggregate_the_same_way_the_text_run_does() {
    let f = Fixture::new("aggregate");
    f.clip("neg", 900).clip("ok", 700);
    let asr = AsrCommand::new(
        f.engine(&[
            ("neg", r#"{"text":"merge this"}"#),
            ("ok", r#"{"text":"ship it when CI passes"}"#),
        ]),
        None,
    )
    .unwrap();

    let cases = [
        f.case("neg", "never merge this", false),
        f.case("ok", "ship it when CI passes", true),
    ];
    let results: Vec<_> = cases
        .iter()
        .map(|case| audio::run_case(case, &f.dir, &asr, None, None, false, &config()).unwrap())
        .collect();

    let scored: Vec<_> = results.iter().map(|r| r.case.clone()).collect();
    let summary = runner::summarise(&scored);

    assert_eq!(summary.cases, 2);
    assert_eq!(summary.controls, 1);
    assert!(summary.baseline_silent_flip_rate > 0.0);
    assert_eq!(
        summary.false_hold_rate, 0.0,
        "the control must not be blocked"
    );
    assert_eq!(audio::engine_flips(&results).len(), 1);
}
