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

/// The rustc version this repo pins, from `rust-toolchain.toml`.
pub fn pinned_rustc_version() -> String {
    let path = repo_root().join("rust-toolchain.toml");
    let manifest = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("channel = "))
        .map(|channel| channel.trim().trim_matches('"').to_string())
        .expect("rust-toolchain.toml pins a channel")
}

/// Compile `file` as a library with `lints` denied, returning whether it built
/// clean.
///
/// The compiler *is* the tool whose lint is being silenced, so parity is proved
/// against the pinned toolchain rustup already resolves `rustc` to — a different
/// compiler would prove a different claim.
pub fn rustc_accepts(file: &Path, lints: &[&str]) -> bool {
    let expected = pinned_rustc_version();
    let version = Command::new("rustc")
        .arg("--version")
        .output()
        .expect("run rustc --version");
    let reported = String::from_utf8_lossy(&version.stdout).trim().to_string();
    assert!(
        reported.starts_with(&format!("rustc {expected} ")),
        "rustc is {reported}, not the pinned {expected}\nACTION: run `rustup toolchain install` \
         from the repository root so the pin resolves"
    );

    let out_dir = tempfile::tempdir().expect("tempdir");
    let mut command = Command::new("rustc");
    command.args([
        "--edition",
        "2021",
        "--crate-type",
        "lib",
        "--emit",
        "metadata",
    ]);
    // Denying exactly the lints under test makes the pass/fail flip attributable
    // to the suppression and to nothing else the compiler happens to say.
    for lint in lints {
        command.arg("-D").arg(lint);
    }
    let output = command
        .arg("--out-dir")
        .arg(out_dir.path())
        .arg(file)
        .output()
        .expect("run rustc");
    assert!(
        output.status.code().is_some(),
        "rustc exited unexpectedly ({:?}): {}",
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

/// Run `git` in `dir` and return its stdout, for a journey that needs to read
/// the change the way git itself describes it.
pub fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("cannot run git {args:?}: {error}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
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

/// Commit the whole work tree, deletions included, so what a journey wrote is
/// exactly what the next diff is taken against.
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
