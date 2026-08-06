//! Parity with the real Biome.
//!
//! Same shape as [`ruff_parity`](crate::ruff_parity): the pinned biome decides
//! whether a fixture actually passes, and notignored has to describe the
//! suppression that made the difference. Neither side is stubbed.
//!
//! Biome *requires* a reason on every suppression, which is what makes its
//! directives worth surfacing in a review — so these journeys check the reason as
//! closely as the span, including one that spans two lines of a block comment.

use crate::support::{biome_passes, fixture, notignored, parse_report};

/// The rule every fixture hinges on.
const RULE: &str = "lint/suspicious/noDebugger";

fn parity_dir() -> std::path::PathBuf {
    fixture("biome-parity")
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
fn real_biome_flags_the_unsuppressed_fixture_and_notignored_reports_nothing() {
    assert!(
        !biome_passes(&parity_dir().join("violation.js"), RULE),
        "the fixture is supposed to violate {RULE}; parity proves nothing otherwise"
    );

    let report = report_for("violation.js");
    assert!(
        report["ignores"].as_array().unwrap().is_empty(),
        "nothing is suppressed here: {report:#}"
    );
}

#[test]
fn a_biome_ignore_makes_real_biome_pass_and_notignored_describes_it_exactly() {
    let file = parity_dir().join("suppressed.js");
    assert!(
        biome_passes(&file, RULE),
        "the `biome-ignore {RULE}` should make biome pass; the fixture or the pin drifted"
    );

    let report = report_for("suppressed.js");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["tool"], "biome");
    assert_eq!(directive["scope"], "next-line");
    assert_eq!(directive["rules"], serde_json::json!([RULE]));
    assert_eq!(directive["reason"], "paused here while tracing a bug");
    assert_eq!(directive["path"], "suppressed.js");
    assert_eq!(directive["line"], 1);
    assert_eq!(directive["end_line"], 1);
    assert_eq!(directive["column"], 1);
    assert_eq!(
        directive["raw"],
        "// biome-ignore lint/suspicious/noDebugger: paused here while tracing a bug"
    );
    assert_eq!(directive["suppressed"]["start_line"], 2);
    assert_eq!(directive["suppressed"]["end_line"], 2);

    // The reported column really is where the directive starts.
    let source = std::fs::read_to_string(&file).unwrap();
    let column = directive["column"].as_u64().unwrap() as usize;
    assert!(
        source.lines().next().unwrap()[column - 1..].starts_with("// biome-ignore"),
        "column {column} does not point at the directive"
    );
}

#[test]
fn a_biome_ignore_all_makes_real_biome_pass_and_is_reported_as_file_scope() {
    assert!(
        biome_passes(&parity_dir().join("whole_file.js"), RULE),
        "the `biome-ignore-all {RULE}` should exempt the whole file"
    );

    let report = report_for("whole_file.js");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["scope"], "file");
    assert_eq!(directive["rules"], serde_json::json!([RULE]));
    assert_eq!(directive["reason"], "a scratch file, never shipped");
    assert_eq!(directive["suppressed"]["start_line"], 1);
    assert!(
        directive["suppressed"]["end_line"].is_null(),
        "a file-scope suppression runs to end-of-file"
    );
}

#[test]
fn every_grammar_form_passes_real_biome_and_is_reported_with_its_span() {
    assert!(
        biome_passes(&parity_dir().join("grammar.js"), RULE),
        "real biome rejected a form this crate claims to parse; the grammar or the pin drifted"
    );

    let report = report_for("grammar.js");
    let ignores = report["ignores"].as_array().unwrap();
    let described: Vec<_> = ignores
        .iter()
        .map(|d| {
            (
                d["line"].as_u64().unwrap(),
                d["end_line"].as_u64().unwrap(),
                d["scope"].as_str().unwrap().to_string(),
                d["reason"].as_str().unwrap().to_string(),
                d["suppressed"]["start_line"].clone(),
                d["suppressed"]["end_line"].clone(),
            )
        })
        .collect();

    assert_eq!(
        described,
        vec![
            // `// biome-ignore …` on its own line
            (
                1,
                1,
                "next-line".into(),
                "the line-comment form".into(),
                serde_json::json!(2),
                serde_json::json!(2)
            ),
            // the block-comment form, whose reason spans two lines
            (
                3,
                4,
                "next-line".into(),
                "the block-comment form, whose reason spans several lines".into(),
                serde_json::json!(5),
                serde_json::json!(5)
            ),
            // `biome-ignore-start` … `biome-ignore-end`
            (
                6,
                6,
                "block".into(),
                "an explicitly delimited region".into(),
                serde_json::json!(6),
                serde_json::json!(9)
            ),
        ],
        "{report:#}"
    );

    // Every rule is captured as the selector Biome itself matches on.
    for directive in ignores {
        assert_eq!(directive["rules"], serde_json::json!([RULE]));
        assert_eq!(directive["tool"], "biome");
    }
}

#[test]
fn removing_the_suppression_flips_real_biome_back_to_failing() {
    // The recovery half of the journey: the same source without its directive
    // must fail biome again, so the pass above is attributable to the directive
    // and not to a rule that never fired.
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        parity_dir().join("biome.json"),
        dir.path().join("biome.json"),
    )
    .unwrap();
    let stripped = dir.path().join("stripped.js");
    let source = std::fs::read_to_string(parity_dir().join("suppressed.js")).unwrap();
    let without_directive = source
        .split_once('\n')
        .map(|(_, code)| code.to_string())
        .expect("a directive line");
    std::fs::write(&stripped, &without_directive).unwrap();

    assert!(
        !biome_passes(&stripped, RULE),
        "stripping the directive must reinstate the violation"
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
