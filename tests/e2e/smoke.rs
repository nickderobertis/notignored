//! The published-package smoke, run over the binary this repo just built.
//!
//! `release.yml`'s verify jobs and `published-smoke.yml` prove `pip install
//! notignored-cli` and `npm install -g notignored-cli` on Linux, both macOS
//! architectures, and Windows — but what they assert is
//! `scripts/smoke-published.sh`, and nothing on a runner can tell whether that
//! script's golden still describes the parser that shipped inside the package. A
//! release is the wrong place to find out.
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
    smoke_with_first_on_path(&bin_dir, args)
}

/// The same, with `first` ahead of the inherited PATH — the one knob that
/// decides which `notignored` the script finds, which is the whole of what an
/// install does.
fn smoke_with_first_on_path(first: &Path, args: &[&str]) -> Output {
    let path = match std::env::var_os("PATH") {
        Some(existing) => {
            let mut entries = vec![first.to_path_buf()];
            entries.extend(std::env::split_paths(&existing));
            std::env::join_paths(entries).expect("a PATH with the given directory first")
        }
        None => first.as_os_str().to_os_string(),
    };
    smoke_on_path(&path, args)
}

/// The `bash` that runs the script, resolved to an absolute path.
///
/// Resolved rather than spawned by name because one journey below hands the
/// script a PATH holding nothing at all — the only way to model a host where no
/// install put `notignored` anywhere — and a bash found through that PATH could
/// not be started either.
fn bash_program() -> PathBuf {
    let name = if cfg!(windows) { "bash.exe" } else { "bash" };
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| {
            panic!(
                "no {name} on PATH\n\
                 ACTION: install bash — the release and scheduled smoke workflows run \
                 scripts/smoke-published.sh on every runner, so it has to be drivable here too"
            )
        })
}

/// The same, on exactly `path` — the only way to model a host where nothing
/// installed `notignored` at all.
fn smoke_on_path(path: &std::ffi::OsStr, args: &[&str]) -> Output {
    let script = repo_root().join("scripts").join("smoke-published.sh");
    Command::new(bash_program())
        .arg(bash_path(&script))
        .args(args)
        .current_dir(repo_root())
        .env("PATH", path)
        .output()
        .unwrap_or_else(|error| panic!("cannot run scripts/smoke-published.sh: {error}"))
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
/// assertion. Reporting any of these as a drifted report would send whoever
/// reads the run looking for a parser bug that is not there.
#[test]
fn the_smoke_reports_assets_it_cannot_find() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let gone = bash_path(&scratch.path().join("gone"));
    let empty = scratch.path().join("empty");
    std::fs::create_dir_all(&empty).expect("create an empty fixture tree");
    let empty = bash_path(&empty);

    for (what, args, expected) in [
        (
            "a fixture tree that is not there",
            vec!["--fixtures", gone.as_str()],
            "no fixture directory at",
        ),
        (
            "a golden that is not there",
            vec!["--expected", gone.as_str()],
            "no golden report at",
        ),
        (
            // A checkout that succeeded and delivered nothing: the comparison
            // would otherwise be against an empty argument list, which every
            // build passes.
            "a fixture tree with nothing in it",
            vec!["--fixtures", empty.as_str()],
            "no fixture files in",
        ),
    ] {
        let mut args = args;
        args.extend(["--label", "npm on ubuntu-latest"]);
        let output = smoke(&args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{what} must fail the smoke\n{}",
            stdout(&output)
        );
        let stderr = stderr(&output);
        assert!(
            stderr.contains("::error::npm on ubuntu-latest:") && stderr.contains(expected),
            "{what} was not reported as a missing asset; wanted {expected:?}:\n{stderr}"
        );
        assert!(
            stderr.contains("ACTION:"),
            "{what} was reported with no next action:\n{stderr}"
        );
    }
}

/// A host where the install put nothing on PATH is told to install it.
///
/// `pip install` and `npm install -g` both put their command somewhere the shell
/// has to already be looking; a runner whose PATH the installer never updated
/// gets an empty `command -v` and no other symptom.
#[test]
fn the_smoke_reports_an_install_that_never_reached_path() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    // Exactly one empty directory: even a `notignored` the developer happens to
    // have installed must not satisfy this.
    let output = smoke_on_path(
        scratch.path().as_os_str(),
        &["--label", "PyPI on macos-latest"],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "a host with no notignored must fail the smoke\n{}",
        stdout(&output)
    );
    let stderr = stderr(&output);
    assert!(
        stderr.contains("::error::PyPI on macos-latest: no 'notignored' on PATH")
            && stderr.contains("pip install notignored-cli"),
        "the failure did not name the missing command or how to install it:\n{stderr}"
    );
}

/// A scan that could not complete is reported as that, with the binary's own
/// diagnosis.
///
/// The published binary is real, so the way this happens in the wild is the
/// input: a file it cannot decode, a path it cannot read. That exits 2, not 1,
/// and a smoke that only diffed the report would call it a drift.
#[test]
fn the_smoke_reports_a_scan_that_could_not_complete() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let tree = scratch.path().join("undecodable");
    std::fs::create_dir_all(&tree).expect("create the fixture copy");
    std::fs::write(tree.join("app.py"), b"x = 1  # noqa: E501 \xff\xfe\n")
        .expect("write a file that is not UTF-8");

    let output = smoke(&[
        "--fixtures",
        &bash_path(&tree),
        "--label",
        "npm on windows-latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a scan that could not complete must fail the smoke\n{}",
        stdout(&output)
    );
    let stderr = stderr(&output);
    assert!(
        stderr.contains("::error::npm on windows-latest: the scan exited non-zero"),
        "the failure did not say the scan itself failed:\n{stderr}"
    );
    assert!(
        stderr.contains("valid UTF-8"),
        "the binary's own diagnosis was swallowed:\n{stderr}"
    );
}

/// A command on PATH that cannot even report its version fails as an install
/// problem.
///
/// npm and pip can both leave a launcher that resolves and a payload that does
/// not run — a package unpacked without the executable bit, a binary for the
/// wrong libc. The first thing the smoke asks it is `--version`, and that has to
/// read as a broken install rather than as a report that differs.
///
/// POSIX-only: the branch needs a `notignored` that fails, which means putting a
/// stand-in on PATH, and a shell stand-in is one on every runner where this
/// branch can be reached. What is under test is the script's handling, not
/// notignored.
#[cfg(unix)]
#[test]
fn the_smoke_reports_a_binary_that_cannot_report_its_version() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = tempfile::tempdir().expect("a scratch directory");
    let stub = scratch.path().join("notignored");
    std::fs::write(
        &stub,
        "#!/bin/sh\necho 'notignored: cannot execute' >&2\nexit 126\n",
    )
    .expect("write the stand-in");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
        .expect("make the stand-in executable");

    let output = smoke_with_first_on_path(scratch.path(), &["--label", "PyPI on ubuntu-latest"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a binary that cannot run must fail the smoke\n{}",
        stdout(&output)
    );
    let stderr = stderr(&output);
    assert!(
        stderr.contains("::error::PyPI on ubuntu-latest: 'notignored --version' exited non-zero"),
        "the failure did not name the version check:\n{stderr}"
    );
    assert!(
        stderr.contains("cannot execute"),
        "the stand-in's own output was swallowed:\n{stderr}"
    );
}
