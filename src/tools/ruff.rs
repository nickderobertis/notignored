//! Ruff (`# noqa`) suppression parsing.
//!
//! Forms understood, mirroring ruff's own directive grammar:
//!
//! | Source | Scope | Rules |
//! | --- | --- | --- |
//! | `# noqa` | line | *(blanket)* |
//! | `# noqa: E501` | line | `E501` |
//! | `# noqa: E501, F401` | line | `E501`, `F401` |
//! | `# ruff: noqa` | file | *(blanket)* |
//! | `# ruff: noqa: E501` | file | `E501` |
//!
//! `noqa` is matched case-insensitively and need not start the comment — ruff
//! honours `# type: ignore  # noqa: E501`, and so do we, reporting the span of
//! the inner `#` that opened the directive. Any trailing `# …` after the codes
//! is captured as the [`reason`](crate::model::IgnoreDirective::reason), up to
//! the next tool's directive: on a shared line each record covers its own
//! directive only (see `src/tools/python.rs::segments`).
//!
//! A directive whose codes don't parse (`# noqa:` with nothing after it) is
//! reported as a **blanket** suppression rather than dropped: the author plainly
//! intended to silence something, and a review tool that swallows it is worse
//! than one that over-reports.

use crate::model::{normalize_reason, IgnoreDirective, Scope, Suppressed, Tool};
use crate::source::{Language, SourceFile};
use crate::tools::python;
use crate::tools::ToolParser;

/// Parses ruff's `# noqa` family out of Python sources.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuffParser;

impl ToolParser for RuffParser {
    fn tool(&self) -> Tool {
        Tool::Ruff
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        file.language() == Language::Python
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        let mut out = Vec::new();
        for comment in file.comments() {
            // Ruff honours at most one directive per comment.
            let found = python::segments(comment).into_iter().find_map(|segment| {
                directive_at(segment.after_hash).map(|parsed| (segment, parsed))
            });
            let Some((segment, (scope, rest))) = found else {
                continue;
            };
            let (rules, reason) = rules_and_reason(rest);
            out.push(IgnoreDirective {
                tool: Tool::Ruff,
                scope,
                rules,
                reason,
                path: file.display_path().to_string(),
                line: comment.line,
                end_line: comment.end_line,
                column: segment.column,
                raw: segment.raw.to_string(),
                suppressed: suppressed_range(scope, comment.line),
                change: None,
            });
        }
        out
    }
}

fn suppressed_range(scope: Scope, line: u32) -> Suppressed {
    match scope {
        // A file-level exemption runs from the first line to end-of-file.
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

/// True when the text after a `#` opens a ruff directive.
///
/// The shared segment scan uses this to bound the run before it; see
/// `src/tools/python.rs::segments`.
pub(super) fn opens_directive(after_hash: &str) -> bool {
    directive_at(after_hash).is_some()
}

/// Recognize a directive immediately after a `#`, returning its scope and the
/// text following the `noqa` keyword.
fn directive_at(after_hash: &str) -> Option<(Scope, &str)> {
    let trimmed = after_hash.trim_start();
    if let Some(after_prefix) = strip_prefix_ci(trimmed, "ruff:") {
        let rest = strip_noqa(after_prefix.trim_start())?;
        return Some((Scope::File, rest));
    }
    strip_noqa(trimmed).map(|rest| (Scope::Line, rest))
}

/// Strip a case-insensitive `noqa` keyword, requiring a word boundary after it
/// so `noqative` never reads as a directive.
fn strip_noqa(input: &str) -> Option<&str> {
    let rest = strip_prefix_ci(input, "noqa")?;
    match rest.chars().next() {
        None => Some(rest),
        Some(ch) if !ch.is_alphanumeric() && ch != '_' => Some(rest),
        Some(_) => None,
    }
}

fn strip_prefix_ci<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    let head = input.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &input[prefix.len()..])
}

/// Split the text after `noqa` into rule codes and the trailing reason.
fn rules_and_reason(rest: &str) -> (Vec<String>, Option<String>) {
    let trimmed = rest.trim_start();
    let (rules, remainder) = match trimmed.strip_prefix(':') {
        Some(after_colon) => parse_codes(after_colon),
        None => (Vec::new(), rest),
    };
    let reason = remainder
        .trim_start()
        .strip_prefix('#')
        .and_then(normalize_reason);
    (rules, reason)
}

/// Consume a comma/whitespace-separated run of rule codes, returning them and
/// the unconsumed remainder.
fn parse_codes(input: &str) -> (Vec<String>, &str) {
    let mut rules = Vec::new();
    let mut remainder = input;
    loop {
        let candidate = remainder.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        let end = candidate
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(candidate.len());
        let token = &candidate[..end];
        if !is_rule_code(token) {
            return (rules, remainder);
        }
        rules.push(token.to_string());
        remainder = &candidate[end..];
    }
}

/// A ruff rule code is a linter prefix followed by a number (`E501`, `PLR0913`).
fn is_rule_code(token: &str) -> bool {
    let letters = token
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .count();
    let digits = token
        .chars()
        .skip(letters)
        .take_while(|c| c.is_ascii_digit())
        .count();
    letters > 0 && digits > 0 && letters + digits == token.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        let file = SourceFile::new("src/app.py", source.to_string());
        RuffParser.parse(&file)
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
        assert_eq!(RuffParser.tool(), Tool::Ruff);
        assert!(RuffParser.applies_to(&SourceFile::new("a.py", String::new())));
        assert!(!RuffParser.applies_to(&SourceFile::new("a.rs", String::new())));
    }

    #[test]
    fn a_bare_noqa_is_a_blanket_line_suppression() {
        let directive = only("x = 1  # noqa\n");
        assert_eq!(directive.tool, Tool::Ruff);
        assert_eq!(directive.scope, Scope::Line);
        assert!(directive.rules.is_empty());
        assert_eq!(directive.reason, None);
        assert_eq!(directive.path, "src/app.py");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (1, 1, 8)
        );
        assert_eq!(directive.raw, "# noqa");
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: Some(1)
            }
        );
    }

    #[test]
    fn a_single_code_is_captured_verbatim() {
        let directive = only("import os  # noqa: F401\n");
        assert_eq!(directive.rules, vec!["F401"]);
        assert_eq!(directive.scope, Scope::Line);
    }

    #[test]
    fn several_codes_split_on_commas_and_whitespace() {
        assert_eq!(
            only("x = 1  # noqa: E501, F401\n").rules,
            vec!["E501", "F401"]
        );
        assert_eq!(
            only("x = 1  # noqa:E501,F401\n").rules,
            vec!["E501", "F401"]
        );
        assert_eq!(
            only("x = 1  # noqa: E501 F401\n").rules,
            vec!["E501", "F401"]
        );
        assert_eq!(only("x = 1  # noqa: PLR0913\n").rules, vec!["PLR0913"]);
    }

    #[test]
    fn a_trailing_comment_becomes_the_reason() {
        let directive = only("u = URL  # noqa: E501  # long wrapped URL\n");
        assert_eq!(directive.rules, vec!["E501"]);
        assert_eq!(directive.reason.as_deref(), Some("long wrapped URL"));
        assert_eq!(directive.raw, "# noqa: E501  # long wrapped URL");
        assert_eq!(directive.column, 10);
    }

    #[test]
    fn a_blanket_directive_can_carry_a_reason_too() {
        let directive = only("x = 1  # noqa  # generated file\n");
        assert!(directive.rules.is_empty());
        assert_eq!(directive.reason.as_deref(), Some("generated file"));
    }

    #[test]
    fn reason_whitespace_is_collapsed_and_an_empty_reason_is_none() {
        assert_eq!(
            only("x = 1  # noqa: E501  #   spaced    out  \n")
                .reason
                .as_deref(),
            Some("spaced out")
        );
        assert_eq!(only("x = 1  # noqa: E501  #   \n").reason, None);
    }

    #[test]
    fn trailing_prose_without_a_hash_is_not_a_reason() {
        let directive = only("x = 1  # noqa: E501 because it is long\n");
        assert_eq!(directive.rules, vec!["E501"]);
        assert_eq!(directive.reason, None);
    }

    #[test]
    fn file_level_directives_run_to_end_of_file() {
        let directive = only("# ruff: noqa\nx = 1\n");
        assert_eq!(directive.scope, Scope::File);
        assert!(directive.rules.is_empty());
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
        assert_eq!(directive.column, 1);
    }

    #[test]
    fn file_level_directives_can_name_codes_and_a_reason() {
        let directive = only("# ruff: noqa: E501  # vendored\nx = 1\n");
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(directive.rules, vec!["E501"]);
        assert_eq!(directive.reason.as_deref(), Some("vendored"));
    }

    #[test]
    fn the_keyword_is_case_insensitive() {
        assert_eq!(only("x = 1  # NOQA: E501\n").rules, vec!["E501"]);
        assert_eq!(only("# RUFF: NoQA\nx = 1\n").scope, Scope::File);
    }

    #[test]
    fn a_directive_need_not_open_the_comment() {
        let directive = only("x = 1  # type: ignore  # noqa: E501\n");
        assert_eq!(directive.rules, vec!["E501"]);
        assert_eq!(directive.raw, "# noqa: E501");
        assert_eq!(directive.column, 24);
    }

    #[test]
    fn look_alike_words_are_not_directives() {
        assert!(parse("x = 1  # noqable\n").is_empty());
        assert!(parse("x = 1  # not a noqa marker\n").is_empty());
        assert!(parse("x = 1  # ruff: format\n").is_empty());
        assert!(parse("x = 1  # ruff: noqable\n").is_empty());
    }

    #[test]
    fn noqa_inside_a_string_literal_is_not_a_directive() {
        assert!(parse("MSG = \"# noqa: E501\"\n").is_empty());
        assert!(parse("DOC = '''\n# noqa\n'''\n").is_empty());
    }

    #[test]
    fn empty_codes_degrade_to_a_blanket_suppression() {
        let directive = only("x = 1  # noqa:\n");
        assert!(directive.rules.is_empty());
        assert_eq!(directive.scope, Scope::Line);
    }

    #[test]
    fn unparseable_codes_stop_the_code_list() {
        let directive = only("x = 1  # noqa: E501, oops\n");
        assert_eq!(directive.rules, vec!["E501"]);
    }

    #[test]
    fn every_directive_in_a_file_is_reported_in_source_order() {
        let found = parse("# ruff: noqa: E501\nimport os  # noqa: F401\nx = 1  # noqa\n");
        assert_eq!(found.len(), 3);
        assert_eq!(
            found.iter().map(|d| d.line).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(found[0].scope, Scope::File);
    }

    #[test]
    fn rule_code_shapes_are_recognized_precisely() {
        assert!(is_rule_code("E501"));
        assert!(is_rule_code("PLR0913"));
        assert!(!is_rule_code(""));
        assert!(!is_rule_code("E"));
        assert!(!is_rule_code("501"));
        assert!(!is_rule_code("E501x"));
    }

    #[test]
    fn a_short_comment_cannot_be_mistaken_for_a_keyword() {
        assert!(parse("x = 1  # no\n").is_empty());
        assert!(parse("x = 1  #\n").is_empty());
    }
}
