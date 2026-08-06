//! End-to-end journeys through `--diff`, over real git repositories.
//!
//! Every repository here is built with the real `git` binary and read by the
//! compiled `notignored` — nothing about the comparison is simulated, because
//! the promise `--diff-base` makes is precisely "the same set of changes git
//! (and therefore a pull request) would show you".

use std::fs;

use crate::support::{commit, git, git_repo, notignored, parse_report, repo_root, write};

/// Every suppression a JSON run reported, as `path:line rules`.
fn reported(stdout: &[u8]) -> Vec<String> {
    parse_report(stdout)["ignores"]
        .as_array()
        .expect("ignores is an array")
        .iter()
        .map(|directive| {
            let rules: Vec<&str> = directive["rules"]
                .as_array()
                .expect("rules is an array")
                .iter()
                .map(|rule| rule.as_str().expect("a rule is a string"))
                .collect();
            format!(
                "{}:{} {}",
                directive["path"].as_str().expect("path is a string"),
                directive["line"].as_u64().expect("line is a number"),
                if rules.is_empty() {
                    "*".to_string()
                } else {
                    rules.join(",")
                }
            )
        })
        .collect()
}

/// Run the binary in `dir` with `args`, asserting the exit code and returning
/// the suppressions it reported.
fn diff_run(dir: &std::path::Path, args: &[&str], expected_code: i32) -> Vec<String> {
    let output = notignored(dir)
        .args(args)
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(
        output.status.code(),
        Some(expected_code),
        "{:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    reported(&output.stdout)
}

#[test]
fn bare_diff_reports_only_the_suppressions_the_change_added() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "import os  # noqa: F401\nvalue = 1\n");
    commit(root, "baseline");
    // One line added below the untouched, pre-existing suppression.
    write(
        root,
        "app.py",
        "import os  # noqa: F401\nvalue = 1\nurl = URL  # noqa: E501\n",
    );

    assert_eq!(diff_run(root, &["--diff"], 0), vec!["app.py:3 E501"]);
    // Without --diff the whole inventory is still reported: the flag narrows the
    // report, it does not change how anything is parsed.
    assert_eq!(
        diff_run(root, &[], 0),
        vec!["app.py:1 F401", "app.py:3 E501"]
    );
}

#[test]
fn a_plain_diff_base_compares_from_the_merge_base() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "value = 1\n");
    write(root, "base_only.py", "legacy = 1  # noqa: E501\n");
    commit(root, "fork point");

    git(root, &["checkout", "-q", "-b", "feature"]);
    write(root, "app.py", "value = 1\nimport os  # noqa: F401\n");
    commit(root, "feature change");

    // main moves on after the fork, rewriting the line the feature branch still
    // carries. The branch is now *behind* its base on that file.
    git(root, &["checkout", "-q", "main"]);
    write(root, "base_only.py", "legacy = 1\n");
    commit(root, "base drift");
    git(root, &["checkout", "-q", "feature"]);

    // Three-dot: only what this branch did. The base's later edit is not this
    // branch's suppression, even though the line differs from the base tip.
    assert_eq!(
        diff_run(root, &["--diff", "--diff-base", "main"], 0),
        vec!["app.py:2 F401"]
    );

    // An explicit range is the caller's own choice of semantics: git's raw
    // two-dot comparison against the base *tip*, base drift and all.
    assert_eq!(
        diff_run(root, &["--diff", "--diff-base", "main..HEAD"], 0),
        vec!["app.py:2 F401", "base_only.py:1 E501"]
    );
}

#[test]
fn a_renamed_file_reports_only_what_the_change_added_to_it() {
    let repo = git_repo();
    let root = repo.path();
    let original = "import os  # noqa: F401\na = 1\nb = 2\nc = 3\nd = 4\ne = 5\n";
    write(root, "old.py", original);
    commit(root, "baseline");

    // A pure move: the suppression travelled, so nothing about it is new.
    git(root, &["mv", "old.py", "new.py"]);
    assert!(diff_run(root, &["--diff"], 0).is_empty());

    // The same move, plus a line: only the added line is new.
    write(
        root,
        "new.py",
        &format!("{original}url = URL  # noqa: E501\n"),
    );
    assert_eq!(diff_run(root, &["--diff"], 0), vec!["new.py:7 E501"]);
}

#[test]
fn staged_and_unstaged_changes_are_both_compared_against_head() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "staged.py", "value = 1\n");
    write(root, "unstaged.py", "value = 2\n");
    commit(root, "baseline");

    write(root, "staged.py", "value = 1  # noqa: E501\n");
    git(root, &["add", "staged.py"]);
    write(root, "unstaged.py", "value = 2  # noqa: F401\n");

    // `git diff HEAD` spans the index and the work tree, and so does this.
    assert_eq!(
        diff_run(root, &["--diff"], 0),
        vec!["staged.py:1 E501", "unstaged.py:1 F401"]
    );

    // Once committed there is nothing left to compare against HEAD...
    commit(root, "both suppressions");
    assert!(diff_run(root, &["--diff"], 0).is_empty());
    // ...but the commit itself still is a change, against the commit before it.
    assert_eq!(
        diff_run(root, &["--diff", "--diff-base", "HEAD~1"], 0),
        vec!["staged.py:1 E501", "unstaged.py:1 F401"]
    );
}

#[test]
fn positional_paths_narrow_the_change_and_an_empty_intersection_is_clean() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "src/app.py", "value = 1\n");
    write(root, "tools/helper.py", "value = 2\n");
    commit(root, "baseline");
    write(root, "src/app.py", "value = 1  # noqa: E501\n");
    write(root, "tools/helper.py", "value = 2  # noqa: F401\n");

    assert_eq!(
        diff_run(root, &["--diff", "src"], 0),
        vec!["src/app.py:1 E501"]
    );
    assert_eq!(
        diff_run(root, &["--diff", "src/app.py", "tools"], 0),
        vec!["src/app.py:1 E501", "tools/helper.py:1 F401"]
    );

    // A directory the change never touched reports nothing, quietly and cleanly.
    write(root, "docs/readme.py", "value = 3\n");
    git(root, &["add", "docs/readme.py"]);
    commit(root, "docs");
    let output = notignored(root)
        .args(["--diff", "docs"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(0), "{:?}", output.status);
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr, "notignored: 0 ignores in 0 files\n", "{stderr}");
}

#[test]
fn a_path_the_change_never_touched_and_that_does_not_exist_is_still_a_typo() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "value = 1  # noqa: E501\n");
    commit(root, "baseline");

    let output = notignored(root)
        .args(["--diff", "nope/"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("nope"), "{stderr}");
    assert!(stderr.contains("hint:"), "{stderr}");
}

#[test]
fn a_file_the_change_deleted_is_skipped_rather_than_reported_as_unreadable() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "gone.py", "value = 1  # noqa: E501\n");
    write(root, "kept.py", "value = 2\n");
    commit(root, "baseline");
    fs::remove_file(root.join("gone.py")).expect("delete the file");
    write(root, "kept.py", "value = 2  # noqa: F401\n");

    let output = notignored(root)
        .args(["--diff", "--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(0), "{:?}", output.status);
    let report = parse_report(&output.stdout);
    assert!(
        report["errors"].as_array().unwrap().is_empty(),
        "a deleted file is not an unreadable one: {report:#}"
    );
    assert_eq!(reported(&output.stdout), vec!["kept.py:1 F401"]);
    // Naming the deleted path explicitly is accepted too — it is exactly what
    // `git diff --name-only` just said changed.
    assert!(diff_run(root, &["--diff", "gone.py"], 0).is_empty());
}

/// The files a change did not touch are never even read — that is what keeps a
/// diff run cheap on a large repository. A file that cannot be read at all is
/// how to observe it: it fails a whole-tree scan and is invisible to a diff run
/// that does not touch it.
#[test]
fn files_the_change_did_not_touch_are_never_read() {
    let repo = git_repo();
    let root = repo.path();
    fs::write(root.join("unreadable.py"), [b'x', b' ', 0xff, b'\n']).unwrap();
    write(root, "app.py", "value = 1\n");
    commit(root, "baseline");
    write(root, "app.py", "value = 1  # noqa: E501\n");

    let output = notignored(root)
        .args(["--diff", "--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(0), "{:?}", output.status);
    let report = parse_report(&output.stdout);
    assert!(
        report["errors"].as_array().unwrap().is_empty(),
        "an untouched file was read: {report:#}"
    );
    assert_eq!(reported(&output.stdout), vec!["app.py:1 E501"]);

    // The same tree scanned whole does read it, and says so.
    let whole = notignored(root)
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(whole.status.code(), Some(2), "{:?}", whole.status);
    assert_eq!(
        parse_report(&whole.stdout)["errors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// A commit range can add a file the work tree has since dropped — reviewing
/// `HEAD~1..HEAD` while working on the next change is exactly that. The
/// suppression is in the range, but there is no file left to read: skip it
/// rather than turning a review into an unreadable-file failure.
#[test]
fn a_file_added_in_the_range_but_gone_from_the_work_tree_is_skipped() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "kept.py", "value = 1\n");
    commit(root, "baseline");
    write(root, "gone.py", "value = 2  # noqa: E501\n");
    write(root, "kept.py", "value = 1  # noqa: F401\n");
    commit(root, "add both suppressions");
    fs::remove_file(root.join("gone.py")).expect("delete the file");

    let output = notignored(root)
        .args(["--diff", "--diff-base", "HEAD~1..HEAD", "--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_report(&output.stdout);
    assert!(
        report["errors"].as_array().unwrap().is_empty(),
        "a file the work tree no longer has is not an unreadable one: {report:#}"
    );
    assert_eq!(reported(&output.stdout), vec!["kept.py:1 F401"]);
}

#[test]
fn a_file_wide_suppression_above_the_change_is_not_reported_as_new() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "vendored.py", "# ruff: noqa: E501\nvalue = 1\n");
    commit(root, "baseline");
    write(
        root,
        "vendored.py",
        "# ruff: noqa: E501\nvalue = 1\nvalue2 = 2\n",
    );

    // The added line is silenced by the file-wide directive, but the directive
    // itself is old: what the change added is a line, not a suppression.
    assert!(diff_run(root, &["--diff"], 0).is_empty());
}

#[test]
fn fail_if_found_answers_for_the_new_suppressions_only() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "import os  # noqa: F401\n");
    commit(root, "baseline");

    // A change that adds no suppression passes, even though the file has one.
    write(root, "app.py", "import os  # noqa: F401\nvalue = 1\n");
    let output = notignored(root)
        .args(["--diff", "--fail-if-found"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(0), "{:?}", output.status);

    // A change that adds one does not.
    write(
        root,
        "app.py",
        "import os  # noqa: F401\nvalue = 1  # noqa: E501\n",
    );
    let output = notignored(root)
        .args(["--diff", "--fail-if-found"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    assert!(String::from_utf8(output.stdout).unwrap().contains("E501"));
}

#[test]
fn diff_outside_a_git_repository_exits_two_with_a_way_out() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("app.py"), "value = 1  # noqa\n").unwrap();

    let output = notignored(dir.path())
        .arg("--diff")
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);
    assert!(
        output.stdout.is_empty(),
        "stdout should stay clean on error"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("not inside a git work tree"), "{stderr}");
    assert!(stderr.contains("hint:"), "{stderr}");
    assert!(stderr.contains("--diff"), "{stderr}");
}

#[test]
fn a_diff_base_the_repository_cannot_resolve_exits_two_with_a_way_out() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "value = 1  # noqa: E501\n");
    commit(root, "baseline");

    let output = notignored(root)
        .args(["--diff", "--diff-base", "no-such-branch"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("no-such-branch"), "{stderr}");
    assert!(stderr.contains("hint:"), "{stderr}");
}

#[test]
fn a_diff_base_without_diff_is_rejected_before_any_scanning() {
    let repo = git_repo();
    let output = notignored(repo.path())
        .args(["--diff-base", "main"])
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--diff"), "{stderr}");
}

#[test]
fn the_diff_json_matches_the_checked_in_golden_report() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "src/app.py", "import os  # noqa: F401\nvalue = 1\n");
    write(root, "src/vendored.py", "# ruff: noqa: E501\nvalue = 2\n");
    commit(root, "baseline");

    git(root, &["checkout", "-q", "-b", "feature"]);
    write(
        root,
        "src/app.py",
        "import os  # noqa: F401\nvalue = 1\nurl = URL  # noqa: E501  # long wrapped URL\n",
    );
    write(
        root,
        "src/new.py",
        "# ruff: noqa  # generated, do not lint\nvalue = 3\n",
    );
    commit(root, "feature change");

    let output = notignored(root)
        .args(["--diff", "--diff-base", "main", "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);

    let golden_path = repo_root().join("tests/golden/diff-report.json");
    let actual = String::from_utf8(output.stdout).unwrap();
    if std::env::var_os("NOTIGNORED_BLESS").is_some() {
        fs::write(&golden_path, &actual).expect("write the golden diff report");
    }
    let expected = fs::read_to_string(&golden_path).expect("read the golden diff report");
    assert_eq!(
        actual, expected,
        "the --diff JSON report changed. If the change is intended, re-run with \
         NOTIGNORED_BLESS=1 and bump REPORT_VERSION when the shape (not just the data) moved."
    );
}
