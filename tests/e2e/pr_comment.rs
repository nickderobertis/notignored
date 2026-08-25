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

/// This host's `PATH`, mirrored into one scratch directory with Node.js taken
/// out of it — which is what a contributor who has never run the capture has.
///
/// Mirrored rather than replaced by a short allowlist: `just` evaluates the
/// justfile's backtick variables before it runs any recipe, and the recipes
/// themselves are shell, so a hand-picked `PATH` fails on whichever tool the
/// justfile reaches for next rather than on the one the journey is about. Every
/// link points at the real program; the only thing invented here is the absence.
fn path_without_node() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a scratch PATH directory");
    for entry in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let Ok(listing) = std::fs::read_dir(&entry) else {
            continue;
        };
        for item in listing.flatten() {
            let name = item.file_name();
            if ["node", "npm", "npx"].iter().any(|hidden| name == **hidden) {
                continue;
            }
            let link = dir.path().join(&name);
            // Earlier `PATH` entries win, the way `PATH` itself resolves them.
            if link.symlink_metadata().is_err() {
                let _ = std::os::unix::fs::symlink(item.path(), link);
            }
        }
    }
    dir
}

/// A `just` recipe, run through the real command surface on that `PATH`.
///
/// `just` itself is resolved absolutely and left off it, so the recipe under
/// test is the one this checkout defines rather than one a mirror link found.
fn recipe_without_node(name: &str) -> Output {
    let path = path_without_node();
    Command::new(just_binary())
        .arg(name)
        .current_dir(repo_root())
        .env("PATH", path.path())
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
    let sha = script
        .lines()
        .find_map(|line| line.trim().strip_prefix("permalink_sha="))
        .map(|value| value.trim().trim_matches('"').to_string())
        .expect("the script pins a permalink_sha");
    // Abbreviated below, so a value read off disk that is not a commit id has to
    // fail here rather than by slicing.
    assert!(
        sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
        "scripts/pr-comment-body.sh pins a permalink_sha that is not a commit id: {sha:?}"
    );
    sha
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
    let output = recipe_without_node("screenshots-pr-comment");
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

/// A scratch directory laid out like the repo root, with the **real** installer
/// linked in — the shape `js_tools_setup.rs` uses, for the same reason: the
/// script resolves its root from its own path, so a controlled layout is what
/// reaches a branch the real checkout never takes.
fn installer_sandbox(with_manifest: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("scratch repo root");
    let scripts = dir.path().join("scripts");
    std::fs::create_dir_all(&scripts).expect("scripts dir");
    std::os::unix::fs::symlink(
        repo_root().join("scripts/setup-comment-render.sh"),
        scripts.join("setup-comment-render.sh"),
    )
    .expect("link the real script");
    if with_manifest {
        let manifest = dir.path().join("scripts/comment-render");
        std::fs::create_dir_all(&manifest).expect("manifest dir");
        for name in ["package.json", "package-lock.json"] {
            std::fs::copy(
                repo_root().join("scripts/comment-render").join(name),
                manifest.join(name),
            )
            .expect("copy the pinned manifest");
        }
    }
    dir
}

/// With the pinned manifest gone there is nothing to install from, and the
/// message has to name the manifest rather than the copy that failed.
#[test]
fn the_installer_names_the_manifest_it_cannot_find() {
    let sandbox = installer_sandbox(false);
    let output = Command::new(bash_program())
        .arg(sandbox.path().join("scripts/setup-comment-render.sh"))
        .output()
        .expect("run the real installer in a sandbox");
    assert!(
        !output.status.success(),
        "the installer claimed to succeed with no pinned manifest"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scripts/comment-render/package.json") && stderr.contains("ACTION:"),
        "the installer does not name the manifest it needs:\n{stderr}"
    );
}

/// The review repository is built in a scratch directory. With nowhere to put
/// one, the capture has to say so rather than fail inside `cp`.
#[test]
fn the_body_script_says_where_its_scratch_directory_should_have_gone() {
    let output = capture("pr-comment-body.sh")
        .env("TMPDIR", repo_root().join("target/no-such-tmpdir"))
        .output()
        .expect("run scripts/pr-comment-body.sh");
    assert!(
        !output.status.success(),
        "the body script claimed to succeed with an unusable TMPDIR"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scratch directory") && stderr.contains("TMPDIR"),
        "the body script does not name the directory it could not create:\n{stderr}"
    );
}

/// The script does not build — the recipe does, the way `screenshots-gif` does —
/// so with no binary there it has to point at the recipe rather than fail inside
/// a subshell.
#[test]
fn the_body_script_points_at_the_recipe_that_builds_the_binary() {
    let output = capture("pr-comment-body.sh")
        .env("NOTIGNORED_BIN", repo_root().join("target/no-such-binary"))
        .output()
        .expect("run scripts/pr-comment-body.sh");
    assert!(
        !output.status.success(),
        "the body script claimed to succeed with no binary to drive"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no notignored binary at")
            && stderr.contains("just screenshots-pr-comment"),
        "the body script does not say how to get a binary:\n{stderr}"
    );
}

/// And what its installer says, for the same reason: a contributor reaching for
/// the toolchain directly is one who already knows they are missing it.
#[test]
fn the_installer_names_what_to_install_when_npm_is_missing() {
    let output = recipe_without_node("screenshots-comment-tools");
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
