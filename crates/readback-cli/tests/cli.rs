//! End-to-end tests against the built binary.
//!
//! These cover the contract shell scripts depend on: exit codes, JSON shape,
//! and the fact that piping output strips colour.

use std::process::{Command, Output};

fn readback(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_readback"))
        .args(args)
        .output()
        .expect("failed to run the readback binary")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn a_dropped_negation_exits_two() {
    let out = readback(&[
        "check",
        "--text",
        "never merge a change like this",
        "--cleaned-text",
        "Merge a change like this.",
    ]);
    assert_eq!(out.status.code(), Some(2), "hold should exit 2");
    assert!(stdout(&out).contains("HOLD"));
}

#[test]
fn harmless_polish_exits_zero() {
    let out = readback(&[
        "check",
        "--text",
        "um so we need to ship it tomorrow",
        "--cleaned-text",
        "We need to ship it tomorrow.",
    ]);
    assert_eq!(out.status.code(), Some(0), "pass should exit 0");
}

#[test]
fn exit_zero_suppresses_the_failure_code() {
    let out = readback(&[
        "check",
        "--text",
        "never merge this",
        "--cleaned-text",
        "Merge this.",
        "--exit-zero",
    ]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn json_output_is_machine_readable() {
    let out = readback(&[
        "check",
        "--text",
        "never merge this",
        "--cleaned-text",
        "Merge this.",
        "--json",
    ]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");
    assert_eq!(value["action"], "hold");
    assert_eq!(value["flags"][0]["kind"], "dropped_negation");
    assert!(
        value["text"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("never")
    );
}

#[test]
fn piped_output_carries_no_escape_codes() {
    let out = readback(&[
        "check",
        "--text",
        "never merge this",
        "--cleaned-text",
        "Merge this.",
    ]);
    assert!(
        !stdout(&out).contains('\x1b'),
        "auto colour must be off when piped"
    );
}

#[test]
fn diff_reports_only_the_guard() {
    let out = readback(&[
        "diff",
        "--text",
        "deploy this to staging",
        "--cleaned-text",
        "Deploy this to production.",
        "--json",
    ]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");
    assert_eq!(value["reverted"], true);
    assert_eq!(value["text"], "Deploy this to staging.");
}

#[test]
fn diff_without_a_rewrite_is_an_error() {
    let out = readback(&["diff", "--text", "deploy this"]);
    assert_eq!(out.status.code(), Some(70));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--cleaned"));
}

#[test]
fn explain_shows_the_arithmetic() {
    let out = readback(&[
        "explain",
        "--text",
        "never merge this",
        "--cleaned-text",
        "Merge this.",
        "--app",
        "Ghostty",
        "--recommended",
    ]);
    let text = stdout(&out);
    for expected in ["suspicion", "stakes", "risk", "hold at", "decision"] {
        assert!(
            text.contains(expected),
            "explain output missing {expected:?}:\n{text}"
        );
    }
}

#[test]
fn lexicon_classify_names_the_class() {
    let out = readback(&["lexicon", "classify", "never", "staging", "laptop"]);
    let text = stdout(&out);
    assert!(text.contains("negation"));
    assert!(text.contains("environment"));
    assert!(text.contains("unprotected"));
}

#[test]
fn hinglish_loads_only_when_requested() {
    let without = stdout(&readback(&["lexicon", "classify", "nahi"]));
    assert!(without.contains("unprotected"));
    let with = stdout(&readback(&[
        "lexicon", "classify", "nahi", "--locale", "en", "--locale", "hinglish",
    ]));
    assert!(with.contains("negation"));
}

#[test]
fn an_unknown_locale_is_reported_clearly() {
    let out = readback(&["lexicon", "list", "--locale", "klingon"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown locale"));
}

#[test]
fn faster_whisper_json_is_detected_automatically() {
    let dir = std::env::temp_dir().join("readback-cli-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fw.json");
    std::fs::write(
        &path,
        r#"{"segments":[{"text":" deploy it to staging","words":[
            {"word":" deploy","probability":0.93},{"word":" staging","probability":0.41}]}]}"#,
    )
    .unwrap();

    let out = readback(&[
        "check",
        "--raw",
        path.to_str().unwrap(),
        "--cleaned-text",
        "Deploy it to production.",
    ]);
    let text = stdout(&out);
    assert!(
        text.contains("faster-whisper"),
        "engine should be detected:\n{text}"
    );
    assert!(text.contains("changed_environment"));
}

#[test]
fn quiet_prints_nothing_but_still_reports_the_verdict() {
    let out = readback(&[
        "check",
        "--text",
        "never merge this",
        "--cleaned-text",
        "Merge this.",
        "--quiet",
    ]);
    assert!(stdout(&out).trim().is_empty(), "quiet should print nothing");
    assert_eq!(out.status.code(), Some(2));
}

/// Writes a history file and returns its path.
fn history(name: &str, lines: &[&str]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("readback-audit-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.jsonl"));
    std::fs::write(&path, lines.join("\n")).unwrap();
    path
}

#[test]
fn audit_counts_meaning_changes_across_a_history() {
    let path = history(
        "mixed",
        &[
            r#"{"raw":"never merge a change like this","cleaned":"Merge a change like this."}"#,
            r#"{"raw":"um so we need to ship it","cleaned":"We need to ship it."}"#,
            r#"{"raw":"deploy this to staging","cleaned":"Deploy this to production."}"#,
        ],
    );
    let out = readback(&["audit", "--pairs", path.to_str().unwrap(), "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");

    assert_eq!(value["pairs"], 3);
    assert_eq!(value["changed"], 2, "the harmless polish must not count");
    assert_eq!(value["by_kind"]["dropped_negation"], 1);
    assert_eq!(value["by_kind"]["changed_environment"], 1);
    assert_eq!(out.status.code(), Some(1), "findings exit non-zero");
}

#[test]
fn audit_is_quiet_when_a_cleanup_step_behaves() {
    let path = history(
        "clean",
        &[
            r#"{"raw":"um so we need to ship it","cleaned":"We need to ship it."}"#,
            r#"{"raw":"i think we're good to go","cleaned":"I think we're good to go."}"#,
        ],
    );
    let out = readback(&["audit", "--pairs", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("your cleanup step is behaving"));
}

#[test]
fn audit_reads_whatever_the_fields_are_called() {
    let path = history(
        "fields",
        &[r#"{"transcript":"don't merge this","polished":"Merge this."}"#],
    );
    let out = readback(&[
        "audit",
        "--pairs",
        path.to_str().unwrap(),
        "--raw-field",
        "transcript",
        "--cleaned-field",
        "polished",
        "--json",
    ]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["changed"], 1);
}

#[test]
fn audit_skips_unusable_lines_rather_than_failing() {
    let path = history(
        "messy",
        &[
            "not json at all",
            r#"{"raw":"only a raw field"}"#,
            "",
            r#"{"raw":"never merge this","cleaned":"Merge this."}"#,
        ],
    );
    let out = readback(&["audit", "--pairs", path.to_str().unwrap(), "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["pairs"], 1);
    assert_eq!(value["changed"], 1);
}

#[test]
fn audit_says_what_it_expected_when_nothing_matches() {
    let path = history("wrong", &[r#"{"a":"1","b":"2"}"#]);
    let out = readback(&["audit", "--pairs", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(70));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("raw") && stderr.contains("cleaned"),
        "{stderr}"
    );
}

#[test]
fn audit_accepts_a_history_on_stdin() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_readback"))
        .args(["audit", "--pairs", "-", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"raw":"never merge this","cleaned":"Merge this."}"#)
        .unwrap();

    let out = child.wait_with_output().unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["changed"], 1);
}
