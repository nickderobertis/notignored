//! Parity with the real clippy and rustc.
//!
//! Rust's `#[allow]`/`#[expect]` are attributes, not comments, so the claim to
//! prove is the same one and the proof is the same shape as
//! [`ruff_parity`](crate::ruff_parity): the pinned toolchain's own
//! `clippy-driver` decides whether a fixture compiles clean under `-D warnings`,
//! and notignored has to describe the attribute that made the difference.
//! Neither side is stubbed.
//!
//! A fixture whose lints are all the compiler's own is judged by `rustc` with
//! exactly those lints denied — `clippy-driver` denies every warning at once, so
//! it cannot say *which* lint a suppression silenced. Both are the pinned
//! toolchain's, so there is still only one pin.

use crate::support::{clippy_passes, fixture, notignored, parse_report, rustc_accepts};

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

/// A Rust attribute may be written across lines, and rustc reads it the same
/// way — so the record has to span every line the directive occupies, which is
/// what `--diff` intersects a change's added lines against.
#[test]
fn a_directive_written_across_lines_is_accepted_by_rustc_and_reported_as_one_span() {
    assert!(
        rustc_accepts(&parity_dir().join("multiline.rs"), &["dead_code"]),
        "a multi-line `#[expect(dead_code, …)]` suppresses just as the one-line form does"
    );

    let report = report_for("multiline.rs");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["rules"], serde_json::json!(["dead_code"]));
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
    // The suppressed range still runs past the attribute to the end of the item.
    assert_eq!(directive["suppressed"]["start_line"], 3);
    assert_eq!(directive["suppressed"]["end_line"], 9);
}

#[test]
fn an_allow_of_several_lints_suppresses_them_all_and_is_reported_verbatim() {
    assert!(
        rustc_accepts(
            &parity_dir().join("allow_list.rs"),
            &["dead_code", "unused_variables"]
        ),
        "one `#[allow(…)]` covers every lint it names"
    );

    let report = report_for("allow_list.rs");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["scope"], "next-line");
    // Every lint path is kept exactly as written, tool prefix and all.
    assert_eq!(
        directive["rules"],
        serde_json::json!(["dead_code", "unused_variables", "clippy::needless_return"])
    );
    assert!(
        directive["reason"].is_null(),
        "none was given: {directive:#}"
    );
}

/// Raising a lint is not silencing it, and a conditional suppression that is not
/// active silences nothing either — rustc settles both, and notignored must not
/// claim a suppression the compiler never applied.
#[test]
fn attributes_that_do_not_silence_the_lint_are_not_reported_as_suppressions() {
    assert!(
        !rustc_accepts(&parity_dir().join("not_suppressed.rs"), &["dead_code"]),
        "`#[deny]` raises dead_code, and the `cfg_attr` allow is inactive outside cfg(test)"
    );

    let report = report_for("not_suppressed.rs");
    assert!(
        report["ignores"].as_array().unwrap().is_empty(),
        "nothing here suppresses anything: {report:#}"
    );
}
