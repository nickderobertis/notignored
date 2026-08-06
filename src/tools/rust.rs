//! Rust (`#[allow(…)]`, `#[expect(…)]`) suppression parsing.
//!
//! Rust silences a lint with an **attribute**, not a comment, and `expect`
//! carries the strongest native reason syntax of any tool here — a first-class
//! `reason = "…"` the compiler itself accepts.
//!
//! | Source | Scope | Rules | Reason |
//! | --- | --- | --- | --- |
//! | `#[allow(dead_code)]` | next-line | `dead_code` | *(none)* |
//! | `#[allow(clippy::needless_collect, dead_code)]` | next-line | both, as written | *(none)* |
//! | `#[expect(dead_code, reason = "…")]` | next-line | `dead_code` | the string |
//! | `#![allow(dead_code)]` | file | `dead_code` | *(none)* |
//! | `#![expect(dead_code, reason = "…")]` | file | `dead_code` | the string |
//!
//! Lint paths are captured exactly as written (`clippy::needless_collect`,
//! `dead_code`), never normalized to a canonical spelling. The reason string may
//! span lines; its escapes are resolved and its whitespace collapsed, so a
//! wrapped justification reads as one sentence.
//!
//! What the ranges mean, and where they are approximate:
//!
//! * An **outer** attribute is `next-line` scope, and `suppressed` runs from the
//!   attribute's own first line through the end of the item it annotates. That
//!   end is found by walking the item punctuation
//!   [the extractor recorded](crate::comments::CodePunctuation) — the first `;`
//!   at depth zero, or the `}` that closes the item's block. It is deliberately
//!   lightweight rather than a real item parse: an attribute on a struct field
//!   or an enum variant resolves to the end of the enclosing item, and one whose
//!   item never terminates leaves `suppressed.end_line` null.
//! * An **inner** attribute is `file` scope running to end-of-file. An inner
//!   attribute at the top of a `mod` block reaches only that module; reporting
//!   it as file-wide over-states the range but never invents a directive.
//!
//! Only `allow` and `expect` silence a lint: `warn`, `deny`, and `forbid` raise
//! one, so they are not suppressions and are not reported. Neither is an
//! `#[allow]` inside `#[cfg_attr(test, allow(…))]`: it applies only under that
//! configuration, and claiming an unconditional suppression would over-report.
//! An argument that could not be a lint path — a scanned file need not compile —
//! is dropped rather than reported as a rule someone silenced.

use crate::comments::Attribute;
use crate::model::{normalize_reason, IgnoreDirective, Scope, Suppressed, Tool};
use crate::source::{Language, SourceFile};
use crate::tools::ToolParser;

/// Parses `#[allow(…)]` and `#[expect(…)]` out of Rust sources.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustParser;

impl ToolParser for RustParser {
    fn tool(&self) -> Tool {
        Tool::Rust
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        file.language() == Language::Rust
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        let mut out = Vec::new();
        for attribute in file.attributes() {
            let Some((rules, reason)) = lint_attribute(&attribute.text) else {
                continue;
            };
            let scope = if attribute.inner {
                Scope::File
            } else {
                Scope::NextLine
            };
            out.push(IgnoreDirective {
                tool: Tool::Rust,
                scope,
                rules,
                reason,
                path: file.display_path().to_string(),
                line: attribute.line,
                end_line: attribute.end_line,
                column: attribute.column,
                raw: attribute.raw.clone(),
                suppressed: suppressed_range(file, attribute),
            });
        }
        out
    }
}

fn suppressed_range(file: &SourceFile, attribute: &Attribute) -> Suppressed {
    if attribute.inner {
        return Suppressed {
            start_line: 1,
            end_line: None,
        };
    }
    Suppressed {
        start_line: attribute.line,
        end_line: item_end_line(file, attribute),
    }
}

/// The line the item annotated by `attribute` ends on, as far as the recorded
/// punctuation can tell.
///
/// Depth counts every opener, so a `;` inside `[u32; 4]` or a `,` inside
/// `Result<A, B>` cannot end the item early. Only a `}` terminates on close —
/// a `)` returning to depth zero is a signature's end, not the item's.
fn item_end_line(file: &SourceFile, attribute: &Attribute) -> Option<u32> {
    let marks = file.punctuation();
    let start = marks.partition_point(|mark| {
        (mark.line, mark.column) < (attribute.end_line, attribute.end_column)
    });
    let mut depth = 0usize;
    for mark in &marks[start..] {
        match mark.character {
            '(' | '[' | '{' => depth += 1,
            '}' if depth <= 1 => return Some(mark.line),
            // A `)` or `]` at depth zero closes something that opened before the
            // attribute — a parameter list holding it, say. The item's own end
            // is not knowable from here.
            ')' | ']' if depth == 0 => return None,
            ')' | ']' | '}' => depth -= 1,
            ';' if depth == 0 => return Some(mark.line),
            _ => {}
        }
    }
    None
}

/// The lint paths and reason of an `allow`/`expect` attribute, or `None` when
/// the attribute is neither (or silences nothing).
fn lint_attribute(text: &str) -> Option<(Vec<String>, Option<String>)> {
    let trimmed = text.trim_start();
    let rest = strip_keyword(trimmed, "allow").or_else(|| strip_keyword(trimmed, "expect"))?;
    let arguments = parenthesized(rest.trim_start())?;

    let mut rules = Vec::new();
    let mut reason = None;
    for argument in split_arguments(arguments) {
        let argument = argument.trim();
        if let Some(value) = strip_keyword(argument, "reason") {
            if let Some(literal) = value.trim_start().strip_prefix('=') {
                reason = string_literal(literal.trim_start()).and_then(|text| {
                    let unescaped = unescape(&text);
                    normalize_reason(&unescaped)
                });
                continue;
            }
        }
        // A scanned file need not compile, so the argument list is not a vetted
        // lint list. Anything that could not be a lint path is dropped rather
        // than reported as a rule someone silenced.
        if is_lint_path(argument) {
            rules.push(argument.to_string());
        }
    }
    // `#[allow()]` and `#[expect(reason = "…")]` silence nothing, so they are
    // not suppressions to report.
    (!rules.is_empty()).then_some((rules, reason))
}

/// Whether an argument has the shape of a lint path — `dead_code`,
/// `clippy::needless_return`, `rustdoc::broken_intra_doc_links`.
fn is_lint_path(argument: &str) -> bool {
    !argument.is_empty()
        && argument.split("::").all(|segment| {
            let segment = segment.strip_prefix("r#").unwrap_or(segment);
            !segment.is_empty()
                && !segment.starts_with(|c: char| c.is_ascii_digit())
                && segment.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
}

/// Strip `keyword` when it stands alone — so `allowance` never reads as
/// `allow`, and `cfg_attr(test, allow(…))` is not an `allow` attribute.
fn strip_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = input.strip_prefix(keyword)?;
    match rest.chars().next() {
        Some(ch) if ch.is_alphanumeric() || ch == '_' || ch == ':' => None,
        _ => Some(rest),
    }
}

/// The contents of a parenthesized run starting at `input`, with nested parens
/// balanced and string literals skipped.
fn parenthesized(input: &str) -> Option<&str> {
    let mut rest = input.strip_prefix('(')?;
    let body_start = input.len() - rest.len();
    let mut depth = 1usize;
    while !rest.is_empty() {
        if let Some(after) = skip_string_literal(rest) {
            rest = after;
            continue;
        }
        let ch = rest.chars().next()?;
        rest = &rest[ch.len_utf8()..];
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let end = input.len() - rest.len() - 1;
                    return Some(&input[body_start..end]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Advance past the string literal at `input`, or `None` when one does not start
/// there.
///
/// Char literals need no handling: in Rust a `'` opens a lifetime as often as a
/// literal, and neither can hold a delimiter that changes this scan's answer.
fn skip_string_literal(input: &str) -> Option<&str> {
    if let Some(rest) = input.strip_prefix('r') {
        let hashes = rest.len() - rest.trim_start_matches('#').len();
        let body = rest[hashes..].strip_prefix('"')?;
        let terminator = format!("\"{}", "#".repeat(hashes));
        let end = body.find(&terminator)?;
        return Some(&body[end + terminator.len()..]);
    }
    let mut rest = input.strip_prefix('"')?;
    while let Some(ch) = rest.chars().next() {
        rest = &rest[ch.len_utf8()..];
        match ch {
            '"' => return Some(rest),
            '\\' => {
                if let Some(escaped) = rest.chars().next() {
                    rest = &rest[escaped.len_utf8()..];
                }
            }
            _ => {}
        }
    }
    None
}

/// Split an argument list on the commas that are not inside a nested group or a
/// string literal.
fn split_arguments(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = input;
    let mut depth = 0usize;
    let mut start = 0usize;
    while let Some(ch) = rest.chars().next() {
        if let Some(after) = skip_string_literal(rest) {
            rest = after;
            continue;
        }
        let offset = input.len() - rest.len();
        rest = &rest[ch.len_utf8()..];
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&input[start..offset]);
                start = offset + 1;
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

/// The contents of the string literal at `input`, quotes and `r#` hashes
/// stripped, or `None` when `input` does not open one.
fn string_literal(input: &str) -> Option<String> {
    if let Some(rest) = input.strip_prefix('r') {
        let hashes = rest.len() - rest.trim_start_matches('#').len();
        let body = rest[hashes..].strip_prefix('"')?;
        let terminator = format!("\"{}", "#".repeat(hashes));
        let end = body.find(&terminator)?;
        return Some(body[..end].to_string());
    }
    let body = input.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => {
                out.push(ch);
                out.extend(chars.next());
            }
            _ => out.push(ch),
        }
    }
    None
}

/// Resolve the escapes a reason string can carry, so `\"` reads as a quote
/// rather than as two characters. Unknown escapes keep their literal spelling.
fn unescape(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            // A backslash before a newline continues the string; the newline and
            // the indentation after it are not part of the reason.
            Some('\n') => {
                while chars.as_str().starts_with(|c: char| c.is_whitespace()) {
                    chars.next();
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        let file = SourceFile::new("src/lib.rs", source.to_string());
        RustParser.parse(&file)
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
    fn the_parser_claims_rust_files_only() {
        assert_eq!(RustParser.tool(), Tool::Rust);
        assert!(RustParser.applies_to(&SourceFile::new("a.rs", String::new())));
        assert!(!RustParser.applies_to(&SourceFile::new("a.py", String::new())));
    }

    #[test]
    fn an_outer_allow_covers_the_item_it_annotates() {
        let directive = only("#[allow(dead_code)]\nfn helper() {\n    todo!()\n}\n");
        assert_eq!(directive.tool, Tool::Rust);
        assert_eq!(directive.scope, Scope::NextLine);
        assert_eq!(directive.rules, vec!["dead_code"]);
        assert_eq!(directive.reason, None);
        assert_eq!(directive.path, "src/lib.rs");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (1, 1, 1)
        );
        assert_eq!(directive.raw, "#[allow(dead_code)]");
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: Some(4)
            }
        );
    }

    #[test]
    fn lint_paths_are_captured_exactly_as_written() {
        let directive = only("#[allow(clippy::needless_collect, dead_code)]\nfn f() {}\n");
        assert_eq!(
            directive.rules,
            vec!["clippy::needless_collect", "dead_code"]
        );
        assert_eq!(
            only("#[expect(rustdoc::broken_intra_doc_links)]\nfn f() {}\n").rules,
            vec!["rustdoc::broken_intra_doc_links"]
        );
    }

    #[test]
    fn an_argument_that_cannot_be_a_lint_name_is_not_reported_as_one() {
        // The scanned file need not compile, so the argument list is not a
        // vetted lint list: only what could be a lint path is reported as one.
        let directive = only("#[allow(dead_code, 42, \"quoted\", a-b, )]\nfn f() {}\n");
        assert_eq!(directive.rules, vec!["dead_code"]);
        assert!(is_lint_path("r#type") && is_lint_path("clippy::all"));
        assert!(!is_lint_path("") && !is_lint_path("a::") && !is_lint_path("9lives"));
        // An attribute whose every argument is unusable names no lint at all.
        assert!(parse("#[allow(42)]\nfn f() {}\n").is_empty());
    }

    #[test]
    fn an_expect_reason_is_the_native_reason() {
        let directive =
            only("#[expect(dead_code, reason = \"kept for the 1.0 API\")]\npub fn shim() {}\n");
        assert_eq!(directive.rules, vec!["dead_code"]);
        assert_eq!(directive.reason.as_deref(), Some("kept for the 1.0 API"));
    }

    #[test]
    fn a_reason_string_may_span_lines() {
        let directive = only(concat!(
            "#[expect(\n",
            "    dead_code,\n",
            "    reason = \"a justification long enough\n",
            "              to wrap across lines\"\n",
            ")]\n",
            "fn f() {}\n",
        ));
        assert_eq!(
            directive.reason.as_deref(),
            Some("a justification long enough to wrap across lines")
        );
        assert_eq!((directive.line, directive.end_line), (1, 5));
        assert_eq!(directive.suppressed.end_line, Some(6));
    }

    #[test]
    fn reason_escapes_are_resolved() {
        assert_eq!(
            only("#[expect(dead_code, reason = \"the \\\"public\\\" shim\")]\nfn f() {}\n")
                .reason
                .as_deref(),
            Some("the \"public\" shim")
        );
        assert_eq!(
            only("#[expect(dead_code, reason = \"first\\nsecond\")]\nfn f() {}\n")
                .reason
                .as_deref(),
            Some("first second")
        );
        assert_eq!(
            only("#[expect(dead_code, reason = \"joined \\\n            across\")]\nfn f() {}\n")
                .reason
                .as_deref(),
            Some("joined across")
        );
        assert_eq!(
            only("#[expect(dead_code, reason = r#\"a raw \"quoted\" reason\"#)]\nfn f() {}\n")
                .reason
                .as_deref(),
            Some("a raw \"quoted\" reason")
        );
    }

    #[test]
    fn a_comma_inside_a_reason_does_not_split_the_argument_list() {
        let directive = only("#[expect(dead_code, reason = \"one, two, three\")]\nfn f() {}\n");
        assert_eq!(directive.rules, vec!["dead_code"]);
        assert_eq!(directive.reason.as_deref(), Some("one, two, three"));
    }

    #[test]
    fn the_remaining_string_escapes_resolve_or_keep_their_spelling() {
        // `\t` and `\r` collapse with the surrounding whitespace, `\0` and an
        // unknown escape survive as the character itself, and a lone trailing
        // backslash stays a backslash.
        assert_eq!(unescape("a\\tb\\rc"), "a\tb\rc");
        assert_eq!(unescape("a\\0b"), "a\0b");
        assert_eq!(unescape("a\\qb"), "aqb");
        assert_eq!(unescape("trailing\\"), "trailing\\");
    }

    #[test]
    fn nested_groups_inside_the_argument_list_do_not_end_it_early() {
        let directive = only("#[allow(clippy::type_complexity, dead_code)]\nfn f() {}\n");
        assert_eq!(
            directive.rules,
            vec!["clippy::type_complexity", "dead_code"]
        );
        assert_eq!(
            split_arguments("a, b(c, d), e"),
            vec!["a", " b(c, d)", " e"]
        );
        assert_eq!(parenthesized("(a(b), c) trailing"), Some("a(b), c"));
        // An argument list that never closes is not an attribute we can read.
        assert_eq!(parenthesized("(a, b"), None);
        assert!(parse("#[allow(dead_code\nfn f() {}\n").is_empty());
    }

    #[test]
    fn an_inner_attribute_is_a_file_scope_suppression() {
        let directive = only("#![allow(dead_code)]\n\nfn f() {}\n");
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(directive.rules, vec!["dead_code"]);
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
        assert_eq!(directive.raw, "#![allow(dead_code)]");
    }

    #[test]
    fn allow_and_expect_tokens_in_strings_comments_and_macros_are_not_directives() {
        assert!(parse("const MSG: &str = \"#[allow(dead_code)]\";\n").is_empty());
        assert!(parse("let raw = r#\"#[expect(dead_code)]\"#;\n").is_empty());
        assert!(parse("// #[allow(dead_code)] in a line comment\n").is_empty());
        assert!(parse("/* #[allow(dead_code)] in a block comment */\n").is_empty());
        assert!(parse("println!(\"#[allow(dead_code)] {}\", x);\n").is_empty());
        assert!(parse("macro_rules! m { () => { \"#[allow(dead_code)]\" }; }\n").is_empty());
    }

    #[test]
    fn attributes_that_are_not_suppressions_are_skipped() {
        for source in [
            "#[derive(Debug)]\nstruct S;\n",
            "#[allowance(dead_code)]\nfn f() {}\n",
            "#[doc = \"allow(dead_code)\"]\nfn f() {}\n",
            // Conditional suppressions overstate the case; we skip them.
            "#[cfg_attr(test, allow(dead_code))]\nfn f() {}\n",
            "#[allow()]\nfn f() {}\n",
            "#[expect(reason = \"nothing named\")]\nfn f() {}\n",
            "#[allow]\nfn f() {}\n",
            "#[allow\nfn f() {}\n",
            // Raising a lint is the opposite of silencing it.
            "#[deny(dead_code)]\nfn f() {}\n",
            "#[warn(dead_code)]\nfn f() {}\n",
            "#[forbid(dead_code)]\nfn f() {}\n",
        ] {
            assert!(parse(source).is_empty(), "{source:?} is not a suppression");
        }
    }

    #[test]
    fn item_ends_are_found_through_signatures_and_nested_braces() {
        let cases = [
            ("#[allow(dead_code)]\nuse std::fmt;\n", Some(2)),
            ("#[allow(dead_code)]\nmod inner;\n", Some(2)),
            ("#[allow(dead_code)]\nstruct S {\n    a: u32,\n}\n", Some(4)),
            (
                "#[allow(dead_code)]\nfn f(a: [u32; 4]) -> Result<u32, u32> {\n    Ok(a[0])\n}\n",
                Some(4),
            ),
            (
                "#[allow(dead_code)]\nfn f() -> S {\n    S { a: 1 }\n}\n",
                Some(4),
            ),
            ("#[allow(dead_code)]\nfn f() {}\n", Some(2)),
        ];
        for (source, expected) in cases {
            assert_eq!(only(source).suppressed.end_line, expected, "{source:?}");
        }
    }

    #[test]
    fn an_item_that_never_terminates_leaves_the_range_open() {
        let directive = only("#[allow(dead_code)]\nfn f()\n");
        assert_eq!(directive.suppressed.end_line, None);

        // An attribute on a function parameter closes something that opened
        // before it, so the item's own end is not knowable.
        let directive = only("fn f(#[allow(dead_code)] a: u32) {}\n");
        assert_eq!(directive.suppressed.end_line, None);
    }

    #[test]
    fn a_field_attribute_resolves_to_the_enclosing_item() {
        let directive = only("struct S {\n    #[allow(dead_code)]\n    a: u32,\n}\n");
        assert_eq!(directive.suppressed.start_line, 2);
        assert_eq!(directive.suppressed.end_line, Some(4));
    }

    #[test]
    fn every_attribute_in_a_file_is_reported_in_source_order() {
        let found = parse(concat!(
            "#![allow(dead_code)]\n",
            "#[allow(clippy::needless_return)]\n",
            "fn a() -> u32 {\n    return 1;\n}\n",
            "#[expect(unused_variables, reason = \"wired up next\")]\n",
            "fn b(x: u32) {}\n",
        ));
        assert_eq!(found.len(), 3);
        assert_eq!(
            found.iter().map(|d| d.line).collect::<Vec<_>>(),
            vec![1, 2, 6]
        );
        assert_eq!(found[0].scope, Scope::File);
        assert_eq!(found[1].suppressed.end_line, Some(5));
        assert_eq!(found[2].suppressed.end_line, Some(7));
    }

    #[test]
    fn an_unterminated_reason_literal_yields_no_reason() {
        let directive = only("#[expect(dead_code, reason = \"never closed)]\nfn f() {}\n");
        assert_eq!(directive.rules, vec!["dead_code"]);
        assert_eq!(directive.reason, None);
    }
}
