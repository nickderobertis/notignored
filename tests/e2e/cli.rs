//! End-to-end journeys through the compiled `notignored` binary.
//!
//! Every test here spawns the real executable against a real directory tree and
//! asserts on exit code, stdout, and stderr — the same surface a user touches.

use std::fs;

use crate::support::{fixture, notignored, parse_report, repo_root};

/// The checked-in tree every format assertion runs over.
fn tree() -> std::path::PathBuf {
    fixture("tree")
}

#[test]
fn human_format_lists_every_suppression_and_summarizes_on_stderr() {
    let output = notignored(&tree()).output().expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout,
        concat!(
            "src/app.py:3:12 ruff F401 (line) -- re-exported for the public API\n",
            "src/app.py:5:58 ruff E501 (line) -- long wrapped URL\n",
            "src/app.py:10:17 ruff * (line)\n",
            "src/vendored.py:1:1 ruff E501 (file) -- vendored upstream, not ours to reformat\n",
        ),
        "{stdout}"
    );

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr, "notignored: 4 ignores in 2 files\n", "{stderr}");
}

#[test]
fn json_format_matches_the_checked_in_golden_report() {
    let output = notignored(&tree())
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);

    let golden_path = repo_root().join("tests/golden/report.json");
    let actual = String::from_utf8(output.stdout).unwrap();
    if std::env::var_os("NOTIGNORED_BLESS").is_some() {
        fs::write(&golden_path, &actual).expect("write golden report");
    }
    let expected = fs::read_to_string(&golden_path).expect("read golden report");
    assert_eq!(
        actual, expected,
        "the JSON report changed. If the change is intended, re-run with NOTIGNORED_BLESS=1 \
         and bump REPORT_VERSION when the shape (not just the data) moved."
    );
}

#[test]
fn string_literals_and_unparsed_languages_are_never_reported() {
    let output = notignored(&tree())
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    let report = parse_report(&output.stdout);
    let paths: Vec<&str> = report["ignores"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap())
        .collect();

    // `src/app.ts` holds an eslint directive but no eslint parser is registered
    // yet; `docs/notes.md` is not a source language at all.
    assert!(!paths.iter().any(|p| p.ends_with(".ts")), "{paths:?}");
    assert!(!paths.iter().any(|p| p.ends_with(".md")), "{paths:?}");
    // `MESSAGE = "# noqa: E722"` is a string literal, not a directive.
    assert!(
        !report["ignores"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["rules"][0] == "E722"),
        "a string literal was reported as a suppression"
    );
}

#[test]
fn an_explicit_file_argument_scans_only_that_file() {
    let output = notignored(&tree())
        .args(["src/vendored.py", "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success());
    let report = parse_report(&output.stdout);
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");
    assert_eq!(ignores[0]["path"], "src/vendored.py");
    assert_eq!(ignores[0]["scope"], "file");
}

#[test]
fn the_tool_filter_selects_and_deselects_parsers() {
    let selected = notignored(&tree())
        .args(["--tool", "ruff", "--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(
        parse_report(&selected.stdout)["ignores"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    let deselected = notignored(&tree())
        .args(["--tool", "mypy", "--tool", "eslint", "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(deselected.status.success());
    assert!(parse_report(&deselected.stdout)["ignores"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn fail_if_found_exits_one_when_a_suppression_is_reported() {
    let output = notignored(&tree())
        .arg("--fail-if-found")
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    assert!(!output.stdout.is_empty());
}

#[test]
fn fail_if_found_exits_zero_on_a_clean_tree() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("clean.py"), "VALUE = 1\n").unwrap();
    let output = notignored(dir.path())
        .arg("--fail-if-found")
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(0), "{:?}", output.status);
    assert!(output.stdout.is_empty());
}

#[test]
fn gitignored_directories_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("generated")).unwrap();
    fs::write(dir.path().join(".gitignore"), "generated/\n").unwrap();
    fs::write(dir.path().join("generated/gen.py"), "x = 1  # noqa: E501\n").unwrap();
    fs::write(dir.path().join("kept.py"), "y = 2  # noqa: F401\n").unwrap();

    let output = notignored(dir.path())
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    let report = parse_report(&output.stdout);
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");
    assert_eq!(ignores[0]["path"], "kept.py");
}

#[test]
fn a_missing_path_exits_two_with_an_actionable_message() {
    let dir = tempfile::tempdir().unwrap();
    let output = notignored(dir.path())
        .arg("nope/")
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);
    assert!(
        output.stdout.is_empty(),
        "stdout should stay clean on error"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("nope"), "{stderr}");
    assert!(stderr.contains("hint:"), "{stderr}");
}

#[test]
fn an_unreadable_source_file_exits_two_and_is_reported_not_panicked() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("broken.py"), [b'x', b' ', 0xff, b'\n']).unwrap();
    let output = notignored(dir.path())
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);

    let report = parse_report(&output.stdout);
    let errors = report["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{report:#}");
    assert_eq!(errors[0]["path"], "broken.py");
    assert!(!errors[0]["message"].as_str().unwrap().is_empty());

    // Piping JSON to a file must not hide why the run exited 2.
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("notignored: error: broken.py"), "{stderr}");
}

#[test]
fn an_unknown_tool_name_is_rejected_before_any_scanning() {
    let dir = tempfile::tempdir().unwrap();
    let output = notignored(dir.path())
        .args(["--tool", "flake8"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("flake8"), "{stderr}");
}

#[test]
fn help_documents_the_flags_the_exit_codes_and_every_tool() {
    let dir = tempfile::tempdir().unwrap();
    let output = notignored(dir.path())
        .arg("--help")
        .output()
        .expect("run notignored");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();

    for flag in ["--format", "--tool", "--fail-if-found", "PATHS"] {
        assert!(help.contains(flag), "--help is missing {flag}:\n{help}");
    }
    assert!(help.contains("Exit codes:"), "{help}");
    for tool in notignored::Tool::ALL {
        assert!(
            help.contains(tool.as_str()),
            "--help is missing tool {tool}:\n{help}"
        );
    }
}

#[test]
fn version_reports_the_crate_version() {
    let dir = tempfile::tempdir().unwrap();
    let output = notignored(dir.path())
        .arg("--version")
        .output()
        .expect("run notignored");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
}
