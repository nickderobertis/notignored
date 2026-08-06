//! Parity with the real ShellCheck.
//!
//! ShellCheck decides what each fixture's directive actually silences — and, for
//! the malformed ones, that it silences nothing at all. notignored has to agree
//! on both counts: the same suppressions, and no invented ones.

use crate::support::{fixture, notignored, parse_report, shellcheck_passes};

fn parity_dir() -> std::path::PathBuf {
    fixture("shellcheck-parity")
}

fn report_for(file: &str) -> serde_json::Value {
    let output = notignored(&parity_dir())
        .args([file, "--tool", "shellcheck", "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);
    parse_report(&output.stdout)
}

fn ignores(file: &str) -> Vec<serde_json::Value> {
    report_for(file)["ignores"].as_array().unwrap().clone()
}

#[test]
fn real_shellcheck_flags_the_unsuppressed_fixture_and_notignored_reports_nothing() {
    assert!(
        !shellcheck_passes(&parity_dir().join("violation.sh")),
        "the fixture is supposed to violate SC2086; parity proves nothing otherwise"
    );
    assert!(
        ignores("violation.sh").is_empty(),
        "nothing is suppressed here"
    );
}

#[test]
fn a_directive_after_a_command_makes_real_shellcheck_pass_and_is_next_line_scope() {
    assert!(
        shellcheck_passes(&parity_dir().join("suppressed.sh")),
        "the `# shellcheck disable=SC2086` should make ShellCheck pass; \
         the fixture or the pin drifted"
    );

    let found = ignores("suppressed.sh");
    assert_eq!(found.len(), 1, "{found:#?}");
    let directive = &found[0];
    assert_eq!(directive["tool"], "shellcheck");
    assert_eq!(directive["scope"], "next-line");
    assert_eq!(directive["rules"], serde_json::json!(["SC2086"]));
    assert_eq!(
        directive["reason"],
        "the caller passes a pre-split argument list"
    );
    assert_eq!(directive["path"], "suppressed.sh");
    assert_eq!(directive["line"], 4);
    assert_eq!(directive["end_line"], 4);
    assert_eq!(directive["column"], 1);
    assert_eq!(
        directive["raw"],
        "# shellcheck disable=SC2086  # the caller passes a pre-split argument list"
    );
    assert_eq!(directive["suppressed"]["start_line"], 5);
    assert_eq!(directive["suppressed"]["end_line"], 5);

    // The reported line really is the directive's.
    let source = std::fs::read_to_string(parity_dir().join("suppressed.sh")).unwrap();
    assert!(source.lines().nth(3).unwrap().starts_with("# shellcheck"));
}

#[test]
fn a_directive_above_the_first_command_makes_real_shellcheck_pass_file_wide() {
    assert!(
        shellcheck_passes(&parity_dir().join("file_suppressed.sh")),
        "the directive above the first command should exempt the whole file"
    );

    let found = ignores("file_suppressed.sh");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0]["scope"], "file");
    assert_eq!(found[0]["rules"], serde_json::json!(["SC2086"]));
    assert_eq!(found[0]["suppressed"]["start_line"], 1);
    assert!(
        found[0]["suppressed"]["end_line"].is_null(),
        "a file-scope suppression runs to end-of-file"
    );
}

#[test]
fn real_shellcheck_accepts_every_form_the_grammar_fixture_uses() {
    assert!(
        shellcheck_passes(&parity_dir().join("grammar.sh")),
        "real ShellCheck rejected a form this crate claims to parse; \
         the grammar or the pin drifted"
    );

    let found = ignores("grammar.sh");
    let described: Vec<(u64, &str, Vec<&str>)> = found
        .iter()
        .map(|directive| {
            (
                directive["line"].as_u64().unwrap(),
                directive["scope"].as_str().unwrap(),
                directive["rules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|rule| rule.as_str().unwrap())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        described,
        vec![
            (2, "file", vec!["SC2086", "SC2046"]),
            // A code range stays the token the author wrote, not a hundred codes.
            (4, "next-line", vec!["SC2000-SC2100"]),
            // `disable=all` is a blanket suppression: no rules at all.
            (6, "next-line", vec![]),
        ],
        "{found:#?}"
    );
}

#[test]
fn directives_real_shellcheck_rejects_are_reported_by_neither() {
    let file = parity_dir().join("rejected.sh");
    assert!(
        !shellcheck_passes(&file),
        "ShellCheck must reject this fixture: trailing prose is SC1072 and a \
         directive after a command is SC1126"
    );
    // ShellCheck reported SC2086 anyway, so neither directive suppressed a thing
    // — and notignored must not claim otherwise.
    assert!(
        ignores("rejected.sh").is_empty(),
        "a directive ShellCheck refuses is not a suppression"
    );
}

#[test]
fn removing_the_directive_flips_real_shellcheck_back_to_failing() {
    let dir = tempfile::tempdir().unwrap();
    let stripped = dir.path().join("stripped.sh");
    let source = std::fs::read_to_string(parity_dir().join("file_suppressed.sh")).unwrap();
    let without_directive: String = source
        .lines()
        .filter(|line| !line.starts_with("# shellcheck"))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&stripped, &without_directive).unwrap();

    assert!(
        !shellcheck_passes(&stripped),
        "stripping the directive must reinstate the violation"
    );

    let output = notignored(dir.path())
        .args(["--tool", "shellcheck", "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(parse_report(&output.stdout)["ignores"]
        .as_array()
        .unwrap()
        .is_empty());
}
