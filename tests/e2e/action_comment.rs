//! The composite action's comment step, driven against a stub GitHub API.
//!
//! GitHub's API is the one boundary these journeys cannot own, so it is the one
//! thing stubbed: the real `scripts/action/comment.sh`, the real `gh` CLI, and
//! the real checked-in comment bodies drive a throwaway HTTP server that records
//! exactly what the script asked it to do. What is proven here is the upsert
//! rule — edit the marked comment when one exists, create it when none does, and
//! post nothing at all when there is nothing to say.
//!
//! POSIX-only: the composite runs bash on GitHub's Linux and macOS runners, and
//! driving it through Git Bash on Windows would be testing the runner's path
//! translation rather than the script.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::support::repo_root;

/// The repository and pull request the journeys pretend to run against.
const REPO: &str = "acme/widgets";
const PULL_REQUEST: &str = "7";
/// The id of the sticky comment a "previous run" left behind.
const STICKY_ID: u64 = 77;

/// One request the script made.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Recorded {
    method: String,
    path: String,
    body: String,
}

/// A throwaway GitHub API that answers the two endpoints the script uses.
struct StubApi {
    address: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    running: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
}

impl StubApi {
    /// Start a stub whose comment list either holds the sticky comment or does
    /// not.
    fn start(sticky: Option<u64>) -> StubApi {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the stub API");
        let address = format!(
            "http://{}",
            listener.local_addr().expect("the stub API address")
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));

        let (log, alive) = (Arc::clone(&requests), Arc::clone(&running));
        let server = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if !alive.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { break };
                // A stub that panics mid-journey would hang the client instead of
                // failing it, so a malformed exchange just closes the connection.
                let _ = serve(&mut stream, sticky, &log);
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
        StubApi {
            address,
            requests,
            running,
            server: Some(server),
        }
    }

    /// Everything the script asked for, in order.
    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("the request log").clone()
    }

    /// The requests that changed something — what the upsert rule is about.
    fn writes(&self) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|request| request.method != "GET")
            .collect()
    }
}

impl Drop for StubApi {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        // Unblock the accept loop so the thread can notice and exit.
        let address = self.address.trim_start_matches("http://").to_string();
        let _ = TcpStream::connect(address);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

/// Answer one request and record it.
fn serve(
    stream: &mut TcpStream,
    sticky: Option<u64>,
    log: &Mutex<Vec<Recorded>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut fields = request_line.split_whitespace();
    let (Some(method), Some(path)) = (fields.next(), fields.next()) else {
        return Ok(());
    };
    // `gh api --paginate` asks for a page size, so the query string is not part
    // of which endpoint was addressed.
    let (method, path) = (
        method.to_string(),
        path.split('?').next().unwrap_or(path).to_string(),
    );

    let mut length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let body = String::from_utf8_lossy(&body).into_owned();

    let comments = format!("/repos/{REPO}/issues/{PULL_REQUEST}/comments");
    let sticky_comment = format!("/repos/{REPO}/issues/comments/{STICKY_ID}");
    let (status, payload) = match (method.as_str(), path.as_str()) {
        ("GET", path) if path == comments => (
            "200 OK",
            match sticky {
                // The list also holds an unrelated comment, so finding the
                // sticky one has to be the marker's doing and not the order's.
                Some(id) => format!(
                    r#"[{{"id":5,"body":"looks good to me"}},{{"id":{id},"body":"{marker}\n\nan earlier run"}}]"#,
                    marker = notignored::cli::MARKER
                ),
                None => r#"[{"id":5,"body":"looks good to me"}]"#.to_string(),
            },
        ),
        ("POST", path) if path == comments => ("201 Created", r#"{"id":101}"#.to_string()),
        ("PATCH", path) if path == sticky_comment => ("200 OK", format!(r#"{{"id":{STICKY_ID}}}"#)),
        _ => (
            "404 Not Found",
            r#"{"message":"the action asked for an endpoint this stub does not serve"}"#
                .to_string(),
        ),
    };
    log.lock()
        .expect("the request log")
        .push(Recorded { method, path, body });

    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
        payload.len()
    )?;
    stream.flush()
}

/// The `gh` the composite calls. Missing, the journey fails with the fix rather
/// than skipping — an unproven upsert must not read as a proven one.
fn require_gh() {
    let found = Command::new("gh").arg("--version").output();
    assert!(
        found.is_ok_and(|output| output.status.success()),
        "the GitHub CLI is not installed\nACTION: install gh (https://cli.github.com) — the \
         composite action calls it, and every GitHub-hosted runner ships it"
    );
}

/// A checked-in golden body, which is what the action really posts.
fn golden_body(count: usize) -> PathBuf {
    repo_root().join(format!("tests/golden/markdown/count-{count}.md"))
}

/// Run the composite's comment step against `api`.
fn comment(api: &StubApi, count: usize) -> Output {
    require_gh();
    let body = golden_body(count);
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &api.address)
        .env("PR_NUMBER", PULL_REQUEST)
        .env("BODY_FILE", &body)
        .env("COUNT", count.to_string())
        .env("GH_TOKEN", "stub-token")
        // Insulate gh from the developer's own configuration and hosts file:
        // the journey must talk to the stub and to nothing else.
        .env("GH_CONFIG_DIR", repo_root().join("target/gh-config"))
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_HOST")
        .env_remove("GH_ENTERPRISE_TOKEN")
        .env_remove("GITHUB_EVENT_PATH")
        .output()
        .expect("run comment.sh");
    assert!(
        output.status.success(),
        "comment.sh failed ({:?}):\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// The body the script sent, as the API received it.
fn posted_body(request: &Recorded) -> String {
    let payload: serde_json::Value =
        serde_json::from_str(&request.body).expect("the request body is JSON");
    payload["body"]
        .as_str()
        .expect("the request sets the comment body")
        .to_string()
}

#[test]
fn a_pull_request_with_no_comment_yet_gets_one() {
    let api = StubApi::start(None);
    let output = comment(&api, 3);

    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:#?}");
    assert_eq!(writes[0].method, "POST");
    assert_eq!(
        writes[0].path,
        format!("/repos/{REPO}/issues/{PULL_REQUEST}/comments")
    );
    assert_eq!(
        posted_body(&writes[0]),
        std::fs::read_to_string(golden_body(3)).unwrap(),
        "the comment body is not the rendered report"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("commented on #7"));
}

#[test]
fn a_second_run_edits_the_comment_the_first_one_left() {
    let api = StubApi::start(Some(STICKY_ID));
    let output = comment(&api, 3);

    let writes = api.writes();
    assert_eq!(writes.len(), 1, "a second comment was posted: {writes:#?}");
    assert_eq!(writes[0].method, "PATCH");
    assert_eq!(
        writes[0].path,
        format!("/repos/{REPO}/issues/comments/{STICKY_ID}")
    );
    assert_eq!(
        posted_body(&writes[0]),
        std::fs::read_to_string(golden_body(3)).unwrap()
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("updated comment 77"));
}

/// The whole point of the zero case: a pull request that adds no suppressions is
/// not worth a comment, and a bot that posts one anyway teaches reviewers to
/// ignore it.
#[test]
fn a_clean_pull_request_with_no_comment_yet_is_left_alone() {
    let api = StubApi::start(None);
    let output = comment(&api, 0);

    assert!(api.writes().is_empty(), "{:#?}", api.writes());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("posted nothing"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// A pull request that *had* suppressions and no longer does must say so, or the
/// stale list stays on the page as the reviewer's last word.
#[test]
fn a_pull_request_that_stopped_adding_suppressions_has_its_comment_cleared() {
    let api = StubApi::start(Some(STICKY_ID));
    comment(&api, 0);

    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:#?}");
    assert_eq!(writes[0].method, "PATCH");
    assert_eq!(
        posted_body(&writes[0]),
        std::fs::read_to_string(golden_body(0)).unwrap()
    );
}

/// Outside a pull request there is nothing to comment on, and guessing would
/// comment on the wrong thing.
#[test]
fn a_run_with_no_pull_request_posts_nothing_and_succeeds() {
    require_gh();
    let api = StubApi::start(None);
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &api.address)
        .env("BODY_FILE", golden_body(0))
        .env("COUNT", "0")
        .env("GH_TOKEN", "stub-token")
        .env_remove("PR_NUMBER")
        .env_remove("GITHUB_EVENT_PATH")
        .output()
        .expect("run comment.sh");

    assert!(output.status.success(), "{:?}", output.status);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("not a pull request"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(api.requests().is_empty(), "{:#?}", api.requests());
}

/// The pull request number is read from the event payload when the caller does
/// not pass one — that is how the composite finds it in a real run.
#[test]
fn the_pull_request_number_is_read_from_the_event_payload() {
    require_gh();
    let api = StubApi::start(None);
    let event = tempfile::NamedTempFile::new().expect("an event payload");
    std::fs::write(
        event.path(),
        format!(r#"{{"pull_request":{{"number":{PULL_REQUEST}}}}}"#),
    )
    .expect("write the event payload");

    let output = Command::new("bash")
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &api.address)
        .env("GITHUB_EVENT_PATH", event.path())
        .env("BODY_FILE", golden_body(1))
        .env("COUNT", "1")
        .env("GH_TOKEN", "stub-token")
        .env_remove("PR_NUMBER")
        .output()
        .expect("run comment.sh");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:#?}");
    assert_eq!(
        writes[0].path,
        format!("/repos/{REPO}/issues/{PULL_REQUEST}/comments")
    );
}

/// A body that never got written is a broken run, not an empty comment.
#[test]
fn a_missing_body_file_fails_with_the_fix() {
    require_gh();
    let api = StubApi::start(None);
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &api.address)
        .env("PR_NUMBER", PULL_REQUEST)
        .env("BODY_FILE", repo_root().join("target/does-not-exist.md"))
        .env("COUNT", "1")
        .env("GH_TOKEN", "stub-token")
        .output()
        .expect("run comment.sh");

    assert!(!output.status.success(), "a missing body was accepted");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist"), "{stderr}");
    assert!(stderr.contains("ACTION:"), "{stderr}");
    assert!(api.requests().is_empty(), "{:#?}", api.requests());
}
