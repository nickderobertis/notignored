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
/// The names are the repo's three deliverables — the CLI crate and the two SDKs
/// — and the crate's two test tiers that are projects of their own so a change
/// pays only for the tiers it reaches: the `tests/*.rs` contract suites and the
/// `tests/e2e/` journeys. The crate's unit tests stay in the crate's project.
const PROJECTS: [&str; 5] = [
    "notignored",
    "notignored-integration",
    "notignored-e2e",
    "notignored-sdk-python",
    "notignored-sdk-npm",
];

/// The crate's own projects: the crate and its test tiers. What CI's
/// cross-platform legs run, so `just affected-crate` asks after every one.
const CRATE_PROJECTS: [&str; 3] = ["notignored", "notignored-integration", "notignored-e2e"];

/// The project whose `coverage` target enforces the crate's floor.
const COVERAGE_PROJECT: &str = "notignored-e2e";

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
        // These read Nx's answer off stdout, so the wrapper streams instead of
        // folding it into its one-line summary. `tests/e2e/nx_wrapper.rs` owns
        // the quiet default and proves this mode still passes stdout through.
        .env("NOTIGNORED_NX_SHOW_OUTPUT", "1")
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
fn the_graph_holds_the_deliverables_and_the_crates_test_tiers() {
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

/// The coverage floor is a property of `src/` as every tier exercises it, so the
/// one report that enforces it has to wait on, and combine, every tier's
/// profiles. A tier missing from either list would let the floor be measured
/// over less of the suite than the e2e-inclusive run it replaced — silently,
/// since a smaller suite can still clear 95% on the lines it happens to reach.
///
/// It is the e2e project's target, and in that project's `check`, because it
/// needs the journeys' profiles: hosted anywhere the journeys are not selected,
/// it would pull the expensive tier back into every change that reaches it.
#[test]
fn the_crates_coverage_combines_every_test_tier() {
    let config: serde_json::Value =
        serde_json::from_str(nx(&["show", "project", COVERAGE_PROJECT, "--json"]).trim())
            .expect("`nx show project` is JSON");
    assert!(
        config["targets"]["check"]["dependsOn"]
            .as_array()
            .is_some_and(|tiers| tiers.iter().any(|tier| tier == "coverage")),
        "{COVERAGE_PROJECT}'s `check` no longer runs `coverage`, so no gate enforces the floor"
    );
    let coverage = &config["targets"]["coverage"];
    let depends_on = coverage["dependsOn"]
        .as_array()
        .expect("the `coverage` target declares dependsOn");
    let waits_on = |project: &str| {
        depends_on.iter().any(|entry| {
            (project == COVERAGE_PROJECT && entry == "test")
                || (entry["target"] == "test"
                    && entry["projects"]
                        .as_array()
                        .is_some_and(|projects| projects.iter().any(|name| name == project)))
        })
    };
    let command = coverage["options"]["command"]
        .as_str()
        .or_else(|| coverage["command"].as_str())
        .expect("the crate's `coverage` runs a command");
    let recipe = std::fs::read_to_string(repo_root().join("justfile")).expect("read justfile");
    let report = recipe
        .lines()
        .find(|line| line.contains("scripts/coverage.sh report"))
        .expect("a justfile recipe runs scripts/coverage.sh report");
    assert_eq!(
        command, "just _crate-coverage",
        "the crate's coverage target no longer runs the recipe this test reads"
    );
    let script =
        std::fs::read_to_string(repo_root().join("scripts/coverage.sh")).expect("read coverage.sh");
    assert!(
        script.contains(r#"readonly STORE="$ROOT/target/coverage-profiles""#),
        "scripts/coverage.sh no longer stores profiles under target/coverage-profiles, \
         the directory every tier's `test` declares as its output"
    );
    for project in CRATE_PROJECTS {
        let tier: serde_json::Value =
            serde_json::from_str(nx(&["show", "project", project, "--json"]).trim())
                .unwrap_or_else(|error| panic!("`nx show project {project}` is not JSON: {error}"));
        assert_eq!(
            tier["targets"]["test"]["outputs"],
            serde_json::json!([format!(
                "{{workspaceRoot}}/target/coverage-profiles/{project}"
            )]),
            "{project}:test does not declare the directory scripts/coverage.sh files its \
             profiles in, so a cached run would replay without them\n\
             ACTION: set its outputs to that directory"
        );
        assert!(
            waits_on(project),
            "`{COVERAGE_PROJECT}:coverage` does not depend on {project}:test, so it can \
             report before that tier's profiles exist\n\
             ACTION: add it to the coverage target's dependsOn in project.json"
        );
        assert!(
            report.split_whitespace().any(|word| word == project),
            "`just _crate-coverage` does not combine {project}'s profiles:\n{report}\n\
             ACTION: name it in the recipe's `scripts/coverage.sh report` list"
        );
    }
}

/// What CI skips a matrix on. Each deliverable's tree must map to its own
/// project and nothing else, or an SDK-only pull request skips the Rust matrices
/// while having changed the Rust artifact.
///
/// The crate's *sources* are the one exception, in the direction the layering
/// allows: the Python SDK's suite compiles and drives that binary, so it names
/// `crateSource` among its `test` inputs and a change there selects both. That is
/// the point — a report shape that moved in `src/` has to re-run the suite that
/// asserts on it rather than replay a cached green from before the move. It is
/// scoped to `src/` and the manifests deliberately, because the crate's project
/// root *is* the repository root: depending on the project would make every file
/// outside the SDK trees affect it.
///
/// The crate's test tiers are the other direction. The contract suites depend on
/// the crate project, since they read nearly every file it owns. The journeys are
/// the expensive tier, so like the SDKs they name what they read — the crate's
/// sources and the root files they drive (`e2eRootInputs`) — rather than the
/// whole root: a change to `AGENTS.md` or `docs/` does not run them. A change to
/// one tier's own tests selects that tier alone; the fixtures and goldens live
/// beside the contract suites but are read by the journeys too.
#[test]
fn affected_selection_maps_each_tree_to_its_own_project() {
    const CRATE_AND_TIERS: &[&str] = &CRATE_PROJECTS;
    let cases: [(&str, &[&str]); 12] = [
        (
            "src/lib.rs",
            &[
                "notignored",
                "notignored-integration",
                "notignored-e2e",
                "notignored-sdk-npm",
                "notignored-sdk-python",
            ],
        ),
        (
            "Cargo.toml",
            &[
                "notignored",
                "notignored-integration",
                "notignored-e2e",
                "notignored-sdk-npm",
                "notignored-sdk-python",
            ],
        ),
        ("npm/notignored/package.json", CRATE_AND_TIERS),
        ("scripts/install.sh", CRATE_AND_TIERS),
        ("README.md", CRATE_AND_TIERS),
        ("AGENTS.md", &["notignored", "notignored-integration"]),
        ("tests/ci_contract.rs", &["notignored-integration"]),
        ("tests/e2e/cli.rs", &["notignored-e2e"]),
        (
            "tests/fixtures/polyglot/api/service.py",
            &["notignored-integration", "notignored-e2e"],
        ),
        (
            "python/notignored-sdk/README.md",
            &["notignored-sdk-python"],
        ),
        ("npm/notignored-sdk/README.md", &["notignored-sdk-npm"]),
        // The orchestrator's own config changes what every target *is*, so it
        // has to reach every project — the one case where scoping would be wrong.
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

/// Whether a `{workspaceRoot}` input pattern from nx.json covers `path`: a
/// `dir/**/*` pattern covers that tree, a `*` matches within one name, and
/// anything else is the path itself.
fn input_covers(pattern: &str, path: &str) -> bool {
    if let Some(tree) = pattern.strip_suffix("**/*") {
        return path.starts_with(tree) || format!("{path}/") == tree;
    }
    match pattern.split_once('*') {
        Some((head, tail)) => {
            path.starts_with(head)
                && path.ends_with(tail)
                && !path[head.len()..path.len() - tail.len()].contains('/')
        }
        None => pattern == path,
    }
}

/// The e2e tier names the root files it reads instead of depending on the root
/// project, so the list can fall behind the journeys silently: a script a
/// journey starts running, missing from it, neither selects the tier when it
/// changes nor invalidates a cached green. Every repository script a journey
/// names is held to the list here; the scripts those source are listed beside
/// them by hand.
#[test]
fn every_script_a_journey_runs_is_an_e2e_input() {
    let root = repo_root();
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("nx.json")).expect("read nx.json"))
            .expect("nx.json is JSON");
    // Named inputs nest (`e2eRootInputs` takes in `crateSource`), so expand them.
    fn expand(config: &serde_json::Value, input: &str, patterns: &mut Vec<String>) {
        match input.strip_prefix("{workspaceRoot}/") {
            Some(pattern) => patterns.push(pattern.to_string()),
            None => {
                for nested in config["namedInputs"][input]
                    .as_array()
                    .unwrap_or_else(|| panic!("nx.json names no `{input}` input"))
                {
                    expand(config, nested.as_str().expect("a string input"), patterns);
                }
            }
        }
    }
    let mut patterns = Vec::new();
    expand(&config, "e2eRootInputs", &mut patterns);
    let mut named = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(root.join("tests/e2e")).expect("read tests/e2e") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("read a journey");
        for (start, _) in source.match_indices("scripts/") {
            let script: String = source[start..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || "_./-".contains(*c))
                .collect();
            let script = script.trim_end_matches(['/', '.']).to_string();
            // Journeys also name scripts inside the scratch repositories they
            // build; only the repository's own files are inputs.
            if script != "scripts" && root.join(&script).is_file() {
                named.insert(script);
            }
        }
    }
    assert!(
        named.contains("scripts/install.sh"),
        "the scan found none of the scripts the journeys are known to run: {named:?}"
    );
    let missing: Vec<&String> = named
        .iter()
        .filter(|script| !patterns.iter().any(|pattern| input_covers(pattern, script)))
        .collect();
    assert!(
        missing.is_empty(),
        "the journeys run {missing:?}, which nx.json's e2eRootInputs does not name, so \
         a change to them neither selects nor re-runs the e2e tier\n\
         ACTION: add each to e2eRootInputs"
    );
    assert!(
        !input_covers("bin/setup-*.sh", "bin/session-setup.sh")
            && !input_covers("bin/setup-*.sh", "bin/setup-x/y.sh")
            && input_covers("bin/setup-*.sh", "bin/setup-js.sh")
            && input_covers("bin/action/**/*", "bin/action/comment.sh")
            && !input_covers("bin/action/**/*", "bin/actions.sh"),
        "input_covers no longer reads nx.json's patterns the way Nx does"
    );
}

/// The layering every dependency in the graph has to respect, as
/// `scope` -> the scopes it may depend on.
///
/// The CLI is the base: it is the published artifact, and the SDKs are clients
/// that will wrap it. An edge the other way would make the crate's gate depend
/// on an SDK's, which is how a monorepo turns into one indivisible build.
///
/// Enforced here rather than through Nx's `@nx/enforce-module-boundaries` lint
/// rule, which is ESLint's and so can only see the one TypeScript project — a
/// rule that cannot reach the Rust or Python project is not enforcing this
/// graph's boundaries. The tags it keys on are the same ones that rule would use.
///
/// The crate's test tiers (`scope:cli-tests`) sit on the CLI the same way: they
/// may depend on it, and nothing may depend on them, so no tier can end up on
/// the path of a change that does not reach it.
const LAYERS: [(&str, &[&str]); 3] = [
    ("scope:cli", &[]),
    ("scope:cli-tests", &["scope:cli"]),
    ("scope:sdk", &["scope:cli"]),
];

/// Whether a project in `source_scope` may depend on one in `target_scope`.
fn may_depend_on(source_scope: &str, target_scope: &str) -> bool {
    LAYERS
        .iter()
        .find(|(scope, _)| *scope == source_scope)
        .is_some_and(|(_, allowed)| allowed.contains(&target_scope))
}

/// The one `scope:` tag a project declares, which is what the layering keys on.
fn scope_of(tags: &[String], project: &str) -> String {
    let scopes: Vec<&String> = tags
        .iter()
        .filter(|tag| tag.starts_with("scope:"))
        .collect();
    assert_eq!(
        scopes.len(),
        1,
        "{project} declares {} `scope:` tags ({tags:?}); the boundary rule cannot \
         decide what it may depend on\n\
         ACTION: give every project exactly one scope in its project.json",
        scopes.len()
    );
    assert!(
        LAYERS.iter().any(|(scope, _)| scope == scopes[0]),
        "{project} is tagged {} which the layering in tests/e2e/nx_workspace.rs \
         knows nothing about\n\
         ACTION: add it to LAYERS with the scopes it may depend on",
        scopes[0]
    );
    scopes[0].clone()
}

/// The whole project graph as Nx computes it, nodes and dependency edges.
fn graph() -> serde_json::Value {
    let stdout = nx(&["graph", "--print"]);
    let start = stdout
        .find('{')
        .unwrap_or_else(|| panic!("`nx graph --print` printed no JSON:\n{stdout}"));
    serde_json::from_str::<serde_json::Value>(&stdout[start..])
        .unwrap_or_else(|error| panic!("`nx graph --print` is not JSON: {error}"))["graph"]
        .clone()
}

fn tags_by_project(graph: &serde_json::Value) -> std::collections::BTreeMap<String, Vec<String>> {
    graph["nodes"]
        .as_object()
        .expect("the graph has nodes")
        .iter()
        .map(|(name, node)| {
            let tags = node["data"]["tags"]
                .as_array()
                .map(|tags| {
                    tags.iter()
                        .filter_map(|tag| tag.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            (name.clone(), tags)
        })
        .collect()
}

/// Every `source -> target` edge Nx resolved between projects.
fn edges(graph: &serde_json::Value) -> Vec<(String, String)> {
    graph["dependencies"]
        .as_object()
        .expect("the graph has a dependency map")
        .iter()
        .flat_map(|(source, targets)| {
            targets
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(move |edge| {
                    edge["target"]
                        .as_str()
                        .map(|target| (source.clone(), target.to_string()))
                })
        })
        .collect()
}

#[test]
fn every_project_declares_the_scope_its_boundaries_are_keyed_on() {
    let graph = graph();
    for (project, tags) in tags_by_project(&graph) {
        scope_of(&tags, &project);
    }
}

/// The boundary rule itself, applied to the graph Nx actually resolved.
#[test]
fn every_dependency_in_the_graph_respects_the_layering() {
    let graph = graph();
    let tags = tags_by_project(&graph);
    for (source, target) in edges(&graph) {
        let (source_scope, target_scope) = (
            scope_of(&tags[&source], &source),
            scope_of(&tags[&target], &target),
        );
        assert!(
            may_depend_on(&source_scope, &target_scope),
            "{source} ({source_scope}) depends on {target} ({target_scope}), which \
             the layering does not allow\n\
             ACTION: invert the dependency, or widen LAYERS deliberately — the CLI \
             is the base and must not depend on an SDK"
        );
    }
}

/// The rule has to *reject* something, and today's graph has no cross-project
/// edges to reject — so the decision is exercised on the edges a future change
/// would introduce. Without this, the check above passes over an empty set and
/// proves nothing.
#[test]
fn the_layering_rejects_an_edge_that_inverts_it() {
    assert!(
        may_depend_on("scope:sdk", "scope:cli"),
        "an SDK wrapping the CLI is the dependency this graph is for"
    );
    assert!(
        !may_depend_on("scope:cli", "scope:sdk"),
        "the CLI must not depend on an SDK: its gate would then wait on theirs"
    );
    assert!(
        !may_depend_on("scope:sdk", "scope:sdk"),
        "the SDKs are siblings; one must not reach into the other"
    );
    assert!(
        !may_depend_on("scope:cli", "scope:cli"),
        "a scope must not depend on itself, which is where a cycle starts"
    );
    assert!(
        may_depend_on("scope:cli-tests", "scope:cli"),
        "a test tier depending on the crate it drives is the edge that selects it"
    );
    for dependent in ["scope:cli", "scope:sdk", "scope:cli-tests"] {
        assert!(
            !may_depend_on(dependent, "scope:cli-tests"),
            "{dependent} must not depend on a test tier: the tier would then run \
             for every change that reaches {dependent}"
        );
    }
}

/// Layering keeps the graph acyclic only while it is actually acyclic — an Nx
/// plugin that inferred an edge could still close a loop, and a cyclic graph is
/// one whose targets cannot be ordered at all.
#[test]
fn the_project_graph_is_acyclic() {
    let graph = graph();
    let edges = edges(&graph);
    let mut settled: std::collections::BTreeSet<String> = Default::default();
    let mut path: Vec<String> = Vec::new();

    fn walk(
        project: &str,
        edges: &[(String, String)],
        settled: &mut std::collections::BTreeSet<String>,
        path: &mut Vec<String>,
    ) {
        if settled.contains(project) {
            return;
        }
        assert!(
            !path.iter().any(|seen| seen == project),
            "the project graph has a cycle: {} -> {project}\n\
             ACTION: break it — Nx cannot order targets around a cycle",
            path.join(" -> ")
        );
        path.push(project.to_string());
        for (source, target) in edges {
            if source == project {
                walk(target, edges, settled, path);
            }
        }
        path.pop();
        settled.insert(project.to_string());
    }

    for project in tags_by_project(&graph).keys() {
        walk(project, &edges, &mut settled, &mut path);
    }
}

/// The variables that choose nx-affected.sh's base, cleared on every journey so
/// a developer's own shell cannot decide one.
const BASE_VARIABLES: [&str; 3] = [
    "NOTIGNORED_NX_BASE_SHA",
    "NOTIGNORED_NX_BASE_REF",
    "GITHUB_BASE_REF",
];

/// `scripts/nx-affected.sh --affects <projects>` — its verdict and its
/// reasoning — with the environment a CI leg hands it plus `base`, the base
/// variables this case sets.
fn affects(projects: &[&str], base: &[(&str, &str)]) -> (String, String) {
    let mut command = Command::new(bash_program());
    command
        .arg("scripts/nx-affected.sh")
        .arg("--affects")
        .args(projects)
        .current_dir(repo_root())
        .env("CI", "1");
    for variable in BASE_VARIABLES {
        command.env_remove(variable);
    }
    command.envs(base.iter().copied());
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("run scripts/nx-affected.sh: {error}"));
    assert!(
        output.status.success(),
        "`nx-affected.sh --affects {projects:?}` failed:\n{}",
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
    for (case, base) in [
        ("a push build, which is on the base branch already", &[][..]),
        (
            "a base branch that does not exist",
            &[("GITHUB_BASE_REF", "no-such-branch")][..],
        ),
        (
            "a base ref that is not a usable branch name",
            &[("GITHUB_BASE_REF", "../evil")][..],
        ),
    ] {
        let (verdict, reasoning) = affects(&["notignored"], base);
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

/// `NOTIGNORED_NX_BASE_SHA` is how a push build names its base — the pushed
/// range's, since it has no base branch to fork from — and it is an instruction,
/// not a hint: it wins over any base ref the environment also carries.
///
/// HEAD against itself has nothing changed, so `false` is an answer only a
/// comparison against that exact commit can give; the conflicting ref, had it
/// been used, names a branch that does not exist and would have selected
/// everything instead.
#[test]
fn an_explicit_base_commit_wins_over_a_base_ref() {
    let head = crate::support::git_stdout(&repo_root(), &["rev-parse", "HEAD"]);
    let (verdict, reasoning) = affects(
        &CRATE_PROJECTS,
        &[
            ("NOTIGNORED_NX_BASE_SHA", head.trim()),
            ("NOTIGNORED_NX_BASE_REF", "no-such-branch"),
            ("GITHUB_BASE_REF", "no-such-branch"),
        ],
    );
    assert_eq!(
        (verdict.as_str(), reasoning.contains("no merge base")),
        ("false", false),
        "with NOTIGNORED_NX_BASE_SHA at HEAD the script still consulted the base \
         ref; it said:\n{reasoning}\n\
         ACTION: an explicit base commit must take precedence over every base ref"
    );
}

/// A base commit that does not resolve is never "close enough" to another: the
/// script must select everything and say which variable it could not use, rather
/// than quietly scope against some other base.
#[test]
fn an_unresolvable_base_commit_fails_closed_naming_the_variable() {
    for (case, sha) in [
        (
            "a commit this checkout does not have",
            "0000000000000000000000000000000000000000",
        ),
        ("a revision that is not a commit id", "HEAD~1"),
        ("shell text", "$(touch pwned)"),
    ] {
        let (verdict, reasoning) = affects(
            &["notignored"],
            &[("NOTIGNORED_NX_BASE_SHA", sha), ("GITHUB_BASE_REF", "main")],
        );
        assert_eq!(
            verdict, "true",
            "with {case} as the base commit, CI would skip the Rust matrices\n\
             ACTION: scripts/nx-affected.sh must fail closed on a base it cannot resolve"
        );
        assert!(
            reasoning.contains("NOTIGNORED_NX_BASE_SHA") && reasoning.contains("no merge base"),
            "with {case}, the script did not refuse the base by name; it said:\n{reasoning}"
        );
    }
}

/// A pinned tool is shared: the crate's parity suites drive it *and* an SDK's
/// gate runs it. Both projects have to re-run when the pin moves, which only
/// happens while each names the pin among its inputs.
#[test]
fn a_shared_toolchain_pin_reaches_both_projects_that_use_it() {
    for (pin, expected) in [
        (
            ".ruff-version",
            [
                "notignored",
                "notignored-integration",
                "notignored-e2e",
                "notignored-sdk-python",
            ]
            .as_slice(),
        ),
        (
            "tests/js-toolchain/package.json",
            [
                "notignored-integration",
                "notignored-e2e",
                "notignored-sdk-npm",
            ]
            .as_slice(),
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
            sorted(expected),
            "moving the {pin} pin no longer re-runs every project that uses it\n\
             ACTION: name it in that project's target inputs (nx.json's \
             pythonToolchain / jsToolchain)"
        );
    }
}

/// The files the project graph and `just affected-crate` are made of, beyond each
/// project's own `project.json`.
#[cfg(unix)]
const GRAPH_FILES: [&str; 9] = [
    "nx.json",
    "package.json",
    "package-lock.json",
    ".gitignore",
    "justfile",
    "Cargo.toml",
    "scripts/nx.sh",
    "scripts/nx-affected.sh",
    "scripts/preserved-log.sh",
];

/// A scratch repository holding this workspace's real graph — every project's
/// definition, the Nx config and lockfile, the recipes and the scripts they run —
/// committed once as a base, so a journey can make a real commit on top and ask
/// the real recipe about it. The orchestrator's install is linked rather than
/// reinstalled. Returns the repository and its base commit.
///
/// With `probe`, every project also gets a `probe` target that records its own
/// name in `probe.log` — the one addition to the real graph, so a journey can
/// see which projects a run executed without running their real targets.
///
/// Unix only for the link; the cross-platform legs still run it on macOS.
#[cfg(unix)]
fn scratch_workspace_with(probe: bool) -> (tempfile::TempDir, String) {
    use crate::support::{commit, git_repo, git_stdout};
    let root = repo_root();
    let dir = git_repo();
    let roots = PROJECTS.map(|project| {
        let config: serde_json::Value =
            serde_json::from_str(nx(&["show", "project", project, "--json"]).trim())
                .unwrap_or_else(|error| panic!("`nx show project {project}` is not JSON: {error}"));
        let root = config["root"].as_str().expect("a project root").to_string();
        if root == "." {
            "project.json".to_string()
        } else {
            format!("{root}/project.json")
        }
    });
    for file in GRAPH_FILES
        .iter()
        .copied()
        .chain(roots.iter().map(String::as_str))
    {
        let target = dir.path().join(file);
        std::fs::create_dir_all(target.parent().expect("a parent")).expect("create parent");
        std::fs::copy(root.join(file), &target)
            .unwrap_or_else(|error| panic!("copy {file} into the scratch workspace: {error}"));
    }
    if probe {
        for file in &roots {
            let path = dir.path().join(file);
            let mut config: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).expect("read project.json"))
                    .expect("project.json is JSON");
            config["targets"]["probe"] =
                serde_json::json!({ "command": "echo {projectName} >> probe.log" });
            std::fs::write(&path, config.to_string()).expect("write project.json");
        }
        crate::support::write(
            dir.path(),
            ".gitignore",
            "/node_modules\n/.nx\n/.logs\n/probe.log\n",
        );
    }
    std::os::unix::fs::symlink(root.join("node_modules"), dir.path().join("node_modules"))
        .expect("link node_modules");
    commit(dir.path(), "base");
    let base = git_stdout(dir.path(), &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    (dir, base)
}

#[cfg(unix)]
fn scratch_workspace() -> (tempfile::TempDir, String) {
    scratch_workspace_with(false)
}

/// `just affected-crate` in `dir`, as a push build hands it a base commit.
#[cfg(unix)]
fn affected_crate(dir: &std::path::Path, base_sha: &str) -> (String, String) {
    let just = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|entry| entry.join("just"))
        .find(|candidate| candidate.is_file())
        .expect("`just` on PATH — it is this repository's command surface");
    let mut command = Command::new(just);
    command
        .arg("affected-crate")
        .current_dir(dir)
        .env("CI", "1")
        .env("NOTIGNORED_NX_BASE_SHA", base_sha);
    for variable in BASE_VARIABLES.iter().skip(1) {
        command.env_remove(variable);
    }
    // An enclosing Nx task's own variables describe *this* workspace, not the
    // scratch one; the nested Nx must find its root from its working directory.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NX_") {
            command.env_remove(name);
        }
    }
    let output = command.output().expect("run just affected-crate");
    assert!(
        output.status.success(),
        "`just affected-crate` failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// What CI skips the cross-platform, MSRV, audit, and install matrices on, asked
/// the way a push to main asks it: the recipe, real commits, an explicit base.
/// Anything that reaches the crate artifact — or a test tier the cross-platform
/// legs run — must answer `true`; an SDK-only change is the one that may skip.
#[cfg(unix)]
#[test]
fn affected_crate_answers_for_real_commits_against_an_explicit_base() {
    use crate::support::{commit, write};
    for (file, expected) in [
        ("src/lib.rs", "true"),
        ("Cargo.toml", "true"),
        ("tests/e2e/cli.rs", "true"),
        ("tests/ci_contract.rs", "true"),
        ("python/notignored-sdk/README.md", "false"),
    ] {
        let (dir, base) = scratch_workspace();
        let path = dir.path().join(file);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        write(dir.path(), file, &format!("{existing}\n# changed\n"));
        commit(dir.path(), &format!("change {file}"));
        let (verdict, reasoning) = affected_crate(dir.path(), &base);
        assert_eq!(
            verdict, expected,
            "a commit changing {file} answered `{verdict}` from `just affected-crate`; \
             it said:\n{reasoning}\n\
             ACTION: CI skips the Rust matrices on this answer — fix the project \
             roots or the recipe's project list"
        );
        assert!(
            !reasoning.contains("no merge base"),
            "for {file} the recipe failed closed instead of scoping:\n{reasoning}"
        );
    }

    let (dir, _) = scratch_workspace();
    let (verdict, reasoning) =
        affected_crate(dir.path(), "1111111111111111111111111111111111111111");
    assert_eq!(
        verdict, "true",
        "an unresolvable base let `just affected-crate` skip the Rust matrices"
    );
    assert!(
        reasoning.contains("NOTIGNORED_NX_BASE_SHA"),
        "the recipe failed closed without naming the variable it could not use:\n{reasoning}"
    );
}

/// `scripts/nx-affected.sh -t probe` in `dir` with `base_sha` as a push build's
/// base: which projects it ran, and what it said.
#[cfg(unix)]
fn affected_run(dir: &std::path::Path, base_sha: &str) -> (Vec<String>, String) {
    let mut command = Command::new(bash_program());
    command
        .args(["scripts/nx-affected.sh", "-t", "probe"])
        .current_dir(dir)
        .env("CI", "1")
        .env("NOTIGNORED_NX_BASE_SHA", base_sha);
    for variable in BASE_VARIABLES.iter().skip(1) {
        command.env_remove(variable);
    }
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NX_") {
            command.env_remove(name);
        }
    }
    let output = command.output().expect("run scripts/nx-affected.sh");
    assert!(
        output.status.success(),
        "`nx-affected.sh -t probe` failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut ran: Vec<String> = std::fs::read_to_string(dir.join("probe.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    ran.sort();
    (ran, String::from_utf8_lossy(&output.stderr).into_owned())
}

/// The push-to-main gate itself: `just check-affected` is `nx-affected.sh -t
/// check`, and with an explicit base it must execute the targets of exactly the
/// projects the pushed range reaches — and of every project when that base
/// does not resolve, saying which variable it could not use.
#[cfg(unix)]
#[test]
fn an_affected_run_executes_against_an_explicit_base() {
    use crate::support::{commit, write};
    for (file, expected) in [
        ("tests/e2e/cli.rs", &["notignored-e2e"][..]),
        ("tests/ci_contract.rs", &["notignored-integration"][..]),
        ("src/lib.rs", &PROJECTS[..]),
    ] {
        let (dir, base) = scratch_workspace_with(true);
        write(dir.path(), file, "// changed\n");
        commit(dir.path(), &format!("change {file}"));
        let (ran, said) = affected_run(dir.path(), &base);
        assert_eq!(
            ran,
            sorted(expected),
            "a pushed change to {file} ran the wrong projects; the script said:\n{said}"
        );
    }

    let (dir, _) = scratch_workspace_with(true);
    let (ran, said) = affected_run(dir.path(), "2222222222222222222222222222222222222222");
    assert_eq!(
        ran,
        sorted(&PROJECTS),
        "an unresolvable base ran less than every project:\n{said}"
    );
    assert!(
        said.contains("NOTIGNORED_NX_BASE_SHA") && said.contains("running every project"),
        "the full run did not say which base it could not use:\n{said}"
    );
}
