//! TypeScript (`// @ts-ignore`) suppression parsing.
//!
//! Forms understood, mirroring the compiler's own directive grammar:
//!
//! | Source | Scope | Rules |
//! | --- | --- | --- |
//! | `// @ts-ignore` | next-line | *(blanket)* |
//! | `// @ts-expect-error the API is untyped` | next-line | *(blanket)* |
//! | `/* @ts-ignore */`, `/** @ts-expect-error */` | next-line | *(blanket)* |
//! | `// @ts-nocheck` | file | *(blanket)* |
//!
//! All of them are blanket: TypeScript has no way to name an error code, so
//! `rules` is always empty and any trailing text is the reason. There is no
//! separator to strip — `@ts-expect-error: legacy` and `@ts-expect-error -- legacy`
//! are both prose to `tsc`, and both are captured as written.
//!
//! Three details are copied from the compiler rather than guessed:
//!
//! * **`@ts-ignore` and `@ts-expect-error` are matched on the comment's *last*
//!   line.** `tsc` scans a block comment from its last line break, so
//!   `/* @ts-ignore\n*/` silences nothing while `/* prose\n @ts-ignore */` does.
//!   The suppressed line is therefore always the one after the comment ends.
//! * **`@ts-nocheck` is a line comment only**, needs a word boundary, and is
//!   matched case-insensitively — `/* @ts-nocheck */` and `// @ts-nocheckish`
//!   are not directives, but `// @ts-NOCHECK` is.
//! * **`@ts-ignore` has no trailing word boundary.** `// @ts-ignoreable` really
//!   does silence the next line, so it is reported (with `able` as its reason)
//!   rather than dropped.

use crate::comments::{Comment, CommentKind};
use crate::model::{normalize_reason, IgnoreDirective, Scope, Suppressed, Tool};
use crate::source::{Language, SourceFile};
use crate::tools::ToolParser;

/// Parses TypeScript's `@ts-` directives out of JavaScript and TypeScript.
///
/// JavaScript is included because `tsc` honours the same directives there under
/// `checkJs`, and a suppression a reviewer should see does not become invisible
/// because the file is `.js`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TypescriptParser;

/// The next-line keywords, longest first: `@ts-expect-error` must not be read as
/// an `@ts-` prefix of anything shorter.
const NEXT_LINE: [&str; 2] = ["@ts-expect-error", "@ts-ignore"];

/// The file-level keyword.
const NOCHECK: &str = "@ts-nocheck";

impl ToolParser for TypescriptParser {
    fn tool(&self) -> Tool {
        Tool::Typescript
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        matches!(file.language(), Language::JavaScript | Language::TypeScript)
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        file.comments()
            .iter()
            .filter_map(|comment| {
                let (scope, reason) = directive(comment)?;
                Some(IgnoreDirective {
                    tool: Tool::Typescript,
                    scope,
                    rules: Vec::new(),
                    reason,
                    path: file.display_path().to_string(),
                    line: comment.line,
                    end_line: comment.end_line,
                    column: comment.column,
                    raw: comment.raw.clone(),
                    suppressed: suppressed_range(scope, comment),
                    change: None,
                })
            })
            .collect()
    }
}

fn suppressed_range(scope: Scope, comment: &Comment) -> Suppressed {
    match scope {
        Scope::File => Suppressed {
            start_line: 1,
            end_line: None,
        },
        _ => {
            let next = comment.end_line.saturating_add(1);
            Suppressed {
                start_line: next,
                end_line: Some(next),
            }
        }
    }
}

/// Recognize a `@ts-` directive in `comment`, returning its scope and reason.
fn directive(comment: &Comment) -> Option<(Scope, Option<String>)> {
    if let Some(rest) = nocheck(comment) {
        return Some((Scope::File, normalize_reason(rest)));
    }
    let body = decorated(last_line(comment));
    let rest = NEXT_LINE
        .iter()
        .find_map(|keyword| body.strip_prefix(keyword))?;
    Some((Scope::NextLine, normalize_reason(trim_decoration(rest))))
}

/// True when the text after a `//` opens a `@ts-` directive.
///
/// A line comment is what the caller has, which is why `@ts-nocheck` counts
/// here; see [`crate::tools::opens_directive`].
pub(super) fn starts_with_directive(after_marker: &str) -> bool {
    nocheck_body(after_marker).is_some()
        || NEXT_LINE
            .iter()
            .any(|keyword| decorated(after_marker).starts_with(keyword))
}

/// The `@ts-nocheck` tail, when this comment is one.
///
/// The compiler accepts it after `//` or `///` only — a fourth slash, or a block
/// comment, is not a directive.
fn nocheck(comment: &Comment) -> Option<&str> {
    if comment.kind != CommentKind::Line {
        return None;
    }
    nocheck_body(&comment.text)
}

/// The `@ts-nocheck` tail of a line comment whose body is `text`.
fn nocheck_body(text: &str) -> Option<&str> {
    // `text` is what follows the `//` marker, so at most one more slash may lead.
    let body = text.strip_prefix('/').unwrap_or(text).trim_start();
    if !body.get(..NOCHECK.len())?.eq_ignore_ascii_case(NOCHECK) {
        return None;
    }
    let rest = &body[NOCHECK.len()..];
    // A word boundary: `@ts-nocheckish` silences nothing.
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some(rest)
}

/// The text `tsc` matches a directive against: everything from the comment's
/// last line break to its end, with the closing delimiter removed.
fn last_line(comment: &Comment) -> &str {
    let tail = match comment.raw.rsplit_once('\n') {
        Some((_, last)) => last,
        None => &comment.raw,
    };
    tail.strip_suffix("*/").unwrap_or(tail)
}

/// Strip the leading `/`, `*` and whitespace decoration `tsc` skips before a
/// directive keyword.
fn decorated(text: &str) -> &str {
    text.trim_start()
        .trim_start_matches(['/', '*'])
        .trim_start()
}

/// Drop the trailing `*` decoration a `/*** … ***/` comment leaves behind, so it
/// never lands in a reason.
fn trim_decoration(text: &str) -> &str {
    text.trim_end().trim_end_matches('*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        let file = SourceFile::new("src/app.ts", source.to_string());
        TypescriptParser.parse(&file)
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
        assert_eq!(TypescriptParser.tool(), Tool::Typescript);
        assert!(TypescriptParser.applies_to(&SourceFile::new("a.ts", String::new())));
        assert!(TypescriptParser.applies_to(&SourceFile::new("a.js", String::new())));
        assert!(!TypescriptParser.applies_to(&SourceFile::new("a.py", String::new())));
    }

    #[test]
    fn expect_error_carries_its_reason_and_span() {
        let directive = only("// @ts-expect-error the API is untyped\nconst x: number = f();\n");
        assert_eq!(directive.tool, Tool::Typescript);
        assert_eq!(directive.scope, Scope::NextLine);
        assert!(directive.rules.is_empty());
        assert_eq!(directive.reason.as_deref(), Some("the API is untyped"));
        assert_eq!(directive.path, "src/app.ts");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (1, 1, 1)
        );
        assert_eq!(directive.raw, "// @ts-expect-error the API is untyped");
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 2,
                end_line: Some(2)
            }
        );
    }

    #[test]
    fn a_bare_ignore_has_no_reason() {
        let directive = only("// @ts-ignore\nconst x: number = f();\n");
        assert_eq!(directive.scope, Scope::NextLine);
        assert_eq!(directive.reason, None);
    }

    #[test]
    fn punctuation_before_the_reason_is_kept_verbatim() {
        // `tsc` parses neither separator, so both are the author's prose.
        assert_eq!(
            only("// @ts-ignore: legacy shim\nx;\n").reason.as_deref(),
            Some(": legacy shim")
        );
        assert_eq!(
            only("// @ts-expect-error -- legacy shim\nx;\n")
                .reason
                .as_deref(),
            Some("-- legacy shim")
        );
    }

    #[test]
    fn nocheck_exempts_the_whole_file() {
        let directive = only("// @ts-nocheck vendored bundle\nconst x: number = f();\n");
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(directive.reason.as_deref(), Some("vendored bundle"));
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
    }

    #[test]
    fn nocheck_is_case_insensitive_but_needs_a_word_boundary() {
        assert_eq!(only("// @ts-NOCHECK\nx;\n").scope, Scope::File);
        assert_eq!(only("/// @ts-nocheck\nx;\n").scope, Scope::File);
        assert!(parse("// @ts-nocheckish\nx;\n").is_empty());
        // A fourth slash, or a block comment, is not a `@ts-nocheck`.
        assert!(parse("//// @ts-nocheck\nx;\n").is_empty());
        assert!(parse("/* @ts-nocheck */\nx;\n").is_empty());
    }

    #[test]
    fn ignore_is_case_sensitive_and_needs_no_trailing_boundary() {
        assert!(parse("// @ts-Ignore\nx;\n").is_empty());
        // The compiler really does honour this one, so reporting it is not a
        // false positive — it is the suppression a reviewer would otherwise miss.
        let sloppy = only("// @ts-ignoreable\nx;\n");
        assert_eq!(sloppy.scope, Scope::NextLine);
        assert_eq!(sloppy.reason.as_deref(), Some("able"));
    }

    #[test]
    fn slash_and_star_decoration_is_skipped() {
        for source in [
            "/* @ts-ignore */\nx;\n",
            "/** @ts-expect-error */\nx;\n",
            "/*@ts-ignore*/\nx;\n",
            "//@ts-ignore\nx;\n",
            "//// @ts-ignore\nx;\n",
            "/*** @ts-ignore ***/\nx;\n",
        ] {
            let directive = only(source);
            assert_eq!(directive.scope, Scope::NextLine, "{source:?}");
            assert_eq!(directive.reason, None, "{source:?}");
            assert_eq!(directive.suppressed.start_line, 2, "{source:?}");
        }
    }

    #[test]
    fn a_block_directive_is_matched_on_the_comments_last_line() {
        // `tsc` scans from the last line break, so this one counts…
        let directive = only("/* prose\n   @ts-ignore */\nconst x: number = f();\n");
        assert_eq!((directive.line, directive.end_line), (1, 2));
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 3,
                end_line: Some(3)
            }
        );

        // …and these do not, because their last line holds no keyword.
        assert!(parse("/* @ts-ignore\n*/ const x: number = f();\n").is_empty());
        assert!(parse("/*\n * @ts-ignore\n */\nx;\n").is_empty());
    }

    #[test]
    fn a_directive_that_does_not_open_the_comment_is_not_one() {
        assert!(parse("// TODO @ts-ignore\nx;\n").is_empty());
        assert!(parse("/* prose @ts-ignore */\nx;\n").is_empty());
    }

    #[test]
    fn a_directive_inside_a_string_literal_is_not_reported() {
        assert!(parse("const m = \"// @ts-ignore\";\n").is_empty());
    }

    #[test]
    fn every_directive_is_reported_in_source_order() {
        let found = parse("// @ts-nocheck\n// @ts-ignore\n/** @ts-expect-error */\nx;\n");
        assert_eq!(
            found.iter().map(|d| (d.line, d.scope)).collect::<Vec<_>>(),
            vec![(1, Scope::File), (2, Scope::NextLine), (3, Scope::NextLine)]
        );
    }
}
