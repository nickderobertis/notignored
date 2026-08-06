//! Shared plumbing for the end-to-end journeys.
//!
//! Everything here resolves *real* artifacts: the compiled `notignored` binary,
//! the pinned linter installs, and the pinned Rust toolchain's own
//! `clippy-driver`. Nothing is stubbed — if a prerequisite is missing the suite
//! fails with an actionable message rather than skipping, so a green run always
//! means the journeys actually ran.

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

/// The version this repo pins for `tool`, from `.<tool>-version`.
pub fn pinned_version(tool: &str) -> String {
    let path = repo_root().join(format!(".{tool}-version"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
        .trim()
        .to_string()
}

/// A pinned linter installed by `scripts/setup-parity-tools.sh`.
///
/// Panics with the fix when it is missing or the wrong version — a parity test
/// that silently skipped would report an unproven claim as proven. The reported
/// version has to *contain* the pin (or its first three components, since
/// `shellcheck-py` 0.11.0.1 ships ShellCheck 0.11.0) because each tool words its
/// `--version` output differently.
pub fn pinned_binary(tool: &str) -> PathBuf {
    let venv = repo_root().join(".dev").join(tool);
    let candidates = [
        venv.join(format!("bin/{tool}")),
        venv.join(format!("Scripts/{tool}.exe")),
    ];
    let binary = candidates
        .iter()
        .find(|path| path.exists())
        .unwrap_or_else(|| {
            panic!(
                "pinned {tool} not installed at {}\n\
                 ACTION: run `just bootstrap` (or ./scripts/setup-parity-tools.sh)",
                venv.display()
            )
        });

    let pin = pinned_version(tool);
    let short: String = pin.split('.').take(3).collect::<Vec<_>>().join(".");
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("run {tool} --version: {error}"));
    let reported = String::from_utf8_lossy(&output.stdout);
    assert!(
        reported.contains(&pin) || reported.contains(&short),
        "the installed {tool} is not the pinned {pin} but {reported:?}\n\
         ACTION: re-run ./scripts/setup-parity-tools.sh"
    );
    binary.clone()
}

/// The pinned `ruff`.
pub fn ruff_binary() -> PathBuf {
    pinned_binary("ruff")
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

/// Run the pinned ShellCheck over `file`, returning whether it found anything.
///
/// ShellCheck needs no rule selection: every fixture is written so the only
/// finding it can produce is the one the directive under test silences, and a
/// directive ShellCheck itself rejects shows up here as a finding too.
pub fn shellcheck_passes(file: &Path) -> bool {
    let output = Command::new(pinned_binary("shellcheck"))
        .arg(file)
        .output()
        .expect("run shellcheck");
    assert!(
        output
            .status
            .code()
            .is_some_and(|code| code == 0 || code == 1),
        "shellcheck exited unexpectedly ({:?}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.status.success()
}

/// Compile `file` with the pinned toolchain's `clippy-driver`, denying every
/// warning, and report whether it built clean.
///
/// `rust-toolchain.toml` pins the toolchain and rustup reads it from the repo
/// root, so this is the same clippy `just lint` runs — no second pin to drift.
/// Metadata goes to a scratch directory: the fixtures are compiled to be judged,
/// not kept.
pub fn clippy_passes(file: &Path) -> bool {
    let out_dir = tempfile::tempdir().expect("scratch dir for clippy output");
    let output = Command::new("clippy-driver")
        .current_dir(repo_root())
        .args([
            "--edition",
            "2021",
            "--crate-type",
            "lib",
            "--emit=metadata",
        ])
        .arg("--out-dir")
        .arg(out_dir.path())
        .arg(file)
        .args(["-D", "warnings", "-W", "clippy::all"])
        .output()
        .expect("run clippy-driver (install it with `rustup component add clippy`)");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("is not installed"),
        "clippy-driver is missing from the pinned toolchain\n\
         ACTION: run `just bootstrap` (or `rustup component add clippy`)"
    );
    assert!(
        !stderr.contains("couldn't read"),
        "clippy-driver could not read the fixture: {stderr}"
    );
    output.status.success()
}

/// Run `llmlint check-ignores` over `dir` with the fixtures' own rule config,
/// returning `(passed, output)`.
///
/// `check-ignores` is llmlint's deterministic, model-free gate: it validates
/// every inline directive without a single judge call, so the parity proof costs
/// no tokens and cannot be flaky. `-c` pins it to the fixtures' config, which is
/// deliberately not named `llmlint.yml` so the repo's own run never discovers it.
pub fn llmlint_check_ignores(dir: &Path) -> (bool, String) {
    let config = fixture("llmlint-parity").join("rules.yml");
    let output = Command::new(pinned_binary("llmlint"))
        .arg("check-ignores")
        .arg("--cwd")
        .arg(dir)
        .arg("-c")
        .arg(&config)
        .output()
        .expect("run llmlint check-ignores");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output
            .status
            .code()
            .is_some_and(|code| code == 0 || code == 2),
        "llmlint exited unexpectedly ({:?}): {combined}",
        output.status
    );
    (output.status.success(), combined)
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
