//! Shared plumbing for the end-to-end journeys.
//!
//! Everything here resolves *real* artifacts: the compiled `notignored` binary,
//! the pinned `ruff`, `mypy`, `pyright`, and `ty` installs, the pinned `eslint` /
//! `biome` / `tsc` install, the pinned `rustc`, and real `git`. Nothing is
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

/// Run the pinned biome over `file` with `rule` as the sole rule in play.
///
/// Biome resolves its configuration by walking up from the working directory, so
/// the run happens inside the fixture directory with an explicit `--config-path`:
/// that pins the linter to the fixture's own `biome.json` and keeps any config
/// above the repo out of the result. `--only` makes the rule under test the sole
/// one in play.
fn biome_lint(file: &Path, rule: &str, extra: &[&str]) -> std::process::Output {
    let directory = file.parent().expect("a fixture directory");
    let name = file.file_name().expect("a fixture file name");
    let output = Command::new(js_binary("biome"))
        .current_dir(directory)
        .args([
            "lint",
            "--config-path=biome.json",
            &format!("--only={rule}"),
        ])
        .args(extra)
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
    output
}

/// Whether the pinned biome found any violation in `file`.
pub fn biome_passes(file: &Path, rule: &str) -> bool {
    biome_lint(file, rule, &[]).status.success()
}

/// Every diagnostic the pinned biome emits for `file`, as `(category, message,
/// start line)`, newest-reported first as biome orders them.
///
/// The pretty output biome writes to stderr is meant for humans and re-flows; its
/// JSON reporter carries the same text as data, so an assertion here is on
/// biome's own wording rather than on a rendering of it.
///
/// This is what proves the `biome-ignore-start` / `-end` pairing rule: a
/// mismatched or unclosed range is only a *warning*, so biome still exits 0 and
/// [`biome_passes`] cannot tell the two apart.
pub fn biome_diagnostics(file: &Path, rule: &str) -> Vec<(String, String, u64)> {
    let output = biome_lint(file, rule, &["--reporter=json", "--colors=off"]);
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "biome did not emit a JSON report: {error}\n{}",
                String::from_utf8_lossy(&output.stdout)
            )
        });
    report["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("biome's JSON report has no diagnostics array: {report:#}"))
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["category"].as_str().unwrap_or_default().into(),
                diagnostic["message"].as_str().unwrap_or_default().into(),
                diagnostic["location"]["start"]["line"]
                    .as_u64()
                    .unwrap_or_default(),
            )
        })
        .collect()
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

/// Which of `targets` the pinned mypy still reports a problem in.
///
/// Paths come back `/`-separated and relative to `cwd` (see [`relative_to`]),
/// deduplicated and sorted, so a test can assert on the whole set at once.
/// `--config-file` is explicit and the cache goes to a scratch directory: neither
/// a developer's own mypy settings nor a stale cache may decide the verdict.
pub fn mypy_failures(cwd: &Path, config: &str, targets: &[&str]) -> Vec<String> {
    let output = run_mypy(cwd, config, targets);
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

/// Whether the pinned mypy gives `target` a clean bill of health.
///
/// [`mypy_failures`] insists mypy exited 0 or 1, because for a well-formed
/// fixture anything else means the run itself broke. A *malformed* directive is
/// the one case where that guard is wrong: mypy may refuse the file outright
/// rather than diagnose it, and "mypy did not pass this" is the claim either way.
pub fn mypy_passes(cwd: &Path, config: &str, target: &str) -> bool {
    run_mypy(cwd, config, &[target]).status.success()
}

/// Run the pinned mypy over `targets` with the developer's own settings and any
/// stale cache kept out of the verdict.
fn run_mypy(cwd: &Path, config: &str, targets: &[&str]) -> std::process::Output {
    let cache = tempfile::tempdir().expect("scratch mypy cache");
    Command::new(tool_binary("mypy"))
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
        .expect("run mypy")
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

/// A `git` invocation rooted at `dir` and insulated from an ambient repository.
///
/// `-C` does not outrank `GIT_DIR`, and the merge-path gate runs this suite from
/// inside a `pre-push` hook — which exports `GIT_DIR` and `GIT_INDEX_FILE` for
/// the repository being pushed. Inherited, they turn every scratch repository
/// below into the real one: `git init` reuses it and `checkout -b main` fails on
/// a branch the journey never created. Mirrors `diff.rs`'s production list.
fn git_command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(name);
    }
    command
}

/// Run `git` in `dir`, failing with the command that broke and git's own reason.
///
/// The diff journeys drive a **real** repository: a stubbed git would prove
/// nothing about the semantics `--diff-base` promises.
pub fn git(dir: &Path, args: &[&str]) {
    let output = git_command(dir)
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

/// Run `git` in `dir` and return its stdout, for a journey that needs to read
/// the change the way git itself describes it.
pub fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = git_command(dir)
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
