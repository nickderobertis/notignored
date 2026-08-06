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
/// Paths come back `/`-separated and relative to `cwd` (see [`relative_to`]),
/// deduplicated and sorted, so a test can assert on the whole set at once.
/// `--config-file` is explicit and the cache goes to a scratch directory: neither
/// a developer's own mypy settings nor a stale cache may decide the verdict.
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
    // 0 is clean and 1 is "found errors"; anything else (a missing fixture, a bad
    // config) would otherwise read as "every file passed".
    assert_checker_ran("mypy", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // `path:line: error: message  [code]`; the `note:` follow-ups name the same
    // file, so keeping only `error:` lines is enough.
    collect_paths(
        cwd,
        stdout
            .lines()
            .filter(|line| line.contains(": error:"))
            .filter_map(|line| split_path_field(line).map(|(path, _)| path)),
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
    assert_checker_ran("ty", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    collect_paths(cwd, stdout.lines().filter_map(ty_diagnostic_path))
}

/// The file named by one `path:line:column: error[rule] message` line, or `None`
/// for the summary and continuation lines around it.
fn ty_diagnostic_path(line: &str) -> Option<&str> {
    let (path, rest) = split_path_field(line)?;
    let mut fields = rest.splitn(3, ':');
    fields.next()?.parse::<u32>().ok()?;
    fields.next()?.parse::<u32>().ok()?;
    fields
        .next()?
        .trim_start()
        .starts_with(['e', 'w'])
        .then_some(path)
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
    let diagnostics = report["generalDiagnostics"]
        .as_array()
        .expect("pyright reports generalDiagnostics")
        .iter()
        .map(|diagnostic| diagnostic["file"].as_str().expect("a diagnostic file"))
        .collect::<Vec<_>>();
    collect_paths(cwd, diagnostics.into_iter())
}

/// A checker that could not run at all must fail the test, not read as a clean
/// bill of health.
///
/// Every checker here exits 0 for "nothing to report" and 1 for "found
/// something"; anything else is a missing fixture, a bad config, or a crash —
/// and all three otherwise surface as an empty diagnostic list.
fn assert_checker_ran(tool: &str, output: &std::process::Output) {
    assert!(
        output
            .status
            .code()
            .is_some_and(|code| code == 0 || code == 1),
        "{tool} exited unexpectedly ({:?})\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Normalize, deduplicate, and sort the paths a checker named, all relative to
/// the directory it ran in.
fn collect_paths<'a>(cwd: &Path, paths: impl Iterator<Item = &'a str>) -> Vec<String> {
    // The absolute paths a checker emits are already symlink-resolved, so the
    // root has to be too or nothing lines up.
    let root = cwd
        .canonicalize()
        .unwrap_or_else(|error| panic!("canonical checker root {}: {error}", cwd.display()));
    let mut found: Vec<String> = paths.map(|path| relative_to(&root, path)).collect();
    found.sort();
    found.dedup();
    found
}

/// The `/`-separated, `root`-relative spelling of a path a checker reported.
///
/// Checkers disagree about how they name a file — mypy and ty echo the relative
/// target back, pyright always answers absolutely — and on Windows no two
/// spellings of the same absolute path agree either: pyright emits
/// `d:/a/notignored/…` while [`Path::canonicalize`] hands back the verbatim
/// `\\?\D:\a\notignored\…`. Drive case, separator, and that `\\?\` prefix all
/// differ, so `strip_prefix` finds nothing and the whole absolute path reaches
/// the assertion. Compare the two the way Windows itself does instead — case- and
/// separator-insensitively — and return the tail.
///
/// The tail comes back byte-for-byte, and a path that is genuinely outside `root`
/// comes back whole, so an assertion still has to name the exact file: this
/// normalizes spellings, it does not widen what counts as a match.
pub fn relative_to(root: &Path, reported: &str) -> String {
    let reported = portable(reported);
    let root = portable(&root.to_string_lossy());
    let root = root.trim_end_matches('/');
    reported
        .get(..root.len())
        .filter(|head| head.eq_ignore_ascii_case(root))
        .and_then(|_| reported[root.len()..].strip_prefix('/'))
        .unwrap_or(&reported)
        .to_string()
}

/// `path` with `/` separators and without the verbatim `\\?\` prefix, which
/// `canonicalize` adds on Windows but no checker ever emits.
fn portable(path: &str) -> String {
    let path = path.replace('\\', "/");
    let Some(rest) = path.strip_prefix("//?/") else {
        return path;
    };
    // `\\?\UNC\server\share` is the verbatim spelling of `\\server\share`.
    match rest.strip_prefix("UNC/") {
        Some(share) => format!("//{share}"),
        None => rest.to_string(),
    }
}

/// Split a `path:rest` diagnostic line at the field separator.
///
/// A Windows drive letter's colon belongs to the path — `d:\x\y.py:7:16: error`
/// names `d:\x\y.py`, not `d` — and a parser that takes the first colon either
/// loses the file or, worse, fails to recognize the line at all and reports a
/// diagnostic-free run.
fn split_path_field(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    let drive = bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':');
    // Start the search past `d:` so its colon cannot be mistaken for the split.
    let start = if drive { 2 } else { 0 };
    let colon = start + line[start..].find(':')?;
    Some((&line[..colon], &line[colon + 1..]))
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

/// The path-normalization these journeys rely on, proven on the spellings only
/// another platform emits.
///
/// The gate runs on Linux, so every Windows and UNC path below is written by
/// hand: without them the normalization is only ever exercised on the one
/// platform whose paths already happen to line up, and a `d:/…`-shaped
/// regression waits for CI to find it.
#[cfg(test)]
mod paths {
    use super::{portable, relative_to, split_path_field, ty_diagnostic_path};
    use std::path::Path;

    /// The exact pair that failed CI: pyright's forward-slashed, lowercase-drive
    /// absolute path against the verbatim root `canonicalize` returns.
    #[test]
    fn a_windows_checker_path_becomes_the_repo_relative_one() {
        let root = Path::new(r"\\?\D:\a\notignored\notignored\tests\fixtures\python-types");
        assert_eq!(
            relative_to(
                root,
                "d:/a/notignored/notignored/tests/fixtures/python-types/pyright/mode_switch.py",
            ),
            "pyright/mode_switch.py"
        );
        // The same file spelled the way Windows itself writes it.
        assert_eq!(
            relative_to(
                root,
                r"D:\a\notignored\notignored\tests\fixtures\python-types\pyright\violation.py",
            ),
            "pyright/violation.py"
        );
    }

    /// A build on a network share: `canonicalize` returns the verbatim UNC form,
    /// checkers report the plain one.
    #[test]
    fn a_windows_unc_root_matches_the_share_path_a_checker_reports() {
        assert_eq!(
            relative_to(
                Path::new(r"\\?\UNC\build\share\python-types"),
                "//build/share/python-types/ty/violation.py",
            ),
            "ty/violation.py"
        );
    }

    #[test]
    fn a_posix_checker_path_becomes_the_repo_relative_one() {
        let root = Path::new("/home/runner/work/notignored/tests/fixtures/python-types");
        assert_eq!(
            relative_to(
                root,
                "/home/runner/work/notignored/tests/fixtures/python-types/ty/next_line.py",
            ),
            "ty/next_line.py"
        );
        // mypy and ty echo the relative target back; it must survive untouched.
        assert_eq!(
            relative_to(root, "mypy/line_codes.py"),
            "mypy/line_codes.py"
        );
    }

    /// Normalizing may not turn a foreign path into a fixture-relative one, or an
    /// assertion stops naming the exact file it claims to.
    #[test]
    fn a_path_outside_the_root_stays_whole() {
        let root = Path::new("/repo/tests/fixtures/python-types");
        for outside in [
            "/repo/tests/fixtures/ruff/violation.py",
            // A sibling whose name merely starts with the root's.
            "/repo/tests/fixtures/python-types-vendored/x.py",
        ] {
            assert_eq!(relative_to(root, outside), outside);
        }
    }

    #[test]
    fn a_drive_letters_colon_is_not_a_field_separator() {
        assert_eq!(
            split_path_field(r"d:\a\x.py:7:16: error[bad-argument-type] no"),
            Some((r"d:\a\x.py", "7:16: error[bad-argument-type] no"))
        );
        assert_eq!(
            split_path_field("ty/violation.py:7: error: no"),
            Some(("ty/violation.py", "7: error: no"))
        );
        assert_eq!(split_path_field("Found 1 error in 1 file"), None);
    }

    /// The drive letter's real cost: split at the wrong colon and ty's line stops
    /// parsing at all, so a file with a diagnostic reads as a file without one.
    #[test]
    fn a_windows_ty_diagnostic_line_still_names_its_file() {
        assert_eq!(
            ty_diagnostic_path(r"d:\a\x\ty\violation.py:7:16: error[invalid-argument-type] no"),
            Some(r"d:\a\x\ty\violation.py")
        );
        assert_eq!(
            ty_diagnostic_path("ty/violation.py:7:16: warning[unused-ignore-comment] no"),
            Some("ty/violation.py")
        );
        assert_eq!(ty_diagnostic_path("Found 1 diagnostic"), None);
        // A `note:` follow-up names no new file.
        assert_eq!(ty_diagnostic_path("ty/violation.py:7:16: info: help"), None);
    }

    #[test]
    fn the_verbatim_prefix_is_only_stripped_where_windows_puts_it() {
        assert_eq!(portable(r"C:\x\y"), "C:/x/y");
        assert_eq!(portable("/x/y"), "/x/y");
        assert_eq!(portable("//server/share/x"), "//server/share/x");
    }
}
