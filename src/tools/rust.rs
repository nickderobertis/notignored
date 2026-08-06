//! Rust and clippy (`#[allow(…)]`, `#[expect(…)]`) suppression parsing.
//!
//! Forms understood:
//!
//! | Source | Scope | Rules |
//! | --- | --- | --- |
//! | `#[allow(dead_code)]` | block | `dead_code` |
//! | `#[expect(dead_code, reason = "…")]` | block | `dead_code` |
//! | `#[allow(clippy::needless_range_loop)]` | block | `clippy::needless_range_loop` |
//! | `#![allow(dead_code)]` | file | `dead_code` |
//!
//! Only `allow` and `expect` silence a lint: `warn`, `deny`, and `forbid` raise
//! one, so they are not suppressions and are not reported. `reason = "…"`
//! (stable since 1.81) is the tool's native justification syntax and becomes the
//! record's [`reason`](crate::model::IgnoreDirective::reason).
//!
//! Two deliberate omissions, both in this crate's safe direction — miss an exotic
//! directive rather than invent one:
//!
//! * `#[cfg_attr(test, allow(dead_code))]` is conditional, and reporting it as an
//!   unconditional suppression would overstate it.
//! * The item an outer attribute annotates is not parsed, so the suppressed range
//!   starts at the attribute and has no known end. A file-level inner attribute
//!   runs from line 1 to end-of-file, which *is* known.

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
        file.attributes()
            .iter()
            .filter_map(|attribute| directive(file, attribute))
            .collect()
    }
}

/// The directive an attribute carries, if it silences anything.
fn directive(file: &SourceFile, attribute: &Attribute) -> Option<IgnoreDirective> {
    let arguments = suppressing_arguments(&attribute.text)?;
    let (rules, reason) = rules_and_reason(arguments);
    // An inner attribute applies to the whole enclosing item, which at the top
    // of a file is the crate: a file-wide exemption.
    let scope = if attribute.inner {
        Scope::File
    } else {
        Scope::Block
    };
    Some(IgnoreDirective {
        tool: Tool::Rust,
        scope,
        rules,
        reason,
        path: file.display_path().to_string(),
        line: attribute.line,
        end_line: attribute.end_line,
        column: attribute.column,
        raw: attribute.raw.clone(),
        suppressed: match scope {
            Scope::File => Suppressed {
                start_line: 1,
                end_line: None,
            },
            // The annotated item's extent is unknown without parsing Rust, so
            // the range is honestly open-ended rather than guessed at.
            _ => Suppressed {
                start_line: attribute.line,
                end_line: None,
            },
        },
    })
}

/// The argument list of an `allow(…)` / `expect(…)` attribute, or `None` for any
/// other attribute.
fn suppressing_arguments(text: &str) -> Option<&str> {
    let text = text.trim();
    let rest = ["allow", "expect"]
        .into_iter()
        .find_map(|keyword| text.strip_prefix(keyword))?;
    let rest = rest.trim_start();
    rest.strip_prefix('(')?.strip_suffix(')')
}

/// Split an argument list into lint paths and the stated reason.
fn rules_and_reason(arguments: &str) -> (Vec<String>, Option<String>) {
    let mut rules = Vec::new();
    let mut reason = None;
    for argument in split_arguments(arguments) {
        let argument = argument.trim();
        if argument.is_empty() {
            continue;
        }
        match argument.strip_prefix("reason") {
            Some(rest) if rest.trim_start().starts_with('=') => {
                reason = string_literal(rest.trim_start().trim_start_matches('='))
                    .as_deref()
                    .and_then(normalize_reason);
            }
            // A lint path is captured exactly as written: `clippy::…` and
            // `rustdoc::…` prefixes are part of the name.
            _ => rules.push(argument.to_string()),
        }
    }
    (rules, reason)
}

/// Split on commas that are not inside a string literal.
fn split_arguments(arguments: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut in_string, mut escaped) = (0, false, false);
    for (offset, ch) in arguments.char_indices() {
        match ch {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            ',' if !in_string => {
                parts.push(&arguments[start..offset]);
                start = offset + 1;
            }
            _ => {}
        }
    }
    parts.push(&arguments[start..]);
    parts
}

/// The contents of a `"…"` literal, with `\"` and `\\` unescaped.
///
/// A raw string (`r"…"`) keeps its escapes, which is what a reader of the source
/// sees anyway.
fn string_literal(text: &str) -> Option<String> {
    let text = text.trim();
    let body = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            let raw = text.trim_start_matches('r');
            let hashes = raw.len() - raw.trim_start_matches('#').len();
            let opener = format!("{}\"", "#".repeat(hashes));
            let closer = format!("\"{}", "#".repeat(hashes));
            raw.strip_prefix(&opener)?.strip_suffix(&closer)
        })?;
    let mut out = String::with_capacity(body.len());
    let mut escaped = false;
    for ch in body.chars() {
        match ch {
            '\\' if !escaped => escaped = true,
            _ => {
                out.push(ch);
                escaped = false;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        RustParser.parse(&SourceFile::new("src/lib.rs", source.to_string()))
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
    fn an_outer_allow_is_a_block_suppression_of_the_lints_it_names() {
        let directive = only("#[allow(dead_code)]\nfn helper() {}\n");
        assert_eq!(directive.tool, Tool::Rust);
        assert_eq!(directive.scope, Scope::Block);
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
                end_line: None
            }
        );
    }

    #[test]
    fn an_inner_allow_exempts_the_whole_file() {
        let directive = only("#![allow(dead_code)]\nfn helper() {}\n");
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
    fn several_lints_and_tool_prefixed_paths_are_captured_verbatim() {
        assert_eq!(
            only("#[allow(dead_code, clippy::needless_range_loop)]\nfn f() {}\n").rules,
            vec!["dead_code", "clippy::needless_range_loop"]
        );
        assert_eq!(
            only("#[expect(rustdoc::broken_intra_doc_links)]\nfn f() {}\n").rules,
            vec!["rustdoc::broken_intra_doc_links"]
        );
    }

    #[test]
    fn a_reason_is_taken_from_the_native_syntax() {
        let directive = only("#[expect(dead_code, reason = \"used by the C API\")]\nfn f() {}\n");
        assert_eq!(directive.rules, vec!["dead_code"]);
        assert_eq!(directive.reason.as_deref(), Some("used by the C API"));

        // Escapes and raw strings both read back as the source shows them.
        assert_eq!(
            only("#[expect(dead_code, reason = \"the \\\"C\\\" API\")]\nfn f() {}\n")
                .reason
                .as_deref(),
            Some("the \"C\" API")
        );
        assert_eq!(
            only("#[expect(dead_code, reason = r#\"the \"C\" API\"#)]\nfn f() {}\n")
                .reason
                .as_deref(),
            Some("the \"C\" API")
        );
        // A comma inside the reason does not split the argument list.
        let directive = only("#[allow(dead_code, reason = \"one, two\")]\nfn f() {}\n");
        assert_eq!(directive.rules, vec!["dead_code"]);
        assert_eq!(directive.reason.as_deref(), Some("one, two"));
    }

    #[test]
    fn a_reason_written_across_lines_is_collapsed_and_spans_the_directive() {
        let directive = only(concat!(
            "#[expect(\n",
            "    dead_code,\n",
            "    reason = \"kept for the\n",
            "              next release\"\n",
            ")]\n",
            "fn helper() {}\n",
        ));
        assert_eq!(directive.rules, vec!["dead_code"]);
        assert_eq!(
            directive.reason.as_deref(),
            Some("kept for the next release")
        );
        // The record spans every line the directive occupies, which is what
        // `--diff` intersects against the change's added lines.
        assert_eq!((directive.line, directive.end_line), (1, 5));
    }

    #[test]
    fn lint_raising_attributes_are_not_suppressions() {
        for source in [
            "#[deny(dead_code)]\nfn f() {}\n",
            "#[warn(dead_code)]\nfn f() {}\n",
            "#[forbid(dead_code)]\nfn f() {}\n",
            "#[derive(Debug)]\nstruct S;\n",
            "#[allowance(dead_code)]\nfn f() {}\n",
            "#[allow]\nfn f() {}\n",
            // Conditional suppressions overstate the case; we skip them.
            "#[cfg_attr(test, allow(dead_code))]\nfn f() {}\n",
        ] {
            assert!(parse(source).is_empty(), "{source:?} is not a suppression");
        }
    }

    #[test]
    fn a_blanket_allow_with_no_lints_is_reported_as_blanket() {
        let directive = only("#[allow()]\nfn f() {}\n");
        assert!(directive.rules.is_empty());
        assert_eq!(directive.scope, Scope::Block);
    }

    #[test]
    fn an_attribute_inside_a_string_literal_is_not_a_directive() {
        assert!(parse("const S: &str = \"#[allow(dead_code)]\";\n").is_empty());
    }

    #[test]
    fn every_attribute_in_a_file_is_reported_in_source_order() {
        let found = parse(concat!(
            "#![allow(dead_code)]\n",
            "#[allow(unused_variables)]\n",
            "fn a() {}\n",
            "#[expect(clippy::all)]\n",
            "fn b() {}\n",
        ));
        assert_eq!(
            found.iter().map(|d| d.line).collect::<Vec<_>>(),
            vec![1, 2, 4]
        );
        assert_eq!(found[0].scope, Scope::File);
    }
}
