//! ESLint (`// eslint-disable-next-line`) suppression parsing.
//!
//! Forms understood, mirroring ESLint's own directive grammar:
//!
//! | Source | Scope | Rules |
//! | --- | --- | --- |
//! | `// eslint-disable-line` | line | *(blanket)* |
//! | `// eslint-disable-line no-alert` | line | `no-alert` |
//! | `// eslint-disable-next-line no-alert, no-console` | next-line | both |
//! | `/* eslint-disable */` … `/* eslint-enable */` | block | *(blanket)* |
//! | `/* eslint-disable no-alert */` … | block | `no-alert` |
//!
//! Four details are copied from ESLint rather than guessed, because each one
//! decides whether a directive is real:
//!
//! * **`eslint-disable` and `eslint-enable` are block comments only.**
//!   `// eslint-disable` is an ordinary comment to ESLint, so it is one here too.
//!   `eslint-disable-line` and `eslint-disable-next-line` work in either form.
//! * **The directive must open the comment.** `// TODO eslint-disable-next-line`
//!   silences nothing.
//! * **Rules split on commas only.** `no-alert no-console` is a single (bogus)
//!   rule id to ESLint, and is reported as written.
//! * **The reason is the ` -- ` description**, matched as ESLint matches it:
//!   whitespace, two or more dashes, whitespace. `no-alert--nope` keeps its
//!   dashes and stays part of the rule id.
//!
//! `eslint-enable` closes a block rather than opening one, so it is not itself
//! reported. A disable is closed when every rule it named is back on — a blanket
//! enable, or one naming all of them. That is best-effort by construction:
//! `/* eslint-disable a, b */ … /* eslint-enable a */` leaves `b` off, so the
//! block runs on, and the reported range over-covers `a`. Over-covering is the
//! safe direction for a review tool; silently ending the block early is not.

use crate::comments::{Comment, CommentKind};
use crate::model::{normalize_reason, IgnoreDirective, Scope, Suppressed, Tool};
use crate::source::{Language, SourceFile};
use crate::tools::ToolParser;

/// Parses ESLint's `eslint-disable` family out of JavaScript and TypeScript.
#[derive(Debug, Clone, Copy, Default)]
pub struct EslintParser;

/// The four directive keywords, longest first so `eslint-disable-next-line` is
/// never truncated to `eslint-disable`.
const KEYWORDS: [(&str, Kind); 4] = [
    ("eslint-disable-next-line", Kind::DisableNextLine),
    ("eslint-disable-line", Kind::DisableLine),
    ("eslint-disable", Kind::Disable),
    ("eslint-enable", Kind::Enable),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Disable,
    Enable,
    DisableLine,
    DisableNextLine,
}

impl Kind {
    /// `eslint-disable` / `eslint-enable` are honoured in block comments only.
    fn allowed_in(self, kind: CommentKind) -> bool {
        match self {
            Kind::Disable | Kind::Enable => kind == CommentKind::Block,
            Kind::DisableLine | Kind::DisableNextLine => true,
        }
    }
}

impl ToolParser for EslintParser {
    fn tool(&self) -> Tool {
        Tool::Eslint
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        matches!(file.language(), Language::JavaScript | Language::TypeScript)
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        let mut out: Vec<IgnoreDirective> = Vec::new();
        // Indices into `out` for `eslint-disable` blocks still waiting on their
        // `eslint-enable`, oldest first.
        let mut open: Vec<usize> = Vec::new();

        for comment in file.comments() {
            let Some((kind, rules, reason)) = directive(&comment.text, comment.kind) else {
                continue;
            };
            if kind == Kind::Enable {
                close_blocks(&mut out, &mut open, &rules, comment.line);
                continue;
            }
            if kind == Kind::Disable {
                open.push(out.len());
            }
            out.push(IgnoreDirective {
                tool: Tool::Eslint,
                scope: scope(kind),
                rules,
                reason,
                path: file.display_path().to_string(),
                line: comment.line,
                end_line: comment.end_line,
                column: comment.column,
                raw: comment.raw.clone(),
                suppressed: suppressed_range(kind, comment),
            });
        }
        out
    }
}

fn scope(kind: Kind) -> Scope {
    match kind {
        Kind::Disable => Scope::Block,
        Kind::DisableLine => Scope::Line,
        Kind::DisableNextLine => Scope::NextLine,
        // Never reported; an enable closes a block instead of opening one.
        Kind::Enable => Scope::Block,
    }
}

fn suppressed_range(kind: Kind, comment: &Comment) -> Suppressed {
    match kind {
        // ESLint applies a `disable-line` to the line the comment *starts* on,
        // and a `disable-next-line` to the line after it *ends* on.
        Kind::DisableLine => Suppressed {
            start_line: comment.line,
            end_line: Some(comment.line),
        },
        Kind::DisableNextLine => {
            let next = comment.end_line.saturating_add(1);
            Suppressed {
                start_line: next,
                end_line: Some(next),
            }
        }
        // Left open until an `eslint-enable` closes it; unterminated means
        // end-of-file.
        _ => Suppressed {
            start_line: comment.line,
            end_line: None,
        },
    }
}

/// Close every open block that `enable_rules` puts fully back on.
fn close_blocks(
    out: &mut [IgnoreDirective],
    open: &mut Vec<usize>,
    enable_rules: &[String],
    line: u32,
) {
    open.retain(|&index| {
        if !closes(&out[index].rules, enable_rules) {
            return true;
        }
        out[index].suppressed.end_line = Some(line);
        false
    });
}

/// Whether an `eslint-enable` naming `enable_rules` re-enables everything a
/// disable naming `disable_rules` silenced.
fn closes(disable_rules: &[String], enable_rules: &[String]) -> bool {
    if enable_rules.is_empty() {
        // A blanket enable turns everything back on.
        return true;
    }
    if disable_rules.is_empty() {
        // A blanket disable covers rules the enable never names.
        return false;
    }
    disable_rules.iter().all(|rule| enable_rules.contains(rule))
}

/// True when the text after a `//` opens an ESLint directive.
///
/// A line comment is what the caller has, so the block-only forms are excluded
/// exactly as [`Kind::allowed_in`] excludes them; see
/// [`crate::tools::opens_directive`].
pub(super) fn starts_with_directive(after_marker: &str) -> bool {
    directive(after_marker, CommentKind::Line).is_some()
}

/// Recognize a directive that opens a comment whose body is `text`, returning
/// its kind, rules and reason.
fn directive(text: &str, comment_kind: CommentKind) -> Option<(Kind, Vec<String>, Option<String>)> {
    let (head, description) = split_description(text);
    let head = head.trim();
    let (kind, rest) = KEYWORDS.iter().find_map(|&(keyword, kind)| {
        let rest = head.strip_prefix(keyword)?;
        // A word boundary, so `eslint-disabled` is not a directive.
        (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some((kind, rest))
    })?;
    if !kind.allowed_in(comment_kind) {
        return None;
    }
    let rules = rest
        .split(',')
        .map(str::trim)
        .filter(|rule| !rule.is_empty())
        .map(str::to_string)
        .collect();
    Some((kind, rules, description.and_then(normalize_reason)))
}

/// Split a directive comment at ESLint's ` -- ` description separator.
///
/// Mirrors ESLint's own `/\s-{2,}\s/u`: whitespace, two or more dashes, then
/// whitespace. Anything else — `--` at the very end, or `no-alert--nope` — is
/// not a separator, exactly as ESLint reads it.
fn split_description(text: &str) -> (&str, Option<&str>) {
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("--") {
        let start = cursor + offset;
        let dashes = text[start..].chars().take_while(|c| *c == '-').count();
        let end = start + dashes;
        let spaced = text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
            && text[end..].chars().next().is_some_and(char::is_whitespace);
        if spaced {
            return (&text[..start], Some(&text[end..]));
        }
        cursor = start + 1;
    }
    (text, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        let file = SourceFile::new("src/app.ts", source.to_string());
        EslintParser.parse(&file)
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
    fn the_parser_claims_javascript_and_typescript() {
        assert_eq!(EslintParser.tool(), Tool::Eslint);
        assert!(EslintParser.applies_to(&SourceFile::new("a.js", String::new())));
        assert!(EslintParser.applies_to(&SourceFile::new("a.tsx", String::new())));
        assert!(!EslintParser.applies_to(&SourceFile::new("a.py", String::new())));
    }

    #[test]
    fn a_next_line_directive_carries_its_rule_reason_and_span() {
        let directive = only("// eslint-disable-next-line no-console -- debugging aid\nlog();\n");
        assert_eq!(directive.tool, Tool::Eslint);
        assert_eq!(directive.scope, Scope::NextLine);
        assert_eq!(directive.rules, vec!["no-console"]);
        assert_eq!(directive.reason.as_deref(), Some("debugging aid"));
        assert_eq!(directive.path, "src/app.ts");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (1, 1, 1)
        );
        assert_eq!(
            directive.raw,
            "// eslint-disable-next-line no-console -- debugging aid"
        );
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 2,
                end_line: Some(2)
            }
        );
    }

    #[test]
    fn a_disable_line_directive_covers_its_own_line() {
        let directive = only("log(); // eslint-disable-line no-console\n");
        assert_eq!(directive.scope, Scope::Line);
        assert_eq!(directive.column, 8);
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: Some(1)
            }
        );
    }

    #[test]
    fn rules_split_on_commas_only() {
        assert_eq!(
            only("// eslint-disable-next-line no-alert, no-console\nx;\n").rules,
            vec!["no-alert", "no-console"]
        );
        assert_eq!(
            only("// eslint-disable-next-line no-alert,no-console\nx;\n").rules,
            vec!["no-alert", "no-console"]
        );
        // ESLint reads this as one (nonexistent) rule id, and so do we.
        assert_eq!(
            only("// eslint-disable-next-line no-alert no-console\nx;\n").rules,
            vec!["no-alert no-console"]
        );
        // Plugin rule ids keep their slashes and scopes.
        assert_eq!(
            only("// eslint-disable-next-line @typescript-eslint/no-explicit-any\nx;\n").rules,
            vec!["@typescript-eslint/no-explicit-any"]
        );
    }

    #[test]
    fn a_directive_with_no_rules_is_blanket() {
        let directive = only("// eslint-disable-next-line\nx;\n");
        assert!(directive.rules.is_empty());
        assert_eq!(directive.reason, None);
    }

    #[test]
    fn the_description_separator_needs_whitespace_and_two_dashes() {
        assert_eq!(
            only("// eslint-disable-next-line no-console ---- long dashes\nx;\n")
                .reason
                .as_deref(),
            Some("long dashes")
        );
        // No trailing whitespace after the dashes: ESLint keeps them in the rule
        // id, so the reason is none and the rule is reported as written.
        let unseparated = only("// eslint-disable-next-line no-console --\nx;\n");
        assert_eq!(unseparated.reason, None);
        assert_eq!(unseparated.rules, vec!["no-console --"]);
        let glued = only("// eslint-disable-line no-console--nope\nx;\n");
        assert_eq!(glued.rules, vec!["no-console--nope"]);
        assert_eq!(glued.reason, None);
        // An empty description is no description.
        assert_eq!(
            only("// eslint-disable-next-line no-console -- \nx;\n").reason,
            None
        );
    }

    #[test]
    fn a_block_comment_reason_may_span_lines() {
        let directive = only(concat!(
            "/* eslint-disable-next-line no-console\n",
            "   -- a reason that spans\n",
            "      several lines */\n",
            "log();\n",
        ));
        assert_eq!(directive.rules, vec!["no-console"]);
        assert_eq!(
            directive.reason.as_deref(),
            Some("a reason that spans several lines")
        );
        assert_eq!((directive.line, directive.end_line), (1, 3));
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 4,
                end_line: Some(4)
            }
        );
    }

    #[test]
    fn disable_and_enable_pair_into_a_block() {
        let directive = only(concat!(
            "/* eslint-disable no-console -- noisy module */\n",
            "log();\n",
            "/* eslint-enable no-console */\n",
            "log();\n",
        ));
        assert_eq!(directive.scope, Scope::Block);
        assert_eq!(directive.rules, vec!["no-console"]);
        assert_eq!(directive.reason.as_deref(), Some("noisy module"));
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: Some(3)
            }
        );
    }

    #[test]
    fn an_unterminated_disable_runs_to_end_of_file() {
        let directive = only("/* eslint-disable */\nlog();\n");
        assert_eq!(directive.scope, Scope::Block);
        assert!(directive.rules.is_empty());
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
    }

    #[test]
    fn a_blanket_enable_closes_every_open_block() {
        let found = parse(concat!(
            "/* eslint-disable no-console */\n",
            "/* eslint-disable no-alert */\n",
            "log();\n",
            "/* eslint-enable */\n",
        ));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].suppressed.end_line, Some(4));
        assert_eq!(found[1].suppressed.end_line, Some(4));
    }

    #[test]
    fn a_partial_enable_leaves_the_block_open() {
        // ESLint keeps `no-alert` disabled here, so the block has not ended.
        let directive = only(concat!(
            "/* eslint-disable no-console, no-alert */\n",
            "/* eslint-enable no-console */\n",
            "log();\n",
        ));
        assert_eq!(directive.suppressed.end_line, None);
    }

    #[test]
    fn a_named_enable_does_not_close_a_blanket_disable() {
        let directive = only("/* eslint-disable */\n/* eslint-enable no-console */\nx;\n");
        assert_eq!(directive.suppressed.end_line, None);
    }

    #[test]
    fn an_enable_without_a_block_is_ignored() {
        assert!(parse("/* eslint-enable no-console */\nx;\n").is_empty());
    }

    #[test]
    fn disable_and_enable_are_block_comments_only() {
        // ESLint does not honour these in a line comment, so neither do we.
        assert!(parse("// eslint-disable\nlog();\n").is_empty());
        assert!(parse("// eslint-enable\nlog();\n").is_empty());
        // …but the line-scoped pair works in either comment form.
        assert_eq!(
            only("/* eslint-disable-line no-console */ log();\n").scope,
            Scope::Line
        );
    }

    #[test]
    fn a_directive_that_does_not_open_the_comment_is_not_one() {
        assert!(parse("// TODO eslint-disable-next-line no-console\nlog();\n").is_empty());
        assert!(parse("// eslint-disabled no-console\nlog();\n").is_empty());
        // The `/* eslint rule: "error" */` configuration comment is not a
        // suppression.
        assert!(parse("/* eslint no-console: \"error\" */\nlog();\n").is_empty());
    }

    #[test]
    fn a_directive_inside_a_string_literal_is_not_reported() {
        assert!(parse("const m = \"// eslint-disable-next-line no-console\";\n").is_empty());
        assert!(parse("const m = `/* eslint-disable */`;\n").is_empty());
    }

    #[test]
    fn every_directive_is_reported_in_source_order() {
        let found = parse(concat!(
            "/* eslint-disable no-console */\n",
            "log(); // eslint-disable-line no-alert\n",
            "// eslint-disable-next-line no-alert\n",
            "alert();\n",
        ));
        assert_eq!(
            found.iter().map(|d| (d.line, d.scope)).collect::<Vec<_>>(),
            vec![(1, Scope::Block), (2, Scope::Line), (3, Scope::NextLine)]
        );
    }

    #[test]
    fn the_description_split_finds_the_first_separator_only() {
        assert_eq!(split_description("a -- b -- c"), ("a ", Some(" b -- c")));
        assert_eq!(split_description("a--b"), ("a--b", None));
        assert_eq!(split_description("a -"), ("a -", None));
        assert_eq!(split_description("--lead"), ("--lead", None));
    }
}
