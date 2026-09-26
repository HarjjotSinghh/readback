//! Core data types shared across the Readback pipeline.

use serde::{Deserialize, Serialize};

/// A single recognised word, as reported by an ASR engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u32>,
    /// Engine-reported probability for this word, in `0.0..=1.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl Word {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            start_ms: None,
            end_ms: None,
            confidence: None,
        }
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = Some(confidence);
        self
    }

    pub fn with_timing(mut self, start_ms: u32, end_ms: u32) -> Self {
        self.start_ms = Some(start_ms);
        self.end_ms = Some(end_ms);
        self
    }
}

/// One candidate transcription of an utterance.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Hypothesis {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Overall sequence score, normalised to `0.0..=1.0` where the engine allows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<Word>,
}

impl Hypothesis {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    /// Builds a hypothesis from words, joining them with single spaces.
    pub fn from_words(words: Vec<Word>) -> Self {
        let text = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            text,
            words,
            ..Default::default()
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }
}

/// Engine-level signals that indicate the decoder itself was struggling.
///
/// These are the Whisper-family hallucination tells: text produced during
/// near-silence, or a suspiciously compressible (repetitive) output.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct DecodeSignals {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_logprob: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_speech_prob: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression_ratio: Option<f32>,
}

/// A normalised ASR result: the engine's best guess plus whatever else it knows.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Transcript {
    pub primary: Hypothesis,
    /// N-best candidates or results from other engines, best first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Hypothesis>,
    #[serde(default)]
    pub signals: DecodeSignals,
}

impl Transcript {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            primary: Hypothesis::from_text(text),
            ..Default::default()
        }
    }

    pub fn from_words(words: Vec<Word>) -> Self {
        Self {
            primary: Hypothesis::from_words(words),
            ..Default::default()
        }
    }
}

/// What the host application knows about where this text is headed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Context {
    /// Frontmost application name, matched against policy rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Terms the user cares about: names, projects, environments, jargon.
    /// Edits touching these are treated as protected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vocabulary: Vec<String>,
}

impl Context {
    pub fn for_app(app: impl Into<String>) -> Self {
        Self {
            app: Some(app.into()),
            vocabulary: Vec::new(),
        }
    }

    pub fn with_vocabulary<I, S>(mut self, terms: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.vocabulary.extend(terms.into_iter().map(Into::into));
        self
    }
}

/// What the host application should do with the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Insert immediately. This is the common case and should stay common.
    Pass,
    /// Insert, but mark the flagged spans so the user's eye lands on them.
    Highlight,
    /// Do not insert or auto-send until the user confirms.
    Hold,
}

/// How much damage this change does if it went the wrong way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Punctuation, filler, articles. Cosmetic.
    None,
    /// Lexical swap that preserves meaning.
    Low,
    /// Named entities and general factual detail.
    Medium,
    /// Dates, numbers, quantities, modality.
    High,
    /// Negation, polarity, direction, destructive instructions.
    Critical,
}

impl Severity {
    /// Weight used when folding flags into a single risk number.
    pub fn weight(self) -> f32 {
        match self {
            Severity::None => 0.0,
            Severity::Low => 0.15,
            Severity::Medium => 0.4,
            Severity::High => 0.7,
            Severity::Critical => 1.0,
        }
    }
}

/// Why a span was flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagKind {
    /// Cleanup deleted a negation that the raw transcript contained.
    DroppedNegation,
    /// Cleanup swapped one negation-bearing token for another.
    AlteredNegation,
    /// A temporal anchor (before/after/until) changed or vanished.
    ChangedTemporal,
    /// A number, quantity or currency amount changed.
    ChangedNumber,
    /// A direction verb flipped: enable/disable, merge/revert, allow/deny.
    ChangedDirection,
    /// A quantifier changed: all/none, always/never, only.
    ChangedQuantifier,
    /// Modality changed: must/might, should/could.
    ChangedModality,
    /// A deployment environment changed: production/staging/dev.
    ChangedEnvironment,
    /// A user-dictionary term or proper noun was removed or rewritten.
    ChangedProtectedTerm,
    /// The ASR itself reported low confidence for this word.
    LowConfidence,
    /// Decoder-level hallucination tell (silence, repetition, low logprob).
    HallucinationSignal,
}

impl FlagKind {
    pub fn severity(self) -> Severity {
        match self {
            FlagKind::DroppedNegation
            | FlagKind::AlteredNegation
            | FlagKind::ChangedDirection
            | FlagKind::ChangedEnvironment => Severity::Critical,
            FlagKind::ChangedNumber
            | FlagKind::ChangedTemporal
            | FlagKind::ChangedQuantifier
            | FlagKind::ChangedModality => Severity::High,
            FlagKind::ChangedProtectedTerm => Severity::Medium,
            FlagKind::LowConfidence | FlagKind::HallucinationSignal => Severity::Low,
        }
    }

    /// True when the flag describes the cleanup step mutating meaning, as
    /// opposed to the acoustic model being unsure.
    pub fn is_semantic(self) -> bool {
        !matches!(
            self,
            FlagKind::LowConfidence | FlagKind::HallucinationSignal
        )
    }
}

/// A byte range into [`Verdict::text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn slice<'a>(&self, text: &'a str) -> &'a str {
        text.get(self.start..self.end).unwrap_or_default()
    }
}

/// One thing Readback wants the user to look at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flag {
    pub kind: FlagKind,
    pub severity: Severity,
    pub span: Span,
    /// Human-readable explanation, safe to show in a tooltip.
    pub evidence: String,
    /// What Readback believes the text should have said, when it knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Which stages actually ran, so integrators can debug a verdict.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_provider: Option<String>,
    pub cleanup_guard_ran: bool,
    pub cleanup_reverted: bool,
    pub scorer: String,
}

/// The answer Readback exists to give.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub action: Action,
    /// The text to insert, with any reverted spans already restored.
    pub text: String,
    pub flags: Vec<Flag>,
    /// Combined risk in `0.0..=1.0`. Drives [`Action`] via policy thresholds.
    pub risk: f32,
    /// How much a wrong word would cost here, in `0.0..=1.0`.
    pub stakes: f32,
    /// How shaky the recognition looks, in `0.0..=1.0`.
    pub suspicion: f32,
    pub provenance: Provenance,
}

impl Verdict {
    /// Flags at or above the given severity, worst first.
    pub fn flags_at_least(&self, severity: Severity) -> Vec<&Flag> {
        let mut found: Vec<&Flag> = self
            .flags
            .iter()
            .filter(|f| f.severity >= severity)
            .collect();
        found.sort_by_key(|f| std::cmp::Reverse(f.severity));
        found
    }

    pub fn is_pass(&self) -> bool {
        self.action == Action::Pass
    }
}
