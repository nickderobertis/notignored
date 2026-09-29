//! CI's `required` verdict, executed — not read.
//!
//! Branch protection requires the one context `required`, and what that job
//! reports is `scripts/ci-required.sh`'s exit status over `toJSON(needs)` and
//! the event name. These journeys run that real script through the `bash` the
//! workflow's step uses, over payloads in the shape GitHub hands it — each job an
//! object with a `result` and an `outputs` map — and assert its verdict: that a
//! job skipped for the reason its own `if:` gives passes, and that every other
//! skip, failure, or malformed input fails naming what it refused.
//!
//! `tests/ci_contract.rs` is the other half: it holds the job's wiring and the
//! script's rule table to `ci.yml`, so what is proven here is what runs there.
//!
//! Runs on every platform: the script needs only `bash` and `node`, which the
//! `cross` legs carry for the rest of this suite.

use std::process::{Command, Output};

use serde_json::{json, Map, Value};

use crate::support::{bash_program, repo_root};

/// Every job `required` covers, as `ci.yml` defines them today.
const JOBS: [&str; 9] = [
    "changes",
    "gate",
    "cross",
    "msrv",
    "deny",
    "install",
    "install-documented",
    "pr-title",
    "llmlint",
];

/// The jobs whose `if:` is `needs.changes.outputs.crate == 'true'`.
const CRATE_JOBS: [&str; 4] = ["cross", "msrv", "deny", "install"];

/// A `toJSON(needs)` payload in which every job succeeded, with `changes`
/// reporting `crate` as given.
fn all_succeeded(crate_output: &str) -> Map<String, Value> {
    JOBS.iter()
        .map(|job| {
            let outputs = if *job == "changes" {
                json!({ "crate": crate_output })
            } else {
                json!({})
            };
            (
                (*job).to_string(),
                json!({ "result": "success", "outputs": outputs }),
            )
        })
        .collect()
}

/// `payload` with each named job's result replaced.
fn with_results(mut payload: Map<String, Value>, results: &[(&str, &str)]) -> Map<String, Value> {
    for (job, result) in results {
        payload
            .entry((*job).to_string())
            .or_insert_with(|| json!({ "outputs": {} }))["result"] = json!(result);
    }
    payload
}

/// The script's verdict over a raw payload and event, either of which may be
/// left unset.
fn verdict_raw(needs: Option<&str>, event: Option<&str>) -> Output {
    let mut command = Command::new(bash_program());
    command
        .arg(repo_root().join("scripts").join("ci-required.sh"))
        .current_dir(repo_root())
        .env_remove("REQUIRED_NEEDS")
        .env_remove("REQUIRED_EVENT");
    if let Some(needs) = needs {
        command.env("REQUIRED_NEEDS", needs);
    }
    if let Some(event) = event {
        command.env("REQUIRED_EVENT", event);
    }
    command
        .output()
        .unwrap_or_else(|error| panic!("run scripts/ci-required.sh: {error}"))
}

fn verdict(payload: &Map<String, Value>, event: &str) -> Output {
    verdict_raw(
        Some(&Value::Object(payload.clone()).to_string()),
        Some(event),
    )
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[track_caller]
fn assert_accepts(output: &Output) {
    assert!(
        output.status.success(),
        "expected the verdict to pass; it exited {:?}:\n{}",
        output.status.code(),
        stderr(output)
    );
}

/// The verdict failed, and its stderr names each of `named` on its own
/// `ci-required: <job>: ...` line — the form a reader scans for which check to
/// open.
#[track_caller]
fn assert_rejects_naming(output: &Output, named: &[&str]) {
    let stderr = stderr(output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected the verdict to fail; stderr:\n{stderr}"
    );
    for job in named {
        assert!(
            stderr.contains(&format!("ci-required: {job}: ")),
            "the refusal does not name `{job}`:\n{stderr}"
        );
    }
    assert!(stderr.contains("ACTION: "), "no next step in:\n{stderr}");
}

#[test]
fn a_pull_request_reaching_the_crate_passes_with_only_the_push_job_skipped() {
    let payload = with_results(all_succeeded("true"), &[("install-documented", "skipped")]);
    assert_accepts(&verdict(&payload, "pull_request"));
}

/// notignored#63: an SDK-only pull request, whose matrices legitimately skip.
#[test]
fn a_pull_request_that_misses_the_crate_passes_with_the_crate_jobs_skipped() {
    let payload = with_results(
        all_succeeded("false"),
        &[
            ("cross", "skipped"),
            ("msrv", "skipped"),
            ("deny", "skipped"),
            ("install", "skipped"),
            ("install-documented", "skipped"),
        ],
    );
    assert_accepts(&verdict(&payload, "pull_request"));
}

#[test]
fn a_push_passes_with_the_pull_request_jobs_skipped() {
    let payload = with_results(
        all_succeeded("true"),
        &[("pr-title", "skipped"), ("llmlint", "skipped")],
    );
    assert_accepts(&verdict(&payload, "push"));
}

#[test]
fn a_job_with_no_condition_never_skips_legitimately() {
    for job in ["gate", "changes"] {
        let payload = with_results(
            all_succeeded("true"),
            &[(job, "skipped"), ("install-documented", "skipped")],
        );
        let output = verdict(&payload, "pull_request");
        assert_rejects_naming(&output, &[job]);
        assert!(stderr(&output).contains(&format!("ci-required: {job}: result skipped")));
    }
}

#[test]
fn the_pull_request_jobs_may_not_skip_on_a_pull_request() {
    for job in ["llmlint", "pr-title"] {
        let payload = with_results(
            all_succeeded("true"),
            &[(job, "skipped"), ("install-documented", "skipped")],
        );
        assert_rejects_naming(&verdict(&payload, "pull_request"), &[job]);
    }
}

#[test]
fn the_documented_install_may_not_skip_on_a_push() {
    let payload = with_results(all_succeeded("true"), &[("install-documented", "skipped")]);
    assert_rejects_naming(&verdict(&payload, "push"), &["install-documented"]);
}

#[test]
fn a_crate_job_may_not_skip_when_the_crate_is_affected() {
    for job in CRATE_JOBS {
        let payload = with_results(all_succeeded("true"), &[(job, "skipped")]);
        let output = verdict(&payload, "push");
        assert_rejects_naming(&output, &[job]);
        for other in CRATE_JOBS.iter().filter(|other| **other != job) {
            assert!(
                !stderr(&output).contains(&format!("ci-required: {other}: ")),
                "`{other}` succeeded but was named:\n{}",
                stderr(&output)
            );
        }
    }
}

/// `changes` fails closed to "affected", so an output it did not set cannot be
/// read as "the crate was not reached".
#[test]
fn the_crate_jobs_may_not_skip_when_the_crate_output_is_missing_or_empty() {
    let skipped: Vec<(&str, &str)> = CRATE_JOBS.iter().map(|job| (*job, "skipped")).collect();
    let mut missing = with_results(all_succeeded("false"), &skipped);
    missing["changes"]["outputs"] = json!({});
    let empty = with_results(all_succeeded(""), &skipped);
    for payload in [missing, empty] {
        assert_rejects_naming(&verdict(&payload, "push"), &CRATE_JOBS);
    }
}

#[test]
fn a_failed_or_cancelled_job_fails_the_verdict() {
    for (job, result) in [
        ("deny", "failure"),
        ("gate", "cancelled"),
        ("msrv", "neutral"),
    ] {
        let payload = with_results(all_succeeded("true"), &[(job, result)]);
        let output = verdict(&payload, "push");
        assert_rejects_naming(&output, &[job]);
        assert!(
            stderr(&output).contains(&format!("ci-required: {job}: result {result}")),
            "the refusal does not give `{job}`'s result:\n{}",
            stderr(&output)
        );
    }
}

#[test]
fn a_job_the_rules_do_not_know_fails_the_verdict() {
    let payload = with_results(all_succeeded("true"), &[("audit", "success")]);
    let output = verdict(&payload, "push");
    assert_rejects_naming(&output, &["audit"]);
    assert!(stderr(&output).contains("no rule"), "{}", stderr(&output));
}

#[test]
fn a_job_the_rules_require_missing_from_the_payload_fails_the_verdict() {
    let mut payload = all_succeeded("true");
    payload.remove("llmlint");
    let output = verdict(&payload, "push");
    assert_rejects_naming(&output, &["llmlint"]);
    assert!(
        stderr(&output).contains("absent from the needs payload"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_job_without_a_result_fails_the_verdict() {
    let mut payload = all_succeeded("true");
    payload["gate"] = json!({ "outputs": {} });
    let output = verdict(&payload, "push");
    assert_rejects_naming(&output, &["gate"]);
    assert!(
        stderr(&output).contains("has no result"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_empty_or_malformed_payload_fails_the_verdict() {
    let full = Value::Object(all_succeeded("true")).to_string();
    let truncated = &full[..full.len() / 2];
    for needs in [None, Some(""), Some(truncated), Some("[]"), Some("null")] {
        let output = verdict_raw(needs, Some("push"));
        let stderr = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{needs:?} passed:\n{stderr}");
        assert!(
            stderr.contains("ci-required: ") && stderr.contains("ACTION: "),
            "{needs:?} was refused without saying why:\n{stderr}"
        );
    }
}

#[test]
fn a_missing_event_fails_the_verdict() {
    let needs = Value::Object(all_succeeded("true")).to_string();
    for event in [None, Some("")] {
        let output = verdict_raw(Some(&needs), event);
        let stderr = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{event:?} passed:\n{stderr}");
        assert!(stderr.contains("REQUIRED_EVENT"), "{stderr}");
    }
}
