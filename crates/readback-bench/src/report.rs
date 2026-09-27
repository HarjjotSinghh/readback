//! Rendering a [`Report`](crate::runner::Report) for humans and for Markdown.

use crate::audio::{self, AudioResult, Coverage};
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

/// The audio run. The headline is different from the text run: here the engine
/// is what makes the mistakes, so the first thing to report is how often it
/// changed the instruction at all.
pub fn render_audio(
    results: &[AudioResult],
    summary: &Summary,
    coverage: Coverage,
    calibrate: bool,
    verbose: bool,
) -> String {
    let engine = results
        .first()
        .map(|r| r.engine.as_str())
        .unwrap_or("unknown");
    let total_ms: u32 = results.iter().map(|r| r.clip_ms).sum();
    let flips = crate::audio::engine_flips(results);
    let silent = crate::audio::silent_flips(results);

    let mut out = String::new();
    out.push('\n');
    out.push_str(&format!(
        "  CriticalSpeechBench (audio) — {} clips, {:.1}s, engine: {engine}\n\n",
        results.len(),
        total_ms as f32 / 1000.0
    ));

    out.push_str(&summary_table(summary));
    out.push('\n');
    out.push_str(&format!(
        "  {:<28} {:>10}   the engine changed the instruction\n",
        "engine meaning flips",
        pct(summary.baseline_silent_flip_rate)
    ));
    out.push_str(&behaviour_table(summary));

    if coverage.engine_correct > 0 {
        let rate = coverage.noise_on_correct as f32 / coverage.engine_correct as f32;
        out.push_str(&format!(
            "  {:<28} {:>10}   of the {} clips the engine got right\n",
            "noise on correct transcripts",
            pct(rate),
            coverage.engine_correct
        ));
    }

    out.push('\n');
    out.push_str(&format!(
        "  {:<28} {:>3}/{} confidence   {}/{} timings   {}/{} vad   {}/{} cleanup\n",
        "evidence supplied",
        coverage.with_confidence,
        coverage.clips,
        coverage.with_timings,
        coverage.clips,
        coverage.with_speech_regions,
        coverage.clips,
        coverage.with_cleanup,
        coverage.clips,
    ));
    if coverage.is_blind() {
        out.push_str(
            "  the engine reported text only, and no cleanup step was given, so there\n  \
             was nothing for Readback to reason about. Ask the engine for word-level\n  \
             output (whisper.cpp --output-json-full, faster-whisper word timestamps).\n",
        );
    } else if coverage.with_timings == 0 && coverage.with_speech_regions > 0 {
        out.push_str(
            "  voice-activity regions were found but the engine gave no word timings,\n  \
             so gaps cannot be located and omission detection stayed silent.\n",
        );
    }

    if !flips.is_empty() {
        out.push_str(&format!(
            "\n  the engine changed the meaning ({}):\n",
            flips.len()
        ));
        for result in &flips {
            out.push_str(&format!(
                "    {:<10} said:   {}\n",
                result.case.id, result.reference
            ));
            out.push_str(&format!("    {:<10} heard:  {}\n", "", result.heard));
            if result.case.readback_text != result.heard {
                out.push_str(&format!(
                    "    {:<10} became: {}\n",
                    "", result.case.readback_text
                ));
            }
            out.push_str(&format!(
                "    {:<10} {}\n",
                "",
                if result.case.action == Action::Pass {
                    "pasted silently"
                } else {
                    "flagged"
                }
            ));
        }
    }

    if !silent.is_empty() {
        out.push_str(&format!(
            "\n  reached the user unflagged ({}):\n",
            silent.len()
        ));
        for result in &silent {
            out.push_str(&format!(
                "    {:<10} said: {}\n",
                result.case.id, result.reference
            ));
            out.push_str(&format!(
                "    {:<10} got:  {}\n",
                "", result.case.readback_text
            ));
        }
    }

    if calibrate {
        let points = audio::calibrate(results);
        out.push_str("\n  threshold sweep — what a different highlight cutoff would do:\n");
        out.push_str(&format!(
            "  {:>9}  {:>8}  {:>8}  {:>7}\n",
            "highlight", "caught", "noise", "marked"
        ));
        for point in points.iter().filter(|p| p.marked > 0) {
            out.push_str(&format!(
                "  {:>9.2}  {:>8}  {:>8}  {:>7}\n",
                point.highlight,
                pct(point.caught),
                pct(point.noise),
                point.marked
            ));
        }
        if let Some(best) = audio::best_threshold(&points) {
            out.push_str(&format!(
                "\n  best trade-off for this engine: highlight at {:.2} \
                 ({} caught, {} noise)\n",
                best.highlight,
                pct(best.caught),
                pct(best.noise)
            ));
            out.push_str(
                "  the shipped default is 0.35, calibrated against the Cleanup Guard\n  \
                 rather than raw acoustic confidence.\n",
            );

            // A sweep is only worth acting on if some threshold actually
            // separates the two populations. Often none does, and saying so is
            // more useful than handing over a number that looks like a fix.
            if best.noise > 0.25 {
                out.push_str(&format!(
                    "\n  but no threshold separates these two populations well: the best\n  \
                     trade-off still marks {} of the clips this engine got right. On this\n  \
                     run the risk signal is not discriminative, and tuning it will trade\n  \
                     catching for quiet rather than buy both. A cleanup step to diff\n  \
                     against is worth far more here than a better threshold.\n",
                    pct(best.noise)
                ));
            }
        }
    }

    if verbose {
        out.push_str("\n  clips:\n");
        for result in results {
            out.push_str(&format!(
                "    {:<10} {:<12} {:<10} {:>6}ms  {} region(s)  cser {:.3} -> {:.3}\n",
                result.case.id,
                result.case.category,
                action_name(result.case.action),
                result.clip_ms,
                result.speech_regions,
                result.case.baseline.cser,
                result.case.readback.cser
            ));
        }
    }

    out.push('\n');
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
