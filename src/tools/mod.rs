//! The tool-parser registry.
//!
//! Adding a tool touches exactly three places: a new module here, one line in
//! [`registry`], and one row in the README's supported-tools table. Keeping it
//! to those three keeps parallel branches from colliding.
//!
//! Parsers read the comments, attributes, and item punctuation a [`SourceFile`]
//! already extracted. They must not re-scan raw source lines — that is how
//! string literals get mistaken for directives.

use crate::model::{IgnoreDirective, Tool};
use crate::source::SourceFile;

mod python;

pub mod biome;
pub mod eslint;
pub mod llmlint;
pub mod mypy;
pub mod pyright;
pub mod ruff;
pub mod rust;
pub mod shellcheck;
pub mod ty;
pub mod typescript;

/// Turns one tool's suppression syntax into [`IgnoreDirective`] records.
///
/// The three methods below are the whole contract, and it is **fixed**: a
/// parser hands back directives and nothing else.
/// `tests/tools_contract.rs` locks the signatures.
///
/// A syntax that can be malformed in a way an [`IgnoreDirective`] cannot express
/// — llmlint's `ignore-block` with no closing directive — keeps that richer
/// result as an inherent method on its own parser and is integrated by
/// [`crate::scan`], rather than widening this trait for the one tool that needs
/// it. See [`llmlint::LlmlintParser::scan`].
pub trait ToolParser: Send + Sync {
    /// The tool this parser understands.
    fn tool(&self) -> Tool;

    /// Whether this parser has anything to say about the given file — normally
    /// a language check.
    fn applies_to(&self, file: &SourceFile) -> bool;

    /// Every directive in the file, in source order.
    ///
    /// Never fails: input this parser cannot make sense of yields no directive
    /// rather than an error, so one odd comment can't abort a scan.
    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective>;
}

/// True when `after_marker` — a comment's text with its opening marker removed
/// — opens a directive for another tool the crate parses.
///
/// This is the boundary two parsers need. `python::segments` walks it along one
/// line, so a `# noqa` never lands in the neighbouring directive's reason; the
/// llmlint parser walks it one line *down*, so a directive on the line below
/// never lands in a wrapped justification. Each tool contributes its own
/// recognizer, so neither has to know another's keywords.
///
/// Two tools are absent. Rust's suppressions are attributes rather than
/// comments, so no comment line can open one. llmlint's own grammar is tested by
/// its parser directly: folding it in here would make an llmlint directive bound
/// a ruff or mypy record too, which is a change to those parsers and not this
/// boundary's to make.
pub(crate) fn opens_directive(after_marker: &str) -> bool {
    python::opens_directive(after_marker)
        || shellcheck::opens_directive(after_marker)
        || eslint::opens_directive(after_marker)
        || biome::opens_directive(after_marker)
        || typescript::opens_directive(after_marker)
}

/// Every registered parser, in [`Tool::ALL`] order.
///
/// Every tool the contract declares has one, and `tests/tools_contract.rs`
/// fails the build if this list and [`Tool::ALL`] ever stop naming the same set.
pub fn registry() -> Vec<Box<dyn ToolParser>> {
    vec![
        Box::new(eslint::EslintParser),
        Box::new(biome::BiomeParser),
        Box::new(ruff::RuffParser),
        Box::new(typescript::TypescriptParser),
        Box::new(mypy::MypyParser),
        Box::new(pyright::PyrightParser),
        Box::new(ty::TyParser),
        Box::new(rust::RustParser),
        Box::new(shellcheck::ShellcheckParser),
        Box::new(llmlint::LlmlintParser),
    ]
}

/// The parsers for `tools`, or all of them when `tools` is empty.
pub fn registry_for(tools: &[Tool]) -> Vec<Box<dyn ToolParser>> {
    registry()
        .into_iter()
        .filter(|parser| tools.is_empty() || tools.contains(&parser.tool()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_holds_one_parser_per_tool() {
        let tools: Vec<Tool> = registry().iter().map(|p| p.tool()).collect();
        let mut sorted = tools.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            tools.len(),
            "a tool is registered twice: {tools:?}"
        );
        assert_eq!(
            tools,
            vec![
                Tool::Eslint,
                Tool::Biome,
                Tool::Ruff,
                Tool::Typescript,
                Tool::Mypy,
                Tool::Pyright,
                Tool::Ty,
                Tool::Rust,
                Tool::Shellcheck,
                Tool::Llmlint
            ]
        );
    }

    /// A wrapped reason stops at any tool's directive, so every tool with a
    /// comment grammar has to be in this union — one that is not would be
    /// swallowed as the justification of the suppression above it.
    #[test]
    fn every_comment_grammar_counts_as_a_directive_opening() {
        for directive in [
            " noqa: E501",
            " ruff: noqa",
            " type: ignore",
            " mypy: ignore-errors",
            " pyright: ignore",
            " ty: ignore",
            " shellcheck disable=SC2086",
            " eslint-disable-next-line no-alert",
            " eslint-disable-line",
            " biome-ignore lint/style/useConst: paused on purpose",
            " @ts-expect-error the API is untyped",
            " @ts-nocheck",
        ] {
            assert!(opens_directive(directive), "{directive:?}");
        }
        // Prose, a mode switch this crate does not report, and the two forms
        // their own tools honour only in a block comment.
        for prose in [
            " and the rest of the sentence",
            " pyright: basic",
            " shellcheckish",
            " eslint-disable",
            " eslint-enable",
        ] {
            assert!(!opens_directive(prose), "{prose:?}");
        }
    }

    /// A parser needs nothing beyond the three contract methods to be
    /// registered — if the trait ever grows a fourth, this stops compiling.
    #[test]
    fn the_three_contract_methods_are_all_a_parser_must_implement() {
        struct Minimal;

        impl ToolParser for Minimal {
            fn tool(&self) -> Tool {
                Tool::Eslint
            }
            fn applies_to(&self, _: &SourceFile) -> bool {
                false
            }
            fn parse(&self, _: &SourceFile) -> Vec<IgnoreDirective> {
                Vec::new()
            }
        }

        let parser: Box<dyn ToolParser> = Box::new(Minimal);
        let file = SourceFile::new("a.py", "x = 1  # noqa\n".to_string());
        assert_eq!(parser.tool(), Tool::Eslint);
        assert!(!parser.applies_to(&file));
        assert!(parser.parse(&file).is_empty());
    }

    #[test]
    fn the_registry_is_listed_in_tool_all_order() {
        let tools: Vec<Tool> = registry().iter().map(|p| p.tool()).collect();
        let mut expected = tools.clone();
        expected.sort_by_key(|t| Tool::ALL.iter().position(|a| a == t));
        assert_eq!(tools, expected);
    }

    #[test]
    fn filtering_selects_the_named_tools_only() {
        assert_eq!(registry_for(&[]).len(), registry().len());
        assert_eq!(registry_for(&[Tool::Ruff]).len(), 1);
        assert_eq!(registry_for(&[Tool::Eslint, Tool::Biome]).len(), 2);
        assert_eq!(registry_for(&[Tool::Mypy, Tool::Ty]).len(), 2);
        assert_eq!(registry_for(&[Tool::Rust, Tool::Llmlint]).len(), 2);
    }
}
