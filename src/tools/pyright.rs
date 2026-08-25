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
//! | `# pyright: reportMissingImports=false` | file | `reportMissingImports` |
//!
//! Pyright honours an *ignore* anywhere in the comment — `# noqa: F401  #
//! pyright: ignore[reportAny]` suppresses — so, like ruff, this reports the span
//! of the inner `#` that opened it. Any further `# …` is the
//! [`reason`](crate::model::IgnoreDirective::reason), up to the next tool's
//! directive, so a `# noqa` sharing the line is never filed as pyright's
//! justification.
//!
//! The `<rule>=<value>` form is pyright's per-file rule override, and the values
//! that switch a rule **off** — `false` and `none` — silence it for the whole
//! file. The other four (`true`, `error`, `warning`, `information`) turn a rule
//! on or move its severity, so they are configuration and not reported; a comment
//! that lists none of the off values yields no record. Neither does a mode switch
//! (`# pyright: basic`, `# pyright: strict`): a review tool that files a config
//! change as a suppression cries wolf.
//!
//! Two constraints on that form come straight from what pyright honours:
//!
//! * **The directive must open its comment.** `# stale  # pyright: reportAny=false`
//!   is prose to pyright, so reporting it would invent a suppression.
//! * **The whole comment is the directive.** Pyright reads the rest of the line
//!   as its comma-separated item list, so a trailing `# reason` — or any value
//!   outside those six — makes it reject the comment and silence nothing. That is
//!   why this form never carries a reason.
//!
//! Where the comment *sits* is not one of them: pyright faults a `<rule>=<value>`
//! comment that trails code or is indented, and then applies it anyway. The rule
//! really is off, so the record is what a reviewer needs either way.
//!
//! Pyright also honours mypy's `# type: ignore`, which is reported once, as
//! mypy's. Its own ignore is always line-scoped: unlike ty it gives a directive
//! above the first statement no special meaning.

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
            let found = python::segments(comment)
                .into_iter()
                .find_map(|segment| directive_body(segment.after_hash).map(|rest| (segment, rest)));
            if let Some((segment, rest)) = found {
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
                    change: None,
                });
                continue;
            }
            // A rule override is the whole comment, so it is read from the raw
            // text rather than from a segment: a second `#` is not a boundary
            // here but a malformation pyright refuses outright.
            let Some(rules) = comment
                .raw
                .strip_prefix('#')
                .and_then(config_body)
                .and_then(disabled_rules)
            else {
                continue;
            };
            out.push(IgnoreDirective {
                tool: Tool::Pyright,
                scope: Scope::File,
                rules,
                // Pyright reads any trailing `# …` as part of its item list and
                // then rejects the comment, so this form has no reason to carry.
                reason: None,
                path: file.display_path().to_string(),
                line: comment.line,
                end_line: comment.end_line,
                column: comment.column,
                raw: comment.raw.trim_end().to_string(),
                suppressed: Suppressed {
                    start_line: 1,
                    end_line: None,
                },
                change: None,
            });
        }
        out
    }
}

/// True when the text after a `#` opens a pyright suppression.
///
/// The shared segment scan uses this to bound the run before it; see
/// `src/tools/python.rs::segments`.
pub(super) fn opens_directive(after_hash: &str) -> bool {
    directive_body(after_hash).is_some()
}

/// The text after `pyright: ignore`, or `None` for any other `# pyright: …`
/// comment (the mode switches and the rule overrides).
fn directive_body(after_hash: &str) -> Option<&str> {
    let after_prefix = after_hash.trim_start().strip_prefix("pyright:")?;
    python::strip_keyword(after_prefix.trim_start(), "ignore")
}

/// The item list of a `# pyright: <item>, <item>` comment, or `None` when the
/// comment does not open with the keyword.
fn config_body(after_hash: &str) -> Option<&str> {
    Some(after_hash.trim_start().strip_prefix("pyright:")?.trim())
}

/// The type-checking modes that may appear among the items without a value.
const MODES: [&str; 3] = ["basic", "standard", "strict"];

/// The values pyright accepts for a rule, and the two that switch it off.
const VALUES: [&str; 6] = ["true", "false", "error", "warning", "information", "none"];
const OFF: [&str; 2] = ["false", "none"];

/// The rules an item list switches off, or `None` when there is nothing to
/// report: a list pyright would reject outright (and so honour nowhere), or one
/// it honours that silences nothing (every rule turned *on*, or a mode switch).
fn disabled_rules(items: &str) -> Option<Vec<String>> {
    let mut disabled = Vec::new();
    for item in items.split(',') {
        let item = item.trim();
        let Some((rule, value)) = item.split_once('=') else {
            // A valueless item is only ever a mode switch; anything else is the
            // form pyright answers with "must be followed by = and a value".
            if MODES.contains(&item) {
                continue;
            }
            return None;
        };
        let (rule, value) = (rule.trim(), value.trim());
        if rule.is_empty() || !VALUES.contains(&value) {
            return None;
        }
        if OFF.contains(&value) {
            disabled.push(rule.to_string());
        }
    }
    (!disabled.is_empty()).then_some(disabled)
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
    }

    #[test]
    fn a_rule_switched_off_is_a_file_wide_suppression() {
        let directive = only("# pyright: reportMissingImports=false\nimport legacy\n");
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(directive.rules, vec!["reportMissingImports"]);
        assert_eq!(directive.reason, None);
        assert_eq!((directive.line, directive.column), (1, 1));
        assert_eq!(directive.raw, "# pyright: reportMissingImports=false");
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
        // `none` is pyright's other off value, and the spaces it tolerates
        // around the `=` do not change what it reads.
        assert_eq!(
            only("# pyright: reportMissingImports = none\nimport legacy\n").rules,
            vec!["reportMissingImports"]
        );
    }

    #[test]
    fn only_the_rules_switched_off_are_reported() {
        let directive = only(
            "# pyright: strict, reportMissingImports=false, reportAny=error, reportUnusedImport=none\n",
        );
        assert_eq!(
            directive.rules,
            vec!["reportMissingImports", "reportUnusedImport"]
        );
        // A rule turned on, or given a severity, silences nothing.
        for on in ["true", "error", "warning", "information"] {
            assert!(
                parse(&format!("# pyright: reportAny={on}\n")).is_empty(),
                "`{on}` is not a suppression"
            );
        }
    }

    #[test]
    fn a_rule_override_pyright_rejects_is_not_a_suppression() {
        // Pyright reads the rest of the line as its item list, so each of these
        // makes it refuse the comment — and silence nothing.
        for rejected in [
            "# pyright: reportAny=false  # no stubs published",
            "# pyright: reportAny=False",
            "# pyright: reportAny",
            "# pyright: reportAny=false, ",
            "# pyright: =false",
        ] {
            assert!(
                parse(&format!("{rejected}\nimport legacy\n")).is_empty(),
                "{rejected:?} is a form pyright refuses"
            );
        }
        // The directive has to open the comment: embedded, pyright reads prose.
        assert!(parse("# stale  # pyright: reportAny=false\n").is_empty());
    }

    #[test]
    fn a_rule_override_is_reported_wherever_pyright_applies_it() {
        // Pyright faults the placement of an override that trails code and then
        // applies it anyway, so the rule really is off for the file.
        let directive = only("import legacy  # pyright: reportMissingImports=false\n");
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(directive.rules, vec!["reportMissingImports"]);
        assert_eq!((directive.line, directive.column), (1, 16));
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
