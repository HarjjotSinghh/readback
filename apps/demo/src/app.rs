//! The commands the window calls, kept free of Tauri types so they can be
//! tested without a running app.

use readback_bench::dataset;
use readback_core::config::Config;
use readback_core::lexicon::Locale;
use readback_core::{Action, AudioEvidence, CheckInput, Context, Readback, Transcript, Word};
use serde::{Deserialize, Serialize};

/// One thing the user can try, drawn from CriticalSpeechBench so the demo and
/// the benchmark cannot drift apart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub category: String,
    pub spoken: String,
    pub heard: String,
    pub polished: Option<String>,
    pub app: Option<String>,
    pub vocabulary: Vec<String>,
    /// True when nothing went wrong; the demo should stay quiet on these.
    pub benign: bool,
}

/// What the window sends back when the user edits the fields by hand.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Request {
    pub heard: String,
    #[serde(default)]
    pub polished: Option<String>,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub vocabulary: Vec<String>,
    #[serde(default)]
    pub locales: Vec<String>,
    /// Per-word confidence, when the scenario carries it.
    #[serde(default)]
    pub words: Vec<WordInput>,
    /// Speech regions, from a loaded clip or a scenario.
    #[serde(default)]
    pub speech: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WordInput {
    pub text: String,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub start_ms: Option<u32>,
    #[serde(default)]
    pub end_ms: Option<u32>,
}

/// A flag, flattened for the window.
#[derive(Debug, Clone, Serialize)]
pub struct FlagView {
    pub kind: String,
    pub severity: String,
    pub start: usize,
    pub end: usize,
    pub evidence: String,
}

/// Everything the overlay needs to draw itself.
#[derive(Debug, Clone, Serialize)]
pub struct VerdictView {
    pub action: String,
    pub text: String,
    pub flags: Vec<FlagView>,
    pub risk: f32,
    pub stakes: f32,
    pub suspicion: f32,
    pub cleanup_reverted: bool,
    pub omission_check_ran: bool,
    /// What the stack would have pasted without Readback, for the comparison.
    pub baseline: String,
}

fn parse_locales(names: &[String]) -> Vec<Locale> {
    let parsed: Vec<Locale> = names
        .iter()
        .filter_map(|name| match name.to_lowercase().as_str() {
            "en" | "english" => Some(Locale::En),
            "hinglish" | "hi" => Some(Locale::Hinglish),
            _ => None,
        })
        .collect();
    if parsed.is_empty() { vec![Locale::En] } else { parsed }
}

/// Runs one request through the pipeline.
pub fn check(request: &Request) -> VerdictView {
    let config = Config::recommended()
        .with_locales(parse_locales(&request.locales))
        .with_vocabulary(request.vocabulary.clone());

    let mut transcript = Transcript::from_text(&request.heard);
    transcript.primary.words = request
        .words
        .iter()
        .map(|w| Word {
            text: w.text.clone(),
            start_ms: w.start_ms,
            end_ms: w.end_ms,
            confidence: w.confidence,
        })
        .collect();

    let mut input = CheckInput::new(transcript);
    input.cleaned = request.polished.clone().filter(|p| !p.trim().is_empty());
    if !request.speech.is_empty() {
        input.audio = Some(AudioEvidence::from_regions(request.speech.iter().copied()));
    }
    if let Some(app) = &request.app {
        input.context = Context::for_app(app);
    }

    let baseline = input.cleaned.clone().unwrap_or_else(|| request.heard.clone());
    let verdict = Readback::with_config(config).check(input);

    VerdictView {
        action: match verdict.action {
            Action::Pass => "pass",
            Action::Highlight => "highlight",
            Action::Hold => "hold",
        }
        .to_string(),
        flags: verdict
            .flags
            .iter()
            .map(|f| FlagView {
                kind: format!("{:?}", f.kind),
                severity: format!("{:?}", f.severity).to_lowercase(),
                start: f.span.start,
                end: f.span.end,
                evidence: f.evidence.clone(),
            })
            .collect(),
        text: verdict.text,
        risk: verdict.risk,
        stakes: verdict.stakes,
        suspicion: verdict.suspicion,
        cleanup_reverted: verdict.provenance.cleanup_reverted,
        omission_check_ran: verdict.provenance.omission_check_ran,
        baseline,
    }
}

/// Every case in CriticalSpeechBench, offered as something to try.
pub fn scenarios() -> Vec<Scenario> {
    dataset::bundled()
        .unwrap_or_default()
        .into_iter()
        .map(|case| Scenario {
            id: case.id,
            category: case.category,
            spoken: case.reference,
            heard: case.raw,
            polished: case.cleaned,
            app: case.app,
            vocabulary: case.vocabulary,
            benign: case.benign,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(heard: &str, polished: Option<&str>) -> Request {
        Request {
            heard: heard.to_string(),
            polished: polished.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn a_dropped_negation_holds_and_is_restored() {
        let view = check(&request("never merge a change like this", Some("Merge a change like this.")));
        assert_eq!(view.action, "hold");
        assert!(view.text.to_lowercase().contains("never"));
        assert!(view.cleanup_reverted);
        assert_eq!(view.baseline, "Merge a change like this.");
    }

    #[test]
    fn harmless_polish_passes() {
        let view = check(&request("um so we need to ship it", Some("We need to ship it.")));
        assert_eq!(view.action, "pass");
        assert!(view.flags.is_empty());
    }

    #[test]
    fn an_empty_polish_field_is_ignored() {
        let view = check(&request("never merge this", Some("   ")));
        assert_eq!(view.action, "pass", "with nothing to compare against there is no evidence");
    }

    #[test]
    fn speech_regions_turn_on_omission_detection() {
        let mut req = request("merge production", None);
        req.words = vec![
            WordInput { text: "merge".into(), confidence: Some(0.96), start_ms: Some(1000), end_ms: Some(1400) },
            WordInput { text: "production".into(), confidence: Some(0.95), start_ms: Some(1400), end_ms: Some(2000) },
        ];
        req.speech = vec![(500, 2000)];
        req.app = Some("Ghostty".into());

        let view = check(&req);
        assert!(view.omission_check_ran);
        assert!(view.flags.iter().any(|f| f.kind == "PossibleOmission"));
    }

    #[test]
    fn hinglish_is_opt_in() {
        let mut req = request("abhi mat bhejo", Some("Bhejo abhi."));
        assert_eq!(check(&req).action, "pass");
        req.locales = vec!["en".into(), "hinglish".into()];
        assert_ne!(check(&req).action, "pass");
    }

    #[test]
    fn the_scenarios_come_from_the_benchmark() {
        let scenarios = scenarios();
        assert!(scenarios.len() >= 40);
        assert!(scenarios.iter().any(|s| s.id == "neg-001"));
        assert!(scenarios.iter().any(|s| s.benign), "controls must be offered too");
    }
}
