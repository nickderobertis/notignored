//! Biome (`// biome-ignore lint/…`) suppression parsing.
//!
//! Forms understood, mirroring Biome's own directive grammar:
//!
//! | Source | Scope | Rules |
//! | --- | --- | --- |
//! | `// biome-ignore lint/style/useConst: reason` | next-line | `lint/style/useConst` |
//! | `// biome-ignore lint/a/x lint/b/y: reason` | next-line | both |
//! | `// biome-ignore-all lint/style/useConst: reason` | file | `lint/style/useConst` |
//! | `// biome-ignore-start lint/…: reason` … `// biome-ignore-end lint/…: reason` | block | as named |
//!
//! Each form also works as a `/* … */` comment, whose reason may span lines.
//!
//! Biome's grammar differs from ESLint's in three ways that matter here:
//!
//! * **A reason is mandatory.** `// biome-ignore lint/style/useConst` with no
//!   `:` is a Biome *error*, not a suppression, so it is not reported as one.
//! * **Rule selectors split on whitespace, not commas**, and are captured whole:
//!   `lint/style/useConst`, the group `lint/style`, and the domain `lint` are all
//!   valid and all reported exactly as written.
//! * **The suppression applies to the next line.** A trailing
//!   `debugger; // biome-ignore …` silences nothing, so it is not a directive.
//!
//! `biome-ignore-end` closes a range rather than opening one, so it is not itself
//! reported. It closes the most recent open `biome-ignore-start` that names one
//! of the same selectors — Biome matches them by selector too, and leaves the
//! range open (warning about it) when nothing matches.

use crate::comments::Comment;
use crate::model::{normalize_reason, IgnoreDirective, Scope, Suppressed, Tool};
use crate::source::{Language, SourceFile};
use crate::tools::ToolParser;

/// Parses Biome's `biome-ignore` family out of JavaScript and TypeScript.
#[derive(Debug, Clone, Copy, Default)]
pub struct BiomeParser;

/// The four directive keywords, longest first so `biome-ignore-start` is never
/// truncated to `biome-ignore`.
const KEYWORDS: [(&str, Kind); 4] = [
    ("biome-ignore-start", Kind::Start),
    ("biome-ignore-end", Kind::End),
    ("biome-ignore-all", Kind::All),
    ("biome-ignore", Kind::One),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `biome-ignore` — the next line.
    One,
    /// `biome-ignore-all` — the whole file.
    All,
    /// `biome-ignore-start` — until the matching end.
    Start,
    /// `biome-ignore-end` — closes a range.
    End,
}

impl ToolParser for BiomeParser {
    fn tool(&self) -> Tool {
        Tool::Biome
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        matches!(file.language(), Language::JavaScript | Language::TypeScript)
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        let mut out: Vec<IgnoreDirective> = Vec::new();
        // Indices into `out` for ranges still waiting on their end, oldest first.
        let mut open: Vec<usize> = Vec::new();

        for comment in file.comments() {
            let Some((kind, rules, reason)) = directive(&comment.text) else {
                continue;
            };
            if kind == Kind::End {
                close_range(&mut out, &mut open, &rules, comment.line);
                continue;
            }
            if kind == Kind::Start {
                open.push(out.len());
            }
            out.push(IgnoreDirective {
                tool: Tool::Biome,
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
        Kind::One => Scope::NextLine,
        Kind::All => Scope::File,
        // `End` never reaches a record; it closes the `Start` it matches.
        Kind::Start | Kind::End => Scope::Block,
    }
}

fn suppressed_range(kind: Kind, comment: &Comment) -> Suppressed {
    match kind {
        Kind::One => {
            let next = comment.end_line.saturating_add(1);
            Suppressed {
                start_line: next,
                end_line: Some(next),
            }
        }
        Kind::All => Suppressed {
            start_line: 1,
            end_line: None,
        },
        // Left open until a matching `biome-ignore-end`; unterminated means
        // end-of-file, which Biome honours while warning about it.
        Kind::Start | Kind::End => Suppressed {
            start_line: comment.line,
            end_line: None,
        },
    }
}

/// Close the most recent open range this end directive matches.
fn close_range(out: &mut [IgnoreDirective], open: &mut Vec<usize>, rules: &[String], line: u32) {
    let Some(position) = open
        .iter()
        .rposition(|&index| out[index].rules.iter().any(|rule| rules.contains(rule)))
    else {
        // Biome reports an unmatched end as a problem and changes nothing.
        return;
    };
    out[open.remove(position)].suppressed.end_line = Some(line);
}

/// True when the text after a comment marker opens a Biome suppression.
///
/// The line-below boundary uses this; see [`crate::tools::opens_directive`].
pub(super) fn starts_with_directive(after_marker: &str) -> bool {
    directive(after_marker).is_some()
}

/// Recognize a directive that opens a comment whose body is `text`, returning
/// its kind, rule selectors and reason.
///
/// Returns `None` for a directive with no `: reason`, which Biome rejects.
fn directive(text: &str) -> Option<(Kind, Vec<String>, Option<String>)> {
    let text = text.trim_start();
    let (kind, rest) = KEYWORDS.iter().find_map(|&(keyword, kind)| {
        let rest = text.strip_prefix(keyword)?;
        // A word boundary, so `biome-ignoreable` is not a directive.
        (rest.is_empty() || rest.starts_with([' ', '\t', '\r', '\n', ':'])).then_some((kind, rest))
    })?;
    let (selectors, reason) = rest.split_once(':')?;
    let rules = selectors
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    // Biome silences nothing without a selector and errors without an
    // explanation, so neither degrades to a blanket suppression here.
    if rules.is_empty() {
        return None;
    }
    let reason = normalize_reason(reason)?;
    Some((kind, rules, Some(reason)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        let file = SourceFile::new("src/app.ts", source.to_string());
        BiomeParser.parse(&file)
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
        assert_eq!(BiomeParser.tool(), Tool::Biome);
        assert!(BiomeParser.applies_to(&SourceFile::new("a.js", String::new())));
        assert!(BiomeParser.applies_to(&SourceFile::new("a.mts", String::new())));
        assert!(!BiomeParser.applies_to(&SourceFile::new("a.rs", String::new())));
    }

    #[test]
    fn a_line_directive_carries_its_selector_reason_and_span() {
        let directive =
            only("// biome-ignore lint/suspicious/noDebugger: paused on purpose\ndebugger;\n");
        assert_eq!(directive.tool, Tool::Biome);
        assert_eq!(directive.scope, Scope::NextLine);
        assert_eq!(directive.rules, vec!["lint/suspicious/noDebugger"]);
        assert_eq!(directive.reason.as_deref(), Some("paused on purpose"));
        assert_eq!(directive.path, "src/app.ts");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (1, 1, 1)
        );
        assert_eq!(
            directive.raw,
            "// biome-ignore lint/suspicious/noDebugger: paused on purpose"
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
    fn selectors_split_on_whitespace_and_keep_their_shape() {
        assert_eq!(
            only("// biome-ignore lint/a/x lint/b/y: two\nx;\n").rules,
            vec!["lint/a/x", "lint/b/y"]
        );
        // A group and a whole domain are valid selectors too.
        assert_eq!(
            only("// biome-ignore lint/suspicious: group\nx;\n").rules,
            vec!["lint/suspicious"]
        );
        assert_eq!(
            only("// biome-ignore lint: domain\nx;\n").rules,
            vec!["lint"]
        );
        // Biome tolerates space before the colon.
        assert_eq!(
            only("// biome-ignore lint/a/x : spaced\nx;\n").rules,
            vec!["lint/a/x"]
        );
    }

    #[test]
    fn a_directive_without_a_reason_is_not_a_suppression() {
        // Biome errors on both of these rather than suppressing anything.
        assert!(parse("// biome-ignore lint/suspicious/noDebugger\ndebugger;\n").is_empty());
        assert!(parse("// biome-ignore lint/suspicious/noDebugger:\ndebugger;\n").is_empty());
        assert!(parse("// biome-ignore lint/suspicious/noDebugger:   \ndebugger;\n").is_empty());
    }

    #[test]
    fn a_block_comment_reason_may_span_lines() {
        let directive = only(concat!(
            "/* biome-ignore lint/suspicious/noDebugger: a reason\n",
            "   that spans lines */\n",
            "debugger;\n",
        ));
        assert_eq!(
            directive.reason.as_deref(),
            Some("a reason that spans lines")
        );
        assert_eq!((directive.line, directive.end_line), (1, 2));
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 3,
                end_line: Some(3)
            }
        );
    }

    #[test]
    fn ignore_all_exempts_the_whole_file() {
        let directive = only("// biome-ignore-all lint/suspicious/noDebugger: legacy\ndebugger;\n");
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
    }

    #[test]
    fn start_and_end_pair_into_a_block() {
        let directive = only(concat!(
            "// biome-ignore-start lint/suspicious/noDebugger: a debugging region\n",
            "debugger;\n",
            "// biome-ignore-end lint/suspicious/noDebugger: region over\n",
            "debugger;\n",
        ));
        assert_eq!(directive.scope, Scope::Block);
        assert_eq!(directive.reason.as_deref(), Some("a debugging region"));
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: Some(3)
            }
        );
    }

    #[test]
    fn an_unterminated_start_runs_to_end_of_file() {
        let directive = only("// biome-ignore-start lint/a/x: open\ndebugger;\n");
        assert_eq!(directive.scope, Scope::Block);
        assert_eq!(directive.suppressed.end_line, None);
    }

    #[test]
    fn an_end_closes_only_a_range_naming_the_same_selector() {
        // Biome does not accept a domain-level end for a rule-level start.
        let directive = only(concat!(
            "// biome-ignore-start lint/suspicious/noDebugger: open\n",
            "// biome-ignore-end lint: wrong selector\n",
            "debugger;\n",
        ));
        assert_eq!(directive.suppressed.end_line, None);
    }

    #[test]
    fn nested_ranges_close_innermost_first() {
        let found = parse(concat!(
            "// biome-ignore-start lint/a/x: outer\n",
            "// biome-ignore-start lint/a/x: inner\n",
            "// biome-ignore-end lint/a/x: closes inner\n",
            "// biome-ignore-end lint/a/x: closes outer\n",
        ));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].suppressed.end_line, Some(4));
        assert_eq!(found[1].suppressed.end_line, Some(3));
    }

    #[test]
    fn an_end_without_a_start_is_ignored() {
        assert!(parse("// biome-ignore-end lint/a/x: stray\nx;\n").is_empty());
    }

    #[test]
    fn a_directive_that_does_not_open_the_comment_is_not_one() {
        assert!(parse("// TODO biome-ignore lint/a/x: nope\nx;\n").is_empty());
        assert!(parse("// biome-ignoreable lint/a/x: nope\nx;\n").is_empty());
        // Biome's directives are case-sensitive.
        assert!(parse("// BIOME-IGNORE lint/a/x: nope\nx;\n").is_empty());
    }

    #[test]
    fn a_directive_inside_a_string_literal_is_not_reported() {
        assert!(parse("const m = \"// biome-ignore lint/a/x: nope\";\n").is_empty());
    }

    #[test]
    fn the_keyword_need_not_be_followed_by_a_space() {
        assert_eq!(
            only("//biome-ignore lint/a/x: tight\nx;\n").rules,
            vec!["lint/a/x"]
        );
    }

    #[test]
    fn a_directive_naming_no_selector_is_not_a_suppression() {
        // Biome silences nothing for `biome-ignore: reason`, so there is no
        // blanket form to report.
        assert!(parse("// biome-ignore: everything here\ndebugger;\n").is_empty());
    }
}
