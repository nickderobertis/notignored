//! Parity with the real Biome.
//!
//! Same shape as [`ruff_parity`](crate::ruff_parity): the pinned biome decides
//! whether a fixture actually passes, and notignored has to describe the
//! suppression that made the difference. Neither side is stubbed.
//!
//! Biome *requires* a reason on every suppression, which is what makes its
//! directives worth surfacing in a review — so these journeys check the reason as
//! closely as the span, including one that spans two lines of a block comment.

use crate::support::{biome_diagnostics, biome_passes, fixture, notignored, parse_report};

/// Biome's category for every complaint about a suppression comment itself.
const PAIRING: &str = "suppressions/incorrect";

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

/// The control for the two pairing journeys below: a range biome accepts draws
/// no complaint at all, so a `suppressions/incorrect` diagnostic there really is
/// the pairing rule firing and not background noise from the fixture or config.
#[test]
fn a_matched_range_draws_no_pairing_complaint_from_real_biome() {
    assert_eq!(
        biome_diagnostics(&parity_dir().join("grammar.js"), RULE),
        vec![],
        "the `-start` … `-end` pair in grammar.js is the one biome accepts"
    );
}

#[test]
fn real_biome_leaves_an_unclosed_range_open_and_notignored_reports_it_that_way() {
    let file = parity_dir().join("unclosed_range.js");

    // Biome's verdict on an unclosed range is a *warning*, so it still exits 0.
    // That is why this journey asserts on the diagnostic rather than the status:
    // the exit code cannot tell an unclosed range from a well-formed one.
    assert_eq!(
        biome_diagnostics(&file, RULE),
        vec![(
            PAIRING.to_string(),
            "Range suppressions must have a matching biome-ignore-end".to_string(),
            1,
        )],
        "real biome no longer complains the way this parser assumes; the pin drifted"
    );
    // The complaint does not withdraw the suppression: both `debugger;` lines
    // stay silenced, which is what makes end-of-file the honest end of the range.
    assert!(
        biome_passes(&file, RULE),
        "biome honours an unclosed range to end-of-file while warning about it"
    );

    let report = report_for("unclosed_range.js");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");
    assert_eq!(ignores[0]["scope"], "block");
    assert_eq!(ignores[0]["reason"], "opened and never closed");
    assert_eq!(ignores[0]["suppressed"]["start_line"], 1);
    assert!(
        ignores[0]["suppressed"]["end_line"].is_null(),
        "an unclosed range runs to end-of-file: {report:#}"
    );
}

#[test]
fn real_biome_rejects_a_mismatched_end_and_notignored_leaves_the_range_open() {
    let file = parity_dir().join("mismatched_range.js");

    // A `-end` naming a different selector does not close the range: biome
    // reports the end as having no start *and* the start as never closed.
    assert_eq!(
        biome_diagnostics(&file, RULE),
        vec![
            (
                PAIRING.to_string(),
                "Found a biome-ignore-end suppression without a biome-ignore-start suppression. \
                 This is invalid"
                    .to_string(),
                3,
            ),
            (
                PAIRING.to_string(),
                "Range suppressions must have a matching biome-ignore-end".to_string(),
                1,
            ),
        ],
        "real biome no longer matches ranges by selector; the pin or the parser drifted"
    );
    // Warnings only, again — and the `debugger;` *after* the rejected end is
    // still silenced, so the range really did run past it to end-of-file.
    assert!(
        biome_passes(&file, RULE),
        "the range outlives the end that failed to close it"
    );

    let report = report_for("mismatched_range.js");
    let ignores = report["ignores"].as_array().unwrap();
    // The `-end` closes a range rather than opening one, so it is never itself a
    // suppression — matched or not.
    assert_eq!(ignores.len(), 1, "{report:#}");
    assert_eq!(ignores[0]["scope"], "block");
    assert_eq!(ignores[0]["reason"], "opened at the rule level");
    assert_eq!(ignores[0]["suppressed"]["start_line"], 1);
    assert!(
        ignores[0]["suppressed"]["end_line"].is_null(),
        "a mismatched end leaves the range open: {report:#}"
    );
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
