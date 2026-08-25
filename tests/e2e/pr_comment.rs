//! The rendered-comment capture's review case, executed — not read.
//!
//! The README's marketing passage leads with a picture of the comment a reviewer
//! meets, and the whole claim behind that picture is that every character in it
//! is real output over the committed `screenshots/fixture/` and
//! `screenshots/change/` trees. These journeys run the real
//! `scripts/pr-comment-body.sh` — real `git`, a real throwaway review
//! repository, the real compiled binary — and assert the three things the
//! picture has to show: a heading counting additions apart from justification
//! edits, an entry marked as an edit, and the provenance stamp.
//!
//! Splitting the capture at the body is what makes that drivable here. What the
//! renderer does with the body needs a headless browser, deliberately outside
//! `just bootstrap` and the gate (screenshots/AGENTS.md), so its happy path is
//! proven by a maintainer running the recipe and committing the PNGs — the same
//! bargain `js_tools_setup.rs` records. What that never reaches is the recovery
//! advice, so both recipes are driven here through `just`, with the tool they
//! need taken off `PATH`.
//!
//! Unix only: the capture is a POSIX-shell surface a maintainer runs, and the
//! `PATH`-stripping journeys below have no Windows analogue.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Output};

use crate::support::{bash_program, repo_root};

/// Run one of the capture scripts with the compiled binary this suite already
/// built, rather than a release build the gate has no reason to pay for — what
/// is under test is the review case, not the profile it was compiled with.
fn capture(script: &str) -> Command {
    let mut command = Command::new(bash_program());
    command
        .arg(repo_root().join("scripts").join(script))
        .current_dir(repo_root())
        .env("NOTIGNORED_BIN", assert_cmd::cargo::cargo_bin("notignored"));
    command
}

/// A `just` recipe, run through the real command surface with a `PATH` that
/// carries a shell and nothing else this repository installs — which is what a
/// contributor who has never run the capture before has.
///
/// `just` itself is resolved absolutely and left out of that `PATH`, so the
/// recipe under test is the one this checkout defines.
fn recipe_without_tools(name: &str) -> Output {
    let mut command = Command::new(just_binary());
    command
        .arg(name)
        .current_dir(repo_root())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap_or_else(|error| panic!("run `just {name}`: {error}"))
}

/// The `just` this checkout is driven by.
fn just_binary() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("just"))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| {
            panic!(
                "no `just` on PATH\n\
                 ACTION: install just (https://just.systems) — it is this \
                 repository's whole command surface, so it has to be drivable here too"
            )
        })
}

/// The commit `scripts/pr-comment-body.sh` pins its permalinks to.
///
/// Read from the script rather than spelled out here: `tests/screenshots_contract.rs`
/// already holds it against the `pr-comment` scene's, and a second literal would
/// be a third place for it to drift.
fn permalink_sha() -> String {
    let script = std::fs::read_to_string(repo_root().join("scripts/pr-comment-body.sh"))
        .expect("read scripts/pr-comment-body.sh");
    script
        .lines()
        .find_map(|line| line.trim().strip_prefix("permalink_sha="))
        .map(|value| value.trim().trim_matches('"').to_string())
        .expect("the script pins a permalink_sha")
}

fn succeeds(output: &Output, what: &str) -> String {
    assert!(
        output.status.success(),
        "{what} failed ({}):\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).expect("the body is UTF-8")
}

/// The picture's three claims, made by the binary rather than by the capture.
///
/// A change overlay that only *added* suppressions would photograph a comment
/// that cannot show the distinction the README's passage sells, and nothing else
/// in the repository would notice — the PNGs are not hash-gated.
#[test]
fn the_review_case_shows_additions_apart_from_justification_edits() {
    let output = capture("pr-comment-body.sh")
        .output()
        .expect("run scripts/pr-comment-body.sh");
    let body = succeeds(&output, "scripts/pr-comment-body.sh");

    assert!(
        body.starts_with("<!-- notignored-report -->"),
        "the body no longer opens with the sticky marker:\n{body}"
    );
    let heading = body
        .lines()
        .find(|line| line.starts_with("### notignored"))
        .unwrap_or_else(|| panic!("the body has no heading:\n{body}"));
    assert!(
        heading.contains("added") && heading.contains("justification edited"),
        "the heading no longer counts additions apart from edits: {heading}"
    );
    assert!(
        body.contains("_(justification edited)_"),
        "no entry is marked as a justification edit, so the picture cannot show \
         one:\n{body}"
    );
    assert_eq!(
        body.matches("_(justification edited)_").count(),
        1,
        "the change overlay stopped being one in-place edit among additions:\n{body}"
    );
    assert!(
        body.contains("<summary>suppressed code</summary>"),
        "no snippet to expand in the picture:\n{body}"
    );

    // The provenance stamp, pinned to the sha the script and the `pr-comment`
    // scene share.
    let sha = permalink_sha();
    assert!(
        body.contains(&format!("Suppressions as of [`{}`]", &sha[..7])),
        "the footer no longer stamps the commit the suppressions were read \
         from:\n{body}"
    );
    assert!(
        body.contains(&format!("/blob/{sha}/src/api_client.py#L3")),
        "the permalinks no longer point at the pinned commit:\n{body}"
    );
}

/// What a contributor without the render toolchain sees.
///
/// The toolchain is a browser download, deliberately outside `just bootstrap`,
/// so "not installed" is the *normal* first encounter with this recipe — and a
/// capture that failed with a bare `command not found` would send them looking
/// in the wrong place.
#[test]
fn the_capture_names_what_to_install_when_node_is_missing() {
    let output = recipe_without_tools("screenshots-pr-comment");
    assert!(
        !output.status.success(),
        "`just screenshots-pr-comment` claimed to succeed with no node on PATH"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("node not found") && stderr.contains("nodejs.org"),
        "the capture does not say what to install:\n{stderr}"
    );
}

/// And what its installer says, for the same reason: a contributor reaching for
/// the toolchain directly is one who already knows they are missing it.
#[test]
fn the_installer_names_what_to_install_when_npm_is_missing() {
    let output = recipe_without_tools("screenshots-comment-tools");
    assert!(
        !output.status.success(),
        "`just screenshots-comment-tools` claimed to succeed with no npm on PATH"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("npm not found") && stderr.contains("nodejs.org"),
        "the installer does not say what to install:\n{stderr}"
    );
}
