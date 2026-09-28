//! `scripts/github-release.sh`: the release pipeline's steps on the GitHub
//! Release itself, driven against a real local GitHub API.
//!
//! Under release immutability a published Release accepts no asset and its tag
//! can no longer move, so `release.yml` attaches everything to a *draft* and only
//! then publishes it, and `major-tag` moves `v0` only after the published Release
//! reads back `"immutable": true`. Those three decisions live in this script, and
//! a mistake in any of them is invisible until a real release: publishing twice,
//! attaching to a Release that can no longer take assets, or moving `v0` onto a
//! Release whose tag could still be moved.
//!
//! Nothing is mocked, as in [`crate::action_comment`]: the real script runs under
//! real bash and calls the real `gh`, which speaks HTTP to a server this module
//! runs on loopback. The server answers the three endpoints the script uses the
//! way GitHub does — drafts only in the list, `releases/tags/…` only for a
//! published Release, `immutable` set at publication from the repository's
//! setting — and records every request.
//!
//! POSIX-only: the release jobs run it under bash on ubuntu-latest.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::support::{cargo_version, repo_root};

const REPO: &str = "acme/widgets";
const RELEASE_ID: u64 = 4242;

/// The Release for this build's tag, as the server holds it.
#[derive(Clone, Copy)]
struct Stored {
    draft: bool,
    immutable: bool,
}

/// The one endpoint the server answers with a 500, if any.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Broken {
    Nothing,
    Listing,
    Publishing,
    ReadBack,
    /// Not a 500: the listing answers, with a `draft` that is not a boolean.
    GarbledDraft,
}

/// What the server knows.
struct State {
    broken: Broken,
    /// The Release for [`tag`], if release-plz has cut one.
    release: Option<Stored>,
    /// The repository's release-immutability setting, applied on publication.
    immutability: bool,
    /// How many Release listings still omit the Release — release-plz pushes
    /// the tag a moment before it cuts the draft.
    unlisted_for: usize,
    /// How many by-tag reads still report a published Release mutable — the
    /// read-back racing GitHub settling the Release it just published.
    settling_for: usize,
    /// Every `METHOD path body` the script sent.
    requests: Vec<(String, String, String)>,
}

struct LocalGitHub {
    address: String,
    state: Arc<Mutex<State>>,
    running: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
}

impl LocalGitHub {
    fn start(release: Option<Stored>, immutability: bool) -> Self {
        Self::lagging(release, immutability, 0, 0)
    }

    /// A server that answers the first `unlisted_for` listings without the
    /// Release and the first `settling_for` read-backs as mutable.
    fn lagging(
        release: Option<Stored>,
        immutability: bool,
        unlisted_for: usize,
        settling_for: usize,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the local API");
        let address = format!(
            "http://{}",
            listener.local_addr().expect("the local API address")
        );
        let state = Arc::new(Mutex::new(State {
            broken: Broken::Nothing,
            release,
            immutability,
            unlisted_for,
            settling_for,
            requests: Vec::new(),
        }));
        let running = Arc::new(AtomicBool::new(true));
        let (shared, alive) = (Arc::clone(&state), Arc::clone(&running));
        let server = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if !alive.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { break };
                let _ = serve(&mut stream, &shared);
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
        LocalGitHub {
            address,
            state,
            running,
            server: Some(server),
        }
    }

    /// The same server, answering `endpoint` with a 500 every time.
    fn breaking(release: Option<Stored>, endpoint: Broken) -> Self {
        let api = Self::start(release, true);
        api.state.lock().expect("the state").broken = endpoint;
        api
    }

    fn release(&self) -> Option<Stored> {
        self.state.lock().expect("the state").release
    }

    fn writes(&self) -> Vec<(String, String, String)> {
        self.state
            .lock()
            .expect("the state")
            .requests
            .iter()
            .filter(|(method, _, _)| method != "GET")
            .cloned()
            .collect()
    }
}

impl Drop for LocalGitHub {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address.trim_start_matches("http://"));
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

fn tag() -> String {
    format!("v{}", cargo_version())
}

fn release_json(stored: Stored) -> String {
    serde_json::json!({
        "id": RELEASE_ID,
        "tag_name": tag(),
        "draft": stored.draft,
        // GitHub reports `immutable: false` on a draft; it is set on publication.
        "immutable": stored.immutable,
    })
    .to_string()
}

fn serve(stream: &mut TcpStream, state: &Mutex<State>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut fields = request_line.split_whitespace();
    let (Some(method), Some(path)) = (fields.next(), fields.next()) else {
        return Ok(());
    };
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

    let mut state = state.lock().expect("the state");
    state
        .requests
        .push((method.clone(), path.clone(), body.clone()));
    let list = format!("/repos/{REPO}/releases");
    let one = format!("/repos/{REPO}/releases/{RELEASE_ID}");
    let by_tag = format!("/repos/{REPO}/releases/tags/{}", tag());
    let not_found = || ("404 Not Found", r#"{"message":"Not Found"}"#.to_string());
    let broken = match state.broken {
        Broken::Nothing => false,
        Broken::Listing => method == "GET" && path == list,
        Broken::Publishing => method == "PATCH",
        Broken::ReadBack => method == "GET" && path == by_tag,
        Broken::GarbledDraft => false,
    };
    if state.broken == Broken::GarbledDraft && method == "GET" && path == list {
        let payload = format!(
            r#"[{{"id":{RELEASE_ID},"tag_name":"{}","draft":"yes"}}]"#,
            tag()
        );
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
            payload.len()
        )?;
        return stream.flush();
    }
    let (status, payload) = match (method.as_str(), path.as_str()) {
        _ if broken => (
            "500 Internal Server Error",
            r#"{"message":"Server Error"}"#.to_string(),
        ),
        ("GET", p) if p == list && state.unlisted_for > 0 => {
            state.unlisted_for -= 1;
            (
                "200 OK",
                r#"[{"id":1,"tag_name":"v0.0.1","draft":false,"immutable":false}]"#.to_string(),
            )
        }
        ("GET", p) if p == list => (
            "200 OK",
            match state.release {
                // Another Release beside it, so finding it is the tag's doing.
                Some(stored) => format!(
                    r#"[{},{{"id":1,"tag_name":"v0.0.1","draft":false,"immutable":false}}]"#,
                    release_json(stored)
                ),
                None => {
                    r#"[{"id":1,"tag_name":"v0.0.1","draft":false,"immutable":false}]"#.to_string()
                }
            },
        ),
        // Like GitHub, the by-tag endpoint does not see a draft.
        ("GET", p) if p == by_tag => match state.release {
            Some(stored) if !stored.draft && state.settling_for > 0 => {
                state.settling_for -= 1;
                (
                    "200 OK",
                    release_json(Stored {
                        draft: false,
                        immutable: false,
                    }),
                )
            }
            Some(stored) if !stored.draft => ("200 OK", release_json(stored)),
            _ => not_found(),
        },
        ("PATCH", p) if p == one && state.release.is_some() => {
            let request: serde_json::Value =
                serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            if request["draft"] == serde_json::Value::Bool(false) {
                let immutable = state.immutability;
                state.release = Some(Stored {
                    draft: false,
                    immutable,
                });
            }
            ("200 OK", release_json(state.release.expect("a release")))
        }
        _ => not_found(),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
        payload.len()
    )?;
    stream.flush()
}

/// Run `github-release.sh <args>` against `api`.
fn github_release(api: &LocalGitHub, args: &[&str]) -> Output {
    let found = Command::new("gh").arg("--version").output();
    assert!(
        found.is_ok_and(|output| output.status.success()),
        "the GitHub CLI is not installed\nACTION: install gh (https://cli.github.com) — \
         release.yml calls it, and every GitHub-hosted runner ships it"
    );
    Command::new("bash")
        .arg(repo_root().join("scripts/github-release.sh"))
        .args(args)
        .env("GITHUB_REPOSITORY", REPO)
        .env("GITHUB_API_URL", &api.address)
        .env("GH_TOKEN", "local-token")
        .env("GH_CONFIG_DIR", repo_root().join("target/gh-config"))
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_HOST")
        .env_remove("GH_ENTERPRISE_TOKEN")
        .output()
        .expect("run github-release.sh")
}

fn text(output: &Output) -> String {
    format!(
        "--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const DRAFT: Stored = Stored {
    draft: true,
    immutable: false,
};

/// The whole order the release takes: the uploads find a draft, the draft is
/// published once, and the read-back passes because the setting is on.
#[test]
fn a_draft_is_found_then_published_and_reads_back_immutable() {
    let api = LocalGitHub::start(Some(DRAFT), true);
    let tag = tag();

    let awaited = github_release(&api, &["await-draft", "--tag", &tag, "--wait", "0"]);
    assert!(awaited.status.success(), "{}", text(&awaited));
    assert!(
        api.writes().is_empty(),
        "waiting for the draft wrote something"
    );

    let published = github_release(&api, &["publish", "--tag", &tag]);
    assert!(published.status.success(), "{}", text(&published));
    assert_eq!(
        String::from_utf8_lossy(&published.stdout).trim(),
        format!("github-release: published {tag}")
    );
    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:#?}");
    let (method, path, body) = &writes[0];
    assert_eq!(method, "PATCH");
    assert_eq!(path, &format!("/repos/{REPO}/releases/{RELEASE_ID}"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body).expect("a JSON body"),
        serde_json::json!({ "draft": false }),
        "publishing must change the draft flag and nothing else"
    );
    assert!(api.release().is_some_and(|release| !release.draft));

    let verified = github_release(&api, &["verify-immutable", "--tag", &tag, "--wait", "0"]);
    assert!(verified.status.success(), "{}", text(&verified));
    assert!(String::from_utf8_lossy(&verified.stdout).contains("published and immutable"));
}

/// A published Release that reads back mutable fails the run, and says the
/// setting is off — which is what keeps `major-tag` from moving `v0` onto it.
#[test]
fn a_release_that_reads_back_mutable_fails_saying_the_setting_is_off() {
    let api = LocalGitHub::start(Some(DRAFT), false);
    let tag = tag();
    let published = github_release(&api, &["publish", "--tag", &tag]);
    assert!(published.status.success(), "{}", text(&published));

    let verified = github_release(&api, &["verify-immutable", "--tag", &tag, "--wait", "0"]);
    assert!(!verified.status.success(), "{}", text(&verified));
    let stderr = String::from_utf8_lossy(&verified.stderr);
    assert!(
        stderr.contains("::error::")
            && stderr.contains("\"immutable\": false")
            && stderr.contains("release immutability is off")
            && stderr.contains("Enable release immutability"),
        "the failure does not say the setting is off:\n{}",
        text(&verified)
    );
}

/// A Release that is still a draft does not pass the read-back either — the
/// by-tag endpoint cannot see it, and a draft's tag is not locked — and the
/// failure says to publish it rather than blaming the setting.
#[test]
fn a_draft_does_not_pass_the_read_back() {
    let api = LocalGitHub::start(Some(DRAFT), true);
    let verified = github_release(&api, &["verify-immutable", "--tag", &tag(), "--wait", "0"]);
    assert!(!verified.status.success(), "{}", text(&verified));
    let stderr = String::from_utf8_lossy(&verified.stderr);
    assert!(
        stderr.contains("still a draft") && !stderr.contains("immutability is off"),
        "{}",
        text(&verified)
    );
}

/// A draft cut a moment after the tag was pushed is waited for, not refused.
#[test]
fn await_draft_waits_for_a_draft_cut_after_the_tag() {
    let api = LocalGitHub::lagging(Some(DRAFT), true, 1, 0);
    let output = github_release(
        &api,
        &[
            "await-draft",
            "--tag",
            &tag(),
            "--wait",
            "30",
            "--interval",
            "1",
        ],
    );
    assert!(output.status.success(), "{}", text(&output));
    let lists = api
        .state
        .lock()
        .expect("the state")
        .requests
        .iter()
        .filter(|(_, path, _)| path == &format!("/repos/{REPO}/releases"))
        .count();
    assert_eq!(lists, 2, "the draft was not looked for a second time");
}

/// A read-back that first sees the Release mutable keeps asking inside its
/// budget, and passes once GitHub reports it immutable.
#[test]
fn the_read_back_waits_for_a_release_that_is_still_settling() {
    let api = LocalGitHub::lagging(
        Some(Stored {
            draft: false,
            immutable: true,
        }),
        true,
        0,
        1,
    );
    let output = github_release(
        &api,
        &[
            "verify-immutable",
            "--tag",
            &tag(),
            "--wait",
            "30",
            "--interval",
            "1",
        ],
    );
    assert!(output.status.success(), "{}", text(&output));
}

/// Re-running the publish job over a Release it already published writes
/// nothing: the second PATCH would be a no-op at best.
#[test]
fn an_already_published_release_is_not_published_again() {
    let api = LocalGitHub::start(
        Some(Stored {
            draft: false,
            immutable: true,
        }),
        true,
    );
    let output = github_release(&api, &["publish", "--tag", &tag()]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(api.writes().is_empty(), "{:#?}", api.writes());
    assert!(String::from_utf8_lossy(&output.stdout).contains("already published"));
}

/// The check each upload leg runs before attaching refuses a Release that is
/// already published — under
/// immutability the attach would be refused anyway, after the build — and one
/// that never appears.
#[test]
fn await_draft_accepts_only_a_draft() {
    let published = LocalGitHub::start(
        Some(Stored {
            draft: false,
            immutable: true,
        }),
        true,
    );
    let output = github_release(&published, &["await-draft", "--tag", &tag(), "--wait", "0"]);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("already published"),
        "{}",
        text(&output)
    );

    let missing = LocalGitHub::start(None, true);
    let output = github_release(&missing, &["await-draft", "--tag", &tag(), "--wait", "0"]);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(&format!("no Release for {}", tag())),
        "{}",
        text(&output)
    );

    let output = github_release(&missing, &["publish", "--tag", &tag()]);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(missing.writes().is_empty());
}

/// A tag that is not a release tag is refused before any request: it becomes
/// part of an API path and a jq filter.
#[test]
fn a_tag_that_is_not_a_version_is_refused_before_the_api_is_asked() {
    let api = LocalGitHub::start(Some(DRAFT), true);
    let output = github_release(&api, &["publish", "--tag", "v1.2.3\" or true"]);
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(api.state.lock().expect("the state").requests.is_empty());
}

/// Each API call that fails fails the step with its cause and what to do —
/// never a silent pass, and never the wrong diagnosis.
#[test]
fn an_api_that_errors_fails_each_step_with_its_cause() {
    let tag = tag();

    let api = LocalGitHub::breaking(Some(DRAFT), Broken::Listing);
    for command in ["await-draft", "publish"] {
        let output = github_release(&api, &[command, "--tag", &tag, "--wait", "0"]);
        assert!(!output.status.success(), "{command}: {}", text(&output));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!("::error::cannot list the Releases of {REPO}"))
                && stderr.contains("ACTION: give the job contents: write"),
            "{command}: {}",
            text(&output)
        );
    }
    assert!(api.writes().is_empty());

    let api = LocalGitHub::breaking(Some(DRAFT), Broken::Publishing);
    let output = github_release(&api, &["publish", "--tag", &tag]);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(&format!(
            "::error::cannot publish the draft Release for {tag}"
        )),
        "{}",
        text(&output)
    );
    assert!(api.release().is_some_and(|release| release.draft));

    let api = LocalGitHub::breaking(
        Some(Stored {
            draft: false,
            immutable: true,
        }),
        Broken::ReadBack,
    );
    let output = github_release(
        &api,
        &[
            "verify-immutable",
            "--tag",
            &tag,
            "--wait",
            "2",
            "--interval",
            "1",
        ],
    );
    assert!(!output.status.success(), "{}", text(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("HTTP 500")
            && stderr.contains(&format!("no published Release for {tag} reads back"))
            && !stderr.contains("immutability is off"),
        "{}",
        text(&output)
    );
}

/// An API origin that could redirect the request is refused before any call.
#[test]
fn an_api_url_with_a_query_or_userinfo_is_refused() {
    let api = LocalGitHub::start(Some(DRAFT), true);
    for url in [
        format!("{}?x=1", api.address),
        "http://user@127.0.0.1:1".to_string(),
        "ftp://127.0.0.1".to_string(),
        "http:///repos".to_string(),
    ] {
        let output = Command::new("bash")
            .arg(repo_root().join("scripts/github-release.sh"))
            .args(["publish", "--tag", &tag()])
            .env("GITHUB_REPOSITORY", REPO)
            .env("GITHUB_API_URL", &url)
            .env("GH_TOKEN", "local-token")
            .output()
            .expect("run github-release.sh");
        assert!(!output.status.success(), "{url}: {}", text(&output));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("not an http(s) API origin"),
            "{url}: {}",
            text(&output)
        );
    }
    assert!(api.state.lock().expect("the state").requests.is_empty());
}

/// A listing whose `draft` is neither `true` nor `false` is not taken as
/// "already published": publishing nothing and passing would leave a draft.
#[test]
fn a_draft_value_that_is_not_a_boolean_is_refused() {
    let api = LocalGitHub::breaking(Some(DRAFT), Broken::GarbledDraft);
    for command in ["publish", "await-draft"] {
        let output = github_release(&api, &[command, "--tag", &tag(), "--wait", "0"]);
        assert!(!output.status.success(), "{command}: {}", text(&output));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("not the GitHub API"),
            "{command}: {}",
            text(&output)
        );
    }
    assert!(api.writes().is_empty());
}
