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
            // The fixture's own llmlint directive is a suppression like any
            // other, so a scan that claims to list every one has to show it.
            "src/app.py:13:3 llmlint suppressions_justified (file) -- fixture input, not production code: the bare directive above proves a blanket, reason-less suppression is reported with empty rules and a null reason (tests/golden/report.json).\n",
            "src/vendored.py:1:1 ruff E501 (file) -- vendored upstream, not ours to reformat\n",
        ),
        "{stdout}"
    );

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr, "notignored: 5 ignores in 2 files\n", "{stderr}");
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

/// The multi-tool tree: one file per newly supported tool, every scope in play.
fn tools_tree() -> std::path::PathBuf {
    fixture("tools-tree")
}

#[test]
fn json_format_over_the_multi_tool_tree_matches_its_checked_in_golden() {
    let output = notignored(&tools_tree())
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);

    let golden_path = repo_root().join("tests/golden/tools-report.json");
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
fn the_multi_tool_tree_renders_every_tool_and_scope_readably() {
    let output = notignored(&tools_tree()).output().expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout,
        concat!(
            "scripts/deploy.sh:2:1 shellcheck SC2086 (file) -- every expansion here is a pre-split argument list\n",
            "scripts/deploy.sh:5:1 shellcheck SC2046,SC2000-SC2100 (next-line)\n",
            "scripts/deploy.sh:7:1 shellcheck * (next-line)\n",
            "src/lints.rs:1:1 rust clippy::needless_return (file)\n",
            "src/lints.rs:4:1 rust dead_code,clippy::needless_collect (next-line)\n",
            "src/lints.rs:10:1 rust dead_code (next-line) -- a justification long enough that it wraps across two lines of the attribute\n",
            "src/service.py:1:3 llmlint boundary_inputs_validated (file) -- a transport shim: the caller validates before this layer\n",
            "src/service.py:2:12 ruff F401 (line) -- re-exported for the public API\n",
            "src/service.py:4:3 llmlint tool_output_is_signal (block) -- the trace is this module's whole job\n",
        ),
        "{stdout}"
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

/// A downstream consumer that stops reading must not turn a good scan into an
/// operational failure — `notignored | grep -q` is how a CI job uses this.
///
/// The tree is large on purpose: the report has to exceed the pipe buffer, or
/// the whole thing lands in the pipe before `grep -q` exits and no write ever
/// hits a closed pipe. That is exactly why this bug reached CI on one platform
/// and not another.
#[cfg(unix)]
#[test]
fn a_consumer_that_stops_reading_does_not_fail_the_scan() {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..500 {
        fs::write(
            dir.path().join(format!("module_{index:04}.py")),
            "VALUE = 1  # noqa: E501  # a reason long enough to fill the pipe buffer\n",
        )
        .unwrap();
    }

    let binary = assert_cmd::cargo::cargo_bin("notignored");
    // bash, not sh: `pipefail` is what makes the pipeline report notignored's
    // own exit code rather than grep's, and Debian's /bin/sh (dash) lacks it.
    let script = format!(
        "set -o pipefail; '{}' --format json | grep -q '\"E501\"'",
        binary.display()
    );
    let output = std::process::Command::new("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(dir.path())
        .output()
        .expect("run the pipeline");

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("cannot write report"), "{stderr}");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{:?}: {stderr}",
        output.status
    );
}

/// A genuine write failure (not a closed pipe) must exit 2 and say why —
/// `/dev/full` is the real ENOSPC a full disk would produce.
#[cfg(target_os = "linux")]
#[test]
fn a_stdout_that_cannot_be_written_exits_two_with_the_reason() {
    let full = fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let output = notignored(&tree())
        .args(["--format", "json"])
        .stdout(full)
        .output()
        .expect("run notignored");

    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("cannot write report"), "{stderr}");
    assert!(stderr.contains("No space left on device"), "{stderr}");
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

    for flag in [
        "--format",
        "--tool",
        "--fail-if-found",
        "--diff",
        "--diff-base",
        "PATHS",
    ] {
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
