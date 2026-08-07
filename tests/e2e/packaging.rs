//! The registry install paths, executed — not read.
//!
//! `pip install notignored-cli` and `npm install -g notignored-cli` promise a
//! prebuilt binary: no Rust toolchain, no compile, seconds. Nothing about that
//! promise is visible in the source tree — it lives in `pyproject.toml`, in
//! `scripts/npm-build.mjs`, and in npm's optional-dependency resolution — and it
//! is otherwise only ever exercised by a published release, where a mistake is
//! already public. So these journeys build both packages from the **real
//! compiled `notignored`**, install them the way a user does, and run what came
//! out. Nothing between the manifest and the artifact is stubbed.
//!
//! The one substitution is *where* the packages come from: a local wheel and
//! local tarballs rather than PyPI and npm. The deterministic gate has to stay
//! offline, so every install runs `--offline` — which also means a step that
//! quietly reached for a registry fails here instead of passing for the wrong
//! reason. Publishing those same artifacts is `release.yml`'s job, held still by
//! [`packaging_contract`](../packaging_contract.rs).
//!
//! Both journeys need the host to be one of the five released targets, which is
//! every platform CI runs and every platform the packages exist for.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::support::{repo_root, tool_binary};

/// The Rust target triple for the host, as `release.yml`'s matrices spell it.
fn host_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        (os, arch) => panic!(
            "no released package for {os}/{arch}\n\
             ACTION: run this suite on one of the five targets release.yml builds, \
             or add {os}/{arch} to that matrix, to scripts/npm-build.mjs, and to \
             npm/notignored/bin/notignored.js"
        ),
    }
}

/// The npm platform package the host's target produces.
fn host_platform_package() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => panic!("unmapped architecture {other}"),
    };
    // npm names the platform after `process.platform`, which is `win32`/`darwin`,
    // not Rust's `windows`/`macos`.
    let platform = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    format!("notignored-cli-{platform}-{arch}")
}

/// The version in `Cargo.toml`'s `[package]` section — the only version source
/// either package is allowed to have.
fn cargo_version() -> String {
    let toml = std::fs::read_to_string(repo_root().join("Cargo.toml")).expect("read Cargo.toml");
    let package = toml.split("[package]").nth(1).expect("a [package] section");
    package
        .split("\n[")
        .next()
        .unwrap_or(package)
        .lines()
        .find_map(|line| line.trim().strip_prefix("version"))
        .and_then(|rest| rest.split('"').nth(1))
        .expect("[package] declares a version")
        .to_string()
}

/// Run `command`, returning its stdout — or panic with everything it printed.
fn run(what: &str, command: &mut Command) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("cannot run {what}: {error}"));
    assert!(
        output.status.success(),
        "{what} failed ({})\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// How `program` is spelled on this platform's PATH.
///
/// npm ships as a *batch file* on Windows (`npm.cmd`), and Windows only appends
/// `.exe` when it resolves a bare program name — so `Command::new("npm")` never
/// spawns there, however npm was installed. Naming the `.cmd` is what a shell
/// does for the user, and Rust escapes a batch file's arguments for `cmd.exe`
/// when the program is spelled that way. `node`, `uv`, and maturin are real
/// executables and need no such spelling.
fn program_name(program: &str) -> &str {
    match program {
        "npm" if cfg!(windows) => "npm.cmd",
        other => other,
    }
}

/// The `node` / `npm` / `uv` the journeys drive.
///
/// All three are `just bootstrap` prerequisites (`scripts/setup-js.sh` needs
/// Node, the Python and misc installers need uv), so a missing one is a setup
/// problem with a fix, not a reason to skip: a packaging journey that silently
/// stopped running would report an unproven install path as proven.
fn required(program: &str) -> Command {
    let name = program_name(program);
    let found = Command::new(name).arg("--version").output().is_ok();
    assert!(
        found,
        "{name} not found on PATH\nACTION: run `just bootstrap`"
    );
    Command::new(name)
}

/// A file with one suppression in it, for smoke-testing an installed binary.
fn smoke_file(dir: &Path) -> PathBuf {
    let path = dir.join("app.py");
    std::fs::write(&path, "x = 1  # noqa: E501  # smoke test\n").expect("write the smoke fixture");
    path
}

/// Assert an installed `notignored` is the real thing: this repo's version, and
/// a scan that finds the suppression in `smoke_file`.
fn assert_works(what: &str, command: impl Fn() -> Command, scratch: &Path) {
    let version = run(
        &format!("{what} --version"),
        command().current_dir(scratch).arg("--version"),
    );
    assert_eq!(
        version,
        format!("notignored {}", cargo_version()),
        "{what} reports a different version than Cargo.toml declares"
    );

    let fixture = smoke_file(scratch);
    let report = run(
        &format!("{what} scan"),
        command()
            .current_dir(scratch)
            .arg(&fixture)
            .args(["--format", "json"]),
    );
    assert!(
        report.contains("\"E501\"") && report.contains("\"smoke test\""),
        "{what} did not report the fixture's suppression:\n{report}"
    );
}

/// Assemble a package directory with `scripts/npm-build.mjs`, returning its path.
fn npm_build(mode: &str, args: &[&str]) -> PathBuf {
    let out = run(
        &format!("npm-build.mjs {mode}"),
        required("node")
            .current_dir(repo_root())
            .arg("scripts/npm-build.mjs")
            .arg(mode)
            .args(args),
    );
    PathBuf::from(out)
}

/// `npm pack` a package directory, returning the tarball.
///
/// Each package packs into its own directory under `into`: the launcher and the
/// platform packages share a name prefix (`notignored-cli-…`), so a shared
/// destination makes "the tarball for this package" ambiguous.
fn npm_pack(package: &Path, into: &Path) -> PathBuf {
    let destination = into.join(package_name(package));
    std::fs::create_dir_all(&destination).expect("create the pack destination");
    run(
        "npm pack",
        required("npm")
            .current_dir(package)
            .arg("pack")
            .arg("--pack-destination")
            .arg(&destination),
    );
    std::fs::read_dir(&destination)
        .expect("read the pack destination")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "tgz"))
        .unwrap_or_else(|| panic!("npm pack produced no tarball in {}", destination.display()))
}

/// The `name` field of an assembled package.
fn package_name(package: &Path) -> String {
    let manifest =
        std::fs::read_to_string(package.join("package.json")).expect("read package.json");
    let value: serde_json::Value = serde_json::from_str(&manifest).expect("valid package.json");
    value["name"].as_str().expect("a package name").to_string()
}

/// The `version` field of an assembled package.
fn package_version(package: &Path) -> String {
    let manifest =
        std::fs::read_to_string(package.join("package.json")).expect("read package.json");
    let value: serde_json::Value = serde_json::from_str(&manifest).expect("valid package.json");
    value["version"]
        .as_str()
        .expect("a package version")
        .to_string()
}

/// `npm install --global --prefix <prefix>`, offline, from local tarballs.
fn npm_install(prefix: &Path, tarballs: &[&Path], extra: &[&str]) -> std::process::Output {
    std::fs::create_dir_all(prefix).expect("create the install prefix");
    required("npm")
        .arg("install")
        // Offline keeps the gate hermetic: everything being installed is a local
        // tarball, so a step that reached for the registry is a bug, not latency.
        .arg("--offline")
        .arg("--global")
        .arg("--prefix")
        .arg(prefix)
        .args(extra)
        .args(tarballs)
        .output()
        .expect("run npm install")
}

/// The `notignored` command an `npm install --global --prefix` produced.
///
/// npm links the launcher into the prefix itself: a symlink under `bin/` on
/// Unix, a `.cmd` batch file at the prefix root on Windows. Running that link —
/// rather than the shim's `.js` through node — is what makes this the journey a
/// user takes. A missing link reports what npm *did* write, so a future layout
/// change reads as a layout change rather than a spawn error.
fn installed_npm_command(prefix: &Path) -> Command {
    let path = if cfg!(windows) {
        prefix.join("notignored.cmd")
    } else {
        prefix.join("bin").join("notignored")
    };
    assert!(
        path.exists(),
        "npm linked no command at {}; the prefix holds {:?}",
        path.display(),
        entries(prefix)
    );
    Command::new(path)
}

/// Every path under `dir`, one level deep — a diagnostic, not a fixture.
fn entries(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect()
}

/// The whole npm install path: build both packages from the compiled binary,
/// pack them, install them into a scratch prefix, and run what npm linked.
///
/// This is the one place the launcher's optional-dependency resolution is real:
/// npm — not the test — decides that the platform package satisfies the
/// launcher's dependency and where to put it, and the committed shim has to find
/// the binary in whatever tree npm built.
#[test]
fn the_npm_package_installs_and_runs_the_prebuilt_binary() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let binary = assert_cmd::cargo::cargo_bin("notignored");

    let platform = npm_build(
        "platform",
        &[
            "--target",
            host_target(),
            "--binary",
            binary.to_str().expect("a UTF-8 binary path"),
            "--out",
            scratch.path().join("dist").to_str().expect("a UTF-8 path"),
        ],
    );
    let launcher = npm_build(
        "launcher",
        &[
            "--out",
            scratch.path().join("dist").to_str().expect("a UTF-8 path"),
        ],
    );

    // Cargo.toml is the single version source: npm-build.mjs read it, and the
    // committed manifests only ever held the `0.0.0-managed` placeholder.
    assert_eq!(package_name(&platform), host_platform_package());
    assert_eq!(package_version(&platform), cargo_version());
    assert_eq!(package_name(&launcher), "notignored-cli");
    assert_eq!(package_version(&launcher), cargo_version());
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(launcher.join("package.json")).unwrap())
            .expect("the stamped launcher manifest is valid JSON");
    assert_eq!(
        manifest["optionalDependencies"][host_platform_package()],
        serde_json::json!(cargo_version()),
        "the launcher does not pin this platform's package at the crate version"
    );

    let tarballs = scratch.path().join("tarballs");
    let platform_tgz = npm_pack(&platform, &tarballs);
    let launcher_tgz = npm_pack(&launcher, &tarballs);

    let prefix = scratch.path().join("prefix");
    let install = npm_install(&prefix, &[&platform_tgz, &launcher_tgz], &[]);
    assert!(
        install.status.success(),
        "npm install failed\n{}",
        String::from_utf8_lossy(&install.stderr)
    );

    assert_works(
        "the npm-installed notignored",
        || installed_npm_command(&prefix),
        scratch.path(),
    );
}

/// Installed without its platform package, the launcher says what to do about it.
///
/// `--omit=optional` is the shape of every real report of this: a lockfile that
/// skipped optional dependencies, a `--no-optional` CI install, a host npm has no
/// package for. The binary genuinely is not there, so the only thing the shim can
/// do is fail *legibly* — and a shim that instead crashed with a module-resolution
/// stack trace would send that user to the wrong repository.
#[test]
fn the_npm_launcher_explains_a_missing_platform_package() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let launcher = npm_build(
        "launcher",
        &[
            "--out",
            scratch.path().join("dist").to_str().expect("a UTF-8 path"),
        ],
    );
    let tarball = npm_pack(&launcher, &scratch.path().join("tarballs"));

    let prefix = scratch.path().join("prefix");
    let install = npm_install(&prefix, &[&tarball], &["--omit=optional"]);
    assert!(
        install.status.success(),
        "npm install failed\n{}",
        String::from_utf8_lossy(&install.stderr)
    );

    let output = installed_npm_command(&prefix)
        .arg("--version")
        .output()
        .expect("run the launcher");
    assert_eq!(
        output.status.code(),
        Some(1),
        "the launcher must exit 1 when it cannot find its binary"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&host_platform_package()) && stderr.contains("optional dependencies"),
        "the launcher did not name the missing platform package:\n{stderr}"
    );
    assert!(
        stderr.contains("pip install notignored-cli"),
        "the launcher did not offer another way to install:\n{stderr}"
    );
}

/// The target directory the wheel build gets to itself.
///
/// **Not** the suite's own. `maturin build` runs cargo, and `[tool.maturin]`
/// sets `strip = true`, so a build in the default directory *replaces*
/// `target/debug/notignored` with a stripped one — mid-run, while nextest is
/// still starting tests that resolve exactly that path. Every sibling journey
/// then either fails to spawn it (the file is briefly gone) or silently runs a
/// different binary than the one the suite compiled. Under `target/` so it is
/// already ignored and stays warm between runs.
fn wheel_target_dir() -> PathBuf {
    repo_root().join("target").join("packaging-e2e")
}

/// The binary the wheel build compiles, inside its own target directory.
fn wheel_built_binary() -> PathBuf {
    let name = if cfg!(windows) {
        "notignored.exe"
    } else {
        "notignored"
    };
    wheel_target_dir().join("debug").join(name)
}

/// Build the wheel from this repo's `pyproject.toml` with the pinned maturin,
/// into `dist`, and return the wheel it wrote.
///
/// Debug rather than `--release`: what is under test is the packaging — the
/// `bin` bindings, the dynamic version, the console command's name — not the
/// optimizer.
fn build_wheel(dist: &Path) -> PathBuf {
    run(
        "maturin build",
        Command::new(tool_binary("maturin"))
            .current_dir(repo_root())
            .env("CARGO_TARGET_DIR", wheel_target_dir())
            .arg("build")
            .arg("--locked")
            .arg("--out")
            .arg(dist),
    );
    std::fs::read_dir(dist)
        .expect("read the wheel output directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "whl"))
        .expect("maturin built a wheel")
}

/// What a file is, for deciding whether something replaced it.
fn fingerprint(path: &Path) -> (u64, std::time::SystemTime) {
    let meta =
        std::fs::metadata(path).unwrap_or_else(|error| panic!("stat {}: {error}", path.display()));
    (meta.len(), meta.modified().expect("a modification time"))
}

/// The whole PyPI install path: build the wheel from this repo's
/// `pyproject.toml`, install it into a scratch venv, and run the console command
/// it put on that venv's PATH — and prove the build did not disturb the binary
/// the rest of this suite is running.
///
/// That last part is a regression, not a nicety. To stage the binary into the
/// wheel, maturin **renames it out** of `<target>/debug/` and puts it back when
/// it is done. Pointed at the suite's own target directory, that takes
/// `target/debug/notignored` away mid-run: sibling journeys resolve exactly that
/// path as they start, so the ones that start inside the window die with
/// `NotFoundError { path: ".../target/debug/notignored" }` — from tests that
/// have nothing to do with packaging. It surfaced as a macOS-only failure in the
/// ShellCheck parity journeys and reproduces on Linux about two runs in three.
///
/// Both halves are asserted because either alone could pass for the wrong
/// reason: the build landing in its own directory is what fails everywhere if
/// [`wheel_target_dir`] is ever dropped, and the suite's own binary being
/// untouched is what catches the theft actually happening. They live inside this
/// journey rather than in a test of their own because two maturin builds sharing
/// one target directory race on that same rename — the isolation has to be one
/// build, not one per assertion.
#[test]
fn the_pypi_wheel_installs_and_runs_the_prebuilt_binary() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let dist = scratch.path().join("dist");

    let shared = assert_cmd::cargo::cargo_bin("notignored");
    let before = fingerprint(&shared);

    let wheel = build_wheel(&dist);

    assert!(
        wheel_built_binary().exists(),
        "the wheel build did not compile into {}; it used the suite's own target \
         directory instead",
        wheel_built_binary().display()
    );
    assert_eq!(
        fingerprint(&shared),
        before,
        "the wheel build disturbed {}, which every other journey in this suite \
         spawns while it runs",
        shared.display()
    );

    let name = wheel
        .file_name()
        .expect("the wheel has a name")
        .to_string_lossy()
        .to_string();
    // The distribution name and the version PyPI will index it under, both read
    // off the artifact rather than a manifest.
    assert!(
        name.starts_with(&format!("notignored_cli-{}-", cargo_version())),
        "the wheel is not notignored_cli at the crate version: {name}"
    );

    let venv = scratch.path().join("venv");
    run(
        "uv venv",
        required("uv").arg("venv").arg("--quiet").arg(&venv),
    );
    run(
        "uv pip install",
        required("uv")
            .args(["pip", "install", "--quiet", "--offline", "--python"])
            .arg(&venv)
            .arg(&wheel),
    );

    // The wheel's console command, on the venv's PATH exactly as `pip install`
    // puts it there.
    let bin = if cfg!(windows) {
        venv.join("Scripts").join("notignored.exe")
    } else {
        venv.join("bin").join("notignored")
    };
    assert!(
        bin.exists(),
        "the wheel did not install a `notignored` command at {}",
        bin.display()
    );
    assert_works(
        "the wheel-installed notignored",
        || Command::new(&bin),
        scratch.path(),
    );
}

/// Every argument the package builder refuses, and what it tells the caller.
///
/// These are the boundaries `scripts/npm-build.mjs` guards: an option it does not
/// implement, a version neither registry could index, a target it has no package
/// for, and a binary that is not there. It runs inside a release job, so the only
/// diagnosis anyone gets is what it printed — which is why each refusal owes an
/// `ACTION:` line as much as it owes a non-zero exit.
#[test]
fn the_package_builder_refuses_input_it_cannot_turn_into_a_package() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let out = scratch.path().join("dist");
    let out = out.to_str().expect("a UTF-8 path");
    let binary = assert_cmd::cargo::cargo_bin("notignored");
    let binary = binary.to_str().expect("a UTF-8 binary path");

    for (what, args, expected) in [
        (
            "an option no mode implements",
            vec!["launcher", "--bogus", "x", "--out", out],
            "unknown option --bogus",
        ),
        (
            "an option given no value",
            vec!["launcher", "--out"],
            "--out needs a value",
        ),
        (
            // The one that matters: a version carrying a specifier would publish
            // under a name no consumer could ask for.
            "a version neither registry can index",
            vec![
                "launcher",
                "--version",
                "1.2.3 --registry evil",
                "--out",
                out,
            ],
            "is not a version either registry can index",
        ),
        (
            "a version with a repeated suffix",
            vec!["launcher", "--version", "1.2.3+one+two", "--out", out],
            "is not a version either registry can index",
        ),
        (
            "a target with no platform package",
            vec![
                "platform",
                "--target",
                "sparc64-unknown-linux-gnu",
                "--binary",
                binary,
                "--out",
                out,
            ],
            "unknown target sparc64-unknown-linux-gnu",
        ),
        (
            "a binary that was never built",
            vec![
                "platform",
                "--target",
                host_target(),
                "--binary",
                "target/debug/never-built",
                "--out",
                out,
            ],
            "binary not found",
        ),
        (
            "a mode it does not have",
            vec!["sideload"],
            "unknown mode sideload",
        ),
    ] {
        let output = required("node")
            .current_dir(repo_root())
            .arg("scripts/npm-build.mjs")
            .args(&args)
            .output()
            .expect("run npm-build.mjs");
        assert!(
            !output.status.success(),
            "{what} was accepted: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
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

/// A platform package whose binary will not exec fails legibly, not with a stack
/// trace.
///
/// npm can deliver a package whose payload is unusable — an archive unpacked
/// without the executable bit, a partially written file, a filesystem mounted
/// `noexec`. The shim has already resolved the package by then, so this is a
/// different failure from a missing one, and the user is one message away from
/// filing it against the wrong repository. Replacing the binary with a directory
/// is the one form of "resolves but will not run" that behaves the same for
/// every user, root included.
#[test]
fn the_npm_launcher_explains_a_binary_it_cannot_run() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let binary = assert_cmd::cargo::cargo_bin("notignored");
    let dist = scratch.path().join("dist");
    let dist = dist.to_str().expect("a UTF-8 path");

    let platform = npm_build(
        "platform",
        &[
            "--target",
            host_target(),
            "--binary",
            binary.to_str().expect("a UTF-8 binary path"),
            "--out",
            dist,
        ],
    );
    let launcher = npm_build("launcher", &["--out", dist]);
    let tarballs = scratch.path().join("tarballs");
    let platform_tgz = npm_pack(&platform, &tarballs);
    let launcher_tgz = npm_pack(&launcher, &tarballs);

    let prefix = scratch.path().join("prefix");
    let install = npm_install(&prefix, &[&platform_tgz, &launcher_tgz], &[]);
    assert!(
        install.status.success(),
        "npm install failed\n{}",
        String::from_utf8_lossy(&install.stderr)
    );

    let name = if cfg!(windows) {
        "notignored.exe"
    } else {
        "notignored"
    };
    let installed = prefix
        .join(if cfg!(windows) { "" } else { "lib" })
        .join("node_modules")
        .join(host_platform_package())
        .join("bin")
        .join(name);
    std::fs::remove_file(&installed).expect("remove the installed binary");
    std::fs::create_dir(&installed).expect("put a directory where the binary was");

    let output = installed_npm_command(&prefix)
        .arg("--version")
        .output()
        .expect("run the launcher");
    assert_eq!(
        output.status.code(),
        Some(1),
        "the launcher must exit 1 when its binary will not run"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to launch the notignored binary"),
        "the launcher did not say it could not start the binary:\n{stderr}"
    );
}
