//! The npm verify leg's install, run against a registry that has not finished
//! publishing.
//!
// llmlint: ignore-block[comments_earn_their_place] this is the one recorded account of which release failure these journeys guard, with the run and registry timings that establish it; the task that added them requires that evidence to live here or in AGENTS.md, and AGENTS.md is kept terse by pointing here.
//! `verify-npm (macos-latest)` failed on v0.1.13, v0.1.14, v0.1.15 and v0.1.16,
//! and each time it was the only red job. The runs say why. `publish-npm`
//! published `notignored-cli-darwin-arm64` *first* and npm acknowledged it
//! (v0.1.16: 18:41:14Z), but the registry recorded it only at 18:42:19Z — after
//! the launcher (18:41:28Z), and after the verify leg's install (18:41:43Z). npm
//! treats an optional dependency it cannot resolve as skippable, so that install
//! exited 0 without the platform package; `scripts/retry-install.sh` took the
//! exit 0 as "installed on attempt 1"; and the smoke test then met a launcher
//! with no binary behind it. v0.1.13 shows the same shape: platform package
//! acknowledged at 14:05:11Z, recorded at 14:07:17Z, launcher at 14:05:23Z.
// llmlint: ignore-end[comments_earn_their_place]
//!
//! These journeys reproduce that registry state and hold the fix,
//! `scripts/npm-install-probe.sh`, to what the release needs from it: the old
//! probe accepts the broken install, and the new one keeps waiting, installs
//! once the package is served, and fails naming the package when it never is.
//!
//! Only the registry is substituted, as in [`crate::publish_npm`]: **real
//! `npm`** installs over HTTP from a server this module runs on loopback, the
//! packages are real ones `scripts/npm-build.mjs` assembles around the compiled
//! `notignored`, and the retry loop and the probe are the real scripts the
//! release runs.
//!
//! Unix only: the verify legs run these scripts under bash, and a Windows host
//! would be testing Git Bash's path translation rather than the probe.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::support::{bash_program, cargo_version, repo_root};

/// The npm platform package this host resolves, as `process.platform` and
/// `process.arch` name it — which is what the probe and the launcher look for.
fn host_platform_package() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => panic!("no released npm package for {other}"),
    };
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("notignored-cli-{platform}-{arch}")
}

fn host_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        (os, arch) => panic!("no released npm package for {os}/{arch}"),
    }
}

/// One package the registry can serve: its tarball and npm's integrity for it.
struct Package {
    manifest: serde_json::Value,
    tarball: Vec<u8>,
    integrity: String,
}

/// When the registry starts serving the host's platform package at this version.
#[derive(Clone, Copy)]
enum Platform {
    /// From the first request: a registry that has converged.
    Served,
    /// Only once the launcher has been asked for this many times — one ask per
    /// install attempt — which is the window the red releases were caught in.
    ServedFromAttempt(usize),
    /// Never: the package was not published at all.
    NeverServed,
}

/// An npm registry on loopback, serving the launcher at this version and the
/// host's platform package on the schedule [`Platform`] sets.
struct Registry {
    url: String,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Registry {
    fn start(packages: BTreeMap<String, Package>, platform: Platform) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
        let url = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().expect("the bound address").port()
        );
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&shutdown);
        let base = url.clone();
        let launcher_reads = Arc::new(AtomicUsize::new(0));
        let worker = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        serve_one(stream, &base, &packages, platform, &launcher_reads);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Registry {
            url,
            shutdown,
            worker: Some(worker),
        }
    }
}

/// Answer one npm request: a packument for `GET /<name>`, bytes for
/// `GET /-/<name>.tgz`, and 404 for everything else — which is what npm gets for
/// the four platform packages this host does not install.
fn serve_one(
    mut stream: TcpStream,
    base: &str,
    packages: &BTreeMap<String, Package>,
    platform: Platform,
    launcher_reads: &AtomicUsize,
) {
    // An accepted socket inherits the listener's O_NONBLOCK on macOS but not on
    // Linux, so set it rather than depend on which.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
    let mut head = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let blank = line.trim().is_empty();
                head.push_str(&line);
                if blank {
                    break;
                }
            }
        }
    }
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let name = path.trim_start_matches('/').split('?').next().unwrap_or("");
    // Anything npm sends a body with (audit, telemetry) is drained and refused.
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0);
    if length > 0 {
        let mut body = vec![0u8; length];
        let _ = reader.read_exact(&mut body);
    }

    let platform_package = host_platform_package();
    let (status, content_type, body): (&str, &str, Vec<u8>) =
        if let Some(file) = name.strip_prefix("-/") {
            match packages
                .iter()
                .find(|(package, _)| tarball_name(package) == file)
            {
                Some((_, package)) => (
                    "200 OK",
                    "application/octet-stream",
                    package.tarball.clone(),
                ),
                None => not_found(),
            }
        } else {
            if name == "notignored-cli" {
                launcher_reads.fetch_add(1, Ordering::SeqCst);
            }
            let served = name != platform_package
                || match platform {
                    Platform::Served => true,
                    Platform::ServedFromAttempt(attempt) => {
                        launcher_reads.load(Ordering::SeqCst) >= attempt
                    }
                    Platform::NeverServed => false,
                };
            match packages.get(name) {
                Some(package) if served => (
                    "200 OK",
                    "application/json",
                    packument(base, name, package).into_bytes(),
                ),
                _ => not_found(),
            }
        };

    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

fn not_found() -> (&'static str, &'static str, Vec<u8>) {
    (
        "404 Not Found",
        "application/json",
        b"{\"error\":\"Not found\"}".to_vec(),
    )
}

fn tarball_name(package: &str) -> String {
    format!("{package}-{}.tgz", cargo_version())
}

/// The metadata document npm resolves `name@<this version>` against: the
/// package's own manifest — its `os`/`cpu`, the launcher's
/// `optionalDependencies` — plus where to fetch it.
fn packument(base: &str, name: &str, package: &Package) -> String {
    let version = cargo_version();
    let mut entry = package.manifest.clone();
    entry["dist"] = serde_json::json!({
        "tarball": format!("{base}-/{}", tarball_name(name)),
        "integrity": package.integrity,
    });
    serde_json::json!({
        "name": name,
        "dist-tags": { "latest": version },
        "versions": { version: entry },
    })
    .to_string()
}

fn run(what: &str, command: &mut Command) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("cannot run {what}: {error}"));
    assert!(
        output.status.success(),
        "{what} failed\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Assemble one package with `scripts/npm-build.mjs` and pack it.
///
/// `packed_as` rewrites the version inside the tarball while the registry keeps
/// listing it under the crate's — a registry serving bytes other than the
/// version it names, which is the one way this host can put a platform package
/// at the wrong version under a launcher that pins it exactly.
fn build(scratch: &Path, args: &[&str], packed_as: Option<&str>) -> Package {
    let dist = scratch.join("dist");
    let package = PathBuf::from(run(
        "npm-build.mjs",
        Command::new("node")
            .current_dir(repo_root())
            .arg("scripts/npm-build.mjs")
            .args(args)
            .arg("--out")
            .arg(&dist),
    ));
    let out = scratch
        .join("packed")
        .join(package.file_name().expect("a package directory has a name"));
    std::fs::create_dir_all(&out).expect("create the pack destination");
    let manifest =
        std::fs::read_to_string(package.join("package.json")).expect("read package.json");
    if let Some(version) = packed_as {
        let mut rewritten: serde_json::Value =
            serde_json::from_str(&manifest).expect("a valid package.json");
        rewritten["version"] = serde_json::json!(version);
        std::fs::write(package.join("package.json"), rewritten.to_string())
            .expect("rewrite package.json");
    }
    let packed: serde_json::Value = serde_json::from_str(&run(
        "npm pack",
        Command::new("npm")
            .current_dir(&package)
            .args(["pack", "--json", "--pack-destination"])
            .arg(&out),
    ))
    .expect("npm pack --json prints JSON");
    let entry = &packed[0];
    Package {
        manifest: serde_json::from_str(&manifest).expect("a valid package.json"),
        tarball: std::fs::read(out.join(entry["filename"].as_str().expect("a filename")))
            .expect("read the packed tarball"),
        integrity: entry["integrity"]
            .as_str()
            .expect("an integrity")
            .to_string(),
    }
}

/// The launcher and the host's platform package, at the crate's own version,
/// wrapping the compiled `notignored`.
fn release(scratch: &Path) -> BTreeMap<String, Package> {
    release_with_platform_packed_as(scratch, None)
}

/// [`release`], with the platform package's tarball claiming `packed_as`.
fn release_with_platform_packed_as(
    scratch: &Path,
    packed_as: Option<&str>,
) -> BTreeMap<String, Package> {
    let binary = assert_cmd::cargo::cargo_bin("notignored");
    let binary = binary.to_str().expect("a UTF-8 binary path");
    BTreeMap::from([
        (
            host_platform_package(),
            build(
                scratch,
                &["platform", "--target", host_target(), "--binary", binary],
                packed_as,
            ),
        ),
        (
            "notignored-cli".to_string(),
            build(scratch, &["launcher"], None),
        ),
    ])
}

/// An environment in which `npm` talks to `registry` and nothing else, and a
/// `--global` install lands in the scratch prefix.
fn npm_env<'a>(command: &'a mut Command, registry: &Registry, scratch: &Path) -> &'a mut Command {
    command
        .env("HOME", scratch)
        .env("npm_config_registry", &registry.url)
        .env("npm_config_prefix", scratch.join("prefix"))
        .env("npm_config_cache", scratch.join("npm-cache"))
        .env("npm_config_userconfig", scratch.join(".npmrc"))
        // A 404 is the answer under test, not something to wait out; npm's own
        // retries would only slow each attempt down.
        .env("npm_config_fetch_retries", "0")
        .env("npm_config_audit", "false")
        .env("npm_config_fund", "false")
        .env("npm_config_update_notifier", "false")
}

/// `scripts/retry-install.sh` around `command`, with a budget sized for a test.
fn retried(registry: &Registry, scratch: &Path, budget: &str, command: &[&str]) -> Output {
    let mut retry = Command::new(bash_program());
    retry
        .current_dir(scratch)
        .arg(repo_root().join("scripts/retry-install.sh"))
        .args(["--budget", budget, "--first-delay", "1", "--max-delay", "1"])
        .args(["--label", "npm notignored-cli under test", "--"])
        .args(command);
    npm_env(&mut retry, registry, scratch)
        .output()
        .expect("run retry-install.sh")
}

/// The fixed probe, invoked as `verify-npm` invokes it.
fn probe(registry: &Registry, scratch: &Path, budget: &str, global: bool) -> Output {
    let script = repo_root().join("scripts/npm-install-probe.sh");
    let spec = format!("notignored-cli@{}", cargo_version());
    let version = cargo_version();
    let mut command = vec!["bash", script.to_str().expect("a UTF-8 path"), "--version"];
    command.push(&version);
    if global {
        command.push("--global");
    }
    command.push(&spec);
    retried(registry, scratch, budget, &command)
}

/// The `notignored` a `--global` install linked into the scratch prefix.
fn installed(scratch: &Path) -> Command {
    Command::new(scratch.join("prefix/bin/notignored"))
}

fn text(output: &Output) -> String {
    format!(
        "--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// What went wrong on v0.1.13–v0.1.16, reproduced: the registry serves the
/// launcher while this host's platform package is not yet served, and the probe
/// the release used — a bare `npm install -g` under the retry loop — calls that
/// installed on its first attempt. The launcher it left then cannot run.
#[test]
fn the_old_probe_accepts_an_install_that_skipped_the_platform_package() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let registry = Registry::start(release(scratch.path()), Platform::NeverServed);

    let spec = format!("notignored-cli@{}", cargo_version());
    let old = retried(
        &registry,
        scratch.path(),
        "30",
        &["npm", "install", "-g", "--prefer-online", &spec],
    );
    assert!(
        old.status.success(),
        "the old probe was expected to accept this install — if npm now fails an \
         unresolvable optional dependency, the failure this guards has changed shape:\n{}",
        text(&old)
    );
    assert!(
        String::from_utf8_lossy(&old.stdout).contains("installed on attempt 1"),
        "{}",
        text(&old)
    );

    let run = installed(scratch.path())
        .arg("--version")
        .output()
        .expect("run the installed launcher");
    assert!(!run.status.success(), "{}", text(&run));
    assert!(
        String::from_utf8_lossy(&run.stderr).contains(&format!(
            "the platform package {} is not installed",
            host_platform_package()
        )),
        "the red release's smoke failure was not reproduced:\n{}",
        text(&run)
    );
}

/// The fixed probe keeps waiting through the same window, and succeeds once the
/// registry serves the platform package — leaving a `notignored` that runs.
#[test]
fn the_probe_waits_for_the_platform_package_and_installs_it_once_served() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let registry = Registry::start(release(scratch.path()), Platform::ServedFromAttempt(2));

    let output = probe(&registry, scratch.path(), "120", true);
    assert!(output.status.success(), "{}", text(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("attempt 1 failed")
            && stderr.contains(&format!(
                "not its platform package {}@{}",
                host_platform_package(),
                cargo_version()
            )),
        "the first attempt, made before the package was served, was not the one retried:\n{}",
        text(&output)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("installed on attempt 2"),
        "{}",
        text(&output)
    );

    let version = run(
        "the installed notignored",
        installed(scratch.path()).arg("--version"),
    );
    assert_eq!(version, format!("notignored {}", cargo_version()));
}

/// A platform package that is never served exhausts the budget, and the failure
/// names it — the package to go and look for on the registry, rather than a
/// smoke test's complaint about a launcher.
#[test]
fn the_probe_fails_naming_the_platform_package_when_it_is_never_served() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let registry = Registry::start(release(scratch.path()), Platform::NeverServed);

    let output = probe(&registry, scratch.path(), "6", true);
    assert!(!output.status.success(), "{}", text(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let wanted = format!("{}@{}", host_platform_package(), cargo_version());
    assert!(
        stderr.contains("::error::npm notignored-cli under test: still not installable")
            && stderr.contains(&format!(
                "npm installed notignored-cli but not its platform package {wanted}"
            )),
        "the exhausted budget does not name {wanted}:\n{}",
        text(&output)
    );
}

/// The SDK step installs into a project rather than globally, so the probe reads
/// that project's `node_modules`: served, it passes; not served, it fails the
/// same way.
#[test]
fn the_probe_checks_a_project_install_the_same_way() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    std::fs::write(
        scratch.path().join("package.json"),
        r#"{"name":"verify-npm-project","version":"1.0.0","private":true}"#,
    )
    .expect("write the project manifest");

    let missing = Registry::start(release(scratch.path()), Platform::NeverServed);
    let output = probe(&missing, scratch.path(), "3", false);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(&format!(
            "not its platform package {}@{}",
            host_platform_package(),
            cargo_version()
        )),
        "{}",
        text(&output)
    );
    drop(missing);

    let served = Registry::start(release(scratch.path()), Platform::Served);
    let output = probe(&served, scratch.path(), "60", false);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        scratch
            .path()
            .join("node_modules")
            .join(host_platform_package())
            .join("package.json")
            .is_file(),
        "the project install has no platform package:\n{}",
        text(&output)
    );
}

/// A platform package that installs at some other version fails the probe and
/// says which version it wanted: the launcher would run a binary that is not
/// the release being verified.
#[test]
fn the_probe_fails_on_a_platform_package_at_another_version() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let registry = Registry::start(
        release_with_platform_packed_as(scratch.path(), Some("0.0.1")),
        Platform::Served,
    );

    let output = probe(&registry, scratch.path(), "3", true);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(&format!(
            "npm installed {pkg}@0.0.1, not {pkg}@{}",
            cargo_version(),
            pkg = host_platform_package()
        )),
        "{}",
        text(&output)
    );
}

/// An install that brought no launcher at all fails the probe with what to pass.
#[test]
fn the_probe_fails_when_no_launcher_was_installed() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let registry = Registry::start(release(scratch.path()), Platform::Served);
    let script = repo_root().join("scripts/npm-install-probe.sh");
    let version = cargo_version();
    let platform_only = format!("{}@{version}", host_platform_package());

    let mut command = Command::new(bash_program());
    command.current_dir(scratch.path()).arg(&script).args([
        "--version",
        &version,
        "--global",
        &platform_only,
    ]);
    let output = npm_env(&mut command, &registry, scratch.path())
        .output()
        .expect("run the probe");
    assert!(!output.status.success(), "{}", text(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("notignored-cli is not installed")
            && stderr.contains("ACTION: pass notignored-cli@<version>"),
        "{}",
        text(&output)
    );
}

/// An install npm itself refuses — here the launcher is not on the registry at
/// all — fails the probe with npm's error, what to do, and a last line naming
/// the install, which is what the retry loop shows per attempt.
#[test]
fn the_probe_fails_with_npms_error_when_the_install_fails() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let mut packages = release(scratch.path());
    packages.remove("notignored-cli");
    let registry = Registry::start(packages, Platform::Served);

    let output = probe(&registry, scratch.path(), "3", true);
    assert!(!output.status.success(), "{}", text(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E404")
            && stderr.contains("ACTION: an E404 or ETARGET above")
            && stderr.contains(&format!(
                "npm-install-probe: npm install notignored-cli@{} failed",
                cargo_version()
            )),
        "{}",
        text(&output)
    );
}
