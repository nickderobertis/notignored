//! The published-package smoke, run over the binary this repo just built.
//!
//! `release.yml`'s verify jobs and `published-smoke.yml` prove `pip install
//! notignored-cli` and `npm install -g notignored-cli` on Linux, macOS, and
//! Windows — but what they assert is `scripts/smoke-published.sh`, and nothing
//! on a runner can
//! tell whether that script's golden still describes the parser that shipped
//! inside the package. A release is the wrong place to find out.
//!
//! So this journey runs the **same file** those workflows run, over the same
//! fixture tree and the same golden, against the freshly compiled `notignored`
//! on PATH. The workflows' expectations cannot drift from the shipped parser
//! without failing here first, on the pull request that moved it.
//!
//! Re-bless with `just bless` after reviewing the diff.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::support::{cargo_version, fixture, notignored, repo_root};

/// The checked-in smoke fixture tree.
fn fixtures() -> PathBuf {
    fixture("smoke")
}

/// The golden report `scripts/smoke-published.sh` compares against.
fn golden() -> PathBuf {
    repo_root().join("tests").join("golden").join("smoke.json")
}

/// A path as bash reads it.
///
/// The script derives its own defaults from `dirname "${BASH_SOURCE[0]}"`, and
/// a backslash path has no directory as far as `dirname` is concerned — Git Bash
/// would resolve the repository root to the wrong place and fail for a reason
/// that has nothing to do with the smoke. Forward slashes are what a Windows
/// bash actually takes, drive letter and all.
fn bash_path(path: &Path) -> String {
    path.to_str()
        .expect("a UTF-8 path")
        .replace('\\', "/")
        .to_string()
}

/// The fixture files, in the order the script's glob hands them to the binary.
///
/// `LC_ALL=C` there and a byte sort here are the same ordering, which is what
/// makes the golden one file rather than one per runner.
fn sources() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .expect("read the smoke fixture tree")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "the smoke fixture tree is empty");
    names
}

/// Run `scripts/smoke-published.sh` with the compiled binary on PATH.
///
/// Prepending its directory rather than naming the binary is the point: the
/// script only ever sees a `notignored` command that PATH resolved, exactly as
/// it does after `pip install` or `npm install -g`.
fn smoke(args: &[&str]) -> Output {
    let binary = assert_cmd::cargo::cargo_bin("notignored");
    let bin_dir = binary
        .parent()
        .expect("the built binary has a directory")
        .to_path_buf();
    let path = match std::env::var_os("PATH") {
        Some(existing) => {
            let mut entries = vec![bin_dir];
            entries.extend(std::env::split_paths(&existing));
            std::env::join_paths(entries).expect("a PATH with the built binary first")
        }
        None => bin_dir.into_os_string(),
    };
    let script = repo_root().join("scripts").join("smoke-published.sh");
    Command::new("bash")
        .arg(bash_path(&script))
        .args(args)
        .current_dir(repo_root())
        .env("PATH", path)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "cannot run scripts/smoke-published.sh: {error}\n\
                 ACTION: install bash — the release and scheduled smoke workflows run this \
                 script on every runner, so it has to be drivable here too"
            )
        })
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Rewrite the golden from the compiled binary when blessing.
fn bless() {
    if std::env::var_os("NOTIGNORED_BLESS").is_none() {
        return;
    }
    let report = notignored(&fixtures())
        .args(sources())
        .args(["--format", "json"])
        .output()
        .expect("run notignored over the smoke fixtures");
    assert!(
        report.status.success(),
        "the smoke scan failed while blessing"
    );
    std::fs::write(golden(), &report.stdout).expect("write the smoke golden");
}

/// A scratch copy of the assets, so a journey can break one without touching the
/// checked-in tree.
fn copied_assets(scratch: &Path) -> (PathBuf, PathBuf) {
    let tree = scratch.join("smoke");
    std::fs::create_dir_all(&tree).expect("create the fixture copy");
    for name in sources() {
        std::fs::copy(fixtures().join(&name), tree.join(&name)).expect("copy a fixture");
    }
    let expected = scratch.join("smoke.json");
    std::fs::copy(golden(), &expected).expect("copy the golden");
    (tree, expected)
}

/// The workflows' assertion, run here against the parser that will ship.
///
/// This is the drift gate: `scripts/smoke-published.sh` is the file the release
/// and scheduled workflows execute, `tests/golden/smoke.json` is the report they
/// compare against, and both are exercised here by the build under test. A
/// record shape that moved without the golden moving with it fails on the pull
/// request instead of on a published release.
#[test]
fn the_smoke_golden_matches_the_shipped_parser() {
    bless();
    let version = cargo_version();
    let output = smoke(&["--expect-version", &version, "--label", "the local build"]);
    assert!(
        output.status.success(),
        "the published-package smoke failed against this build\n--- stdout ---\n{}\n--- stderr ---\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stdout(&output).contains(&format!("the local build: notignored {version}")),
        "the smoke did not report the build it ran, or the label it was given:\n{}",
        stdout(&output)
    );
}

/// A published build whose report drifted is caught, and says so with its label.
///
/// The whole value of the scheduled sweep is this branch: when a registry serves
/// a package whose binary no longer parses the fixture the same way, the run has
/// to go red and name the platform and registry that broke. Tampering with the
/// golden is the one way to put a *correct* binary on the wrong side of that
/// comparison without shipping a broken one.
#[test]
fn a_report_that_drifted_from_the_golden_fails_and_names_the_install() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let (tree, expected) = copied_assets(scratch.path());
    let drifted = std::fs::read_to_string(&expected)
        .expect("read the copied golden")
        .replace("E501", "E502");
    std::fs::write(&expected, drifted).expect("write the drifted golden");

    let output = smoke(&[
        "--fixtures",
        &bash_path(&tree),
        "--expected",
        &bash_path(&expected),
        "--label",
        "PyPI on macos-latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a drifted report must fail the smoke\n{}",
        stdout(&output)
    );
    let stderr = stderr(&output);
    assert!(
        stderr.contains("::error::PyPI on macos-latest:"),
        "the failure did not name the install that broke:\n{stderr}"
    );
    assert!(
        stderr.contains("ACTION:"),
        "the failure offered no next action:\n{stderr}"
    );
    // The diff itself is the diagnosis; without it the annotation says only that
    // something differs.
    assert!(
        stdout(&output).contains("E502"),
        "the failure did not show what differed:\n{}",
        stdout(&output)
    );
}

/// An install that resolved a different build than the one published fails.
///
/// This is the assertion the verify jobs exist for: PyPI and npm both serve a
/// launcher and a platform payload, and a stale or mis-resolved payload installs
/// cleanly and runs — it just is not the version that was released.
#[test]
fn a_version_that_is_not_the_published_one_fails() {
    let output = smoke(&[
        "--expect-version",
        "9.9.9",
        "--label",
        "npm on windows-latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "an unexpected version must fail the smoke\n{}",
        stdout(&output)
    );
    let stderr = stderr(&output);
    assert!(
        stderr.contains("::error::npm on windows-latest:")
            && stderr.contains(&format!("notignored {}", cargo_version()))
            && stderr.contains("notignored 9.9.9"),
        "the failure did not name both versions:\n{stderr}"
    );
}

/// Arguments the script refuses, and what it tells the workflow that passed them.
///
/// It runs inside a matrix leg where the only diagnosis anyone gets is what it
/// printed, so a mistyped option owes an `ACTION:` line as much as a non-zero
/// exit — and exit 2, not 1, so an argument error is never read as a broken
/// package.
#[test]
fn the_smoke_refuses_arguments_it_cannot_act_on() {
    for (what, args, expected) in [
        (
            "an option it does not implement",
            vec!["--registry", "pypi"],
            "unknown option --registry",
        ),
        (
            "an option given no value",
            vec!["--expect-version"],
            "--expect-version needs a value",
        ),
    ] {
        let output = smoke(&args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{what} was not refused as an argument error\n{}",
            stdout(&output)
        );
        let stderr = stderr(&output);
        assert!(
            stderr.contains(expected),
            "{what} was refused without saying why; wanted {expected:?}:\n{stderr}"
        );
        assert!(
            stderr.contains("ACTION:"),
            "{what} was refused with no next action:\n{stderr}"
        );
    }
}

/// Checked out without its assets, the smoke says which one is missing.
///
/// The workflows run this from a fresh checkout at the released tag, where a
/// path that moved shows up as an absent file rather than as a failing
/// assertion. Reporting it as a drifted report would send whoever reads the run
/// looking for a parser bug that is not there.
#[test]
fn the_smoke_reports_assets_it_cannot_find() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let missing = scratch.path().join("gone");
    let output = smoke(&[
        "--fixtures",
        &bash_path(&missing),
        "--label",
        "npm on ubuntu-latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a missing fixture tree must fail the smoke\n{}",
        stdout(&output)
    );
    let stderr = stderr(&output);
    assert!(
        stderr.contains("no fixture directory at") && stderr.contains("check out the repository"),
        "the failure did not name the missing assets:\n{stderr}"
    );
}
