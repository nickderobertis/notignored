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
        vec![
            "diff-base",
            "github-token",
            "max-entries",
            "paths",
            "version"
        ],
        "the action's inputs changed"
    );
    assert_eq!(
        inputs.get("github-token").get("default").scalar(),
        "${{ github.token }}"
    );
    assert_eq!(inputs.get("version").get("default").scalar(), "latest");
    // The renderer's own default, so the action and the binary cannot disagree
    // about how long a comment gets before it stops listing.
    assert_eq!(
        inputs.get("max-entries").get("default").scalar(),
        notignored::cli::DEFAULT_MAX_ENTRIES.to_string(),
        "the action's max-entries default drifted from the renderer's"
    );
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

/// Every array expansion, and whether it survives the oldest bash the action
/// runs under.
///
/// A macOS runner's `/bin/bash` is 3.2, where `set -u` rejects `"${a[@]}"` as an
/// unbound variable whenever the array is empty — the state an unset `paths`
/// input leaves the scan step in. Bash 4.4 and later expand it to nothing, so
/// the fault is invisible on Linux and Windows. The `${a[@]+"${a[@]}"}`
/// alternate form means the same thing to every bash, and is what this insists
/// on; the inner half of that form is itself an expansion, so it is recognized
/// rather than reported.
fn unguarded_array_expansions(script: &str) -> Vec<String> {
    let mut unguarded = Vec::new();
    for subscript in ["[@]", "[*]"] {
        for (at, _) in script.match_indices(subscript) {
            let head = &script[..at];
            let Some(open) = head.rfind("${") else {
                continue;
            };
            let name = &head[open + 2..];
            let guard = format!("${{{name}{subscript}+\"");
            let outer = format!("+\"${{{name}{subscript}}}\"}}");
            if head[..open].ends_with(&guard) || script[at + subscript.len()..].starts_with(&outer)
            {
                continue;
            }
            unguarded.push(format!("${{{name}{subscript}}}"));
        }
    }
    unguarded
}

/// The action's own scripts, as the shell receives them.
fn action_scripts() -> Vec<(String, String)> {
    let mut scripts: Vec<(String, String)> = action()
        .get("runs")
        .get("steps")
        .to_vec()
        .iter()
        .filter_map(|step| {
            let name = step.find("name").map_or("a step", Node::scalar).to_string();
            step.find("run").map(|run| (name, run.scalar().to_string()))
        })
        .collect();
    scripts.push((
        "scripts/action/comment.sh".to_string(),
        read("scripts/action/comment.sh"),
    ));
    scripts
}

#[test]
fn no_array_expansion_breaks_on_the_bash_a_macos_runner_ships() {
    for (name, script) in action_scripts() {
        assert_eq!(
            unguarded_array_expansions(&script),
            Vec::<String>::new(),
            "`{name}` expands an array in a form bash 3.2 rejects under `set -u` \
             when it is empty; write it as ${{name[@]+\"${{name[@]}}\"}}"
        );
    }
}

/// The recognizer above, on the two forms it has to tell apart.
#[test]
fn the_guarded_array_form_is_the_only_one_that_passes() {
    assert_eq!(
        unguarded_array_expansions(r#"cmd ${paths[@]+"${paths[@]}"} > out"#),
        Vec::<String>::new()
    );
    assert_eq!(
        unguarded_array_expansions(r#"cmd "${paths[@]}" "${other[*]}""#),
        vec!["${paths[@]}", "${other[*]}"]
    );
    // A guard that names a different array is not a guard.
    assert_eq!(
        unguarded_array_expansions(r#"cmd ${paths[@]+"${other[@]}"}"#),
        vec!["${paths[@]}", "${other[@]}"]
    );
    // `$@` is special-cased by every bash, and a subscript that is not an
    // expansion at all is left alone.
    assert_eq!(
        unguarded_array_expansions(r#"cmd "$@" # see foo[@]"#),
        Vec::<String>::new()
    );
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

/// The dogfood run builds the binary from source on every pull request, which is
/// minutes of release build unless it is cached.
///
/// Both halves are load-bearing and neither is visible in a green run: `cargo
/// install` builds in a throwaway target directory unless `CARGO_TARGET_DIR`
/// points elsewhere, so without it the cache action has nothing of the build to
/// save and every run pays full price. Dropping either one only shows up as a
/// slow job, which is exactly the kind of regression nobody files.
#[test]
fn the_dogfood_workflow_caches_the_build_it_does_on_every_pull_request() {
    let job = parse(&read(".github/workflows/notignored.yml"))
        .get("jobs")
        .get("suppressions")
        .clone();
    assert_eq!(
        job.get("env").get("CARGO_TARGET_DIR").scalar(),
        "${{ github.workspace }}/target",
        "`cargo install` must build into the cached workspace target directory"
    );
    assert!(
        job.get("steps").to_vec().iter().any(|step| step
            .find("uses")
            .is_some_and(|uses| uses.scalar().starts_with("Swatinem/rust-cache"))),
        "the dogfood workflow no longer caches the cargo build"
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

/// The `nickderobertis/notignored@<ref>` spellings the README hands a consumer,
/// paired with whether each one is a `uses:` line rather than prose.
fn readme_action_refs() -> Vec<(String, bool)> {
    let readme = read("README.md");
    let mut refs = Vec::new();
    for line in readme.lines() {
        let is_uses = line
            .trim_start()
            .trim_start_matches("- ")
            .starts_with("uses:");
        for (at, _) in line.match_indices("nickderobertis/notignored@") {
            let rest = &line[at + "nickderobertis/notignored@".len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && !"._-".contains(c))
                .unwrap_or(rest.len());
            refs.push((rest[..end].to_string(), is_uses));
        }
    }
    assert!(
        !refs.is_empty(),
        "the README no longer shows how to consume the action"
    );
    refs
}

/// What a consumer copies out of the README is a floating major tag.
///
/// It used to be `@main`, which ships every merged commit — released or not — to
/// every consumer on their next run. A major tag follows patches and minors,
/// never a breaking change, and (because `release.yml`'s `major-tag` job is what
/// moves it) never a release whose artifacts failed to publish. Nothing else is
/// acceptable in an example, because an example is what gets pasted.
#[test]
fn the_readme_tells_consumers_to_use_the_floating_major_tag() {
    let refs = readme_action_refs();
    let floating: Vec<&str> = refs
        .iter()
        .map(|(reference, _)| reference.as_str())
        .filter(|reference| is_floating_major(reference))
        .collect();
    let major = *floating
        .first()
        .expect("the README shows the action pinned to a floating major tag such as @v0");

    for (reference, is_uses) in &refs {
        assert!(
            is_floating_major(reference) || is_exact_version(reference),
            "the README consumes the action at `@{reference}`; a branch or an \
             arbitrary ref ships unreleased work to everyone who copies it"
        );
        if *is_uses {
            assert_eq!(
                reference, major,
                "a `uses:` example pins `@{reference}` rather than the floating major \
                 tag `@{major}`; an exact version in a copied example goes stale"
            );
        }
    }

    // The reproducible alternative, offered in prose beside the examples: a
    // version tag never moves. Its major has to be the same line the floating
    // tag follows, or the two paragraphs describe different releases.
    let exact = refs
        .iter()
        .map(|(reference, _)| reference.as_str())
        .find(|reference| is_exact_version(reference))
        .expect("the README offers an exact pin as the reproducible alternative");
    assert_eq!(
        exact.split('.').next(),
        Some(major),
        "the exact-pin example `@{exact}` is not in the `@{major}` line the floating \
         tag follows"
    );
}

/// The floating tag the README promises is maintained by the release, not by
/// hand.
///
/// A README that says `@v0` while nothing moves `v0` is worse than `@main`: it
/// pins consumers to whatever release happened to be current when someone last
/// remembered. The logic itself is proven by `tests/e2e/major_tag.rs`, and *when*
/// it runs by `tests/packaging_contract.rs`; this is the edge between them and
/// the documentation.
#[test]
fn the_release_pipeline_maintains_the_tag_the_readme_documents() {
    let job = parse(&read(".github/workflows/release.yml"))
        .get("jobs")
        .get("major-tag")
        .clone();
    assert!(
        run_steps(job.get("steps").list())
            .iter()
            .any(|step| step.get("run").scalar().contains("update-major-tag.sh")),
        "nothing in the release moves the floating major tag the README documents"
    );
}

fn is_floating_major(reference: &str) -> bool {
    reference
        .strip_prefix('v')
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
}

fn is_exact_version(reference: &str) -> bool {
    let Some(rest) = reference.strip_prefix('v') else {
        return false;
    };
    let parts: Vec<&str> = rest.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}
