//! The npm publisher, executed against a registry that answers for real.
//!
//! `scripts/publish-npm.sh` decides whether a release publishes. Getting that
//! wrong is expensive in one direction and silent in the other: republishing a
//! live version red-fails a release that already succeeded, and treating an
//! auth or outage error as "not published yet" would push over a registry that
//! merely could not answer. Neither shows up locally, and neither can be
//! rehearsed against npmjs.com — a public publish cannot be taken back.
//!
//! So the registry is the one host these journeys cannot own, and it is the only
//! thing substituted: **real `npm`** talks HTTP to a server the test runs on
//! loopback, exactly as `tests/e2e/action_comment.rs` does for `gh` and
//! github.com. The script is the real one, the package is a real one assembled
//! by `scripts/npm-build.mjs`, and `npm view` / `npm publish` are the real
//! client. What the server chooses to answer is what puts the script in each of
//! the three states it has to tell apart.
//!
//! Unix only: `publish-npm.sh` is a POSIX-shell surface, and the release job that
//! calls it runs on ubuntu-latest.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::support::{cargo_version, repo_root};

/// How the fake registry answers a metadata read.
#[derive(Clone)]
enum Registry {
    /// The package is live at the versions listed: 200 with a versions map.
    ///
    /// What is in that map decides the whole journey, so it is never a literal:
    /// see [`this_version`] and [`an_earlier_version`].
    Published(Vec<String>),
    /// The package has never been published: 404, which is npm's `E404`.
    Absent,
    /// The registry is having a bad day: 503, which is neither of the above.
    Unavailable,
    /// The package is absent, but the upload is refused — a token without rights
    /// to this name, which is what a mis-scoped automation token looks like.
    AbsentAndUnwritable,
}

/// A registry on loopback, and the record of what was PUT to it.
struct FakeRegistry {
    url: String,
    published: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for FakeRegistry {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The version this build publishes — the one `scripts/npm-build.mjs` stamps
/// into the package it assembles, read from `Cargo.toml` at run time.
fn this_version() -> String {
    cargo_version()
}

/// The version a previous release left on the registry: this build's with its
/// last non-zero component stepped back, so it is derived from the one version
/// source and no future bump can make the two collide.
///
/// Strictly *lower* rather than merely different, because npm is: it refuses to
/// move the `latest` tag backwards, so publishing under a registry whose latest
/// outranks this build fails on the tag rather than on the decision under test.
fn an_earlier_version() -> String {
    let version = this_version();
    // A pre-release or build suffix would not parse; the crate has never carried
    // one, and release-plz does not produce them.
    let mut parts: Vec<u32> = version
        .split('.')
        .map(|part| {
            part.parse()
                .unwrap_or_else(|error| panic!("{version} is not major.minor.patch: {error}"))
        })
        .collect();
    assert_eq!(parts.len(), 3, "{version} is not major.minor.patch");
    let last = parts
        .iter()
        .rposition(|&part| part > 0)
        .unwrap_or_else(|| panic!("no version is below {version}"));
    parts[last] -= 1;
    parts
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

impl FakeRegistry {
    /// Serve until dropped, answering every metadata read with `answer`.
    fn start(answer: Registry) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
        let port = listener.local_addr().expect("the bound address").port();
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");

        let shutdown = Arc::new(AtomicBool::new(false));
        let published = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::clone(&shutdown);
        let recorded = Arc::clone(&published);
        let worker = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => serve_one(stream, &answer, &recorded),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        FakeRegistry {
            url: format!("http://127.0.0.1:{port}/"),
            published,
            shutdown,
            worker: Some(worker),
        }
    }

    /// The package names `npm publish` uploaded, in order.
    fn publishes(&self) -> Vec<String> {
        self.published.lock().expect("the publish log").clone()
    }
}

/// Answer one npm request.
///
/// npm reads a package with `GET /<name>` and publishes with `PUT /<name>`. Only
/// what the script branches on is modelled: the status, and for a published
/// package the `versions` map it matches the requested version against.
fn serve_one(mut stream: TcpStream, answer: &Registry, published: &Arc<Mutex<Vec<String>>>) {
    // An accepted socket inherits the listener's O_NONBLOCK on BSD-derived
    // systems (macOS) but not on Linux, so set it rather than depend on which.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));

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
    let Some(request) = head.lines().next().map(str::to_string) else {
        return;
    };
    let mut fields = request.split_whitespace();
    let method = fields.next().unwrap_or_default().to_string();
    let path = fields.next().unwrap_or("/").to_string();
    // npm URL-encodes a scoped name's slash; these packages are unscoped, so the
    // path after the leading `/` is the name.
    let name = path.trim_start_matches('/').split('?').next().unwrap_or("");

    // Drain the body so npm sees its upload accepted rather than a reset.
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

    let (status, body) = if method == "PUT" {
        published
            .lock()
            .expect("the publish log")
            .push(name.to_string());
        match answer {
            Registry::AbsentAndUnwritable => (
                "403 Forbidden",
                format!("{{\"error\":\"you do not have permission to publish {name}\"}}"),
            ),
            _ => ("201 Created", format!("{{\"ok\":\"created {name}\"}}")),
        }
    } else {
        match answer {
            Registry::Published(versions) => ("200 OK", packument(name, versions)),
            Registry::Absent | Registry::AbsentAndUnwritable => {
                ("404 Not Found", "{\"error\":\"Not found\"}".to_string())
            }
            Registry::Unavailable => (
                "503 Service Unavailable",
                "{\"error\":\"registry is down\"}".to_string(),
            ),
        }
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// The metadata document npm reads for `name`, listing exactly `versions`.
///
/// `npm view <name>@<version>` fetches this whole document and resolves the
/// requested version against the `versions` map itself, so a name that is live
/// at some *other* version answers 200 here and still reports `E404` to the
/// script — which is precisely the state every release after the first is in.
fn packument(name: &str, versions: &[String]) -> String {
    let latest = versions.last().expect("a published package has a version");
    let entries: Vec<String> = versions
        .iter()
        .map(|version| format!("\"{version}\":{{\"name\":\"{name}\",\"version\":\"{version}\"}}"))
        .collect();
    format!(
        "{{\"name\":\"{name}\",\"dist-tags\":{{\"latest\":\"{latest}\"}},\
         \"versions\":{{{}}}}}",
        entries.join(",")
    )
}

/// Assemble the launcher package at the crate's own version, into `out`.
fn launcher_package(out: &Path) -> std::path::PathBuf {
    let assembled = Command::new("node")
        .current_dir(repo_root())
        .arg("scripts/npm-build.mjs")
        .arg("launcher")
        .arg("--out")
        .arg(out)
        .output()
        .expect("run npm-build.mjs; ACTION: run `just bootstrap` if node is missing");
    assert!(
        assembled.status.success(),
        "npm-build.mjs launcher failed: {}",
        String::from_utf8_lossy(&assembled.stderr)
    );
    std::path::PathBuf::from(
        String::from_utf8_lossy(&assembled.stdout)
            .trim()
            .to_string(),
    )
}

/// Run `scripts/publish-npm.sh` against `registry` for one package directory.
///
/// `HOME` is redirected at a scratch directory holding the only `.npmrc` in
/// play, so a developer's own registry, token, or proxy cannot decide what these
/// journeys observe.
fn publish(registry: &FakeRegistry, package: &Path, home: &Path) -> Output {
    // npm authenticates per registry, keyed by the URL with the scheme stripped —
    // a global token is ignored, and without a matching one `npm publish` stops
    // at ENEEDAUTH before it ever reaches the server.
    let host = registry
        .url
        .strip_prefix("http://")
        .expect("a loopback http url");
    std::fs::write(
        home.join(".npmrc"),
        format!(
            "registry={}\n//{host}:_authToken=publish-npm-e2e\n\
             // A 5xx is a real answer here, not something to wait out: npm's\n\
             // default retries would spend a minute rediscovering it.\n\
             fetch-retries=0\n",
            registry.url
        ),
    )
    .expect("write the scratch .npmrc");

    Command::new("bash")
        .current_dir(repo_root())
        .arg("scripts/publish-npm.sh")
        .arg(package)
        .env("HOME", home)
        .env("npm_config_cache", home.join("npm-cache"))
        .output()
        .expect("run publish-npm.sh")
}

/// A scratch HOME plus an assembled package, for one journey.
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let package = launcher_package(&scratch.path().join("dist"));
    (scratch, package)
}

/// A name the registry has never seen is published.
#[test]
fn a_version_the_registry_does_not_have_is_published() {
    let registry = FakeRegistry::start(Registry::Absent);
    let (scratch, package) = fixture();

    let output = publish(&registry, &package, scratch.path());
    assert!(
        output.status.success(),
        "publishing a new version must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        registry.publishes(),
        vec!["notignored-cli".to_string()],
        "the registry did not receive exactly one publish"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("published notignored-cli@{}", this_version()))
            && stdout.contains("already on npm none"),
        "the run's one summary line does not say what it published:\n{stdout}"
    );
}

/// A name that is live, but not at this version, is published.
///
/// This is what every release after the first actually meets, and it is a
/// different registry answer from an absent package: 200 with a metadata
/// document, which npm then resolves the requested version against and reports
/// `E404` for. Mistaking "the name exists" for "this version exists" would skip
/// every publish from the second release onward, and the release would report
/// success with nothing new on the registry.
#[test]
fn a_name_live_at_another_version_still_publishes_this_one() {
    let registry = FakeRegistry::start(Registry::Published(vec![an_earlier_version()]));
    let (scratch, package) = fixture();

    let output = publish(&registry, &package, scratch.path());
    assert!(
        output.status.success(),
        "publishing a version the registry does not list must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        registry.publishes(),
        vec!["notignored-cli".to_string()],
        "the script skipped a version that was not on the registry"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("published notignored-cli@{}", this_version()))
            && stdout.contains("already on npm none"),
        "the run's one summary line does not say what it published:\n{stdout}"
    );
}

/// A version already live is left alone.
///
/// npm versions are immutable, so a re-run — to finish a sibling job, say —
/// would fail on the second publish and redden a release that had worked. The
/// script asks first, and this is what proves it does not ask and publish anyway.
///
/// The version the registry holds is *this build's*, read from `Cargo.toml`:
/// spelling it out would make the next release PR — whose only payload is that
/// bump — arrive at the gate looking like a double-publish.
#[test]
fn a_version_already_on_the_registry_is_not_published_again() {
    let registry = FakeRegistry::start(Registry::Published(vec![this_version()]));
    let (scratch, package) = fixture();

    let output = publish(&registry, &package, scratch.path());
    assert!(
        output.status.success(),
        "an already-published version is not an error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        registry.publishes().is_empty(),
        "the script published over a version that was already live: {:?}",
        registry.publishes()
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("published none")
            && stdout.contains(&format!("already on npm notignored-cli@{}", this_version())),
        "the run's one summary line does not say what it skipped:\n{stdout}"
    );
    assert_eq!(
        stdout.lines().count(),
        1,
        "a successful run owes a release log one line, not a commentary:\n{stdout}"
    );
}

/// A registry that answers, but not with an answer, fails the release closed.
///
/// This is the branch that matters most. "Cannot read the metadata" and "the
/// package is not published" look alike from the outside, and a script that
/// conflated them would publish blind through an outage — or worse, through an
/// authentication failure. Only a 404 is permission to publish.
#[test]
fn a_registry_that_will_not_answer_fails_the_release_rather_than_publishing() {
    let registry = FakeRegistry::start(Registry::Unavailable);
    let (scratch, package) = fixture();

    let output = publish(&registry, &package, scratch.path());
    assert!(
        !output.status.success(),
        "an unreadable registry must fail the release, not publish into it"
    );
    assert!(
        registry.publishes().is_empty(),
        "the script published despite not knowing what was there: {:?}",
        registry.publishes()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot query") && stderr.contains("reachable"),
        "the failure does not say what to do about it:\n{stderr}"
    );
}

/// Handed nothing to publish, the script says so rather than succeeding quietly.
///
/// The release job builds its argument list from a glob, and a glob that matched
/// nothing would otherwise make "published no packages" look like a clean run.
#[test]
fn no_package_argument_is_refused() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let output = Command::new("bash")
        .current_dir(repo_root())
        .arg("scripts/publish-npm.sh")
        .env("HOME", scratch.path())
        .output()
        .expect("run publish-npm.sh");
    assert!(
        !output.status.success(),
        "an empty argument list is not a publish"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("at least one package"),
        "the refusal does not say what was missing:\n{stderr}"
    );
}

/// A publish the registry refuses fails the release, and says what npm said.
///
/// The registry agrees the version is new, so the script is right to try — and
/// then the upload is rejected anyway, which is what a token scoped to the wrong
/// package looks like. Swallowing that would leave a release reporting success
/// with nothing on the registry, so the exit code has to carry it and npm's own
/// reason has to reach the log.
#[test]
fn a_publish_the_registry_refuses_fails_the_release() {
    let registry = FakeRegistry::start(Registry::AbsentAndUnwritable);
    let (scratch, package) = fixture();

    let output = publish(&registry, &package, scratch.path());
    assert!(
        !output.status.success(),
        "a refused upload must fail the release: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        registry.publishes(),
        vec!["notignored-cli".to_string()],
        "the script did not actually attempt the publish it reported failing"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not publish") && stderr.contains("re-run the release"),
        "the failure does not say what to do about it:\n{stderr}"
    );
    assert!(
        stderr.contains("permission"),
        "npm's own reason never reached the log:\n{stderr}"
    );
}

/// Something that is not a package at all is refused before any registry call.
///
/// `npm pack` is what turns the argument into a name and version, so a path that
/// does not hold a manifest has no identity to ask the registry about. Guessing
/// one would query — and possibly publish — under a name nobody chose.
#[test]
fn an_argument_that_is_not_a_package_is_refused_before_the_registry_is_asked() {
    let registry = FakeRegistry::start(Registry::Absent);
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let not_a_package = scratch.path().join("not-a-package");
    std::fs::create_dir_all(&not_a_package).expect("create the directory");

    let output = publish(&registry, &not_a_package, scratch.path());
    assert!(
        !output.status.success(),
        "a directory with no manifest is not a package"
    );
    assert!(
        registry.publishes().is_empty(),
        "the script published something it could not name: {:?}",
        registry.publishes()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot read package metadata") && stderr.contains("npm-build.mjs"),
        "the refusal does not name the fix:\n{stderr}"
    );
}
