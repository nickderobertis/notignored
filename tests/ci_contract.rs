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

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

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
