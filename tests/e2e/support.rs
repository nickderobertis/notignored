//! Shared plumbing for the end-to-end journeys.
//!
//! Everything here resolves *real* artifacts: the compiled `notignored` binary
//! and the pinned `ruff`, `mypy`, `pyright`, and `ty` installs. Nothing is
//! stubbed — if a prerequisite is missing the suite fails with an actionable
//! message rather than skipping, so a green run always means the journeys
//! actually ran.

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

/// The pinned `tool` binary installed by `scripts/setup-python-tools.sh`.
///
/// Panics with the fix when it is missing or the wrong version — a parity test
/// that silently skipped would report an unproven claim as proven.
pub fn tool_binary(tool: &str) -> PathBuf {
    let venv = repo_root().join(".dev").join(tool);
    let candidates = [
        venv.join(format!("bin/{tool}")),
        venv.join(format!("Scripts/{tool}.exe")),
    ];
    let binary = candidates.iter().find(|path| path.exists()).unwrap_or_else(|| {
        panic!(
            "pinned {tool} not installed at {}\nACTION: run `just bootstrap` (or ./scripts/setup-python-tools.sh)",
            venv.display()
        )
    });

    let expected = pinned_version(tool);
    let output = Command::new(binary)
        .arg("--version")
        .env("PYRIGHT_PYTHON_IGNORE_WARNINGS", "1")
        .output()
        .unwrap_or_else(|error| panic!("run {tool} --version: {error}"));
    // Tools pad the line differently (`mypy 2.3.0 (compiled: yes)`), so match
    // the leading `<tool> <version>` token rather than the whole line.
    let reported = String::from_utf8_lossy(&output.stdout);
    let reported = reported.lines().next().unwrap_or_default().trim();
    let prefix = format!("{tool} {expected}");
    assert!(
        reported == prefix || reported.starts_with(&format!("{prefix} ")),
        "the installed {tool} reports {reported:?}, not the pinned {prefix:?}\n\
         ACTION: re-run ./scripts/setup-python-tools.sh"
    );
    binary.clone()
}

/// Run the pinned ruff over `file`, returning whether it found any violation.
///
/// `--isolated` keeps the developer's own `pyproject.toml`/`ruff.toml` out of
/// the result, and an explicit `--select` makes the rule under test the only one
/// in play — so the pass/fail flip is caused by the suppression and nothing else.
pub fn ruff_passes(file: &Path, rule: &str) -> bool {
    let output = Command::new(tool_binary("ruff"))
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

/// Which of `targets` the pinned mypy still reports a problem in.
///
/// Paths come back `/`-separated and relative to `cwd`, deduplicated and sorted,
/// so a test can assert on the whole set at once. `--config-file` is explicit and
/// the cache goes to a scratch directory: neither a developer's own mypy settings
/// nor a stale cache may decide the verdict.
pub fn mypy_failures(cwd: &Path, config: &str, targets: &[&str]) -> Vec<String> {
    let cache = tempfile::tempdir().expect("scratch mypy cache");
    let output = Command::new(tool_binary("mypy"))
        .current_dir(cwd)
        .args(["--config-file", config])
        .args([
            "--no-incremental",
            "--no-error-summary",
            "--no-color-output",
        ])
        .arg("--cache-dir")
        .arg(cache.path())
        .args(targets)
        .output()
        .expect("run mypy");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // `path:line: error: message  [code]`; the `note:` follow-ups name the same
    // file, so keeping only `error:` lines is enough.
    collect_paths(
        stdout
            .lines()
            .filter(|line| line.contains(": error:"))
            .map(|line| line.split(':').next().unwrap_or_default()),
    )
}

/// Which of `targets` the pinned ty still reports a diagnostic in.
///
/// Warnings count: ty answers an ignore comment it cannot use with
/// `unused-ignore-comment` or `invalid-ignore-comment`, and a fixture that earned
/// one is not proving what it claims to.
pub fn ty_failures(cwd: &Path, config: &str, targets: &[&str]) -> Vec<String> {
    let output = Command::new(tool_binary("ty"))
        .current_dir(cwd)
        .args(["check", "--config-file", config])
        .args(["--output-format", "concise", "--color", "never"])
        .args(targets)
        .output()
        .expect("run ty check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // `path:line:column: error[rule] message`
    collect_paths(stdout.lines().filter_map(|line| {
        let mut fields = line.splitn(4, ':');
        let path = fields.next()?;
        fields.next()?.parse::<u32>().ok()?;
        fields.next()?.parse::<u32>().ok()?;
        fields
            .next()?
            .trim_start()
            .starts_with(['e', 'w'])
            .then_some(path)
    }))
}

/// Which of `targets` the pinned pyright still reports a diagnostic in.
///
/// `--outputjson` is what makes this parseable *and* silent: the wrapper skips
/// its "a newer pyright exists" notice — and the PyPI lookup behind it — whenever
/// the output has to be machine-readable.
pub fn pyright_failures(cwd: &Path, project: &str, targets: &[&str]) -> Vec<String> {
    let output = Command::new(tool_binary("pyright"))
        .current_dir(cwd)
        .env("PYRIGHT_PYTHON_IGNORE_WARNINGS", "1")
        .args(["--project", project, "--outputjson"])
        .args(targets)
        .output()
        .expect("run pyright");
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "pyright did not emit JSON: {error}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
    let root = cwd.canonicalize().expect("canonical fixture root");
    let diagnostics = report["generalDiagnostics"]
        .as_array()
        .expect("pyright reports generalDiagnostics")
        .iter()
        .map(|diagnostic| {
            let file = Path::new(diagnostic["file"].as_str().expect("a diagnostic file"));
            display_path(file.strip_prefix(&root).unwrap_or(file))
        })
        .collect::<Vec<_>>();
    collect_paths(diagnostics.iter().map(String::as_str))
}

/// Normalize, deduplicate, and sort the paths a checker named.
fn collect_paths<'a>(paths: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut found: Vec<String> = paths.map(display_path_str).collect();
    found.sort();
    found.dedup();
    found
}

fn display_path(path: &Path) -> String {
    display_path_str(&path.to_string_lossy())
}

fn display_path_str(path: &str) -> String {
    path.replace('\\', "/")
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
