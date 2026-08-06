//! Every `# noqa` form the parser claims to understand, driven end to end.
//!
//! [`ruff_parity`](crate::ruff_parity) proves the headline claim on the two
//! common shapes. This file walks the whole grammar: each row below is a form
//! the README advertises, and each is checked twice — real ruff must actually
//! suppress it, and the CLI must report it with the right codes, span, and
//! reason. A grammar branch covered only by a unit test is a claim about the
//! parser, not about what a user gets.

use crate::support::{fixture, notignored, parse_report, ruff_passes};

/// The rule every fixture line suppresses.
const RULE: &str = "F401";

fn grammar_dir() -> std::path::PathBuf {
    fixture("ruff-grammar")
}

/// One row of `grammar.py`: the line, and what the CLI must say about it.
struct Expected {
    line: u64,
    column: u64,
    rules: &'static [&'static str],
    reason: Option<&'static str>,
    raw: &'static str,
}

const EXPECTED: &[Expected] = &[
    // Uppercase keyword.
    Expected {
        line: 1,
        column: 12,
        rules: &["F401"],
        reason: None,
        raw: "# NOQA: F401",
    },
    // No space after the colon, comma-separated codes.
    Expected {
        line: 2,
        column: 13,
        rules: &["F401", "E402"],
        reason: None,
        raw: "# noqa:F401,E402",
    },
    // Whitespace-separated codes.
    Expected {
        line: 3,
        column: 14,
        rules: &["F401", "E402"],
        reason: None,
        raw: "# noqa: F401 E402",
    },
    // A directive that does not open the comment, with a trailing reason.
    Expected {
        line: 4,
        column: 28,
        rules: &["F401"],
        reason: Some("embedded after another directive"),
        raw: "# noqa: F401  # embedded after another directive",
    },
    // An unparseable trailing token ends the code list rather than the directive.
    Expected {
        line: 5,
        column: 12,
        rules: &["F401"],
        reason: None,
        raw: "# noqa: F401, oops",
    },
    // Blanket.
    Expected {
        line: 6,
        column: 13,
        rules: &[],
        reason: None,
        raw: "# noqa",
    },
];

#[test]
fn real_ruff_accepts_every_form_the_grammar_fixture_uses() {
    // Without the directives the same imports are all violations, so a pass on
    // `grammar.py` is attributable to the suppressions and nothing else.
    assert!(
        !ruff_passes(&grammar_dir().join("unsuppressed.py"), RULE),
        "the unsuppressed fixture must violate {RULE}"
    );
    assert!(
        ruff_passes(&grammar_dir().join("grammar.py"), RULE),
        "real ruff rejected a form this crate claims to parse; the grammar or the pin drifted"
    );
}

#[test]
fn the_cli_reports_every_form_with_its_codes_span_and_reason() {
    let output = notignored(&grammar_dir())
        .args(["grammar.py", "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);

    let report = parse_report(&output.stdout);
    let found = report["ignores"].as_array().unwrap();
    assert_eq!(found.len(), EXPECTED.len(), "{report:#}");

    for (actual, expected) in found.iter().zip(EXPECTED) {
        let at = format!("grammar.py:{}", expected.line);
        assert_eq!(actual["tool"], "ruff", "{at}");
        assert_eq!(actual["scope"], "line", "{at}");
        assert_eq!(actual["line"], expected.line, "{at}");
        assert_eq!(actual["end_line"], expected.line, "{at}");
        assert_eq!(actual["column"], expected.column, "{at}");
        assert_eq!(actual["raw"], expected.raw, "{at}");
        assert_eq!(
            actual["rules"],
            serde_json::to_value(expected.rules).unwrap(),
            "{at}"
        );
        match expected.reason {
            Some(reason) => assert_eq!(actual["reason"], reason, "{at}"),
            None => assert!(actual["reason"].is_null(), "{at}: {actual}"),
        }
    }
}

#[test]
fn a_noqa_inside_a_string_literal_is_never_reported() {
    let output = notignored(&grammar_dir())
        .args(["grammar.py", "--format", "json"])
        .output()
        .expect("run notignored");
    let report = parse_report(&output.stdout);

    // `grammar.py` ends with a tuple holding the literal "# noqa: F811".
    assert!(
        !report["ignores"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["rules"][0] == "F811"),
        "a string literal was reported as a suppression: {report:#}"
    );
}

#[test]
fn the_human_format_renders_the_same_grammar_readably() {
    let output = notignored(&grammar_dir())
        .arg("grammar.py")
        .output()
        .expect("run notignored");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout,
        concat!(
            "grammar.py:1:12 ruff F401 (line)\n",
            "grammar.py:2:13 ruff F401,E402 (line)\n",
            "grammar.py:3:14 ruff F401,E402 (line)\n",
            "grammar.py:4:28 ruff F401 (line) -- embedded after another directive\n",
            "grammar.py:5:12 ruff F401 (line)\n",
            "grammar.py:6:13 ruff * (line)\n",
        ),
        "{stdout}"
    );
}
