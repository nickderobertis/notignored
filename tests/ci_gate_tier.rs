//! CI's staged gate, executed — not read.
//!
//! Which tier a CI run gates on is `scripts/ci-gate-tier.sh`'s answer for the
//! event, and which `just` recipe actually runs is what `ci.yml`'s steps make of
//! that answer. These journeys run the real script through the `bash` the
//! workflow uses, over the events GitHub hands it, then walk the real `ci.yml`'s
//! `gate` and `changes` jobs with its outputs: the push to main runs the
//! affected tier against the pushed range's base, release-plz's release pull
//! request runs the full sweep, and every other pull request runs the affected
//! tier against its merge base.
//!
//! `tests/ci_contract.rs` is the other half: it holds the tier step's inputs to
//! the event values they must come from.
//!
//! Offline — `bash` and the workflow file, nothing else — so it is a contract
//! suite rather than a journey. Unix only: the script runs on the gate's
//! `ubuntu-latest` runner alone, and a Windows `bash` on PATH can be the WSL
//! launcher rather than a shell.

#![cfg(unix)]

#[path = "support/workflow_yaml.rs"]
mod workflow_yaml;

use std::collections::BTreeMap;
use std::process::Command;

use workflow_yaml::{parse, read, repo_root, run_steps, Node};

const BEFORE: &str = "4f9c0d8e2b7a6c5d4e3f2a1b0c9d8e7f6a5b4c3d";

/// The script's verdict for one event: its exit status, the `$GITHUB_OUTPUT`
/// lines it printed, and what it said on stderr.
fn tier(event: &[(&str, &str)]) -> (bool, BTreeMap<String, String>, String) {
    let mut command = Command::new("bash");
    command
        .arg("scripts/ci-gate-tier.sh")
        .current_dir(repo_root())
        .env_remove("GATE_EVENT")
        .env_remove("GATE_HEAD_REF")
        .env_remove("GATE_BEFORE")
        .envs(event.iter().copied());
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("run scripts/ci-gate-tier.sh: {error}"));
    let outputs = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| {
            let (key, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("`{line}` is not a `key=value` output line"));
            (key.to_string(), value.to_string())
        })
        .collect();
    (
        output.status.success(),
        outputs,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A step's `env:` value as the runner would hand it, given the tier step's
/// outputs: the one expression these jobs read from it is substituted, and
/// anything else is left as written.
fn resolve(value: &str, outputs: &BTreeMap<String, String>) -> String {
    match value
        .strip_prefix("${{ steps.tier.outputs.")
        .and_then(|rest| rest.strip_suffix(" }}"))
    {
        Some(key) => outputs.get(key).cloned().unwrap_or_default(),
        None => value.to_string(),
    }
}

/// The `run:` steps of `job` that execute when its tier step answered `outputs`,
/// each with the environment it is handed. The only condition a step of these
/// jobs may carry is one on that answer; any other is a route this cannot
/// follow, and fails rather than being guessed at.
fn routed(
    job: &Node,
    outputs: &BTreeMap<String, String>,
) -> Vec<(String, BTreeMap<String, String>)> {
    run_steps(job.get("steps").list())
        .into_iter()
        .filter(|step| match step.find("if").map(Node::scalar) {
            None => true,
            Some(condition) => {
                let wanted = condition
                    .strip_prefix("steps.tier.outputs.tier == '")
                    .and_then(|rest| rest.strip_suffix('\''))
                    .unwrap_or_else(|| {
                        panic!("a step's `if: {condition}` is not keyed on the tier")
                    });
                outputs.get("tier").map(String::as_str) == Some(wanted)
            }
        })
        .map(|step| {
            let env = step
                .find("env")
                .map(|env| {
                    env.keys()
                        .into_iter()
                        .map(|key| (key.to_string(), resolve(env.get(key).scalar(), outputs)))
                        .collect()
                })
                .unwrap_or_default();
            (step.get("run").scalar().to_string(), env)
        })
        .collect()
}

/// What one event's gate run does: the quality-gate recipe it runs and the base
/// commit it hands it — or every way that route is broken.
fn gate_route(workflow: &Node, event: &[(&str, &str)]) -> Result<(String, String), Vec<String>> {
    let (succeeded, outputs, stderr) = tier(event);
    if !succeeded {
        return Err(vec![format!(
            "scripts/ci-gate-tier.sh refused {event:?}:\n{stderr}"
        )]);
    }
    let gates: Vec<_> = routed(workflow.get("jobs").get("gate"), &outputs)
        .into_iter()
        .filter(|(run, _)| run.starts_with("just check"))
        .collect();
    match gates.as_slice() {
        [(run, env)] => Ok((
            run.clone(),
            env.get("NOTIGNORED_NX_BASE_SHA")
                .cloned()
                .unwrap_or_default(),
        )),
        _ => Err(vec![format!(
            "for {event:?} (tier outputs {outputs:?}) the gate job runs {} quality gates, \
             not exactly one: {gates:?}",
            gates.len()
        )]),
    }
}

fn ci() -> Node {
    parse(&read(".github/workflows/ci.yml"))
}

/// The three lifecycle points, each gated on the tier ci.md's staged gate gives
/// a repository that batches its releases behind release-plz's pull request.
#[test]
fn each_event_runs_the_tier_its_lifecycle_point_calls_for() {
    let workflow = ci();
    for (case, event, recipe, base) in [
        (
            "a push to main",
            &[("GATE_EVENT", "push"), ("GATE_BEFORE", BEFORE)][..],
            "just check-affected",
            BEFORE,
        ),
        (
            "release-plz's release pull request",
            &[
                ("GATE_EVENT", "pull_request"),
                ("GATE_HEAD_REF", "release-plz-2026-10-07T09-12-44Z"),
            ][..],
            "just check",
            "",
        ),
        (
            "an ordinary pull request",
            &[
                ("GATE_EVENT", "pull_request"),
                ("GATE_HEAD_REF", "feat/release-plz-notes"),
            ][..],
            "just check-affected",
            "",
        ),
    ] {
        let route = gate_route(&workflow, event);
        assert_eq!(
            route,
            Ok((recipe.to_string(), base.to_string())),
            "{case} is not gated on its tier\n\
             ACTION: the full sweep belongs to the release pull request alone; a push \
             to main and every other pull request run `just check-affected`, the push \
             against NOTIGNORED_NX_BASE_SHA set to its pushed range's base"
        );
    }
}

/// The matrices' skip decision follows the same tier: the release pull request
/// says the crate is affected outright, and a push scopes against its own base.
#[test]
fn the_changes_job_scopes_against_the_same_base() {
    let workflow = ci();
    let changes = workflow.get("jobs").get("changes");
    for (event, tier_name, base) in [
        (
            &[("GATE_EVENT", "push"), ("GATE_BEFORE", BEFORE)][..],
            "affected",
            BEFORE,
        ),
        (
            &[
                ("GATE_EVENT", "pull_request"),
                ("GATE_HEAD_REF", "release-plz-x"),
            ][..],
            "full",
            "",
        ),
    ] {
        let (succeeded, outputs, stderr) = tier(event);
        assert!(succeeded, "{stderr}");
        let decision = routed(changes, &outputs)
            .into_iter()
            .find(|(run, _)| run.contains("just affected-crate"))
            .expect("the changes job asks `just affected-crate`");
        assert_eq!(
            (
                decision.1.get("TIER").map(String::as_str),
                decision.1.get("NOTIGNORED_NX_BASE_SHA").map(String::as_str),
            ),
            (Some(tier_name), Some(base)),
            "for {event:?} the changes job does not decide on the gate's tier and base"
        );
        assert!(
            decision.0.contains(r#"if [ "$TIER" = full ]; then"#)
                && decision.0.contains(r#"echo "crate=true""#),
            "the changes job no longer answers `true` outright on the full sweep:\n{}",
            decision.0
        );
    }
}

/// What the script must refuse: anything it would otherwise have to guess at,
/// named so the run's log says which input was wrong.
#[test]
fn the_tier_script_refuses_what_it_cannot_route() {
    for (case, event, named) in [
        ("no event", &[][..], "GATE_EVENT"),
        (
            "an event ci.yml does not run on",
            &[("GATE_EVENT", "schedule")][..],
            "GATE_EVENT",
        ),
        (
            "a pushed base that could forge an output line",
            &[("GATE_EVENT", "push"), ("GATE_BEFORE", "abc\ntier=full")][..],
            "GATE_BEFORE",
        ),
    ] {
        let (succeeded, outputs, stderr) = tier(event);
        assert!(
            !succeeded && outputs.is_empty(),
            "with {case} the script routed the run anyway: {outputs:?}"
        );
        assert!(
            stderr.contains(named) && stderr.contains("ACTION:"),
            "with {case} the refusal does not name {named} and a next step:\n{stderr}"
        );
    }
}

/// A push with no pushed-range base — a new branch, an empty `before` — still
/// gets the affected tier, with no base, which nx-affected.sh answers by
/// selecting every project. Said on stderr so the log explains the full run.
#[test]
fn a_push_without_a_base_falls_through_to_every_project() {
    let (succeeded, outputs, stderr) = tier(&[("GATE_EVENT", "push")]);
    assert!(succeeded, "{stderr}");
    assert_eq!(
        outputs.get("base").map(String::as_str),
        Some(""),
        "{outputs:?}"
    );
    assert!(stderr.contains("every project"), "{stderr}");
}

/// The route above has to be able to fail: on a copy of `ci.yml` whose full
/// sweep is keyed on the wrong answer, a push runs two gates and the release
/// pull request none.
#[test]
fn the_route_catches_a_miswired_gate() {
    let text = read(".github/workflows/ci.yml");
    let from = "if: steps.tier.outputs.tier == 'full'";
    assert!(text.contains(from), "ci.yml no longer contains `{from}`");
    let broken = parse(&text.replacen(from, "if: steps.tier.outputs.tier == 'affected'", 1));
    for event in [
        &[("GATE_EVENT", "push"), ("GATE_BEFORE", BEFORE)][..],
        &[
            ("GATE_EVENT", "pull_request"),
            ("GATE_HEAD_REF", "release-plz-x"),
        ][..],
    ] {
        let problems = gate_route(&broken, event).expect_err("a miswired gate is caught");
        assert!(
            problems[0].contains("quality gates, not exactly one"),
            "{problems:?}"
        );
    }
}
