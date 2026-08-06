//! Shared plumbing for the end-to-end journeys.
//!
//! Everything here resolves *real* artifacts: the compiled `notignored` binary
//! and the pinned `ruff` install. Nothing is stubbed — if a prerequisite is
//! missing the suite fails with an actionable message rather than skipping, so a
//! green run always means the journeys actually ran.

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::CommandCargoExt;

/// The repository root (the directory holding `Cargo.toml`).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A checked-in fixture directory.
pub fn fixture(name: &str) -> PathBuf {
    repo_root().join("tests/fixtures").join(name)
}

/// The compiled `notignored` binary, ready to run with `cwd` as its working
/// directory. Report paths are relative to the invocation directory, so running
/// from inside the fixture tree is what makes them stable across machines.
pub fn notignored(cwd: &Path) -> Command {
    let mut command = Command::cargo_bin("notignored").expect("built notignored binary");
    command.current_dir(cwd);
    // Keep output byte-stable regardless of the developer's environment.
    command.env("NO_COLOR", "1");
    command
}

/// The ruff version this repo pins, from `.ruff-version`.
pub fn pinned_ruff_version() -> String {
    let path = repo_root().join(".ruff-version");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
        .trim()
        .to_string()
}

/// The pinned `ruff` binary installed by `scripts/setup-ruff.sh`.
///
/// Panics with the fix when it is missing or the wrong version — a parity test
/// that silently skipped would report an unproven claim as proven.
pub fn ruff_binary() -> PathBuf {
    let venv = repo_root().join(".dev/ruff");
    let candidates = [venv.join("bin/ruff"), venv.join("Scripts/ruff.exe")];
    let binary = candidates.iter().find(|path| path.exists()).unwrap_or_else(|| {
        panic!(
            "pinned ruff not installed at {}\nACTION: run `just bootstrap` (or ./scripts/setup-ruff.sh)",
            venv.display()
        )
    });

    let expected = pinned_ruff_version();
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .expect("run ruff --version");
    let reported = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(
        reported,
        format!("ruff {expected}"),
        "the installed ruff is not the pinned one\nACTION: re-run ./scripts/setup-ruff.sh"
    );
    binary.clone()
}

/// Run the pinned ruff over `file`, returning whether it found any violation.
///
/// `--isolated` keeps the developer's own `pyproject.toml`/`ruff.toml` out of
/// the result, and an explicit `--select` makes the rule under test the only one
/// in play — so the pass/fail flip is caused by the suppression and nothing else.
pub fn ruff_passes(file: &Path, rule: &str) -> bool {
    let output = Command::new(ruff_binary())
        .args(["check", "--isolated", "--no-cache", "--select", rule])
        .arg(file)
        .output()
        .expect("run ruff check");
    assert!(
        output
            .status
            .code()
            .is_some_and(|code| code == 0 || code == 1),
        "ruff exited unexpectedly ({:?}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.status.success()
}

/// Run `git` in `dir`, failing with the command that broke and git's own reason.
///
/// The diff journeys drive a **real** repository: a stubbed git would prove
/// nothing about the semantics `--diff-base` promises.
pub fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap_or_else(|error| {
            panic!("cannot run git {args:?}: {error}\nACTION: install git and re-run")
        });
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository on `main` with no commits yet, configured so the developer's own
/// git settings (signing keys, default branch, identity) cannot change what the
/// journeys see.
pub fn git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "tester@example.com"]);
    git(dir.path(), &["config", "user.name", "Tester"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    git(dir.path(), &["checkout", "-q", "-b", "main"]);
    dir
}

/// Stage everything and commit it.
pub fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Write a file, creating any parent directories it names.
pub fn write(dir: &Path, name: &str, contents: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent directory");
    }
    std::fs::write(&path, contents).unwrap_or_else(|error| {
        panic!("cannot write {}: {error}", path.display());
    });
}

/// Parse a JSON report emitted by the binary.
pub fn parse_report(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not a JSON report: {error}\n{}",
            String::from_utf8_lossy(stdout)
        )
    })
}
