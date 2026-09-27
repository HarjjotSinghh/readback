//! Running a dataset through Readback and scoring both sides.

use crate::dataset::Case;
use crate::metric::{Scores, score};
use anyhow::Result;
use readback_core::config::Config;
use readback_core::lexicon::Lexicon;
use readback_core::{Action, AudioEvidence, CheckInput, Context, Readback, Transcript};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    pub id: String,
    pub category: String,
    pub benign: bool,
    /// What the stack would have pasted without Readback.
    pub baseline_text: String,
    pub baseline: Scores,
    /// What Readback produced, and what it told the host to do with it.
    pub readback_text: String,
    pub readback: Scores,
    pub action: Action,
    pub risk: f32,
    pub stakes: f32,
}

impl CaseResult {
    /// A meaning-changing error that reached the user with no warning. This is
    /// the number the whole project exists to reduce.
    pub fn silent_flip(&self) -> bool {
        self.readback.meaning_flip && self.action == Action::Pass
    }

    /// The baseline always pastes, so any flip in it is silent by definition.
    pub fn baseline_silent_flip(&self) -> bool {
        self.baseline.meaning_flip
    }

    /// A control that got blocked. Too many of these and people uninstall.
    pub fn false_hold(&self) -> bool {
        self.benign && self.action == Action::Hold
    }

    /// A control that got marked up unnecessarily. Cheaper than a false hold,
    /// but still noise.
    pub fn false_highlight(&self) -> bool {
        self.benign && self.action == Action::Highlight
    }
}

fn transcript_for(case: &Case) -> Transcript {
    let mut transcript = Transcript::from_text(&case.raw);
    if let Some(words) = &case.words {
        transcript.primary.words = words.clone();
    }
    transcript
}

/// Runs one case and scores both the baseline and the Readback output.
pub fn run_case(case: &Case, config: &Config) -> CaseResult {
    let mut config = config.clone();
    config.vocabulary.extend(case.vocabulary.iter().cloned());

    let mut lexicon = Lexicon::new(&config.locales);
    lexicon.protect(&config.vocabulary);

    let mut input = CheckInput::new(transcript_for(case));
    input.cleaned = case.cleaned.clone();
    if let Some(speech) = &case.speech {
        input.audio = Some(AudioEvidence {
            speech: speech.clone(),
        });
    }
    if let Some(app) = &case.app {
        input.context = Context::for_app(app);
    }

    let verdict = Readback::with_config(config).check(input);
    let baseline_text = case.baseline_text().to_string();

    CaseResult {
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
    }
}

/// Aggregate numbers over a set of results.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Summary {
    pub cases: usize,
    pub controls: usize,

    pub baseline_wer: f32,
    pub baseline_cser: f32,
    /// Share of all cases where the baseline pasted a changed instruction.
    pub baseline_silent_flip_rate: f32,

    pub readback_wer: f32,
    pub readback_cser: f32,
    /// Share of all cases where a changed instruction still reached the user
    /// with no warning.
    pub readback_silent_flip_rate: f32,

    /// Of the baseline's meaning flips, the share Readback repaired or warned
    /// about.
    pub caught_rate: f32,
    /// Of the controls, the share Readback blocked.
    pub false_hold_rate: f32,
    /// Of the controls, the share Readback marked up.
    pub false_highlight_rate: f32,

    pub pass_rate: f32,
    pub highlight_rate: f32,
    pub hold_rate: f32,
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let mut count = 0usize;
    let mut total = 0.0f32;
    for value in values {
        total += value;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

fn rate(matching: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        matching as f32 / total as f32
    }
}

pub fn summarise(results: &[CaseResult]) -> Summary {
    let n = results.len();
    let controls = results.iter().filter(|r| r.benign).count();
    let baseline_flips = results.iter().filter(|r| r.baseline_silent_flip()).count();
    let caught = results
        .iter()
        .filter(|r| r.baseline_silent_flip() && !r.silent_flip())
        .count();

    Summary {
        cases: n,
        controls,
        baseline_wer: mean(results.iter().map(|r| r.baseline.wer)),
        baseline_cser: mean(results.iter().map(|r| r.baseline.cser)),
        baseline_silent_flip_rate: rate(baseline_flips, n),
        readback_wer: mean(results.iter().map(|r| r.readback.wer)),
        readback_cser: mean(results.iter().map(|r| r.readback.cser)),
        readback_silent_flip_rate: rate(results.iter().filter(|r| r.silent_flip()).count(), n),
        caught_rate: rate(caught, baseline_flips),
        false_hold_rate: rate(results.iter().filter(|r| r.false_hold()).count(), controls),
        false_highlight_rate: rate(
            results.iter().filter(|r| r.false_highlight()).count(),
            controls,
        ),
        pass_rate: rate(
            results.iter().filter(|r| r.action == Action::Pass).count(),
            n,
        ),
        highlight_rate: rate(
            results
                .iter()
                .filter(|r| r.action == Action::Highlight)
                .count(),
            n,
        ),
        hold_rate: rate(
            results.iter().filter(|r| r.action == Action::Hold).count(),
            n,
        ),
    }
}

/// One row of the per-category breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategorySummary {
    pub category: String,
    #[serde(flatten)]
    pub summary: Summary,
}

pub fn by_category(results: &[CaseResult]) -> Vec<CategorySummary> {
    let mut categories: Vec<&str> = results.iter().map(|r| r.category.as_str()).collect();
    categories.sort_unstable();
    categories.dedup();

    categories
        .into_iter()
        .map(|category| {
            let subset: Vec<CaseResult> = results
                .iter()
                .filter(|r| r.category == category)
                .cloned()
                .collect();
            CategorySummary {
                category: category.to_string(),
                summary: summarise(&subset),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub summary: Summary,
    pub categories: Vec<CategorySummary>,
    pub cases: Vec<CaseResult>,
}

pub fn run(cases: &[Case], config: &Config) -> Result<Report> {
    let results: Vec<CaseResult> = cases.iter().map(|case| run_case(case, config)).collect();
    Ok(Report {
        summary: summarise(&results),
        categories: by_category(&results),
        cases: results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset;
    use readback_core::Locale;

    fn config() -> Config {
        Config::recommended().with_locales([Locale::En, Locale::Hinglish])
    }

    #[test]
    fn the_bundled_dataset_runs() {
        let cases = dataset::bundled().unwrap();
        let report = run(&cases, &config()).unwrap();
        assert_eq!(report.summary.cases, cases.len());
        assert!(!report.categories.is_empty());
    }

    #[test]
    fn readback_reduces_silent_meaning_flips() {
        let report = run(&dataset::bundled().unwrap(), &config()).unwrap();
        assert!(
            report.summary.readback_silent_flip_rate < report.summary.baseline_silent_flip_rate,
            "baseline {:.3} vs readback {:.3}",
            report.summary.baseline_silent_flip_rate,
            report.summary.readback_silent_flip_rate
        );
    }

    #[test]
    fn controls_are_mostly_left_alone() {
        let report = run(&dataset::bundled().unwrap(), &config()).unwrap();
        assert!(
            report.summary.false_hold_rate <= 0.1,
            "false hold rate was {:.3}; flag fatigue is the thing that gets this uninstalled",
            report.summary.false_hold_rate
        );
    }

    #[test]
    fn a_control_that_was_already_correct_passes_untouched() {
        let cases = dataset::bundled().unwrap();
        let case = cases.iter().find(|c| c.id == "ben-007").unwrap();
        let result = run_case(case, &config());
        assert_eq!(result.action, Action::Pass);
        assert_eq!(result.readback.cser, 0.0);
    }
}
