//! End-to-end journeys through `--diff`, over real git repositories.
//!
//! Every repository here is built with the real `git` binary and read by the
//! compiled `notignored` — nothing about the comparison is simulated, because
//! the promise `--diff-base` makes is precisely "the same set of changes git
//! (and therefore a pull request) would show you".

use std::fs;

use crate::support::{
    commit, git, git_repo, git_stdout, notignored, parse_report, repo_root, write,
};

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

/// The 1-based lines `git diff` itself shows as added for `path`.
///
/// Read straight from git's hunk headers, independently of the binary under
/// test, so a journey that hinges on *which* lines changed cannot quietly drift
/// into proving nothing.
fn git_added_lines(root: &std::path::Path, path: &str) -> Vec<u32> {
    let patch = git_stdout(root, &["diff", "--unified=0", "--no-color", "--", path]);
    let mut added = Vec::new();
    for header in patch.lines().filter_map(|line| line.strip_prefix("@@ ")) {
        let new_side = header
            .split_whitespace()
            .find_map(|field| field.strip_prefix('+'))
            .expect("a hunk header names its new side");
        let (first, count) = new_side.split_once(',').unwrap_or((new_side, "1"));
        let first: u32 = first.parse().expect("a hunk header line number");
        let count: u32 = count.parse().expect("a hunk header line count");
        added.extend(first..first + count);
    }
    added
}

/// Run the binary in `dir` with `args` and `--format json`, asserting the exit
/// code and returning the suppressions it reported.
fn run_json(dir: &std::path::Path, args: &[&str], expected_code: i32) -> Vec<String> {
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

    assert_eq!(run_json(root, &["--diff"], 0), vec!["app.py:3 E501"]);
    // Without --diff the whole inventory is still reported: the flag narrows the
    // report, it does not change how anything is parsed.
    assert_eq!(
        run_json(root, &[], 0),
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
        run_json(root, &["--diff", "--diff-base", "main"], 0),
        vec!["app.py:2 F401"]
    );

    // An explicit range is the caller's own choice of semantics: git's raw
    // two-dot comparison against the base *tip*, base drift and all.
    assert_eq!(
        run_json(root, &["--diff", "--diff-base", "main..HEAD"], 0),
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
    assert!(run_json(root, &["--diff"], 0).is_empty());

    // The same move, plus a line: only the added line is new.
    write(
        root,
        "new.py",
        &format!("{original}url = URL  # noqa: E501\n"),
    );
    assert_eq!(run_json(root, &["--diff"], 0), vec!["new.py:7 E501"]);
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
        run_json(root, &["--diff"], 0),
        vec!["staged.py:1 E501", "unstaged.py:1 F401"]
    );

    // Once committed there is nothing left to compare against HEAD...
    commit(root, "both suppressions");
    assert!(run_json(root, &["--diff"], 0).is_empty());
    // ...but the commit itself still is a change, against the commit before it.
    assert_eq!(
        run_json(root, &["--diff", "--diff-base", "HEAD~1"], 0),
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
        run_json(root, &["--diff", "src"], 0),
        vec!["src/app.py:1 E501"]
    );
    assert_eq!(
        run_json(root, &["--diff", "src/app.py", "tools"], 0),
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
    assert!(run_json(root, &["--diff", "gone.py"], 0).is_empty());
}

/// The bytes of a file name that no `String` can hold.
#[cfg(unix)]
const UNDECODABLE_NAME: &[u8] = b"caf\xe9.py";

/// The suppression such a file carries — reportable only if its path were
/// somehow decodable, so seeing it would mean the error path was skipped.
#[cfg(unix)]
const UNDECODABLE_CONTENT: &str = "import os  # noqa: F401  # its name is not UTF-8\n";

/// Assert a run reported the undecodable path as an error rather than dropping
/// it, and still reported `also_reported` — the rest of the same change.
#[cfg(unix)]
fn assert_undecodable_path_reported(output: &std::process::Output, also_reported: &[&str]) {
    assert_eq!(
        output.status.code(),
        Some(2),
        "a file that could not be scanned is not a clean run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The rest of the change is still reported: one unnameable file does not
    // take the review down with it.
    assert_eq!(reported(&output.stdout), also_reported);
    let report = parse_report(&output.stdout);
    let errors = report["errors"].as_array().expect("an errors array");
    assert_eq!(errors.len(), 1, "{report:#}");
    assert_eq!(errors[0]["path"], "caf\u{fffd}.py");
    assert!(
        errors[0]["message"]
            .as_str()
            .expect("a message")
            .contains("UTF-8"),
        "{report:#}"
    );
    // And a person watching the terminal is told, not just the JSON consumer.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("caf\u{fffd}.py"), "{stderr}");
}

/// Run the binary's JSON diff in `root`.
#[cfg(unix)]
fn diff_json(root: &std::path::Path) -> std::process::Output {
    notignored(root)
        .args(["--diff", "--format", "json"])
        .output()
        .expect("run notignored")
}

/// A file the change touched under a name that is not valid UTF-8 becomes a
/// report error, not a file that quietly vanishes from the review.
///
/// Git speaks paths as bytes and the report contract speaks `String`, so such a
/// name has no faithful spelling in a report. Decoded lossily it names a file
/// that does not exist: handed back to git as a pathspec it matches nothing, and
/// a change carrying a fresh suppression reads as clean.
///
/// Whether such a file can exist at all is a property of the **filesystem**, not
/// of the operating system: Linux passes the name through as bytes, while APFS
/// and HFS+ reject it at `write` with `EILSEQ`, so one `#[cfg(unix)]` build
/// cannot assume it. This journey therefore asks the filesystem rather than the
/// target. Where the name is permitted it drives the whole thing for real
/// against a committed baseline; where it is not, it proves the refusal is about
/// *these bytes* — the same content under a decodable name in the same directory
/// writes fine — and that the review survives it.
///
/// The behaviour itself is not left to one platform. The decoding is covered on
/// every target by
/// `src/diff.rs::a_path_that_is_not_utf8_is_set_aside_rather_than_guessed_at`,
/// and the whole journey — real git, real binary, same report error — is proven
/// wherever a file cannot carry the name by
/// [`an_undecodable_path_staged_in_the_index_is_reported_rather_than_dropped`],
/// which reaches it through the index instead of the work tree.
#[cfg(unix)]
#[test]
fn a_changed_path_that_is_not_utf8_is_reported_rather_than_dropped() {
    use std::os::unix::ffi::OsStrExt;

    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "value = 1\n");
    commit(root, "baseline");

    let undecodable = root.join(std::ffi::OsStr::from_bytes(UNDECODABLE_NAME));
    match fs::write(&undecodable, UNDECODABLE_CONTENT) {
        Ok(()) => {
            write(root, "app.py", "value = 1\nurl = URL  # noqa: E501\n");
            // `git diff` only knows about tracked files, and this one is new.
            git(root, &["add", "-A"]);
            assert_undecodable_path_reported(&diff_json(root), &["app.py:2 E501"]);
        }
        Err(refused) => {
            // The *name* was refused, not the write: identical bytes under a
            // decodable name in the same directory land without complaint, so
            // this is the filesystem's rule about file names rather than a
            // permission, space, or path-length problem.
            fs::write(root.join("cafe.py"), UNDECODABLE_CONTENT)
                .unwrap_or_else(|error| panic!("this directory is not writable at all: {error}"));
            assert!(
                !undecodable.exists(),
                "the filesystem refused the name with {refused}, yet something is there"
            );

            // Recovery: the refusal leaves an ordinary repository behind, so the
            // review still completes and still reports every suppression that is
            // reachable on this platform.
            write(root, "app.py", "value = 1\nurl = URL  # noqa: E501\n");
            git(root, &["add", "-A"]);
            let output = diff_json(root);
            assert_eq!(
                output.status.code(),
                Some(0),
                "a name this filesystem cannot hold is not an error to report: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                reported(&output.stdout),
                vec!["app.py:2 E501", "cafe.py:1 F401"]
            );
            assert!(
                parse_report(&output.stdout)["errors"]
                    .as_array()
                    .expect("an errors array")
                    .is_empty(),
                "nothing failed to scan: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}

/// The same contract, reached through the index instead of the work tree.
///
/// A path git reports is bytes whether or not the local filesystem could ever
/// hold it — which is exactly the state a macOS clone of a repository containing
/// such a path ends up in: the index entry is there, the checkout of that one
/// file failed, and a review that silently dropped it would call the change
/// clean. Staging the entry with git's own plumbing reproduces that without
/// asking the filesystem for anything, so this journey proves the behaviour on
/// every Unix — including the ones where
/// [`a_changed_path_that_is_not_utf8_is_reported_rather_than_dropped`] cannot
/// build its fixture. With no commit yet, `--diff` compares the index against
/// the empty tree, so both staged paths are the change.
#[cfg(unix)]
#[test]
fn an_undecodable_path_staged_in_the_index_is_reported_rather_than_dropped() {
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::OsStringExt;

    use crate::support::git_os;

    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "url = URL  # noqa: E501\n");

    // The blob is hashed from a decodable name that is then removed, so only the
    // index entry below carries the undecodable one.
    write(root, "staged-blob.py", UNDECODABLE_CONTENT);
    let blob = git_stdout(root, &["hash-object", "-w", "staged-blob.py"])
        .trim()
        .to_string();
    fs::remove_file(root.join("staged-blob.py")).expect("remove the hashed source");

    let mut cacheinfo = format!("100644,{blob},").into_bytes();
    cacheinfo.extend_from_slice(UNDECODABLE_NAME);
    let cacheinfo = OsString::from_vec(cacheinfo);
    git_os(
        root,
        &[
            OsStr::new("update-index"),
            OsStr::new("--add"),
            OsStr::new("--cacheinfo"),
            &cacheinfo,
        ],
    );
    git(root, &["add", "app.py"]);
    // Read the index back: git holds those bytes verbatim, so the journey really
    // is driving the state it claims to.
    let staged = git_stdout(root, &["ls-files", "-z"]);
    assert!(
        staged.contains(&String::from_utf8_lossy(UNDECODABLE_NAME).into_owned()),
        "the undecodable path is not staged: {staged:?}"
    );

    assert_undecodable_path_reported(&diff_json(root), &["app.py:1 E501"]);
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

/// A directive written across lines counts as new when the change added **any**
/// of the lines it occupies, not only its first.
///
/// A reviewer editing the lint list inside an existing `#[allow(…)]` is adding a
/// suppression — the directive now silences something it did not silence before
/// — even though the `#[allow(` line itself is untouched. The record here is a
/// real one: `end_line > line` comes from the Rust parser reading a real
/// attribute, and rustc agrees it suppresses (see `rust_parity.rs`).
#[test]
fn a_directive_spanning_lines_is_new_when_the_change_added_part_of_it() {
    let repo = git_repo();
    let root = repo.path();
    // A second multi-line directive further down, which the change never touches.
    let untouched = concat!(
        "\n",
        "#[allow(\n",
        "    unused_variables,\n",
        ")]\n",
        "fn other(value: u32) {}\n",
    );
    write(
        root,
        "src/lib.rs",
        &format!("#[allow(\n    dead_code,\n)]\nfn helper() {{}}\n{untouched}"),
    );
    commit(root, "baseline");

    // Exactly one line added, *inside* the first directive's span: neither its
    // opening `#[allow(` nor its closing `)]` is touched, and the trailing comma
    // was already there so no other line moves.
    write(
        root,
        "src/lib.rs",
        &format!(
            "#[allow(\n    dead_code,\n    unused_imports,\n)]\nfn helper() {{}}\n{untouched}"
        ),
    );

    let output = notignored(root)
        .args(["--diff", "--format", "json"])
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
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(
        ignores.len(),
        1,
        "only the directive the change edited is new: {report:#}"
    );
    let directive = &ignores[0];
    assert_eq!(directive["path"], "src/lib.rs");
    assert_eq!(
        directive["rules"],
        serde_json::json!(["dead_code", "unused_imports"])
    );
    // The record really does span lines, and the change added neither its first
    // nor its last one — only line 3, in the middle.
    assert_eq!(directive["line"], 1);
    assert_eq!(directive["end_line"], 4);
    assert_eq!(
        git_added_lines(root, "src/lib.rs"),
        vec![3],
        "the scenario hinges on the added line falling inside, not at, the span"
    );
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
    assert!(run_json(root, &["--diff"], 0).is_empty());
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

/// A repository with nothing committed yet has no HEAD to compare against, so
/// the comparison falls back to the index — a staged file is still a change
/// someone is about to review, and must not fatal.
#[test]
fn a_repository_with_no_commit_yet_diffs_what_is_staged() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "import os  # noqa: F401\n");
    git(root, &["add", "app.py"]);

    assert_eq!(run_json(root, &["--diff"], 0), vec!["app.py:1 F401"]);
}

/// A base with no common ancestor cannot have a merge base, so the comparison
/// falls back to git's plain two-dot diff: an unrelated base is still diffable
/// rather than an error.
#[test]
fn a_base_sharing_no_history_is_still_compared() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "value = 1\n");
    commit(root, "main baseline");

    git(root, &["checkout", "-q", "--orphan", "unrelated"]);
    write(root, "app.py", "value = 1  # noqa: E501\n");
    commit(root, "unrelated baseline");

    assert_eq!(
        run_json(root, &["--diff", "--diff-base", "main"], 0),
        vec!["app.py:1 E501"]
    );
}

/// `--diff` needs git, and the way out has to be on the terminal: a missing git
/// must not read as "this change added no suppressions".
#[cfg(unix)]
#[test]
fn diff_without_git_on_the_path_exits_two_with_a_way_out() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "value = 1  # noqa: E501\n");
    commit(root, "baseline");
    let empty = tempfile::tempdir().unwrap();

    let output = notignored(root)
        .arg("--diff")
        .env("PATH", empty.path())
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);
    assert!(
        output.stdout.is_empty(),
        "stdout should stay clean on error"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("git not found"), "{stderr}");
    assert!(stderr.contains("install git"), "{stderr}");
}

/// A git that is on PATH but cannot be started is a different fault with a
/// different fix, and says so.
#[cfg(unix)]
#[test]
fn diff_with_a_git_that_cannot_be_started_exits_two_with_a_way_out() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "value = 1  # noqa: E501\n");
    commit(root, "baseline");
    // A `git` that is found but not executable.
    let bin = tempfile::tempdir().unwrap();
    fs::write(bin.path().join("git"), "#!/bin/sh\n").unwrap();

    let output = notignored(root)
        .arg("--diff")
        .env("PATH", bin.path())
        .output()
        .expect("run notignored");
    assert_eq!(output.status.code(), Some(2), "{:?}", output.status);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("cannot run git"), "{stderr}");
    assert!(stderr.contains("executable"), "{stderr}");
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

/// `--diff` from inside a git hook reads the repository it was pointed at, not
/// the one the hook fired in.
///
/// Every hook runs with `GIT_DIR` exported, and `pre-push` adds `GIT_INDEX_FILE`;
/// neither is outranked by the `-C` this tool passes. Inherited, they answered
/// for the *hook's* repository — so a pre-push gate or a CI step git itself
/// invoked would compare a tree nobody asked about and report its suppressions
/// as this change's. The variables are set on the child alone, the way git sets
/// them, so nothing here depends on the test process's own environment.
#[test]
fn a_diff_run_from_inside_a_git_hook_still_reads_the_repository_it_was_given() {
    let hooks_repo = git_repo();
    let elsewhere = hooks_repo.path();
    write(elsewhere, "elsewhere.py", "other = 1  # noqa: E501\n");
    commit(elsewhere, "a repository this run must not read");

    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "value = 1\n");
    commit(root, "baseline");
    write(root, "app.py", "value = 1\nimport os  # noqa: F401\n");

    let output = notignored(root)
        .args(["--diff", "--format", "json"])
        // Exactly what git exports to a `pre-push` hook running in `elsewhere`.
        .env("GIT_DIR", elsewhere.join(".git"))
        .env("GIT_INDEX_FILE", elsewhere.join(".git/index"))
        .env("GIT_PREFIX", "")
        .output()
        .expect("run notignored");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(reported(&output.stdout), vec!["app.py:2 F401"]);
}

/// Every suppression a JSON run reported, as `path:line change`.
///
/// The classification is what the review comment counts on, so it is read back
/// off the binary's own JSON rather than from any function inside it.
fn classified(stdout: &[u8]) -> Vec<String> {
    parse_report(stdout)["ignores"]
        .as_array()
        .expect("ignores is an array")
        .iter()
        .map(|directive| {
            format!(
                "{}:{} {}",
                directive["path"].as_str().expect("path is a string"),
                directive["line"].as_u64().expect("line is a number"),
                directive["change"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a --diff record carries change: {directive:#}")),
            )
        })
        .collect()
}

/// Run the binary's JSON report in `dir`, asserting it completed cleanly.
fn json_report(dir: &std::path::Path, args: &[&str]) -> Vec<u8> {
    let output = notignored(dir)
        .args(args)
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// A wrapped llmlint justification: the directive on one line, the rest of the
/// sentence on the next. Only the continuation moves in the journey below.
fn wrapped(tail: &str) -> String {
    format!("# llmlint: ignore[dead_code] the first half of the justification,\n# {tail}\nx = 1\n")
}

/// The word `--diff` puts on each suppression, over a change carrying one of
/// every case the contract names.
///
/// A justification edit reported as a new suppression is the failure this
/// exists to prevent: the reviewer reads "2 suppressions" on a pull request
/// that added none, learns the number lies, and stops trusting the comment.
#[test]
fn a_diff_says_of_each_suppression_whether_it_was_added_or_only_rejustified() {
    let repo = git_repo();
    let root = repo.path();
    write(
        root,
        "reworded.py",
        "x = 1  # noqa: E501  # the old reason\n",
    );
    write(root, "wrapped.py", &wrapped("and the old second half"));
    write(root, "gained.py", "x = 1  # noqa: E501\n");
    write(
        root,
        "lost.py",
        "x = 1  # noqa: E501  # a reason on its way out\n",
    );
    write(
        root,
        "rules.py",
        "x = 1  # noqa: E501  # an unchanged reason\n",
    );
    write(
        root,
        "scope.py",
        "x = 1  # noqa: E501  # an unchanged reason\n",
    );
    write(root, "grown.py", "x = 1\n");
    commit(root, "baseline");

    git(root, &["checkout", "-q", "-b", "feature"]);
    // The justification, and only the justification: reworded in place, wrapped
    // onto a second line, written where there was none, and taken away.
    write(
        root,
        "reworded.py",
        "x = 1  # noqa: E501  # the new reason\n",
    );
    write(root, "wrapped.py", &wrapped("and the new second half"));
    write(root, "gained.py", "x = 1  # noqa: E501  # now justified\n");
    write(root, "lost.py", "x = 1  # noqa: E501\n");
    // What the suppression *silences*: a rule added to the list, and a line
    // exemption widened to the whole file. Each now silences something its base
    // version did not, so each is an addition however unchanged its words are.
    write(
        root,
        "rules.py",
        "x = 1  # noqa: E501,F401  # an unchanged reason\n",
    );
    write(
        root,
        "scope.py",
        "# ruff: noqa: E501  # an unchanged reason\nx = 1\n",
    );
    // A suppression written into a file that had none, and one in a file the
    // change created.
    write(
        root,
        "grown.py",
        "x = 1\ny = 2  # noqa: F401  # newly silenced\n",
    );
    write(root, "created.py", "z = 3  # noqa: E501  # brand new\n");
    commit(root, "the change under review");

    let stdout = json_report(root, &["--diff", "--diff-base", "main"]);
    assert_eq!(
        classified(&stdout),
        vec![
            "created.py:1 added",
            "gained.py:1 justification-edited",
            "grown.py:2 added",
            "lost.py:1 justification-edited",
            "reworded.py:1 justification-edited",
            "rules.py:1 added",
            "scope.py:1 added",
            "wrapped.py:1 justification-edited",
        ]
    );

    // The scenario hinges on the wrapped reason's *continuation* being the only
    // line that moved: its directive opens on line 1, which the change never
    // touched, so the pairing cannot rest on the hunk containing the directive.
    let patch = git_stdout(
        root,
        &[
            "diff",
            "--unified=0",
            "--no-color",
            "main...HEAD",
            "--",
            "wrapped.py",
        ],
    );
    assert!(
        patch.contains("@@ -2 +2 @@"),
        "only the continuation line moved: {patch}"
    );
    let wrapped_record = parse_report(&stdout)["ignores"]
        .as_array()
        .unwrap()
        .iter()
        .find(|directive| directive["path"] == "wrapped.py")
        .expect("the wrapped directive is reported")
        .clone();
    assert_eq!(wrapped_record["line"], 1);
    assert_eq!(wrapped_record["end_line"], 2);
}

/// Classification labels; it never changes what is reported.
///
/// The same change, read by the same binary, with the word stripped back off:
/// the paths, lines and rules are exactly what `--diff` has always answered.
#[test]
fn classifying_reports_the_same_suppressions_in_the_same_order() {
    let repo = git_repo();
    let root = repo.path();
    write(
        root,
        "app.py",
        "import os  # noqa: F401  # the old reason\n",
    );
    commit(root, "baseline");
    write(
        root,
        "app.py",
        "import os  # noqa: F401  # the new reason\nurl = URL  # noqa: E501\n",
    );

    let stdout = json_report(root, &["--diff"]);
    assert_eq!(
        reported(&stdout),
        vec!["app.py:1 F401", "app.py:2 E501"],
        "the selection --diff has always made"
    );
    assert_eq!(
        classified(&stdout),
        vec!["app.py:1 justification-edited", "app.py:2 added"]
    );
}

/// A run without `--diff` has no base, so it classifies nothing — and says so by
/// leaving the field out rather than writing a third value.
#[test]
fn a_whole_tree_scan_leaves_change_off_every_record() {
    let repo = git_repo();
    let root = repo.path();
    write(root, "app.py", "import os  # noqa: F401  # a reason\n");
    commit(root, "baseline");
    write(
        root,
        "app.py",
        "import os  # noqa: F401  # a different reason\n",
    );

    let report = parse_report(&json_report(root, &[]));
    let ignores = report["ignores"].as_array().expect("an ignores array");
    assert_eq!(ignores.len(), 1, "{report:#}");
    assert!(
        ignores[0].get("change").is_none(),
        "an unclassified record must omit change entirely: {report:#}"
    );
}

/// Where there is nothing to compare against, everything is an addition — and
/// nothing fails.
///
/// Three ways to have no counterpart: a repository with no commit yet, which
/// `--diff` already compares to the empty tree; a file the change created; and a
/// file whose previous contents this build cannot read as text. The last one is
/// the one that must not fail the run: a pre-image it cannot parse is a file it
/// knows nothing about, and refusing to answer would turn a reviewable change
/// into no review at all.
#[test]
fn a_change_with_no_counterpart_to_compare_against_is_all_additions() {
    let unborn = git_repo();
    let root = unborn.path();
    write(
        root,
        "app.py",
        "import os  # noqa: F401  # staged, never committed\n",
    );
    git(root, &["add", "app.py"]);
    assert_eq!(
        classified(&json_report(root, &["--diff"])),
        vec!["app.py:1 added"]
    );

    let repo = git_repo();
    let root = repo.path();
    // Committed as bytes no build can read as source, then rewritten as source.
    fs::write(root.join("was_binary.py"), [b'x', b' ', 0xff, 0xfe, b'\n']).unwrap();
    commit(root, "baseline");
    write(
        root,
        "was_binary.py",
        "x = 1  # noqa: E501  # readable at last\n",
    );
    write(root, "created.py", "y = 2  # noqa: F401  # a new file\n");
    git(root, &["add", "-A"]);

    let output = notignored(root)
        .args(["--diff", "--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(
        output.status.code(),
        Some(0),
        "an unreadable pre-image is not a failed review: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        parse_report(&output.stdout)["errors"]
            .as_array()
            .expect("an errors array")
            .is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        classified(&output.stdout),
        vec!["created.py:1 added", "was_binary.py:1 added"]
    );
}
