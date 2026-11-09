//! The action's install step under a failing release host.
//!
//! A 504 on one checksum download failed every repository's suppressions check,
//! so the install retries what can recover and refuses at once what cannot. These
//! journeys run the real step lifted out of `action.yml`, so the real
//! `scripts/install.sh` and a real curl or wget talk to a loopback server that
//! really answers 5xx, really never replies, and really drops the connection.
//! Only the wait times are shortened, through the script's documented settings;
//! one journey leaves them at their defaults.
//!
//! POSIX-only, for the reason given in [`crate::installer`].
#![cfg(unix)]

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::action_scan::step_script;
use crate::installer::{
    host_target, path_with, publish, read_request, respond_from, Release, HASHERS,
};

const TAG: &str = "v9.9.9";
const LATEST_PATH: &str = "/repos/nickderobertis/notignored/releases/latest";
/// Attempts, and the delays between them, for the runs that exhaust a retry.
const ATTEMPTS: usize = 4;
const DELAYS: [u64; 3] = [1, 2, 2];
/// How long any one run of the install step may take. Exhausting a retry here
/// takes about 13 seconds; the margin is for a loaded runner.
const INSTALL_DEADLINE: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug)]
enum Fault {
    Status(u16),
    /// Read the request and never answer it.
    Hang,
    /// Close the connection without a byte of response.
    DropBeforeReply,
    /// Promise a body, send part of it, and close.
    DropMidBody,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Lookup,
    Archive,
    Checksum,
}

impl Op {
    fn path(self) -> String {
        match self {
            Op::Lookup => LATEST_PATH.to_string(),
            Op::Archive => format!("/{TAG}/notignored-{TAG}-{}.tar.gz", host_target()),
            Op::Checksum => format!("/{TAG}/notignored-{TAG}-{}.sha256", host_target()),
        }
    }

    /// How the failure names this operation.
    fn name(self) -> &'static str {
        match self {
            Op::Lookup => "the latest release of nickderobertis/notignored",
            Op::Archive => "archive notignored-",
            Op::Checksum => "the checksum for notignored-",
        }
    }

    /// The `version` input that makes the step perform this operation.
    fn version(self) -> &'static str {
        match self {
            Op::Lookup => "latest",
            Op::Archive | Op::Checksum => TAG,
        }
    }
}

/// The three ways a release host fails transiently.
const KINDS: [(&str, &[Fault]); 3] = [
    (
        "server error",
        &[Fault::Status(502), Fault::Status(503), Fault::Status(504)],
    ),
    ("timeout", &[Fault::Hang, Fault::Hang]),
    (
        "dropped connection",
        &[
            Fault::DropBeforeReply,
            Fault::DropMidBody,
            Fault::DropBeforeReply,
        ],
    ),
];

#[derive(Clone, Copy)]
enum Downloader {
    Curl,
    Wget,
}

struct Plan {
    path: String,
    faults: VecDeque<Fault>,
    /// Replayed once `faults` runs out; empty means answer truthfully.
    forever: Vec<Fault>,
}

struct FaultServer {
    base_url: String,
    arrivals: Arc<Mutex<Vec<(String, Instant)>>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for FaultServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl FaultServer {
    /// When each request for `path` arrived, in order.
    fn arrivals_at(&self, path: &str) -> Vec<Instant> {
        self.arrivals
            .lock()
            .expect("the arrivals lock")
            .iter()
            .filter(|(seen, _)| seen == path)
            .map(|(_, at)| *at)
            .collect()
    }
}

/// Serve `root`, applying `plan` to the one path it names.
fn serve_with(root: PathBuf, mut plan: Plan) -> FaultServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
    let port = listener.local_addr().expect("the bound address").port();
    listener
        .set_nonblocking(true)
        .expect("non-blocking listener");
    let arrivals = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&arrivals);
    let shutdown = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&shutdown);

    let worker = std::thread::spawn(move || {
        // Hung connections stay open here until the server goes, so the client
        // sees silence and has to time out on its own.
        let mut hung: Vec<TcpStream> = Vec::new();
        let mut replayed = 0;
        while !stop.load(Ordering::SeqCst) {
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => break,
            };
            let Some((_, path)) = read_request(&stream) else {
                continue;
            };
            log.lock()
                .expect("the arrivals lock")
                .push((path.clone(), Instant::now()));

            let fault = if path != plan.path {
                None
            } else if let Some(fault) = plan.faults.pop_front() {
                Some(fault)
            } else if plan.forever.is_empty() {
                None
            } else {
                replayed += 1;
                Some(plan.forever[(replayed - 1) % plan.forever.len()])
            };
            match fault {
                None => respond_from(stream, &root, &path),
                Some(fault) => apply(fault, stream, &mut hung),
            }
        }
    });

    FaultServer {
        base_url: format!("http://127.0.0.1:{port}"),
        arrivals,
        shutdown,
        worker: Some(worker),
    }
}

fn apply(fault: Fault, mut stream: TcpStream, hung: &mut Vec<TcpStream>) {
    match fault {
        Fault::Status(code) => {
            let _ = write!(
                stream,
                "HTTP/1.1 {code} Failing\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
        Fault::Hang => hung.push(stream),
        Fault::DropBeforeReply => {
            let _ = stream.shutdown(Shutdown::Both);
        }
        Fault::DropMidBody => {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\n\r\npartial");
            let _ = stream.flush();
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

/// A release behind a server failing the way `plan` says, and a place to install.
struct Host {
    server: FaultServer,
    /// Held so the published files outlive the server reading them.
    _release: Release,
    temp: tempfile::TempDir,
}

impl Host {
    fn new(release: Release, op: Op, faults: &[Fault], forever: &[Fault]) -> Self {
        let server = serve_with(
            release.dir.join("releases"),
            Plan {
                path: op.path(),
                faults: faults.iter().copied().collect(),
                forever: forever.to_vec(),
            },
        );
        Self {
            server,
            _release: release,
            temp: tempfile::tempdir().expect("a runner temp directory"),
        }
    }

    fn bin_dir(&self) -> PathBuf {
        self.temp.path().join("notignored/bin")
    }

    fn install(&self, downloader: Downloader, version: &str, settings: &[(&str, &str)]) -> Output {
        let step = self.temp.path().join("install-step.sh");
        std::fs::write(&step, step_script("Install notignored")).expect("write the step script");
        let outputs = self.temp.path().join("outputs.txt");
        std::fs::write(&outputs, "").expect("create the step output file");

        let mut command = Command::new("bash");
        command
            .arg(&step)
            .current_dir(self.temp.path())
            .env("VERSION", version)
            .env("GITHUB_ACTION_PATH", crate::support::repo_root())
            .env("RUNNER_TEMP", self.temp.path())
            .env("GITHUB_OUTPUT", &outputs)
            .env("NOTIGNORED_RELEASE_BASE_URL", &self.server.base_url)
            .env("NOTIGNORED_RELEASE_API_URL", &self.server.base_url)
            .env_remove("GITHUB_TOKEN")
            .env_remove("NOTIGNORED_VERSION")
            .env_remove("NOTIGNORED_INSTALL_DIR")
            .envs(settings.iter().copied());
        // Held for the whole run: dropping it would take the PATH away.
        let narrowed = match downloader {
            Downloader::Curl => None,
            Downloader::Wget => Some(path_with(&[&["bash", "wget"], HASHERS].concat())),
        };
        if let Some(bin) = &narrowed {
            command.env("PATH", bin.path());
        }
        run_with_deadline(command, self.temp.path())
    }

    /// `notignored --version`, found on a PATH holding the directory installed to.
    fn assert_runnable(&self) {
        let run = Command::new("notignored")
            .arg("--version")
            .env("PATH", self.bin_dir())
            .output()
            .expect("run the installed binary from the PATH");
        assert!(run.status.success(), "{}", text(&run));
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            format!("notignored {}", env!("CARGO_PKG_VERSION"))
        );
    }

    fn assert_nothing_installed(&self, output: &Output) {
        assert!(
            !self.bin_dir().join("notignored").exists(),
            "a binary was installed anyway:\n{}",
            text(output)
        );
    }

    fn url_of(&self, op: Op) -> String {
        format!("{}{}", self.server.base_url, op.path())
    }
}

fn text(output: &Output) -> String {
    format!(
        "{:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn healthy_release() -> Release {
    publish(TAG, false, true)
}

/// Run the step, failing the test if it outlives [`INSTALL_DEADLINE`].
///
/// An installer that never gives up on a silent server must fail here rather
/// than hang the suite. The step runs in its own process group, because killing
/// only the shell would leave the downloader it spawned waiting on the socket;
/// its output goes to files, which no surviving process can hold a read open on.
fn run_with_deadline(mut command: Command, dir: &Path) -> Output {
    let stdout = dir.join("step.stdout");
    let stderr = dir.join("step.stderr");
    let mut child = command
        .process_group(0)
        .stdout(File::create(&stdout).expect("create the stdout capture"))
        .stderr(File::create(&stderr).expect("create the stderr capture"))
        .spawn()
        .expect("run the install step");

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the install step") {
            break status;
        }
        if started.elapsed() >= INSTALL_DEADLINE {
            // The shell's builtin, which takes a negative id for the group.
            let _ = Command::new("sh")
                .arg("-c")
                .arg(format!("kill -KILL -{}", child.id()))
                .status();
            let _ = child.wait();
            panic!(
                "the install step had not exited after the {INSTALL_DEADLINE:?} deadline\n\
                 --- stderr ---\n{}",
                std::fs::read_to_string(&stderr).unwrap_or_default()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: std::fs::read(&stdout).expect("read the stdout capture"),
        stderr: std::fs::read(&stderr).expect("read the stderr capture"),
    }
}

/// Each kind answered a few times, then the truth: a working install, having
/// asked exactly once more than it was failed.
fn recovers(downloader: Downloader, op: Op) {
    for (kind, faults) in KINDS {
        let host = Host::new(healthy_release(), op, faults, &[]);
        let output = host.install(
            downloader,
            op.version(),
            &[
                ("NOTIGNORED_DOWNLOAD_FIRST_DELAY", "0"),
                ("NOTIGNORED_DOWNLOAD_TIMEOUT", "2"),
            ],
        );
        assert!(output.status.success(), "{kind}: {}", text(&output));
        assert_eq!(
            host.server.arrivals_at(&op.path()).len(),
            faults.len() + 1,
            "{kind}: {}",
            text(&output)
        );
        host.assert_runnable();
    }
}

/// Each kind on every attempt: a bounded number of attempts, spaced by a delay
/// that grows to its cap, in bounded time ([`INSTALL_DEADLINE`] enforces it), and
/// a failure naming what failed.
fn exhausts(downloader: Downloader, op: Op) {
    std::thread::scope(|scope| {
        for (kind, faults) in KINDS {
            scope.spawn(move || exhausts_one(downloader, op, kind, faults));
        }
    });
}

fn exhausts_one(downloader: Downloader, op: Op, kind: &str, faults: &[Fault]) {
    let host = Host::new(healthy_release(), op, &[], faults);
    let attempts = ATTEMPTS.to_string();
    let output = host.install(
        downloader,
        op.version(),
        &[
            ("NOTIGNORED_DOWNLOAD_ATTEMPTS", attempts.as_str()),
            ("NOTIGNORED_DOWNLOAD_FIRST_DELAY", "1"),
            ("NOTIGNORED_DOWNLOAD_MAX_DELAY", "2"),
            ("NOTIGNORED_DOWNLOAD_TIMEOUT", "2"),
        ],
    );
    let report = format!("{kind}: {}", text(&output));

    assert_eq!(output.status.code(), Some(1), "{report}");
    host.assert_nothing_installed(&output);

    let arrivals = host.server.arrivals_at(&op.path());
    assert_eq!(arrivals.len(), ATTEMPTS, "{report}");
    for (pair, delay) in arrivals.windows(2).zip(DELAYS) {
        let gap = pair[1].duration_since(pair[0]);
        assert!(
            gap >= Duration::from_secs(delay),
            "attempts {gap:?} apart, wanted at least {delay}s\n{report}"
        );
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let announced: Vec<&str> = stderr
        .lines()
        .filter_map(|line| {
            line.rsplit("retrying in ")
                .next()
                .filter(|_| line.contains("retrying in "))
        })
        .collect();
    assert_eq!(announced, ["1s", "2s", "2s"], "{report}");
    let last = stderr.lines().last().unwrap_or_default();
    assert!(
        last.contains(op.name())
            && last.contains(&host.url_of(op))
            && last.contains(&format!("after {ATTEMPTS} attempts")),
        "{report}"
    );
}

#[test]
fn a_flaky_lookup_recovers() {
    recovers(Downloader::Curl, Op::Lookup);
}

#[test]
fn a_flaky_archive_download_recovers() {
    recovers(Downloader::Curl, Op::Archive);
}

#[test]
fn a_flaky_checksum_download_recovers() {
    recovers(Downloader::Curl, Op::Checksum);
}

#[test]
fn a_flaky_lookup_recovers_under_wget() {
    recovers(Downloader::Wget, Op::Lookup);
}

#[test]
fn a_flaky_archive_download_recovers_under_wget() {
    recovers(Downloader::Wget, Op::Archive);
}

#[test]
fn a_flaky_checksum_download_recovers_under_wget() {
    recovers(Downloader::Wget, Op::Checksum);
}

#[test]
fn a_lookup_failing_every_attempt_gives_up_in_bounded_time() {
    exhausts(Downloader::Curl, Op::Lookup);
}

#[test]
fn an_archive_failing_every_attempt_gives_up_in_bounded_time() {
    exhausts(Downloader::Curl, Op::Archive);
}

#[test]
fn a_checksum_failing_every_attempt_gives_up_in_bounded_time() {
    exhausts(Downloader::Curl, Op::Checksum);
}

#[test]
fn a_lookup_failing_every_attempt_gives_up_under_wget() {
    exhausts(Downloader::Wget, Op::Lookup);
}

#[test]
fn an_archive_failing_every_attempt_gives_up_under_wget() {
    exhausts(Downloader::Wget, Op::Archive);
}

#[test]
fn a_checksum_failing_every_attempt_gives_up_under_wget() {
    exhausts(Downloader::Wget, Op::Checksum);
}

/// The incident itself, on the settings consumers get: two 504s on the checksum
/// and then an install, 2s and 4s apart.
#[test]
fn the_default_settings_ride_out_a_checksum_504() {
    let faults = [Fault::Status(504), Fault::Status(504)];
    let host = Host::new(healthy_release(), Op::Checksum, &faults, &[]);
    let output = host.install(Downloader::Curl, TAG, &[]);
    assert!(output.status.success(), "{}", text(&output));
    host.assert_runnable();

    let arrivals = host.server.arrivals_at(&Op::Checksum.path());
    assert_eq!(arrivals.len(), 3, "{}", text(&output));
    assert!(arrivals[1].duration_since(arrivals[0]) >= Duration::from_secs(2));
    assert!(arrivals[2].duration_since(arrivals[1]) >= Duration::from_secs(4));
}

/// The attempt bound consumers get, with only the wait removed.
#[test]
fn the_default_attempt_bound_is_five() {
    let host = Host::new(healthy_release(), Op::Archive, &[], &[Fault::Status(503)]);
    let output = host.install(
        Downloader::Curl,
        TAG,
        &[("NOTIGNORED_DOWNLOAD_FIRST_DELAY", "0")],
    );
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert_eq!(host.server.arrivals_at(&Op::Archive.path()).len(), 5);
}

/// Throttling and a request timeout are the server asking to be asked again.
#[test]
fn a_throttled_or_timed_out_response_is_retried() {
    let faults = [Fault::Status(429), Fault::Status(408)];
    let host = Host::new(healthy_release(), Op::Archive, &faults, &[]);
    let output = host.install(
        Downloader::Curl,
        TAG,
        &[("NOTIGNORED_DOWNLOAD_FIRST_DELAY", "0")],
    );
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(host.server.arrivals_at(&Op::Archive.path()).len(), 3);
}

/// Remove what `op` asks for, so the server's own 404 answers it.
fn delete_target(release: &Release, op: Op) {
    let served = release.dir.join("releases");
    std::fs::remove_file(served.join(op.path().trim_start_matches('/')))
        .expect("remove the published file");
}

fn fails_once_on_404(downloader: Downloader, op: Op) {
    let release = healthy_release();
    delete_target(&release, op);
    let host = Host::new(release, op, &[], &[]);
    let output = host.install(downloader, op.version(), &[]);

    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert_eq!(
        host.server.arrivals_at(&op.path()).len(),
        1,
        "a 404 was retried: {}",
        text(&output)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(op.name())
            && stderr.contains("not found")
            && stderr.contains(&host.url_of(op))
            && stderr.contains("HTTP 404"),
        "{}",
        text(&output)
    );
    assert!(!stderr.contains("retrying"), "{}", text(&output));
    host.assert_nothing_installed(&output);
}

#[test]
fn a_release_that_does_not_exist_fails_on_the_first_lookup() {
    fails_once_on_404(Downloader::Curl, Op::Lookup);
}

#[test]
fn a_missing_archive_fails_on_the_first_attempt() {
    fails_once_on_404(Downloader::Curl, Op::Archive);
}

#[test]
fn a_missing_checksum_fails_on_the_first_attempt() {
    fails_once_on_404(Downloader::Curl, Op::Checksum);
}

#[test]
fn a_404_under_wget_fails_on_the_first_attempt() {
    fails_once_on_404(Downloader::Wget, Op::Lookup);
    fails_once_on_404(Downloader::Wget, Op::Archive);
    fails_once_on_404(Downloader::Wget, Op::Checksum);
}

/// A tag nobody released is a 404 on its archive, which is told apart from an
/// outage by naming the release.
#[test]
fn a_tag_nobody_released_fails_on_the_first_attempt_naming_it() {
    let host = Host::new(healthy_release(), Op::Archive, &[], &[]);
    let output = host.install(Downloader::Curl, "v8.8.8", &[]);

    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    let requested = format!("/v8.8.8/notignored-v8.8.8-{}.tar.gz", host_target());
    assert_eq!(host.server.arrivals_at(&requested).len(), 1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("HTTP 404") && stderr.contains("release v8.8.8"),
        "{}",
        text(&output)
    );
}

/// Any other client error is as permanent as a 404, and says which it was.
#[test]
fn a_forbidden_lookup_fails_on_the_first_attempt() {
    let host = Host::new(healthy_release(), Op::Lookup, &[], &[Fault::Status(403)]);
    let output = host.install(Downloader::Curl, "latest", &[]);

    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert_eq!(host.server.arrivals_at(LATEST_PATH).len(), 1);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("HTTP 403"),
        "{}",
        text(&output)
    );
    host.assert_nothing_installed(&output);
}

/// Retrying the transport must not soften verification: a mismatch is refused,
/// after a retried download as after a clean one, and is never itself retried.
#[test]
fn a_checksum_mismatch_is_still_refused() {
    for faults in [vec![], vec![Fault::Status(503)]] {
        let host = Host::new(publish(TAG, true, true), Op::Checksum, &faults, &[]);
        let output = host.install(
            Downloader::Curl,
            TAG,
            &[("NOTIGNORED_DOWNLOAD_FIRST_DELAY", "0")],
        );

        assert_eq!(output.status.code(), Some(1), "{}", text(&output));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"),
            "{}",
            text(&output)
        );
        assert_eq!(
            host.server.arrivals_at(&Op::Checksum.path()).len(),
            faults.len() + 1
        );
        assert_eq!(host.server.arrivals_at(&Op::Archive.path()).len(), 1);
        host.assert_nothing_installed(&output);
    }
}

/// A setting that would reach shell arithmetic is refused by name first.
#[test]
fn a_malformed_retry_setting_is_refused_before_any_download() {
    let host = Host::new(healthy_release(), Op::Archive, &[], &[]);
    for (name, value) in [
        ("NOTIGNORED_DOWNLOAD_ATTEMPTS", "0"),
        ("NOTIGNORED_DOWNLOAD_ATTEMPTS", "many"),
        ("NOTIGNORED_DOWNLOAD_FIRST_DELAY", "08"),
        ("NOTIGNORED_DOWNLOAD_MAX_DELAY", "-1"),
        ("NOTIGNORED_DOWNLOAD_TIMEOUT", "0"),
        ("NOTIGNORED_DOWNLOAD_TIMEOUT", "99999999999999999999"),
    ] {
        let output = host.install(Downloader::Curl, TAG, &[(name, value)]);
        assert_eq!(output.status.code(), Some(1), "{name}={value}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(name),
            "{name}={value}: {}",
            text(&output)
        );
    }
    assert!(host.server.arrivals_at(&Op::Archive.path()).is_empty());
}
