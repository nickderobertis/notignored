//! The comment renderer as a **library** caller reaches it.
//!
//! `notignored::cli::render_markdown` is public API, so its `MarkdownOptions`
//! arrive from callers who never passed through `--github-repo` and
//! `--github-sha`. The binary journeys in `tests/e2e/markdown.rs` cover
//! everything argv can produce; this covers what only a library caller can — a
//! value those flags would have rejected — through the same public function the
//! CLI itself calls.

use notignored::cli::{render_markdown, MarkdownOptions};
use notignored::{IgnoreDirective, Report, Scope, Suppressed, Tool};

/// A commit id the flags accept, and its abbreviation.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// One directive, so a body has an entry whose location is either a permalink
/// or plain text.
fn report() -> Report {
    let mut report = Report::new();
    report.ignores.push(IgnoreDirective {
        tool: Tool::Ruff,
        scope: Scope::Line,
        rules: vec!["E501".into()],
        reason: Some("the vendor URL cannot be wrapped".into()),
        path: "src/app.py".into(),
        line: 12,
        end_line: 12,
        column: 20,
        raw: "# noqa: E501".into(),
        suppressed: Suppressed {
            start_line: 12,
            end_line: Some(12),
        },
        change: None,
    });
    report
}

fn body(repo: Option<&str>, sha: Option<&str>) -> String {
    let options = MarkdownOptions {
        repo: repo.map(str::to_string),
        sha: sha.map(str::to_string),
        ..MarkdownOptions::default()
    };
    let mut out = Vec::new();
    render_markdown(&report(), &options, &mut out).expect("render the comment body");
    String::from_utf8(out).expect("a UTF-8 comment body")
}

/// The values the CLI would have accepted render the body the action posts:
/// every location a permalink, and the footer naming the commit it was read
/// from.
#[test]
fn the_values_the_flags_accept_link_the_entries_and_stamp_the_body() {
    let rendered = body(Some("acme/widgets"), Some(SHA));
    assert!(
        rendered.contains(&format!(
            "[src/app.py:12](https://github.com/acme/widgets/blob/{SHA}/src/app.py#L12)"
        )),
        "{rendered}"
    );
    assert!(
        rendered.ends_with(&format!(
            "---\n\n<sub>Suppressions as of [`0123456`](https://github.com/acme/widgets/commit/{SHA}).</sub>\n"
        )),
        "{rendered}"
    );
}

/// A repository slug the flags would have rejected reaches no URL.
///
/// The body degrades to the plain-text locations a run told nothing renders,
/// and the footer to the unlinked form — the sha is still a commit, so the
/// stamp is still true.
#[test]
fn a_repository_the_flags_would_reject_is_interpolated_into_nothing() {
    let rendered = body(Some("acme/widgets)](https://evil.example"), Some(SHA));
    assert!(!rendered.contains("evil.example"), "{rendered}");
    assert!(!rendered.contains("https://github.com/"), "{rendered}");
    assert!(rendered.contains("— `src/app.py:12`"), "{rendered}");
    assert!(
        rendered.ends_with("---\n\n<sub>Suppressions as of `0123456`.</sub>\n"),
        "{rendered}"
    );
}

/// A revision that is not a commit id names no commit, so there is nothing
/// truthful to stamp and nothing to pin a permalink to.
#[test]
fn a_sha_the_flags_would_reject_leaves_the_body_unstamped_and_unlinked() {
    for moving in ["main", "0123", "refs/heads/main"] {
        let rendered = body(Some("acme/widgets"), Some(moving));
        assert!(!rendered.contains("https://github.com/"), "{rendered}");
        assert!(!rendered.contains("<sub>"), "{rendered}");
        assert!(rendered.contains("— `src/app.py:12`"), "{rendered}");
    }
}
