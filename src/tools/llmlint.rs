//! llmlint (`llmlint: ignore[rule] reason`) suppression parsing.
//!
//! llmlint is the LLM-judge linter this project mirrors, and its directives are
//! the richest test of the record model: every scope the contract defines, a
//! required native reason, and an explicitly delimited block.
//!
//! | Source | Scope | Rules | Reason |
//! | --- | --- | --- | --- |
//! | `llmlint: ignore[rule] why` | line | `rule` | `why` |
//! | `llmlint: ignore-file[a, b] why` | file | `a`, `b` | `why` |
//! | `llmlint: ignore-block[rule] why` … `llmlint: ignore-end[rule]` | block | `rule` | `why` |
//!
//! The directive is written in whatever comment syntax the host language uses,
//! so this parser claims every language the extractor understands. The keyword
//! is lower-case, takes no space before its colon, and need not open the comment
//! — matching what the real `llmlint check-ignores` accepts.
//!
//! A block record spans from its `ignore-block` line through its
//! `ignore-end` line; the closing directive is part of that record, not one of
//! its own. Blocks are tracked per rule, exactly as llmlint tracks them, so an
//! `ignore-end` naming only some of an open block's rules leaves the rest open.
//! An unclosed block keeps `suppressed.end_line` null **and** raises a report
//! error — llmlint rejects the file, and a range that silently ran to
//! end-of-file would hide that.
//!
//! Reasons are required (except on `ignore-end`, which carries none), so a
//! directive with valid rules and no reason is still reported, with a null
//! reason: llmlint will reject it, and that is precisely what a reviewer wants
//! to see. A directive with no rule list at all is a different matter — llmlint
//! honours nothing without one — so it is not reported.

use crate::comments::Comment;
use crate::model::{normalize_reason, IgnoreDirective, ReportError, Scope, Suppressed, Tool};
use crate::source::SourceFile;
use crate::tools::{Parsed, ToolParser};

/// The keyword every llmlint directive opens with.
const KEYWORD: &str = "llmlint";

/// Parses llmlint's `ignore` family out of any language's comments.
#[derive(Debug, Clone, Copy, Default)]
pub struct LlmlintParser;

impl ToolParser for LlmlintParser {
    fn tool(&self) -> Tool {
        Tool::Llmlint
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        file.language().is_scannable()
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        self.parse_all(file).directives
    }

    fn parse_all(&self, file: &SourceFile) -> Parsed {
        let mut parsed = Parsed::default();
        // One open block per rule, in the order they were opened, so an
        // unclosed-block error lands in source order too.
        let mut open: Vec<(String, usize)> = Vec::new();

        for comment in file.comments() {
            for (line, column, text) in comment_lines(comment) {
                let Some(found) = find_directive(text) else {
                    continue;
                };
                let column = column.saturating_add(prefix_width(text, found.offset));
                if found.verb == Verb::End {
                    close_blocks(&mut parsed, &mut open, &found.rules, line);
                    continue;
                }
                let scope = found.verb.scope();
                if scope == Scope::Block {
                    for rule in &found.rules {
                        if !open.iter().any(|(open_rule, _)| open_rule == rule) {
                            open.push((rule.clone(), parsed.directives.len()));
                        }
                    }
                }
                parsed.directives.push(IgnoreDirective {
                    tool: Tool::Llmlint,
                    scope,
                    rules: found.rules,
                    reason: found.reason,
                    path: file.display_path().to_string(),
                    line,
                    end_line: line,
                    column,
                    raw: found.raw.to_string(),
                    suppressed: suppressed_range(scope, line),
                });
            }
        }

        for (rule, index) in open {
            let line = parsed.directives[index].line;
            parsed.errors.push(ReportError {
                path: file.display_path().to_string(),
                message: format!(
                    "unclosed llmlint ignore-block for rule {rule:?} opened at line {line}; \
                     add a matching `llmlint: ignore-end[{rule}]`"
                ),
            });
        }
        parsed
    }
}

/// Record the end line on every block `rules` closes, dropping them from the
/// open set. A block closes at the last `ignore-end` that names one of its rules.
fn close_blocks(parsed: &mut Parsed, open: &mut Vec<(String, usize)>, rules: &[String], line: u32) {
    open.retain(|(open_rule, index)| {
        if !rules.contains(open_rule) {
            return true;
        }
        let suppressed = &mut parsed.directives[*index].suppressed;
        suppressed.end_line = Some(suppressed.end_line.unwrap_or(line).max(line));
        false
    });
}

fn suppressed_range(scope: Scope, line: u32) -> Suppressed {
    match scope {
        Scope::File => Suppressed {
            start_line: 1,
            end_line: None,
        },
        // An unterminated block keeps its end open; `close_blocks` fills it in.
        Scope::Block => Suppressed {
            start_line: line,
            end_line: None,
        },
        _ => Suppressed {
            start_line: line,
            end_line: Some(line),
        },
    }
}

/// Each physical line of a comment, as `(line, column of the line's first
/// character, text)`.
///
/// A block comment's closing delimiter is dropped so it cannot land in a
/// directive's reason, and the opening one is left in place — the keyword is
/// searched for inside, never at a fixed offset.
fn comment_lines(comment: &Comment) -> Vec<(u32, u32, &str)> {
    let body = comment
        .raw
        .strip_suffix("*/")
        .filter(|_| comment.raw.starts_with("/*"))
        .unwrap_or(&comment.raw);
    body.split('\n')
        .enumerate()
        .map(|(index, text)| {
            let offset = u32::try_from(index).unwrap_or(u32::MAX);
            let column = if index == 0 { comment.column } else { 1 };
            (
                comment.line.saturating_add(offset),
                column,
                text.strip_suffix('\r').unwrap_or(text),
            )
        })
        .collect()
}

/// How many columns of `text` precede the byte offset `at`.
fn prefix_width(text: &str, at: usize) -> u32 {
    u32::try_from(text[..at].chars().count()).unwrap_or(u32::MAX)
}

/// Which `ignore` form a directive uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    /// `ignore` — the line the directive sits on.
    Line,
    /// `ignore-file` — the whole file.
    File,
    /// `ignore-block` — up to the matching `ignore-end`.
    Block,
    /// `ignore-end` — closes an open block; not a record of its own.
    End,
}

impl Verb {
    /// Every verb, longest keyword first so `ignore` cannot shadow
    /// `ignore-file`.
    const ALL: [(&'static str, Verb); 4] = [
        ("ignore-block", Verb::Block),
        ("ignore-file", Verb::File),
        ("ignore-end", Verb::End),
        ("ignore", Verb::Line),
    ];

    fn scope(self) -> Scope {
        match self {
            Verb::Line | Verb::End => Scope::Line,
            Verb::File => Scope::File,
            Verb::Block => Scope::Block,
        }
    }
}

/// One directive found on one line of a comment.
struct Found<'a> {
    verb: Verb,
    rules: Vec<String>,
    reason: Option<String>,
    /// Byte offset of the keyword within the line.
    offset: usize,
    /// The directive as written, from the keyword to the end of the line.
    raw: &'a str,
}

/// The first llmlint directive on `text`, if any.
fn find_directive(text: &str) -> Option<Found<'_>> {
    let mut searched = 0usize;
    while let Some(at) = text[searched..].find(KEYWORD) {
        let start = searched + at;
        let after = &text[start + KEYWORD.len()..];
        if let Some((verb, rules, reason)) = directive_body(after) {
            return Some(Found {
                verb,
                rules,
                reason,
                offset: start,
                raw: text[start..].trim_end(),
            });
        }
        searched = start + KEYWORD.len();
    }
    None
}

/// Parse `: <verb>[<rules>] <reason>` from the text just after the keyword.
fn directive_body(after_keyword: &str) -> Option<(Verb, Vec<String>, Option<String>)> {
    let rest = after_keyword.strip_prefix(':')?.trim_start();
    let (verb, after_verb) = Verb::ALL.iter().find_map(|(name, verb)| {
        let tail = rest.strip_prefix(name)?;
        // `ignore-foo` is not a directive, and neither is `ignoreblock`.
        match tail.chars().next() {
            Some(ch) if ch.is_alphanumeric() || ch == '_' || ch == '-' => None,
            _ => Some((*verb, tail)),
        }
    })?;

    let (rules, after_rules) = after_verb.trim_start().strip_prefix('[')?.split_once(']')?;
    let rules: Vec<String> = rules
        .split(',')
        .map(str::trim)
        .filter(|rule| !rule.is_empty())
        .map(str::to_string)
        .collect();
    // llmlint honours nothing without a named rule, so neither do we.
    if rules.is_empty() {
        return None;
    }
    Some((verb, rules, normalize_reason(after_rules)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_in(path: &str, source: &str) -> Parsed {
        LlmlintParser.parse_all(&SourceFile::new(path, source.to_string()))
    }

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        parse_in("src/app.py", source).directives
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
    fn the_parser_claims_every_language_with_a_comment_grammar() {
        assert_eq!(LlmlintParser.tool(), Tool::Llmlint);
        for name in ["a.py", "a.rs", "a.ts", "a.sh", "a.yml", "a.toml"] {
            assert!(
                LlmlintParser.applies_to(&SourceFile::new(name, String::new())),
                "{name}"
            );
        }
        assert!(!LlmlintParser.applies_to(&SourceFile::new("a.md", String::new())));
    }

    #[test]
    fn an_ignore_covers_the_line_it_sits_on() {
        let directive = only("# llmlint: ignore[no_todo] tracked in issue 42\nx = 1\n");
        assert_eq!(directive.tool, Tool::Llmlint);
        assert_eq!(directive.scope, Scope::Line);
        assert_eq!(directive.rules, vec!["no_todo"]);
        assert_eq!(directive.reason.as_deref(), Some("tracked in issue 42"));
        assert_eq!(directive.path, "src/app.py");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (1, 1, 3)
        );
        assert_eq!(
            directive.raw,
            "llmlint: ignore[no_todo] tracked in issue 42"
        );
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: Some(1)
            }
        );
    }

    #[test]
    fn an_ignore_file_covers_the_whole_file() {
        let directive = only("# llmlint: ignore-file[a, b] generated module\nx = 1\n");
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(directive.rules, vec!["a", "b"]);
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
    }

    #[test]
    fn a_block_spans_from_its_opening_directive_to_its_closing_one() {
        let found = parse(concat!(
            "x = 0\n",
            "# llmlint: ignore-block[no_print] a debugging aid, deliberately\n",
            "print(1)\n",
            "print(2)\n",
            "# llmlint: ignore-end[no_print]\n",
            "y = 1\n",
        ));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].scope, Scope::Block);
        assert_eq!(found[0].rules, vec!["no_print"]);
        assert_eq!(
            found[0].reason.as_deref(),
            Some("a debugging aid, deliberately")
        );
        assert_eq!(
            found[0].suppressed,
            Suppressed {
                start_line: 2,
                end_line: Some(5)
            }
        );
    }

    #[test]
    fn a_block_closes_only_when_every_rule_it_named_is_closed() {
        let parsed = parse_in(
            "src/app.py",
            concat!(
                "# llmlint: ignore-block[a, b] two rules at once\n",
                "x = 1\n",
                "# llmlint: ignore-end[a]\n",
                "y = 2\n",
                "# llmlint: ignore-end[b]\n",
            ),
        );
        assert!(parsed.errors.is_empty(), "{parsed:#?}");
        assert_eq!(parsed.directives.len(), 1);
        assert_eq!(parsed.directives[0].suppressed.end_line, Some(5));
    }

    #[test]
    fn an_unclosed_block_stays_open_and_raises_a_report_error() {
        let parsed = parse_in(
            "src/app.py",
            "# llmlint: ignore-block[no_print] never closed\nprint(1)\n",
        );
        assert_eq!(parsed.directives.len(), 1);
        assert_eq!(
            parsed.directives[0].suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
        assert_eq!(parsed.errors.len(), 1, "{parsed:#?}");
        assert_eq!(parsed.errors[0].path, "src/app.py");
        assert!(
            parsed.errors[0].message.contains("no_print"),
            "{}",
            parsed.errors[0].message
        );
        assert!(
            parsed.errors[0].message.contains("line 1"),
            "{}",
            parsed.errors[0].message
        );
    }

    #[test]
    fn an_ignore_end_with_no_open_block_is_not_a_record() {
        let parsed = parse_in("src/app.py", "# llmlint: ignore-end[no_print]\nx = 1\n");
        assert!(parsed.directives.is_empty(), "{parsed:#?}");
        assert!(parsed.errors.is_empty());
    }

    #[test]
    fn parse_returns_the_directives_parse_all_found() {
        let source = "# llmlint: ignore-block[a] open forever\nx = 1\n";
        let file = SourceFile::new("src/app.py", source.to_string());
        assert_eq!(
            LlmlintParser.parse(&file),
            LlmlintParser.parse_all(&file).directives
        );
    }

    #[test]
    fn directives_are_found_in_every_host_comment_syntax() {
        let slash = LlmlintParser
            .parse(&SourceFile::new(
                "a.ts",
                "// llmlint: ignore[a] a slash comment\nconst x = 1;\n".to_string(),
            ))
            .remove(0);
        assert_eq!(slash.rules, vec!["a"]);
        let directive = LlmlintParser
            .parse(&SourceFile::new(
                "a.rs",
                "/* llmlint: ignore[a] inside a block comment */\nfn f() {}\n".to_string(),
            ))
            .remove(0);
        assert_eq!(directive.reason.as_deref(), Some("inside a block comment"));
        assert_eq!(directive.raw, "llmlint: ignore[a] inside a block comment");
        assert_eq!((directive.line, directive.column), (1, 4));
    }

    #[test]
    fn a_directive_on_an_inner_line_of_a_block_comment_reports_that_line() {
        let directive = LlmlintParser
            .parse(&SourceFile::new(
                "a.rs",
                "/*\n  llmlint: ignore[a] on the second line\n*/\nfn f() {}\n".to_string(),
            ))
            .remove(0);
        assert_eq!((directive.line, directive.column), (2, 3));
        assert_eq!(directive.reason.as_deref(), Some("on the second line"));
    }

    #[test]
    fn the_directive_need_not_open_the_comment() {
        let directive = only("# see the ADR: llmlint: ignore[a] deliberate\nx = 1\n");
        assert_eq!(directive.rules, vec!["a"]);
        assert_eq!(directive.column, 16);
        assert_eq!(directive.raw, "llmlint: ignore[a] deliberate");
    }

    #[test]
    fn whitespace_around_the_verb_is_tolerated_but_the_colon_binds_tight() {
        assert_eq!(only("# llmlint:ignore[a] tight\nx = 1\n").rules, vec!["a"]);
        assert_eq!(
            only("# llmlint:  ignore [a] loose\nx = 1\n").rules,
            vec!["a"]
        );
        assert!(parse("# llmlint : ignore[a] spaced colon\nx = 1\n").is_empty());
    }

    #[test]
    fn the_keyword_and_verb_are_lower_case() {
        assert!(parse("# LLMLINT: ignore[a] shouting\nx = 1\n").is_empty());
        assert!(parse("# llmlint: IGNORE[a] shouting\nx = 1\n").is_empty());
    }

    #[test]
    fn look_alike_verbs_are_not_directives() {
        assert!(parse("# llmlint: ignoreblock[a] no dash\nx = 1\n").is_empty());
        assert!(parse("# llmlint: ignore-foo[a] unknown verb\nx = 1\n").is_empty());
        assert!(parse("# llmlint: ignore no brackets at all\nx = 1\n").is_empty());
        assert!(parse("# llmlint: ignore[] nothing named\nx = 1\n").is_empty());
        assert!(parse("# llmlint: ignore[a unterminated\nx = 1\n").is_empty());
        assert!(parse("# llmlint is a linter we use\nx = 1\n").is_empty());
    }

    #[test]
    fn a_directive_with_no_reason_is_still_reported() {
        let directive = only("# llmlint: ignore[a]\nx = 1\n");
        assert_eq!(directive.rules, vec!["a"]);
        assert_eq!(directive.reason, None);
    }

    #[test]
    fn a_directive_inside_a_string_literal_is_not_reported() {
        assert!(parse("MSG = \"# llmlint: ignore[a] in a string\"\n").is_empty());
    }

    #[test]
    fn a_second_keyword_on_a_line_is_reached_when_the_first_is_prose() {
        let directive = only("# llmlint is fine: llmlint: ignore[a] the real one\nx = 1\n");
        assert_eq!(directive.rules, vec!["a"]);
        assert_eq!(directive.raw, "llmlint: ignore[a] the real one");
    }

    #[test]
    fn every_directive_in_a_file_is_reported_in_source_order() {
        let found = parse(concat!(
            "# llmlint: ignore-file[a] whole module\n",
            "# llmlint: ignore-block[b] a stretch of it\n",
            "x = 1\n",
            "# llmlint: ignore-end[b]\n",
            "y = 2  # llmlint: ignore[c] just here\n",
        ));
        assert_eq!(
            found.iter().map(|d| (d.line, d.scope)).collect::<Vec<_>>(),
            vec![(1, Scope::File), (2, Scope::Block), (5, Scope::Line)]
        );
    }
}
