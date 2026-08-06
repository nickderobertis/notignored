//! Parity with the real ESLint.
//!
//! Same shape as [`ruff_parity`](crate::ruff_parity): the pinned eslint decides
//! whether a fixture actually passes, and notignored has to describe the
//! suppression that made the difference. Neither side is stubbed.
//!
//! ESLint parses the ` -- ` description itself and hands it back in
//! `suppressedMessages[].suppressions[].justification`, so these journeys can go
//! further than pass/fail: every reason we report is compared against ESLint's
//! own reading of the same comment.

use crate::support::{collapse, eslint_passes, eslint_result, fixture, notignored, parse_report};

/// The rules the fixtures hinge on.
const RULES: [&str; 2] = ["no-console", "no-alert"];

fn parity_dir() -> std::path::PathBuf {
    fixture("eslint-parity")
}

fn report_for(file: &str) -> serde_json::Value {
    let output = notignored(&parity_dir())
        // Scoped to eslint: the grammar fixture's own `llmlint: ignore-file`
        // footer is a directive we parse too, and this suite asserts on the
        // fixture's whole record set.
        .args([file, "--tool", "eslint", "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);
    parse_report(&output.stdout)
}

/// Every reason ESLint itself extracted, in the order it reported them.
fn eslint_justifications(file: &str) -> Vec<String> {
    let result = eslint_result(&parity_dir().join(file), &RULES);
    result["suppressedMessages"]
        .as_array()
        .expect("suppressedMessages")
        .iter()
        .map(|message| {
            collapse(
                message["suppressions"][0]["justification"]
                    .as_str()
                    .unwrap(),
            )
        })
        .collect()
}

#[test]
fn real_eslint_flags_the_unsuppressed_fixture_and_notignored_reports_nothing() {
    assert!(
        !eslint_passes(&parity_dir().join("violation.js"), &RULES),
        "the fixture is supposed to violate no-console; parity proves nothing otherwise"
    );

    let report = report_for("violation.js");
    assert!(
        report["ignores"].as_array().unwrap().is_empty(),
        "nothing is suppressed here: {report:#}"
    );
}

#[test]
fn a_next_line_directive_makes_real_eslint_pass_and_notignored_describes_it_exactly() {
    let file = parity_dir().join("suppressed.js");
    assert!(
        eslint_passes(&file, &RULES),
        "the `eslint-disable-next-line` should make eslint pass; the fixture or the pin drifted"
    );

    let report = report_for("suppressed.js");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["tool"], "eslint");
    assert_eq!(directive["scope"], "next-line");
    assert_eq!(directive["rules"], serde_json::json!(["no-console"]));
    assert_eq!(directive["reason"], "the CLI prints its own progress");
    assert_eq!(directive["path"], "suppressed.js");
    assert_eq!(directive["line"], 1);
    assert_eq!(directive["end_line"], 1);
    assert_eq!(directive["column"], 1);
    assert_eq!(
        directive["raw"],
        "// eslint-disable-next-line no-console -- the CLI prints its own progress"
    );
    assert_eq!(directive["suppressed"]["start_line"], 2);
    assert_eq!(directive["suppressed"]["end_line"], 2);

    // The reported column really is where the directive starts.
    let source = std::fs::read_to_string(&file).unwrap();
    let column = directive["column"].as_u64().unwrap() as usize;
    assert!(
        source.lines().next().unwrap()[column - 1..].starts_with("// eslint-disable-next-line"),
        "column {column} does not point at the directive"
    );

    // And the line ESLint actually silenced is the one we call suppressed.
    let suppressed_line = eslint_result(&file, &RULES)["suppressedMessages"][0]["line"]
        .as_u64()
        .expect("a suppressed message");
    assert_eq!(
        directive["suppressed"]["start_line"].as_u64(),
        Some(suppressed_line)
    );
}

#[test]
fn every_grammar_form_passes_real_eslint_and_is_reported_with_its_span() {
    let file = parity_dir().join("grammar.js");
    assert!(
        eslint_passes(&file, &RULES),
        "real eslint rejected a form this crate claims to parse (or found an unused \
         directive); the grammar or the pin drifted"
    );

    let report = report_for("grammar.js");
    let ignores = report["ignores"].as_array().unwrap();
    let described: Vec<_> = ignores
        .iter()
        .map(|d| {
            (
                d["line"].as_u64().unwrap(),
                d["column"].as_u64().unwrap(),
                d["scope"].as_str().unwrap().to_string(),
                d["rules"].clone(),
                d["suppressed"]["start_line"].clone(),
                d["suppressed"]["end_line"].clone(),
            )
        })
        .collect();

    assert_eq!(
        described,
        vec![
            // `/* eslint-disable no-alert */` … `/* eslint-enable no-alert */`
            (
                1,
                1,
                "block".into(),
                serde_json::json!(["no-alert"]),
                serde_json::json!(1),
                serde_json::json!(3)
            ),
            // `// eslint-disable-line no-console`, trailing its own line
            (
                4,
                26,
                "line".into(),
                serde_json::json!(["no-console"]),
                serde_json::json!(4),
                serde_json::json!(4)
            ),
            // `// eslint-disable-next-line no-console`
            (
                5,
                1,
                "next-line".into(),
                serde_json::json!(["no-console"]),
                serde_json::json!(6),
                serde_json::json!(6)
            ),
            // the same, as a block comment whose reason spans three lines
            (
                7,
                1,
                "next-line".into(),
                serde_json::json!(["no-console"]),
                serde_json::json!(10),
                serde_json::json!(10)
            ),
            // a blanket `/* eslint-disable */` that is never re-enabled
            (
                11,
                1,
                "block".into(),
                serde_json::json!([]),
                serde_json::json!(11),
                serde_json::Value::Null
            ),
        ],
        "{report:#}"
    );
}

#[test]
fn the_reasons_we_report_are_the_reasons_eslint_itself_extracted() {
    let ours: Vec<String> = report_for("grammar.js")["ignores"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["reason"].as_str().unwrap_or_default().to_string())
        .collect();

    // ESLint reports one suppressed message per silenced problem; the blanket
    // block at the end covers two, and carries no description.
    assert_eq!(
        eslint_justifications("grammar.js"),
        vec![
            "this prelude is interactive on purpose",
            "the trailing form",
            "the next-line form",
            "a reason that spans several lines",
            "",
            "",
        ]
    );
    assert_eq!(
        ours,
        vec![
            "this prelude is interactive on purpose",
            "the trailing form",
            "the next-line form",
            "a reason that spans several lines",
            "",
        ]
    );
}

#[test]
fn removing_the_suppression_flips_real_eslint_back_to_failing() {
    // The recovery half of the journey: the same source without its directive
    // must fail eslint again, so the pass above is attributable to the directive
    // and not to a rule that never fired.
    let dir = tempfile::tempdir().unwrap();
    let stripped = dir.path().join("stripped.js");
    let source = std::fs::read_to_string(parity_dir().join("suppressed.js")).unwrap();
    let without_directive = source
        .split_once('\n')
        .map(|(_, code)| code.to_string())
        .expect("a directive line");
    std::fs::write(&stripped, &without_directive).unwrap();

    assert!(
        !eslint_passes(&stripped, &RULES),
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
