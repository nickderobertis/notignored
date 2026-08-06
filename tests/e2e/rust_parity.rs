//! Parity with the real clippy and rustc.
//!
//! Rust's `#[allow]`/`#[expect]` are attributes, not comments, so the claim to
//! prove is the same one and the proof is the same shape as
//! [`ruff_parity`](crate::ruff_parity): the pinned toolchain's own
//! `clippy-driver` decides whether a fixture compiles clean under `-D warnings`,
//! and notignored has to describe the attribute that made the difference.
//! Neither side is stubbed.

use crate::support::{clippy_passes, fixture, notignored, parse_report};

fn parity_dir() -> std::path::PathBuf {
    fixture("rust-parity")
}

fn report_for(file: &str) -> serde_json::Value {
    let output = notignored(&parity_dir())
        .args([file, "--tool", "rust", "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);
    parse_report(&output.stdout)
}

#[test]
fn real_clippy_flags_the_unsuppressed_fixture_and_notignored_reports_nothing() {
    assert!(
        !clippy_passes(&parity_dir().join("violation.rs")),
        "the fixture is supposed to violate dead_code and clippy::needless_return; \
         parity proves nothing otherwise"
    );

    let report = report_for("violation.rs");
    assert!(
        report["ignores"].as_array().unwrap().is_empty(),
        "nothing is suppressed here: {report:#}"
    );
}

#[test]
fn outer_attributes_make_real_clippy_pass_and_notignored_describes_them_exactly() {
    assert!(
        clippy_passes(&parity_dir().join("suppressed.rs")),
        "the `#[allow]`/`#[expect]` attributes should make clippy pass; \
         the fixture or the toolchain pin drifted"
    );

    let report = report_for("suppressed.rs");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 2, "{report:#}");

    let allowed = &ignores[0];
    assert_eq!(allowed["tool"], "rust");
    assert_eq!(allowed["scope"], "next-line");
    assert_eq!(
        allowed["rules"],
        serde_json::json!(["clippy::needless_return"])
    );
    assert!(allowed["reason"].is_null(), "{allowed}");
    assert_eq!(allowed["path"], "suppressed.rs");
    assert_eq!(allowed["line"], 3);
    assert_eq!(allowed["end_line"], 3);
    assert_eq!(allowed["column"], 1);
    assert_eq!(allowed["raw"], "#[allow(clippy::needless_return)]");
    // The range covers the annotated `fn`, which ends on line 6.
    assert_eq!(allowed["suppressed"]["start_line"], 3);
    assert_eq!(allowed["suppressed"]["end_line"], 6);

    let expected = &ignores[1];
    assert_eq!(expected["scope"], "next-line");
    assert_eq!(expected["rules"], serde_json::json!(["dead_code"]));
    assert_eq!(
        expected["reason"],
        "kept for the 1.0 surface, wired up next release"
    );
    assert_eq!(expected["suppressed"]["start_line"], 8);
    assert_eq!(expected["suppressed"]["end_line"], 11);

    // The reported column really is where the attribute starts.
    let source = std::fs::read_to_string(parity_dir().join("suppressed.rs")).unwrap();
    let line = source.lines().nth(2).unwrap();
    let column = allowed["column"].as_u64().unwrap() as usize;
    assert!(
        line[column - 1..].starts_with("#[allow"),
        "column {column} does not point at the attribute: {line:?}"
    );
}

#[test]
fn an_inner_attribute_makes_real_clippy_pass_and_is_reported_as_file_scope() {
    assert!(
        clippy_passes(&parity_dir().join("file_suppressed.rs")),
        "the `#![allow(…)]` should exempt the whole crate root"
    );

    let report = report_for("file_suppressed.rs");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["scope"], "file");
    assert_eq!(
        directive["rules"],
        serde_json::json!(["clippy::needless_return"])
    );
    assert_eq!(directive["raw"], "#![allow(clippy::needless_return)]");
    assert_eq!(directive["suppressed"]["start_line"], 1);
    assert!(
        directive["suppressed"]["end_line"].is_null(),
        "a file-scope suppression runs to end-of-file"
    );
}

#[test]
fn removing_the_attributes_flips_real_clippy_back_to_failing() {
    // The recovery half of the journey: the same source without its attributes
    // must fail again, so the pass above is attributable to them and not to
    // lints that never fired.
    let dir = tempfile::tempdir().unwrap();
    let stripped = dir.path().join("stripped.rs");
    let source = std::fs::read_to_string(parity_dir().join("suppressed.rs")).unwrap();
    let without_attributes: String = source
        .lines()
        .filter(|line| !line.starts_with("#[allow") && !line.starts_with("#[expect"))
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(
        !without_attributes.contains("#[allow") && !without_attributes.contains("#[expect"),
        "the fixture's attributes are not all on one line each: {without_attributes}"
    );
    std::fs::write(&stripped, &without_attributes).unwrap();

    assert!(
        !clippy_passes(&stripped),
        "stripping the attributes must reinstate the violations"
    );

    let output = notignored(dir.path())
        .args(["--tool", "rust", "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(parse_report(&output.stdout)["ignores"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn allow_tokens_that_are_not_attributes_are_never_reported() {
    let report = report_for("../tools-tree/src/lints.rs");
    let raws: Vec<&str> = report["ignores"]
        .as_array()
        .unwrap()
        .iter()
        .map(|directive| directive["raw"].as_str().unwrap())
        .collect();
    // `lints.rs` ends with a string literal holding `#[allow(dead_code)]`.
    assert_eq!(raws.len(), 3, "{report:#}");
    assert!(
        raws.iter()
            .all(|raw| raw.starts_with("#[") || raw.starts_with("#![")),
        "a string literal was reported as an attribute: {raws:?}"
    );
}
