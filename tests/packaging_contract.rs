//! Structural gate for the release pipeline and the registry packages.
//!
//! `pip install notignored-cli` and `npm install -g notignored-cli` ship the same
//! prebuilt binary the GitHub Release attaches, and every part of that is only
//! ever exercised for real by a published Release — the one thing a pull request
//! cannot rehearse. The journey that *can* be rehearsed is in
//! [`tests/e2e/packaging.rs`](e2e/packaging.rs), which builds both packages from
//! the real binary and installs them; what is left over is the wiring, and this
//! is where it is held still.
//!
//! Three facts have to agree in five places, so each is asserted against the
//! others rather than against a literal:
//!
//!   * the **version** comes from `Cargo.toml` alone — the wheel takes it via
//!     `dynamic = ["version"]`, the npm packages via `scripts/npm-build.mjs`, the
//!     Python SDK via `scripts/python-sdk-build.mjs`, and both committed
//!     manifests carry placeholders so neither can become a second source;
//!   * the **five targets** are the same in the binary, wheel, and npm matrices,
//!     in the platform-package names, and in the launcher's resolution map;
//!   * the **publish gating** is the two repository variables and the two repo
//!     secrets, with the build jobs deliberately left ungated so a packaging
//!     break reddens a release even while publishing is off.
//!
//! A fourth fact spans two files: the **post-publish verification** — the
//! release's `verify-*` jobs and the weekly `published-smoke.yml` sweep — runs on
//! every OS and asserts through the one `scripts/smoke-published.sh` that
//! [`tests/e2e/smoke.rs`](e2e/smoke.rs) drives against the build under test, so a
//! workflow's idea of "it works" cannot drift from the parser that ships.
//!
//! Both workflows are read through the same strict YAML reader `action.yml` is
//! ([`workflow_yaml`]), so a malformed workflow is a failing test rather than a
//! failing release.

#[path = "support/workflow_yaml.rs"]
mod workflow_yaml;

use std::collections::BTreeSet;

use workflow_yaml::{parse, read, repo_root, run_steps, Node};

/// The Rust targets every release matrix builds, paired with the npm platform
/// package each one produces.
const TARGETS: [(&str, &str); 5] = [
    ("x86_64-unknown-linux-gnu", "notignored-cli-linux-x64"),
    ("aarch64-unknown-linux-gnu", "notignored-cli-linux-arm64"),
    ("x86_64-apple-darwin", "notignored-cli-darwin-x64"),
    ("aarch64-apple-darwin", "notignored-cli-darwin-arm64"),
    ("x86_64-pc-windows-msvc", "notignored-cli-win32-x64"),
];

const RELEASE: &str = ".github/workflows/release.yml";
const SMOKE: &str = ".github/workflows/published-smoke.yml";

/// The one script that decides whether an installed `notignored` is the build
/// that shipped. Both workflows call it; `tests/e2e/smoke.rs` runs the same file
/// over the build under test.
const SMOKE_SCRIPT: &str = "scripts/smoke-published.sh";

/// The runner labels every post-publish verification uses.
///
/// One leg per *installable artifact*, not per OS: `pip install` and `npm
/// install -g` each resolve a different wheel and a different platform package
/// per platform **and architecture**, and only a runner of that pair can prove
/// the right one was picked. `macos-latest` is arm64, so `macos-15-intel` is
/// what covers `x86_64-apple-darwin` — without it, `notignored-cli-darwin-x64`
/// is published and never installed. The one released target still with no leg,
/// `aarch64-unknown-linux-gnu`, is covered by the build matrices and by
/// `tests/e2e/packaging.rs` on whichever host runs it.
const VERIFY_RUNNERS: [&str; 4] = [
    "ubuntu-latest",
    "macos-latest",
    "macos-15-intel",
    "windows-latest",
];

/// Every job that installs a published package and smoke-tests it, as
/// `(workflow, job)`.
const VERIFICATIONS: [(&str, &str); 4] = [
    (RELEASE, "verify-pypi"),
    (RELEASE, "verify-npm"),
    (SMOKE, "pypi"),
    (SMOKE, "npm"),
];

fn workflow(relative: &str) -> Node {
    parse(&read(relative))
}

fn release_workflow() -> Node {
    workflow(RELEASE)
}

fn jobs() -> Node {
    release_workflow().get("jobs").clone()
}

/// Every `run:` script in a job, as the reader flattened it.
fn scripts(job: &Node) -> Vec<String> {
    job.find("steps")
        .map(|steps| {
            run_steps(steps.list())
                .iter()
                .map(|step| step.get("run").scalar().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Every action a job `uses:`.
fn actions_used(job: &Node) -> Vec<String> {
    job.find("steps")
        .map(|steps| {
            steps
                .list()
                .iter()
                .filter_map(|step| step.find("uses"))
                .map(|uses| uses.scalar().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn job(name: &str) -> Node {
    jobs().get(name).clone()
}

/// The `needs:` of a job, as a set — the key spells one job as a scalar and
/// several as a sequence.
fn needs(job: &Node) -> BTreeSet<String> {
    match job.find("needs") {
        None => BTreeSet::new(),
        Some(Node::Scalar(one)) => [one.clone()].into_iter().collect(),
        Some(list) => list.list().iter().map(|n| n.scalar().to_string()).collect(),
    }
}

/// The targets named by a job's `strategy.matrix.include` entries.
fn matrix_targets(job: &Node) -> Vec<String> {
    job.get("strategy")
        .get("matrix")
        .get("include")
        .list()
        .iter()
        .map(|entry| entry.get("target").scalar().to_string())
        .collect()
}

fn cargo_version() -> String {
    let toml = read("Cargo.toml");
    let package = toml
        .split("[package]")
        .nth(1)
        .expect("Cargo.toml has a [package] section");
    let section = package.split("\n[").next().unwrap_or(package);
    section
        .lines()
        .find_map(|line| line.trim().strip_prefix("version"))
        .and_then(|rest| rest.split('"').nth(1))
        .expect("Cargo.toml [package] declares a version")
        .to_string()
}

fn launcher_manifest() -> serde_json::Value {
    serde_json::from_str(&read("npm/notignored/package.json"))
        .expect("the launcher package.json is valid JSON")
}

/// A Release, not a tag push, is what starts the pipeline.
///
/// release-plz cuts the Release with a PAT, and that is the event this workflow
/// answers; creating a Release by hand in the UI is the documented manual
/// fallback and takes the identical path. Firing on a bare tag push instead would
/// build artifacts for a Release that may not exist yet.
#[test]
fn the_release_pipeline_runs_on_a_published_release() {
    let workflow = release_workflow();
    let types = workflow.get("on").get("release").get("types").list();
    assert_eq!(
        types.iter().map(Node::scalar).collect::<Vec<_>>(),
        vec!["published"],
        "release.yml no longer runs on a published Release"
    );
}

/// Every publishing job publishes only what the gate passed.
///
/// The whole point of re-running `just check` inside the release is that no
/// artifact reaches a registry without it, and `needs:` is the only thing that
/// enforces it — a publish job that lost its edge to `test` would ship an
/// unvalidated build silently.
#[test]
fn nothing_publishes_without_the_gate() {
    let jobs = jobs();
    for name in [
        "upload",
        "build-wheels",
        "build-npm",
        "build-python-sdk",
        "publish-crate",
    ] {
        assert!(
            needs(jobs.get(name)).contains("test"),
            "`{name}` no longer waits for the `test` gate"
        );
    }
    // The publish and verify jobs inherit the gate transitively, through the
    // build job whose artifacts they consume. The SDK additionally waits on the
    // CLI's own publish: its dependency is an exact `notignored-cli==<version>`,
    // so that version has to be on PyPI before the SDK is installable at all.
    for (name, upstream) in [
        ("publish-pypi", "build-wheels"),
        ("verify-pypi", "publish-pypi"),
        ("publish-npm", "build-npm"),
        ("verify-npm", "publish-npm"),
        ("publish-python-sdk", "build-python-sdk"),
        ("publish-python-sdk", "publish-pypi"),
        ("verify-python-sdk", "publish-python-sdk"),
    ] {
        assert!(
            needs(jobs.get(name)).contains(upstream),
            "`{name}` no longer waits for `{upstream}`"
        );
    }
}

/// Publishing is switched by a repository variable; *building* is not.
///
/// A build that only ran when publishing was enabled would let a broken
/// `pyproject.toml` or launcher manifest sit green until the day someone flipped
/// the switch — which is the day it is hardest to debug. So the wheels and the
/// npm packages are built on every release, and only the upload is conditional.
#[test]
fn publishing_is_gated_on_a_repository_variable_but_building_is_not() {
    let jobs = jobs();
    for (name, variable) in [
        ("publish-pypi", "PYPI_PUBLISH"),
        ("verify-pypi", "PYPI_PUBLISH"),
        ("publish-python-sdk", "PYPI_PUBLISH"),
        ("verify-python-sdk", "PYPI_PUBLISH"),
        ("publish-npm", "NPM_PUBLISH"),
        ("verify-npm", "NPM_PUBLISH"),
    ] {
        assert_eq!(
            jobs.get(name).get("if").scalar(),
            format!("${{{{ vars.{variable} == 'true' }}}}"),
            "`{name}` is no longer gated on the {variable} repository variable"
        );
    }
    for name in ["build-wheels", "build-npm", "build-python-sdk"] {
        assert!(
            jobs.get(name).find("if").is_none(),
            "`{name}` became conditional; a packaging break would then stay \
             invisible until publishing is switched on"
        );
    }
}

/// The secrets a workflow actually reads, as `${{ secrets.NAME }}` expressions.
///
/// Only inside an expression: this file is mostly comments, and `secrets.` also
/// occurs in the prose that names the `gh-secrets.json` manifest. Reading the
/// whole text would count that as a secret called `json`.
fn secrets_referenced(workflow: &str) -> BTreeSet<&str> {
    workflow
        .match_indices("${{")
        .filter_map(|(at, _)| {
            let expression = workflow[at..].split_once("}}")?.0;
            let name = expression.split_once("secrets.")?.1;
            Some(
                name.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .next()
                    .expect("a name follows `secrets.`"),
            )
        })
        .collect()
}

/// The publishing credentials are exactly the four provisioned secrets.
///
/// `gh-secrets.json` is what puts a secret on the repository, so a workflow
/// referencing one that is not in the manifest would resolve to the empty string
/// at release time — an authentication failure with no local symptom.
#[test]
fn every_secret_the_pipeline_uses_is_provisioned() {
    let workflow = read(".github/workflows/release.yml");
    let used = secrets_referenced(&workflow);
    assert_eq!(
        used,
        BTreeSet::from([
            "CARGO_REGISTRY_TOKEN",
            "GITHUB_TOKEN",
            "NPM_TOKEN",
            "PYPI_TOKEN"
        ]),
        "the set of secrets release.yml uses changed"
    );

    // GITHUB_TOKEN is minted by the runner; the rest have to be provisioned.
    let manifest = read("gh-secrets.json");
    for secret in used.iter().filter(|name| **name != "GITHUB_TOKEN") {
        assert!(
            manifest.contains(&format!("\"name\": \"{secret}\"")),
            "release.yml uses {secret}, which gh-secrets.json does not provision"
        );
    }
}

/// The three matrices build the same five targets.
///
/// A wheel matrix that quietly lost a target ships a release where `pip install`
/// works on four platforms and falls back to a source build on the fifth — with
/// no Rust toolchain there to do it.
#[test]
fn the_binary_wheel_and_npm_matrices_cover_the_same_targets() {
    let expected: Vec<&str> = TARGETS.iter().map(|(target, _)| *target).collect();
    for name in ["upload", "build-wheels", "build-npm"] {
        assert_eq!(
            matrix_targets(&job(name)),
            expected,
            "`{name}` no longer builds the five released targets"
        );
    }
}

/// The wheel CI builds with is the wheel this repo proves.
///
/// `tests/e2e/packaging.rs` builds a wheel with the pinned local maturin; if
/// release.yml let maturin-action pick its own, the proven build and the shipped
/// build would be different programs.
#[test]
fn the_release_wheel_uses_the_pinned_maturin() {
    let pin = read(".maturin-version").trim().to_string();
    let build = job("build-wheels");
    let step = build
        .get("steps")
        .list()
        .iter()
        .find(|step| {
            step.find("uses")
                .is_some_and(|uses| uses.scalar().starts_with("PyO3/maturin-action@"))
        })
        .expect("build-wheels runs maturin-action");
    assert_eq!(
        step.get("with").get("maturin-version").scalar(),
        format!("v{pin}"),
        "release.yml pins a different maturin than .maturin-version"
    );
}

/// Every published package is installed and run on every OS a user installs it
/// on, by the one script this repo also runs against its own build.
///
/// A single-runner verification proves the *upload* worked, not the install: the
/// wheel `pip` resolves and the platform package `npm` resolves are chosen per
/// platform, so an install that is broken only on Windows looks green from
/// Linux. And what each leg asserts has to be the shared
/// [`SMOKE_SCRIPT`] — an inlined `grep -q '"E501"'` would keep passing after the
/// record around it changed shape, which is exactly the drift
/// `tests/e2e/smoke.rs` exists to catch on the pull request that caused it.
#[test]
fn every_published_package_is_smoke_tested_on_every_supported_os() {
    assert!(
        repo_root().join(SMOKE_SCRIPT).is_file(),
        "{SMOKE_SCRIPT} is gone, and every verification below calls it"
    );
    for (file, name) in VERIFICATIONS {
        let job = workflow(file).get("jobs").get(name).clone();
        let runners: Vec<&str> = job
            .get("strategy")
            .get("matrix")
            .get("os")
            .list()
            .iter()
            .map(Node::scalar)
            .collect();
        assert_eq!(
            runners, VERIFY_RUNNERS,
            "`{name}` in {file} no longer verifies the published package on every OS"
        );
        assert_eq!(
            job.get("runs-on").scalar(),
            "${{ matrix.os }}",
            "`{name}` in {file} declares a matrix it does not run on"
        );
        assert_eq!(
            job.get("strategy").get("fail-fast").scalar(),
            "false",
            "`{name}` in {file} stops at the first red platform, hiding the others"
        );
        // Windows defaults to pwsh, where none of these scripts are valid.
        assert_eq!(
            job.get("defaults").get("run").get("shell").scalar(),
            "bash",
            "`{name}` in {file} does not pin bash, so its Windows leg runs pwsh"
        );
        assert!(
            actions_used(&job)
                .iter()
                .any(|uses| uses.starts_with("actions/checkout@")),
            "`{name}` in {file} never checks out the smoke assets it asserts against"
        );
        let calls_smoke = scripts(&job).iter().any(|script| {
            script.contains(&format!("bash {SMOKE_SCRIPT}")) && script.contains("--expect-version")
        });
        assert!(
            calls_smoke,
            "`{name}` in {file} does not run `bash {SMOKE_SCRIPT} --expect-version …`; \
             an assertion written inline there cannot be held to the shipped parser"
        );
    }
}

/// The release verification installs the exact version the Release published.
///
/// `latest` would pass against the *previous* release for as long as the new one
/// takes to become the default — which is the window this job exists to cover.
#[test]
fn the_release_verification_installs_the_version_it_asserts() {
    for (name, specifier) in [
        ("verify-pypi", "pip install \"notignored-cli==${ver}\""),
        ("verify-npm", "npm install -g \"notignored-cli@${ver}\""),
    ] {
        let job = job(name);
        let installs = scripts(&job)
            .iter()
            .any(|script| script.contains(specifier) && script.contains("${GITHUB_REF_NAME#v}"));
        assert!(
            installs,
            "`{name}` no longer installs the released version with `{specifier}`"
        );
    }
}

/// The scheduled sweep runs on a schedule and by hand, and on nothing else.
///
/// Its whole purpose is to look at the registries when no one is releasing, so a
/// trigger tied to this repository's activity would defeat it. It also means the
/// sweep can never be a required check — branch protection lists contexts a
/// *pull request* reports, and this reports on none. That is recorded in
/// AGENTS.md rather than worked around.
#[test]
fn the_scheduled_smoke_runs_weekly_and_by_hand_and_never_on_a_pull_request() {
    let on = workflow(SMOKE).get("on").clone();
    assert_eq!(
        on.keys(),
        vec!["schedule", "workflow_dispatch"],
        "the published smoke's triggers changed; a pull-request trigger would make it \
         look requirable, and losing the schedule would make it a manual step nobody runs"
    );
    let crons: Vec<&str> = on
        .get("schedule")
        .list()
        .iter()
        .map(|entry| entry.get("cron").scalar())
        .collect();
    assert_eq!(
        crons.len(),
        1,
        "the published smoke declares {} schedules; one weekly sweep is the budget",
        crons.len()
    );
    // Day-of-week field pinned: `* * *` would be a daily run of six runners.
    let day_of_week = crons[0]
        .split_whitespace()
        .nth(4)
        .expect("a cron with five fields");
    assert_ne!(
        day_of_week, "*",
        "the published smoke's cron `{}` runs every day; it is a weekly sweep",
        crons[0]
    );
}

/// The sweep installs from the registries and builds nothing.
///
/// A weekly job that compiled the crate would cost more than the release it is
/// watching, and would prove the source tree rather than the artifact the
/// registry served — the one thing only this workflow can see. Holding the
/// action list to exactly the three installers is what keeps a cache, a
/// toolchain, or a `cargo build` from being added without saying so.
#[test]
fn the_scheduled_smoke_installs_from_the_registries_and_nothing_else() {
    let jobs = workflow(SMOKE).get("jobs").clone();
    let allowed = [
        "actions/checkout@",
        "actions/setup-python@",
        "actions/setup-node@",
    ];
    for name in jobs.keys() {
        let job = jobs.get(name);
        for uses in actions_used(job) {
            assert!(
                allowed.iter().any(|prefix| uses.starts_with(prefix)),
                "`{name}` in {SMOKE} uses `{uses}`; the sweep installs from the registries \
                 only — no toolchain, no cache, no build"
            );
        }
        for script in scripts(job) {
            for heavyweight in ["cargo ", "rustup ", "just ", "maturin "] {
                assert!(
                    !script.contains(heavyweight),
                    "`{name}` in {SMOKE} runs `{heavyweight}`; the sweep must stay a \
                     registry install and a smoke test"
                );
            }
        }
    }
}

/// The sweep is switched by the same variables that decide whether this project
/// publishes at all.
///
/// A repo with `PYPI_PUBLISH` off has nothing on PyPI to smoke, and a weekly red
/// run for an absent package is how a scheduled check gets switched off — taking
/// the real coverage with it.
#[test]
fn the_scheduled_smoke_is_gated_on_the_same_publish_variables() {
    let jobs = workflow(SMOKE).get("jobs").clone();
    for (name, variable) in [("pypi", "PYPI_PUBLISH"), ("npm", "NPM_PUBLISH")] {
        assert_eq!(
            jobs.get(name).get("if").scalar(),
            format!("${{{{ vars.{variable} == 'true' }}}}"),
            "`{name}` in {SMOKE} is no longer gated on the {variable} repository variable"
        );
    }
}

/// No untrusted event payload is spliced into a release script.
///
/// The same rule `tests/action_contract.rs` enforces for the composite action:
/// `${{ }}` interpolation splices its text into the script as *source*, so a
/// value an outsider controls becomes shell. Matrix and workspace values are
/// generated by the runner and are safe; anything else goes through `env:`.
#[test]
fn no_run_script_interpolates_an_untrusted_value() {
    for file in [RELEASE, SMOKE] {
        no_run_script_interpolates_an_untrusted_value_in(file);
    }
}

fn no_run_script_interpolates_an_untrusted_value_in(file: &str) {
    let jobs = workflow(file).get("jobs").clone();
    for name in jobs.keys() {
        let job = jobs.get(name);
        let Some(steps) = job.find("steps") else {
            continue;
        };
        for step in run_steps(steps.list()) {
            let script = step.get("run").scalar();
            for (at, _) in script.match_indices("${{") {
                let expression = script[at..]
                    .split_once("}}")
                    .map(|(head, _)| head.trim_start_matches("${{").trim())
                    .unwrap_or_default();
                assert!(
                    expression.starts_with("matrix.")
                        || expression.starts_with("github.repository")
                        || expression.starts_with("steps."),
                    "job `{name}` interpolates `{expression}` into a run script; \
                     pass it through `env:` instead"
                );
            }
        }
    }
}

/// The wheel is `notignored-cli`, built by maturin, with no version of its own.
///
/// A literal `version = ` in `pyproject.toml` would be a second version source:
/// release-plz bumps `Cargo.toml` and nothing else, so the wheel would ship a
/// number that had stopped moving.
#[test]
fn the_wheel_takes_its_name_from_pyproject_and_its_version_from_cargo() {
    let pyproject = read("pyproject.toml");
    assert!(
        pyproject.contains("name = \"notignored-cli\""),
        "the PyPI distribution is no longer notignored-cli"
    );
    assert!(
        pyproject.contains("dynamic = [\"version\"]"),
        "pyproject.toml must leave the version dynamic so maturin reads Cargo.toml"
    );
    assert!(
        !pyproject.contains("\nversion = "),
        "pyproject.toml declares a static version; Cargo.toml is the only version source"
    );
    // `bin` bindings are what make the wheel carry the compiled binary instead of
    // a Python extension module — the whole reason `pip install` needs no Rust.
    assert!(
        pyproject.contains("bindings = \"bin\""),
        "pyproject.toml no longer packages the binary with maturin's bin bindings"
    );
}

/// The Python SDK is a fourth registry package, and it has no version either.
///
/// `notignored-sdk` is pure Python, so nothing about it is per-platform — what it
/// *is* is a client pinned to one CLI. Both numbers that says are placeholders
/// here, stamped from `Cargo.toml` by `scripts/python-sdk-build.mjs`;
/// `python/notignored-sdk/tests/test_packaging.py` builds that wheel on every
/// gate run and reads the version and the pin back out of its metadata, so this
/// only has to hold the placeholders still.
#[test]
fn the_python_sdk_carries_placeholders_rather_than_a_second_version_source() {
    let pyproject = read("python/notignored-sdk/pyproject.toml");
    assert!(
        pyproject.contains("name = \"notignored-sdk\""),
        "the SDK distribution is no longer notignored-sdk"
    );
    let placeholder = "0.0.0.dev0";
    assert!(
        pyproject.contains(&format!("version = \"{placeholder}\"")),
        "python/notignored-sdk/pyproject.toml declares a real version; Cargo.toml is \
         the only version source and the packer stamps this one"
    );
    assert!(
        pyproject.contains("dependencies = [\"notignored-cli\"]"),
        "the SDK's committed notignored-cli dependency is no longer the unpinned \
         placeholder scripts/python-sdk-build.mjs tightens to an exact version"
    );
    assert_ne!(
        cargo_version(),
        placeholder,
        "Cargo.toml would have to hold the placeholder for this test to prove nothing"
    );

    // The packer is the only thing that turns those two into a release, so a
    // release that stopped calling it would publish a `.dev0` package.
    let build = job("build-python-sdk");
    let runs_packer = scripts(&build).iter().any(|script| {
        script.contains("node scripts/python-sdk-build.mjs") && script.contains("uv build")
    });
    assert!(
        runs_packer,
        "`build-python-sdk` no longer stamps the version in with \
         scripts/python-sdk-build.mjs before building the distributions"
    );
    assert!(
        repo_root().join("scripts/python-sdk-build.mjs").is_file(),
        "scripts/python-sdk-build.mjs is gone, and the release job calls it"
    );
}

/// The SDK release verification installs the exact version it asserts, and
/// proves the CLI came with it.
///
/// `pip install notignored-sdk` is only a promise until something resolves it: an
/// SDK whose `notignored-cli` pin does not exist installs cleanly and then cannot
/// scan anything. Checking both distributions' versions in the same interpreter
/// is what turns "we uploaded something" into "the pair works".
#[test]
fn the_sdk_verification_proves_both_halves_of_the_pin() {
    let job = job("verify-python-sdk");
    let scripts = scripts(&job);
    assert!(
        scripts.iter().any(
            |script| script.contains("pip install \"notignored-sdk==${ver}\"")
                && script.contains("${GITHUB_REF_NAME#v}")
        ),
        "`verify-python-sdk` no longer installs the released version"
    );
    for evidence in [
        "version(\"notignored-sdk\") == expected",
        "version(\"notignored-cli\") == expected",
        "from notignored_sdk import",
    ] {
        assert!(
            scripts.iter().any(|script| script.contains(evidence)),
            "`verify-python-sdk` no longer checks `{evidence}`; without it the job \
             proves the upload happened and nothing about what it published"
        );
    }
}

/// The launcher installs the `notignored` command and carries nothing else.
#[test]
fn the_npm_launcher_installs_the_notignored_command() {
    let manifest = launcher_manifest();
    assert_eq!(manifest["name"], "notignored-cli");
    assert_eq!(manifest["bin"]["notignored"], "bin/notignored.js");
    assert_eq!(
        manifest["files"],
        serde_json::json!(["bin/notignored.js"]),
        "the launcher must ship only its shim; the binaries live in the platform packages"
    );
}

/// The launcher's committed version is a placeholder, not a number.
///
/// `scripts/npm-build.mjs` stamps the real one from Cargo.toml at publish time
/// (proven end to end in `tests/e2e/packaging.rs`). A real version here would be
/// a second source that silently went stale.
#[test]
fn the_committed_launcher_manifest_holds_a_placeholder_version() {
    let manifest = launcher_manifest();
    let placeholder = "0.0.0-managed";
    assert_eq!(manifest["version"], placeholder);
    for (name, version) in manifest["optionalDependencies"]
        .as_object()
        .expect("the launcher declares optionalDependencies")
    {
        assert_eq!(
            version, placeholder,
            "the committed optionalDependency on {name} pins a real version"
        );
    }
    assert_ne!(
        cargo_version(),
        placeholder,
        "Cargo.toml would have to hold the placeholder for this test to prove nothing"
    );
}

/// The five platform packages are named the same in all three places.
///
/// The launcher's `optionalDependencies` decide what npm downloads, its
/// `PACKAGES` map decides what the shim resolves, and `npm-build.mjs` decides
/// what gets published. Any one of them drifting produces an install that
/// succeeds and a command that cannot find its binary.
#[test]
fn the_platform_package_names_agree_across_the_manifest_shim_and_builder() {
    let expected: BTreeSet<&str> = TARGETS.iter().map(|(_, package)| *package).collect();

    let manifest = launcher_manifest();
    let declared: BTreeSet<String> = manifest["optionalDependencies"]
        .as_object()
        .expect("the launcher declares optionalDependencies")
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        declared,
        expected.iter().map(|s| (*s).to_string()).collect(),
        "the launcher's optionalDependencies drifted from the released platforms"
    );

    let shim = read("npm/notignored/bin/notignored.js");
    let builder = read("scripts/npm-build.mjs");
    for (target, package) in TARGETS {
        // The shim keys on `<platform>-<arch>`, which is the package name's tail.
        let key = package
            .strip_prefix("notignored-cli-")
            .expect("a platform package is named after the launcher");
        assert!(
            shim.contains(&format!("\"{key}\": \"{package}\"")),
            "the launcher shim cannot resolve {package}"
        );
        assert!(
            builder.contains(target),
            "scripts/npm-build.mjs cannot build a package for {target}"
        );
    }
    assert!(
        !expected.iter().any(|package| package.starts_with('@')),
        "the platform packages must stay unscoped: a scoped name needs an npm \
         organization, which a publish token cannot create"
    );
}

/// The two READMEs name the platforms the release actually builds.
///
/// Both promise a prebuilt binary for a specific set, which is a second and third
/// copy of the release matrices. A target added to the matrices and not to the
/// prose leaves a user reading that their platform is unsupported while a package
/// for it sits on the registry; one removed leaves the opposite, which is worse.
/// The sentence is generated here from the same [`TARGETS`] the matrices are
/// checked against, so it cannot be updated in one place only.
#[test]
fn the_readmes_name_the_platforms_the_release_builds() {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for (_, package) in TARGETS {
        let facts = package
            .strip_prefix("notignored-cli-")
            .expect("a platform package is named after the launcher");
        let (platform, arch) = facts.split_once('-').expect("a <platform>-<arch> tail");
        let display = match platform {
            "linux" => "Linux",
            "darwin" => "macOS",
            "win32" => "Windows",
            other => panic!("no README spelling for the {other} platform"),
        };
        match groups.iter_mut().find(|(name, _)| *name == display) {
            Some((_, arches)) => arches.push(arch),
            None => groups.push((display, vec![arch])),
        }
    }
    let phrases: Vec<String> = groups
        .iter()
        .map(|(display, arches)| format!("{display} ({})", arches.join(", ")))
        .collect();
    let (last, rest) = phrases.split_last().expect("at least one platform");
    let sentence = format!("{}, and {last}", rest.join(", "));

    for readme in ["README.md", "npm/notignored/README.md"] {
        // Prose wraps, so compare against the text with its runs of whitespace
        // collapsed: where the line breaks fall is not part of the contract.
        let unwrapped = read(readme)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            unwrapped.contains(&sentence),
            "{readme} does not name the released platforms as `{sentence}`"
        );
    }
}

/// Only the jobs that read the repository are given a token that can.
///
/// The release workflow holds the credentials for three registries, so its token
/// is worth keeping small: a top-level grant is inherited by every job, including
/// the ones that only move artifacts between jobs or install from a public
/// registry. This ties the grant to the evidence — a job that checks out gets
/// repository access, and a job that does not gets none — so adding a job cannot
/// silently widen the token.
///
/// The scheduled sweep answers to the same rule: it runs unattended on `main`,
/// which is the worst place for a token that can write.
#[test]
fn only_the_jobs_that_read_the_repository_may() {
    for file in [RELEASE, SMOKE] {
        only_the_jobs_that_read_the_repository_may_in(file);
    }
}

fn only_the_jobs_that_read_the_repository_may_in(file: &str) {
    let workflow = workflow(file);
    assert_eq!(
        workflow.get("permissions").scalar(),
        "{}",
        "{file} grants a permission to every job by default"
    );

    let jobs = workflow.get("jobs").clone();
    for name in jobs.keys() {
        let job = jobs.get(name);
        let checks_out = job.find("steps").is_some_and(|steps| {
            steps.list().iter().any(|step| {
                step.find("uses")
                    .is_some_and(|uses| uses.scalar().starts_with("actions/checkout@"))
            })
        });
        let granted = job
            .find("permissions")
            .is_some_and(|permissions| permissions.find("contents").is_some());
        assert_eq!(
            granted,
            checks_out,
            "`{name}` {} the repository but {} `contents` access",
            if checks_out {
                "checks out"
            } else {
                "never reads"
            },
            if granted {
                "is granted"
            } else {
                "is not granted"
            }
        );
    }
}
