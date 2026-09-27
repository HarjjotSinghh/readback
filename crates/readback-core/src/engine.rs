//! The pipeline itself: adapters in, one [`Verdict`] out.

use crate::config::Config;
use crate::guard::{GuardOutcome, check_cleanup};
use crate::lexicon::Lexicon;
use crate::omission;
use crate::redecode::{self, AudioSpan, Redecoder};
use crate::scorer::{RulesScorer, StakesScorer};
use crate::suspicion::assess;
use crate::tokenize::tokenize;
use crate::types::{AudioEvidence, Context, Flag, Provenance, Transcript, Verdict};

/// One utterance, with whatever evidence the host app has.
///
/// Only `raw` is required. A caller with no cleanup step and no confidence data
/// still gets stakes scoring and the protected lexicon.
#[derive(Debug, Clone, Default)]
pub struct CheckInput {
    /// The recogniser's output, via an adapter.
    pub raw: Transcript,
    /// The LLM-polished rewrite, when the host app runs one. Supplying this
    /// enables the Cleanup Guard, which is the cheapest win in the pipeline.
    pub cleaned: Option<String>,
    /// Voice-activity regions from the host, enabling omission detection. Needs
    /// word timings on `raw` to be useful.
    pub audio: Option<AudioEvidence>,
    /// Where the text is headed.
    pub context: Context,
}

impl CheckInput {
    pub fn new(raw: Transcript) -> Self {
        Self {
            raw,
            cleaned: None,
            audio: None,
            context: Context::default(),
        }
    }

    pub fn from_text(text: impl Into<String>) -> Self {
        Self::new(Transcript::from_text(text))
    }

    pub fn with_cleaned(mut self, cleaned: impl Into<String>) -> Self {
        self.cleaned = Some(cleaned.into());
        self
    }

    pub fn with_context(mut self, context: Context) -> Self {
        self.context = context;
        self
    }

    /// Supplies voice-activity regions so dropped words can be detected.
    pub fn with_audio(mut self, audio: AudioEvidence) -> Self {
        self.audio = Some(audio);
        self
    }
}

/// A configured pipeline. Cheap to clone the config, so build one and reuse it.
pub struct Readback {
    config: Config,
    lexicon: Lexicon,
    scorer: Box<dyn StakesScorer>,
    redecoder: Option<Box<dyn Redecoder>>,
}

impl Default for Readback {
    fn default() -> Self {
        Self::new()
    }
}

impl Readback {
    /// Default lexicon, default thresholds, rules scorer.
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    /// Recommended per-app routing: paranoid in terminals, relaxed in notes.
    pub fn recommended() -> Self {
        Self::with_config(Config::recommended())
    }

    pub fn with_config(config: Config) -> Self {
        let mut lexicon = Lexicon::new(&config.locales);
        lexicon.protect(&config.vocabulary);
        Self {
            config,
            lexicon,
            scorer: Box::new(RulesScorer::new()),
            redecoder: None,
        }
    }

    /// Swaps in another stakes scorer, such as a local Laya checkpoint.
    pub fn with_scorer(mut self, scorer: Box<dyn StakesScorer>) -> Self {
        self.scorer = scorer;
        self
    }

    /// Adds a second opinion on shaky slices of audio.
    ///
    /// This is the expensive path and it only runs on spans the cheap stages
    /// already flagged, so the common case stays untouched. It is also the only
    /// stage that can catch a confidently wrong word that reads perfectly.
    pub fn with_redecoder(mut self, redecoder: Box<dyn Redecoder>) -> Self {
        self.redecoder = Some(redecoder);
        self
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Folds the evidence into one number in `0.0..=1.0`.
    ///
    /// Stakes scale the evidence rather than adding to it: a shaky word in
    /// "lol sounds good" is not a problem, and the same word in "don't deploy
    /// to production" is. The floor keeps a critical flag dangerous even when
    /// the sentence around it looks harmless.
    fn risk(&self, suspicion: f32, semantic: f32, stakes: f32) -> f32 {
        let floor = self.config.risk.floor.clamp(0.0, 1.0);
        let scale = floor + (1.0 - floor) * stakes.clamp(0.0, 1.0);
        (suspicion.max(semantic) * scale).clamp(0.0, 1.0)
    }

    /// Runs the pipeline. Never fails: a verdict with no evidence is `Pass`.
    pub fn check(&self, input: CheckInput) -> Verdict {
        // Per-call vocabulary is merged on top of the configured lexicon.
        let lexicon = if input.context.vocabulary.is_empty() {
            None
        } else {
            let mut lex = self.lexicon.clone();
            lex.protect(&input.context.vocabulary);
            Some(lex)
        };
        let lexicon = lexicon.as_ref().unwrap_or(&self.lexicon);

        let guard = match input.cleaned.as_deref() {
            Some(cleaned) => check_cleanup(&input.raw.primary.text, cleaned, lexicon),
            None => GuardOutcome {
                text: input.raw.primary.text.clone(),
                flags: Vec::new(),
                reverted: false,
            },
        };

        let mut suspicion = assess(&input.raw, &guard.text, &self.config.suspicion, lexicon);

        let omission = input.audio.as_ref().map(|audio| {
            omission::detect(
                audio,
                &input.raw.primary.words,
                &guard.text,
                &self.config.omission,
            )
        });
        if let Some(found) = &omission {
            suspicion.score = suspicion.score.max(found.score);
        }

        // Re-decoding is the only stage that goes back to the audio, so it runs
        // last and only where the cheap stages already found something.
        let redecode = self.redecoder.as_ref().and_then(|decoder| {
            let words = &input.raw.primary.words;
            let padding = self.config.redecode.padding_ms;
            let slices: Vec<(AudioSpan, String, crate::types::Span)> = suspicion
                .candidates
                .iter()
                .filter_map(|candidate| {
                    let word = words.get(candidate.word)?;
                    let span = AudioSpan::new(word.start_ms?, word.end_ms?).padded(padding);

                    // Everything the padded slice actually contains, so the
                    // second decode is compared against the same audio rather
                    // than against one word out of the middle of it.
                    let original: Vec<&str> = words
                        .iter()
                        .filter(|w| match (w.start_ms, w.end_ms) {
                            (Some(s), Some(e)) => s < span.end_ms && e > span.start_ms,
                            _ => false,
                        })
                        .map(|w| w.text.as_str())
                        .collect();

                    Some((span, original.join(" "), candidate.span))
                })
                .collect();
            if slices.is_empty() {
                return None;
            }
            Some(redecode::run(
                decoder.as_ref(),
                &slices,
                lexicon,
                &self.config.redecode,
            ))
        });
        if let Some(found) = &redecode {
            for flag in &found.flags {
                suspicion.score = suspicion.score.max(flag.severity.weight());
            }
        }

        let tokens = tokenize(&guard.text);
        let stakes = self.scorer.score(&guard.text, &tokens, lexicon);

        let semantic = guard
            .flags
            .iter()
            .map(|f| f.severity.weight())
            .fold(0.0_f32, f32::max);
        let risk = self.risk(suspicion.score, semantic, stakes);

        let mut flags: Vec<Flag> = guard.flags;
        flags.extend(suspicion.flags);
        if let Some(found) = omission {
            flags.extend(found.flags);
        }
        let spans_redecoded = redecode.as_ref().map(|r| r.spans_decoded).unwrap_or(0);
        if let Some(found) = redecode {
            flags.extend(found.flags);
        }
        flags.sort_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then(a.span.start.cmp(&b.span.start))
        });

        let action = self
            .config
            .policy
            .for_app(input.context.app.as_deref())
            .decide(risk);

        Verdict {
            action,
            text: guard.text,
            flags,
            risk,
            stakes,
            suspicion: suspicion.score,
            provenance: Provenance {
                primary_provider: input.raw.primary.provider.clone(),
                cleanup_guard_ran: input.cleaned.is_some(),
                cleanup_reverted: guard.reverted,
                omission_check_ran: input.audio.is_some(),
                spans_redecoded,
                scorer: self.scorer.name().to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Action, AudioEvidence, FlagKind, Word};

    #[test]
    fn plain_text_with_no_evidence_passes() {
        let v = Readback::new().check(CheckInput::from_text("sounds good to me"));
        assert_eq!(v.action, Action::Pass);
        assert!(v.flags.is_empty());
        assert_eq!(v.risk, 0.0);
    }

    #[test]
    fn a_dropped_negation_is_held() {
        let v = Readback::new().check(
            CheckInput::from_text("never merge a change like this")
                .with_cleaned("Merge a change like this."),
        );
        assert_eq!(v.action, Action::Hold);
        assert!(v.text.to_lowercase().contains("never"));
        assert_eq!(v.flags[0].kind, FlagKind::DroppedNegation);
        assert!(v.provenance.cleanup_reverted);
    }

    #[test]
    fn a_shaky_word_in_small_talk_still_passes() {
        let raw = Transcript::from_words(vec![
            Word::new("lol").with_confidence(0.41),
            Word::new("sounds").with_confidence(0.98),
            Word::new("good").with_confidence(0.99),
        ]);
        let v = Readback::new().check(CheckInput::new(raw));
        assert_eq!(v.action, Action::Pass);
        assert!(v.stakes < 0.2);
        // The word is still reported — it is a content word, so it could be
        // wrong in a way that matters — but nothing here is worth acting on.
        assert!(
            !v.flags.is_empty(),
            "the word is still flagged, just not acted on"
        );
    }

    #[test]
    fn the_same_shakiness_escalates_in_a_terminal() {
        let raw = Transcript::from_words(vec![
            Word::new("never").with_confidence(0.41),
            Word::new("delete").with_confidence(0.98),
            Word::new("production").with_confidence(0.99),
        ]);
        let rb = Readback::recommended();
        let notes = rb.check(CheckInput::new(raw.clone()).with_context(Context::for_app("Notes")));
        let term = rb.check(CheckInput::new(raw).with_context(Context::for_app("Ghostty")));
        assert_eq!(term.action, Action::Hold);
        assert_ne!(notes.action, Action::Hold);
    }

    #[test]
    fn per_call_vocabulary_is_protected() {
        let v = Readback::new().check(
            CheckInput::from_text("ask Harpawan to check it")
                .with_cleaned("Ask Harpreet to check it.")
                .with_context(Context::default().with_vocabulary(["Harpawan"])),
        );
        assert!(v.text.contains("Harpawan"));
        assert_eq!(v.flags[0].kind, FlagKind::ChangedProtectedTerm);
    }

    #[test]
    fn a_vad_gap_is_reported_as_a_possible_omission() {
        // "never" was spoken between 0.5s and 0.9s and never transcribed.
        let raw = Transcript::from_words(vec![
            Word::new("merge").with_timing(1000, 1400),
            Word::new("production").with_timing(1400, 2000),
        ]);
        let v = Readback::recommended().check(
            CheckInput::new(raw)
                .with_audio(AudioEvidence::from_regions([(500, 2000)]))
                .with_context(Context::for_app("Ghostty")),
        );
        assert!(v.flags.iter().any(|f| f.kind == FlagKind::PossibleOmission));
        assert!(v.provenance.omission_check_ran);
        assert_ne!(v.action, Action::Pass);
    }

    #[test]
    fn without_audio_the_omission_stage_is_skipped() {
        let raw = Transcript::from_words(vec![Word::new("merge").with_timing(1000, 1400)]);
        let v = Readback::new().check(CheckInput::new(raw));
        assert!(!v.provenance.omission_check_ran);
        assert!(v.flags.is_empty());
    }

    #[test]
    fn a_second_decode_can_recover_a_word_the_text_cannot_reveal() {
        // The failure nothing else catches: "do merge that branch" is fluent,
        // unremarkable, and the opposite of what was said. Only the audio knows.
        struct HeardTheNegation;
        impl crate::redecode::Redecoder for HeardTheNegation {
            fn redecode(&self, _r: &crate::redecode::RedecodeRequest) -> Vec<String> {
                vec!["don't merge".to_string()]
            }
        }

        let raw = Transcript::from_words(vec![
            Word::new("do")
                .with_confidence(0.38)
                .with_timing(1000, 1300),
            Word::new("merge")
                .with_confidence(0.94)
                .with_timing(1300, 1700),
            Word::new("that")
                .with_confidence(0.97)
                .with_timing(1700, 1900),
            Word::new("branch")
                .with_confidence(0.95)
                .with_timing(1900, 2300),
        ]);

        let without = Readback::recommended().check(CheckInput::new(raw.clone()));
        let with = Readback::recommended()
            .with_redecoder(Box::new(HeardTheNegation))
            .check(CheckInput::new(raw));

        assert!(
            with.flags
                .iter()
                .any(|f| f.kind == FlagKind::RedecodeDisagreement),
            "the second decode should disagree"
        );
        assert_eq!(with.action, Action::Hold);
        assert!(
            with.risk > without.risk,
            "{} vs {}",
            with.risk,
            without.risk
        );
        assert_eq!(with.provenance.spans_redecoded, 1);
    }

    #[test]
    fn re_decoding_never_runs_without_timings() {
        struct Never;
        impl crate::redecode::Redecoder for Never {
            fn redecode(&self, _r: &crate::redecode::RedecodeRequest) -> Vec<String> {
                panic!("must not be called without word timings");
            }
        }
        let raw = Transcript::from_words(vec![Word::new("do").with_confidence(0.30)]);
        let v = Readback::recommended()
            .with_redecoder(Box::new(Never))
            .check(CheckInput::new(raw));
        assert_eq!(v.provenance.spans_redecoded, 0);
    }

    #[test]
    fn a_confident_utterance_never_reaches_the_expensive_path() {
        struct Never;
        impl crate::redecode::Redecoder for Never {
            fn redecode(&self, _r: &crate::redecode::RedecodeRequest) -> Vec<String> {
                panic!("the fast path must stay fast");
            }
        }
        let raw = Transcript::from_words(vec![
            Word::new("ship").with_confidence(0.99).with_timing(0, 300),
            Word::new("it").with_confidence(0.98).with_timing(300, 500),
        ]);
        let v = Readback::recommended()
            .with_redecoder(Box::new(Never))
            .check(CheckInput::new(raw));
        assert_eq!(v.action, Action::Pass);
        assert_eq!(v.provenance.spans_redecoded, 0);
    }

    #[test]
    fn harmless_polish_passes_through_untouched() {
        let v = Readback::new().check(
            CheckInput::from_text("um so we need to ship it tomorrow")
                .with_cleaned("We need to ship it tomorrow."),
        );
        assert_eq!(v.action, Action::Pass);
        assert_eq!(v.text, "We need to ship it tomorrow.");
    }
}
