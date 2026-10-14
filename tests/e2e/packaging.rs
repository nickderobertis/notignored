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

use crate::support::{cargo_version, repo_root, tool_binary};

/// The Rust target triple for the host, as `release.yml`'s matrices spell it.
fn host_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        (os, arch) => panic!(
            "no released package for {os}/{arch}\n\
             ACTION: run this suite on one of the four targets release.yml builds, \
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

/// Install the launcher and this platform's package into a scratch prefix,
/// returning the prefix. The journeys that exercise the shim's own behaviour all
/// need the same real npm-built tree.
fn installed_launcher(scratch: &Path) -> PathBuf {
    let binary = assert_cmd::cargo::cargo_bin("notignored");
    let dist = scratch.join("dist");
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
    let tarballs = scratch.join("tarballs");
    let platform_tgz = npm_pack(&platform, &tarballs);
    let launcher_tgz = npm_pack(&launcher, &tarballs);

    let prefix = scratch.join("prefix");
    let install = npm_install(&prefix, &[&platform_tgz, &launcher_tgz], &[]);
    assert!(
        install.status.success(),
        "npm install failed\n{}",
        String::from_utf8_lossy(&install.stderr)
    );
    prefix
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

/// On an Intel Mac the launcher refuses with the reason, rather than listing the
/// packages it does ship or hunting for a `darwin-x64` package that no release
/// carries any more.
///
/// The installed launcher runs under real node; the one double is the host's
/// identity, which a preload sets the way node reports it on an Intel Mac.
#[test]
fn the_npm_launcher_refuses_intel_macos_with_the_reason() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let launcher = npm_build(
        "launcher",
        &[
            "--out",
            scratch.path().join("dist").to_str().expect("a UTF-8 path"),
        ],
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(launcher.join("package.json")).unwrap())
            .expect("the stamped launcher manifest is valid JSON");
    let optional = manifest["optionalDependencies"]
        .as_object()
        .expect("the launcher declares its platform packages");
    assert!(
        optional.contains_key("notignored-cli-darwin-arm64")
            && !optional.contains_key("notignored-cli-darwin-x64"),
        "the launcher must depend on the arm64 Darwin package only: {optional:?}"
    );
    let tarball = npm_pack(&launcher, &scratch.path().join("tarballs"));
    let prefix = scratch.path().join("prefix");
    let install = npm_install(&prefix, &[&tarball], &["--omit=optional"]);
    assert!(
        install.status.success(),
        "npm install failed\n{}",
        String::from_utf8_lossy(&install.stderr)
    );

    // llmlint: ignore-block[e2e_not_mocked, tests_mirror_real_usage] no runner here or in CI is an Intel Mac, and the release no longer has one, so the host's identity is the one boundary that cannot be real. Only process.platform/arch are set, by a preload, before the real installed launcher runs unchanged under real node.
    let intel = scratch.path().join("intel-mac.cjs");
    std::fs::write(
        &intel,
        "Object.defineProperty(process, 'platform', { value: 'darwin' });\n\
         Object.defineProperty(process, 'arch', { value: 'x64' });\n",
    )
    .expect("write the host-identity preload");
    let output = installed_npm_command(&prefix)
        .arg("--version")
        .env(
            "NODE_OPTIONS",
            format!("--require={}", intel.to_str().expect("a UTF-8 path")),
        )
        .output()
        .expect("run the launcher");
    // llmlint: ignore-end[e2e_not_mocked, tests_mirror_real_usage]
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Intel macOS is no longer supported") && stderr.contains("cargo install"),
        "the launcher did not explain the Intel macOS refusal:\n{stderr}"
    );
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

/// The suite's own binary, held across a wheel build and put back afterwards.
///
/// `maturin build` runs cargo in this clone's one target directory (the
/// `.cargo/config.toml` contract: nothing builds anywhere else), and there it
/// disturbs `target/debug/notignored` — the path every other journey in this
/// suite spawns — twice over. Its cargo invocation asks for a `-C strip=symbols`
/// variant of the binary and uplifts *that* into place, and staging the wheel
/// then **renames the file out** to `target/maturin/` and back. A sibling that
/// spawns the path inside that window dies with `NotFoundError` from a test
/// with nothing to do with packaging; one that spawns it afterwards runs the
/// stripped variant rather than the binary the suite compiled. (Under `cargo
/// llvm-cov` the suite runs from its own `target/llvm-cov-target/debug`, which
/// the wheel build — steered by `.cargo/config.toml` alone — never reaches; the
/// uninstrumented runs, `just test-e2e` and `test-quick`, are where the two
/// collide.)
///
/// The window is closed by `.config/nextest.toml`, which runs the wheel journey
/// first and alone — every test thread reserved — so nothing resolves the path
/// while maturin holds it ([`packaging_contract`](../packaging_contract.rs)
/// holds that override to the journey's name). One journey, because a second
/// maturin build would race the first on that same rename. The replacement is
/// undone here:
/// the suite's binary is kept as a **hard link** under cargo's own `target/tmp`
/// and renamed back over whatever the build left, atomically and in `Drop`, so
/// a build that fails part-way restores it too. A link rather than a copy
/// because the file is thirty-odd megabytes and the point of one target
/// directory is fewer writes.
struct KeptBinary {
    shared: PathBuf,
    keep: Option<PathBuf>,
}

impl KeptBinary {
    /// Hold `shared` — the binary at that path today — until [`restore`].
    ///
    /// [`restore`]: Self::restore
    fn hold(shared: &Path) -> Self {
        // `CARGO_TARGET_TMPDIR` is cargo's own scratch directory for integration
        // tests, on the same filesystem as the binary — a hard link needs that,
        // and `tempfile`'s directory may be a different one (tmpfs). Held to
        // being `<target>/tmp` beside the binary's `<target>/debug` before
        // anything is swept from it.
        let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let target = shared.parent().and_then(Path::parent);
        assert!(
            tmp.file_name().is_some_and(|name| name == "tmp") && tmp.parent() == target,
            "{} is not the tmp directory of the target directory holding {}",
            tmp.display(),
            shared.display()
        );
        std::fs::create_dir_all(&tmp)
            .unwrap_or_else(|error| panic!("create {}: {error}", tmp.display()));
        // A run killed mid-build leaves its link behind, and a link pins the
        // binary it names after cargo has replaced it — so sweep the files this
        // mechanism names (`notignored-kept-<pid>`, and only those) first.
        for entry in std::fs::read_dir(&tmp).expect("read cargo's tmp directory") {
            let entry = entry.expect("a tmp entry");
            let name = entry.file_name();
            let ours = name
                .to_string_lossy()
                .strip_prefix("notignored-kept-")
                .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()));
            if ours && entry.file_type().expect("a file type").is_file() {
                let path = entry.path();
                std::fs::remove_file(&path)
                    .unwrap_or_else(|error| panic!("remove {}: {error}", path.display()));
            }
        }
        let keep = tmp.join(format!("notignored-kept-{}", std::process::id()));
        std::fs::hard_link(shared, &keep).unwrap_or_else(|error| {
            panic!(
                "cannot keep {} as a link at {}: {error}",
                shared.display(),
                keep.display()
            )
        });
        Self {
            shared: shared.to_path_buf(),
            keep: Some(keep),
        }
    }

    /// Put the held binary back at its path, replacing whatever is there now.
    fn restore(mut self) {
        let keep = self.keep.take().expect("restored once");
        put_back(&self.shared, &keep).unwrap_or_else(|error| {
            panic!(
                "cannot put {} back from {}: {error}",
                self.shared.display(),
                keep.display()
            )
        });
    }
}

impl Drop for KeptBinary {
    fn drop(&mut self) {
        // Only reached when the build panicked before `restore`: best effort, so
        // the siblings that follow spawn the binary the suite compiled, and the
        // journey's own failure stays the one reported.
        if let Some(keep) = self.keep.take() {
            let _ = put_back(&self.shared, &keep);
        }
    }
}

/// Rename the `keep` link back over `shared` if the build replaced it, and drop
/// the link either way.
///
/// A build that never reached the binary leaves the two as names for one file,
/// and renaming one such name onto the other is a no-op that keeps both — the
/// link would linger, pinning that binary's bytes after cargo's next uplift.
fn put_back(shared: &Path, keep: &Path) -> std::io::Result<()> {
    // Either replacement — cargo uplifting another artifact, maturin staging a
    // stripped one — changes the length or the modification time.
    let same_len_and_mtime = std::fs::metadata(shared)
        .and_then(|meta| Ok((meta.len(), meta.modified()?)))
        .is_ok_and(|now| now == fingerprint(keep));
    if !same_len_and_mtime {
        std::fs::rename(keep, shared)?;
    }
    match std::fs::remove_file(keep) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// The whole PyPI install path: build the wheel from this repo's
/// `pyproject.toml`, install it into a scratch venv, and run the console command
/// it put on that venv's PATH — and prove the binary the rest of this suite is
/// running is the one that was there before the build ([`KeptBinary`] says why
/// that needs proving, and why here rather than in a test of its own).
// llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] The task explicitly requires the production builder journey behind its existing packaging/e2e edge, without a new project or unrelated-project dependency. Its staging source inputs in nx.json invalidate this same owning tier so cached results cannot hide changed builders.
#[test]
fn the_pypi_wheel_installs_and_runs_the_prebuilt_binary() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let dist = scratch.path().join("dist");

    let shared = assert_cmd::cargo::cargo_bin("notignored");
    let before = fingerprint(&shared);
    let kept = KeptBinary::hold(&shared);

    let wheel = build_wheel(&dist);

    kept.restore();
    assert_eq!(
        fingerprint(&shared),
        before,
        "the wheel build left a different {} behind, which every other journey in \
         this suite spawns after it",
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

    let stage = scratch.path().join("staged");
    run(
        "stage the release artifact",
        Command::new("python3")
            .current_dir(repo_root())
            .arg("scripts/stage-release.py")
            .args(["--target", host_target(), "--wheel"])
            .arg(&wheel)
            .arg("--out")
            .arg(&stage),
    );
    run(
        "verify native staged payloads on every released host platform",
        Command::new("python3")
            .current_dir(repo_root())
            .arg("scripts/stage-release.py")
            .arg("--verify")
            .arg("--out")
            .arg(&stage),
    );
    // Drive the release's actual staging and install commands over this wheel;
    // transport is loopback, while packing, installation and CLI execution are real.
    if cfg!(unix) {
        for mutation in [
            "member",
            "metadata",
            "tag",
            "architecture",
            "format",
            "zip",
            "filename",
            "unknown-format",
            "truncated",
            "machine",
            "pe-signature",
        ] {
            let broken = scratch.path().join(format!("{mutation}.whl"));
            run(
                "make a malformed real wheel",
                Command::new("python3").arg("-c").arg(r#"
import sys,zipfile,struct
source,destination,mutation=sys.argv[1:]
if mutation == 'zip':
    open(destination,'wb').write(b'not a zip')
    sys.exit(0)
with zipfile.ZipFile(source) as original, zipfile.ZipFile(destination,'w') as changed:
    for member in original.infolist():
        name=member.filename
        payload=original.read(name)
        if '.data/scripts/' in name:
            if mutation == 'member': name += '-other'
            elif mutation == 'architecture':
                payload=bytearray(payload)
                if payload[:4] == b'\x7fELF':
                    machine=struct.unpack_from('<H',payload,18)[0]
                    struct.pack_into('<H',payload,18,183 if machine == 62 else 62)
                else:
                    machine=struct.unpack_from('<I',payload,4)[0]
                    struct.pack_into('<I',payload,4,0x0100000c if machine == 0x01000007 else 0x01000007)
            elif mutation == 'format':
                if payload[:4] == b'\x7fELF':
                    machine=struct.unpack_from('<H',payload,18)[0]
                    payload=b'\xcf\xfa\xed\xfe'+struct.pack('<I',0x01000007 if machine == 62 else 0x0100000c)+payload[8:]
                else:
                    machine=struct.unpack_from('<I',payload,4)[0]
                    payload=bytearray(payload)
                    payload[:4]=b'\x7fELF'
                    struct.pack_into('<H',payload,18,62 if machine == 0x01000007 else 183)
            elif mutation == 'unknown-format':
                payload=b'\x00\x01\x02\x03'+payload[4:]
            elif mutation == 'truncated':
                payload=payload[:6]
            elif mutation == 'machine':
                payload=bytearray(payload)
                if payload[:4] == b'\x7fELF':
                    struct.pack_into('<H',payload,18,3)
                else:
                    struct.pack_into('<I',payload,4,7)
            elif mutation == 'pe-signature':
                payload=b'MZ'+bytes(62)
        elif name.endswith('/METADATA') and mutation == 'metadata':
            payload=payload.replace(b'Name: notignored-cli',b'Name: wrong-cli')
        elif name.endswith('/WHEEL') and mutation == 'tag':
            payload=b'Wheel-Version: 1.0\nTag: py3-none-any\n'
        changed.writestr(name,payload)
"#).arg(&wheel).arg(&broken).arg(mutation),
            );
            let refused = Command::new("python3")
                .current_dir(repo_root())
                .arg("scripts/stage-release.py")
                .args(["--target", host_target(), "--wheel"])
                .arg(&broken)
                .arg("--out")
                .arg(scratch.path().join("rejected"))
                .output()
                .expect("run malformed wheel staging");
            assert!(!refused.status.success(), "{mutation} was accepted");
            let expected = match mutation {
                "member" => "exact cli payload",
                "metadata" => "name/version",
                "tag" => "platform tags",
                "architecture" => "architecture",
                "format" => "operating-system format",
                "zip" => "not a zip",
                "filename" => "wheel filename",
                "unknown-format" => "unrecognized executable format",
                "truncated" => "truncated executable header",
                "machine" => "unsupported executable machine",
                "pe-signature" => "invalid pe signature",
                _ => unreachable!("the mutation vocabulary is fixed above"),
            };
            assert!(
                String::from_utf8_lossy(&refused.stderr)
                    .to_lowercase()
                    .contains(expected),
                "{mutation} failed for the wrong reason: {}",
                String::from_utf8_lossy(&refused.stderr)
            );
            assert!(
                String::from_utf8_lossy(&refused.stderr).contains("ACTION:"),
                "{mutation} did not name a recovery action"
            );
        }
        let refused = Command::new("python3")
            .current_dir(repo_root())
            .arg("scripts/stage-release.py")
            .args(["--target", "unsupported-target", "--wheel"])
            .arg(&wheel)
            .arg("--out")
            .arg(scratch.path().join("rejected"))
            .output()
            .expect("run unsupported staging target");
        assert!(!refused.status.success());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("unsupported target"));
        for arguments in [vec![], vec!["--target", host_target()]] {
            let refused = Command::new("python3")
                .current_dir(repo_root())
                .arg("scripts/stage-release.py")
                .args(arguments)
                .arg("--out")
                .arg(scratch.path().join("missing-mode"))
                .output()
                .expect("run staging without a complete mode");
            assert!(!refused.status.success());
            assert!(refused.stdout.is_empty());
            let error = String::from_utf8_lossy(&refused.stderr);
            assert!(error.contains("packing requires --target and --wheel"));
            assert!(
                error.contains("ACTION:"),
                "missing mode had no recovery: {error}"
            );
        }
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(stage.join("manifest.json")).expect("read payload manifest"),
        )
        .expect("payload manifest JSON");
        assert_eq!(manifest["target"], host_target());
        assert_eq!(manifest["version"], cargo_version());
        assert_eq!(manifest["hashes"]["archive"], manifest["hashes"]["wheel"]);
        assert_eq!(manifest["hashes"]["npm"], manifest["hashes"]["wheel"]);
        run(
            "build the staged TypeScript SDK",
            Command::new(crate::support::bash_program())
                .current_dir(repo_root())
                .args([
                    "scripts/dev-tool.sh",
                    "tsc",
                    "-p",
                    "npm/notignored-sdk/tsconfig.json",
                ]),
        );
        let sdk = PathBuf::from(run(
            "stamp the staged TypeScript SDK",
            required("node")
                .current_dir(repo_root())
                .arg("npm/notignored-sdk/scripts/pack.mjs")
                .arg("--out")
                .arg(scratch.path().join("sdk")),
        ));
        let sdk_tgz = npm_pack(&sdk, &scratch.path().join("sdk-tarballs"));
        // Every staging run's scratch lands here, so cleanup is observable.
        let stage_tmp = scratch.path().join("stage-tmp");
        std::fs::create_dir(&stage_tmp).expect("create staging scratch");
        let verify = || {
            let mut command = Command::new("bash");
            command
                .current_dir(repo_root())
                .arg("scripts/stage-release.sh")
                .arg(&stage)
                .arg("python3")
                .arg(&sdk_tgz)
                .env("TMPDIR", &stage_tmp);
            command
        };
        // After a run, its scratch tree is gone and its loopback transport —
        // the one serving this stage's archives — is no longer running.
        let assert_cleaned_up = |after: &str| {
            let leftovers: Vec<_> = std::fs::read_dir(&stage_tmp)
                .expect("read staging scratch")
                .map(|entry| entry.expect("scratch entry").file_name())
                .filter(|name| name.to_string_lossy().starts_with("notignored-stage."))
                .collect();
            assert!(leftovers.is_empty(), "{after} left {leftovers:?}");
            let archives = stage.join("archives");
            let processes = run("list processes", Command::new("ps").args(["-eo", "args"]));
            assert!(
                !processes
                    .lines()
                    .any(|line| line.contains("staged-transport.py")
                        && line.contains(archives.to_str().expect("a UTF-8 path"))),
                "{after} left the staged transport running"
            );
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let shim_dir = scratch.path().join("delayed-python");
            std::fs::create_dir(&shim_dir).expect("create interpreter wrapper directory");
            let shim = shim_dir.join("python3");
            std::fs::write(
                &shim,
                "#!/usr/bin/env bash\ncase \"$1\" in */scripts/staged-transport.py) sleep 6;; esac\nexec \"$STAGED_REAL_PYTHON\" \"$@\"\n",
            )
            .expect("write delayed real-interpreter wrapper");
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))
                .expect("make interpreter wrapper executable");
            let python = run(
                "locate the real Python interpreter",
                required("python3").args(["-c", "import sys; print(sys.executable)"]),
            );
            let mut paths = vec![shim_dir];
            paths.extend(std::env::split_paths(
                &std::env::var_os("PATH").expect("PATH"),
            ));
            let wrapper_path = std::env::join_paths(&paths).expect("wrapper PATH");
            run(
                "install and execute staged forms after delayed real server startup",
                verify()
                    .env("PATH", &wrapper_path)
                    .env("STAGED_REAL_PYTHON", python.trim()),
            );
            // llmlint: ignore-block[e2e_not_mocked, tests_mirror_real_usage] the layer under test is stage-release.sh, and this is fault injection into its input: a real packed SDK artifact with a wrong export, which the staging verifier must refuse and clean up after. Nothing between the script and that artifact is replaced, and no public SDK input can make the real scan return a wrong report.
            let bad_sdk = scratch.path().join("sdk-wrong-report.tgz");
            run(
                "corrupt the real SDK artifact's exported report",
                Command::new(python.trim())
                    .arg("-c")
                    .arg(
                        r#"
import io,sys,tarfile
found=False
with tarfile.open(sys.argv[1]) as source, tarfile.open(sys.argv[2],'w:gz') as dest:
    for member in source.getmembers():
        data=source.extractfile(member).read() if member.isfile() else None
        if member.name == 'package/dist/index.js':
            data=b'export async function scan(){return {ignores:[]};}\n'
            member.size=len(data)
            found=True
        dest.addfile(member,io.BytesIO(data) if data is not None else None)
assert found, 'real SDK entry point was not present'
"#,
                    )
                    .arg(&sdk_tgz)
                    .arg(&bad_sdk),
            );
            let sdk_work = scratch.path().join("bad-sdk-work");
            std::fs::create_dir(&sdk_work).expect("create SDK failure scratch");
            let rejected = Command::new("bash")
                .current_dir(repo_root())
                .arg("scripts/stage-release.sh")
                .arg(&stage)
                .arg("python3")
                .arg(&bad_sdk)
                .env("TMPDIR", &sdk_work)
                .output()
                .expect("install SDK artifact with an incorrect report");
            assert!(!rejected.status.success());
            let error = String::from_utf8_lossy(&rejected.stderr);
            assert!(
                error.contains("ignores") && error.contains("ACTION:"),
                "wrong SDK error: {error}"
            );
            // llmlint: ignore-end[e2e_not_mocked, tests_mirror_real_usage]
            assert!(
                std::fs::read_dir(&sdk_work)
                    .expect("SDK scratch")
                    .find(|entry| entry
                        .as_ref()
                        .expect("temporary entry")
                        .file_name()
                        .to_string_lossy()
                        .starts_with("notignored-stage."))
                    .is_none(),
                "SDK rejection left temporary installs: {:?}",
                std::fs::read_dir(&sdk_work)
                    .expect("read leftover entries")
                    .map(|entry| entry.expect("leftover entry").file_name())
                    .collect::<Vec<_>>()
            );
        }

        for arguments in [vec![], vec![stage.to_str().expect("staged path")]] {
            let refused = Command::new("bash")
                .current_dir(repo_root())
                .arg("scripts/stage-release.sh")
                .args(arguments)
                .output()
                .expect("run staging with missing arguments");
            assert!(!refused.status.success());
            assert!(refused.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&refused.stderr)
                    .contains("ACTION: run bash scripts/stage-release.sh"),
                "missing arguments did not explain the real invocation"
            );
        }

        run(
            "install and execute the staged CLI forms without an SDK",
            Command::new("bash")
                .current_dir(repo_root())
                .arg("scripts/stage-release.sh")
                .arg(&stage)
                .arg("python3"),
        );
        let manifest_path = stage.join("manifest.json");
        let original_manifest = std::fs::read(&manifest_path).expect("read staged manifest");
        let good: serde_json::Value =
            serde_json::from_slice(&original_manifest).expect("manifest JSON");
        for (field, value) in [
            ("target", serde_json::json!("unknown-target")),
            ("version", serde_json::json!("")),
            ("version", serde_json::json!(42)),
            ("tag", serde_json::json!("v999.0.0")),
            ("architecture", serde_json::json!("unknown")),
            ("archive", serde_json::json!("../outside.tar.gz")),
            ("hashes", serde_json::json!({})),
            (
                "hashes",
                serde_json::json!({"archive": 123, "npm": "bad", "wheel": "bad"}),
            ),
        ] {
            let mut invalid = good.clone();
            invalid[field] = value;
            std::fs::write(
                &manifest_path,
                serde_json::to_vec(&invalid).expect("serialize bad manifest"),
            )
            .expect("write bad manifest");
            let refused = verify().output().expect("verify malformed manifest");
            assert!(
                !refused.status.success(),
                "invalid manifest {field} was accepted"
            );
            assert!(String::from_utf8_lossy(&refused.stderr).contains("ACTION:"));
        }
        // The Python verifier is its own entry point, so each shape it refuses
        // is driven at it directly rather than through the shell's pre-check.
        let (other_target, other_arch) = if host_target().starts_with("x86_64-") {
            ("aarch64-unknown-linux-gnu", "aarch64")
        } else {
            ("x86_64-unknown-linux-gnu", "x86_64")
        };
        let host_arch = if other_arch == "aarch64" {
            "x86_64"
        } else {
            "aarch64"
        };
        let with = |changes: &[(&str, serde_json::Value)]| {
            let mut changed = good.clone();
            for (field, value) in changes {
                changed[*field] = value.clone();
            }
            changed
        };
        let mut missing = good.clone();
        missing
            .as_object_mut()
            .expect("manifest object")
            .remove("npm");
        let mut extra = good.clone();
        extra["extra"] = serde_json::json!("x");
        let mut short_hashes = good.clone();
        short_hashes["hashes"]
            .as_object_mut()
            .expect("hashes object")
            .remove("npm");
        for (manifest, expected) in [
            (serde_json::json!([]), "must be an object"),
            (missing, "fields differ"),
            (extra, "fields differ"),
            (short_hashes, "invalid staged manifest hashes"),
            (with(&[("target", serde_json::json!(7))]), "must be strings"),
            (
                with(&[("architecture", serde_json::json!("riscv64"))]),
                "unknown staged manifest architecture",
            ),
            (
                with(&[("architecture", serde_json::json!(other_arch))]),
                "manifest architecture differs from target",
            ),
            (
                with(&[
                    ("target", serde_json::json!(other_target)),
                    ("architecture", serde_json::json!(other_arch)),
                ]),
                "manifest architecture differs from executable payload",
            ),
        ] {
            std::fs::write(
                &manifest_path,
                serde_json::to_vec(&manifest).expect("serialize bad manifest"),
            )
            .expect("write bad manifest");
            let refused = Command::new("python3")
                .current_dir(repo_root())
                .arg("scripts/stage-release.py")
                .args(["--verify", "--out"])
                .arg(&stage)
                .output()
                .expect("verify malformed manifest directly");
            let error = String::from_utf8_lossy(&refused.stderr);
            assert!(!refused.status.success(), "{manifest} was accepted");
            assert!(
                error.contains(expected) && error.contains("ACTION:"),
                "{manifest} failed for the wrong reason: {error}"
            );
        }
        assert_eq!(
            good["architecture"], host_arch,
            "the staged manifest records the host's architecture"
        );
        std::fs::write(&manifest_path, original_manifest).expect("restore staged manifest");
        for (assets, port) in [
            (
                scratch.path().join("missing-assets"),
                scratch.path().join("port"),
            ),
            (stage.join("archives"), manifest_path),
            (
                stage.join("archives"),
                scratch.path().join("missing-parent/port"),
            ),
        ] {
            let refused = Command::new("python3")
                .current_dir(repo_root())
                .arg("scripts/staged-transport.py")
                .arg(assets)
                .arg(port)
                .output()
                .expect("run invalid transport");
            assert!(!refused.status.success());
            assert!(String::from_utf8_lossy(&refused.stderr).contains("ACTION:"));
        }

        for (directory, name) in [
            ("archives", "archive"),
            ("wheels", "wheel"),
            ("npm-artifacts", "npm"),
        ] {
            let artifact = stage
                .join(directory)
                .join(good[name].as_str().expect("artifact filename"));
            let original = std::fs::read(&artifact).expect("read staged artifact");
            run(
                "remove the real packaged binary member",
                Command::new("python3").arg("-c").arg(r#"
import io,sys,tarfile,zipfile
path,kind=sys.argv[1:]
if kind == 'wheel':
    with zipfile.ZipFile(path) as source:
        members=[(member,source.read(member)) for member in source.infolist() if '.data/scripts/' not in member.filename]
    with zipfile.ZipFile(path,'w') as dest:
        for member,data in members: dest.writestr(member,data)
else:
    with tarfile.open(path) as source:
        members=[(member,source.extractfile(member).read() if member.isfile() else None) for member in source.getmembers() if member.name not in ('notignored','package/bin/notignored')]
    with tarfile.open(path,'w:gz') as dest:
        for member,data in members: dest.addfile(member,io.BytesIO(data) if data is not None else None)
"#).arg(&artifact).arg(name),
            );
            let refused = Command::new("python3")
                .current_dir(repo_root())
                .arg("scripts/stage-release.py")
                .args(["--verify", "--out"])
                .arg(&stage)
                .output()
                .expect("verify artifact without its payload member");
            assert!(
                !refused.status.success(),
                "missing {name} payload was accepted"
            );
            let error = String::from_utf8_lossy(&refused.stderr);
            assert!(
                error.contains("notignored") && error.contains("ACTION:"),
                "wrong missing-member error: {error}"
            );
            std::fs::write(&artifact, original).expect("restore packaged member");
        }
        let staged_wheel = stage
            .join("wheels")
            .join(wheel.file_name().expect("wheel filename"));
        let original_wheel = std::fs::read(&staged_wheel).expect("read staged wheel");
        run(
            "corrupt one staged payload",
            Command::new("python3")
                .arg("-c")
                .arg(
                    r#"
import sys,zipfile,os
path=sys.argv[1]
with zipfile.ZipFile(path) as original, zipfile.ZipFile(path+'.new','w') as changed:
    for name in original.namelist():
        payload=original.read(name)
        if '.data/scripts/' in name: payload += b'changed'
        changed.writestr(name,payload)
os.replace(path+'.new',path)
"#,
                )
                .arg(&staged_wheel),
        );
        let disagreement = verify().output().expect("verify mismatched payloads");
        assert!(!disagreement.status.success());
        assert!(String::from_utf8_lossy(&disagreement.stderr).contains("hashes differ"));
        std::fs::write(staged_wheel, original_wheel).expect("restore staged wheel");
        let checksum = std::fs::read_dir(stage.join("archives"))
            .expect("read archives")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "sha256")
            })
            .expect("a checksum sibling");
        let original = std::fs::read(&checksum).expect("read checksum");
        std::fs::write(&checksum, "0".repeat(64)).expect("corrupt checksum");
        let failure = verify().output().expect("run corrupted stage");
        assert!(
            !failure.status.success(),
            "a bad checksum installed successfully"
        );
        assert!(
            String::from_utf8_lossy(&failure.stderr).contains("checksum"),
            "failure should name the checksum: {}",
            String::from_utf8_lossy(&failure.stderr)
        );
        assert_cleaned_up("an installer failure after the transport started");
        std::fs::write(checksum, original).expect("restore checksum");
        run("recover after restoring staged checksum", &mut verify());
        assert_cleaned_up("a successful staged install");
    }

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

// llmlint: ignore-end[expensive_tests_stay_behind_their_own_edge]

/// Malformed arguments to the staging scripts are refused with the next action,
/// not argparse's bare usage line: the release log is the only diagnosis a Mac
/// staging failure gets.
#[test]
fn the_staging_scripts_refuse_malformed_arguments_with_an_action() {
    for (script, args) in [
        ("scripts/stage-release.py", vec!["--target", "x"]),
        ("scripts/stage-release.py", vec!["--out", "x", "--bogus"]),
        ("scripts/staged-transport.py", vec![]),
        ("scripts/staged-transport.py", vec!["a", "b", "--bogus"]),
    ] {
        let refused = Command::new("python3")
            .current_dir(repo_root())
            .arg(script)
            .args(&args)
            .output()
            .expect("run a staging script");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert_eq!(
            refused.status.code(),
            Some(2),
            "{script} {args:?}: {stderr}"
        );
        assert!(
            stderr.contains("ACTION:"),
            "{script} {args:?} gave no next action: {stderr}"
        );
    }
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

// llmlint: ignore-block[tests_mirror_real_usage] the two journeys below reach the
// launcher's last two branches — a child it cannot start, and one the kernel killed —
// and the only way into either is to put the *child* in that state. What is under test
// is the shim's handling, not notignored: substituting the binary it spawns is the same
// move as the loopback registry in tests/e2e/publish_npm.rs, one layer down. Everything
// around them is real — npm assembled and installed the tree, and the command that runs
// is the one npm linked. `changed_behavior_has_e2e` requires these branches be covered,
// and no user-facing input reaches them; leaving them untested is the alternative.
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

/// The launcher hands back the binary's own exit code.
///
/// `--fail-if-found` exits 1, and a CI job's whole verdict is that number. A shim
/// that swallowed it — returning 0 because *it* succeeded in spawning — would
/// turn every gate built on this package green while suppressions piled up.
#[test]
fn the_npm_launcher_returns_the_binarys_own_exit_code() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let prefix = installed_launcher(scratch.path());

    // Exit 0: a tree with nothing to report.
    let clean = scratch.path().join("clean");
    std::fs::create_dir_all(&clean).expect("create the clean tree");
    std::fs::write(clean.join("app.py"), "x = 1\n").expect("write a clean file");
    let output = installed_npm_command(&prefix)
        .current_dir(scratch.path())
        .arg(&clean)
        .arg("--fail-if-found")
        .output()
        .expect("run the launcher");
    assert_eq!(
        output.status.code(),
        Some(0),
        "nothing was suppressed, so --fail-if-found has nothing to fail on: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Exit 1: the same flag over a file that does carry one.
    let fixture = smoke_file(scratch.path());
    let output = installed_npm_command(&prefix)
        .current_dir(scratch.path())
        .arg(&fixture)
        .arg("--fail-if-found")
        .output()
        .expect("run the launcher");
    assert_eq!(
        output.status.code(),
        Some(1),
        "the launcher did not pass the binary's exit 1 through: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A binary killed by a signal takes the launcher down the same way.
///
/// `spawnSync` reports a signalled child as `status: null`, so a shim that just
/// forwarded `status` would report 0 — success — for a process the kernel
/// killed. The launcher re-raises the signal instead, and the only way to put a
/// child in that state on purpose is to give it one that signals itself: what is
/// under test is the shim's handling, not what notignored does.
#[cfg(unix)]
#[test]
fn the_npm_launcher_re_raises_a_signal_that_killed_the_binary() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let prefix = installed_launcher(scratch.path());

    let installed = prefix
        .join("lib")
        .join("node_modules")
        .join(host_platform_package())
        .join("bin")
        .join("notignored");
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(&installed, "#!/bin/sh\nkill -TERM $$\n").expect("replace the binary");
    std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755))
        .expect("make the replacement executable");

    let output = installed_npm_command(&prefix)
        .current_dir(scratch.path())
        .arg("--version")
        .output()
        .expect("run the launcher");
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&output.status),
        Some(15),
        "the launcher did not die of the signal that killed its child; it exited {:?}",
        output.status.code()
    );
}
// llmlint: ignore-end[tests_mirror_real_usage]
