//! Rendering a [`Report`] as a pull-request comment body.
//!
//! This is the shape the GitHub Action posts: the logic lives here, in Rust,
//! rather than in the composite's shell, so it is testable against golden bodies
//! instead of only observable in a live comment.
//!
//! The body opens with a hidden [`MARKER`] comment. That marker is how the
//! action finds its own comment again and edits it in place, so exactly one
//! sticky comment accumulates on a pull request rather than one per push.

use std::io::{self, Write};
use std::path::Path;

use super::render::ChangeCounts;
use super::{github_repo, github_sha};
use crate::model::{Change, IgnoreDirective, Report, Scope};
use crate::source::Language;

/// The hidden marker every rendered body starts with.
///
/// `scripts/action/comment.sh` searches for this exact string; the drift between
/// the two is gated by `tests/action_contract.rs`.
pub const MARKER: &str = "<!-- notignored-report -->";

/// Entries a body lists before it stops and counts the rest.
///
/// A pull request that adds a hundred suppressions must not become a comment a
/// hundred entries long — the reviewer scrolls past the diff to reach it. The
/// closing line still names the total, so nothing is hidden, only unlisted.
pub const DEFAULT_MAX_ENTRIES: u32 = 20;

/// Lines of suppressed source a snippet shows.
///
/// Not configurable: the snippet is a glance at what a directive silences, and a
/// block long enough to need its own scrollbar has stopped being one — a reader
/// who wants the rest follows the permalink.
const SNIPPET_LINES: u32 = 10;

/// Lines of unsuppressed source shown either side of a **single** suppressed
/// line.
///
/// One line on its own is not enough to judge — the reviewer cannot see which
/// function or branch it sits in. A span of several lines already carries that
/// context, so it gets none added and stays capped by [`SNIPPET_LINES`].
const SNIPPET_CONTEXT: u32 = 2;

/// Characters of the commit id the provenance footer shows.
///
/// The abbreviation every GitHub surface uses, and the one a reviewer compares
/// against the checks list without reading 40 hex digits. The link behind it
/// carries the sha in full.
const SHORT_SHA: usize = 7;

/// The gutter each snippet line opens with: the suppressed line is marked, the
/// context around it is aligned with spaces so the two cannot be confused.
const SUPPRESSED_GUTTER: &str = "> ";
const CONTEXT_GUTTER: &str = "  ";

/// What the permalinks in a rendered body point at, and how much of the report
/// it lists.
///
/// Both permalink parts are needed to build one; with either missing the
/// location renders as plain `path:line` text, so a run without them still
/// produces a usable body.
// llmlint: ignore[invalid_states_unrepresentable] every field is validated where it enters the process — clap's `github_repo` and `github_sha` value parsers reject anything but an owner/repo slug and a hex commit id, and `--max-entries` is a ranged parser that rejects zero, at the trust boundary the invariants call for — and this struct mirrors `Cli`'s public fields one for one, so a newtype here would move the crate's public surface without adding a check that is not already made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownOptions {
    /// `owner/repo` the permalinks address.
    pub repo: Option<String>,
    /// Commit the permalinks pin the source to.
    pub sha: Option<String>,
    /// Most entries the body lists before summarizing the remainder.
    pub max_entries: u32,
}

/// The defaults a `--format markdown` run gets when it is told nothing: no
/// permalinks, and the standard cap.
impl Default for MarkdownOptions {
    fn default() -> Self {
        MarkdownOptions {
            repo: None,
            sha: None,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }
}

impl MarkdownOptions {
    /// The commit these options may name, or `None` when there is none to name
    /// or the one given could not have come from `--github-sha`.
    ///
    /// [`render_markdown`] is public, so these two strings also reach the
    /// renderer from a library caller, who never passed through the flags' value
    /// parsers. Those parsers *are* the definition of what may be interpolated
    /// into a github.com URL, so they are the check here too rather than a
    /// second spelling of one: a value the command line would have rejected is
    /// written into no link and no body, and the run renders exactly as one that
    /// was told nothing does.
    fn commit(&self) -> Option<&str> {
        let sha = self.sha.as_deref()?;
        github_sha(sha).is_ok().then_some(sha)
    }

    /// The `owner/repo` these options may link to, under the same rule.
    fn repository(&self) -> Option<&str> {
        let repo = self.repo.as_deref()?;
        github_repo(repo).is_ok().then_some(repo)
    }

    /// The `https://github.com/<owner>/<repo>/blob/<sha>/<path>#L<line>`
    /// permalink for a directive, or `None` when this run was not told where the
    /// source lives.
    fn permalink(&self, directive: &IgnoreDirective) -> Option<String> {
        let (repo, sha) = (self.repository()?, self.commit()?);
        Some(format!(
            "https://github.com/{repo}/blob/{sha}/{}#L{}",
            encode_path(&directive.path),
            directive.line
        ))
    }
}

/// Where a snippet's source lines come from.
///
/// Reports name paths relative to the invocation directory, which is where the
/// scan read them from a moment earlier; a file that has since become unreadable
/// simply renders without its snippet.
trait SnippetSource {
    /// The lines of `path`, or `None` when it cannot be read.
    fn lines(&self, path: &str) -> Option<Vec<String>>;
}

/// The real filesystem.
struct Files;

impl SnippetSource for Files {
    fn lines(&self, path: &str) -> Option<Vec<String>> {
        let text = std::fs::read_to_string(path).ok()?;
        Some(text.lines().map(str::to_string).collect())
    }
}

/// Write the report as a pull-request comment body.
pub fn render_markdown(
    report: &Report,
    options: &MarkdownOptions,
    out: &mut dyn Write,
) -> io::Result<()> {
    out.write_all(body(report, options, &Files).as_bytes())?;
    out.flush()
}

/// The whole comment body, marker included.
fn body(report: &Report, options: &MarkdownOptions, source: &dyn SnippetSource) -> String {
    let mut body = format!("{MARKER}\n\n");
    if report.ignores.is_empty() {
        body.push_str("### notignored\n\nNo lint or type-check suppressions found.\n");
    } else {
        let count = report.ignores.len();
        body.push_str(&format!("### notignored: {}\n\n", heading(report)));
        let listed = usize::try_from(options.max_entries).unwrap_or(usize::MAX);
        for directive in report.ignores.iter().take(listed) {
            body.push_str(&entry(directive, options));
            if let Some(snippet) = snippet(directive, source) {
                body.push('\n');
                body.push_str(&snippet);
                body.push('\n');
            }
        }
        let omitted = count.saturating_sub(listed);
        if omitted > 0 {
            separate(&mut body);
            body.push_str(&format!(
                "_… and {omitted} more not shown ({count} total)._\n"
            ));
        }
    }
    if !report.errors.is_empty() {
        separate(&mut body);
        body.push_str("#### Could not be scanned\n\n");
        for error in &report.errors {
            body.push_str(&format!(
                "- `{}` — {}\n",
                error.path,
                escape(&error.message)
            ));
        }
    }
    if let Some(stamp) = stamp(options) {
        separate(&mut body);
        body.push_str(&stamp);
    }
    body
}

/// The provenance footer: the commit the suppressions above were read from.
///
/// The action upserts one comment and edits it in place across pushes, so
/// without this nothing in the body says which tree it describes and a reviewer
/// reading it after a push cannot tell a current comment from a stale one.
///
/// `None` when the run was not told a sha: a local `notignored --format
/// markdown` is a preview, and there is nothing truthful to stamp. Without a
/// repository the id renders unlinked, exactly as a permalink cannot be built.
fn stamp(options: &MarkdownOptions) -> Option<String> {
    let sha = options.commit()?;
    // A commit id is 7 to 64 hex digits — what `commit` above accepted — so the
    // abbreviation is whole and neither form has anything to escape.
    let short: String = sha.chars().take(SHORT_SHA).collect();
    let commit = match options.repository() {
        Some(repo) => format!("[`{short}`](https://github.com/{repo}/commit/{sha})"),
        None => format!("`{short}`"),
    };
    // `<sub>` rather than italics: GitHub renders it small and grey, which is
    // what keeps a provenance footer out of the reader's way.
    Some(format!("---\n\n<sub>Suppressions as of {commit}.</sub>\n"))
}

/// What the heading counts, in the words the count is true in.
///
/// A run without `--diff` cannot say what a change did, so it counts
/// suppressions and stops — the line it always printed. A classified run names
/// each kind it has to report and leaves out the kind it does not, because the
/// first thing a reviewer reads must not tell a pull request that rewrote two
/// justifications that it added two suppressions.
fn heading(report: &Report) -> String {
    let Some(counts) = ChangeCounts::of(report) else {
        return plural(report.ignores.len(), "suppression", "suppressions");
    };
    let added = format!(
        "{} added",
        plural(counts.added, "suppression", "suppressions")
    );
    let edited = format!(
        "{} edited",
        plural(
            counts.justification_edited,
            "justification",
            "justifications"
        )
    );
    match (counts.added, counts.justification_edited) {
        (0, _) => edited,
        (_, 0) => added,
        _ => format!("{added}, {edited}"),
    }
}

/// `1 suppression` / `2 suppressions`, and the same for the other noun.
fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// End `body` with exactly one blank line, so what follows starts its own block.
///
/// An entry that carried a snippet already left one behind and an entry that
/// could not read its file did not; without this the two spellings differ by a
/// stray newline that every golden body would have to encode.
fn separate(body: &mut String) {
    if !body.ends_with("\n\n") {
        body.push('\n');
    }
}

/// One directive as a list item: what is silenced, why, and where.
fn entry(directive: &IgnoreDirective, options: &MarkdownOptions) -> String {
    let rules = if directive.rules.is_empty() {
        "(all rules)".to_string()
    } else {
        directive
            .rules
            .iter()
            .map(|rule| escape(rule))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let reason = match &directive.reason {
        Some(reason) => format!("_{}_", escape(reason)),
        None => "_no reason given_".to_string(),
    };
    let location = format!("{}:{}", directive.path, directive.line);
    let location = match options.permalink(directive) {
        Some(url) => format!("[{}]({url})", escape(&location)),
        None => format!("`{location}`"),
    };
    // The marker sits at the head of the item, where the eye lands, rather than
    // inside the italic prose the stated reason already owns.
    let edited = match directive.change {
        Some(Change::JustificationEdited) => " _(justification edited)_",
        _ => "",
    };
    format!(
        "- **{} {rules}**{edited} — {reason} — {location}\n",
        directive.tool
    )
}

/// The 1-based source range a directive silences, clamped to a file of `count`
/// lines.
///
/// `None` when the file cannot hold it — a report rendered somewhere other than
/// where it was produced names lines a file may since have lost, and the snippet
/// must show real source or none.
///
/// A `file`-scope directive silences everything, so its range is the file; the
/// record's own `suppressed` says so too, but only the scope says it is the
/// *whole* file rather than a run that happens to reach the end.
fn suppressed_range(directive: &IgnoreDirective, count: u32) -> Option<(u32, u32)> {
    if count == 0 {
        return None;
    }
    if directive.scope == Scope::File {
        return Some((1, count));
    }
    let first = directive.suppressed.start_line;
    if first == 0 || first > count {
        return None;
    }
    // An unterminated block runs to end-of-file, which is what `None` records.
    let last = directive
        .suppressed
        .end_line
        .unwrap_or(count)
        .clamp(first, count);
    Some((first, last))
}

/// The code the directive silences, as a collapsed `<details>` block under its
/// list item.
///
/// Collapsed by default because the point of the comment is that a reviewer can
/// read it without reading the diff: the context is one click away for every
/// entry rather than inline for a lucky few.
///
/// A span of one line is shown in situ — [`SNIPPET_CONTEXT`] lines either side,
/// clamped to the file, with the suppressed line marked in the gutter. A longer
/// span, and a `file`-scope directive, carry their own context and are shown as
/// they always were.
///
/// `None` when the file cannot be read or does not hold the range the record
/// names — the entry then renders without its snippet rather than failing.
fn snippet(directive: &IgnoreDirective, source: &dyn SnippetSource) -> Option<String> {
    let lines = source.lines(&directive.path)?;
    let count = u32::try_from(lines.len()).unwrap_or(u32::MAX);
    let (first, last) = suppressed_range(directive, count)?;
    let single = first == last && directive.scope != Scope::File;
    let (from, to) = if single {
        (
            first.saturating_sub(SNIPPET_CONTEXT).max(1),
            last.saturating_add(SNIPPET_CONTEXT).min(count),
        )
    } else {
        (first, last.min(first.saturating_add(SNIPPET_LINES - 1)))
    };

    let window = &lines[(from as usize - 1)..to as usize];
    let fence = fence_for(window);
    let width = to.to_string().len();
    let mut block = String::from("  <details>\n  <summary>suppressed code</summary>\n\n");
    if let Some(note) = note(directive, last - first + 1) {
        block.push_str(&format!("  {note}\n\n"));
    }
    block.push_str(&format!("  {fence}{}\n", language_tag(&directive.path)));
    for (offset, text) in window.iter().enumerate() {
        let number = from + u32::try_from(offset).unwrap_or_default();
        let gutter = match (single, number == first) {
            (true, true) => SUPPRESSED_GUTTER,
            (true, false) => CONTEXT_GUTTER,
            (false, _) => "",
        };
        // An empty source line renders as a bare gutter: a trailing space here
        // is whitespace every editor in the repo is configured to strip, which
        // would break the checked-in golden bodies on the next save.
        match text.trim_end().is_empty() {
            true => block.push_str(&format!("  {gutter}{number:>width$} |\n")),
            false => block.push_str(&format!(
                "  {gutter}{number:>width$} | {}\n",
                text.trim_end()
            )),
        }
    }
    block.push_str(&format!("  {fence}\n\n  </details>\n"));
    Some(block)
}

/// What the snippet is not showing, when that changes how it should be read: a
/// range longer than [`SNIPPET_LINES`], or a directive that covers the whole
/// file rather than the lines on screen.
fn note(directive: &IgnoreDirective, span: u32) -> Option<String> {
    let truncated = span > SNIPPET_LINES;
    match (directive.scope == Scope::File, truncated) {
        (true, true) => Some(format!(
            "_the whole file is suppressed; showing its first {SNIPPET_LINES} of {span} lines._"
        )),
        (true, false) => Some("_the whole file is suppressed._".to_string()),
        (false, true) => Some(format!(
            "_showing the first {SNIPPET_LINES} of {span} suppressed lines._"
        )),
        (false, false) => None,
    }
}

/// A fence long enough to hold `window` — source that itself contains a run of
/// backticks would otherwise close the block early.
fn fence_for(window: &[String]) -> String {
    let longest = window
        .iter()
        .flat_map(|line| line.split(|c| c != '`').map(str::len))
        .max()
        .unwrap_or(0);
    "`".repeat(longest.saturating_add(1).max(3))
}

/// The fenced-block language tag for a path, or `""` when we have no grammar for
/// it (which renders as a plain block rather than a mislabelled one).
fn language_tag(path: &str) -> &'static str {
    match Language::from_path(Path::new(path)) {
        Language::Python => "python",
        Language::Rust => "rust",
        Language::JavaScript => "javascript",
        Language::TypeScript => "typescript",
        Language::Shell => "bash",
        Language::Yaml => "yaml",
        Language::Toml => "toml",
        Language::Unknown => "",
    }
}

/// Escape the markdown-significant characters in text taken verbatim from
/// source.
///
/// A reason is whatever its author wrote: an unbalanced `*` or a stray `<span>`
/// would otherwise re-flow — or silently swallow — the rest of the comment.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '\\' | '`' | '*' | '_' | '[' | ']' | '<') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Percent-encode a report path for use in a URL.
///
/// Report paths are `/`-separated and may hold spaces or any other byte a file
/// name allows; the separators have to survive, everything unusual must not.
fn encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::model::{ReportError, Suppressed, Tool};

    /// Source held in memory, so a body can be rendered without touching disk.
    #[derive(Default)]
    struct Stub(BTreeMap<String, Vec<String>>);

    impl Stub {
        fn with(mut self, path: &str, text: &str) -> Self {
            self.0
                .insert(path.to_string(), text.lines().map(str::to_string).collect());
            self
        }
    }

    impl SnippetSource for Stub {
        fn lines(&self, path: &str) -> Option<Vec<String>> {
            self.0.get(path).cloned()
        }
    }

    fn directive(line: u32, reason: Option<&str>) -> IgnoreDirective {
        IgnoreDirective {
            tool: Tool::Ruff,
            scope: Scope::Line,
            rules: vec!["E501".into()],
            reason: reason.map(str::to_string),
            path: "src/app.py".into(),
            line,
            end_line: line,
            column: 20,
            raw: "# noqa: E501".into(),
            suppressed: Suppressed {
                start_line: line,
                end_line: Some(line),
            },
            change: None,
        }
    }

    /// A directive whose `suppressed` range is `start..=end`, for the span rules
    /// — the record, not the directive's own line, is what a snippet shows.
    fn spanning(scope: Scope, start: u32, end: Option<u32>) -> IgnoreDirective {
        IgnoreDirective {
            scope,
            suppressed: Suppressed {
                start_line: start,
                end_line: end,
            },
            ..directive(start, Some("why"))
        }
    }

    /// The provenance footer [`options`] renders, which every body below closes
    /// with. The rules that decide its form are driven end to end in
    /// `tests/e2e/markdown.rs`; here it is only the tail every other assertion
    /// has to reach past.
    const STAMP: &str = "---\n\n<sub>Suppressions as of [`0123456`](https://github.com/acme/widgets/commit/0123456789abcdef0123456789abcdef01234567).</sub>\n";

    fn options() -> MarkdownOptions {
        MarkdownOptions {
            repo: Some("acme/widgets".into()),
            sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
            ..MarkdownOptions::default()
        }
    }

    fn render(report: &Report, options: &MarkdownOptions) -> String {
        body(report, options, &Stub::default())
    }

    #[test]
    fn an_empty_report_renders_the_marker_and_an_all_clear() {
        let rendered = render(&Report::new(), &options());
        assert_eq!(
            rendered,
            format!(
                "{MARKER}\n\n### notignored\n\nNo lint or type-check suppressions found.\n\n{STAMP}"
            )
        );
    }

    #[test]
    fn an_entry_carries_the_tool_rules_reason_and_permalink() {
        let mut report = Report::new();
        report.ignores.push(directive(12, Some("long wrapped URL")));
        let rendered = render(&report, &options());
        assert!(rendered.starts_with(&format!("{MARKER}\n")), "{rendered}");
        assert!(
            rendered.contains("### notignored: 1 suppression\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains(
                "- **ruff E501** — _long wrapped URL_ — [src/app.py:12](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/src/app.py#L12)\n"
            ),
            "{rendered}"
        );
    }

    /// A classified report is rendered as `path`, `line`, `column` order leaves
    /// it: `added` first, then the requested number of rewritten justifications.
    fn classified(added: usize, edited: usize) -> Report {
        let mut report = Report::new();
        let mut line = 0;
        for _ in 0..added {
            line += 1;
            report.ignores.push(IgnoreDirective {
                change: Some(Change::Added),
                ..directive(line, Some("why"))
            });
        }
        for _ in 0..edited {
            line += 1;
            report.ignores.push(IgnoreDirective {
                change: Some(Change::JustificationEdited),
                ..directive(line, Some("why"))
            });
        }
        report
    }

    /// The first thing a reviewer reads, in each of the four shapes a report can
    /// arrive in. The fourth is the one this distinction exists for: a change
    /// that rewrote justifications and added nothing must not announce additions.
    #[test]
    fn the_heading_counts_what_the_report_can_actually_say() {
        for (report, expected) in [
            (classified(2, 0), "### notignored: 2 suppressions added\n"),
            (classified(1, 0), "### notignored: 1 suppression added\n"),
            (
                classified(2, 1),
                "### notignored: 2 suppressions added, 1 justification edited\n",
            ),
            (classified(0, 1), "### notignored: 1 justification edited\n"),
            (
                classified(0, 3),
                "### notignored: 3 justifications edited\n",
            ),
        ] {
            let rendered = render(&report, &options());
            assert!(
                rendered.contains(expected),
                "{expected:?} not in:\n{rendered}"
            );
        }

        // An unclassified report — no `--diff` — counts suppressions and says
        // nothing about a change it has no base to compare against.
        let mut unclassified = Report::new();
        unclassified.ignores.push(directive(1, Some("why")));
        unclassified.ignores.push(directive(2, Some("why")));
        assert!(
            render(&unclassified, &options()).contains("### notignored: 2 suppressions\n"),
            "an unclassified heading changed"
        );
    }

    /// The entry says which kind it is where the eye lands, and an added entry
    /// is the line it always was.
    #[test]
    fn only_a_rewritten_justification_marks_its_entry() {
        let mut report = Report::new();
        report.ignores.push(IgnoreDirective {
            change: Some(Change::JustificationEdited),
            ..directive(12, Some("long wrapped URL"))
        });
        report.ignores.push(IgnoreDirective {
            change: Some(Change::Added),
            ..directive(13, Some("long wrapped URL"))
        });
        let rendered = render(&report, &options());
        assert!(
            rendered.contains(
                "- **ruff E501** _(justification edited)_ — _long wrapped URL_ — [src/app.py:12]"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains("- **ruff E501** — _long wrapped URL_ — [src/app.py:13]"),
            "an added entry gained a marker:\n{rendered}"
        );
    }

    /// Nothing found is nothing to count by kind, whatever the run was: the
    /// all-clear body is the one it always was.
    #[test]
    fn an_empty_classified_report_still_renders_the_all_clear() {
        assert_eq!(
            render(&classified(0, 0), &options()),
            render(&Report::new(), &options())
        );
    }

    #[test]
    fn a_directive_without_a_reason_says_so() {
        let mut report = Report::new();
        report.ignores.push(directive(12, None));
        assert!(
            render(&report, &options()).contains("— _no reason given_ —"),
            "the missing-reason wording changed"
        );
    }

    #[test]
    fn a_blanket_directive_names_every_rule_without_breaking_the_bold_span() {
        let mut report = Report::new();
        let mut blanket = directive(1, None);
        blanket.rules.clear();
        report.ignores.push(blanket);
        let rendered = render(&report, &options());
        assert!(rendered.contains("- **ruff (all rules)** —"), "{rendered}");
    }

    #[test]
    fn several_rules_are_listed_together() {
        let mut report = Report::new();
        let mut several = directive(3, None);
        several.rules = vec!["E501".into(), "F401".into()];
        report.ignores.push(several);
        assert!(render(&report, &options()).contains("**ruff E501, F401**"));
    }

    #[test]
    fn without_a_repo_and_sha_the_location_is_plain_text() {
        let mut report = Report::new();
        report.ignores.push(directive(12, Some("why")));
        let rendered = render(&report, &MarkdownOptions::default());
        assert!(rendered.contains("— `src/app.py:12`\n"), "{rendered}");
        assert!(!rendered.contains("https://"), "{rendered}");

        // Half the pair is not enough to build a link that resolves.
        let half = MarkdownOptions {
            repo: Some("acme/widgets".into()),
            ..MarkdownOptions::default()
        };
        assert!(render(&report, &half).contains("— `src/app.py:12`\n"));
    }

    #[test]
    fn a_reason_cannot_re_flow_the_rest_of_the_comment() {
        let mut report = Report::new();
        report
            .ignores
            .push(directive(12, Some("*not* a `code` <span> [link]")));
        let rendered = render(&report, &options());
        assert!(
            rendered.contains("_\\*not\\* a \\`code\\` \\<span> \\[link\\]_"),
            "{rendered}"
        );
    }

    #[test]
    fn a_path_with_unusual_bytes_still_builds_a_resolvable_permalink() {
        let mut report = Report::new();
        let mut spaced = directive(4, None);
        spaced.path = "src/my app/a b.py".into();
        report.ignores.push(spaced);
        assert!(
            render(&report, &options()).contains("/src/my%20app/a%20b.py#L4)"),
            "{}",
            render(&report, &options())
        );
    }

    /// Every entry that is listed carries its snippet, collapsed, and the whole
    /// block is spelled out here because its exact shape is what makes GitHub
    /// render markdown inside the `<details>` rather than as literal text.
    #[test]
    fn every_listed_entry_carries_a_collapsed_snippet_of_what_it_suppresses() {
        let source =
            Stub::default().with("src/app.py", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        let mut report = Report::new();
        report.ignores.push(directive(4, Some("why")));
        let rendered = body(&report, &options(), &source);
        assert!(
            rendered.ends_with(&format!(
                "{}{STAMP}",
                concat!(
                    "  <details>\n",
                    "  <summary>suppressed code</summary>\n",
                    "\n",
                    "  ```python\n",
                    "    2 | two\n",
                    "    3 | three\n",
                    "  > 4 | four\n",
                    "    5 | five\n",
                    "    6 | six\n",
                    "  ```\n",
                    "\n",
                    "  </details>\n",
                    "\n",
                )
            )),
            "{rendered}"
        );
    }

    /// A single suppressed line is unreadable alone — the reviewer cannot see
    /// what it sits in — so it is shown with two lines either side, and marked
    /// so the context around it cannot be mistaken for what is silenced.
    #[test]
    fn a_single_suppressed_line_is_shown_with_two_lines_of_context_either_side() {
        let source =
            Stub::default().with("src/app.py", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::Line, 4, Some(4)));
        let rendered = body(&report, &options(), &source);
        assert!(
            rendered.contains(concat!(
                "  ```python\n",
                "    2 | two\n",
                "    3 | three\n",
                "  > 4 | four\n",
                "    5 | five\n",
                "    6 | six\n",
                "  ```\n",
            )),
            "{rendered}"
        );
        // Context is context: it must not read as one more suppression.
        assert_eq!(rendered.matches("\n  > ").count(), 1, "{rendered}");
        assert!(!rendered.contains("seven"), "{rendered}");
    }

    /// The window is clamped to the file rather than padded: a directive on the
    /// first or last line has less than two lines on one side, and the marker
    /// stays on the suppressed line wherever it lands.
    #[test]
    fn a_single_line_span_at_a_file_edge_is_clamped_to_it() {
        let source =
            Stub::default().with("src/app.py", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        for (start, expected) in [
            (
                1,
                concat!("  > 1 | one\n", "    2 | two\n", "    3 | three\n"),
            ),
            (
                2,
                concat!(
                    "    1 | one\n",
                    "  > 2 | two\n",
                    "    3 | three\n",
                    "    4 | four\n",
                ),
            ),
            (
                7,
                concat!("    5 | five\n", "    6 | six\n", "  > 7 | seven\n"),
            ),
        ] {
            let mut report = Report::new();
            report
                .ignores
                .push(spanning(Scope::Line, start, Some(start)));
            let rendered = body(&report, &options(), &source);
            assert!(
                rendered.contains(&format!("  ```python\n{expected}  ```\n")),
                "line {start} rendered:\n{rendered}"
            );
        }
    }

    /// The cap, at the two counts that decide it.
    #[test]
    fn the_body_lists_at_most_max_entries_and_counts_the_rest() {
        for (count, listed, overflow) in [
            (20, 20, None),
            (21, 20, Some("_… and 1 more not shown (21 total)._\n")),
            (25, 20, Some("_… and 5 more not shown (25 total)._\n")),
        ] {
            let mut report = Report::new();
            for line in 1..=count {
                report.ignores.push(directive(line, Some("why")));
            }
            let rendered = body(&report, &options(), &Stub::default());
            assert_eq!(
                rendered.matches("- **ruff E501**").count(),
                listed,
                "{count} findings listed the wrong number of entries:\n{rendered}"
            );
            assert!(
                rendered.contains(&format!("### notignored: {count} suppressions\n")),
                "the heading must still name the total:\n{rendered}"
            );
            match overflow {
                Some(line) => assert!(
                    rendered.ends_with(&format!("{line}\n{STAMP}")),
                    "{rendered}"
                ),
                None => assert!(!rendered.contains("not shown"), "{rendered}"),
            }
        }
    }

    /// The cap is the caller's to set, and a body under it is unchanged by it.
    #[test]
    fn max_entries_is_configurable() {
        let mut report = Report::new();
        for line in 1..=3 {
            report.ignores.push(directive(line, Some("why")));
        }
        let capped = MarkdownOptions {
            max_entries: 2,
            ..options()
        };
        let rendered = body(&report, &capped, &Stub::default());
        assert_eq!(rendered.matches("- **ruff E501**").count(), 2, "{rendered}");
        assert!(
            rendered.ends_with(&format!("_… and 1 more not shown (3 total)._\n\n{STAMP}")),
            "{rendered}"
        );

        let roomy = MarkdownOptions {
            max_entries: 50,
            ..options()
        };
        let rendered = body(&report, &roomy, &Stub::default());
        assert_eq!(rendered.matches("- **ruff E501**").count(), 3, "{rendered}");
        assert!(!rendered.contains("not shown"), "{rendered}");
    }

    /// What a snippet shows is the record's `suppressed` span, which is what
    /// each scope means: the directive's own line, the line below it, or the
    /// whole delimited region. A one-line span carries its context and its
    /// marker whichever scope produced it; a block is shown bare.
    #[test]
    fn each_scope_shows_the_span_it_suppresses() {
        let source =
            Stub::default().with("src/app.py", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        for (directive, expected) in [
            (
                spanning(Scope::Line, 4, Some(4)),
                "    2 | two\n    3 | three\n  > 4 | four\n    5 | five\n    6 | six\n",
            ),
            (
                spanning(Scope::NextLine, 5, Some(5)),
                "    3 | three\n    4 | four\n  > 5 | five\n    6 | six\n    7 | seven\n",
            ),
            (
                spanning(Scope::Block, 3, Some(5)),
                "  3 | three\n  4 | four\n  5 | five\n",
            ),
            // An unterminated block runs to end-of-file.
            (
                spanning(Scope::Block, 5, None),
                "  5 | five\n  6 | six\n  7 | seven\n",
            ),
        ] {
            let scope = directive.scope;
            let mut report = Report::new();
            report.ignores.push(directive);
            let rendered = body(&report, &options(), &source);
            assert!(
                rendered.contains(&format!("  ```python\n{expected}  ```\n")),
                "{scope} rendered:\n{rendered}"
            );
            assert!(!rendered.contains("whole file"), "{scope}: {rendered}");
        }
    }

    /// `file` scope silences code the record's span does not single out, so the
    /// snippet shows the top of the file and says what it is standing in for.
    #[test]
    fn a_file_scope_directive_shows_the_top_of_the_file_and_says_so() {
        let short = Stub::default().with("src/app.py", "one\ntwo\nthree\n");
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::File, 1, None));
        let rendered = body(&report, &options(), &short);
        assert!(
            rendered.contains("  _the whole file is suppressed._\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("  ```python\n  1 | one\n  2 | two\n  3 | three\n  ```\n"),
            "{rendered}"
        );

        // A directive can sit anywhere in the file and still cover all of it.
        let long: String = (1..=14).map(|n| format!("line {n}\n")).collect();
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::File, 6, None));
        let rendered = body(
            &report,
            &options(),
            &Stub::default().with("src/app.py", &long),
        );
        assert!(
            rendered
                .contains("  _the whole file is suppressed; showing its first 10 of 14 lines._\n"),
            "{rendered}"
        );
        assert!(rendered.contains("   1 | line 1\n"), "{rendered}");
        assert!(rendered.contains("  10 | line 10\n"), "{rendered}");
        assert!(!rendered.contains("line 11"), "{rendered}");
    }

    /// A span longer than the snippet allows is cut, never re-centred: the first
    /// lines of what is silenced are the ones a reviewer needs.
    #[test]
    fn a_span_longer_than_ten_lines_is_truncated_with_a_note() {
        let text: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let source = Stub::default().with("src/app.py", &text);
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::Block, 5, Some(28)));
        let rendered = body(&report, &options(), &source);
        assert!(
            rendered.contains("  _showing the first 10 of 24 suppressed lines._\n"),
            "{rendered}"
        );
        assert!(rendered.contains("   5 | line 5\n"), "{rendered}");
        assert!(rendered.contains("  14 | line 14\n"), "{rendered}");
        assert!(!rendered.contains("line 15"), "{rendered}");

        // Exactly ten is not truncated, so the note is about a real omission.
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::Block, 5, Some(14)));
        let rendered = body(&report, &options(), &source);
        assert!(!rendered.contains("showing the first"), "{rendered}");
        assert!(rendered.contains("  14 | line 14\n"), "{rendered}");
    }

    /// A span the file cannot hold is clamped to it rather than dropped: the
    /// lines that do exist are real source.
    #[test]
    fn a_span_running_past_the_end_of_the_file_stops_at_it() {
        let source = Stub::default().with("src/app.py", "one\ntwo\nthree\n");
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::Block, 2, Some(9)));
        let rendered = body(&report, &options(), &source);
        assert!(
            rendered.contains("  ```python\n  2 | two\n  3 | three\n  ```\n"),
            "{rendered}"
        );
    }

    /// Every editor in this repo strips trailing whitespace on save, so a body
    /// that emitted any could not survive as a checked-in golden.
    #[test]
    fn no_rendered_line_ends_in_whitespace() {
        let source = Stub::default().with("src/app.py", "one\n\nthree   \n\nfive\n");
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::Block, 2, Some(3)));
        let rendered = body(&report, &options(), &source);
        for line in rendered.lines() {
            assert_eq!(line, line.trim_end(), "trailing space in {line:?}");
        }
        assert!(rendered.contains("  2 |\n  3 | three\n"), "{rendered}");
    }

    #[test]
    fn line_numbers_are_padded_to_a_common_width() {
        let text: String = (1..=12).map(|n| format!("line {n}\n")).collect();
        let source = Stub::default().with("src/app.py", &text);
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::Block, 8, Some(12)));
        let rendered = body(&report, &options(), &source);
        assert!(rendered.contains("   8 | line 8\n"), "{rendered}");
        assert!(rendered.contains("  12 | line 12\n"), "{rendered}");
    }

    #[test]
    fn source_that_contains_a_fence_cannot_close_the_block_early() {
        let source = Stub::default().with(
            "src/app.py",
            "a\nDOC = \"\"\"```\nx = 1  # noqa\n```\"\"\"\nb\n",
        );
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::Block, 2, Some(4)));
        let rendered = body(&report, &options(), &source);
        assert!(rendered.contains("  ````python\n"), "{rendered}");
        assert!(
            rendered.contains("  ````\n\n  </details>\n"),
            "the closing fence must be as long as the opening one:\n{rendered}"
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_renders_without_a_snippet() {
        let mut report = Report::new();
        report.ignores.push(directive(12, Some("why")));
        let rendered = body(&report, &options(), &Stub::default());
        assert!(!rendered.contains("<details>"), "{rendered}");
        assert!(rendered.contains("- **ruff E501**"), "{rendered}");
    }

    /// A record naming a line the file does not have would otherwise index past
    /// the end of the source. An empty file has nothing to show either, whatever
    /// the scope claims to cover.
    #[test]
    fn a_span_beyond_the_end_of_the_file_renders_without_a_snippet() {
        let source = Stub::default().with("src/app.py", "one\ntwo\n");
        let mut report = Report::new();
        report.ignores.push(directive(9, Some("why")));
        assert!(!body(&report, &options(), &source).contains("<details>"));

        let empty = Stub::default().with("src/app.py", "");
        let mut report = Report::new();
        report.ignores.push(spanning(Scope::File, 1, None));
        assert!(!body(&report, &options(), &empty).contains("<details>"));
    }

    #[test]
    fn the_language_tag_follows_the_path() {
        for (path, tag) in [
            ("a.py", "python"),
            ("a.rs", "rust"),
            ("a.js", "javascript"),
            ("a.tsx", "typescript"),
            ("a.sh", "bash"),
            ("a.yml", "yaml"),
            ("a.toml", "toml"),
            ("Makefile", ""),
        ] {
            assert_eq!(language_tag(path), tag, "{path}");
        }
    }

    #[test]
    fn files_that_could_not_be_scanned_are_named_in_the_body() {
        let mut report = Report::new();
        report.errors.push(ReportError {
            path: "broken.py".into(),
            message: "stream did not contain valid UTF-8".into(),
        });
        let rendered = render(&report, &options());
        assert!(
            rendered.contains("#### Could not be scanned\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("- `broken.py` — stream did not contain valid UTF-8\n"),
            "{rendered}"
        );
    }

    #[test]
    fn render_markdown_writes_the_body_to_the_writer() {
        let mut report = Report::new();
        report.ignores.push(directive(12, Some("why")));
        let mut out = Vec::new();
        render_markdown(&report, &options(), &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.starts_with(MARKER), "{rendered}");
        assert!(rendered.contains("- **ruff E501**"), "{rendered}");
    }
}
