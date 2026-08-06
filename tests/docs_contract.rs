//! Drift gate between the code and the docs that describe it.
//!
//! The exit codes live in three places — the [`notignored::cli`] constants, the
//! `--help` text, and the README table — and the report envelope is spelled out
//! a fourth time as a README example. Each duplication is there because users
//! need it where they are; these tests are what stop the copies from disagreeing.

use clap::CommandFactory;
use notignored::cli::{Cli, EXIT_ERROR, EXIT_FOUND, EXIT_OK};
use notignored::Report;

fn readme() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// The section of the README under `heading`, up to the next heading of the same
/// or a higher level.
fn section(readme: &str, heading: &str) -> String {
    let level = heading.chars().take_while(|c| *c == '#').count();
    readme
        .lines()
        .skip_while(|line| line.trim() != heading)
        .skip(1)
        .take_while(|line| !(line.starts_with('#') && heading_level(line) <= level))
        .collect::<Vec<_>>()
        .join("\n")
}

fn heading_level(line: &str) -> usize {
    line.chars().take_while(|c| *c == '#').count()
}

#[test]
fn the_readme_exit_code_table_matches_the_constants() {
    let table = section(&readme(), "### Exit codes");
    let documented: Vec<u8> = table
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter_map(|line| {
            line.trim_start_matches("| `")
                .split('`')
                .next()?
                .parse()
                .ok()
        })
        .collect();
    assert_eq!(
        documented,
        vec![EXIT_OK, EXIT_FOUND, EXIT_ERROR],
        "the README exit-code table drifted from the cli constants"
    );
}

#[test]
fn the_help_text_documents_the_same_exit_codes() {
    let long_about = Cli::command()
        .get_long_about()
        .expect("the command has long help")
        .to_string();
    let codes = long_about
        .lines()
        .skip_while(|line| !line.contains("Exit codes:"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !codes.is_empty(),
        "--help no longer documents the exit codes"
    );
    for code in [EXIT_OK, EXIT_FOUND, EXIT_ERROR] {
        assert!(
            codes.contains(&format!("{code}  ")),
            "--help does not document exit code {code}:\n{codes}"
        );
    }
}

/// The first fenced JSON block in the README's `## Output` section.
fn readme_report_example(readme: &str) -> String {
    let output = section(readme, "## Output");
    output
        .lines()
        .skip_while(|line| line.trim() != "```json")
        .skip(1)
        .take_while(|line| line.trim() != "```")
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_readme_report_example_is_a_valid_current_shape_report() {
    let example = readme_report_example(&readme());
    assert!(
        !example.is_empty(),
        "the README no longer shows a JSON report example"
    );

    let raw: serde_json::Value =
        serde_json::from_str(&example).expect("the README example is valid JSON");
    let parsed: Report =
        serde_json::from_str(&example).expect("the README example deserializes as a Report");

    // Re-serializing and comparing catches a field the example invented, renamed,
    // or dropped — not just one the parser happened to tolerate.
    assert_eq!(
        serde_json::to_value(&parsed).unwrap(),
        raw,
        "the README report example drifted from the serialized shape"
    );
    assert_eq!(parsed.version, notignored::REPORT_VERSION);
}

/// The TypeScript parity claim names the compiler it was proven against, and
/// names the one actually installed.
///
/// TypeScript 7 is a different implementation from the 5.x compiler, so "we
/// agree with tsc" is only true of a particular one. A version spelled out in
/// prose rots the first time the pin moves; this is what makes moving the pin
/// move the sentence.
#[test]
fn the_readme_names_the_typescript_the_parity_claim_is_pinned_to() {
    let manifest =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/js-toolchain/package.json");
    let pins: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", manifest.display())),
    )
    .expect("the JS toolchain manifest is valid JSON");
    let pinned = pins["dependencies"]["typescript"]
        .as_str()
        .expect("the manifest pins a typescript version");

    let tools = section(&readme(), "## Supported tools");
    assert!(
        tools.contains("tests/js-toolchain/package.json"),
        "the README no longer says where the typescript parity pin lives"
    );
    assert!(
        tools.contains(pinned),
        "the README claims parity with a typescript other than the pinned {pinned}"
    );
}

#[test]
fn the_readme_documents_every_scope_the_contract_defines() {
    let output = section(&readme(), "## Output");
    for scope in ["line", "next-line", "file", "block"] {
        assert!(
            output.contains(&format!("`{scope}`")),
            "the README omits the `{scope}` scope"
        );
    }
}
