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
//! Unix only: `install.sh` is the POSIX-shell surface, and the Windows ZIP branch
//! needs a Windows shell plus `unzip`, which the runners do not ship. The install
//! method the README gives Windows users is `cargo install`, and the CI
//! `install` / `install-documented` jobs exercise exactly that on windows-latest.
// llmlint: ignore-file[changed_behavior_has_e2e] the Windows/ZIP branch of install.sh
// cannot run on a Unix host without stubbing `uname` — which would test the stub — and
// the documented Windows install path (`cargo install`) is covered by CI's install jobs
// on windows-latest instead.
#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::support::repo_root;

/// A directory laid out like a GitHub release — one archive plus its `.sha256`,
/// named exactly as `release.yml` publishes them — served over real HTTP.
struct Release {
    dir: PathBuf,
    _tempdir: tempfile::TempDir,
    _server: Option<Server>,
    base_url: String,
    tag: String,
}

/// A minimal static-file HTTP server, in-process.
///
/// The installer has to reach its release over a real socket with a real client
/// (curl or wget) for these journeys to mean anything — but spawning an external
/// server per test made the suite hostage to that program's startup time, which
/// is exactly how it flaked on a loaded macOS runner. This serves the same bytes
/// with no external dependency, and the listener is already bound when the
/// constructor returns, so there is nothing to wait for.
struct Server {
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Serve `root` over HTTP until the returned server is dropped.
fn serve(root: &Path) -> (Server, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
    let port = listener.local_addr().expect("the bound address").port();
    listener
        .set_nonblocking(true)
        .expect("non-blocking listener");

    let shutdown = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&shutdown);
    let root = root.to_path_buf();
    let worker = std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => serve_one(stream, &root),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    (
        Server {
            shutdown,
            worker: Some(worker),
        },
        format!("http://127.0.0.1:{port}"),
    )
}

/// Answer one request: 200 with the file's bytes, or 404. Every request is
/// appended to `<root>/../requests.log` so a test can assert on what was sent.
fn serve_one(mut stream: TcpStream, root: &Path) {
    // On BSD-derived systems (macOS) an accepted socket inherits the listener's
    // O_NONBLOCK, so the read below would return WouldBlock and answer nothing;
    // on Linux it would not. Set it explicitly rather than depend on which.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));

    // One reader for the whole head: a second BufReader would discard whatever
    // the first had already buffered, so the headers would look absent.
    let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
    let mut head = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let blank = line.trim().is_empty();
                head.push_str(&line);
                if blank {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if head.is_empty() {
        return;
    }
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();

    if let Some(log) = root.parent().map(|parent| parent.join("requests.log")) {
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .map(|mut file| file.write_all(head.as_bytes()));
    }

    let relative = path.trim_start_matches('/');
    let body = if relative
        .split('/')
        .any(|part| part == ".." || part.is_empty())
    {
        None
    } else {
        fs::read(root.join(relative)).ok()
    };

    let response = match &body {
        Some(bytes) => format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        ),
        None => {
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
        }
    };
    if stream.write_all(response.as_bytes()).is_err() {
        return;
    }
    if let Some(bytes) = body {
        let _ = stream.write_all(&bytes);
    }
    let _ = stream.flush();
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

    // The `releases/latest` document the API lookup parses, served from the same
    // root so `NOTIGNORED_RELEASE_API_URL` can point at it.
    let api = dir
        .path()
        .join("releases/repos/nickderobertis/notignored/releases");
    fs::create_dir_all(&api).unwrap();
    fs::write(
        api.join("latest"),
        format!("{{\n  \"tag_name\": \"{tag}\",\n  \"name\": \"{tag}\"\n}}\n"),
    )
    .unwrap();

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
        dir: dir.path().to_path_buf(),
        _tempdir: dir,
    }
}

impl Release {
    /// Every request line and header the server received.
    fn requests(&self) -> String {
        fs::read_to_string(self.dir.join("requests.log")).unwrap_or_default()
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
fn a_github_token_is_never_sent_to_a_mirror() {
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();
    let output = Command::new("sh")
        .arg(repo_root().join("scripts/install.sh"))
        .args([
            "--version",
            &release.tag,
            "--to",
            target.path().to_str().unwrap(),
        ])
        .env("NOTIGNORED_RELEASE_BASE_URL", &release.base_url)
        .env("GITHUB_TOKEN", "ghp_supersecrettokenvalue")
        .output()
        .expect("run install.sh");

    assert!(output.status.success(), "{}", stderr_of(&output));
    let requests = release.requests();
    assert!(
        !requests.is_empty(),
        "the mirror received no requests at all"
    );
    assert!(
        !requests.to_lowercase().contains("authorization"),
        "the installer sent a credential to a mirror:\n{requests}"
    );
    assert!(
        !requests.contains("ghp_supersecrettokenvalue"),
        "the token leaked to the mirror:\n{requests}"
    );
}

#[test]
fn a_mirror_url_must_be_http_or_https() {
    let target = tempfile::tempdir().unwrap();
    for url in ["ftp://evil.example/x", "file:///etc", "javascript:alert(1)"] {
        let output = Command::new("sh")
            .arg(repo_root().join("scripts/install.sh"))
            .args([
                "--version",
                "v9.9.9",
                "--to",
                target.path().to_str().unwrap(),
            ])
            .env("NOTIGNORED_RELEASE_BASE_URL", url)
            .output()
            .expect("run install.sh");
        assert_eq!(output.status.code(), Some(1), "{url} was accepted");
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("must start with http:// or https://"),
            "{url}: {stderr}"
        );
        // Refusing is half the message: the reader also has to be told which
        // override to drop to get the published release back.
        assert!(
            stderr.contains("NOTIGNORED_RELEASE_BASE_URL"),
            "{url}: {stderr}"
        );
    }
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
    let scratch = tempfile::tempdir().unwrap();
    let missing = Release {
        _server: None,
        base_url: release.base_url.clone(),
        tag: "v8.8.8".to_string(),
        dir: scratch.path().to_path_buf(),
        _tempdir: scratch,
    };
    let output = install(&missing, target.path(), &[]);

    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    assert!(
        stderr_of(&output).contains("v8.8.8"),
        "{}",
        stderr_of(&output)
    );
}

/// The tools `install.sh` needs before it reaches the step under test. Symlinked
/// into a scratch directory so PATH can be narrowed to exactly these — the real
/// "this tool is missing" boundary, with no stubbing.
const CORE_TOOLS: &[&str] = &[
    "sh", "tar", "gzip", "gunzip", "uname", "mktemp", "rm", "find", "head", "cut", "mkdir",
    "install", "cp", "chmod", "printf", "sed", "tr", "cat",
];

/// SHA-256 tools `install.sh` knows how to use; whichever the host has is enough.
const HASHERS: &[&str] = &["sha256sum", "shasum", "openssl"];

fn path_with(extra: &[&str]) -> tempfile::TempDir {
    let bin = tempfile::tempdir().unwrap();
    for tool in CORE_TOOLS.iter().chain(extra) {
        if let Ok(found) = which(tool) {
            let _ = std::os::unix::fs::symlink(found, bin.path().join(tool));
        }
    }
    bin
}

fn install_with_path(release: &Release, target: &Path, bin: &Path, args: &[&str]) -> Output {
    Command::new("sh")
        .arg(repo_root().join("scripts/install.sh"))
        .args(["--to", target.to_str().unwrap()])
        .args(args)
        .env("NOTIGNORED_RELEASE_BASE_URL", &release.base_url)
        .env("NOTIGNORED_RELEASE_API_URL", &release.base_url)
        .env("PATH", bin)
        .output()
        .expect("run install.sh")
}

#[test]
fn with_no_version_the_installer_resolves_the_latest_release() {
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();
    let bin = path_with(&[&["curl"], HASHERS].concat());
    let output = install_with_path(&release, target.path(), bin.path(), &[]);

    let stderr = stderr_of(&output);
    assert!(output.status.success(), "{:?}: {stderr}", output.status);
    assert!(stderr.contains("notignored v9.9.9 installed"), "{stderr}");
    assert!(target.path().join("notignored").exists());
}

/// A host that can download but cannot hash refuses to install rather than
/// skipping verification — and names the tools that would fix it.
#[test]
fn without_a_sha256_tool_the_installer_says_which_to_install() {
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();
    let bin = path_with(&["curl"]);
    let output = install_with_path(
        &release,
        target.path(),
        bin.path(),
        &["--version", "v9.9.9"],
    );

    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("refusing to install unverified"),
        "{stderr}"
    );
    assert!(stderr.contains("sha256sum"), "{stderr}");
    assert!(!target.path().join("notignored").exists(), "{stderr}");
}

#[test]
fn wget_stands_in_when_curl_is_unavailable() {
    if which("wget").is_err() {
        panic!("wget is required to prove the installer's downloader fallback");
    }
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();
    let bin = path_with(&[&["wget"], HASHERS].concat());
    let output = install_with_path(
        &release,
        target.path(),
        bin.path(),
        &["--version", "v9.9.9"],
    );

    let stderr = stderr_of(&output);
    assert!(output.status.success(), "{:?}: {stderr}", output.status);
    assert!(target.path().join("notignored").exists(), "{stderr}");
}

#[test]
fn without_any_downloader_the_installer_says_what_to_install() {
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();
    let bin = path_with(&[]);
    let output = install_with_path(
        &release,
        target.path(),
        bin.path(),
        &["--version", "v9.9.9"],
    );

    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = stderr_of(&output);
    assert!(stderr.contains("neither curl nor wget"), "{stderr}");
    assert!(stderr.contains("install one and re-run"), "{stderr}");
    assert!(!target.path().join("notignored").exists());
}

#[test]
fn without_a_sha256_tool_the_installer_refuses_rather_than_skipping_verification() {
    let release = publish("v9.9.9", false, true);
    let target = tempfile::tempdir().unwrap();
    // Everything the script needs to reach the hashing step — and no hasher.
    let bin = path_with(&["curl"]);
    let output = install_with_path(
        &release,
        target.path(),
        bin.path(),
        &["--version", "v9.9.9"],
    );

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

/// A flag whose value the caller forgot: naming the flag is half the message,
/// so each failure shows the shape of the value it wanted.
#[test]
fn a_flag_with_no_value_is_rejected_with_an_example() {
    for (flag, example) in [("--version", "v0.1.0"), ("--to", ".local/bin")] {
        let output = Command::new("sh")
            .arg(repo_root().join("scripts/install.sh"))
            .arg(flag)
            .output()
            .expect("run install.sh");
        assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains(&format!("{flag} needs a value")),
            "{stderr}"
        );
        assert!(
            stderr.contains(example),
            "the message shows no example value: {stderr}"
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
