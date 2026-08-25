//! The composite action's comment step, driven against a real local GitHub API.
//!
//! Nothing here is mocked. The real `scripts/action/comment.sh` runs under real
//! bash, calls the real `gh` CLI, and reads the real checked-in comment bodies;
//! `gh` speaks HTTP over a loopback socket to a real server this module runs,
//! which answers the two endpoints the script uses and records every request it
//! received. Only the *host* is local: github.com is the one boundary these
//! journeys cannot own, so it is served here instead of reached over the
//! network. What is proven is the upsert rule — edit the marked comment when one
//! exists, create it when none does, and post nothing at all when there is
//! nothing to say.
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

use crate::support::{commit, git_repo, repo_root, write};

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

/// A real HTTP server on loopback, answering the two GitHub endpoints the
/// script calls and logging what it was asked for.
struct LocalGitHub {
    address: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    running: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
}

impl LocalGitHub {
    /// Start a server whose comment list either holds the sticky comment or does
    /// not.
    fn start(sticky: Option<u64>) -> LocalGitHub {
        LocalGitHub::listen(sticky, true)
    }

    /// The same server, but refusing every write the way GitHub refuses a token
    /// without `pull-requests: write`.
    fn refusing_writes(sticky: Option<u64>) -> LocalGitHub {
        LocalGitHub::listen(sticky, false)
    }

    fn listen(sticky: Option<u64>, writable: bool) -> LocalGitHub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the local API");
        let address = format!(
            "http://{}",
            listener.local_addr().expect("the local API address")
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
                // A server that panics mid-journey would hang the client instead
                // of failing it, so a malformed exchange just closes the
                // connection.
                let _ = serve(&mut stream, sticky, writable, &log);
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
        LocalGitHub {
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

impl Drop for LocalGitHub {
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
    writable: bool,
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
        (method, path)
            if !writable && method != "GET" && (path == comments || path == sticky_comment) =>
        {
            (
                "403 Forbidden",
                r#"{"message":"Resource not accessible by integration"}"#.to_string(),
            )
        }
        ("POST", path) if path == comments => ("201 Created", r#"{"id":101}"#.to_string()),
        ("PATCH", path) if path == sticky_comment => ("200 OK", format!(r#"{{"id":{STICKY_ID}}}"#)),
        _ => (
            "404 Not Found",
            r#"{"message":"the action asked for an endpoint this server does not implement"}"#
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
fn comment(api: &LocalGitHub, count: usize) -> Output {
    require_gh();
    let body = golden_body(count);
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &api.address)
        .env("PR_NUMBER", PULL_REQUEST)
        .env("BODY_FILE", &body)
        .env("COUNT", count.to_string())
        .env("GH_TOKEN", "local-token")
        // Insulate gh from the developer's own configuration and hosts file:
        // the journey must talk to the local server and to nothing else.
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
    let api = LocalGitHub::start(None);
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
    let api = LocalGitHub::start(Some(STICKY_ID));
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
    let api = LocalGitHub::start(None);
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
    let api = LocalGitHub::start(Some(STICKY_ID));
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
    let api = LocalGitHub::start(None);
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &api.address)
        .env("BODY_FILE", golden_body(0))
        .env("COUNT", "0")
        .env("GH_TOKEN", "local-token")
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
    let api = LocalGitHub::start(None);
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
        .env("GH_TOKEN", "local-token")
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

/// An input the composite forgot to pass fails the same way a bad one does.
///
/// Each of these is a wiring mistake in the workflow that called the step, so
/// the message has to name the variable *and* where its value comes from —
/// bash's own "parameter null or not set" names only the half the reader
/// already has.
#[test]
fn an_unset_input_fails_with_the_cause_and_the_fix() {
    require_gh();
    for missing in ["GITHUB_REPOSITORY", "BODY_FILE", "COUNT"] {
        let api = LocalGitHub::start(None);
        let mut command = Command::new("bash");
        command
            .arg(repo_root().join("scripts/action/comment.sh"))
            .env("GITHUB_REPOSITORY", REPO)
            .env("GITHUB_API_URL", &api.address)
            .env("PR_NUMBER", PULL_REQUEST)
            .env("BODY_FILE", golden_body(1))
            .env("COUNT", "1")
            .env("GH_TOKEN", "local-token")
            .env_remove(missing);
        let output = command.output().expect("run comment.sh");

        assert!(
            !output.status.success(),
            "a run with no {missing} was accepted"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(missing), "{stderr}");
        assert!(stderr.contains("ACTION:"), "{stderr}");
        assert!(api.requests().is_empty(), "{:#?}", api.requests());
    }
}

/// Run the comment step with `overrides` applied to the working environment.
fn comment_with(api: &LocalGitHub, overrides: &[(&str, &str)]) -> Output {
    require_gh();
    let mut command = Command::new("bash");
    command
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &api.address)
        .env("PR_NUMBER", PULL_REQUEST)
        .env("BODY_FILE", golden_body(1))
        .env("COUNT", "1")
        .env("GH_TOKEN", "local-token")
        .env("GH_CONFIG_DIR", repo_root().join("target/gh-config"))
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .env_remove("GITHUB_EVENT_PATH");
    for (name, value) in overrides {
        command.env(name, value);
    }
    command.output().expect("run comment.sh")
}

/// Assert the step failed naming `cause`, with a next action, having changed
/// nothing.
fn assert_failed(output: &Output, api: &LocalGitHub, cause: &str) {
    assert!(
        !output.status.success(),
        "the step accepted it: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(cause), "{stderr}");
    assert!(stderr.contains("ACTION:"), "{stderr}");
    assert!(api.writes().is_empty(), "{:#?}", api.requests());
}

/// The API refusing the call is the failure this action actually has in the
/// field, and `gh` reports the status without saying which knob fixes it.
///
/// Both writes are covered: the token is refused whether the pull request has a
/// sticky comment to edit or needs its first one.
#[test]
fn an_api_that_refuses_the_write_names_the_permission_to_grant() {
    for (sticky, cause) in [
        (
            Some(STICKY_ID),
            format!("cannot update comment {STICKY_ID}"),
        ),
        (None, format!("cannot comment on #{PULL_REQUEST}")),
    ] {
        let api = LocalGitHub::refusing_writes(sticky);
        let output = comment_with(&api, &[]);
        assert!(!output.status.success(), "a refused write was accepted");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(&cause), "{stderr}");
        assert!(stderr.contains("pull-requests: write"), "{stderr}");
    }
}

/// A read the token cannot make is the same failure one step earlier.
#[test]
fn an_api_that_refuses_the_read_names_the_permission_to_grant() {
    let api = LocalGitHub::start(None);
    // A repository this server does not serve answers 404, which is what a token
    // that cannot see the pull request gets too.
    let output = comment_with(&api, &[("GITHUB_REPOSITORY", "acme/not-served")]);
    assert_failed(&output, &api, "cannot list the comments");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("pull-requests: write"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The two values that are interpolated into an API path are bounded to their
/// documented shapes, so a mis-wired workflow cannot aim the request elsewhere.
#[test]
fn an_input_that_would_redirect_the_request_is_refused() {
    let api = LocalGitHub::start(None);
    for (name, value, cause) in [
        (
            "GITHUB_REPOSITORY",
            "acme/widgets/../../evil",
            "not owner/repo",
        ),
        ("GITHUB_REPOSITORY", "widgets", "not owner/repo"),
        // A character outside the slug's alphabet would truncate or redirect
        // the path it is interpolated into.
        ("GITHUB_REPOSITORY", "acme/widgets?x=1", "not owner/repo"),
        (
            "GITHUB_API_URL",
            "file:///etc/passwd",
            "not an http(s) origin",
        ),
    ] {
        let output = comment_with(&api, &[(name, value)]);
        assert_failed(&output, &api, cause);
    }
}

/// The event payload is read as data, and data can be malformed.
#[test]
fn an_unreadable_event_payload_fails_with_the_fix() {
    let api = LocalGitHub::start(None);
    let event = tempfile::NamedTempFile::new().expect("an event payload");
    std::fs::write(event.path(), "{not json").expect("write the event payload");

    let output = comment_with(
        &api,
        &[
            ("PR_NUMBER", ""),
            ("GITHUB_EVENT_PATH", &event.path().to_string_lossy()),
        ],
    );
    assert_failed(&output, &api, "cannot read the pull request number");
}

/// A body that never got written is a broken run, not an empty comment.
#[test]
fn a_missing_body_file_fails_with_the_fix() {
    require_gh();
    let api = LocalGitHub::start(None);
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &api.address)
        .env("PR_NUMBER", PULL_REQUEST)
        .env("BODY_FILE", repo_root().join("target/does-not-exist.md"))
        .env("COUNT", "1")
        .env("GH_TOKEN", "local-token")
        .output()
        .expect("run comment.sh");

    assert!(!output.status.success(), "a missing body was accepted");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist"), "{stderr}");
    assert!(stderr.contains("ACTION:"), "{stderr}");
    assert!(api.requests().is_empty(), "{:#?}", api.requests());
}

/// The body a pull request that only rewrote justifications actually renders,
/// with the repository it was rendered from.
///
/// Built rather than checked in: the goldens are unclassified scans, and the
/// question here is what the script does with a body whose heading counts no
/// additions at all. The binary renders it from a real branch, so the body the
/// upsert carries is the body a reviewer would have received.
fn rejustified_body() -> (tempfile::TempDir, PathBuf) {
    let repo = git_repo();
    write(
        repo.path(),
        "src/app.py",
        "import os  # noqa: F401  # imported for its side effects\n",
    );
    commit(repo.path(), "base");
    write(
        repo.path(),
        "src/app.py",
        "import os  # noqa: F401  # re-exported so callers can configure retries\n",
    );
    commit(repo.path(), "reword the justification");

    let rendered = crate::support::notignored(repo.path())
        .args(["--diff", "--diff-base", "HEAD~1", "--format", "markdown"])
        .output()
        .expect("render the comment body");
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let body = String::from_utf8(rendered.stdout).expect("a UTF-8 comment body");
    assert!(
        body.contains("### notignored: 1 justification edited"),
        "the branch did not render a rejustified body:\n{body}"
    );
    let path = repo.path().join("comment.md");
    std::fs::write(&path, &body).expect("write the rendered body");
    (repo, path)
}

/// The pull request this whole distinction exists for: it added no suppression,
/// so the additions count is zero — and it still has something to tell the
/// reviewer, so it still gets a comment.
#[test]
fn a_pull_request_that_only_rewrote_a_justification_still_gets_its_comment() {
    let api = LocalGitHub::start(None);
    let (_repo, body) = rejustified_body();
    let output = comment_with(
        &api,
        &[
            ("BODY_FILE", &body.to_string_lossy()),
            ("COUNT", "0"),
            ("JUSTIFICATION_EDITED_COUNT", "1"),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:#?}");
    assert_eq!(writes[0].method, "POST");
    assert_eq!(
        posted_body(&writes[0]),
        std::fs::read_to_string(&body).expect("the rendered body"),
        "the comment body is not the rendered report"
    );
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(
        log.contains("0 suppression(s) added, 1 justification(s) edited"),
        "the log line names one number or the wrong words: {log}"
    );
}

/// The same pull request on its second push edits the comment the first one
/// left, exactly as an addition-only one does.
#[test]
fn a_second_push_that_only_rewrote_a_justification_edits_the_same_comment() {
    let api = LocalGitHub::start(Some(STICKY_ID));
    let (_repo, body) = rejustified_body();
    let output = comment_with(
        &api,
        &[
            ("BODY_FILE", &body.to_string_lossy()),
            ("COUNT", "0"),
            ("JUSTIFICATION_EDITED_COUNT", "1"),
        ],
    );
    assert!(output.status.success(), "{:?}", output.status);

    let writes = api.writes();
    assert_eq!(writes.len(), 1, "a second comment was posted: {writes:#?}");
    assert_eq!(writes[0].method, "PATCH");
    assert_eq!(
        writes[0].path,
        format!("/repos/{REPO}/issues/comments/{STICKY_ID}")
    );
}

/// A caller that never heard of the second count is a caller from before it
/// existed, not a broken one: unset means zero, and the decision is the one it
/// has always been.
#[test]
fn an_unset_second_count_behaves_as_it_did_before_there_was_one() {
    let clean = LocalGitHub::start(None);
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &clean.address)
        .env("PR_NUMBER", PULL_REQUEST)
        .env("BODY_FILE", golden_body(0))
        .env("COUNT", "0")
        .env("GH_TOKEN", "local-token")
        .env_remove("JUSTIFICATION_EDITED_COUNT")
        .env_remove("GITHUB_EVENT_PATH")
        .output()
        .expect("run comment.sh");
    assert!(output.status.success(), "{:?}", output.status);
    assert!(clean.writes().is_empty(), "{:#?}", clean.writes());

    // And an addition with no second count still posts.
    let adding = LocalGitHub::start(None);
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/action/comment.sh"))
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &adding.address)
        .env("PR_NUMBER", PULL_REQUEST)
        .env("BODY_FILE", golden_body(1))
        .env("COUNT", "1")
        .env("GH_TOKEN", "local-token")
        .env_remove("JUSTIFICATION_EDITED_COUNT")
        .env_remove("GITHUB_EVENT_PATH")
        .output()
        .expect("run comment.sh");
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(adding.writes().len(), 1, "{:#?}", adding.writes());
    assert_eq!(adding.writes()[0].method, "POST");
}

/// Set, it is held to the same shape as the count beside it: a mis-wired
/// workflow must fail rather than post a comment under the wrong rule.
#[test]
fn a_second_count_that_is_not_a_number_is_refused_when_it_is_set() {
    let api = LocalGitHub::start(None);
    let output = comment_with(&api, &[("JUSTIFICATION_EDITED_COUNT", "several")]);
    assert_failed(&output, &api, "JUSTIFICATION_EDITED_COUNT is not a count");
}
