//! Structural gate for the release pipeline and the two registry packages.
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
//!     `dynamic = ["version"]`, the npm packages via `scripts/npm-build.mjs`, and
//!     the committed npm manifest carries a placeholder so it cannot become a
//!     second source;
//!   * the **five targets** are the same in the binary, wheel, and npm matrices,
//!     in the platform-package names, and in the launcher's resolution map;
//!   * the **publish gating** is the two repository variables and the two repo
//!     secrets, with the build jobs deliberately left ungated so a packaging
//!     break reddens a release even while publishing is off.
//!
//! `release.yml` is read through the same strict YAML reader `action.yml` is
//! ([`workflow_yaml`]), so a malformed workflow is a failing test rather than a
//! failing release.

#[path = "support/workflow_yaml.rs"]
mod workflow_yaml;

use std::collections::BTreeSet;

use workflow_yaml::{parse, read, run_steps, Node};

/// The Rust targets every release matrix builds, paired with the npm platform
/// package each one produces.
const TARGETS: [(&str, &str); 5] = [
    ("x86_64-unknown-linux-gnu", "notignored-cli-linux-x64"),
    ("aarch64-unknown-linux-gnu", "notignored-cli-linux-arm64"),
    ("x86_64-apple-darwin", "notignored-cli-darwin-x64"),
    ("aarch64-apple-darwin", "notignored-cli-darwin-arm64"),
    ("x86_64-pc-windows-msvc", "notignored-cli-win32-x64"),
];

fn release_workflow() -> Node {
    parse(&read(".github/workflows/release.yml"))
}

fn jobs() -> Node {
    release_workflow().get("jobs").clone()
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
    for name in ["upload", "build-wheels", "build-npm", "publish-crate"] {
        assert!(
            needs(jobs.get(name)).contains("test"),
            "`{name}` no longer waits for the `test` gate"
        );
    }
    // The publish and verify jobs inherit the gate transitively, through the
    // build job whose artifacts they consume.
    for (name, upstream) in [
        ("publish-pypi", "build-wheels"),
        ("verify-pypi", "publish-pypi"),
        ("publish-npm", "build-npm"),
        ("verify-npm", "publish-npm"),
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
        ("publish-npm", "NPM_PUBLISH"),
        ("verify-npm", "NPM_PUBLISH"),
    ] {
        assert_eq!(
            jobs.get(name).get("if").scalar(),
            format!("${{{{ vars.{variable} == 'true' }}}}"),
            "`{name}` is no longer gated on the {variable} repository variable"
        );
    }
    for name in ["build-wheels", "build-npm"] {
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

/// No untrusted event payload is spliced into a release script.
///
/// The same rule `tests/action_contract.rs` enforces for the composite action:
/// `${{ }}` interpolation splices its text into the script as *source*, so a
/// value an outsider controls becomes shell. Matrix and workspace values are
/// generated by the runner and are safe; anything else goes through `env:`.
#[test]
fn no_run_script_interpolates_an_untrusted_value() {
    let jobs = jobs();
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
