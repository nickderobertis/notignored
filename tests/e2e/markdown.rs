//! The pull-request comment body, rendered by the real binary.
//!
//! This is what the GitHub Action posts, so it is proven the same way the JSON
//! report is: real files in, a checked-in golden out. The counts below are the
//! ones the rendering rules turn on — 0 is the all-clear body, 1 and 3 carry
//! source snippets, 4 and 5 are past the limit and carry only permalinks.
//!
//! Re-bless with `just bless` after reviewing the diff.

use std::fs;

use crate::support::{notignored, repo_root};

/// The repo and commit the goldens' permalinks are built from. Fixed, so a
/// golden body is a byte-stable artifact rather than a function of the checkout.
const REPO: &str = "acme/widgets";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// The tools the fixtures below are written in. Scoping the run keeps the
/// llmlint `ignore-file` footer a reason-less fixture must carry from
/// counting as one more finding and shifting every golden by a file.
const TOOLS: [&str; 4] = ["eslint", "ruff", "rust", "typescript"];

/// Fixture files, in the order that grows the report one finding at a time.
///
/// Every count the rules distinguish is reached by adding a file rather than by
/// editing one, so the same fixtures back every golden.
const FIXTURES: [(&str, usize); 5] = [
    ("tests/fixtures/markdown/clean.py", 0),
    ("tests/fixtures/markdown/single.py", 1),
    ("tests/fixtures/markdown/pair.ts", 2),
    ("tests/fixtures/markdown/blanket.py", 1),
    ("tests/fixtures/markdown/guard.rs", 1),
];

/// Render the first `files` fixtures as a comment body, with the permalink flags
/// the action passes.
fn render(files: usize) -> String {
    let mut command = notignored(&repo_root());
    for tool in TOOLS {
        command.args(["--tool", tool]);
    }
    let output = command
        .args(FIXTURES.iter().take(files).map(|(path, _)| *path))
        .args(["--format", "markdown"])
        .args(["--github-repo", REPO, "--github-sha", SHA])
        .output()
        .expect("run notignored");
    assert!(
        output.status.success(),
        "exit: {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("a UTF-8 comment body")
}

/// The body for the fixture prefix that adds up to `count` findings.
fn body_for(count: usize) -> String {
    let mut running = 0;
    let files = FIXTURES
        .iter()
        .position(|(_, findings)| {
            running += findings;
            running == count
        })
        .unwrap_or_else(|| panic!("no fixture prefix adds up to {count} findings"))
        + 1;
    render(files)
}

#[test]
fn golden_comment_bodies_render_as_checked_in() {
    for count in [0, 1, 3, 4, 5] {
        let actual = body_for(count);
        let path = repo_root().join(format!("tests/golden/markdown/count-{count}.md"));
        if std::env::var_os("NOTIGNORED_BLESS").is_some() {
            fs::create_dir_all(path.parent().expect("a golden directory"))
                .expect("create the golden directory");
            fs::write(&path, &actual).expect("write the golden body");
        }
        let expected = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        assert_eq!(
            actual,
            expected,
            "the rendered comment body for {count} findings changed. If the change is \
             intended, re-run with NOTIGNORED_BLESS=1 ({}).",
            path.display()
        );
    }
}

/// The three rules a reviewer reads the comment for, asserted on the goldens
/// themselves so a re-bless cannot quietly drop one.
#[test]
fn every_body_opens_with_the_marker_the_action_upserts_by() {
    for count in [0, 1, 3, 4, 5] {
        let body = body_for(count);
        assert!(
            body.starts_with(notignored::cli::MARKER),
            "the body for {count} findings does not open with the sticky marker:\n{body}"
        );
    }
}

#[test]
fn a_snippet_accompanies_every_entry_below_the_limit_and_none_above_it() {
    for (count, snippets) in [(0, 0), (1, 1), (3, 3), (4, 0), (5, 0)] {
        let body = body_for(count);
        assert_eq!(
            body.matches("\n  ```").count(),
            snippets * 2,
            "the body for {count} findings carries the wrong number of snippets:\n{body}"
        );
    }
}

#[test]
fn every_entry_links_to_its_line_in_the_named_commit() {
    let body = body_for(5);
    for (path, line) in [
        ("tests/fixtures/markdown/blanket.py", 1),
        ("tests/fixtures/markdown/guard.rs", 1),
        ("tests/fixtures/markdown/pair.ts", 1),
        ("tests/fixtures/markdown/pair.ts", 8),
        ("tests/fixtures/markdown/single.py", 5),
    ] {
        let permalink = format!("https://github.com/{REPO}/blob/{SHA}/{path}#L{line}");
        assert!(body.contains(&permalink), "{permalink} is missing:\n{body}");
    }
}

/// The permalink flags are a trust boundary: they are interpolated into a URL a
/// reviewer is invited to click.
#[test]
fn a_repo_or_sha_that_could_not_build_a_permalink_is_rejected() {
    for (flag, value) in [
        ("--github-repo", "https://evil.example/acme/widgets"),
        ("--github-repo", "widgets"),
        ("--github-sha", "main"),
    ] {
        let output = notignored(&repo_root())
            .args(["tests/fixtures/markdown/single.py", "--format", "markdown"])
            .args([flag, value])
            .output()
            .expect("run notignored");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{flag} {value} was accepted: {:?}",
            output.status
        );
        assert!(
            output.stdout.is_empty(),
            "stdout should stay clean on error"
        );
    }
}

/// Without them the body still renders — a local `notignored --format markdown`
/// is a preview, not a broken run.
#[test]
fn without_the_permalink_flags_locations_render_as_plain_text() {
    let output = notignored(&repo_root())
        .args(["tests/fixtures/markdown/single.py", "--format", "markdown"])
        .output()
        .expect("run notignored");
    assert!(output.status.success());
    let body = String::from_utf8(output.stdout).unwrap();
    assert!(
        body.contains("— `tests/fixtures/markdown/single.py:5`"),
        "{body}"
    );
    assert!(!body.contains("https://github.com/"), "{body}");
}
