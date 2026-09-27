//! Human-readable output.

use crate::cli::ColorChoice;
use readback_core::{Action, Flag, Severity, Verdict};
use std::io::IsTerminal;

#[derive(Debug, Clone, Copy)]
pub struct Style {
    enabled: bool,
}

impl Style {
    /// Colour follows the usual rules: never when piped, never when `NO_COLOR`
    /// is set, unless the user asked for it explicitly.
    pub fn resolve(choice: ColorChoice) -> Self {
        let enabled = match choice {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => {
                std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
            }
        };
        Self { enabled }
    }

    pub fn paint(&self, code: &str, text: &str) -> String {
        if !self.enabled || text.is_empty() {
            return text.to_string();
        }
        format!("\x1b[{code}m{text}\x1b[0m")
    }

    pub fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }

    pub fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }
}

fn severity_code(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "1;4;31",
        Severity::High => "4;33",
        Severity::Medium => "4;35",
        Severity::Low => "4;2",
        Severity::None => "2",
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::High => "high",
        Severity::Medium => "medium",
        Severity::Low => "low",
        Severity::None => "none",
    }
}

fn action_code(action: Action) -> &'static str {
    match action {
        Action::Hold => "1;41;97",
        Action::Highlight => "1;43;30",
        Action::Pass => "1;42;30",
    }
}

fn action_label(action: Action) -> &'static str {
    match action {
        Action::Hold => " HOLD ",
        Action::Highlight => " HIGHLIGHT ",
        Action::Pass => " PASS ",
    }
}

/// Paints the flagged spans inside the verdict text.
///
/// Flags may overlap, so severity is resolved per byte and the highest wins.
/// Zero-width spans mark a place where something went missing and are drawn as
/// a caret rather than a range.
pub fn mark_text(text: &str, flags: &[Flag], style: Style) -> String {
    let mut worst: Vec<Option<Severity>> = vec![None; text.len()];
    let mut carets: Vec<usize> = Vec::new();

    for flag in flags {
        if flag.span.start >= flag.span.end {
            if flag.span.start <= text.len() {
                carets.push(flag.span.start);
            }
            continue;
        }
        for slot in worst
            .iter_mut()
            .take(flag.span.end.min(text.len()))
            .skip(flag.span.start)
        {
            *slot = Some(slot.map_or(flag.severity, |cur| cur.max(flag.severity)));
        }
    }
    carets.sort_unstable();
    carets.dedup();

    let mut out = String::with_capacity(text.len());
    let mut run: Option<(Severity, String)> = None;

    let flush = |run: &mut Option<(Severity, String)>, out: &mut String| {
        if let Some((severity, buf)) = run.take() {
            out.push_str(&style.paint(severity_code(severity), &buf));
        }
    };

    for (i, ch) in text.char_indices() {
        if carets.binary_search(&i).is_ok() {
            flush(&mut run, &mut out);
            out.push_str(&style.paint(severity_code(Severity::Critical), "\u{2038}"));
        }
        let severity = worst[i];
        match (&mut run, severity) {
            (Some((current, buf)), Some(s)) if *current == s => buf.push(ch),
            (_, Some(s)) => {
                flush(&mut run, &mut out);
                run = Some((s, ch.to_string()));
            }
            (_, None) => {
                flush(&mut run, &mut out);
                out.push(ch);
            }
        }
    }
    flush(&mut run, &mut out);
    if carets.last().is_some_and(|&c| c >= text.len()) {
        out.push_str(&style.paint(severity_code(Severity::Critical), "\u{2038}"));
    }
    out
}

/// Pads before styling, since ANSI escapes would otherwise count toward width.
fn field(style: Style, label: &str, value: &str) -> String {
    format!("  {} {}", style.dim(&format!("{label:<8}")), value)
}

/// The flag kind as it appears in JSON output, so the two agree.
pub fn flag_kind_name(kind: readback_core::FlagKind) -> String {
    serde_json::to_string(&kind)
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|_| format!("{kind:?}"))
}

/// Prints the verdict: the decision, the numbers, the text and the flags.
pub fn print_verdict(verdict: &Verdict, engine: &str, style: Style) {
    println!();
    println!(
        "  {}  {}",
        style.paint(action_code(verdict.action), action_label(verdict.action)),
        style.dim(&format!(
            "risk {:.2}   stakes {:.2}   suspicion {:.2}",
            verdict.risk, verdict.stakes, verdict.suspicion
        ))
    );
    println!();
    println!(
        "{}",
        field(
            style,
            "text",
            &mark_text(&verdict.text, &verdict.flags, style)
        )
    );
    println!("{}", field(style, "engine", engine));
    if verdict.provenance.cleanup_guard_ran {
        let state = if verdict.provenance.cleanup_reverted {
            "ran, reverted a span"
        } else {
            "ran, kept the polish"
        };
        println!("{}", field(style, "guard", state));
    }
    println!("{}", field(style, "scorer", &verdict.provenance.scorer));

    if verdict.flags.is_empty() {
        println!();
        println!("{}", style.dim("  no flags"));
        println!();
        return;
    }

    println!();
    println!(
        "{}",
        style.dim(&format!("  {} flag(s)", verdict.flags.len()))
    );
    for flag in &verdict.flags {
        print_flag(flag, &verdict.text, style);
    }
    println!();
}

pub fn print_flag(flag: &Flag, text: &str, style: Style) {
    let quoted = flag.span.slice(text);
    let head = format!(
        "  {} {}",
        style.paint(severity_code(flag.severity), severity_label(flag.severity)),
        style.bold(&flag_kind_name(flag.kind)),
    );
    if quoted.is_empty() {
        println!("{head}");
    } else {
        println!("{head} {}", style.dim(&format!("\u{2014} \"{quoted}\"")));
    }
    println!("      {}", flag.evidence);
}

/// Exit code so shell scripts can branch without parsing output.
pub fn exit_code(action: Action) -> i32 {
    match action {
        Action::Pass => 0,
        Action::Highlight => 1,
        Action::Hold => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use readback_core::{FlagKind, Span};

    fn flag(kind: FlagKind, start: usize, end: usize) -> Flag {
        Flag {
            kind,
            severity: kind.severity(),
            span: Span::new(start, end),
            evidence: String::new(),
            suggestion: None,
        }
    }

    #[test]
    fn flag_kinds_render_as_snake_case() {
        assert_eq!(
            flag_kind_name(FlagKind::DroppedNegation),
            "dropped_negation"
        );
        assert_eq!(flag_kind_name(FlagKind::LowConfidence), "low_confidence");
    }

    #[test]
    fn plain_style_leaves_text_alone() {
        let style = Style { enabled: false };
        let flags = [flag(FlagKind::DroppedNegation, 0, 5)];
        assert_eq!(mark_text("never merge", &flags, style), "never merge");
    }

    #[test]
    fn coloured_style_wraps_only_the_span() {
        let style = Style { enabled: true };
        let flags = [flag(FlagKind::DroppedNegation, 0, 5)];
        let out = mark_text("never merge", &flags, style);
        assert!(out.starts_with("\x1b["));
        assert!(out.ends_with(" merge"));
    }

    #[test]
    fn overlapping_flags_take_the_worst_severity() {
        let style = Style { enabled: true };
        let flags = [
            flag(FlagKind::LowConfidence, 0, 11),
            flag(FlagKind::DroppedNegation, 0, 5),
        ];
        let out = mark_text("never merge", &flags, style);
        assert!(
            out.contains("1;4;31"),
            "critical styling should win: {out:?}"
        );
    }

    #[test]
    fn a_zero_width_flag_draws_a_caret() {
        let style = Style { enabled: false };
        let flags = [flag(FlagKind::DroppedNegation, 6, 6)];
        assert_eq!(mark_text("merge this", &flags, style), "merge \u{2038}this");
    }

    #[test]
    fn multibyte_text_is_not_split() {
        let style = Style { enabled: false };
        let flags = [flag(FlagKind::ChangedProtectedTerm, 0, 6)];
        assert_eq!(mark_text("café ok", &flags, style), "café ok");
    }
}
