//! Parity with the real mypy, pyright, and ty.
//!
//! The product claim is that notignored reports exactly what each type checker
//! would have suppressed, without running it. These journeys prove it by running
//! **both**: the pinned checker decides whether a fixture actually passes, and
//! the CLI has to describe the directive that made the difference. Neither side
//! is stubbed — a mocked checker here would prove the mock and nothing else.
//!
//! Every fixture under `tests/fixtures/python-types/{mypy,pyright,ty}/` is the
//! *same seven-line program*; they differ only in which comment slot holds a
//! directive. [`fixtures_differ_only_in_their_comments`] pins that down, which is
//! what makes `violation.py` a true control: when the checker flags it and passes
//! its siblings, the directive is the only thing that can account for the flip.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::support::{
    fixture, mypy_failures, mypy_passes, notignored, parse_report, pyright_diagnostics,
    pyright_failures, ruff_passes, ty_failures,
};

fn family_dir() -> PathBuf {
    fixture("python-types")
}

/// The CLI's JSON report for one fixture, with paths relative to the family root.
///
/// Scoped to the tools these fixtures are about: one holding the reason-less
/// form of a directive carries an llmlint `ignore-file` footer to keep it,
/// and that footer is this repo's own lint bookkeeping rather than a record the
/// family asserts on. The golden report below scans unfiltered and does show it.
fn report_for(path: &str) -> serde_json::Value {
    let output = notignored(&family_dir())
        .args([path, "--format", "json"])
        .args(["--tool", "ruff", "--tool", "mypy"])
        .args(["--tool", "pyright", "--tool", "ty"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);
    parse_report(&output.stdout)
}

/// Every directive the CLI reports for one fixture, as
/// `(tool, scope, rules, reason, (line, column), raw, suppressed-end)`.
type Record = (
    String,
    String,
    Vec<String>,
    Option<String>,
    (u64, u64),
    String,
    Option<u64>,
);

fn records_for(path: &str) -> Vec<Record> {
    let report = report_for(path);
    report["ignores"]
        .as_array()
        .expect("an ignores array")
        .iter()
        .map(|directive| {
            assert_eq!(directive["path"], path, "{directive:#}");
            (
                directive["tool"].as_str().expect("a tool").to_string(),
                directive["scope"].as_str().expect("a scope").to_string(),
                directive["rules"]
                    .as_array()
                    .expect("a rules array")
                    .iter()
                    .map(|rule| rule.as_str().expect("a rule name").to_string())
                    .collect(),
                directive["reason"].as_str().map(str::to_string),
                (
                    directive["line"].as_u64().expect("a line"),
                    directive["column"].as_u64().expect("a column"),
                ),
                directive["raw"].as_str().expect("a raw span").to_string(),
                directive["suppressed"]["end_line"].as_u64(),
            )
        })
        .collect()
}

/// One expected record, spelled the way the assertions read best.
fn record(
    tool: &str,
    scope: &str,
    rules: &[&str],
    reason: Option<&str>,
    at: (u64, u64),
    raw: &str,
    suppressed_end: Option<u64>,
) -> Record {
    (
        tool.to_string(),
        scope.to_string(),
        rules.iter().map(|rule| (*rule).to_string()).collect(),
        reason.map(str::to_string),
        at,
        raw.to_string(),
        suppressed_end,
    )
}

/// `source` with every comment removed, so two fixtures can be compared on the
/// code alone. The fixtures keep no `#` inside a string literal, which is what
/// lets a cut at the first `#` stand in for a parse.
fn without_comments(source: &str) -> String {
    let mut code: Vec<&str> = source
        .lines()
        .map(|line| match line.find('#') {
            Some(hash) => line[..hash].trim_end(),
            None => line.trim_end(),
        })
        .collect();
    // Some fixtures close with a comment block (an `llmlint: ignore-file`
    // directive earning the reason-less form its keep). That is not code, and it
    // sits after every line these assertions cite, so it must not make two
    // identical programs read as different. Blank lines *between* code still
    // count: those shift the line numbers the parity tests pin.
    while code.last().is_some_and(|line| line.is_empty()) {
        code.pop();
    }
    code.join("\n")
}

fn read_fixture(path: &str) -> String {
    let full = family_dir().join(path);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|error| panic!("read {}: {error}", full.display()))
}

const MYPY_FIXTURES: &[&str] = &[
    "mypy/violation.py",
    "mypy/line_blanket.py",
    "mypy/line_codes.py",
    "mypy/ignore_errors.py",
    "mypy/disable_error_code.py",
    "mypy/trailing_config.py",
];

const PYRIGHT_FIXTURES: &[&str] = &[
    "pyright/violation.py",
    "pyright/blanket.py",
    "pyright/codes.py",
    "pyright/mode_switch.py",
    "pyright/embedded.py",
    "pyright/rule_value.py",
    "pyright/rule_value_severity.py",
    "pyright/rule_value_trailing.py",
    "pyright/rule_value_embedded.py",
];

const TY_FIXTURES: &[&str] = &[
    "ty/violation.py",
    "ty/line_blanket.py",
    "ty/line_codes.py",
    "ty/file_header.py",
    "ty/next_line.py",
    "ty/embedded.py",
];

#[test]
fn fixtures_differ_only_in_their_comments() {
    for family in [MYPY_FIXTURES, PYRIGHT_FIXTURES, TY_FIXTURES] {
        let control = without_comments(&read_fixture(family[0]));
        for path in &family[1..] {
            assert_eq!(
                without_comments(&read_fixture(path)),
                control,
                "{path} is not the control program plus a directive, so a checker \
                 passing it would not prove the directive did it"
            );
        }
    }
    assert_eq!(
        without_comments(&read_fixture("mixed/suppressed.py")),
        without_comments(&read_fixture("mixed/unsuppressed.py"))
    );
    for outside_a_family in [MALFORMED_FIXTURE, BEHIND_CODE_FIXTURE] {
        assert_eq!(
            without_comments(&read_fixture(outside_a_family)),
            without_comments(&read_fixture(MYPY_FIXTURES[0])),
            "{outside_a_family} is not the control program plus a directive"
        );
    }
}

/// `# mypy: disable-error-code` with nothing assigned to it.
///
/// It cannot join [`MYPY_FIXTURES`]: one mypy run decides that whole family, and
/// mypy 2.3.0 does not merely reject this form — it aborts with an internal
/// error, which would take every sibling verdict down with it.
const MALFORMED_FIXTURE: &str = "malformed/disable_error_code_no_value.py";

/// A directive that silences nothing is exactly what a reviewer needs to see, so
/// the parser reports it rather than dropping it as unparseable.
#[test]
fn a_valueless_disable_error_code_suppresses_nothing_and_is_reported_anyway() {
    assert!(
        !mypy_passes(&family_dir(), "mypy.ini", MALFORMED_FIXTURE),
        "real mypy now accepts `# mypy: disable-error-code` with no value — the \
         form suppresses something after all, so its scope and rules have to be \
         re-derived from the tool"
    );
    assert_eq!(
        records_for(MALFORMED_FIXTURE),
        vec![record(
            "mypy",
            "file",
            &[],
            None,
            (6, 1),
            "# mypy: disable-error-code",
            None,
        )]
    );
}

#[test]
fn real_mypy_is_flipped_by_every_directive_the_parser_claims() {
    assert_eq!(
        mypy_failures(&family_dir(), "mypy.ini", MYPY_FIXTURES),
        // `# mypy: ignore-errors` behind code on the same line is not inline
        // config — mypy reads that form only from a comment owning its line, so
        // the call is still an error and the parser must report nothing.
        vec!["mypy/trailing_config.py", "mypy/violation.py"],
        "real mypy disagrees about which fixtures are suppressed; the fixtures, \
         the grammar, or the pin drifted"
    );
}

#[test]
fn real_pyright_is_flipped_by_every_directive_the_parser_claims() {
    assert_eq!(
        pyright_failures(&family_dir(), ".", PYRIGHT_FIXTURES),
        vec![
            // `# pyright: basic` switches the type-checking mode; it silences
            // nothing, which is exactly why the parser must not report it.
            "pyright/mode_switch.py",
            // A directive that opens no comment is prose to pyright, so the
            // override never applies and the parser must report nothing.
            "pyright/rule_value_embedded.py",
            // A rule moved to another severity is still diagnosed.
            "pyright/rule_value_severity.py",
            // Pyright reads a trailing `# reason` as more of its item list and
            // refuses the whole comment, silencing nothing.
            "pyright/rule_value_trailing.py",
            "pyright/violation.py",
        ],
        "real pyright disagrees about which fixtures are suppressed; the fixtures, \
         the grammar, or the pin drifted"
    );
}

/// An override behind code, which pyright faults *and* applies.
///
/// It cannot join [`PYRIGHT_FIXTURES`]: pyright's complaint about the placement
/// puts the file in the failure list, which in that family reads as "suppressed
/// nothing" — and here the rule really is off.
const BEHIND_CODE_FIXTURE: &str = "pyright/rule_value_behind_code.py";

/// Where a rule override sits does not decide whether it silences the rule, so
/// the record is what a reviewer needs either way.
#[test]
fn an_override_pyright_faults_the_placement_of_is_still_a_live_suppression() {
    let diagnostics = pyright_diagnostics(&family_dir(), ".", &[BEHIND_CODE_FIXTURE]);
    assert_eq!(
        diagnostics,
        // One diagnostic, about where line 7's override was written — and not
        // the `reportArgumentType` the control earns, so the override applied.
        vec![(BEHIND_CODE_FIXTURE.to_string(), None, 7)],
        "real pyright no longer applies an override that trails code; the form's \
         scope has to be re-derived from the tool"
    );

    assert_eq!(
        records_for(BEHIND_CODE_FIXTURE),
        vec![record(
            "pyright",
            "file",
            &["reportArgumentType"],
            None,
            (7, 16),
            "# pyright: reportArgumentType=false",
            None,
        )]
    );
}

#[test]
fn real_ty_is_flipped_by_every_directive_the_parser_claims() {
    assert_eq!(
        ty_failures(&family_dir(), "ty.toml", TY_FIXTURES),
        vec!["ty/violation.py"],
        "real ty disagrees about which fixtures are suppressed; the fixtures, \
         the grammar, or the pin drifted"
    );
}

#[test]
fn the_cli_describes_every_mypy_directive_exactly() {
    let expected: BTreeMap<&str, Vec<Record>> = BTreeMap::from([
        ("mypy/violation.py", vec![]),
        (
            "mypy/line_blanket.py",
            vec![record(
                "mypy",
                "line",
                &[],
                None,
                (7, 16),
                "# type: ignore",
                Some(7),
            )],
        ),
        (
            "mypy/line_codes.py",
            vec![record(
                "mypy",
                "line",
                &["arg-type"],
                Some("upstream stub is wrong"),
                (7, 16),
                "# type: ignore[arg-type]  # upstream stub is wrong",
                Some(7),
            )],
        ),
        (
            "mypy/ignore_errors.py",
            vec![record(
                "mypy",
                "file",
                &[],
                None,
                (6, 1),
                "# mypy: ignore-errors",
                None,
            )],
        ),
        (
            "mypy/disable_error_code.py",
            vec![record(
                "mypy",
                "file",
                &["arg-type", "index"],
                None,
                (6, 1),
                "# mypy: disable-error-code=\"arg-type, index\"",
                None,
            )],
        ),
        // Inline config behind code is not config; reporting it would put a
        // suppression in front of a reviewer that suppresses nothing.
        ("mypy/trailing_config.py", vec![]),
    ]);

    for (path, records) in expected {
        assert_eq!(records_for(path), records, "{path}");
    }
}

#[test]
fn the_cli_describes_every_pyright_directive_exactly() {
    let expected: BTreeMap<&str, Vec<Record>> = BTreeMap::from([
        ("pyright/violation.py", vec![]),
        // A mode switch is configuration, not a suppression.
        ("pyright/mode_switch.py", vec![]),
        (
            "pyright/blanket.py",
            vec![record(
                "pyright",
                "line",
                &[],
                Some("legacy call site, tracked upstream"),
                (7, 16),
                "# pyright: ignore  # legacy call site, tracked upstream",
                Some(7),
            )],
        ),
        (
            "pyright/codes.py",
            vec![record(
                "pyright",
                "line",
                &["reportArgumentType"],
                Some("upstream stub is wrong"),
                (7, 16),
                "# pyright: ignore[reportArgumentType]  # upstream stub is wrong",
                Some(7),
            )],
        ),
        (
            // A rule switched off is a file-wide suppression. Pyright reads the
            // rest of the line as its item list, so the form carries no reason.
            "pyright/rule_value.py",
            vec![record(
                "pyright",
                "file",
                &["reportArgumentType"],
                None,
                (6, 1),
                "# pyright: reportArgumentType=false",
                None,
            )],
        ),
        // A severity change silences nothing, a trailing reason makes pyright
        // refuse the comment, and an override that opens no comment is prose:
        // none of the three is a suppression, so none is reported.
        ("pyright/rule_value_severity.py", vec![]),
        ("pyright/rule_value_trailing.py", vec![]),
        ("pyright/rule_value_embedded.py", vec![]),
        (
            // Real pyright honours a directive that opens no comment, so the
            // record starts at the directive — not at the prose before it, which
            // is someone else's comment and not this suppression's reason.
            "pyright/embedded.py",
            vec![record(
                "pyright",
                "line",
                &["reportArgumentType"],
                None,
                (7, 36),
                "# pyright: ignore[reportArgumentType]",
                Some(7),
            )],
        ),
    ]);

    for (path, records) in expected {
        assert_eq!(records_for(path), records, "{path}");
    }
}

#[test]
fn the_cli_describes_every_ty_directive_exactly() {
    let expected: BTreeMap<&str, Vec<Record>> = BTreeMap::from([
        ("ty/violation.py", vec![]),
        (
            "ty/line_blanket.py",
            vec![record(
                "ty",
                "line",
                &[],
                None,
                (7, 16),
                "# ty: ignore",
                Some(7),
            )],
        ),
        (
            "ty/line_codes.py",
            vec![record(
                "ty",
                "line",
                &["invalid-argument-type"],
                Some("upstream stub is wrong"),
                (7, 16),
                "# ty: ignore[invalid-argument-type]  # upstream stub is wrong",
                Some(7),
            )],
        ),
        (
            "ty/file_header.py",
            vec![record(
                "ty",
                "file",
                &[],
                Some("the whole module is generated from protobuf"),
                (1, 1),
                "# ty: ignore  # the whole module is generated from protobuf",
                None,
            )],
        ),
        (
            // On its own line in the body, ty's directive covers the line below —
            // reporting this one as `line` would point a reviewer at a comment.
            "ty/next_line.py",
            vec![record(
                "ty",
                "next-line",
                &["invalid-argument-type"],
                Some("the call below is deliberately wrong"),
                (6, 1),
                "# ty: ignore[invalid-argument-type]  # the call below is deliberately wrong",
                Some(7),
            )],
        ),
        (
            // Real ty honours a directive that opens no comment. Behind code on
            // the same line it is a `line` directive, not the `next-line` form —
            // only a comment that owns its line reaches the line below.
            "ty/embedded.py",
            vec![record(
                "ty",
                "line",
                &["invalid-argument-type"],
                None,
                (7, 36),
                "# ty: ignore[invalid-argument-type]",
                Some(7),
            )],
        ),
    ]);

    for (path, records) in expected {
        assert_eq!(records_for(path), records, "{path}");
    }
}

/// One line, two tools: both directives are live, and each record covers its own
/// directive only — its own rules, its own reason, its own span.
///
/// Getting this wrong inverts the tool's purpose: without a directive boundary
/// ruff's live `# noqa: F401` reads as mypy's stated justification, and a
/// reviewer sees a suppression that looks explained when it is not.
#[test]
fn a_line_carrying_two_tools_directives_yields_a_record_for_each() {
    let mixed = ["mixed/unsuppressed.py", "mixed/suppressed.py"];
    assert_eq!(
        mypy_failures(&family_dir(), "mypy.ini", &mixed),
        vec!["mixed/unsuppressed.py"],
        "the `# type: ignore[import-not-found]` should be the only thing mypy needs"
    );
    assert!(
        !ruff_passes(&family_dir().join("mixed/unsuppressed.py"), "F401"),
        "the control must violate F401, or the noqa below proves nothing"
    );
    assert!(
        ruff_passes(&family_dir().join("mixed/suppressed.py"), "F401"),
        "the embedded `# noqa: F401` should still reach ruff"
    );

    assert_eq!(
        records_for("mixed/suppressed.py"),
        vec![
            record(
                "mypy",
                "line",
                &["import-not-found"],
                Some("no stubs published"),
                (3, 35),
                "# type: ignore[import-not-found]  # no stubs published",
                Some(3),
            ),
            record(
                "ruff",
                "line",
                &["F401"],
                Some("imported for its side effects"),
                (3, 91),
                "# noqa: F401  # imported for its side effects",
                Some(3),
            ),
        ]
    );

    // Neither record may quote the other's directive anywhere a reviewer reads it.
    for directive in records_for("mixed/suppressed.py") {
        let (tool, .., reason, _, raw, _) = &directive;
        let foreign = if tool == "mypy" {
            "noqa"
        } else {
            "type: ignore"
        };
        assert!(
            !raw.contains(foreign),
            "{tool}'s raw span swallowed the other directive: {raw}"
        );
        assert!(
            !reason.as_deref().unwrap_or_default().contains(foreign),
            "{tool}'s reason swallowed the other directive: {reason:?}"
        );
    }

    assert!(records_for("mixed/unsuppressed.py").is_empty());
}

#[test]
fn the_python_types_json_report_matches_the_checked_in_golden_report() {
    let output = notignored(&family_dir())
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);

    let golden_path = crate::support::repo_root().join("tests/golden/python-types.json");
    let actual = String::from_utf8(output.stdout).unwrap();
    if std::env::var_os("NOTIGNORED_BLESS").is_some() {
        std::fs::write(&golden_path, &actual).expect("write golden report");
    }
    let expected = std::fs::read_to_string(&golden_path).expect("read golden report");
    assert_eq!(
        actual, expected,
        "the JSON report changed. If the change is intended, re-run with NOTIGNORED_BLESS=1 \
         and bump REPORT_VERSION when the shape (not just the data) moved."
    );
}
