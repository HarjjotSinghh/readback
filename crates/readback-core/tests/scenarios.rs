//! End-to-end scenarios, written as the failures people actually report.
//!
//! Each case is a real complaint about dictation tools: a cleanup step deleting
//! a negation, an environment swap, a number swap. The point of the suite is
//! that the dangerous cases escalate and the boring ones do not.

use readback_core::config::Config;
use readback_core::{Action, CheckInput, Context, FlagKind, Locale, Readback, Transcript, Word};

fn check(raw: &str, cleaned: &str) -> readback_core::Verdict {
    Readback::new().check(CheckInput::from_text(raw).with_cleaned(cleaned))
}

#[test]
fn the_never_merge_case() {
    // "Do you know how big a gap there is between 'never merge a change like
    // this' and 'merge a change like this'?"
    let v = check(
        "never merge a change like this",
        "Merge a change like this.",
    );
    assert_eq!(v.action, Action::Hold);
    assert!(v.text.to_lowercase().contains("never"));
    assert_eq!(v.flags[0].kind, FlagKind::DroppedNegation);
}

#[test]
fn dangerous_pairs_all_escalate() {
    let cases = [
        ("don't merge this", "Merge this."),
        ("deploy to staging", "Deploy to production."),
        ("send fifteen", "Send fifty."),
        ("set max retries to 15", "Set max retries to 50."),
        ("before Friday", "After Friday."),
        ("enable logging", "Disable logging."),
        ("we can't remove it", "We can remove it."),
        (
            "do not delete the old deployment",
            "Delete the old deployment.",
        ),
    ];
    for (raw, cleaned) in cases {
        let v = check(raw, cleaned);
        assert_ne!(
            v.action,
            Action::Pass,
            "{raw:?} -> {cleaned:?} should not pass silently"
        );
        assert!(!v.flags.is_empty(), "{raw:?} produced no flags");
    }
}

#[test]
fn ordinary_polish_is_left_alone() {
    let cases = [
        (
            "um so basically we uh need to deploy it tomorrow",
            "We need to deploy it tomorrow.",
        ),
        (
            "i'll send that over tomorrow",
            "I'll send it over tomorrow.",
        ),
        ("yeah lol sounds good to me", "Sounds good to me."),
        (
            "the build finished a while back i think",
            "The build finished a while back.",
        ),
    ];
    for (raw, cleaned) in cases {
        let v = check(raw, cleaned);
        assert_eq!(v.action, Action::Pass, "{raw:?} -> {cleaned:?} should pass");
        assert_eq!(
            v.text, cleaned,
            "harmless polish must survive byte for byte"
        );
    }
}

#[test]
fn hinglish_negation_survives_cleanup() {
    let rb = Readback::with_config(Config::default().with_locales([Locale::En, Locale::Hinglish]));
    let v = rb.check(CheckInput::from_text("abhi mat bhejo").with_cleaned("Bhejo abhi."));
    assert_ne!(v.action, Action::Pass);
    assert!(v.text.to_lowercase().contains("mat"));
}

#[test]
fn a_verdict_can_be_serialised_for_a_host_app() {
    let v = check("never merge this", "Merge this.");
    let json = serde_json::to_string(&v).expect("verdict serialises");
    assert!(json.contains("\"action\":\"hold\""));
    assert!(json.contains("dropped_negation"));
}

#[test]
fn flags_point_at_spans_the_host_app_can_highlight() {
    let v = check(
        "do not deploy this to production",
        "Deploy this to production.",
    );
    for flag in &v.flags {
        let slice = flag.span.slice(&v.text);
        assert!(!slice.is_empty(), "flag {:?} pointed at nothing", flag.kind);
        assert!(v.text.contains(slice));
    }
}

#[test]
fn the_same_utterance_routes_differently_per_app() {
    // Identical evidence, three destinations, three answers.
    let raw = Transcript::from_words(vec![
        Word::new("delete").with_confidence(0.44),
        Word::new("the").with_confidence(0.99),
        Word::new("production").with_confidence(0.98),
        Word::new("tables").with_confidence(0.97),
    ]);
    let rb = Readback::recommended();

    let terminal = rb.check(CheckInput::new(raw.clone()).with_context(Context::for_app("Ghostty")));
    let slack = rb.check(CheckInput::new(raw.clone()).with_context(Context::for_app("Slack")));
    let notes = rb.check(CheckInput::new(raw).with_context(Context::for_app("Obsidian")));

    assert_eq!(terminal.action, Action::Hold);
    assert_eq!(notes.action, Action::Highlight);
    assert!(matches!(slack.action, Action::Highlight | Action::Hold));
    // Same evidence, so the underlying numbers must not move.
    assert_eq!(terminal.risk, slack.risk);
    assert_eq!(terminal.risk, notes.risk);
}

#[test]
fn no_evidence_means_no_opinion() {
    let v = Readback::new().check(CheckInput::from_text("hey are you around"));
    assert_eq!(v.action, Action::Pass);
    assert_eq!(v.risk, 0.0);
    assert!(v.flags.is_empty());
    assert!(!v.provenance.cleanup_guard_ran);
}
