//! Node.js binding for Readback.
//!
//! The JavaScript surface mirrors the Rust one: build a `Readback`, hand it a
//! transcript and whatever else you have, get back a verdict. Field names are
//! camelCase on the JS side, which napi handles automatically.

#![deny(clippy::all)]

use napi_derive::napi;
use readback_core::config::Config;
use readback_core::lexicon::Locale;
use readback_core::policy::{AppPolicy, AppRule, Policy};
use readback_core::{
    Action, AudioEvidence, CheckInput, Context, Flag, Hypothesis, Severity, SpeechRegion,
    Transcript, Verdict, Word,
};

/// One recognised word, with whatever the engine reported about it.
#[napi(object)]
pub struct JsWord {
    pub text: String,
    pub start_ms: Option<u32>,
    pub end_ms: Option<u32>,
    /// `0..1`. Omit when the engine does not report confidence.
    pub confidence: Option<f64>,
}

impl From<JsWord> for Word {
    fn from(w: JsWord) -> Self {
        Word {
            text: w.text,
            start_ms: w.start_ms,
            end_ms: w.end_ms,
            confidence: w.confidence.map(|c| c as f32),
        }
    }
}

/// A stretch of audio that voice-activity detection called speech.
#[napi(object)]
pub struct JsSpeechRegion {
    pub start_ms: u32,
    pub end_ms: u32,
}

/// Decoder-level signals, when the engine exposes them.
#[napi(object)]
pub struct JsSignals {
    pub avg_logprob: Option<f64>,
    pub no_speech_prob: Option<f64>,
    pub compression_ratio: Option<f64>,
}

/// Everything the host application knows about one utterance.
///
/// Only `text` is required. Each further field turns on another stage, so
/// adoption can be incremental.
#[napi(object)]
pub struct JsCheckInput {
    /// The recogniser's output.
    pub text: String,
    /// Per-word confidence and timings.
    pub words: Option<Vec<JsWord>>,
    /// Competing transcriptions, from N-best or a second engine.
    pub alternatives: Option<Vec<String>>,
    /// Decoder-level signals.
    pub signals: Option<JsSignals>,
    /// The LLM-polished rewrite. Supplying this turns on the Cleanup Guard.
    pub cleaned: Option<String>,
    /// Voice-activity regions. Needs `words` with timings to be useful.
    pub speech: Option<Vec<JsSpeechRegion>>,
    /// Frontmost application, used to select a policy.
    pub app: Option<String>,
    /// Terms to protect for this call only.
    pub vocabulary: Option<Vec<String>>,
}

/// Thresholds for one destination app.
#[napi(object)]
pub struct JsAppPolicy {
    /// Pipe-separated app names, matched case-insensitively: `"Terminal|Cursor"`.
    pub pattern: String,
    /// Risk at or above this holds. Use a value above 1 to never hold.
    pub hold: f64,
    /// Risk at or above this highlights.
    pub highlight: f64,
}

/// Constructor options.
#[napi(object)]
pub struct JsOptions {
    /// `"en"`, `"hinglish"`. Defaults to `["en"]`.
    pub locales: Option<Vec<String>>,
    /// Always-protected terms: names, projects, environments, jargon.
    pub vocabulary: Option<Vec<String>>,
    /// Use the built-in per-app routing: paranoid in terminals, relaxed in notes.
    pub recommended: Option<bool>,
    /// Per-app overrides, checked in order. First match wins.
    pub apps: Option<Vec<JsAppPolicy>>,
    /// Default hold threshold for apps with no rule.
    pub hold: Option<f64>,
    /// Default highlight threshold for apps with no rule.
    pub highlight: Option<f64>,
}

/// One thing Readback wants the user to look at.
#[napi(object)]
pub struct JsFlag {
    /// `"dropped_negation"`, `"changed_environment"`, `"low_confidence"`, …
    pub kind: String,
    /// `"critical"`, `"high"`, `"medium"`, `"low"` or `"none"`.
    pub severity: String,
    /// Byte offset into `text`, inclusive.
    pub start: u32,
    /// Byte offset into `text`, exclusive.
    pub end: u32,
    /// Safe to show in a tooltip.
    pub evidence: String,
    pub suggestion: Option<String>,
}

/// Which stages ran, for debugging a verdict.
#[napi(object)]
pub struct JsProvenance {
    pub primary_provider: Option<String>,
    pub cleanup_guard_ran: bool,
    pub cleanup_reverted: bool,
    pub omission_check_ran: bool,
    pub scorer: String,
}

/// The answer.
#[napi(object)]
pub struct JsVerdict {
    /// `"pass"`, `"highlight"` or `"hold"`.
    pub action: String,
    /// The text to insert, with any reverted spans already restored.
    pub text: String,
    pub flags: Vec<JsFlag>,
    pub risk: f64,
    pub stakes: f64,
    pub suspicion: f64,
    pub provenance: JsProvenance,
}

/// What the Cleanup Guard did on its own.
#[napi(object)]
pub struct JsGuardOutcome {
    pub text: String,
    pub reverted: bool,
    pub flags: Vec<JsFlag>,
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::High => "high",
        Severity::Medium => "medium",
        Severity::Low => "low",
        Severity::None => "none",
    }
}

fn action_name(action: Action) -> &'static str {
    match action {
        Action::Pass => "pass",
        Action::Highlight => "highlight",
        Action::Hold => "hold",
    }
}

/// Uses the serde name so the string matches the JSON and CLI output exactly.
fn flag_kind_name(kind: readback_core::FlagKind) -> String {
    match kind {
        readback_core::FlagKind::DroppedNegation => "dropped_negation",
        readback_core::FlagKind::AlteredNegation => "altered_negation",
        readback_core::FlagKind::ChangedTemporal => "changed_temporal",
        readback_core::FlagKind::ChangedNumber => "changed_number",
        readback_core::FlagKind::ChangedDirection => "changed_direction",
        readback_core::FlagKind::ChangedQuantifier => "changed_quantifier",
        readback_core::FlagKind::ChangedModality => "changed_modality",
        readback_core::FlagKind::ChangedEnvironment => "changed_environment",
        readback_core::FlagKind::ChangedProtectedTerm => "changed_protected_term",
        readback_core::FlagKind::LowConfidence => "low_confidence",
        readback_core::FlagKind::HallucinationSignal => "hallucination_signal",
        readback_core::FlagKind::PossibleOmission => "possible_omission",
    }
    .to_string()
}

fn flag_to_js(flag: &Flag) -> JsFlag {
    JsFlag {
        kind: flag_kind_name(flag.kind),
        severity: severity_name(flag.severity).to_string(),
        start: flag.span.start as u32,
        end: flag.span.end as u32,
        evidence: flag.evidence.clone(),
        suggestion: flag.suggestion.clone(),
    }
}

fn verdict_to_js(verdict: Verdict) -> JsVerdict {
    JsVerdict {
        action: action_name(verdict.action).to_string(),
        flags: verdict.flags.iter().map(flag_to_js).collect(),
        text: verdict.text,
        risk: verdict.risk as f64,
        stakes: verdict.stakes as f64,
        suspicion: verdict.suspicion as f64,
        provenance: JsProvenance {
            primary_provider: verdict.provenance.primary_provider,
            cleanup_guard_ran: verdict.provenance.cleanup_guard_ran,
            cleanup_reverted: verdict.provenance.cleanup_reverted,
            omission_check_ran: verdict.provenance.omission_check_ran,
            scorer: verdict.provenance.scorer,
        },
    }
}

fn parse_locale(name: &str) -> napi::Result<Locale> {
    match name.to_lowercase().as_str() {
        "en" | "english" => Ok(Locale::En),
        "hinglish" | "hi" => Ok(Locale::Hinglish),
        other => Err(napi::Error::from_reason(format!(
            "unknown locale {other:?}; expected \"en\" or \"hinglish\""
        ))),
    }
}

fn build_config(options: Option<JsOptions>) -> napi::Result<Config> {
    let Some(options) = options else {
        return Ok(Config::default());
    };

    let mut config = if options.recommended.unwrap_or(false) {
        Config::recommended()
    } else {
        Config::default()
    };

    if let Some(locales) = options.locales {
        config.locales = locales
            .iter()
            .map(|l| parse_locale(l))
            .collect::<napi::Result<Vec<_>>>()?;
    }
    if let Some(vocabulary) = options.vocabulary {
        config.vocabulary.extend(vocabulary);
    }
    if let Some(hold) = options.hold {
        config.policy.default.hold = hold as f32;
    }
    if let Some(highlight) = options.highlight {
        config.policy.default.highlight = highlight as f32;
    }
    if let Some(apps) = options.apps {
        // Explicit rules come first, so they win over the recommended set.
        let mut rules: Vec<AppRule> = apps
            .into_iter()
            .map(|app| {
                AppRule::new(
                    app.pattern,
                    AppPolicy {
                        hold: app.hold as f32,
                        highlight: app.highlight as f32,
                    },
                )
            })
            .collect();
        rules.append(&mut config.policy.apps);
        config.policy = Policy {
            default: config.policy.default,
            apps: rules,
        };
    }

    Ok(config)
}

fn build_input(input: JsCheckInput) -> CheckInput {
    let mut transcript = Transcript::from_text(&input.text);
    if let Some(words) = input.words {
        transcript.primary.words = words.into_iter().map(Into::into).collect();
    }
    if let Some(alternatives) = input.alternatives {
        transcript.alternatives = alternatives
            .into_iter()
            .map(Hypothesis::from_text)
            .collect();
    }
    if let Some(signals) = input.signals {
        transcript.signals.avg_logprob = signals.avg_logprob.map(|v| v as f32);
        transcript.signals.no_speech_prob = signals.no_speech_prob.map(|v| v as f32);
        transcript.signals.compression_ratio = signals.compression_ratio.map(|v| v as f32);
    }

    let mut check = CheckInput::new(transcript);
    check.cleaned = input.cleaned;
    if let Some(speech) = input.speech {
        check.audio = Some(AudioEvidence {
            speech: speech
                .into_iter()
                .map(|r| SpeechRegion::new(r.start_ms, r.end_ms))
                .collect(),
        });
    }
    check.context = Context {
        app: input.app,
        vocabulary: input.vocabulary.unwrap_or_default(),
    };
    check
}

/// A configured pipeline. Build one and reuse it.
#[napi]
pub struct Readback {
    inner: readback_core::Readback,
}

#[napi]
impl Readback {
    #[napi(constructor)]
    pub fn new(options: Option<JsOptions>) -> napi::Result<Self> {
        Ok(Self {
            inner: readback_core::Readback::with_config(build_config(options)?),
        })
    }

    /// Runs the pipeline. Never throws for ordinary input.
    #[napi]
    pub fn check(&self, input: JsCheckInput) -> JsVerdict {
        verdict_to_js(self.inner.check(build_input(input)))
    }
}

/// Runs the Cleanup Guard alone: revert any edit that removed a protected
/// token, keep the rest of the polish.
///
/// Useful when you want to sanitise an LLM rewrite without the rest of the
/// pipeline.
#[napi]
pub fn check_cleanup(
    raw: String,
    cleaned: String,
    options: Option<JsOptions>,
) -> napi::Result<JsGuardOutcome> {
    let config = build_config(options)?;
    let mut lexicon = readback_core::Lexicon::new(&config.locales);
    lexicon.protect(&config.vocabulary);

    let outcome = readback_core::check_cleanup(&raw, &cleaned, &lexicon);
    Ok(JsGuardOutcome {
        flags: outcome.flags.iter().map(flag_to_js).collect(),
        text: outcome.text,
        reverted: outcome.reverted,
    })
}

/// The crate version, so hosts can report what they loaded.
#[napi]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
