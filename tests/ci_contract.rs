//! Keeps CI's `llmlint` job and `oneharness.toml`'s harness chain in step.
//!
//! `oneharness.toml` decides which harnesses llmlint drives and in what order,
//! but the runner is what puts their binaries on PATH and hands them their
//! credentials — and oneharness only falls through to a harness it can actually
//! spawn and authenticate. So a chain entry the job never installs, or installs
//! without its credential, is a fallback in name only: the chain has nowhere to
//! degrade to the moment the primary breaks. That is exactly how a floating
//! `npm install -g @openai/codex` took the required check down for every pull
//! request when upstream shipped a bad release.
//!
//! Neither failure mode is visible until a pull request goes red, which is one
//! round trip too late, so these read `ci.yml` as text — as
//! `install_contract.rs` reads `release.yml` — and fail the build instead.
//!
//! The rest of this file holds the `required` job — the one context branch
//! protection requires — to the workflow it summarises, structurally through
//! [`workflow_yaml`]: every other job is in its `needs:`, it keeps the shape a
//! required context needs, and the rule table in `scripts/ci-required.sh`
//! records exactly the `if:` each covered job carries, so the verdict's copy of
//! the workflow's conditions cannot drift from them.

#[path = "support/workflow_yaml.rs"]
mod workflow_yaml;

use workflow_yaml::{parse, read, repo_root, run_steps, Node};

/// Every harness `oneharness.toml` may name, the npm package that puts its
/// binary on PATH, and the credential it authenticates with. Adding a harness to
/// the chain without adding it here fails
/// [`the_harness_table_covers_the_whole_chain`] with the name that is missing.
const HARNESSES: &[Harness] = &[
    Harness {
        name: "codex",
        package: "@openai/codex",
        credential: "OPENAI_API_KEY",
    },
    Harness {
        name: "claude-code",
        package: "@anthropic-ai/claude-code",
        credential: "CLAUDE_CODE_OAUTH_TOKEN",
    },
];

struct Harness {
    name: &'static str,
    package: &'static str,
    credential: &'static str,
}

/// The harnesses `oneharness.toml` names, in priority order.
fn oneharness_chain() -> Vec<String> {
    let config = read("oneharness.toml");
    let list = config
        .lines()
        .find_map(|line| line.trim().strip_prefix("harnesses = "))
        .expect("oneharness.toml declares a `harnesses` chain");
    list.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|entry| entry.trim().trim_matches('"').to_string())
        .filter(|entry| !entry.is_empty())
        .collect()
}

/// The body of one top-level job, from its header to the next thing at job
/// indentation. Job bodies are indented four spaces or more.
fn job(workflow: &str, name: &str) -> String {
    let header = format!("  {name}:");
    let mut lines = workflow.lines();
    lines
        .find(|line| *line == header)
        .unwrap_or_else(|| panic!("ci.yml declares no `{name}` job"));
    lines
        .take_while(|line| line.trim().is_empty() || line.starts_with("    "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `npm install -g <spec>` in `text`, as the package spec it installs.
/// Reaches the command whether it is a step's own `run:` or a line inside a
/// block scalar — a spec that only one of those forms found would be a pin these
/// checks silently stopped covering.
fn npm_global_installs(text: &str) -> Vec<&str> {
    text.lines()
        .map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("- ").unwrap_or(line);
            line.strip_prefix("run: ").unwrap_or(line)
        })
        .filter_map(|command| command.strip_prefix("npm install -g "))
        .map(str::trim)
        .collect()
}

/// Whether a spec carries an explicit version. The leading `@` of a scoped name
/// is not the version separator, so skip it before looking for one.
fn is_version_pinned(spec: &str) -> bool {
    spec.strip_prefix('@')
        .unwrap_or(spec)
        .split_once('@')
        .is_some_and(|(_, version)| !version.is_empty())
}

fn llmlint_job() -> String {
    job(&read(".github/workflows/ci.yml"), "llmlint")
}

#[test]
fn the_harness_table_covers_the_whole_chain() {
    for name in oneharness_chain() {
        assert!(
            HARNESSES.iter().any(|harness| harness.name == name),
            "oneharness.toml names the `{name}` harness, but tests/ci_contract.rs \
             knows no npm package or credential for it — add it to HARNESSES so \
             the checks below can prove CI installs and authenticates it"
        );
    }
}

#[test]
fn ci_installs_every_harness_the_chain_names() {
    let job = llmlint_job();
    for name in oneharness_chain() {
        let Some(harness) = HARNESSES.iter().find(|harness| harness.name == name) else {
            continue; // Reported by `the_harness_table_covers_the_whole_chain`.
        };
        assert!(
            npm_global_installs(&job)
                .iter()
                .any(|spec| spec.starts_with(&format!("{}@", harness.package))),
            "ci.yml's llmlint job never installs `{}`, so oneharness cannot fall \
             through to the `{name}` harness it names — the chain would report \
             `{name} (skipped: not found on PATH)` and fail the required check",
            harness.package
        );
    }
}

/// Scoped to the `llmlint` job on purpose: what this contract owns is the judge
/// binaries whose upstream releases can take a *required* check down. Another
/// job installing a test runner globally answers to its own tradeoffs, and
/// failing it here would be this change legislating beyond its scope.
#[test]
fn every_harness_install_is_version_pinned() {
    let job = llmlint_job();
    let installs = npm_global_installs(&job);
    // Without this the check passes on an empty list — which is what it did
    // while the extractor was reading past the steps' `run:` keys.
    assert!(
        !installs.is_empty(),
        "ci.yml's llmlint job has no `npm install -g` at all; either the harness \
         installs were removed or this check stopped finding them"
    );
    for spec in installs {
        assert!(
            is_version_pinned(spec),
            "ci.yml's llmlint job installs `{spec}` unpinned; any upstream release \
             can then take the required llmlint check — and with it the merge \
             path — down"
        );
    }
}

#[test]
fn every_harness_credential_is_passed_and_required() {
    let job = llmlint_job();
    for name in oneharness_chain() {
        let Some(harness) = HARNESSES.iter().find(|harness| harness.name == name) else {
            continue; // Reported by `the_harness_table_covers_the_whole_chain`.
        };
        let credential = harness.credential;
        assert!(
            job.contains(&format!("{credential}: ${{{{ secrets.{credential} }}}}")),
            "ci.yml's llmlint job never passes {credential} to the lint step, so \
             the `{name}` harness is installed but cannot authenticate"
        );
        assert!(
            job.contains(&format!(r#"[ -z "${{{credential}:-}}" ]"#)),
            "ci.yml's llmlint job runs without checking {credential} first; a \
             missing credential would surface as the `{name}` harness silently \
             falling through rather than as a named configuration error"
        );
    }
}

const CI: &str = ".github/workflows/ci.yml";
const VERDICT_SCRIPT: &str = "scripts/ci-required.sh";

const NO_CONDITION: &str = "(none)";

const REQUIRED: &str = "required";

const VERDICT_COMMAND: &str = "bash scripts/ci-required.sh";

const VERDICT_INPUTS: [(&str, &str); 2] = [
    ("REQUIRED_NEEDS", "${{ toJSON(needs) }}"),
    ("REQUIRED_EVENT", "${{ github.event_name }}"),
];

/// The names a job's `needs:` lists, whether written as one name or a sequence.
fn needs_of(job: &Node) -> Vec<String> {
    match job.find("needs") {
        None => Vec::new(),
        Some(Node::Scalar(name)) => vec![name.clone()],
        Some(needs) => needs
            .list()
            .iter()
            .map(|name| name.scalar().to_string())
            .collect(),
    }
}

fn verdict_rules(script: &str) -> Vec<(String, String)> {
    let mut lines = script.lines();
    lines
        .find(|line| line.trim_end().ends_with("<<'RULES'"))
        .unwrap_or_else(|| panic!("{VERDICT_SCRIPT} has no `<<'RULES'` rule table"));
    lines
        .take_while(|line| *line != "RULES")
        .map(|line| {
            let (job, condition) = line.split_once(" | ").unwrap_or_else(|| {
                panic!("{VERDICT_SCRIPT}'s rule `{line}` is not `<job> | <condition>`")
            });
            (job.to_string(), condition.to_string())
        })
        .collect()
}

/// Every job the workflow defines that `required`'s `needs:` leaves out.
fn uncovered_jobs(workflow: &Node) -> Vec<String> {
    let jobs = workflow.get("jobs");
    let covered = needs_of(jobs.get(REQUIRED));
    jobs.keys()
        .into_iter()
        .filter(|job| *job != REQUIRED && !covered.iter().any(|name| name == job))
        .map(|job| {
            format!(
                "ci.yml defines the `{job}` job, but `{REQUIRED}`'s needs: does not name it, \
                 so branch protection would merge with `{job}` red — add `{job}` to \
                 `{REQUIRED}`'s needs: and a row for it to {VERDICT_SCRIPT}'s rule table"
            )
        })
        .collect()
}

/// Every way `required` has stopped being a context protection can rely on.
fn required_shape_problems(workflow: &Node) -> Vec<String> {
    let job = workflow.get("jobs").get(REQUIRED);
    let mut problems = Vec::new();
    if job.find("strategy").is_some() {
        problems.push(format!(
            "`{REQUIRED}` has a strategy/matrix; its contexts would be per-variant, and a \
             skipped matrix emits none of them — remove it"
        ));
    }
    if let Some(name) = job.find("name") {
        problems.push(format!(
            "`{REQUIRED}` overrides its name with `{}`; protection requires the context \
             `{REQUIRED}`, so the override would leave it waiting forever — remove `name:`",
            name.scalar()
        ));
    }
    let runner = job.find("runs-on").map_or("", Node::scalar);
    if runner != "ubuntu-latest" {
        problems.push(format!(
            "`{REQUIRED}` runs on `{runner}` rather than `ubuntu-latest`"
        ));
    }
    let condition = job.find("if").map_or("", Node::scalar);
    if condition != "always()" {
        problems.push(format!(
            "`{REQUIRED}` has `if: {condition}` rather than `if: always()`; any other \
             condition skips it when a job it needs fails, and a skipped required \
             context passes"
        ));
    }
    let steps = job.get("steps").list();
    let verdict = run_steps(steps)
        .into_iter()
        .find(|step| step.get("run").scalar().contains(VERDICT_SCRIPT));
    let Some(verdict) = verdict else {
        problems.push(format!(
            "`{REQUIRED}` has no step running {VERDICT_SCRIPT}, so nothing decides it — \
             add a step whose run: is `{VERDICT_COMMAND}`"
        ));
        return problems;
    };
    let run = verdict.get("run").scalar();
    if run != VERDICT_COMMAND {
        problems.push(format!(
            "`{REQUIRED}`'s verdict step runs `{run}` rather than exactly \
             `{VERDICT_COMMAND}`, so its outcome is not that script's exit status alone"
        ));
    }
    for (variable, expression) in VERDICT_INPUTS {
        let given = verdict
            .find("env")
            .and_then(|env| env.find(variable))
            .map(Node::scalar);
        if given != Some(expression) {
            problems.push(format!(
                "`{REQUIRED}`'s verdict step sets {variable} to {given:?} rather than \
                 `{expression}`, which is what {VERDICT_SCRIPT} reads it as"
            ));
        }
    }
    let checked_out = steps
        .iter()
        .take_while(|step| !std::ptr::eq(*step, verdict))
        .any(|step| {
            step.find("uses")
                .is_some_and(|uses| uses.scalar().starts_with("actions/checkout@"))
        });
    if !checked_out {
        problems.push(format!(
            "`{REQUIRED}` runs {VERDICT_SCRIPT} without an actions/checkout step before \
             it, so the script is not there to run"
        ));
    }
    problems
}

/// Every disagreement between the jobs `required` covers, the jobs ci.yml
/// defines, and the conditions the verdict script's rule table records.
fn rule_drift(workflow: &Node, rules: &[(String, String)]) -> Vec<String> {
    let jobs = workflow.get("jobs");
    let mut problems = Vec::new();
    for job in needs_of(jobs.get(REQUIRED)) {
        if !rules.iter().any(|(name, _)| *name == job) {
            problems.push(format!(
                "`{REQUIRED}` needs `{job}`, but {VERDICT_SCRIPT}'s rule table has no row \
                 for it, so the verdict refuses every run — add `{job} | <its if:>`"
            ));
        }
    }
    for (job, recorded) in rules {
        let Some(defined) = jobs.find(job) else {
            problems.push(format!(
                "{VERDICT_SCRIPT}'s rule table covers `{job}`, which ci.yml does not \
                 define — remove the row"
            ));
            continue;
        };
        let actual = defined.find("if").map_or(NO_CONDITION, Node::scalar);
        if recorded != actual {
            problems.push(format!(
                "`{job}`'s condition drifted: ci.yml has `if: {actual}` but \
                 {VERDICT_SCRIPT} records `{recorded}` — update the row so the verdict \
                 judges its skips by the condition that actually decides them \
                 (a job with no `if:` is recorded as `{NO_CONDITION}`)"
            ));
        }
    }
    problems
}

fn verdict_events(script: &str) -> Vec<String> {
    let line = script
        .lines()
        .find_map(|line| line.strip_prefix("events="))
        .unwrap_or_else(|| panic!("{VERDICT_SCRIPT} declares no `events=` list"));
    let mut events: Vec<String> = line
        .trim_matches('"')
        .split_whitespace()
        .map(str::to_string)
        .collect();
    events.sort();
    events
}

fn workflow_events(workflow: &Node) -> Vec<String> {
    let mut events: Vec<String> = workflow
        .get("on")
        .keys()
        .into_iter()
        .map(str::to_string)
        .collect();
    events.sort();
    events
}

fn assert_none(problems: &[String]) {
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn required_needs_every_other_job() {
    assert_none(&uncovered_jobs(&parse(&read(CI))));
}

#[test]
fn required_keeps_the_shape_protection_relies_on() {
    assert_none(&required_shape_problems(&parse(&read(CI))));
}

#[test]
fn the_verdict_rules_match_the_workflows_conditions() {
    assert_none(&rule_drift(
        &parse(&read(CI)),
        &verdict_rules(&read(VERDICT_SCRIPT)),
    ));
}

/// The script refuses any event outside its list, because the conditions were
/// read against those alone; a trigger ci.yml gained would otherwise fail every
/// run it starts, and one it lost would be accepted unexamined.
#[test]
fn the_verdict_accepts_exactly_the_events_ci_runs_on() {
    let (script, workflow) = (
        verdict_events(&read(VERDICT_SCRIPT)),
        workflow_events(&parse(&read(CI))),
    );
    assert_eq!(
        script, workflow,
        "{VERDICT_SCRIPT}'s `events` is {script:?} but ci.yml runs on {workflow:?} — \
         read every rule's skip under the new event, then make the two agree"
    );
}

/// The verdict script is invoked as `bash <path>`, so it must be where the step
/// says; a rename that left the step behind would fail only on GitHub.
#[test]
fn the_verdict_script_exists() {
    assert!(
        repo_root().join(VERDICT_SCRIPT).is_file(),
        "{VERDICT_SCRIPT} is missing, but `{REQUIRED}` runs it"
    );
}

/// The checks above, shown catching what they exist for on edited copies of the
/// real files — a check that passes the real tree could also be one that
/// cannot fail.
mod the_checks_catch {
    use super::*;

    fn ci_with(from: &str, to: &str) -> Node {
        let text = read(CI);
        assert!(text.contains(from), "ci.yml no longer contains `{from}`");
        parse(&text.replacen(from, to, 1))
    }

    #[test]
    fn a_job_required_does_not_need() {
        let workflow = parse(&format!(
            "{}\n  audit:\n    runs-on: ubuntu-latest\n    steps:\n      - run: true\n",
            read(CI)
        ));
        let problems = uncovered_jobs(&workflow);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("`audit` job"), "{problems:?}");
    }

    #[test]
    fn each_broken_shape() {
        for (from, to, named) in [
            ("  required:\n", "  required:\n    name: Required\n", "overrides its name"),
            (
                "  required:\n",
                "  required:\n    strategy:\n      matrix:\n        os: [ubuntu-latest]\n",
                "strategy/matrix",
            ),
            ("    if: always()\n", "    if: success()\n", "if: success()"),
            (
                "    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      # Both",
                "    runs-on: macos-latest\n    steps:\n      - uses: actions/checkout@v4\n      # Both",
                "runs on `macos-latest`",
            ),
            ("REQUIRED_NEEDS: ${{ toJSON(needs) }}", "REQUIRED_NEEDS: ${{ needs }}", "REQUIRED_NEEDS"),
            ("REQUIRED_EVENT: ${{ github.event_name }}", "OTHER: x", "REQUIRED_EVENT"),
            ("run: bash scripts/ci-required.sh", "run: bash scripts/ci-required.sh || true", "exit status"),
            ("run: bash scripts/ci-required.sh", "run: echo ok", "no step running"),
        ] {
            let problems = required_shape_problems(&ci_with(from, to));
            assert!(
                problems.iter().any(|problem| problem.contains(named)),
                "`{from}` -> `{to}` went unreported: {problems:?}"
            );
        }
    }

    #[test]
    fn a_drifted_condition_a_missing_row_and_a_stale_row() {
        let mut rules = verdict_rules(&read(VERDICT_SCRIPT));
        let deny = rules
            .iter_mut()
            .find(|(job, _)| job == "deny")
            .expect("a deny row");
        deny.1 = "needs.changes.outputs.crate != 'false'".to_string();
        rules.retain(|(job, _)| job != "gate");
        rules.push(("retired".to_string(), NO_CONDITION.to_string()));
        let problems = rule_drift(&parse(&read(CI)), &rules).join("\n");
        for named in [
            "`deny`'s condition drifted: ci.yml has `if: needs.changes.outputs.crate == 'true'` \
             but scripts/ci-required.sh records `needs.changes.outputs.crate != 'false'`",
            "`required` needs `gate`, but",
            "covers `retired`, which ci.yml does not define",
        ] {
            assert!(
                problems.contains(named),
                "missing `{named}` in:\n{problems}"
            );
        }
    }
}
