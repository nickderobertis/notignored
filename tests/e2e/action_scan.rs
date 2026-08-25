//! The composite action's scan step, run over a real repository.
//!
//! The script under test is lifted out of `action.yml` itself rather than
//! copied, so what runs here is what the runner runs: the base the pull request
//! forked from, the head commit its permalinks pin, the JSON report, the comment
//! body, and the `count` a calling workflow reads.
//!
//! POSIX-only, for the reason given in [`crate::action_comment`].
#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Output};

use crate::support::{commit, git, git_repo, git_stdout, repo_root, write};

/// The script of the composite step named `name`, dedented the way the runner
/// hands it to bash.
///
/// Read out of `action.yml` on purpose: a copy in this file would keep passing
/// long after the action stopped doing what it says.
fn step_script(name: &str) -> String {
    let action = std::fs::read_to_string(repo_root().join("action.yml")).expect("read action.yml");
    let mut lines = action
        .lines()
        .skip_while(|line| line.trim() != format!("- name: {name}"))
        .skip(1)
        .peekable();
    assert!(lines.peek().is_some(), "action.yml has no `{name}` step");

    let script: Vec<&str> = lines
        .by_ref()
        .skip_while(|line| line.trim() != "run: |")
        .skip(1)
        .take_while(|line| line.trim().is_empty() || line.starts_with("        "))
        .collect();
    assert!(!script.is_empty(), "the `{name}` step runs no script");
    script
        .iter()
        .map(|line| line.strip_prefix("        ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `jq` reads the event payload and the report inside the composite. Missing, the
/// journey fails with the fix rather than skipping.
fn require_jq() {
    let found = Command::new("jq").arg("--version").output();
    assert!(
        found.is_ok_and(|output| output.status.success()),
        "jq is not installed\nACTION: install jq — the composite action uses it, and every \
         GitHub-hosted runner ships it"
    );
}

/// A repository whose branch adds one suppression on top of `main`, with the
/// remote-tracking `origin/main` a real checkout would have created.
fn pull_request() -> tempfile::TempDir {
    let repo = git_repo();
    write(repo.path(), "src/app.py", "VALUE = 1\n");
    write(repo.path(), "docs/notes.md", "notes\n");
    commit(repo.path(), "base");
    git(
        repo.path(),
        &["update-ref", "refs/remotes/origin/main", "main"],
    );

    git(repo.path(), &["checkout", "-q", "-b", "feature"]);
    write(
        repo.path(),
        "src/app.py",
        "VALUE = 1\nimport os  # noqa: F401  # imported for its side effects\n",
    );
    commit(repo.path(), "add a suppression");
    repo
}

/// A repository whose branch rewrites the justification of a suppression the
/// base already carried, and adds one of its own.
///
/// The pull request the two counts have to tell apart: one number is what it
/// silenced that it did not before, the other is what it merely reworded.
fn rejustified_pull_request() -> tempfile::TempDir {
    let repo = git_repo();
    write(
        repo.path(),
        "src/app.py",
        "import os  # noqa: F401  # imported for its side effects\n",
    );
    commit(repo.path(), "base");
    git(
        repo.path(),
        &["update-ref", "refs/remotes/origin/main", "main"],
    );

    git(repo.path(), &["checkout", "-q", "-b", "feature"]);
    write(
        repo.path(),
        "src/app.py",
        "import os  # noqa: F401  # imported for the side effects of importing it\n",
    );
    commit(repo.path(), "reword the justification");
    repo
}

/// The environment one composite run sees, over `repo`.
struct Run {
    output: Output,
    outputs: String,
    report: String,
    body: String,
}

impl Run {
    fn count(&self) -> &str {
        self.output_named("count")
    }

    fn justification_edited_count(&self) -> &str {
        self.output_named("justification-edited-count")
    }

    /// One `name=value` line of the step's `$GITHUB_OUTPUT` file.
    fn output_named(&self, name: &str) -> &str {
        self.outputs
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("the step sets a {name} output:\n{}", self.outputs))
    }
}

/// Run the scan step over `repo` with `env` layered on top of a real event.
fn scan(repo: &Path, env: &[(&str, &str)]) -> Run {
    require_jq();
    let temp = repo.join(".runner");
    std::fs::create_dir_all(&temp).expect("create the runner temp directory");
    let outputs = temp.join("outputs.txt");
    std::fs::write(&outputs, "").expect("create the step output file");

    let script = temp.join("scan.sh");
    std::fs::write(&script, step_script("Scan the change")).expect("write the step script");

    let mut command = Command::new("bash");
    command
        .arg(&script)
        .current_dir(repo)
        .env("NOTIGNORED", assert_cmd::cargo::cargo_bin("notignored"))
        .env("GITHUB_REPOSITORY", "acme/widgets")
        .env("RUNNER_TEMP", &temp)
        .env("GITHUB_OUTPUT", &outputs)
        .env("DIFF_BASE", "")
        .env("SCAN_PATHS", "")
        .env("MAX_ENTRIES", "20")
        .env("GITHUB_SHA", "0000000000000000000000000000000000000000")
        .env_remove("GITHUB_BASE_REF")
        .env_remove("GITHUB_EVENT_PATH");
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command.output().expect("run the scan step");

    let read = |name: &str| std::fs::read_to_string(temp.join(name)).unwrap_or_default();
    Run {
        outputs: std::fs::read_to_string(&outputs).unwrap_or_default(),
        report: read("notignored-report.json"),
        body: read("notignored-comment.md"),
        output,
    }
}

/// The commit a pull request's comment should link to: its head, not the
/// throwaway merge commit GitHub checks out.
fn head_sha(repo: &Path) -> String {
    git_stdout(repo, &["rev-parse", "HEAD"]).trim().to_string()
}

/// An event payload naming `sha` as the pull request's head.
fn event_payload(repo: &Path, sha: &str) -> String {
    let path = repo.join(".runner/event.json");
    std::fs::create_dir_all(path.parent().expect("a runner directory")).expect("create it");
    std::fs::write(
        &path,
        format!(r#"{{"pull_request":{{"number":7,"head":{{"sha":"{sha}"}}}}}}"#),
    )
    .expect("write the event payload");
    path.to_string_lossy().into_owned()
}

#[test]
fn the_step_reports_what_the_branch_added_and_links_it_to_the_head_commit() {
    let repo = pull_request();
    let sha = head_sha(repo.path());
    let event = event_payload(repo.path(), &sha);
    let run = scan(
        repo.path(),
        &[("GITHUB_BASE_REF", "main"), ("GITHUB_EVENT_PATH", &event)],
    );
    assert!(
        run.output.status.success(),
        "{}",
        String::from_utf8_lossy(&run.output.stderr)
    );

    let report: serde_json::Value = serde_json::from_str(&run.report).expect("a JSON report");
    let ignores = report["ignores"].as_array().expect("an ignores array");
    assert_eq!(ignores.len(), 1, "{report:#}");
    assert_eq!(ignores[0]["rules"][0], "F401");
    assert_eq!(run.count(), "1");
    assert!(
        run.outputs.contains("report-path=") && run.outputs.contains("notignored-report.json"),
        "{}",
        run.outputs
    );

    assert!(
        run.body.starts_with(notignored::cli::MARKER),
        "{}",
        run.body
    );
    assert!(
        run.body.contains(&format!(
            "https://github.com/acme/widgets/blob/{sha}/src/app.py#L2"
        )),
        "the comment links somewhere other than the head commit:\n{}",
        run.body
    );
    // The base is inherited from the pull request, not guessed: `VALUE = 1` was
    // already there and carries no directive, and the untouched docs file is
    // never read at all.
    assert!(!run.body.contains("docs/notes.md"), "{}", run.body);
}

#[test]
fn an_explicit_diff_base_outranks_the_pull_requests_own() {
    let repo = pull_request();
    let run = scan(
        repo.path(),
        &[("DIFF_BASE", "main"), ("GITHUB_BASE_REF", "no-such-branch")],
    );
    assert!(
        run.output.status.success(),
        "{}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(run.count(), "1");
}

#[test]
fn paths_narrow_the_scan_the_way_they_narrow_the_command() {
    let repo = pull_request();
    let run = scan(
        repo.path(),
        &[("GITHUB_BASE_REF", "main"), ("SCAN_PATHS", "docs src")],
    );
    assert!(run.output.status.success());
    assert_eq!(run.count(), "1");

    let narrowed = scan(
        repo.path(),
        &[("GITHUB_BASE_REF", "main"), ("SCAN_PATHS", "docs")],
    );
    assert!(
        narrowed.output.status.success(),
        "{}",
        String::from_utf8_lossy(&narrowed.output.stderr)
    );
    assert_eq!(narrowed.count(), "0");
    assert!(
        narrowed
            .body
            .contains("No lint or type-check suppressions found."),
        "{}",
        narrowed.body
    );
}

/// Without a pull request to inherit a base from, the step has to say what to
/// set rather than scan the wrong thing.
#[test]
fn a_run_with_no_base_at_all_fails_with_the_input_to_set() {
    let repo = pull_request();
    let run = scan(repo.path(), &[]);
    assert!(!run.output.status.success(), "an empty base was accepted");
    let stderr =
        String::from_utf8_lossy(&run.output.stdout) + String::from_utf8_lossy(&run.output.stderr);
    assert!(stderr.contains("::error::"), "{stderr}");
    assert!(stderr.contains("diff-base"), "{stderr}");
}

/// The failure a shallow checkout produces, and the one line that fixes it.
#[test]
fn a_base_the_checkout_never_fetched_names_the_fix() {
    let repo = pull_request();
    let run = scan(repo.path(), &[("GITHUB_BASE_REF", "release-9")]);
    assert!(!run.output.status.success(), "a missing base was accepted");
    let reported =
        String::from_utf8_lossy(&run.output.stdout) + String::from_utf8_lossy(&run.output.stderr);
    assert!(reported.contains("origin/release-9"), "{reported}");
    assert!(reported.contains("fetch-depth: 0"), "{reported}");
}

/// The two numbers a workflow reads, over a pull request that did one of each.
///
/// `count` keeps meaning additions — a build gating on it must not start
/// failing the day somebody rewords a reason — so the rewritten justification
/// is counted beside it, not into it, and the step says both out loud.
#[test]
fn the_step_counts_additions_and_rewritten_justifications_apart() {
    let repo = rejustified_pull_request();
    write(
        repo.path(),
        "src/extra.py",
        "import sys  # noqa: F401  # re-exported for callers\n",
    );
    commit(repo.path(), "add one as well");

    let run = scan(repo.path(), &[("GITHUB_BASE_REF", "main")]);
    assert!(
        run.output.status.success(),
        "{}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(run.count(), "1");
    assert_eq!(run.justification_edited_count(), "1");

    let log = String::from_utf8_lossy(&run.output.stdout);
    assert!(
        log.contains("1 suppression(s) added, 1 justification(s) edited"),
        "the step's log line names one number or the wrong words: {log}"
    );
    assert!(
        run.body
            .contains("### notignored: 1 suppression added, 1 justification edited"),
        "{}",
        run.body
    );
}

/// A pull request that rewrote a justification and added nothing: the number a
/// build gates on is zero, and the other number is what happened.
#[test]
fn a_branch_that_only_rewrote_a_justification_adds_nothing() {
    let repo = rejustified_pull_request();
    let run = scan(repo.path(), &[("GITHUB_BASE_REF", "main")]);
    assert!(
        run.output.status.success(),
        "{}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(run.count(), "0");
    assert_eq!(run.justification_edited_count(), "1");
    assert!(
        run.body.contains("### notignored: 1 justification edited"),
        "{}",
        run.body
    );
}

/// The two commands the scan step counts a report with, lifted out of
/// `action.yml` and run over `report` by the real bash and the real jq the
/// runner uses.
///
/// Read out of the step for the same reason the whole step is read out of it
/// above: a copy here would keep agreeing with itself long after the action
/// stopped counting this way. Returns `(count, justification-edited-count)`.
fn counts_of(report: &Path) -> (String, String) {
    require_jq();
    let step = step_script("Scan the change");
    let counting: Vec<&str> = step
        .lines()
        .filter(|line| line.starts_with("count=") || line.starts_with("edited="))
        .collect();
    assert_eq!(
        counting.len(),
        2,
        "the scan step no longer counts its report in two assignments:\n{step}"
    );

    let script = format!(
        "set -euo pipefail\nreport=\"$1\"\n{}\nprintf '%s\\n%s\\n' \"$count\" \"$edited\"\n",
        counting.join("\n")
    );
    let output = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .arg("count-the-report")
        .arg(report)
        .output()
        .expect("run the step's counting commands");
    assert!(
        output.status.success(),
        "counting failed ({:?}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let printed = String::from_utf8(output.stdout).expect("UTF-8 counts");
    let mut lines = printed.lines().map(str::to_string);
    (
        lines.next().expect("a count"),
        lines.next().expect("a rewritten-justification count"),
    )
}

/// Every record of a report from a `notignored` that never classified counts as
/// added.
///
/// The action installs `latest` by default but can be pinned to any release, so
/// the step has to stay correct against a build from before the word existed.
/// The report here is this repository's own committed golden — a real scan whose
/// records carry no `change` at all, which is exactly what those builds wrote.
/// Reading the absence as "not added" would silently zero the number those users
/// have been reading all along.
#[test]
fn a_report_from_a_notignored_that_never_classified_counts_every_record_as_added() {
    let golden = repo_root().join("tests/golden/report.json");
    let text = std::fs::read_to_string(&golden).expect("read the golden report");
    assert!(
        !text.contains("\"change\""),
        "the golden report is classified; it can no longer stand in for an older build's"
    );
    let records = serde_json::from_str::<serde_json::Value>(&text).expect("a JSON report")
        ["ignores"]
        .as_array()
        .expect("an ignores array")
        .len();
    assert!(records > 0, "the golden report holds nothing to count");

    assert_eq!(counts_of(&golden), (records.to_string(), "0".to_string()));
}

/// A change word this version has never heard of is counted, not dropped.
///
/// The action can also install a *newer* release than the workflow was written
/// against, so the step has to survive a vocabulary that grew. The two counts
/// partition the report between them, so such a record is reported as an
/// addition — the conservative reading — rather than falling out of both
/// numbers and understating what the pull request did.
#[test]
fn a_change_word_this_version_never_heard_of_is_still_counted() {
    let classified = repo_root().join("tests/golden/diff-report.json");
    let mut report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&classified).expect("read the classified golden report"),
    )
    .expect("a JSON report");
    let records = report["ignores"]
        .as_array()
        .expect("an ignores array")
        .len();
    assert!(records > 0, "the golden diff report holds nothing to count");
    report["ignores"][0]["change"] = serde_json::Value::String("rules-widened".into());

    let widened = tempfile::NamedTempFile::new().expect("a report file");
    std::fs::write(
        widened.path(),
        serde_json::to_string_pretty(&report).expect("serialize the report"),
    )
    .expect("write the report");

    assert_eq!(
        counts_of(widened.path()),
        (records.to_string(), "0".to_string()),
        "a record carrying an unknown change word fell out of both counts"
    );
}
