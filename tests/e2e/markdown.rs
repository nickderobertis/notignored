//! The pull-request comment body, rendered by the real binary.
//!
//! This is what the GitHub Action posts, so it is proven the same way the JSON
//! report is: real files in, a checked-in golden out. 0 is the all-clear body
//! and every other count carries one entry per suppression, each with the
//! collapsed snippet of the code it silences.
//!
//! The entry cap is not reachable from these fixtures — twenty findings would be
//! twenty files — so it is proven in `src/cli/markdown.rs`'s unit tests, at the
//! counts either side of the boundary.
//!
//! Re-bless with `just bless` after reviewing the diff.

use std::fs;

use crate::support::{commit, git, git_repo, notignored, repo_root, write};

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

/// Every listed entry carries its own collapsed snippet — the rule that
/// replaced the old below-four-findings-only one — and the fixtures are all
/// readable, so none of them takes the degrade path.
#[test]
fn a_collapsed_snippet_accompanies_every_entry() {
    for count in [0, 1, 3, 4, 5] {
        let body = body_for(count);
        for (tag, expected) in [
            ("  <details>\n", count),
            ("  <summary>suppressed code</summary>\n", count),
            ("  </details>\n", count),
            ("\n  ```", count * 2),
        ] {
            assert_eq!(
                body.matches(tag).count(),
                expected,
                "the body for {count} findings carries the wrong number of {tag:?}:\n{body}"
            );
        }
    }
}

/// The snippet is there so a reviewer reads the code a directive silences.
///
/// A directive that has its line to itself silences the line **below** it, and
/// this is the journey where that has to be true: the dogfood comment on this
/// repo's own PR #26 quoted the directive back instead, which tells a reviewer
/// nothing they could judge. Scoped to llmlint, whose `ignore` is the form that
/// was wrong; the fixture is a Rust file, because the directive is hosted in
/// whatever comment syntax it lands in.
#[test]
fn a_directive_alone_on_its_line_shows_the_code_below_it_not_itself() {
    let output = notignored(&repo_root())
        .args(["--tool", "llmlint"])
        .arg("tests/fixtures/markdown/coordinates.rs")
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
    let body = String::from_utf8(output.stdout).expect("a UTF-8 comment body");

    assert!(body.contains("### notignored: 1 suppression\n"), "{body}");
    let snippet = body
        .split_once("```rust\n")
        .and_then(|(_, rest)| rest.split_once("\n  ```"))
        .map(|(snippet, _)| snippet)
        .unwrap_or_else(|| panic!("no fenced snippet in the comment body:\n{body}"));
    let marked: Vec<&str> = snippet
        .lines()
        .filter(|line| line.starts_with("  > "))
        .collect();
    assert_eq!(
        marked,
        vec!["  > 2 | #[derive(Debug, Clone, PartialEq, Eq)]"],
        "the marker does not sit on the line the directive silences:\n{body}"
    );
    // The directive's own line is now readable *as context*, unmarked — the
    // keyword is never written out here; see `src/tools/llmlint.rs`.
    for line in snippet.lines() {
        assert_eq!(
            line.contains(notignored::tools::llmlint::KEYWORD),
            line.starts_with("    1 | "),
            "the directive is quoted as something other than context:\n{body}"
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

/// The cap the action passes through, driven end to end: five findings under a
/// cap of two list two entries and close with a line naming the rest.
#[test]
fn max_entries_bounds_the_listed_entries_and_names_what_it_left_out() {
    let mut command = notignored(&repo_root());
    for tool in TOOLS {
        command.args(["--tool", tool]);
    }
    let output = command
        .args(FIXTURES.iter().map(|(path, _)| *path))
        .args(["--format", "markdown", "--max-entries", "2"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);
    let body = String::from_utf8(output.stdout).expect("a UTF-8 comment body");

    assert!(body.contains("### notignored: 5 suppressions\n"), "{body}");
    assert_eq!(body.matches("\n- **").count(), 2, "{body}");
    assert!(
        body.ends_with("_… and 3 more not shown (5 total)._\n"),
        "{body}"
    );
}

/// A cap that would render a different body than the caller asked for stops the
/// run: the action passes a workflow input straight into this flag.
#[test]
fn a_max_entries_that_is_not_a_positive_number_is_rejected() {
    for bad in ["0", "twenty", "1.5"] {
        let output = notignored(&repo_root())
            .args(["tests/fixtures/markdown/single.py", "--format", "markdown"])
            .args(["--max-entries", bad])
            .output()
            .expect("run notignored");
        assert_eq!(
            output.status.code(),
            Some(2),
            "--max-entries {bad} was accepted"
        );
        assert!(
            output.stdout.is_empty(),
            "stdout should stay clean on error"
        );
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

/// The heading a reviewer reads first, in each of the four shapes a report can
/// arrive in — rendered by the real binary over a real repository, because the
/// heading is the one line of this comment that can lie about a pull request.
///
/// The fourth shape is why this exists: a change that rewrote justifications
/// and added nothing must announce exactly that, and say "added" nowhere.
#[test]
fn the_comment_heading_counts_what_the_change_actually_did() {
    let repo = git_repo();
    let root = repo.path();
    write(
        root,
        "kept.py",
        "x = 1  # noqa: E501  # the SDK builds this path\n",
    );
    write(root, "quiet.py", "y = 2\n");
    commit(root, "baseline");

    let body = |args: &[&str]| -> String {
        let output = notignored(root)
            .args(args)
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
    };
    let diff = ["--diff", "--diff-base", "main"];

    // Nothing found is nothing to count by kind: a `--diff` run over a change
    // that touched no directive renders the all-clear body it always rendered.
    assert_eq!(
        body(&diff),
        format!(
            "{}\n\n### notignored\n\nNo lint or type-check suppressions found.\n",
            notignored::cli::MARKER
        )
    );

    // Added only, singular and plural.
    git(root, &["checkout", "-q", "-b", "added-one"]);
    write(
        root,
        "quiet.py",
        "y = 2  # noqa: F401  # imported for its side effects\n",
    );
    commit(root, "add one");
    assert!(
        body(&diff).contains("### notignored: 1 suppression added\n"),
        "{}",
        body(&diff)
    );
    write(root, "quiet.py", "y = 2  # noqa: F401  # imported for its side effects\nz = 3  # noqa: E501  # the SDK builds this path\n");
    commit(root, "add another");
    assert!(
        body(&diff).contains("### notignored: 2 suppressions added\n"),
        "{}",
        body(&diff)
    );

    // Added and rejustified together: both counts, and only the rejustified
    // entry carries the marker.
    write(
        root,
        "kept.py",
        "x = 1  # noqa: E501  # the SDK builds this path, not us\n",
    );
    commit(root, "reword the inherited one");
    let both = body(&diff);
    assert!(
        both.contains("### notignored: 2 suppressions added, 1 justification edited\n"),
        "{both}"
    );
    assert!(
        both.contains(&format!(
            "- **ruff E501** _(justification edited)_ — _the SDK builds this path, not us_ — [kept.py:1](https://github.com/{REPO}/blob/{SHA}/kept.py#L1)\n"
        )),
        "{both}"
    );
    assert!(
        both.contains(&format!(
            "- **ruff F401** — _imported for its side effects_ — [quiet.py:1](https://github.com/{REPO}/blob/{SHA}/quiet.py#L1)\n"
        )),
        "an added entry gained a marker:\n{both}"
    );
    assert_eq!(
        both.matches("_(justification edited)_").count(),
        1,
        "{both}"
    );

    // Rejustified only: the pull request that this whole distinction exists
    // for. A justification rewritten, no suppression added, and the word
    // "added" nowhere in the body.
    git(root, &["checkout", "-q", "main"]);
    git(root, &["checkout", "-q", "-b", "reword-only"]);
    write(
        root,
        "kept.py",
        "x = 1  # noqa: E501  # the SDK builds this path, we do not\n",
    );
    write(
        root,
        "quiet.py",
        "y = 2  # noqa: F401  # re-exported on purpose\n",
    );
    commit(root, "add one, to reword it next");
    write(
        root,
        "quiet.py",
        "y = 2  # noqa: F401  # re-exported so callers can configure retries\n",
    );
    commit(root, "reword it");
    let edited = body(&["--diff", "--diff-base", "HEAD~1"]);
    assert!(
        edited.contains("### notignored: 1 justification edited\n"),
        "{edited}"
    );
    assert!(!edited.contains("added"), "{edited}");

    let both_reworded = body(&diff);
    assert!(
        both_reworded.contains("### notignored: 1 suppression added, 1 justification edited\n"),
        "{both_reworded}"
    );
}
