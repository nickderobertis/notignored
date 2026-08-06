//! Shared plumbing for the end-to-end journeys.
//!
//! Everything here resolves *real* artifacts: the compiled `notignored` binary,
//! the pinned `ruff` install, and the pinned `eslint` / `biome` / `tsc` install.
//! Nothing is stubbed — if a prerequisite is missing the suite fails with an
//! actionable message rather than skipping, so a green run always means the
//! journeys actually ran.

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

/// The npm package, and the exact `--version` line, behind each JS tool binary.
///
/// The version line is spelled out per tool rather than substring-matched: a
/// pinned `2.5.7` must not be satisfied by an installed `12.5.71`.
const JS_TOOLS: [(&str, &str, &str); 3] = [
    ("eslint", "eslint", "v{version}"),
    ("biome", "@biomejs/biome", "Version: {version}"),
    ("tsc", "typescript", "Version {version}"),
];

/// The version `tests/js-toolchain/package.json` pins for `package`.
///
/// The manifest holds exact versions (no `^`), so it is the pin the lockfile
/// resolves and the tests assert against.
pub fn pinned_js_version(package: &str) -> String {
    let path = repo_root().join("tests/js-toolchain/package.json");
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()));
    manifest["dependencies"][package]
        .as_str()
        .unwrap_or_else(|| panic!("{} pins no version for {package}", path.display()))
        .to_string()
}

/// A binary from the pinned JS toolchain installed by `scripts/setup-js.sh`.
///
/// Panics with the fix when it is missing or the wrong version — a parity test
/// that silently skipped would report an unproven claim as proven.
pub fn js_binary(name: &str) -> PathBuf {
    let (_, package, version_line) = JS_TOOLS
        .iter()
        .find(|(tool, _, _)| *tool == name)
        .unwrap_or_else(|| panic!("{name} is not part of the pinned JS toolchain"));
    let bin_dir = repo_root().join(".dev/js/node_modules/.bin");
    // On Windows npm writes a `.cmd` shim beside a POSIX shell script of the
    // same name; only the shim is executable there.
    let candidates = if cfg!(windows) {
        vec![bin_dir.join(format!("{name}.cmd")), bin_dir.join(name)]
    } else {
        vec![bin_dir.join(name)]
    };
    let binary = candidates.iter().find(|path| path.exists()).unwrap_or_else(|| {
        panic!(
            "pinned {name} not installed under {}\nACTION: run `just bootstrap` (or ./scripts/setup-js.sh)",
            bin_dir.display()
        )
    });

    let expected = version_line.replace("{version}", &pinned_js_version(package));
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("run {name} --version: {error}"));
    let reported = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(
        reported, expected,
        "the installed {name} is not the pinned one\nACTION: re-run ./scripts/setup-js.sh"
    );
    binary.clone()
}

/// Run the pinned eslint over `file` with exactly `rules` in play, returning its
/// JSON result for that file.
///
/// `--no-config-lookup` keeps the developer's own `eslint.config.js` out of the
/// result and `--rule` makes the rules under test the only ones enabled — so a
/// pass/fail flip is caused by the suppression and nothing else. The JSON
/// formatter is what carries `suppressedMessages`, which is ESLint's *own* read
/// of each directive and its ` -- ` description.
pub fn eslint_result(file: &Path, rules: &[&str]) -> serde_json::Value {
    let config = format!(
        "{{{}}}",
        rules
            .iter()
            .map(|rule| format!("\"{rule}\":\"error\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    let output = Command::new(js_binary("eslint"))
        .args(["--no-config-lookup", "--format", "json", "--rule", &config])
        .arg(file)
        .output()
        .expect("run eslint");
    assert!(
        output
            .status
            .code()
            .is_some_and(|code| code == 0 || code == 1),
        "eslint exited unexpectedly ({:?}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let results: serde_json::Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "eslint did not emit a JSON report: {error}\n{}",
                String::from_utf8_lossy(&output.stdout)
            )
        });
    results
        .as_array()
        .and_then(|files| files.first())
        .cloned()
        .unwrap_or_else(|| panic!("eslint reported nothing for {}", file.display()))
}

/// Whether the pinned eslint found any problem in `file`.
///
/// ESLint counts an *unused* disable directive as a problem too, so a fixture
/// only passes when every directive it carries actually suppressed something.
pub fn eslint_passes(file: &Path, rules: &[&str]) -> bool {
    eslint_result(file, rules)["messages"]
        .as_array()
        .is_some_and(|messages| messages.is_empty())
}

/// Run the pinned biome over `file`, returning whether it found any violation.
///
/// Biome resolves its configuration by walking up from the working directory, so
/// the run happens inside the fixture directory with an explicit `--config-path`:
/// that pins the linter to the fixture's own `biome.json` and keeps any config
/// above the repo out of the result. `--only` makes the rule under test the sole
/// one in play.
pub fn biome_passes(file: &Path, rule: &str) -> bool {
    let directory = file.parent().expect("a fixture directory");
    let name = file.file_name().expect("a fixture file name");
    let output = Command::new(js_binary("biome"))
        .current_dir(directory)
        .args([
            "lint",
            "--config-path=biome.json",
            &format!("--only={rule}"),
        ])
        .arg(name)
        .output()
        .expect("run biome lint");
    assert!(
        output
            .status
            .code()
            .is_some_and(|code| code == 0 || code == 1),
        "biome exited unexpectedly ({:?}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.status.success()
}

/// Run the pinned tsc over `file`, returning whether it type-checked cleanly.
///
/// Passing the file on the command line makes tsc ignore any `tsconfig.json`, and
/// `--strict` is what turns the fixtures' assignments into errors. An unused
/// `@ts-expect-error` is itself an error (TS2578), so a fixture only passes when
/// every directive it carries actually suppressed something.
pub fn tsc_passes(file: &Path) -> bool {
    let output = Command::new(js_binary("tsc"))
        .args(["--noEmit", "--strict"])
        .arg(file)
        .output()
        .expect("run tsc");
    assert!(
        output
            .status
            .code()
            .is_some_and(|code| code == 0 || code == 1),
        "tsc exited unexpectedly ({:?}): {}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output.status.success()
}

/// Collapse whitespace the way the report's `reason` field does.
///
/// A tool that parses its own reason (ESLint's ` -- ` description) hands it back
/// with the source's line breaks and indentation intact; this is what makes the
/// two comparable.
pub fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
