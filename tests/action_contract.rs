//! Structural gate for the composite action and the workflow that dogfoods it.
//!
//! `action.yml` is only ever exercised for real inside GitHub Actions, where a
//! mistake costs a red pull request and a round trip. These tests read it the way
//! the runner does — as structure, not as text — so a renamed input, a step that
//! forgot its shell, or an untrusted event value spliced into a script fails the
//! build instead of a run.
//!
//! The YAML reader lives in `tests/support/workflow_yaml.rs`; the release
//! pipeline's own structural gate ([`packaging_contract`](../packaging_contract.rs))
//! reads `release.yml` through the same parser.

#[path = "support/workflow_yaml.rs"]
mod workflow_yaml;

use workflow_yaml::{parse, read, repo_root, run_steps, Node};

fn action() -> Node {
    parse(&read("action.yml"))
}

#[test]
fn the_action_declares_the_documented_inputs_and_defaults() {
    let action = action();
    let inputs = action.get("inputs");
    assert_eq!(
        inputs.keys(),
        vec!["diff-base", "github-token", "paths", "version"],
        "the action's inputs changed"
    );
    assert_eq!(
        inputs.get("github-token").get("default").scalar(),
        "${{ github.token }}"
    );
    assert_eq!(inputs.get("version").get("default").scalar(), "latest");
    // Both optional: an empty base means "the pull request's base branch" and
    // empty paths mean "the whole repository".
    for input in ["diff-base", "paths"] {
        assert_eq!(inputs.get(input).get("default").scalar(), "", "{input}");
    }
}

#[test]
fn the_action_exposes_the_count_and_the_report_it_produced() {
    let outputs = action().get("outputs").clone();
    assert_eq!(outputs.keys(), vec!["count", "report-path"]);
    assert_eq!(
        outputs.get("count").get("value").scalar(),
        "${{ steps.scan.outputs.count }}"
    );
    assert_eq!(
        outputs.get("report-path").get("value").scalar(),
        "${{ steps.scan.outputs.report-path }}"
    );
}

#[test]
fn every_step_of_the_composite_declares_the_shell_it_runs_in() {
    let action = action();
    assert_eq!(action.get("runs").get("using").scalar(), "composite");
    let steps = action.get("runs").get("steps").list();
    assert!(!steps.is_empty(), "the composite runs no steps");
    for step in run_steps(steps) {
        assert_eq!(
            step.get("shell").scalar(),
            "bash",
            "a composite `run:` step without `shell: bash` never starts"
        );
    }
}

/// The two ways the binary can arrive, and the one guarantee each owes: `local`
/// builds the source that ships with the action (which is what makes a pull
/// request here test its own code), and every other value goes through the
/// checked-in installer that verifies what it downloads.
#[test]
fn the_install_step_builds_from_source_or_installs_a_verified_release() {
    let action = action();
    let install = action
        .get("runs")
        .get("steps")
        .list()
        .iter()
        .find(|step| step.find("id").is_some_and(|id| id.scalar() == "install"))
        .expect("the composite has an install step")
        .clone();
    let script = install.get("run").scalar().to_string();

    assert!(
        script.contains(r#"cargo install --path "$GITHUB_ACTION_PATH""#),
        "`version: local` must build the action's own checkout: {script}"
    );
    assert!(
        script.contains(r#""$GITHUB_ACTION_PATH/scripts/install.sh""#),
        "a released version must arrive through the checked-in installer: {script}"
    );
    assert!(
        repo_root().join("scripts/install.sh").exists(),
        "the installer the action calls is missing"
    );
    assert_eq!(
        install.get("env").get("VERSION").scalar(),
        "${{ inputs.version }}",
        "the version input must reach the script through the environment"
    );
}

/// The security rule this action lives under: a fork's branch name, title, or
/// body is attacker-controlled text, and `${{ }}` splices it into the script
/// *before* the shell sees it. Everything reaches a step through `env` instead.
#[test]
fn no_untrusted_event_value_is_interpolated_into_a_script() {
    for file in ["action.yml", ".github/workflows/notignored.yml"] {
        let text = read(file);
        let parsed = parse(&text);
        let steps: Vec<Node> = match file {
            "action.yml" => parsed.get("runs").get("steps").list().to_vec(),
            _ => parsed
                .get("jobs")
                .get("suppressions")
                .get("steps")
                .list()
                .to_vec(),
        };
        for step in run_steps(&steps) {
            let script = step.get("run").scalar();
            assert!(
                !script.contains("github.event"),
                "{file} interpolates an event value into a script: {script}"
            );
            assert!(
                !script.contains("inputs."),
                "{file} interpolates an input into a script; pass it through `env`: {script}"
            );
        }
    }
}

/// The action finds its own comment by the marker the renderer writes; drift
/// between the two turns every run into a fresh comment.
#[test]
fn the_comment_script_looks_for_the_marker_the_renderer_writes() {
    let script = read("scripts/action/comment.sh");
    let recorded = script
        .lines()
        .find_map(|line| line.trim().strip_prefix("# STICKY_MARKER:"))
        .map(str::trim)
        .expect("comment.sh records a STICKY_MARKER marker");
    assert_eq!(
        recorded,
        notignored::cli::MARKER,
        "comment.sh and the markdown renderer disagree about the sticky marker"
    );
    assert!(
        script.contains(&format!("MARKER='{recorded}'")),
        "comment.sh does not search for the marker it records"
    );
}

#[test]
fn the_dogfood_workflow_runs_this_repositorys_own_action_on_its_own_pull_requests() {
    let workflow = parse(&read(".github/workflows/notignored.yml"));
    assert!(
        workflow.get("on").find("pull_request").is_some(),
        "the dogfood workflow no longer runs on pull requests"
    );
    // Upserting a comment is a write; reading the diff is not.
    let permissions = workflow.get("permissions");
    assert_eq!(permissions.get("contents").scalar(), "read");
    assert_eq!(permissions.get("pull-requests").scalar(), "write");

    let steps = workflow
        .get("jobs")
        .get("suppressions")
        .get("steps")
        .to_vec();
    let checkout = steps
        .iter()
        .find(|step| {
            step.find("uses")
                .is_some_and(|uses| uses.scalar().starts_with("actions/checkout"))
        })
        .expect("the workflow checks the repository out");
    assert_eq!(
        checkout.get("with").get("fetch-depth").scalar(),
        "0",
        "--diff needs the base branch, which a shallow checkout does not fetch"
    );

    let action = steps
        .iter()
        .find(|step| step.find("uses").is_some_and(|uses| uses.scalar() == "./"))
        .expect("the workflow uses the action from this checkout");
    assert_eq!(
        action.get("with").get("version").scalar(),
        "local",
        "dogfooding must build the branch's own source, not a published release"
    );
}

/// A helper for the assertions above: sequences come back borrowed.
trait ToVec {
    fn to_vec(&self) -> Vec<Node>;
}

impl ToVec for Node {
    fn to_vec(&self) -> Vec<Node> {
        self.list().to_vec()
    }
}
