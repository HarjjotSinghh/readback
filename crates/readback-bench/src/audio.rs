//! Running the benchmark against real audio.
//!
//! The text-level dataset measures how a reliability layer *responds* to an
//! error. It cannot measure how often an engine *makes* one, because the errors
//! are written by hand. This module closes that gap: point it at clips and an
//! ASR command, and the same CSER machinery scores what the engine actually
//! produced.
//!
//! Readback stays out of the audio business. The engine is an external command,
//! and the only thing read from the clip here is voice-activity energy, so that
//! omission detection has regions to work with.

use crate::metric::score;
use crate::runner::CaseResult;
use crate::{dataset, vad};
use anyhow::{Context as _, Result, bail};
use readback_core::config::Config;
use readback_core::lexicon::Lexicon;
use readback_core::redecode::{RedecodeRequest, Redecoder};
use readback_core::{Action, AudioEvidence, CheckInput, Context, Readback, Transcript, adapters};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One clip, and what was actually said in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioCase {
    pub id: String,
    pub category: String,
    /// The ground truth: what the speaker said.
    pub reference: String,
    /// Path to the clip, relative to the manifest.
    pub wav: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vocabulary: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub benign: bool,
}

/// Wraps a value in single quotes for `sh`, escaping any quote inside it.
///
/// Both substitutions go through this. A manifest is data — it can come from
/// another machine, another team, or a generator — so a `wav` path of
/// `a.wav; rm -rf ~` must end up as one argument rather than two commands. The
/// same goes for transcript text, which is whatever an engine decided to emit.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// An external command that turns a clip into a transcript.
///
/// `{wav}` in the template is replaced with the clip path. Whatever the command
/// writes to stdout is parsed by the normal adapters, so any engine that can
/// emit whisper.cpp, faster-whisper, Parakeet or `{"text": ...}` JSON works
/// without new code. Plain text works too, with no confidence data.
#[derive(Debug, Clone, PartialEq)]
pub struct AsrCommand {
    template: String,
    /// Reported in the results, so two runs can be told apart.
    pub label: String,
}

impl AsrCommand {
    pub fn new(template: impl Into<String>, label: Option<String>) -> Result<Self> {
        let template = template.into();
        if !template.contains("{wav}") {
            bail!("the ASR command must contain {{wav}}, which is replaced with the clip path");
        }
        let label = label.unwrap_or_else(|| {
            template
                .split_whitespace()
                .next()
                .unwrap_or("asr")
                .to_string()
        });
        Ok(Self { template, label })
    }

    fn run(&self, wav: &Path) -> Result<String> {
        let command = self
            .template
            .replace("{wav}", &shell_quote(&wav.display().to_string()));
        let output = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
            .with_context(|| format!("running {command:?}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Keep it to the decisive line; an engine's full log is not useful here.
            let detail = stderr
                .lines()
                .last()
                .unwrap_or("no stderr")
                .trim()
                .to_string();
            bail!("ASR command failed for {}: {detail}", wav.display());
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// An optional LLM polish step, so the Cleanup Guard can be measured too.
///
/// `{text}` is replaced with the raw transcript; stdout is the polished text.
#[derive(Debug, Clone, PartialEq)]
pub struct CleanupCommand {
    template: String,
}

impl CleanupCommand {
    pub fn new(template: impl Into<String>) -> Result<Self> {
        let template = template.into();
        if !template.contains("{text}") {
            bail!(
                "the cleanup command must contain {{text}}, which is replaced with the transcript"
            );
        }
        Ok(Self { template })
    }

    fn run(&self, text: &str) -> Result<String> {
        let command = self.template.replace("{text}", &shell_quote(text));
        let output = Command::new("sh").arg("-c").arg(&command).output()?;
        if !output.status.success() {
            bail!("cleanup command failed");
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// A second decode of one slice, run as an external command.
///
/// `{wav}` is the clip, `{start}` and `{duration}` are milliseconds. A decoder
/// that ignores them and re-runs the whole clip still works; it is just slower
/// and less likely to disagree usefully.
pub struct CommandRedecoder {
    template: String,
    wav: PathBuf,
    label: String,
}

impl CommandRedecoder {
    pub fn new(template: impl Into<String>, wav: PathBuf) -> Result<Self> {
        let template = template.into();
        if !template.contains("{wav}") {
            bail!("the re-decode command must contain {{wav}}");
        }
        Ok(Self {
            template,
            wav,
            label: "command".to_string(),
        })
    }
}

impl Redecoder for CommandRedecoder {
    fn redecode(&self, request: &RedecodeRequest) -> Vec<String> {
        let command = self
            .template
            .replace("{wav}", &shell_quote(&self.wav.display().to_string()))
            .replace("{start}", &request.span.start_ms.to_string())
            .replace("{duration}", &request.span.duration_ms().to_string())
            .replace("{end}", &request.span.end_ms.to_string());

        let Ok(output) = Command::new("sh").arg("-c").arg(&command).output() else {
            return Vec::new();
        };
        if !output.status.success() {
            // A second opinion that fails is simply no opinion; it must never
            // take the whole run down.
            return Vec::new();
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        match parse_transcript(&stdout) {
            // whisper-cli prints `[timestamps]  text` unless asked otherwise,
            // so strip anything in brackets before comparing.
            Ok(transcript) => {
                let text = strip_timestamps(&transcript.primary.text);
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![text]
                }
            }
            Err(_) => Vec::new(),
        }
    }

    fn name(&self) -> &str {
        &self.label
    }
}

/// Removes `[00:00:00.000 --> 00:00:01.200]` style prefixes.
fn strip_timestamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parses engine output, guessing the format from its shape.
fn parse_transcript(payload: &str) -> Result<Transcript> {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        bail!("the ASR command produced no output");
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Ok(adapters::generic::from_text(trimmed));
    };

    let parsed = if value.get("transcription").is_some() {
        adapters::whisper_cpp::from_json(trimmed)
    } else if value.get("segments").is_some() || value.is_array() {
        adapters::faster_whisper::from_json(trimmed)
    } else if value.get("hypotheses").is_some() {
        adapters::parakeet::from_json(trimmed)
    } else {
        adapters::generic::from_json(trimmed)
    };
    Ok(parsed?)
}

/// How a case was handled, on top of the shared scoring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioResult {
    #[serde(flatten)]
    pub case: CaseResult,
    /// What the speaker actually said.
    pub reference: String,
    /// What the engine produced before any cleanup.
    pub heard: String,
    pub engine: String,
    pub speech_regions: usize,
    pub clip_ms: u32,
    /// True when the engine reported per-word confidence.
    pub had_confidence: bool,
    /// True when the engine reported word timings, without which voice-activity
    /// regions cannot be turned into omission evidence.
    pub had_timings: bool,
    /// Slices decoded a second time for this clip.
    #[serde(default)]
    pub spans_redecoded: usize,
}

impl AudioResult {
    /// True when the engine transcribed the clip without a semantic error.
    ///
    /// On an audio run this, not the manifest's `benign` flag, is what makes a
    /// clip a control. `benign` describes whether the *hand-written* error in
    /// the text dataset was harmless; it says nothing about whether the engine
    /// erred on the same sentence. Judging noise by `benign` charges Readback
    /// for flagging transcripts the engine genuinely broke.
    pub fn engine_was_correct(&self) -> bool {
        self.case.baseline.cser == 0.0
    }

    /// Readback marked up a clip the engine got right. This is the number that
    /// actually predicts flag fatigue.
    pub fn is_noise(&self) -> bool {
        self.engine_was_correct() && self.case.action != Action::Pass
    }
}

/// Runs one clip end to end.
#[allow(clippy::too_many_arguments)]
pub fn run_case(
    case: &AudioCase,
    base: &Path,
    asr: &AsrCommand,
    cleanup: Option<&CleanupCommand>,
    redecode: Option<&str>,
    use_vad: bool,
    config: &Config,
) -> Result<AudioResult> {
    let wav = base.join(&case.wav);
    if !wav.exists() {
        bail!("clip not found: {}", wav.display());
    }

    let transcript = parse_transcript(&asr.run(&wav)?)
        .with_context(|| format!("parsing ASR output for {}", case.id))?;
    let heard = transcript.primary.text.clone();

    let polished = match cleanup {
        Some(command) => Some(command.run(&heard)?),
        None => None,
    };

    let mut config = config.clone();
    config.vocabulary.extend(case.vocabulary.iter().cloned());
    let mut lexicon = Lexicon::new(&config.locales);
    lexicon.protect(&config.vocabulary);

    let clip = vad::read_wav(&wav).map_err(anyhow::Error::msg)?;
    let speech = if use_vad {
        vad::detect(&clip, &vad::VadConfig::default())
    } else {
        Vec::new()
    };

    let mut input = CheckInput::new(transcript);
    input.cleaned = polished.clone();
    if !speech.is_empty() {
        input.audio = Some(AudioEvidence {
            speech: speech.clone(),
        });
    }
    if let Some(app) = &case.app {
        input.context = Context::for_app(app);
    }

    let words = &input.raw.primary.words;
    let had_confidence = words.iter().any(|w| w.confidence.is_some());
    let had_timings = words
        .iter()
        .any(|w| w.start_ms.is_some() && w.end_ms.is_some());

    let mut readback = Readback::with_config(config);
    if let Some(template) = redecode {
        readback = readback.with_redecoder(Box::new(CommandRedecoder::new(template, wav.clone())?));
    }
    let verdict = readback.check(input);
    let verdict_spans = verdict.provenance.spans_redecoded;
    let baseline_text = polished.unwrap_or_else(|| heard.clone());

    Ok(AudioResult {
        case: CaseResult {
            id: case.id.clone(),
            category: case.category.clone(),
            benign: case.benign,
            baseline: score(&case.reference, &baseline_text, &lexicon),
            baseline_text,
            readback: score(&case.reference, &verdict.text, &lexicon),
            readback_text: verdict.text,
            action: verdict.action,
            risk: verdict.risk,
            stakes: verdict.stakes,
        },
        reference: case.reference.clone(),
        heard,
        engine: asr.label.clone(),
        speech_regions: speech.len(),
        clip_ms: clip.duration_ms(),
        had_confidence,
        had_timings,
        spans_redecoded: verdict_spans,
    })
}

/// Loads a JSONL manifest of clips.
pub fn load_manifest(path: &Path) -> Result<Vec<AudioCase>> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with("//"))
        .map(|(i, line)| {
            serde_json::from_str::<AudioCase>(line)
                .with_context(|| format!("{}: line {}", path.display(), i + 1))
        })
        .collect()
}

/// Builds a manifest from the text dataset, naming a clip per case.
///
/// Used by the fixture script so the two datasets stay in step: every audio case
/// keeps the id, category and reference of the text case it came from.
pub fn manifest_from_dataset(clip_dir: &Path) -> Result<Vec<AudioCase>> {
    Ok(dataset::bundled()?
        .into_iter()
        .map(|case| AudioCase {
            wav: clip_dir.join(format!("{}.wav", case.id)),
            id: case.id,
            category: case.category,
            reference: case.reference,
            app: case.app,
            vocabulary: case.vocabulary,
            benign: case.benign,
        })
        .collect())
}

/// What evidence the engine actually handed over.
///
/// A run where this is all zeroes explains an otherwise baffling result: with
/// no confidence, no timings and no cleanup step to diff against, Readback has
/// nothing to reason about and will pass everything. That is the engine's
/// output format, not a verdict about the audio.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Coverage {
    pub clips: usize,
    /// Clips the engine transcribed without a semantic error.
    pub engine_correct: usize,
    /// Of those, how many Readback marked up anyway.
    pub noise_on_correct: usize,
    pub with_confidence: usize,
    pub with_timings: usize,
    pub with_speech_regions: usize,
    pub with_cleanup: usize,
}

impl Coverage {
    /// True when nothing at all was supplied for the acoustic stages.
    pub fn is_blind(&self) -> bool {
        self.with_confidence == 0 && self.with_timings == 0 && self.with_cleanup == 0
    }
}

pub fn coverage(results: &[AudioResult], had_cleanup: bool) -> Coverage {
    Coverage {
        clips: results.len(),
        engine_correct: results.iter().filter(|r| r.engine_was_correct()).count(),
        noise_on_correct: results.iter().filter(|r| r.is_noise()).count(),
        with_confidence: results.iter().filter(|r| r.had_confidence).count(),
        with_timings: results.iter().filter(|r| r.had_timings).count(),
        with_speech_regions: results.iter().filter(|r| r.speech_regions > 0).count(),
        with_cleanup: if had_cleanup { results.len() } else { 0 },
    }
}

/// One candidate highlight threshold, and what it would have done.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CalibrationPoint {
    /// Risk at or above this would be marked.
    pub highlight: f32,
    /// Of the engine's meaning flips, the share this threshold would mark.
    pub caught: f32,
    /// Of the clips the engine got right, the share this threshold would mark.
    pub noise: f32,
    pub marked: usize,
}

impl CalibrationPoint {
    /// How much catching is bought per unit of noise. Used to pick the knee.
    fn value(&self) -> f32 {
        self.caught - self.noise
    }
}

/// Sweeps the highlight threshold over a run and reports the trade-off.
///
/// The default thresholds were calibrated against the Cleanup Guard, whose
/// evidence is a concrete reverted span. Raw acoustic confidence from a weak
/// recogniser is a much noisier signal, and wants a different threshold. This
/// gives a run the numbers to choose one rather than guess.
///
/// Recomputing from risk is exact: an action is a pure function of risk and the
/// thresholds, so no re-run is needed.
pub fn calibrate(results: &[AudioResult]) -> Vec<CalibrationPoint> {
    let flips: Vec<&AudioResult> = results
        .iter()
        .filter(|r| r.case.baseline.meaning_flip)
        .collect();
    let correct: Vec<&AudioResult> = results.iter().filter(|r| r.engine_was_correct()).collect();

    (1..=19)
        .map(|step| {
            let highlight = step as f32 * 0.05;
            let marked =
                |set: &[&AudioResult]| set.iter().filter(|r| r.case.risk >= highlight).count();
            let rate = |hits: usize, total: usize| {
                if total == 0 {
                    0.0
                } else {
                    hits as f32 / total as f32
                }
            };
            CalibrationPoint {
                highlight,
                caught: rate(marked(&flips), flips.len()),
                noise: rate(marked(&correct), correct.len()),
                marked: results.iter().filter(|r| r.case.risk >= highlight).count(),
            }
        })
        .collect()
}

/// The threshold with the best catch-to-noise trade-off, preferring the higher
/// threshold when two are equally good, since a quieter tool survives longer.
pub fn best_threshold(points: &[CalibrationPoint]) -> Option<CalibrationPoint> {
    points.iter().copied().reduce(|best, point| {
        if point.value() >= best.value() {
            point
        } else {
            best
        }
    })
}

/// Cases where the engine itself changed the instruction, which is what an
/// audio run measures that a text run cannot.
pub fn engine_flips(results: &[AudioResult]) -> Vec<&AudioResult> {
    results
        .iter()
        .filter(|r| r.case.baseline.meaning_flip)
        .collect()
}

/// Cases a host would have pasted with no warning.
pub fn silent_flips(results: &[AudioResult]) -> Vec<&AudioResult> {
    results
        .iter()
        .filter(|r| r.case.readback.meaning_flip && r.case.action == Action::Pass)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_asr_command_must_name_the_clip() {
        assert!(AsrCommand::new("whisper --model tiny", None).is_err());
        assert!(AsrCommand::new("whisper {wav}", None).is_ok());
    }

    #[test]
    fn the_label_defaults_to_the_program_name() {
        let asr = AsrCommand::new("whisper-cli -f {wav}", None).unwrap();
        assert_eq!(asr.label, "whisper-cli");
        let named = AsrCommand::new("whisper-cli -f {wav}", Some("tiny.en".into())).unwrap();
        assert_eq!(named.label, "tiny.en");
    }

    #[test]
    fn a_cleanup_command_must_name_the_text() {
        assert!(CleanupCommand::new("llm polish").is_err());
        assert!(CleanupCommand::new("llm polish {text}").is_ok());
    }

    fn result_at(risk: f32, flip: bool, correct: bool) -> AudioResult {
        use crate::metric::Scores;
        use readback_core::Action;
        let scores = |cser: f32, flip: bool| Scores {
            wer: 0.0,
            cser,
            meaning_flip: flip,
            errors: Vec::new(),
        };
        AudioResult {
            case: CaseResult {
                id: "x".into(),
                category: "c".into(),
                benign: false,
                baseline_text: String::new(),
                baseline: scores(if correct { 0.0 } else { 0.5 }, flip),
                readback_text: String::new(),
                readback: scores(0.0, false),
                action: Action::Pass,
                risk,
                stakes: 0.0,
            },
            reference: String::new(),
            heard: String::new(),
            engine: "stub".into(),
            speech_regions: 0,
            clip_ms: 0,
            had_confidence: true,
            had_timings: true,
            spans_redecoded: 0,
        }
    }

    #[test]
    fn calibration_separates_catching_from_noise() {
        // Two flips at high risk, two clean clips at low risk: a threshold
        // between them catches everything and costs nothing.
        let results = [
            result_at(0.80, true, false),
            result_at(0.75, true, false),
            result_at(0.20, false, true),
            result_at(0.15, false, true),
        ];
        let best = best_threshold(&calibrate(&results)).unwrap();
        assert_eq!(best.caught, 1.0);
        assert_eq!(best.noise, 0.0);
        assert!(best.highlight > 0.20 && best.highlight <= 0.75, "{best:?}");
    }

    #[test]
    fn a_threshold_of_zero_marks_everything() {
        let results = [result_at(0.5, true, false), result_at(0.1, false, true)];
        let points = calibrate(&results);
        let lowest = points.first().unwrap();
        assert_eq!(lowest.marked, 2);
        assert_eq!(lowest.noise, 1.0);
    }

    #[test]
    fn calibration_copes_with_no_flips_at_all() {
        let results = [result_at(0.1, false, true)];
        let best = best_threshold(&calibrate(&results)).unwrap();
        assert_eq!(best.caught, 0.0);
        assert_eq!(best.noise, 0.0, "a high threshold should mark nothing");
    }

    #[test]
    fn timestamps_are_stripped_from_a_second_decode() {
        assert_eq!(
            strip_timestamps("[00:00:00.000 --> 00:00:01.200]   Don't merge."),
            "Don't merge."
        );
        assert_eq!(strip_timestamps("  plain text  "), "plain text");
    }

    #[test]
    fn a_redecode_command_must_name_the_clip() {
        let wav = PathBuf::from("clip.wav");
        assert!(CommandRedecoder::new("whisper --offset {start}", wav.clone()).is_err());
        assert!(CommandRedecoder::new("whisper {wav} --offset {start}", wav).is_ok());
    }

    #[test]
    fn a_failing_second_decode_is_simply_no_opinion() {
        let decoder =
            CommandRedecoder::new("sh -c 'exit 1' {wav}", PathBuf::from("clip.wav")).unwrap();
        let request = RedecodeRequest {
            span: readback_core::AudioSpan::new(0, 500),
            original: "do".into(),
        };
        assert!(decoder.redecode(&request).is_empty());
    }

    #[test]
    fn shell_metacharacters_in_a_path_are_neutralised() {
        // A manifest is data and may come from anywhere.
        let quoted = shell_quote("clips/a.wav; rm -rf ~");
        assert_eq!(quoted, "'clips/a.wav; rm -rf ~'");
    }

    /// Round-trips a value through a real shell, which is the only check that
    /// proves the quoting rather than describing it.
    fn echoed_by_sh(value: &str) -> String {
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", shell_quote(value)))
            .output()
            .expect("sh should run");
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    #[test]
    fn an_embedded_quote_cannot_break_out() {
        for hostile in [
            "it's",
            "x'; touch /tmp/readback-pwned; echo '",
            "$(touch /tmp/readback-pwned)",
            "`touch /tmp/readback-pwned`",
            "a.wav; rm -rf ~",
            "a.wav && echo nope",
        ] {
            assert_eq!(
                echoed_by_sh(hostile),
                hostile,
                "quoting failed for {hostile:?}"
            );
        }
        assert!(
            !std::path::Path::new("/tmp/readback-pwned").exists(),
            "a test payload executed; the quoting is not holding"
        );
    }

    #[test]
    fn transcripts_are_parsed_by_shape() {
        let whisper_cpp = r#"{"transcription":[{"text":" merge this","tokens":[]}]}"#;
        assert_eq!(
            parse_transcript(whisper_cpp).unwrap().primary.text,
            "merge this"
        );

        let faster = r#"{"segments":[{"text":" merge this","words":[]}]}"#;
        assert_eq!(parse_transcript(faster).unwrap().primary.text, "merge this");

        let parakeet = r#"{"hypotheses":[{"text":"merge this","score":0.9}]}"#;
        assert_eq!(
            parse_transcript(parakeet).unwrap().primary.text,
            "merge this"
        );

        let generic = r#"{"text":"merge this"}"#;
        assert_eq!(
            parse_transcript(generic).unwrap().primary.text,
            "merge this"
        );
    }

    #[test]
    fn plain_text_output_is_accepted() {
        let t = parse_transcript("  merge this  ").unwrap();
        assert_eq!(t.primary.text, "merge this");
        assert!(
            t.primary.words.is_empty(),
            "no confidence data, and that is fine"
        );
    }

    #[test]
    fn empty_output_is_an_error() {
        assert!(parse_transcript("   ").is_err());
    }

    #[test]
    fn the_manifest_tracks_the_text_dataset() {
        let manifest = manifest_from_dataset(Path::new("clips")).unwrap();
        assert!(manifest.len() >= 60);
        let first = manifest.iter().find(|c| c.id == "neg-001").unwrap();
        assert_eq!(first.wav, Path::new("clips/neg-001.wav"));
        assert_eq!(first.reference, "never merge a change like this");
    }
}
