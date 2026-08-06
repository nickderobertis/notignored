//! The installer, executed — not read.
//!
//! [`install_contract`](../install_contract.rs) checks that `install.sh` and the
//! release workflow spell the asset name the same way. That is a text check;
//! these journeys *run* the script end to end: a real HTTP server serves a real
//! release layout, real `curl` downloads it, a real SHA-256 tool verifies it, and
//! the archive holds the **real compiled `notignored`**, which the test then
//! executes. Nothing between the script and the artifact is stubbed.
//!
//! The one substitution is *where* the release lives: a local server rather than
//! github.com, via the documented `NOTIGNORED_RELEASE_BASE_URL` override. The
//! deterministic gate has to stay offline, and the network path itself is still
//! exercised for real.
//!
//! Unix only: `install.sh` is the POSIX-shell surface. Windows users take
//! `cargo install`, which the CI install job exercises on its own.
#![cfg(unix)]

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::support::repo_root;

/// A directory laid out like a GitHub release — one archive plus its `.sha256`,
/// named exactly as `release.yml` publishes them — served over real HTTP.
struct Release {
    _dir: tempfile::TempDir,
    _server: Option<Server>,
    base_url: String,
    tag: String,
}

/// A child process that is always killed and reaped when it goes out of scope,
/// so no server outlives the test that started it.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A port nothing is listening on. Bound and released so the server can take it.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("the bound address")
        .port()
}

/// Serve `root` over HTTP until the returned child is dropped.
fn serve(root: &Path) -> (Server, String) {
    let port = free_port();
    let server = Server(
        Command::new("python3")
            .args([
                "-m",
                "http.server",
                &port.to_string(),
                "--bind",
                "127.0.0.1",
            ])
            .arg("--directory")
            .arg(root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("python3 -m http.server (required by the installer e2e)"),
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (server, format!("http://127.0.0.1:{port}"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("the release server never started on port {port}");
}

fn host_target() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        }
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

fn sha256_of(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .or_else(|_| {
            Command::new("shasum")
                .args(["-a", "256"])
                .arg(path)
                .output()
        })
        .expect("a SHA-256 tool (sha256sum or shasum)");
    String::from_utf8(output.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .expect("a digest")
        .to_string()
}

/// Publish a release holding the real compiled `notignored`, served over HTTP.
fn publish(tag: &str, corrupt_checksum: bool, with_checksum: bool) -> Release {
    let dir = tempfile::tempdir().unwrap();
    let stage = dir.path().join("stage");
    fs::create_dir_all(&stage).unwrap();

    // The artifact users would receive, not a stand-in: the test runs it after
    // installing, so a broken archive or a lost executable bit is a failure.
    fs::copy(
        assert_cmd::cargo::cargo_bin("notignored"),
        stage.join("notignored"),
    )
    .expect("copy the compiled binary into the release archive");

    let release = dir.path().join("releases").join(tag);
    fs::create_dir_all(&release).unwrap();
    let archive = release.join(format!("notignored-{tag}-{}.tar.gz", host_target()));
    let status = Command::new("tar")
        .args([
            "-czf",
            archive.to_str().unwrap(),
            "-C",
            stage.to_str().unwrap(),
            "notignored",
        ])
        .status()
        .expect("tar");
    assert!(status.success(), "could not build the fixture archive");

    if with_checksum {
        let digest = if corrupt_checksum {
            "0".repeat(64)
        } else {
            sha256_of(&archive)
        };
        let name = archive.file_name().unwrap().to_string_lossy().to_string();
        fs::write(
            release.join(format!("{name}.sha256")),
            format!("{digest}  {name}\n"),
        )
        .unwrap();
    }

    let (server, base_url) = serve(&dir.path().join("releases"));
    Release {
        _server: Some(server),
        base_url,
        tag: tag.to_string(),
        _dir: dir,
    }
}

/// Run `install.sh` against a release, installing into `install_dir`.
fn install(release: &Release, install_dir: &Path, extra: &[&str]) -> Output {
    let mut command = Command::new("sh");
    command
        .arg(repo_root().join("scripts/install.sh"))
        .args([
            "--version",
            &release.tag,
            "--to",
            install_dir.to_str().unwrap(),
        ])
        .args(extra)
        .env("NOTIGNORED_RELEASE_BASE_URL", &release.base_url);
    command.output().expect("run install.sh")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn the_installer_downloads_verifies_and_installs_a_runnable_binary() {
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();
    let output = install(&release, target.path(), &[]);

    let stderr = stderr_of(&output);
    assert!(output.status.success(), "{:?}: {stderr}", output.status);
    assert!(stderr.contains("notignored v9.9.9 installed"), "{stderr}");

    let installed = target.path().join("notignored");
    assert!(installed.exists(), "the binary was not placed in --to");
    let run = Command::new(&installed)
        .arg("--version")
        .output()
        .expect("run the installed binary");
    assert!(run.status.success(), "the installed binary does not run");
    assert!(
        String::from_utf8(run.stdout)
            .unwrap()
            .contains(env!("CARGO_PKG_VERSION")),
        "the installed binary is not the artifact we packaged"
    );
}

#[test]
fn a_checksum_mismatch_aborts_without_installing() {
    let release = publish("v9.9.9", true, true);
    let target = tempfile::tempdir().unwrap();
    let output = install(&release, target.path(), &[]);

    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = stderr_of(&output);
    assert!(stderr.contains("checksum mismatch"), "{stderr}");
    assert!(stderr.contains("refusing to install"), "{stderr}");
    assert!(
        !target.path().join("notignored").exists(),
        "a binary that failed verification was installed anyway"
    );
}

#[test]
fn a_missing_checksum_aborts_rather_than_installing_unverified() {
    let release = publish("v9.9.9", false, false);
    let target = tempfile::tempdir().unwrap();
    let output = install(&release, target.path(), &[]);

    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("refusing to install unverified"),
        "{stderr}"
    );
    assert!(!target.path().join("notignored").exists());
}

#[test]
fn a_malformed_version_tag_is_rejected_before_any_download() {
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();
    for tag in ["9.9.9", "v9.9", "v9.9.9.9", "v../../etc"] {
        let mut command = Command::new("sh");
        let output = command
            .arg(repo_root().join("scripts/install.sh"))
            .args(["--version", tag, "--to", target.path().to_str().unwrap()])
            .env("NOTIGNORED_RELEASE_BASE_URL", &release.base_url)
            .output()
            .expect("run install.sh");
        assert_eq!(output.status.code(), Some(1), "{tag} was accepted");
        assert!(stderr_of(&output).contains("invalid release tag"), "{tag}");
    }
}

#[test]
fn a_missing_release_asset_names_the_release_that_lacks_it() {
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();
    let missing = Release {
        _server: None,
        base_url: release.base_url.clone(),
        tag: "v8.8.8".to_string(),
        _dir: tempfile::tempdir().unwrap(),
    };
    let output = install(&missing, target.path(), &[]);

    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    assert!(
        stderr_of(&output).contains("v8.8.8"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn without_a_sha256_tool_the_installer_refuses_rather_than_skipping_verification() {
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();

    // A PATH holding only the tools the script needs to get as far as hashing —
    // and no hasher. This is the real "nothing can vouch for it" boundary.
    let bin = tempfile::tempdir().unwrap();
    for tool in [
        "sh", "curl", "tar", "uname", "mktemp", "rm", "find", "head", "cut", "mkdir", "install",
        "cp", "chmod", "printf", "sed", "tr",
    ] {
        if let Ok(found) = which(tool) {
            let _ = std::os::unix::fs::symlink(found, bin.path().join(tool));
        }
    }

    let output = Command::new("sh")
        .arg(repo_root().join("scripts/install.sh"))
        .args([
            "--version",
            &release.tag,
            "--to",
            target.path().to_str().unwrap(),
        ])
        .env("NOTIGNORED_RELEASE_BASE_URL", &release.base_url)
        .env("PATH", bin.path())
        .output()
        .expect("run install.sh");

    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = stderr_of(&output);
    assert!(stderr.contains("no SHA-256 tool found"), "{stderr}");
    assert!(
        stderr.contains("refusing to install unverified"),
        "{stderr}"
    );
    assert!(!target.path().join("notignored").exists());
}

/// Resolve a command the way `command -v` does, so the pared-down PATH above
/// holds real binaries rather than stubs.
fn which(tool: &str) -> Result<PathBuf, ()> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool}"))
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let path = String::from_utf8(output.stdout)
        .map_err(|_| ())?
        .trim()
        .to_string();
    if path.is_empty() {
        Err(())
    } else {
        Ok(PathBuf::from(path))
    }
}

#[test]
fn help_documents_the_flags_the_readme_advertises() {
    let output = Command::new("sh")
        .arg(repo_root().join("scripts/install.sh"))
        .arg("--help")
        .output()
        .expect("run install.sh --help");
    assert!(output.status.success());
    let help = stderr_of(&output);
    for flag in ["--version", "--to"] {
        assert!(
            help.contains(flag),
            "install.sh --help omits {flag}:\n{help}"
        );
    }
}

#[test]
fn an_unknown_argument_is_rejected_with_usage() {
    let output = Command::new("sh")
        .arg(repo_root().join("scripts/install.sh"))
        .arg("--nope")
        .output()
        .expect("run install.sh");
    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = stderr_of(&output);
    assert!(stderr.contains("unknown argument"), "{stderr}");
    assert!(stderr.contains("Usage:"), "{stderr}");
}
