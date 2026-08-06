//! One pass over a tree that speaks every language, in both modes.
//!
//! Each parser has its own parity suite proving it agrees with the tool it
//! claims to read. The product promise is the layer above that: **one** fast
//! pass over a mixed repository, where ten tools' directives share a tree, a
//! file, and sometimes a line. This is where that composition is proven — the
//! whole-folder inventory, the `--diff` review a pull request gets, and the
//! comment body the action posts from it, each against a checked-in golden.
//!
//! `tests/fixtures/polyglot/` is that tree: Python, TypeScript, JavaScript,
//! Rust, and shell, with llmlint directives hosted in three of them, every scope
//! the contract defines, reasons that wrap across lines, and decoys inside string
//! literals. Re-bless with `just bless` after reviewing the diff.

use std::fs;
use std::path::Path;

use notignored::{Scope, Tool};

use crate::support::{commit, fixture, git, git_repo, notignored, parse_report, repo_root, write};

/// The repo and commit the golden comment body's permalinks are built from.
/// Fixed, so the body is a byte-stable artifact rather than a function of the
/// checkout.
const REPO: &str = "acme/widgets";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn polyglot_dir() -> std::path::PathBuf {
    fixture("polyglot")
}

/// Compare `actual` against the checked-in golden at `relative`, writing it
/// instead when blessing.
fn assert_golden(relative: &str, actual: &str) {
    let path = repo_root().join(relative);
    if std::env::var_os("NOTIGNORED_BLESS").is_some() {
        fs::create_dir_all(path.parent().expect("a golden directory"))
            .expect("create the golden directory");
        fs::write(&path, actual).expect("write the golden report");
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    assert_eq!(
        actual,
        expected,
        "{} changed. If the change is intended, re-run with NOTIGNORED_BLESS=1 \
         and bump REPORT_VERSION when the shape (not just the data) moved.",
        path.display()
    );
}

/// Run the binary in `dir`, asserting it exited cleanly, and return its stdout.
fn run(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = notignored(dir).args(args).output().expect("run notignored");
    assert!(
        output.status.success(),
        "exit {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn the_polyglot_tree_matches_its_checked_in_golden_report() {
    let stdout = run(&polyglot_dir(), &["--format", "json"]);
    assert_golden(
        "tests/golden/polyglot.json",
        &String::from_utf8(stdout).expect("a UTF-8 report"),
    );
}

/// Every tool in the contract, and every scope, in one pass.
///
/// A parser that stopped applying — or one whose language claim narrowed — would
/// still pass its own suite while quietly dropping out of the combined run this
/// asserts on.
#[test]
fn one_pass_reports_every_tool_and_every_scope() {
    let report = parse_report(&run(&polyglot_dir(), &["--format", "json"]));
    let found: Vec<(&str, &str, &str)> = report["ignores"]
        .as_array()
        .expect("an ignores array")
        .iter()
        .map(|directive| {
            (
                directive["tool"].as_str().expect("a tool"),
                directive["scope"].as_str().expect("a scope"),
                directive["path"].as_str().expect("a path"),
            )
        })
        .collect();

    for tool in Tool::ALL {
        assert!(
            found.iter().any(|(named, _, _)| *named == tool.as_str()),
            "the polyglot tree no longer exercises {tool}: {found:#?}"
        );
    }
    for scope in [Scope::Line, Scope::NextLine, Scope::File, Scope::Block] {
        assert!(
            found.iter().any(|(_, named, _)| *named == scope.as_str()),
            "the polyglot tree no longer exercises the {scope} scope: {found:#?}"
        );
    }

    // llmlint's directive is hosted in the comment syntax of whatever language
    // it lands in, so the one parser has to work across all three.
    let mut hosts: Vec<&str> = found
        .iter()
        .filter(|(tool, _, _)| *tool == Tool::Llmlint.as_str())
        .map(|(_, _, path)| *path)
        .collect();
    hosts.dedup();
    assert_eq!(
        hosts,
        vec![
            "api/service.py",
            "api/vendored.py",
            "crates/tables.rs",
            "scripts/release.sh",
            "web/widget.ts",
        ],
        "llmlint's directive is no longer read in every host language"
    );

    // A directive quoted inside a string literal is never a suppression, in any
    // of them, and a language with no comment grammar is not scanned at all.
    assert!(
        !found.iter().any(|(_, _, path)| *path == "docs/notes.md"),
        "a markdown heading was read as a directive: {found:#?}"
    );
    // Four in the Rust file: its inner attribute, its two outer ones, and the
    // llmlint directive — and not the `#[allow]` its last line quotes inside a
    // string literal.
    assert_eq!(
        found
            .iter()
            .filter(|(_, _, path)| *path == "crates/tables.rs")
            .count(),
        4,
        "an attribute inside a string literal was reported: {found:#?}"
    );
}

/// One line, four `#` runs, two live directives — and each record covers its own.
///
/// This is the inversion the tool exists to prevent: without a boundary between
/// them, ruff's live `# noqa: F401` reads as mypy's stated justification and a
/// reviewer sees a suppression that looks explained when it is not. The Python
/// parity suite proves it against the real checkers; this pins it in the tree
/// every other assertion here reads.
#[test]
fn a_shared_line_gives_each_tool_its_own_record_reason_and_span() {
    let report = parse_report(&run(&polyglot_dir(), &["--format", "json"]));
    let shared: Vec<&serde_json::Value> = report["ignores"]
        .as_array()
        .expect("an ignores array")
        .iter()
        .filter(|directive| directive["path"] == "api/service.py" && directive["line"] == 4)
        .collect();

    assert_eq!(shared.len(), 2, "{report:#}");
    assert_eq!(shared[0]["tool"], "mypy");
    assert_eq!(shared[0]["rules"], serde_json::json!(["import-not-found"]));
    assert_eq!(shared[0]["reason"], "no stubs published");
    assert_eq!(
        shared[0]["raw"],
        "# type: ignore[import-not-found]  # no stubs published"
    );
    assert_eq!(shared[1]["tool"], "ruff");
    assert_eq!(shared[1]["rules"], serde_json::json!(["F401"]));
    assert_eq!(shared[1]["reason"], "imported for its side effects");
    assert_eq!(
        shared[1]["raw"],
        "# noqa: F401  # imported for its side effects"
    );
    // Neither record may quote the other's directive anywhere a reviewer reads.
    for directive in shared {
        let foreign = if directive["tool"] == "mypy" {
            "noqa"
        } else {
            "type: ignore"
        };
        for field in ["raw", "reason"] {
            assert!(
                !directive[field]
                    .as_str()
                    .expect("a string field")
                    .contains(foreign),
                "the {} record's {field} swallowed the other directive: {directive:#}",
                directive["tool"]
            );
        }
    }
}

/// Copy a directory tree, so a fixture can be committed to a scratch repository
/// and then changed the way a branch changes it.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create the destination directory");
    for entry in fs::read_dir(from).expect("read the fixture tree") {
        let entry = entry.expect("a directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("an entry type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy a fixture file");
        }
    }
}

/// Append to a file already in the tree, the way a change adds a line to it.
fn append(root: &Path, name: &str, extra: &str) {
    let path = root.join(name);
    let existing = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    fs::write(&path, format!("{existing}{extra}"))
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

/// The polyglot tree committed on `main`, then changed on a branch the way a
/// pull request changes it: three new suppressions, in three languages, in a
/// tree that already carries two dozen.
fn polyglot_branch() -> tempfile::TempDir {
    let repo = git_repo();
    let root = repo.path();
    copy_tree(&polyglot_dir(), root);
    commit(root, "the tree as it stands");

    git(root, &["checkout", "-q", "-b", "feature"]);
    // A file that carried no suppression at all gains one: the review case a
    // whole-tree inventory cannot distinguish from the twenty-odd it inherited.
    append(
        root,
        "api/clean.py",
        "\n\ndef widths(rows):  # noqa: ANN001  # the gateway fixes this signature\n\
         \x20   return [len(row) for row in rows]\n",
    );
    append(
        root,
        "web/widget.ts",
        "\n// @ts-expect-error the SDK's teardown hook is untyped\nsdk.teardown();\n",
    );
    write(
        root,
        "crates/lexer.rs",
        "#[expect(dead_code, reason = \"error recovery lands with the next parser\")]\n\
         fn recover() {}\n",
    );
    commit(root, "add the gateway fan-out");
    repo
}

/// The review a pull request actually gets: only what the change added, rendered
/// both as the JSON report and as the comment body the action posts.
#[test]
fn a_diff_over_the_polyglot_tree_matches_its_checked_in_goldens() {
    let repo = polyglot_branch();
    let root = repo.path();

    let json = run(root, &["--diff", "--diff-base", "main", "--format", "json"]);
    assert_golden(
        "tests/golden/polyglot-diff.json",
        &String::from_utf8(json.clone()).expect("a UTF-8 report"),
    );

    let body = run(
        root,
        &[
            "--diff",
            "--diff-base",
            "main",
            "--format",
            "markdown",
            "--github-repo",
            REPO,
            "--github-sha",
            SHA,
        ],
    );
    assert_golden(
        "tests/golden/markdown/polyglot-diff.md",
        &String::from_utf8(body).expect("a UTF-8 comment body"),
    );
}

/// The inventory the branch inherited is not the branch's own work.
///
/// A whole-tree scan of the same repository reports every directive; the diff
/// reports the three the change wrote, in the three files it touched, and says
/// nothing about the twenty-four it did not.
#[test]
fn the_diff_reports_the_changes_own_suppressions_and_none_it_inherited() {
    let repo = polyglot_branch();
    let root = repo.path();

    let described = |args: &[&str]| -> Vec<String> {
        parse_report(&run(root, args))["ignores"]
            .as_array()
            .expect("an ignores array")
            .iter()
            .map(|directive| {
                format!(
                    "{}:{} {}",
                    directive["path"].as_str().expect("a path"),
                    directive["line"].as_u64().expect("a line"),
                    directive["tool"].as_str().expect("a tool"),
                )
            })
            .collect()
    };

    let inherited = described(&["--format", "json"]);
    assert_eq!(inherited.len(), 29, "{inherited:#?}");

    let added = described(&["--diff", "--diff-base", "main", "--format", "json"]);
    assert_eq!(
        added,
        vec![
            "api/clean.py:4 ruff",
            "crates/lexer.rs:1 rust",
            "web/widget.ts:17 typescript",
        ]
    );
    // Every one of them is in the inventory too: --diff narrows the report, it
    // does not parse anything differently.
    for directive in &added {
        assert!(inherited.contains(directive), "{inherited:#?}");
    }

    // Narrowing by path narrows the change as well.
    assert_eq!(
        described(&["--diff", "--diff-base", "main", "--format", "json", "web"]),
        vec!["web/widget.ts:17 typescript"]
    );
}
