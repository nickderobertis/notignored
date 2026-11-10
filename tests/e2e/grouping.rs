//! Identical suppressions, presented as one entry, by the real binary.
//!
//! `tests/fixtures/grouping/` writes each part of the identity rule as source a
//! parser really reads: the same rules reordered, repeated and respaced, and
//! beside them a blanket, a subset, a case change, another reason, another
//! scope and another tool, each of which must stay an entry of its own. Its two
//! Python files share decisions, so a group spans files. `tests/fixtures/
//! grouping-cap/` holds the counts the markdown caps decide on.
//!
//! The `--no-group` outputs are compared with goldens recorded from the binary
//! *before* the renderers grouped anything (commit 04eb8a7), so they prove the
//! flag restores that output rather than whatever this build prints. They are
//! never re-blessed; the grouped goldens are, with `just bless`.

use std::fs;
use std::process::Output;

use crate::support::{commit, git, git_repo, notignored, parse_report, repo_root, write};

/// The repo and commit permalinks are built from.
const REPO: &str = "acme/widgets";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// The identity-rule fixture tree, as the report names it.
const TREE: &str = "tests/fixtures/grouping";
/// The cap fixtures' directory.
const CAP: &str = "tests/fixtures/grouping-cap";

/// Run the binary from the repository root and require a clean exit.
fn run(args: &[&str]) -> Output {
    let output = notignored(&repo_root())
        .args(args)
        .output()
        .expect("run notignored");
    assert!(
        output.status.success(),
        "notignored {args:?} exited {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("UTF-8 stdout")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("UTF-8 stderr")
}

/// The human report over `paths`, with `extra` flags.
fn human(paths: &[&str], extra: &[&str]) -> (String, String) {
    let args: Vec<&str> = paths.iter().chain(extra).copied().collect();
    let output = run(&args);
    (stdout(&output), stderr(&output))
}

/// The markdown body over `paths`, with permalinks and `extra` flags.
fn markdown(paths: &[&str], extra: &[&str]) -> String {
    let mut args: Vec<&str> = paths.to_vec();
    args.extend([
        "--format",
        "markdown",
        "--github-repo",
        REPO,
        "--github-sha",
        SHA,
    ]);
    args.extend(extra);
    stdout(&run(&args))
}

/// The permalink markdown renders for `path`, line `line`.
fn permalink(path: &str, line: u32) -> String {
    format!("[{path}:{line}](https://github.com/{REPO}/blob/{SHA}/{path}#L{line})")
}

/// A fixture file's report path.
fn src(file: &str) -> String {
    format!("{TREE}/src/{file}")
}

/// The human output's entry heads: every line that is not an indented location.
fn human_heads(out: &str) -> Vec<&str> {
    out.lines().filter(|line| !line.starts_with("  ")).collect()
}

/// The locations listed under the human header `head`.
fn human_locations<'a>(out: &'a str, head: &str) -> Vec<&'a str> {
    out.lines()
        .skip_while(|line| *line != head)
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(str::trim_start)
        .collect()
}

/// The markdown body's top-level bullets, in order.
fn bullets(body: &str) -> Vec<&str> {
    body.lines().filter(|line| line.starts_with("- ")).collect()
}

/// The nested location bullets under the top-level bullet that starts with
/// `head`.
fn nested<'a>(body: &'a str, head: &str) -> Vec<&'a str> {
    body.lines()
        .skip_while(|line| !line.starts_with(head))
        .skip(1)
        .take_while(|line| line.starts_with("  - "))
        .map(|line| &line["  - ".len()..])
        .collect()
}

/// The block from the bullet that starts with `head` up to the next top-level
/// bullet or the closing material.
fn block<'a>(body: &'a str, head: &str) -> &'a str {
    let start = body
        .find(&format!("\n{head}"))
        .unwrap_or_else(|| panic!("no bullet starting {head:?} in:\n{body}"))
        + 1;
    let rest = &body[start..];
    let end = rest[1..]
        .find("\n- ")
        .or_else(|| rest.find("\n---"))
        .map_or(rest.len(), |end| end + 1);
    &rest[..end]
}

/// The identity rule, read off the human report: which suppressions share a
/// header, which stand alone, and in what order.
#[test]
fn the_human_report_groups_by_tool_scope_rule_set_and_reason() {
    let (out, _) = human(&[TREE], &[]);
    let models = src("models.py");
    let views = src("views.py");
    assert_eq!(
        human_heads(&out),
        vec![
            // Two spans of different length, written with different spacing.
            "rust dead_code (next-line) (2 locations) -- kept for the plugin API".to_string(),
            // The same rules reordered, repeated and respaced, in two files.
            "ruff E501,F401 (line) (4 locations) -- re-exported for the plugin API".to_string(),
            // A blanket, a subset, a case change, another reason, another scope.
            format!("{models}:6:13 ruff * (line) -- re-exported for the plugin API"),
            format!("{models}:7:12 ruff F401 (line) -- re-exported for the plugin API"),
            format!("{models}:8:13 ruff e501,F401 (line) -- re-exported for the plugin API"),
            format!("{models}:9:13 ruff E501,F401 (line) -- re-exported for the public API"),
            format!("{models}:10:1 ruff E501,F401 (file) -- re-exported for the plugin API"),
            "mypy attr-defined (line) (2 locations) -- the stub predates this constant".to_string(),
            // The same rule and reason under another tool.
            format!("{models}:13:21 ty attr-defined (line) -- the stub predates this constant"),
            format!("{views}:6:25 pyright reportPrivateUsage (line) -- the private cache is ours"),
        ],
        "{out}"
    );
    assert_eq!(
        human_locations(
            &out,
            "ruff E501,F401 (line) (4 locations) -- re-exported for the plugin API"
        ),
        vec![
            format!("{models}:3:14"),
            format!("{models}:4:12"),
            format!("{models}:5:12"),
            format!("{views}:3:23"),
        ],
        "{out}"
    );
    assert_eq!(
        human_locations(
            &out,
            "rust dead_code (next-line) (2 locations) -- kept for the plugin API"
        ),
        vec![
            format!("{}:4:1", src("lib.rs")),
            format!("{}:7:1", src("lib.rs"))
        ],
        "{out}"
    );
    assert_eq!(
        human_locations(
            &out,
            "mypy attr-defined (line) (2 locations) -- the stub predates this constant"
        ),
        vec![format!("{models}:12:19"), format!("{views}:5:28")],
        "{out}"
    );
}

/// The same rule, read off the comment body: one bullet per decision, in the
/// order of their first suppression, each group's locations in report order.
#[test]
fn the_markdown_body_groups_by_the_same_rule() {
    let body = markdown(&[TREE], &[]);
    let (models, views, lib) = (src("models.py"), src("views.py"), src("lib.rs"));
    assert_eq!(
        bullets(&body),
        vec![
            "- **rust dead\\_code** — _kept for the plugin API_ — 2 locations".to_string(),
            "- **ruff E501, F401** — _re-exported for the plugin API_ — 4 locations".to_string(),
            format!(
                "- **ruff (all rules)** — _re-exported for the plugin API_ — {}",
                permalink(&models, 6)
            ),
            format!(
                "- **ruff F401** — _re-exported for the plugin API_ — {}",
                permalink(&models, 7)
            ),
            format!(
                "- **ruff e501, F401** — _re-exported for the plugin API_ — {}",
                permalink(&models, 8)
            ),
            format!(
                "- **ruff E501, F401** — _re-exported for the public API_ — {}",
                permalink(&models, 9)
            ),
            format!(
                "- **ruff E501, F401** — _re-exported for the plugin API_ — {}",
                permalink(&models, 10)
            ),
            "- **mypy attr-defined** — _the stub predates this constant_ — 2 locations".to_string(),
            format!(
                "- **ty attr-defined** — _the stub predates this constant_ — {}",
                permalink(&models, 13)
            ),
            format!(
                "- **pyright reportPrivateUsage** — _the private cache is ours_ — {}",
                permalink(&views, 6)
            ),
        ],
        "{body}"
    );
    assert_eq!(
        nested(
            &body,
            "- **ruff E501, F401** — _re-exported for the plugin API_ — 4"
        ),
        vec![
            permalink(&models, 3),
            permalink(&models, 4),
            permalink(&models, 5),
            permalink(&views, 3),
        ],
        "{body}"
    );
    assert_eq!(
        nested(&body, "- **rust dead\\_code**"),
        vec![permalink(&lib, 4), permalink(&lib, 7)],
        "{body}"
    );
    assert_eq!(
        nested(&body, "- **mypy attr-defined**"),
        vec![permalink(&models, 12), permalink(&views, 5)],
        "{body}"
    );
}

/// Grouping is a way of reading the report, not a smaller report: the human
/// summary and the comment heading count every suppression, and the summary
/// every file they sit in.
#[test]
fn the_counts_stay_counts_of_suppressions_and_files() {
    let (_, err) = human(&[TREE], &[]);
    assert_eq!(err, "notignored: 15 ignores in 3 files\n");
    let body = markdown(&[TREE], &[]);
    assert!(
        body.contains("### notignored: 15 suppressions\n"),
        "the heading counts groups rather than suppressions:\n{body}"
    );
}

/// A suppression with no twin, beside groups, is exactly what it always was:
/// one human line with its location inline, one bullet with its own permalink
/// and its own snippet.
#[test]
fn a_suppression_with_no_twin_renders_on_its_own() {
    let views = src("views.py");
    let (out, _) = human(&[TREE], &[]);
    let line =
        format!("{views}:6:25 pyright reportPrivateUsage (line) -- the private cache is ours");
    assert!(out.lines().any(|candidate| candidate == line), "{out}");
    assert!(!line.contains("locations"));

    let body = markdown(&[TREE], &[]);
    let entry = block(&body, "- **pyright reportPrivateUsage**");
    assert!(
        entry.starts_with(&format!(
            "- **pyright reportPrivateUsage** — _the private cache is ours_ — {}\n\n  <details>\n  <summary>suppressed code</summary>\n",
            permalink(&views, 6)
        )),
        "{entry}"
    );
    assert!(
        !entry.contains("\n  - "),
        "a single entry nested a list:\n{entry}"
    );
    assert_eq!(entry.matches("<details>").count(), 1, "{entry}");
}

/// A group carries one snippet, its first location's, and names that location.
#[test]
fn a_group_carries_one_snippet_naming_its_first_location() {
    let body = markdown(&[TREE], &[]);
    let models = src("models.py");
    let entry = block(
        &body,
        "- **ruff E501, F401** — _re-exported for the plugin API_ — 4",
    );
    assert_eq!(entry.matches("<details>").count(), 1, "{entry}");
    assert!(
        entry.contains(&format!(
            "  <summary>suppressed code ({models}:3)</summary>\n"
        )),
        "{entry}"
    );
    assert!(
        entry.contains(
            "  > 3 | import json  # noqa: E501, F401  # re-exported for the plugin API\n"
        ),
        "{entry}"
    );
}

/// Ten locations are every one listed, each linked to its own line at the sha.
#[test]
fn a_group_of_ten_lists_every_location() {
    let path = format!("{CAP}/ten.py");
    let body = markdown(&[&path], &[]);
    let expected: Vec<String> = (3..=12).map(|line| permalink(&path, line)).collect();
    assert_eq!(nested(&body, "- **ruff E501**"), expected, "{body}");
    assert!(!body.contains("more location"), "{body}");
    assert!(body.contains(" — 10 locations\n"), "{body}");
    assert_eq!(body.matches("<details>").count(), 1, "{body}");
    assert!(
        body.contains(&format!("<summary>suppressed code ({path}:3)</summary>")),
        "{body}"
    );
}

/// The eleventh location is counted, not listed.
#[test]
fn a_group_of_eleven_lists_ten_and_counts_one_more() {
    let path = format!("{CAP}/eleven.py");
    let body = markdown(&[&path], &[]);
    let mut expected: Vec<String> = (3..=12).map(|line| permalink(&path, line)).collect();
    expected.push("… and 1 more location".to_string());
    assert_eq!(nested(&body, "- **ruff E501**"), expected, "{body}");
    assert!(!body.contains(&format!("{path}#L13")), "{body}");
    assert!(body.contains(" — 11 locations\n"), "{body}");
}

/// Twenty-five identical suppressions are one entry, so the three distinct
/// ones beside them fit under the default cap with room to spare.
#[test]
fn twenty_five_identical_and_three_distinct_are_four_entries() {
    let path = format!("{CAP}/many.py");
    let body = markdown(&[&path], &[]);
    assert_eq!(
        bullets(&body),
        vec![
            format!(
                "- **ruff F401** — _re-exported for the plugin API_ — {}",
                permalink(&path, 3)
            ),
            format!(
                "- **ruff F811** — _the shim redefines it on purpose_ — {}",
                permalink(&path, 4)
            ),
            "- **ruff E501** — _the vendor URL cannot be wrapped_ — 25 locations".to_string(),
            format!(
                "- **ruff E402** — _imported after the path is patched_ — {}",
                permalink(&path, 30)
            ),
        ],
        "{body}"
    );
    assert!(body.contains("### notignored: 28 suppressions\n"), "{body}");
    assert!(!body.contains("not shown"), "{body}");
    assert!(body.contains("  - … and 15 more locations\n"), "{body}");
}

/// `--max-entries` counts entries. With a group among the unlisted, the closing
/// line names entries and suppressions; with only single suppressions unlisted
/// it is the line it always was.
#[test]
fn max_entries_counts_entries_and_the_closing_line_counts_both() {
    let path = format!("{CAP}/many.py");
    let body = markdown(&[&path], &["--max-entries", "2"]);
    assert_eq!(bullets(&body).len(), 2, "{body}");
    assert!(
        body.contains(
            "\n_… and 2 more entries (26 suppressions) not shown (28 suppressions total)._\n"
        ),
        "{body}"
    );

    let body = markdown(&[&path], &["--max-entries", "3"]);
    assert_eq!(bullets(&body).len(), 3, "{body}");
    assert!(
        body.contains("\n_… and 1 more not shown (28 total)._\n"),
        "{body}"
    );
}

/// Read a golden recorded before grouping existed. Never re-blessed.
fn base_golden(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// `--no-group` is the human report as it was: one line per suppression, each
/// with its own location, tool, rules and reason, and no count.
#[test]
fn no_group_prints_one_human_line_per_suppression() {
    let (out, err) = human(&[TREE], &["--no-group"]);
    assert_eq!(
        out,
        base_golden("tests/golden/human/grouping-ungrouped.txt")
    );
    assert_eq!(err, "notignored: 15 ignores in 3 files\n");
    assert_eq!(out.lines().count(), 15, "{out}");
    for line in out.lines() {
        assert!(line.starts_with(&format!("{TREE}/src/")), "{line}");
        assert!(line.contains(" -- "), "{line}");
        assert!(!line.contains("locations"), "{line}");
    }
    assert!(
        out.contains(&format!(
            "{}:4:12 ruff F401,E501 (line) -- re-exported for the plugin API\n",
            src("models.py")
        )),
        "{out}"
    );
}

/// `--no-group` is the comment body as it was: a bullet, a permalink and a
/// snippet per suppression, and the cap counting suppressions.
#[test]
fn no_group_renders_one_markdown_bullet_per_suppression() {
    let body = markdown(&[TREE], &["--no-group"]);
    assert_eq!(
        body,
        base_golden("tests/golden/markdown/grouping-ungrouped.md")
    );
    assert_eq!(bullets(&body).len(), 15, "{body}");
    assert!(!body.contains("\n  - "), "{body}");
    assert!(!body.contains("locations"), "{body}");
    assert_eq!(
        body.matches("  <summary>suppressed code</summary>\n")
            .count(),
        15,
        "{body}"
    );
    assert!(
        body.contains(&format!(
            "- **ruff F401, E501** — _re-exported for the plugin API_ — {}\n",
            permalink(&src("views.py"), 3)
        )),
        "{body}"
    );

    let capped = markdown(&[TREE], &["--no-group", "--max-entries", "3"]);
    assert_eq!(bullets(&capped).len(), 3, "{capped}");
    assert!(
        capped.contains("\n_… and 12 more not shown (15 total)._\n"),
        "{capped}"
    );
}

/// JSON is the record contract: one record per suppression whatever the flag
/// says, byte for byte what it was before grouping existed.
#[test]
fn json_is_one_record_per_suppression_with_or_without_no_group() {
    let grouped = run(&[TREE, "--format", "json"]).stdout;
    let ungrouped = run(&[TREE, "--format", "json", "--no-group"]).stdout;
    assert_eq!(grouped, ungrouped);
    assert_eq!(
        String::from_utf8(grouped.clone()).expect("UTF-8 JSON"),
        base_golden("tests/golden/grouping.json")
    );
    let report = parse_report(&grouped);
    assert_eq!(report["version"], 1);
    let records = report["ignores"].as_array().expect("an ignores array");
    assert_eq!(records.len(), 15);
    for record in records {
        for field in ["tool", "scope", "rules", "reason", "path", "line", "raw"] {
            assert!(record.get(field).is_some(), "{field} missing: {record}");
        }
        assert!(record.get("locations").is_none(), "{record}");
    }
}

/// The grouped renderings, recorded. The assertions above are the proof; these
/// make a change to the layout visible in review. Re-bless with `just bless`.
#[test]
fn golden_grouped_renderings_render_as_checked_in() {
    for (relative, actual) in [
        ("tests/golden/human/grouping.txt", human(&[TREE], &[]).0),
        ("tests/golden/markdown/grouping.md", markdown(&[TREE], &[])),
    ] {
        let path = repo_root().join(relative);
        if std::env::var_os("NOTIGNORED_BLESS").is_some() {
            fs::write(&path, &actual).expect("write the golden");
        }
        assert_eq!(
            actual,
            base_golden(relative),
            "{relative} changed. If the change is intended, re-run with NOTIGNORED_BLESS=1."
        );
    }
}

/// On a `--diff` run the change word is part of the decision: an added
/// suppression and a rewritten justification identical in every other way stay
/// two entries, so the edited marker is never folded away.
#[test]
fn a_diff_never_groups_an_addition_with_a_rewritten_justification() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "edited.py", "x = 1  # noqa: E501  # the old reason\n");
    write(root, "added.py", "y = 2\n");
    commit(root, "baseline");
    git(root, &["checkout", "-q", "-b", "feature"]);
    let directive = "# noqa: E501  # the vendor URL cannot be wrapped";
    write(root, "edited.py", &format!("x = 1  {directive}\n"));
    write(root, "added.py", &format!("y = 2  {directive}\n"));
    commit(root, "rejustify one, add its twin");

    let diff = |extra: &[&str]| {
        let output = notignored(root)
            .args(["--diff", "--diff-base", "main"])
            .args(extra)
            .output()
            .expect("run notignored");
        assert!(output.status.success(), "{}", stderr(&output));
        (stdout(&output), stderr(&output))
    };

    let (out, err) = diff(&[]);
    assert_eq!(
        out,
        "added.py:1:8 ruff E501 (line) -- the vendor URL cannot be wrapped\n\
         edited.py:1:8 ruff E501 (line) (justification edited) -- the vendor URL cannot be wrapped\n"
    );
    assert_eq!(
        err,
        "notignored: 1 added, 1 justification edited in 2 files\n"
    );

    let (body, _) = diff(&["--format", "markdown"]);
    assert!(
        body.contains("### notignored: 1 suppression added, 1 justification edited\n"),
        "{body}"
    );
    assert_eq!(
        bullets(&body),
        vec![
            "- **ruff E501** — _the vendor URL cannot be wrapped_ — `added.py:1`",
            "- **ruff E501** _(justification edited)_ — _the vendor URL cannot be wrapped_ — `edited.py:1`",
        ],
        "{body}"
    );
}

/// Run the binary in `dir` over `.` and require a clean exit.
fn run_in(dir: &std::path::Path, args: &[&str]) -> (String, String) {
    let output = notignored(dir).args(args).output().expect("run notignored");
    assert!(output.status.success(), "{}", stderr(&output));
    (stdout(&output), stderr(&output))
}

/// A missing reason is a reason of its own: two reason-less suppressions
/// group, and neither joins a reasoned one. Written at run time rather than
/// committed, because a committed reason-less directive is a finding this
/// repository's own scan would have to be told to ignore.
#[test]
fn suppressions_without_a_reason_group_only_with_each_other() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "app.py",
        "x = 1  # noqa: E501\ny = 2  # noqa: E501\nz = 3  # noqa: E501  # why\n",
    );
    let (out, _) = run_in(dir.path(), &[]);
    assert_eq!(
        out,
        "ruff E501 (line) (2 locations)\n  app.py:1:8\n  app.py:2:8\napp.py:3:8 ruff E501 (line) -- why\n"
    );
    let (body, _) = run_in(dir.path(), &["--format", "markdown"]);
    assert_eq!(
        bullets(&body),
        vec![
            "- **ruff E501** — _no reason given_ — 2 locations",
            "- **ruff E501** — _why_ — `app.py:3`",
        ],
        "{body}"
    );
    // Without the permalink flags a group's locations are code text.
    assert_eq!(
        nested(&body, "- **ruff E501** — _no reason given_"),
        vec!["`app.py:1`", "`app.py:2`"],
        "{body}"
    );
}

/// Colour stays presentation: a grouped report forced into colour reads the
/// same once the escapes are gone, and the count and every location carry the
/// location's role.
#[test]
fn a_colored_group_changes_no_character_and_colors_its_locations() {
    let (plain, plain_err) = human(&[TREE], &[]);
    let (colored, colored_err) = human(&[TREE], &["--color", "always"]);
    assert_ne!(colored, plain, "nothing was colorized");
    assert_eq!(strip_ansi(&colored), plain);
    assert_eq!(strip_ansi(&colored_err), plain_err);
    let cyan = |text: &str| format!("\u{1b}[36m{text}\u{1b}[0m");
    assert!(colored.contains(&cyan("(4 locations)")), "{colored:?}");
    for line in human_locations(
        &plain,
        "ruff E501,F401 (line) (4 locations) -- re-exported for the plugin API",
    ) {
        assert!(
            colored.contains(&format!("\n  {}\n", cyan(line))),
            "{line} is not in the location colour: {colored:?}"
        );
    }
}

/// Drop every SGR escape, leaving the text a terminal would show.
fn strip_ansi(text: &str) -> String {
    let mut plain = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('\u{1b}') {
        plain.push_str(&rest[..start]);
        let after = &rest[start..];
        let end = after.find('m').expect("an SGR escape ends in `m`");
        rest = &after[end + 1..];
    }
    plain.push_str(rest);
    plain
}

/// Rewritten justifications identical in every other way group like any
/// others, and the group keeps the marker the summary and heading count.
#[test]
fn a_diff_groups_identical_rewritten_justifications_under_their_marker() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "a.py", "x = 1  # noqa: E501  # the old reason\n");
    write(root, "b.py", "y = 2  # noqa: E501  # an older reason\n");
    commit(root, "baseline");
    let directive = "# noqa: E501  # the vendor URL cannot be wrapped";
    write(root, "a.py", &format!("x = 1  {directive}\n"));
    write(root, "b.py", &format!("y = 2  {directive}\n"));

    let (out, err) = run_in(root, &["--diff"]);
    assert_eq!(
        out,
        "ruff E501 (line) (2 locations) (justification edited) -- the vendor URL cannot be wrapped\n  a.py:1:8\n  b.py:1:8\n"
    );
    assert_eq!(
        err,
        "notignored: 0 added, 2 justifications edited in 2 files\n"
    );

    let (body, _) = run_in(root, &["--diff", "--format", "markdown"]);
    assert!(
        body.contains("### notignored: 2 justifications edited\n"),
        "{body}"
    );
    assert_eq!(
        bullets(&body),
        vec![
            "- **ruff E501** _(justification edited)_ — _the vendor URL cannot be wrapped_ — 2 locations"
        ],
        "{body}"
    );
    assert_eq!(
        nested(&body, "- **ruff E501**"),
        vec!["`a.py:1`", "`b.py:1`"],
        "{body}"
    );
}

/// A group's `<summary>` is raw HTML, where markdown's backslash escapes mean
/// nothing, so a path holding `&` is written as an entity.
#[test]
fn a_group_summary_escapes_its_path_as_html() {
    let dir = tempfile::tempdir().expect("tempdir");
    let directive = "# noqa: E501  # the vendor URL cannot be wrapped";
    write(
        dir.path(),
        "R&D.py",
        &format!("x = 1  {directive}\ny = 2  {directive}\n"),
    );
    let (body, _) = run_in(dir.path(), &["--format", "markdown"]);
    assert!(
        body.contains("  <summary>suppressed code (R&amp;D.py:1)</summary>\n"),
        "{body}"
    );
}
