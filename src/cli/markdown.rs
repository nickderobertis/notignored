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

use crate::model::{IgnoreDirective, Report};
use crate::source::Language;

/// The hidden marker every rendered body starts with.
///
/// `scripts/action/comment.sh` searches for this exact string; the drift between
/// the two is gated by `tests/action_contract.rs`.
pub const MARKER: &str = "<!-- notignored-report -->";

/// Below this many findings, every entry carries a source snippet.
///
/// A short list is worth reading inline; a long one would bury the pull request
/// under context nobody scrolls through, so past this count the permalinks carry
/// the reader instead.
const SNIPPET_LIMIT: usize = 4;

/// Lines of context shown on each side of a directive.
const CONTEXT_LINES: u32 = 2;

/// What the permalinks in a rendered body point at.
///
/// Both parts are needed to build one; with either missing the location renders
/// as plain `path:line` text, so a run without them still produces a usable body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkdownOptions {
    /// `owner/repo` the permalinks address.
    pub repo: Option<String>,
    /// Commit the permalinks pin the source to.
    pub sha: Option<String>,
}

impl MarkdownOptions {
    /// The `https://github.com/<owner>/<repo>/blob/<sha>/<path>#L<line>`
    /// permalink for a directive, or `None` when this run was not told where the
    /// source lives.
    fn permalink(&self, directive: &IgnoreDirective) -> Option<String> {
        let (repo, sha) = (self.repo.as_ref()?, self.sha.as_ref()?);
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
        body.push_str(&format!(
            "### notignored: {count} {}\n\n",
            if count == 1 {
                "suppression"
            } else {
                "suppressions"
            }
        ));
        for directive in &report.ignores {
            body.push_str(&entry(directive, options));
            if count < SNIPPET_LIMIT {
                if let Some(snippet) = snippet(directive, source) {
                    body.push('\n');
                    body.push_str(&snippet);
                    body.push('\n');
                }
            }
        }
    }
    if !report.errors.is_empty() {
        body.push_str("\n#### Could not be scanned\n\n");
        for error in &report.errors {
            body.push_str(&format!(
                "- `{}` — {}\n",
                error.path,
                escape(&error.message)
            ));
        }
    }
    body
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
    format!("- **{} {rules}** — {reason} — {location}\n", directive.tool)
}

/// The directive's line with [`CONTEXT_LINES`] on each side, as an indented
/// fenced block under its list item.
///
/// `None` when the file cannot be read, or when it is too short to hold the line
/// the record names — a report rendered somewhere other than where it was
/// produced must not invent context.
fn snippet(directive: &IgnoreDirective, source: &dyn SnippetSource) -> Option<String> {
    let lines = source.lines(&directive.path)?;
    let count = u32::try_from(lines.len()).unwrap_or(u32::MAX);
    if directive.line == 0 || directive.line > count {
        return None;
    }
    let first = directive.line.saturating_sub(CONTEXT_LINES).max(1);
    let last = directive.line.saturating_add(CONTEXT_LINES).min(count);

    let window = &lines[(first as usize - 1)..last as usize];
    let fence = fence_for(window);
    let width = last.to_string().len();
    let mut block = format!("  {fence}{}\n", language_tag(&directive.path));
    for (offset, text) in window.iter().enumerate() {
        let number = first + u32::try_from(offset).unwrap_or_default();
        // An empty source line renders as a bare gutter: a trailing space here
        // is whitespace every editor in the repo is configured to strip, which
        // would break the checked-in golden bodies on the next save.
        match text.trim_end().is_empty() {
            true => block.push_str(&format!("  {number:>width$} |\n")),
            false => block.push_str(&format!("  {number:>width$} | {}\n", text.trim_end())),
        }
    }
    block.push_str(&format!("  {fence}\n"));
    Some(block)
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
    use crate::model::{ReportError, Scope, Suppressed, Tool};

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
        }
    }

    fn options() -> MarkdownOptions {
        MarkdownOptions {
            repo: Some("acme/widgets".into()),
            sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
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
            format!("{MARKER}\n\n### notignored\n\nNo lint or type-check suppressions found.\n")
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
            sha: None,
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

    /// The rule the count decides: a short list is worth reading inline.
    #[test]
    fn entries_carry_a_snippet_only_below_the_limit() {
        let source =
            Stub::default().with("src/app.py", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        for count in 1..=5 {
            let mut report = Report::new();
            for _ in 0..count {
                report.ignores.push(directive(4, Some("why")));
            }
            let rendered = body(&report, &options(), &source);
            assert_eq!(
                rendered.contains("```python"),
                count < SNIPPET_LIMIT,
                "count {count} rendered:\n{rendered}"
            );
        }
    }

    #[test]
    fn a_snippet_shows_two_lines_on_each_side_with_their_numbers() {
        let source =
            Stub::default().with("src/app.py", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        let mut report = Report::new();
        report.ignores.push(directive(4, Some("why")));
        let rendered = body(&report, &options(), &source);
        assert!(
            rendered.ends_with(concat!(
                "  ```python\n",
                "  2 | two\n",
                "  3 | three\n",
                "  4 | four\n",
                "  5 | five\n",
                "  6 | six\n",
                "  ```\n",
                "\n",
            )),
            "{rendered}"
        );
    }

    #[test]
    fn a_snippet_at_the_edges_of_a_file_is_clamped_to_it() {
        let source = Stub::default().with("src/app.py", "one\ntwo\nthree\n");
        let mut report = Report::new();
        report.ignores.push(directive(1, Some("top")));
        let rendered = body(&report, &options(), &source);
        assert!(
            rendered.contains("  1 | one\n  2 | two\n  3 | three\n"),
            "{rendered}"
        );

        let mut report = Report::new();
        report.ignores.push(directive(3, Some("bottom")));
        let rendered = body(&report, &options(), &source);
        assert!(
            rendered.contains("  1 | one\n  2 | two\n  3 | three\n"),
            "{rendered}"
        );
    }

    /// Every editor in this repo strips trailing whitespace on save, so a body
    /// that emitted any could not survive as a checked-in golden.
    #[test]
    fn no_rendered_line_ends_in_whitespace() {
        let source = Stub::default().with("src/app.py", "one\n\nthree   \n\nfive\n");
        let mut report = Report::new();
        report.ignores.push(directive(3, Some("why")));
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
        report.ignores.push(directive(10, Some("why")));
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
        report.ignores.push(directive(3, Some("why")));
        let rendered = body(&report, &options(), &source);
        assert!(rendered.contains("  ````python\n"), "{rendered}");
        assert!(rendered.trim_end().ends_with("  ````"), "{rendered}");
    }

    #[test]
    fn a_file_that_cannot_be_read_renders_without_a_snippet() {
        let mut report = Report::new();
        report.ignores.push(directive(12, Some("why")));
        let rendered = body(&report, &options(), &Stub::default());
        assert!(!rendered.contains("```"), "{rendered}");
        assert!(rendered.contains("- **ruff E501**"), "{rendered}");
    }

    /// A record naming a line the file does not have would otherwise index past
    /// the end of the source.
    #[test]
    fn a_line_beyond_the_end_of_the_file_renders_without_a_snippet() {
        let source = Stub::default().with("src/app.py", "one\ntwo\n");
        let mut report = Report::new();
        report.ignores.push(directive(9, Some("why")));
        assert!(!body(&report, &options(), &source).contains("```"));
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
