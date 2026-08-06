//! Parity with the real Rust compiler.
//!
//! The product claim is that notignored reports exactly what a tool would have
//! suppressed, without running it. These journeys prove it for Rust by running
//! **both**: the pinned `rustc` decides whether a fixture actually compiles
//! under `-D dead_code`, and notignored has to describe the suppression that
//! made the difference. Neither side is stubbed.

use crate::support::{fixture, notignored, parse_report, rustc_accepts};

/// Lint the parity fixtures hinge on: an item nothing uses.
const LINT: &str = "dead_code";

fn parity_dir() -> std::path::PathBuf {
    fixture("rust-parity")
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
fn real_rustc_rejects_the_unsuppressed_fixture_and_notignored_reports_nothing() {
    let file = parity_dir().join("violation.rs");
    assert!(
        !rustc_accepts(&file, &[LINT]),
        "the fixture is supposed to violate {LINT}; parity proves nothing otherwise"
    );

    let report = report_for("violation.rs");
    assert!(
        report["ignores"].as_array().unwrap().is_empty(),
        "nothing is suppressed here: {report:#}"
    );
}

#[test]
fn an_expect_attribute_makes_real_rustc_accept_it_and_notignored_describes_it_exactly() {
    let file = parity_dir().join("suppressed.rs");
    assert!(
        rustc_accepts(&file, &[LINT]),
        "the `#[expect({LINT}, …)]` should make rustc accept the file; the fixture drifted"
    );

    let report = report_for("suppressed.rs");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["tool"], "rust");
    assert_eq!(directive["scope"], "block");
    assert_eq!(directive["rules"], serde_json::json!([LINT]));
    assert_eq!(directive["reason"], "kept until the C API lands");
    assert_eq!(directive["path"], "suppressed.rs");
    assert_eq!(directive["line"], 3);
    assert_eq!(directive["end_line"], 3);
    assert_eq!(directive["column"], 1);
    assert_eq!(
        directive["raw"],
        "#[expect(dead_code, reason = \"kept until the C API lands\")]"
    );
    assert_eq!(directive["suppressed"]["start_line"], 3);
    assert!(
        directive["suppressed"]["end_line"].is_null(),
        "the annotated item's extent is not known without parsing Rust"
    );
}

/// A Rust attribute may be written across lines, and rustc reads it the same
/// way — so the record has to span every line the directive occupies, which is
/// what `--diff` intersects a change's added lines against.
#[test]
fn a_directive_written_across_lines_is_accepted_by_rustc_and_reported_as_one_span() {
    let file = parity_dir().join("multiline.rs");
    assert!(
        rustc_accepts(&file, &[LINT]),
        "a multi-line `#[expect({LINT}, …)]` suppresses just as the one-line form does"
    );

    let report = report_for("multiline.rs");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["rules"], serde_json::json!([LINT]));
    assert_eq!(directive["reason"], "kept until the C API lands");
    assert_eq!(directive["line"], 3);
    assert_eq!(
        directive["end_line"], 6,
        "the record must cover the whole attribute: {directive:#}"
    );
    assert!(
        directive["raw"].as_str().unwrap().contains('\n'),
        "the raw directive spans lines: {directive:#}"
    );
}

#[test]
fn removing_the_suppression_flips_real_rustc_back_to_rejecting() {
    // The recovery half of the journey: the same source without its attribute
    // must fail again, so the acceptance above is attributable to the
    // suppression and not to a lint that never fired.
    let dir = tempfile::tempdir().unwrap();
    let stripped = dir.path().join("stripped.rs");
    let source = std::fs::read_to_string(parity_dir().join("suppressed.rs")).unwrap();
    let without_directive: String = source
        .lines()
        .filter(|line| !line.starts_with("#[expect("))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_ne!(without_directive, source, "a directive was there to strip");
    std::fs::write(&stripped, &without_directive).unwrap();

    assert!(
        !rustc_accepts(&stripped, &[LINT]),
        "stripping the attribute must reinstate the violation"
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

#[test]
fn an_allow_of_several_lints_suppresses_them_all_and_is_reported_verbatim() {
    let file = parity_dir().join("allow_list.rs");
    assert!(
        rustc_accepts(&file, &[LINT, "unused_variables"]),
        "one `#[allow(…)]` covers every lint it names"
    );

    let report = report_for("allow_list.rs");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["scope"], "block");
    // Every lint path is kept exactly as written, tool prefix and all.
    assert_eq!(
        directive["rules"],
        serde_json::json!([LINT, "unused_variables", "clippy::needless_return"])
    );
    assert!(
        directive["reason"].is_null(),
        "none was given: {directive:#}"
    );
}

#[test]
fn an_inner_attribute_suppresses_the_whole_file_and_is_reported_as_file_scope() {
    let file = parity_dir().join("inner.rs");
    assert!(
        rustc_accepts(&file, &[LINT]),
        "the inner `#![allow({LINT})]` exempts the whole crate"
    );

    let report = report_for("inner.rs");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["scope"], "file");
    assert_eq!(directive["rules"], serde_json::json!([LINT]));
    assert_eq!(directive["raw"], "#![allow(dead_code)]");
    assert_eq!(directive["suppressed"]["start_line"], 1);
    assert!(
        directive["suppressed"]["end_line"].is_null(),
        "a file-wide exemption runs to end-of-file"
    );
}
