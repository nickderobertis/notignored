//! Pyright (`# pyright: ignore`) suppression parsing.
//!
//! Forms understood, each verified against the pinned pyright by
//! `tests/e2e/python_types_parity.rs`:
//!
//! | Source | Scope | Rules |
//! | --- | --- | --- |
//! | `# pyright: ignore` | line | *(blanket)* |
//! | `# pyright: ignore[reportArgumentType]` | line | `reportArgumentType` |
//! | `# pyright: ignore[reportAny, reportUnusedImport]` | line | both |
//!
//! Pyright honours a directive anywhere in the comment — `# noqa: F401  #
//! pyright: ignore[reportAny]` suppresses — so, like ruff, this reports the span
//! of the inner `#` that opened it. Any further `# …` is the
//! [`reason`](crate::model::IgnoreDirective::reason).
//!
//! Everything else a `# pyright: …` comment can say (`# pyright: basic`,
//! `# pyright: strict`) switches the type-checking mode rather than silencing a
//! diagnostic, and is deliberately **not** reported: a mode switch is a config
//! change, and a review tool that files it as a suppression cries wolf. Pyright
//! also honours mypy's `# type: ignore`, which is reported once, as mypy's.
//!
//! Pyright's ignore is always line-scoped: unlike mypy and ty it gives a
//! directive above the first statement no special meaning.

use crate::model::{IgnoreDirective, Scope, Suppressed, Tool};
use crate::source::{Language, SourceFile};
use crate::tools::python;
use crate::tools::ToolParser;

/// Parses pyright's `# pyright: ignore` family out of Python sources.
#[derive(Debug, Clone, Copy, Default)]
pub struct PyrightParser;

impl ToolParser for PyrightParser {
    fn tool(&self) -> Tool {
        Tool::Pyright
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        file.language() == Language::Python
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        let mut out = Vec::new();
        for comment in file.comments() {
            // One suppression per comment: a second `# pyright: ignore` on the
            // same line silences nothing the first did not.
            let Some((segment, rest)) = python::segments(comment)
                .find_map(|segment| directive_body(segment.after_hash).map(|rest| (segment, rest)))
            else {
                continue;
            };
            let (rules, reason) = python::rules_and_reason(rest);
            out.push(IgnoreDirective {
                tool: Tool::Pyright,
                scope: Scope::Line,
                rules,
                reason,
                path: file.display_path().to_string(),
                line: comment.line,
                end_line: comment.end_line,
                column: segment.column,
                raw: segment.raw.to_string(),
                suppressed: Suppressed {
                    start_line: comment.line,
                    end_line: Some(comment.line),
                },
            });
        }
        out
    }
}

/// The text after `pyright: ignore`, or `None` for any other `# pyright: …`
/// comment (the mode switches).
fn directive_body(after_hash: &str) -> Option<&str> {
    let after_prefix = after_hash.trim_start().strip_prefix("pyright:")?;
    python::strip_keyword(after_prefix.trim_start(), "ignore")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        let file = SourceFile::new("src/app.py", source.to_string());
        PyrightParser.parse(&file)
    }

    fn only(source: &str) -> IgnoreDirective {
        let mut found = parse(source);
        assert_eq!(
            found.len(),
            1,
            "expected one directive in {source:?}: {found:#?}"
        );
        found.remove(0)
    }

    #[test]
    fn the_parser_claims_python_files_only() {
        assert_eq!(PyrightParser.tool(), Tool::Pyright);
        assert!(PyrightParser.applies_to(&SourceFile::new("a.py", String::new())));
        assert!(!PyrightParser.applies_to(&SourceFile::new("a.rs", String::new())));
    }

    #[test]
    fn a_bare_ignore_is_a_blanket_line_suppression() {
        let directive = only("f(1)  # pyright: ignore\n");
        assert_eq!(directive.tool, Tool::Pyright);
        assert_eq!(directive.scope, Scope::Line);
        assert!(directive.rules.is_empty());
        assert_eq!(directive.reason, None);
        assert_eq!(directive.path, "src/app.py");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (1, 1, 7)
        );
        assert_eq!(directive.raw, "# pyright: ignore");
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: Some(1)
            }
        );
    }

    #[test]
    fn rule_names_are_captured_verbatim() {
        assert_eq!(
            only("f(1)  # pyright: ignore[reportArgumentType]\n").rules,
            vec!["reportArgumentType"]
        );
        assert_eq!(
            only("f(1)  # pyright: ignore[reportAny, reportUnusedImport]\n").rules,
            vec!["reportAny", "reportUnusedImport"]
        );
    }

    #[test]
    fn a_trailing_comment_becomes_the_reason() {
        let directive = only("f(1)  # pyright: ignore[reportAny]  # third-party stub is Any\n");
        assert_eq!(directive.rules, vec!["reportAny"]);
        assert_eq!(directive.reason.as_deref(), Some("third-party stub is Any"));
    }

    #[test]
    fn a_directive_need_not_open_the_comment() {
        let directive = only("f(1)  # noqa: F401  # pyright: ignore[reportAny]\n");
        assert_eq!(directive.rules, vec!["reportAny"]);
        assert_eq!(directive.raw, "# pyright: ignore[reportAny]");
        assert_eq!(directive.column, 21);
    }

    #[test]
    fn a_leading_directive_still_only_covers_its_own_line() {
        // Pyright gives a comment above the first statement no module-wide
        // meaning, so neither do we.
        let directive = only("# pyright: ignore\nimport os\n");
        assert_eq!(directive.scope, Scope::Line);
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: Some(1)
            }
        );
    }

    #[test]
    fn mode_switches_are_configuration_not_suppressions() {
        assert!(parse("# pyright: basic\nimport os\n").is_empty());
        assert!(parse("# pyright: strict\nimport os\n").is_empty());
        assert!(parse("# pyright: reportMissingImports=false\nimport os\n").is_empty());
    }

    #[test]
    fn look_alike_comments_are_not_directives() {
        assert!(parse("f(1)  # pyright: ignored\n").is_empty());
        assert!(parse("f(1)  # pyright ignore\n").is_empty());
        assert!(parse("f(1)  # PYRIGHT: IGNORE\n").is_empty());
        assert!(parse("f(1)  # type: ignore\n").is_empty());
        assert!(parse("f(1)  # copyright: ignore\n").is_empty());
    }

    #[test]
    fn a_directive_inside_a_string_literal_is_not_reported() {
        assert!(parse("MSG = \"# pyright: ignore\"\n").is_empty());
    }

    #[test]
    fn every_directive_in_a_file_is_reported_in_source_order() {
        let found = parse("f(1)  # pyright: ignore\ng(2)  # pyright: ignore[reportAny]\n");
        assert_eq!(found.len(), 2);
        assert_eq!(found.iter().map(|d| d.line).collect::<Vec<_>>(), vec![1, 2]);
    }
}
