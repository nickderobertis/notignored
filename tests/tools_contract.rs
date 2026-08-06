//! Keeps the three places a tool is declared from drifting apart.
//!
//! Adding a tool touches its module, one registry line, and one README row.
//! These tests fail when any one of the three is forgotten — which is what keeps
//! parallel parser branches from silently disagreeing about the tool set.

use notignored::{tools::registry, Tool};

fn readme() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
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
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools/mod.rs"),
    )
    .unwrap();
    for tool in Tool::ALL.into_iter().filter(|tool| !tool.is_implemented()) {
        assert!(
            source.contains(&format!("// {}: planned", tool.as_str())),
            "src/tools/mod.rs has no placeholder line for the planned tool `{tool}`"
        );
    }
}
