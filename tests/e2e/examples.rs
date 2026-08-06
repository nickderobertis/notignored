//! The README's `notignored examples/` block, run.
//!
//! `examples/` is documentation: it is what a reader sees before they install
//! anything, and the only claim in the README that shows the tool's whole
//! surface at once. A block hand-edited to say something the binary no longer
//! says is worse than no example, so this drives the real binary over the real
//! tree and compares byte for byte — the block is output, not prose about it.

use crate::support::{notignored, repo_root};

/// The lines of the first `console` fence under `## Try it`, without the `$`
/// command line that opens it.
fn documented_output() -> String {
    let readme = std::fs::read_to_string(repo_root().join("README.md")).expect("read README.md");
    let block: Vec<&str> = readme
        .lines()
        .skip_while(|line| line.trim() != "## Try it")
        .skip_while(|line| line.trim() != "```console")
        .skip(1)
        .take_while(|line| line.trim() != "```")
        .collect();
    assert!(
        block
            .first()
            .is_some_and(|line| *line == "$ notignored examples/"),
        "the README's `## Try it` section no longer opens with `$ notignored examples/`: {block:?}"
    );
    block[1..].join("\n") + "\n"
}

/// A terminal shows the findings (stdout) and then the summary (stderr), which
/// is how the README block is written.
#[test]
fn the_readme_shows_what_scanning_the_examples_tree_actually_prints() {
    let output = notignored(&repo_root())
        .arg("examples/")
        .output()
        .expect("run notignored");
    assert!(
        output.status.success(),
        "exit: {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let printed = String::from_utf8(output.stdout).expect("UTF-8 findings")
        + &String::from_utf8(output.stderr).expect("UTF-8 summary");
    assert_eq!(
        printed,
        documented_output(),
        "the README's examples block and the binary disagree; re-run \
         `notignored examples/` and paste what it prints"
    );
}

/// The tree is only documentation if it keeps covering more than one ecosystem:
/// a language dropped from it silently narrows what the README demonstrates.
#[test]
fn the_examples_tree_carries_a_reasoned_directive_for_several_ecosystems() {
    let output = notignored(&repo_root())
        .args(["examples/", "--format", "json"])
        .output()
        .expect("run notignored");
    let report = crate::support::parse_report(&output.stdout);
    let ignores = report["ignores"].as_array().expect("an ignores array");

    let mut suffixes: Vec<&str> = ignores
        .iter()
        .filter_map(|ignore| {
            ignore["path"]
                .as_str()?
                .rsplit_once('.')
                .map(|(_, ext)| ext)
        })
        .collect();
    suffixes.sort_unstable();
    suffixes.dedup();
    assert!(
        suffixes.len() >= 3,
        "the examples tree demonstrates only {suffixes:?}"
    );
    for ignore in ignores {
        assert!(
            ignore["reason"].as_str().is_some_and(|r| !r.is_empty()),
            "an example carries a suppression with no stated reason: {ignore:#}"
        );
    }
}
