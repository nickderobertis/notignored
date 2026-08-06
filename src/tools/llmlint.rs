//! llmlint suppression parsing.
//!
//! llmlint is the LLM-judge linter this project mirrors, and its directives are
//! the richest test of the record model: every scope the contract defines, a
//! required native reason, and an explicitly delimited block.
//!
//! Each form below is written after the [`KEYWORD`] and a colon, inside whatever
//! comment syntax the host language uses — so this parser claims every language
//! the extractor understands.
//!
//! | Form | Scope | Rules | Reason |
//! | --- | --- | --- | --- |
//! | `ignore[rule] why` | line | `rule` | `why` |
//! | `ignore-file[a, b] why` | file | `a`, `b` | `why` |
//! | `ignore-block[rule] why` … `ignore-end[rule]` | block | `rule` | `why` |
//!
//! The keyword is lower-case, takes no space before its colon, and need not open
//! the comment — matching what the real `llmlint check-ignores` accepts.
//!
//! A block record spans from its `ignore-block` line through its
//! `ignore-end` line; the closing directive is part of that record, not one of
//! its own. Blocks are tracked per rule, exactly as llmlint tracks them, so an
//! `ignore-end` naming only some of an open block's rules leaves the rest open.
//! An unclosed block keeps `suppressed.end_line` null **and** raises a report
//! error — llmlint rejects the file, and a range that silently ran to
//! end-of-file would hide that. That error has nowhere to live in an
//! [`IgnoreDirective`], and [`ToolParser`] is a fixed three-method contract, so
//! it rides on the inherent [`LlmlintParser::scan`] and
//! [`crate::scan::scan_files`] folds it into the report.
//!
//! Reasons are required (except on `ignore-end`, which carries none), so a
//! directive with valid rules and no reason is still reported, with a null
//! reason: llmlint will reject it, and that is precisely what a reviewer wants
//! to see. A directive with no rule list at all is a different matter — llmlint
//! honours nothing without one — so it is not reported.
//!
//! **This module never spells a directive out.** `check-ignores` scans raw
//! lines, so a keyword sitting next to a verb anywhere here — a doc table, a
//! test literal — would read as a real (and usually deliberately malformed)
//! suppression of this repo's own rules. Every example is assembled from
//! [`KEYWORD`] instead, which is what keeps this file under llmlint's gate
//! rather than exempt from it.

use crate::comments::Comment;
use crate::model::{normalize_reason, IgnoreDirective, ReportError, Scope, Suppressed, Tool};
use crate::source::SourceFile;
use crate::tools::ToolParser;

/// The keyword every llmlint directive opens with.
pub const KEYWORD: &str = "llmlint";

/// Everything [`LlmlintParser::scan`] found in one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scanned {
    /// Directives, in source order.
    pub directives: Vec<IgnoreDirective>,
    /// Blocks left open at end-of-file. The directive is still reported, with
    /// its end unknown; this is the part of the defect a record cannot carry.
    pub errors: Vec<ReportError>,
}

/// Parses llmlint's `ignore` family out of any language's comments.
#[derive(Debug, Clone, Copy, Default)]
pub struct LlmlintParser;

impl LlmlintParser {
    /// Every directive in `file`, plus the blocks left unclosed.
    ///
    /// Inherent rather than a [`ToolParser`] method: that trait is a fixed
    /// contract that hands back directives and nothing else, and llmlint is the
    /// only tool here with a defect an [`IgnoreDirective`] cannot express.
    /// [`ToolParser::parse`] is this, minus the errors.
    pub fn scan(&self, file: &SourceFile) -> Scanned {
        let mut scanned = Scanned::default();
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
                    close_blocks(&mut scanned, &mut open, &found.rules, line);
                    continue;
                }
                let scope = found.verb.scope();
                if scope == Scope::Block {
                    for rule in &found.rules {
                        if !open.iter().any(|(open_rule, _)| open_rule == rule) {
                            open.push((rule.clone(), scanned.directives.len()));
                        }
                    }
                }
                scanned.directives.push(IgnoreDirective {
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
            let line = scanned.directives[index].line;
            scanned.errors.push(ReportError {
                path: file.display_path().to_string(),
                message: format!(
                    "unclosed {KEYWORD} ignore-block for rule {rule:?} opened at line {line}; \
                     add a matching `{KEYWORD}: ignore-end[{rule}]`"
                ),
            });
        }
        scanned
    }
}

impl ToolParser for LlmlintParser {
    fn tool(&self) -> Tool {
        Tool::Llmlint
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        file.language().is_scannable()
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        self.scan(file).directives
    }
}

/// Record the end line on every block `rules` closes, dropping them from the
/// open set. A block closes at the last `ignore-end` that names one of its rules.
fn close_blocks(
    scanned: &mut Scanned,
    open: &mut Vec<(String, usize)>,
    rules: &[String],
    line: u32,
) {
    open.retain(|(open_rule, index)| {
        if !rules.contains(open_rule) {
            return true;
        }
        let suppressed = &mut scanned.directives[*index].suppressed;
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

    /// A `#` comment carrying `body`, assembled from [`KEYWORD`] rather than
    /// written out — see the "never spells a directive out" note in the module
    /// docs for why every fixture below is built this way.
    fn hash(body: &str) -> String {
        format!("# {KEYWORD}: {body}")
    }

    /// A source file whose lines are `lines`, newline-terminated.
    fn script(lines: &[&str]) -> String {
        format!("{}\n", lines.join("\n"))
    }

    fn scan_in(path: &str, source: &str) -> Scanned {
        LlmlintParser.scan(&SourceFile::new(path, source.to_string()))
    }

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        scan_in("src/app.py", source).directives
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

    /// The single directive in a one-line `#` comment followed by one statement.
    fn only_hash(body: &str) -> IgnoreDirective {
        only(&script(&[&hash(body), "x = 1"]))
    }

    fn parse_hash(body: &str) -> Vec<IgnoreDirective> {
        parse(&script(&[&hash(body), "x = 1"]))
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
        let directive = only_hash("ignore[no_todo] tracked in issue 42");
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
            format!("{KEYWORD}: ignore[no_todo] tracked in issue 42")
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
        let directive = only_hash("ignore-file[a, b] generated module");
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
        let found = parse(&script(&[
            "x = 0",
            &hash("ignore-block[no_print] a debugging aid, deliberately"),
            "print(1)",
            "print(2)",
            &hash("ignore-end[no_print]"),
            "y = 1",
        ]));
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
        let scanned = scan_in(
            "src/app.py",
            &script(&[
                &hash("ignore-block[a, b] two rules at once"),
                "x = 1",
                &hash("ignore-end[a]"),
                "y = 2",
                &hash("ignore-end[b]"),
            ]),
        );
        assert!(scanned.errors.is_empty(), "{scanned:#?}");
        assert_eq!(scanned.directives.len(), 1);
        assert_eq!(scanned.directives[0].suppressed.end_line, Some(5));
    }

    #[test]
    fn an_unclosed_block_stays_open_and_raises_a_report_error() {
        let scanned = scan_in(
            "src/app.py",
            &script(&[&hash("ignore-block[no_print] never closed"), "print(1)"]),
        );
        assert_eq!(scanned.directives.len(), 1);
        assert_eq!(
            scanned.directives[0].suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
        assert_eq!(scanned.errors.len(), 1, "{scanned:#?}");
        assert_eq!(scanned.errors[0].path, "src/app.py");
        let message = &scanned.errors[0].message;
        assert!(message.contains("no_print"), "{message}");
        assert!(message.contains("line 1"), "{message}");
    }

    #[test]
    fn an_ignore_end_with_no_open_block_is_not_a_record() {
        let scanned = scan_in(
            "src/app.py",
            &script(&[&hash("ignore-end[no_print]"), "x = 1"]),
        );
        assert!(scanned.directives.is_empty(), "{scanned:#?}");
        assert!(scanned.errors.is_empty());
    }

    #[test]
    fn the_trait_method_is_the_scan_without_its_errors() {
        let source = script(&[&hash("ignore-block[a] open forever"), "x = 1"]);
        let file = SourceFile::new("src/app.py", source);
        let scanned = LlmlintParser.scan(&file);
        assert_eq!(LlmlintParser.parse(&file), scanned.directives);
        assert_eq!(scanned.errors.len(), 1);
    }

    #[test]
    fn directives_are_found_in_every_host_comment_syntax() {
        let slash = LlmlintParser
            .parse(&SourceFile::new(
                "a.ts",
                script(&[
                    &format!("// {KEYWORD}: ignore[a] a slash comment"),
                    "const x = 1;",
                ]),
            ))
            .remove(0);
        assert_eq!(slash.rules, vec!["a"]);

        let directive = LlmlintParser
            .parse(&SourceFile::new(
                "a.rs",
                script(&[
                    &format!("/* {KEYWORD}: ignore[a] inside a block comment */"),
                    "fn f() {}",
                ]),
            ))
            .remove(0);
        assert_eq!(directive.reason.as_deref(), Some("inside a block comment"));
        assert_eq!(
            directive.raw,
            format!("{KEYWORD}: ignore[a] inside a block comment")
        );
        assert_eq!((directive.line, directive.column), (1, 4));
    }

    #[test]
    fn a_directive_on_an_inner_line_of_a_block_comment_reports_that_line() {
        let directive = LlmlintParser
            .parse(&SourceFile::new(
                "a.rs",
                script(&[
                    "/*",
                    &format!("  {KEYWORD}: ignore[a] on the second line"),
                    "*/",
                    "fn f() {}",
                ]),
            ))
            .remove(0);
        assert_eq!((directive.line, directive.column), (2, 3));
        assert_eq!(directive.reason.as_deref(), Some("on the second line"));
    }

    #[test]
    fn the_directive_need_not_open_the_comment() {
        let directive = only(&script(&[
            &format!("# see the ADR: {KEYWORD}: ignore[a] deliberate"),
            "x = 1",
        ]));
        assert_eq!(directive.rules, vec!["a"]);
        assert_eq!(directive.column, 16);
        assert_eq!(directive.raw, format!("{KEYWORD}: ignore[a] deliberate"));
    }

    #[test]
    fn whitespace_around_the_verb_is_tolerated_but_the_colon_binds_tight() {
        assert_eq!(
            only(&script(&[&format!("# {KEYWORD}:ignore[a] tight"), "x = 1"])).rules,
            vec!["a"]
        );
        assert_eq!(only_hash(" ignore [a] loose").rules, vec!["a"]);
        assert!(parse(&script(&[
            &format!("# {KEYWORD} : ignore[a] spaced"),
            "x = 1"
        ]))
        .is_empty());
    }

    #[test]
    fn the_keyword_and_verb_are_lower_case() {
        let shouted = KEYWORD.to_uppercase();
        assert!(parse(&script(&[
            &format!("# {shouted}: ignore[a] shouting"),
            "x = 1"
        ]))
        .is_empty());
        assert!(parse_hash("IGNORE[a] shouting").is_empty());
    }

    #[test]
    fn look_alike_verbs_are_not_directives() {
        assert!(parse_hash("ignoreblock[a] no dash").is_empty());
        assert!(parse_hash("ignore-foo[a] unknown verb").is_empty());
        assert!(parse_hash("ignore no brackets at all").is_empty());
        assert!(parse_hash("ignore[] nothing named").is_empty());
        assert!(parse_hash("ignore[a unterminated").is_empty());
        assert!(parse(&script(&[
            &format!("# {KEYWORD} is a linter we use"),
            "x = 1"
        ]))
        .is_empty());
    }

    #[test]
    fn a_directive_with_no_reason_is_still_reported() {
        let directive = only_hash("ignore[a]");
        assert_eq!(directive.rules, vec!["a"]);
        assert_eq!(directive.reason, None);
    }

    #[test]
    fn a_directive_inside_a_string_literal_is_not_reported() {
        let source = script(&[&format!("MSG = \"# {KEYWORD}: ignore[a] in a string\"")]);
        assert!(parse(&source).is_empty(), "{source}");
    }

    #[test]
    fn a_second_keyword_on_a_line_is_reached_when_the_first_is_prose() {
        let directive = only(&script(&[
            &format!("# {KEYWORD} is fine: {KEYWORD}: ignore[a] the real one"),
            "x = 1",
        ]));
        assert_eq!(directive.rules, vec!["a"]);
        assert_eq!(directive.raw, format!("{KEYWORD}: ignore[a] the real one"));
    }

    #[test]
    fn every_directive_in_a_file_is_reported_in_source_order() {
        let found = parse(&script(&[
            &hash("ignore-file[a] whole module"),
            &hash("ignore-block[b] a stretch of it"),
            "x = 1",
            &hash("ignore-end[b]"),
            &format!("y = 2  # {KEYWORD}: ignore[c] just here"),
        ]));
        assert_eq!(
            found.iter().map(|d| (d.line, d.scope)).collect::<Vec<_>>(),
            vec![(1, Scope::File), (2, Scope::Block), (5, Scope::Line)]
        );
    }
}
