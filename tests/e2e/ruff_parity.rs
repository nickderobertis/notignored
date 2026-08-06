//! Parity with the real ruff.
//!
//! The product claim is that notignored reports exactly what ruff would have
//! suppressed, without running ruff. These journeys prove it by running **both**:
//! the pinned ruff decides whether a fixture actually passes, and notignored has
//! to describe the suppression that made the difference. Neither side is stubbed
//! — a mocked ruff here would prove the mock and nothing about the claim.

use crate::support::{fixture, notignored, parse_report, ruff_passes};

/// Rule the parity fixtures hinge on: `F401`, unused import.
const RULE: &str = "F401";

fn parity_dir() -> std::path::PathBuf {
    fixture("ruff-parity")
}

fn report_for(file: &str) -> serde_json::Value {
    let output = notignored(&parity_dir())
        .args([file, "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);
    parse_report(&output.stdout)
}

#[test]
fn real_ruff_flags_the_unsuppressed_fixture_and_notignored_reports_nothing() {
    let file = parity_dir().join("violation.py");
    assert!(
        !ruff_passes(&file, RULE),
        "the fixture is supposed to violate {RULE}; parity proves nothing otherwise"
    );

    let report = report_for("violation.py");
    assert!(
        report["ignores"].as_array().unwrap().is_empty(),
        "nothing is suppressed here: {report:#}"
    );
}

#[test]
fn a_line_noqa_makes_real_ruff_pass_and_notignored_describes_it_exactly() {
    let file = parity_dir().join("suppressed.py");
    assert!(
        ruff_passes(&file, RULE),
        "the `# noqa: {RULE}` should make ruff pass; the fixture or the pin drifted"
    );

    let report = report_for("suppressed.py");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["tool"], "ruff");
    assert_eq!(directive["scope"], "line");
    assert_eq!(directive["rules"], serde_json::json!([RULE]));
    assert_eq!(directive["reason"], "kept for its import side effects");
    assert_eq!(directive["path"], "suppressed.py");
    assert_eq!(directive["line"], 1);
    assert_eq!(directive["end_line"], 1);
    assert_eq!(directive["column"], 12);
    assert_eq!(
        directive["raw"],
        "# noqa: F401  # kept for its import side effects"
    );
    assert_eq!(directive["suppressed"]["start_line"], 1);
    assert_eq!(directive["suppressed"]["end_line"], 1);

    // The reported column really is where the directive starts.
    let source = std::fs::read_to_string(&file).unwrap();
    let column = directive["column"].as_u64().unwrap() as usize;
    assert!(
        source.lines().next().unwrap()[column - 1..].starts_with("# noqa"),
        "column {column} does not point at the directive"
    );
}

#[test]
fn a_file_level_noqa_makes_real_ruff_pass_and_is_reported_as_file_scope() {
    let file = parity_dir().join("file_suppressed.py");
    assert!(
        ruff_passes(&file, RULE),
        "the `# ruff: noqa: {RULE}` should exempt the whole file"
    );

    let report = report_for("file_suppressed.py");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["scope"], "file");
    assert_eq!(directive["rules"], serde_json::json!([RULE]));
    assert_eq!(
        directive["reason"],
        "generated module, imports are the point"
    );
    assert_eq!(directive["suppressed"]["start_line"], 1);
    assert!(
        directive["suppressed"]["end_line"].is_null(),
        "a file-scope suppression runs to end-of-file"
    );
}

#[test]
fn removing_the_suppression_flips_real_ruff_back_to_failing() {
    // The recovery half of the journey: the same source without its directive
    // must fail ruff again, so the pass above is attributable to the noqa and
    // not to a rule that never fired.
    let dir = tempfile::tempdir().unwrap();
    let stripped = dir.path().join("stripped.py");
    let source = std::fs::read_to_string(parity_dir().join("suppressed.py")).unwrap();
    let without_directive = source
        .split_once("  # noqa")
        .map(|(code, _)| format!("{code}\n"))
        .expect("a directive");
    std::fs::write(&stripped, &without_directive).unwrap();

    assert!(
        !ruff_passes(&stripped, RULE),
        "stripping the noqa must reinstate the violation"
    );

    let output = notignored(dir.path())
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    assert!(parse_report(&output.stdout)["ignores"]
        .as_array()
        .unwrap()
        .is_empty());
}
