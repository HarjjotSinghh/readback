//! Rendering a [`Report`](crate::runner::Report) for humans and for Markdown.

use crate::runner::{CaseResult, Report, Summary};
use readback_core::Action;

fn pct(value: f32) -> String {
    format!("{:.1}%", value * 100.0)
}

fn action_name(action: Action) -> &'static str {
    match action {
        Action::Pass => "pass",
        Action::Highlight => "highlight",
        Action::Hold => "hold",
    }
}

/// The headline comparison: what the stack would have pasted, versus what
/// Readback let through.
pub fn summary_table(summary: &Summary) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "  {:<28} {:>10} {:>10}\n",
        "", "baseline", "readback"
    ));
    out.push_str(&format!("  {}\n", "-".repeat(50)));
    out.push_str(&format!(
        "  {:<28} {:>10.3} {:>10.3}\n",
        "word error rate", summary.baseline_wer, summary.readback_wer
    ));
    out.push_str(&format!(
        "  {:<28} {:>10.3} {:>10.3}\n",
        "critical semantic error rate", summary.baseline_cser, summary.readback_cser
    ));
    out.push_str(&format!(
        "  {:<28} {:>10} {:>10}\n",
        "silent meaning flips",
        pct(summary.baseline_silent_flip_rate),
        pct(summary.readback_silent_flip_rate)
    ));
    out
}

/// The numbers that decide whether anyone keeps this installed.
pub fn behaviour_table(summary: &Summary) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "  {:<28} {:>10}   caught or repaired, of the baseline's flips\n",
        "caught",
        pct(summary.caught_rate)
    ));
    out.push_str(&format!(
        "  {:<28} {:>10}   controls blocked\n",
        "false hold rate",
        pct(summary.false_hold_rate)
    ));
    out.push_str(&format!(
        "  {:<28} {:>10}   controls marked up\n",
        "false highlight rate",
        pct(summary.false_highlight_rate)
    ));
    out.push_str(&format!(
        "  {:<28} {:>10} / {} / {}\n",
        "pass / highlight / hold",
        pct(summary.pass_rate),
        pct(summary.highlight_rate),
        pct(summary.hold_rate)
    ));
    out
}

pub fn category_table(report: &Report) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "  {:<16} {:>5} {:>10} {:>10} {:>9}\n",
        "category", "n", "base cser", "rb cser", "caught"
    ));
    out.push_str(&format!("  {}\n", "-".repeat(54)));
    for row in &report.categories {
        out.push_str(&format!(
            "  {:<16} {:>5} {:>10.3} {:>10.3} {:>9}\n",
            row.category,
            row.summary.cases,
            row.summary.baseline_cser,
            row.summary.readback_cser,
            if row.summary.baseline_silent_flip_rate > 0.0 {
                pct(row.summary.caught_rate)
            } else {
                "-".to_string()
            }
        ));
    }
    out
}

/// Cases where a changed instruction still reached the user unmarked. These are
/// the failures worth reading one by one.
pub fn misses(report: &Report) -> Vec<&CaseResult> {
    report.cases.iter().filter(|c| c.silent_flip()).collect()
}

/// Controls that were blocked or marked up without cause.
pub fn noise(report: &Report) -> Vec<&CaseResult> {
    report
        .cases
        .iter()
        .filter(|c| c.false_hold() || c.false_highlight())
        .collect()
}

pub fn render(report: &Report, verbose: bool) -> String {
    let mut out = String::new();
    out.push('\n');
    out.push_str(&format!(
        "  CriticalSpeechBench — {} cases, {} controls\n\n",
        report.summary.cases, report.summary.controls
    ));
    out.push_str(&summary_table(&report.summary));
    out.push('\n');
    out.push_str(&behaviour_table(&report.summary));
    out.push('\n');
    out.push_str(&category_table(report));

    let misses = misses(report);
    if !misses.is_empty() {
        out.push_str(&format!("\n  missed ({}):\n", misses.len()));
        for case in &misses {
            out.push_str(&format!("    {:<10} {}\n", case.id, case.readback_text));
        }
    }

    let noise = noise(report);
    if !noise.is_empty() {
        out.push_str(&format!("\n  noise on controls ({}):\n", noise.len()));
        for case in &noise {
            out.push_str(&format!(
                "    {:<10} {:<10} {}\n",
                case.id,
                action_name(case.action),
                case.readback_text
            ));
        }
    }

    if verbose {
        out.push_str("\n  cases:\n");
        for case in &report.cases {
            out.push_str(&format!(
                "    {:<10} {:<12} {:<10} risk {:.2}  cser {:.3} -> {:.3}\n",
                case.id,
                case.category,
                action_name(case.action),
                case.risk,
                case.baseline.cser,
                case.readback.cser
            ));
        }
    }

    out.push('\n');
    out
}

/// Markdown for pasting into a README or a release note.
pub fn render_markdown(report: &Report) -> String {
    let s = &report.summary;
    let mut out = String::new();
    out.push_str(&format!(
        "CriticalSpeechBench v0 — {} cases, {} controls.\n\n",
        s.cases, s.controls
    ));
    out.push_str("| Metric | Baseline | Readback |\n|---|---:|---:|\n");
    out.push_str(&format!(
        "| Word error rate | {:.3} | {:.3} |\n",
        s.baseline_wer, s.readback_wer
    ));
    out.push_str(&format!(
        "| Critical semantic error rate | {:.3} | {:.3} |\n",
        s.baseline_cser, s.readback_cser
    ));
    out.push_str(&format!(
        "| Silent meaning flips | {} | {} |\n\n",
        pct(s.baseline_silent_flip_rate),
        pct(s.readback_silent_flip_rate)
    ));
    out.push_str(&format!(
        "Caught {} of the baseline's meaning flips. False hold rate on controls: {}. \
         Verdicts: {} pass, {} highlight, {} hold.\n",
        pct(s.caught_rate),
        pct(s.false_hold_rate),
        pct(s.pass_rate),
        pct(s.highlight_rate),
        pct(s.hold_rate)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset;
    use readback_core::Locale;
    use readback_core::config::Config;

    fn report() -> Report {
        let config = Config::recommended().with_locales([Locale::En, Locale::Hinglish]);
        crate::runner::run(&dataset::bundled().unwrap(), &config).unwrap()
    }

    #[test]
    fn the_rendered_report_names_both_metrics() {
        let text = render(&report(), false);
        assert!(text.contains("word error rate"));
        assert!(text.contains("critical semantic error rate"));
        assert!(text.contains("caught"));
    }

    #[test]
    fn markdown_is_a_table() {
        let md = render_markdown(&report());
        assert!(md.contains("| Metric | Baseline | Readback |"));
    }
}
