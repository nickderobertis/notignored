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

/// The whole PyPI install path: build the wheel with the pinned maturin from this
/// repo's `pyproject.toml`, install it into a scratch venv, and run the console
/// command it put on that venv's PATH.
///
/// The wheel is built from whatever the suite already compiled rather than
/// `--release`: what is under test is the packaging — the `bin` bindings, the
/// dynamic version, the console command's name — not the optimizer.
#[test]
fn the_pypi_wheel_installs_and_runs_the_prebuilt_binary() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let dist = scratch.path().join("dist");

    run(
        "maturin build",
        Command::new(tool_binary("maturin"))
            .current_dir(repo_root())
            .arg("build")
            .arg("--locked")
            .arg("--out")
            .arg(&dist),
    );

    let wheel = std::fs::read_dir(&dist)
        .expect("read the wheel output directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "whl"))
        .expect("maturin built a wheel");
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
