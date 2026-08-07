//! The project graph, resolved by the real Nx — not read out of `nx.json`.
//!
//! Two things break silently here. A project that falls out of the graph — a
//! `project.json` that stopped parsing, a name typo — makes `nx run-many` quietly
//! cover less than the repo, and every gate still reports green. And affected
//! selection is what CI skips the cross-platform and install matrices on
//! (`just affected-crate`), so a file that stops mapping to the Rust project
//! turns a skipped matrix into an unproven artifact.
//!
//! Both are answers only Nx can give, so these journeys ask it: `scripts/nx.sh`,
//! the real workspace, read-only commands. Nothing is stubbed.

use std::process::Command;

use crate::support::{bash_program, repo_root};

/// Every project in the graph, and the uniform targets each one owes.
///
/// The names are the *repo's deliverables*: the CLI crate and the two SDKs. A
/// fourth project added without a line here is not a failure — a project that
/// disappeared is.
const PROJECTS: [&str; 3] = ["notignored", "notignored-sdk-python", "notignored-sdk-npm"];

/// `run-many`/`affected` fan out by target *name*, so one root command only
/// covers the whole repo while these mean the same thing in every project.
const UNIFORM_TARGETS: [&str; 6] = [
    "bootstrap",
    "format",
    "format-check",
    "lint",
    "test",
    "check",
];

/// Nx's stdout for a read-only command, or a panic naming what it printed.
///
/// Read-only on purpose: these run inside `just check`, which is itself an Nx
/// invocation, and a nested command that wrote cache entries would race the run
/// that spawned it.
fn nx(args: &[&str]) -> String {
    let output = Command::new(bash_program())
        // Named relative to the working directory below, not as an absolute
        // path: on Windows an absolute one carries a drive letter and
        // backslashes, and the script resolves its own root through `dirname`,
        // which reads neither.
        .arg("scripts/nx.sh")
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|error| {
            panic!("run scripts/nx.sh {args:?}: {error}\nACTION: run `just bootstrap`")
        });
    assert!(
        output.status.success(),
        "`nx {}` failed:\n{}\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A JSON array of project names on Nx's stdout, sorted so order cannot matter.
fn project_list(args: &[&str]) -> Vec<String> {
    let stdout = nx(args);
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('['))
        .unwrap_or_else(|| panic!("`nx {}` printed no JSON array:\n{stdout}", args.join(" ")));
    let mut names: Vec<String> = serde_json::from_str(line.trim())
        .unwrap_or_else(|error| panic!("{line:?} is not a JSON array of names: {error}"));
    names.sort();
    names
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut sorted: Vec<String> = names.iter().map(|name| (*name).to_string()).collect();
    sorted.sort();
    sorted
}

#[test]
fn the_graph_holds_one_project_per_deliverable() {
    assert_eq!(
        project_list(&["show", "projects", "--json"]),
        sorted(&PROJECTS),
        "the project graph is not the repo's deliverables\n\
         ACTION: a project.json that stopped parsing drops out of the graph \
         silently — run `just nx show projects` and restore the missing one"
    );
}

#[test]
fn every_project_declares_the_uniform_targets() {
    for project in PROJECTS {
        let config: serde_json::Value =
            serde_json::from_str(nx(&["show", "project", project, "--json"]).trim())
                .unwrap_or_else(|error| panic!("`nx show project {project}` is not JSON: {error}"));
        let targets = config["targets"]
            .as_object()
            .unwrap_or_else(|| panic!("{project} declares no targets"));
        for target in UNIFORM_TARGETS {
            assert!(
                targets.contains_key(target),
                "{project} has no `{target}` target, so `nx run-many -t {target}` \
                 silently skips it\n\
                 ACTION: add it to that project's project.json"
            );
        }
    }
}

/// The crate's gate keeps its fourth tier. `doc` is not uniform — only the crate
/// publishes rustdoc — so `check` is the one place it can be aggregated, and a
/// `check` that lost it would drop docs from every gate at once.
#[test]
fn the_crates_check_aggregates_its_docs_tier() {
    let config: serde_json::Value =
        serde_json::from_str(nx(&["show", "project", "notignored", "--json"]).trim())
            .expect("`nx show project notignored` is JSON");
    let depends_on = config["targets"]["check"]["dependsOn"]
        .as_array()
        .expect("the crate's `check` declares dependsOn");
    for tier in ["format-check", "lint", "test", "doc"] {
        assert!(
            depends_on.iter().any(|entry| entry == tier),
            "the crate's `check` no longer depends on `{tier}`, so `just check` \
             stopped running it\nACTION: restore it in project.json"
        );
    }
}

/// What CI skips a matrix on. Each deliverable's tree must map to its own
/// project and nothing else, or an SDK-only pull request skips the Rust matrices
/// while having changed the Rust artifact.
#[test]
fn affected_selection_maps_each_tree_to_its_own_project() {
    let cases: [(&str, &[&str]); 5] = [
        ("src/lib.rs", &["notignored"]),
        ("npm/notignored/package.json", &["notignored"]),
        (
            "python/notignored-sdk/README.md",
            &["notignored-sdk-python"],
        ),
        ("npm/notignored-sdk/README.md", &["notignored-sdk-npm"]),
        // The orchestrator's own config changes what every target *is*, so it
        // has to reach all three — the one case where scoping would be wrong.
        ("nx.json", &PROJECTS),
    ];
    for (file, expected) in cases {
        assert_eq!(
            project_list(&[
                "show",
                "projects",
                "--affected",
                &format!("--files={file}"),
                "--json",
            ]),
            sorted(expected),
            "changing {file} no longer selects the projects it belongs to\n\
             ACTION: CI skips the cross/install matrices on `just affected-crate`; \
             fix the project roots or nx.json's namedInputs before merging"
        );
    }
}

/// `scripts/nx-affected.sh --affects <project>` — its verdict and its reasoning —
/// with the environment a CI leg hands it.
fn affects(project: &str, base_ref: Option<&str>) -> (String, String) {
    let mut command = Command::new(bash_program());
    command
        .arg("scripts/nx-affected.sh")
        .arg("--affects")
        .arg(project)
        .current_dir(repo_root())
        .env("CI", "1")
        .env_remove("NOTIGNORED_NX_BASE_REF")
        .env_remove("GITHUB_BASE_REF");
    if let Some(base_ref) = base_ref {
        command.env("GITHUB_BASE_REF", base_ref);
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("run scripts/nx-affected.sh: {error}"));
    assert!(
        output.status.success(),
        "`nx-affected.sh --affects {project}` failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    (
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Affected selection is a speed optimisation; one that can silently skip a
/// check is a correctness hole. Each of these is a real way the merge base goes
/// missing on a runner, and every one of them has to answer "run it".
///
/// The push build is the one that bites: a build *on* `main` has no base to
/// scope against, and comparing `main` with itself finds nothing changed — which
/// would skip the cross-platform, MSRV, audit, and install matrices on exactly
/// the commit that is about to be released.
///
/// The verdict alone would not prove this from a branch that genuinely touches
/// the crate — `true` is also what a *scoped* answer says there. So each case
/// asserts the reason too: that the script found no base and selected
/// everything, rather than having scoped and happened to agree.
#[test]
fn a_missing_merge_base_selects_the_crate_rather_than_skipping_it() {
    for (case, base_ref) in [
        ("a push build, which is on the base branch already", None),
        ("a base branch that does not exist", Some("no-such-branch")),
        (
            "a base ref that is not a usable branch name",
            Some("../evil"),
        ),
    ] {
        let (verdict, reasoning) = affects("notignored", base_ref);
        assert_eq!(
            verdict, "true",
            "with {case}, CI would skip the Rust matrices\n\
             ACTION: scripts/nx-affected.sh must fail closed — no derivable merge \
             base means every project is affected"
        );
        assert!(
            reasoning.contains("no merge base"),
            "with {case}, the script scoped instead of failing closed; it said:\n\
             {reasoning}\n\
             ACTION: it must report that it could not derive a base and select \
             everything, not answer from a comparison it should never have made"
        );
    }
}

/// A pinned tool is shared: the crate's parity suites drive it *and* an SDK's
/// gate runs it. Both projects have to re-run when the pin moves, which only
/// happens while each names the pin among its inputs.
#[test]
fn a_shared_toolchain_pin_reaches_both_projects_that_use_it() {
    for (pin, expected) in [
        (".ruff-version", ["notignored", "notignored-sdk-python"]),
        (
            "tests/js-toolchain/package.json",
            ["notignored", "notignored-sdk-npm"],
        ),
    ] {
        assert_eq!(
            project_list(&[
                "show",
                "projects",
                "--affected",
                &format!("--files={pin}"),
                "--json",
            ]),
            sorted(&expected),
            "moving the {pin} pin no longer re-runs every project that uses it\n\
             ACTION: name it in that project's target inputs (nx.json's \
             pythonToolchain / jsToolchain)"
        );
    }
}
