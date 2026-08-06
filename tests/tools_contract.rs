//! Keeps the three places a tool is declared from drifting apart.
//!
//! Adding a tool touches its module, one registry line, and one README row.
//! These tests fail when any one of the three is forgotten — which is what keeps
//! parallel parser branches from silently disagreeing about the tool set.

use notignored::{tools::registry, Tool};

fn source(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// The `fn …;` / `fn … {` lines declared directly inside `pub trait <name>`.
fn trait_methods(source: &str, name: &str) -> Vec<String> {
    source
        .lines()
        .skip_while(|line| !line.starts_with(&format!("pub trait {name}")))
        .skip(1)
        .take_while(|line| *line != "}")
        .map(str::trim)
        .filter(|line| line.starts_with("fn "))
        .map(str::to_string)
        .collect()
}

/// `ToolParser` is a fixed contract: a parser hands back directives and nothing
/// else.
///
/// A fourth method — even a defaulted one carrying an error channel — changes
/// what every downstream implementor is promised, so it is a deliberate contract
/// move rather than an implementation detail. llmlint's unclosed-block errors,
/// which an `IgnoreDirective` cannot express, ride on an inherent method on its
/// own parser and are folded in by the scan layer instead.
///
/// A compile-time companion to this lives in `src/tools/mod.rs`: a minimal
/// parser implementing exactly these three and nothing more.
#[test]
fn the_tool_parser_trait_declares_exactly_its_three_methods() {
    let mod_source = source("src/tools/mod.rs");
    assert_eq!(
        trait_methods(&mod_source, "ToolParser"),
        vec![
            "fn tool(&self) -> Tool;",
            "fn applies_to(&self, file: &SourceFile) -> bool;",
            "fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective>;",
        ],
        "the ToolParser contract moved; see the note above this test"
    );
}

/// The scan layer is where a parser's richer result is integrated, and it names
/// the one tool that has one — so the seam stays visible instead of becoming a
/// trait everyone pays for.
#[test]
fn the_scan_layer_is_where_llmlints_extra_errors_are_folded_in() {
    let scan_source = source("src/scan.rs");
    assert!(
        scan_source.contains("LlmlintParser.scan(&file)"),
        "src/scan.rs no longer collects llmlint's unclosed-block errors"
    );
}

fn readme() -> String {
    source("README.md")
}

/// The supported-tools table, as `(tool, row)` pairs.
///
/// Scoped to the `## Supported tools` section so other tables in the README
/// (the exit codes, for one) cannot be mistaken for tool rows.
fn table_rows(readme: &str) -> Vec<(String, String)> {
    readme
        .lines()
        .skip_while(|line| line.trim() != "## Supported tools")
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter(|line| line.starts_with("| `"))
        .filter_map(|line| {
            let name = line
                .trim_start_matches("| `")
                .split('`')
                .next()?
                .to_string();
            Some((name, line.to_string()))
        })
        .collect()
}

#[test]
fn the_readme_table_lists_every_tool_exactly_once() {
    let readme = readme();
    let rows = table_rows(&readme);
    let listed: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
    let expected: Vec<&str> = Tool::ALL.iter().map(|tool| tool.as_str()).collect();
    assert_eq!(
        listed, expected,
        "the README supported-tools table must list every tool once, in Tool::ALL order"
    );
}

#[test]
fn the_readme_marks_unimplemented_tools_as_planned() {
    let readme = readme();
    for (name, row) in table_rows(&readme) {
        let tool: Tool = name.parse().expect("a README row names a real tool");
        let says_planned = row.to_lowercase().contains("planned");
        assert_eq!(
            says_planned,
            !tool.is_implemented(),
            "README row for `{name}` disagrees with the registry: {row}"
        );
    }
}

#[test]
fn every_registered_parser_is_a_declared_tool() {
    for parser in registry() {
        assert!(
            Tool::ALL.contains(&parser.tool()),
            "{} is registered but missing from Tool::ALL",
            parser.tool()
        );
        assert!(parser.tool().is_implemented());
    }
}

#[test]
fn the_planned_tools_are_visibly_placeholdered_in_the_registry() {
    // The registry keeps a commented placeholder per planned tool so a follow-up
    // PR swaps one line instead of reshuffling the list.
    let source = source("src/tools/mod.rs");
    for tool in Tool::ALL.into_iter().filter(|tool| !tool.is_implemented()) {
        assert!(
            source.contains(&format!("// {}: planned", tool.as_str())),
            "src/tools/mod.rs has no placeholder line for the planned tool `{tool}`"
        );
    }
}
