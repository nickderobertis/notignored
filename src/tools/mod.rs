//! The tool-parser registry.
//!
//! Adding a tool touches exactly three places: a new module here, one line in
//! [`registry`], and one row in the README's supported-tools table. Keeping it
//! to those three keeps parallel branches from colliding.
//!
//! Parsers read the comments and attributes a [`SourceFile`] already extracted.
//! They must not re-scan raw source lines — that is how string literals get
//! mistaken for directives.

use crate::model::{IgnoreDirective, Tool};
use crate::source::SourceFile;

mod python;

pub mod biome;
pub mod eslint;
pub mod mypy;
pub mod pyright;
pub mod ruff;
pub mod rust;
pub mod ty;
pub mod typescript;

/// Turns one tool's suppression syntax into [`IgnoreDirective`] records.
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

/// Every registered parser, in [`Tool::ALL`] order.
///
/// Planned tools keep their place as a comment so the next contributor swaps one
/// line rather than reshuffling the list. Each planned entry stays listed in
/// [`Tool::ALL`] and in the README table, so the contract is visible before the
/// parser exists.
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
        // shellcheck: planned — `# shellcheck disable=SC2086  # reason`
        // llmlint: planned — its inline `ignore[rule] reason` directive
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
                Tool::Rust
            ]
        );
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
        assert!(registry_for(&[Tool::Shellcheck]).is_empty());
    }
}
