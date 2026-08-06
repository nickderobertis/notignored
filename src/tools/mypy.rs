//! mypy (`# type: ignore`, `# mypy: …`) suppression parsing.
//!
//! Forms understood, each verified against the pinned mypy by
//! `tests/e2e/python_types_parity.rs`:
//!
//! | Source | Scope | Rules |
//! | --- | --- | --- |
//! | `# type: ignore` | line | *(blanket)* |
//! | `# type: ignore[arg-type, index]` | line | `arg-type`, `index` |
//! | `# type: ignore` above all code | file | *(blanket)* |
//! | `# mypy: ignore-errors` | file | *(blanket)* |
//! | `# mypy: disable-error-code="arg-type, index"` | file | `arg-type`, `index` |
//!
//! Three constraints come straight from what mypy actually honours, and each one
//! is the difference between reporting a live suppression and inventing one:
//!
//! * **The directive must open its comment.** mypy reads `# type: ignore  # noqa`
//!   but not `# noqa  # type: ignore`, so — unlike ruff, pyright, and ty — an
//!   embedded directive is not reported.
//! * **A bare `# type: ignore` above all code exempts the whole module.** After
//!   so much as a docstring the same comment is line-scoped. The bracketed form
//!   is never file-scoped: mypy rejects it there outright.
//! * **`# mypy: …` config comments must be on their own line**, anywhere in the
//!   file. Trailing after code, mypy ignores them.
//!
//! Any further `# …` becomes the [`reason`](crate::model::IgnoreDirective::reason).
//! Note that mypy itself only tolerates one after `# type: ignore`: a trailing
//! comment on a `# mypy: …` line is parsed as part of the option value and makes
//! mypy fail, so a reason there is a mistake we report rather than endorse.
//!
//! pyright and ty honour `# type: ignore` too. It is reported once, as mypy's,
//! because that is whose syntax it is; see the README's supported-tools table.

use crate::model::{IgnoreDirective, Scope, Suppressed, Tool};
use crate::source::{Language, SourceFile};
use crate::tools::python::{self, Segment};
use crate::tools::ToolParser;

/// Parses mypy's `# type: ignore` and `# mypy: …` families out of Python sources.
#[derive(Debug, Clone, Copy, Default)]
pub struct MypyParser;

impl ToolParser for MypyParser {
    fn tool(&self) -> Tool {
        Tool::Mypy
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        file.language() == Language::Python
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        let mut out = Vec::new();
        for comment in file.comments() {
            // mypy only reads the run that opens the comment.
            let found = python::segments(comment).next().and_then(|segment| {
                let in_header = || python::in_file_header(file, comment);
                directive(&segment, comment.leading, in_header).map(|parsed| (segment, parsed))
            });
            let Some((segment, (scope, rules, reason))) = found else {
                continue;
            };
            out.push(IgnoreDirective {
                tool: Tool::Mypy,
                scope,
                rules,
                reason,
                path: file.display_path().to_string(),
                line: comment.line,
                end_line: comment.end_line,
                column: segment.column,
                raw: segment.raw.to_string(),
                suppressed: suppressed_range(scope, comment.line),
            });
        }
        out
    }
}

fn suppressed_range(scope: Scope, line: u32) -> Suppressed {
    match scope {
        // A module-level exemption runs from the first line to end-of-file.
        Scope::File => Suppressed {
            start_line: 1,
            end_line: None,
        },
        _ => Suppressed {
            start_line: line,
            end_line: Some(line),
        },
    }
}

/// Recognize a mypy directive at the start of a comment.
///
/// `in_header` is deferred because working it out means walking the lines above
/// the comment, and only a bare `# type: ignore` ever asks.
fn directive(
    segment: &Segment<'_>,
    leading: bool,
    in_header: impl Fn() -> bool,
) -> Option<(Scope, Vec<String>, Option<String>)> {
    let body = segment.after_hash.trim_start();

    if let Some(rest) = strip_directive(body, "type:", "ignore") {
        let (rules, reason) = python::rules_and_reason(rest);
        // Only the blanket form reaches module scope; mypy rejects
        // `# type: ignore[code]` above the first statement outright.
        let scope = if rules.is_empty() && leading && in_header() {
            Scope::File
        } else {
            Scope::Line
        };
        return Some((scope, rules, reason));
    }

    // Everything below is mypy's inline config, which it reads only from a
    // comment that owns its line.
    if !leading {
        return None;
    }
    if let Some(rest) = strip_directive(body, "mypy:", "ignore-errors") {
        return Some((Scope::File, Vec::new(), python::trailing_reason(rest)));
    }
    if let Some(rest) = strip_directive(body, "mypy:", "disable-error-code") {
        let (rules, reason) = disabled_codes(rest);
        return Some((Scope::File, rules, reason));
    }
    None
}

/// Strip a `<prefix> <keyword>` pair, tolerating the whitespace mypy tolerates
/// (`#type:ignore` is a directive to mypy, so it is one here too).
fn strip_directive<'a>(body: &'a str, prefix: &str, keyword: &str) -> Option<&'a str> {
    let after_prefix = body.strip_prefix(prefix)?;
    python::strip_keyword(after_prefix.trim_start(), keyword)
}

/// Split `=arg-type,index` or `="arg-type, index"` into codes and any reason.
fn disabled_codes(rest: &str) -> (Vec<String>, Option<String>) {
    let Some(after_equals) = rest.trim_start().strip_prefix('=') else {
        // `# mypy: disable-error-code` with no value silences nothing, but the
        // intent is plain, so report it as a blanket exemption rather than drop it.
        return (Vec::new(), python::trailing_reason(rest));
    };
    let (list, remainder) = match after_equals.find('#') {
        Some(hash) => after_equals.split_at(hash),
        None => (after_equals, ""),
    };
    (
        python::split_rules(list),
        python::trailing_reason(remainder),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        let file = SourceFile::new("src/app.py", source.to_string());
        MypyParser.parse(&file)
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
        assert_eq!(MypyParser.tool(), Tool::Mypy);
        assert!(MypyParser.applies_to(&SourceFile::new("a.py", String::new())));
        assert!(!MypyParser.applies_to(&SourceFile::new("a.ts", String::new())));
    }

    #[test]
    fn a_bare_type_ignore_is_a_blanket_line_suppression() {
        let directive = only("import os\nf(1)  # type: ignore\n");
        assert_eq!(directive.tool, Tool::Mypy);
        assert_eq!(directive.scope, Scope::Line);
        assert!(directive.rules.is_empty());
        assert_eq!(directive.reason, None);
        assert_eq!(directive.path, "src/app.py");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (2, 2, 7)
        );
        assert_eq!(directive.raw, "# type: ignore");
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 2,
                end_line: Some(2)
            }
        );
    }

    #[test]
    fn error_codes_are_captured_verbatim() {
        assert_eq!(
            only("x = 1\nf(1)  # type: ignore[arg-type]\n").rules,
            vec!["arg-type"]
        );
        assert_eq!(
            only("x = 1\nf(1)  # type: ignore[arg-type, index]\n").rules,
            vec!["arg-type", "index"]
        );
        assert_eq!(
            only("x = 1\nf(1)  # type: ignore[arg-type,index]\n").rules,
            vec!["arg-type", "index"]
        );
    }

    #[test]
    fn whitespace_around_the_keywords_is_optional() {
        assert_eq!(only("x = 1\nf(1)  #type:ignore\n").scope, Scope::Line);
        assert_eq!(
            only("x = 1\nf(1)  #  type:  ignore[misc]\n").rules,
            vec!["misc"]
        );
    }

    #[test]
    fn a_trailing_comment_becomes_the_reason() {
        let directive = only("x = 1\nf(1)  # type: ignore[arg-type]  # upstream stub is wrong\n");
        assert_eq!(directive.rules, vec!["arg-type"]);
        assert_eq!(directive.reason.as_deref(), Some("upstream stub is wrong"));
        assert_eq!(
            directive.raw,
            "# type: ignore[arg-type]  # upstream stub is wrong"
        );
    }

    #[test]
    fn a_bare_type_ignore_above_all_code_exempts_the_module() {
        let directive = only("#!/usr/bin/env python\n\n# type: ignore  # generated\nimport os\n");
        assert_eq!(directive.scope, Scope::File);
        assert!(directive.rules.is_empty());
        assert_eq!(directive.reason.as_deref(), Some("generated"));
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
    }

    #[test]
    fn the_module_form_needs_both_a_bare_code_list_and_an_empty_header() {
        // A docstring is code, so the same comment is line-scoped after one.
        assert_eq!(
            only("\"\"\"Doc.\"\"\"\n# type: ignore\nimport os\n").scope,
            Scope::Line
        );
        // mypy rejects a code list at module level, so this is never file scope.
        assert_eq!(
            only("# type: ignore[arg-type]\nimport os\n").scope,
            Scope::Line
        );
        // And a trailing directive is scoped to its own line, header or not.
        assert_eq!(only("x = 1  # type: ignore\n").scope, Scope::Line);
    }

    #[test]
    fn ignore_errors_exempts_the_whole_file_from_wherever_it_sits() {
        let directive = only("import os\n# mypy: ignore-errors\nf(1)\n");
        assert_eq!(directive.scope, Scope::File);
        assert!(directive.rules.is_empty());
        assert_eq!(directive.line, 2);
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
    }

    #[test]
    fn disable_error_code_captures_its_codes_quoted_or_not() {
        assert_eq!(
            only("# mypy: disable-error-code=arg-type\nf(1)\n").rules,
            vec!["arg-type"]
        );
        let quoted = only("# mypy: disable-error-code=\"arg-type, index\"\nf(1)\n");
        assert_eq!(quoted.rules, vec!["arg-type", "index"]);
        assert_eq!(quoted.scope, Scope::File);
    }

    #[test]
    fn a_valueless_disable_error_code_degrades_to_a_blanket_exemption() {
        let directive = only("# mypy: disable-error-code  # oops\nf(1)\n");
        assert!(directive.rules.is_empty());
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(directive.reason.as_deref(), Some("oops"));
    }

    #[test]
    fn a_reason_after_the_config_forms_is_still_reported() {
        // mypy itself chokes on this — it reads the prose as part of the option
        // — but the point of the tool is to surface what someone wrote.
        assert_eq!(
            only("# mypy: ignore-errors  # generated protobuf stubs\nf(1)\n")
                .reason
                .as_deref(),
            Some("generated protobuf stubs")
        );
        assert_eq!(
            only("# mypy: disable-error-code=arg-type  # narrow\nf(1)\n")
                .reason
                .as_deref(),
            Some("narrow")
        );
    }

    #[test]
    fn a_directive_that_does_not_open_the_comment_is_not_mypys() {
        // Real mypy leaves the error in place here, so reporting it would claim
        // a suppression that is not one.
        assert!(parse("x = 1\nf(1)  # noqa: F401  # type: ignore\n").is_empty());
    }

    #[test]
    fn the_config_forms_are_ignored_when_they_trail_code() {
        assert!(parse("f(1)  # mypy: ignore-errors\n").is_empty());
        assert!(parse("f(1)  # mypy: disable-error-code=arg-type\n").is_empty());
    }

    #[test]
    fn look_alike_comments_are_not_directives() {
        assert!(parse("x = 1  # type: int\n").is_empty());
        assert!(parse("x = 1  # TYPE: IGNORE\n").is_empty());
        assert!(parse("x = 1  # type: ignored\n").is_empty());
        assert!(parse("# mypy: strict\nx = 1\n").is_empty());
        assert!(parse("# mypy: ignore-errors-please\nx = 1\n").is_empty());
        assert!(parse("x = 1  # noqa: E501\n").is_empty());
    }

    #[test]
    fn a_type_ignore_inside_a_string_literal_is_not_a_directive() {
        assert!(parse("MSG = \"# type: ignore\"\n").is_empty());
        assert!(parse("DOC = '''\n# type: ignore\n'''\n").is_empty());
    }

    #[test]
    fn every_directive_in_a_file_is_reported_in_source_order() {
        let found =
            parse("# mypy: ignore-errors\nf(1)  # type: ignore\ng(2)  # type: ignore[misc]\n");
        assert_eq!(found.len(), 3);
        assert_eq!(
            found.iter().map(|d| d.line).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(found[0].scope, Scope::File);
    }
}
