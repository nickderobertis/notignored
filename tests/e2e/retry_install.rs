//! `scripts/retry-install.sh`: the bounded wait every post-publish install goes
//! through.
//!
//! A publish is not one event. PyPI's JSON API answers for a new version while
//! `pip install` still resolves through a simple index that converges later and
//! per CDN edge — which is how releases v0.1.4, v0.1.5 and v0.1.6 each went red
//! in a verify leg *after* a wait step had printed the version as available. The
//! fix makes the install itself the probe, and it is only worth anything if
//! three properties hold, none of them visible from reading the script:
//!
//!   * a command that works first time costs nothing and says one line — a
//!     retry that slept before its first attempt would add minutes to every
//!     green release;
//!   * a command that fails and then succeeds is *retried*, not reported;
//!   * a command that never succeeds gives up inside its budget, and what it
//!     prints is the tool's own error, because that is the only thing that
//!     tells a human whether the registry was slow or the publish was broken.
//!
//! The registry is substituted and nothing else: a stand-in command plays the
//! part `pip install` plays on a runner, so the journeys stay offline and take
//! seconds. It is spelled as the real errors are — the pip message from the red
//! releases — so a failing run here reads like the one it is standing in for.
//!
//! Unix only: the release jobs run it under `shell: bash`, and these journeys
//! write the stand-in as a shell script.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::support::{bash_program, repo_root};

/// The pip error the three red releases actually printed, both lines of it.
///
/// Two lines because only the first carries the detail that decides what went
/// wrong — *which* versions the index was serving instead. A failure that showed
/// only the last line of the output would look complete and say nothing.
const PIP_ERROR: &str = "ERROR: Could not find a version that satisfies the requirement \
     notignored-cli==0.1.6 (from versions: 0.1.4, 0.1.5)";
const PIP_ERROR_TAIL: &str = "ERROR: No matching distribution found for notignored-cli==0.1.6";

/// A stand-in for the install command, in a fresh directory.
///
/// It counts its own invocations in a file beside it, so a journey can say "fail
/// until the third attempt" the way a CDN edge does — and can then read back how
/// many times the script actually ran it.
struct FakeInstall {
    dir: tempfile::TempDir,
}

impl FakeInstall {
    /// A command that fails until `succeed_on`, then succeeds. `succeed_on: 0`
    /// never succeeds, which is the publish that never happened.
    fn new(succeed_on: u32) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let script = format!(
            r#"#!/usr/bin/env bash
set -eu
count_file="{count}"
attempts="$(cat "$count_file" 2>/dev/null || echo 0)"
attempts=$((attempts + 1))
echo "$attempts" > "$count_file"
echo "Looking in indexes: https://pypi.org/simple"
if [ "{succeed_on}" -eq 0 ] || [ "$attempts" -lt "{succeed_on}" ]; then
  echo "{error}" >&2
  echo "{error_tail}" >&2
  exit 1
fi
echo "Successfully installed notignored-cli-0.1.6"
"#,
            count = Self::counter_path(dir.path()).display(),
            succeed_on = succeed_on,
            error = PIP_ERROR,
            error_tail = PIP_ERROR_TAIL,
        );
        let path = dir.path().join("install.sh");
        std::fs::write(&path, script).expect("write the stand-in install");
        Self { dir }
    }

    fn counter_path(dir: &Path) -> PathBuf {
        dir.join("attempts")
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("install.sh")
    }

    /// How many times the retry loop ran it.
    fn attempts(&self) -> u32 {
        std::fs::read_to_string(Self::counter_path(self.dir.path()))
            .map(|text| text.trim().parse().expect("a count"))
            .unwrap_or(0)
    }
}

/// Run the real script over the stand-in, with the delays wound down so a
/// journey takes seconds rather than the ten minutes a release is given.
fn retry(install: &FakeInstall, extra: &[&str]) -> Output {
    let script = repo_root().join("scripts/retry-install.sh");
    let mut command = Command::new(bash_program());
    command.arg(&script);
    command.args(extra);
    command.args([
        "--label",
        "PyPI notignored-cli 0.1.6 on Linux/X64",
        "--action",
        "check https://pypi.org/project/notignored-cli/",
        "--",
        "bash",
    ]);
    command.arg(install.path());
    command.output().expect("run scripts/retry-install.sh")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The common case — the index already serves it — costs one attempt and no
/// sleep, and says so in one line.
#[test]
fn an_install_that_works_first_time_runs_once_and_reports_one_line() {
    let install = FakeInstall::new(1);
    let output = retry(&install, &["--budget", "30", "--first-delay", "1"]);

    assert!(
        output.status.success(),
        "a working install failed: {}",
        stderr(&output)
    );
    assert_eq!(
        install.attempts(),
        1,
        "an install that succeeded was run more than once"
    );
    let reported = stdout(&output);
    assert_eq!(
        reported.lines().count(),
        1,
        "a green install is one line, got:\n{reported}"
    );
    assert!(
        reported.contains("installed on attempt 1"),
        "the green line does not name the attempt: {reported}"
    );
    // The install's own chatter stays out of a green run.
    assert!(
        !reported.contains("Looking in indexes"),
        "a green run printed the install's output: {reported}"
    );
}

/// The case the fix exists for: the index answers on a later attempt, and the
/// job goes green instead of red.
#[test]
fn an_install_the_index_serves_later_is_retried_until_it_succeeds() {
    let install = FakeInstall::new(3);
    let output = retry(&install, &["--budget", "30", "--first-delay", "1"]);

    assert!(
        output.status.success(),
        "the install that succeeds on the third attempt was not retried into success: {}",
        stderr(&output)
    );
    assert_eq!(
        install.attempts(),
        3,
        "the loop did not run the install until it succeeded"
    );
    assert!(
        stdout(&output).contains("installed on attempt 3"),
        "the green line does not name the attempt it took: {}",
        stdout(&output)
    );
    // The attempts that failed are visible, so a job that took four minutes to
    // install explains itself.
    let progress = stderr(&output);
    assert!(
        progress.contains("attempt 1 failed") && progress.contains("attempt 2 failed"),
        "the failed attempts left no trace: {progress}"
    );
}

/// A publish that never propagated — or never happened — fails inside the
/// budget, with the installer's own words rather than this script's guess.
#[test]
fn an_install_that_never_resolves_gives_up_inside_its_budget_showing_the_last_error() {
    let install = FakeInstall::new(0);
    let started = std::time::Instant::now();
    let output = retry(&install, &["--budget", "6", "--first-delay", "1"]);
    let elapsed = started.elapsed();

    assert!(
        !output.status.success(),
        "an install that never resolved was reported as a success"
    );
    assert!(
        install.attempts() > 1,
        "the loop gave up after {} attempt(s) instead of retrying",
        install.attempts()
    );
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "a 6-second budget took {elapsed:?}; the loop is not bounded by it"
    );
    let reported = stderr(&output);
    // The whole of the last attempt, not a one-line summary of it: the line that
    // says which versions the index *was* serving is the one that separates "the
    // CDN was slow" from "the publish never happened".
    assert!(
        reported.contains(PIP_ERROR) && reported.contains(PIP_ERROR_TAIL),
        "the installer's own error is not in the failure in full: {reported}"
    );
    assert!(
        reported.contains("::error::PyPI notignored-cli 0.1.6 on Linux/X64"),
        "the failure does not name what was being installed, or where from: {reported}"
    );
    assert!(
        reported.contains("ACTION: check https://pypi.org/project/notignored-cli/"),
        "the failure offers no next step: {reported}"
    );
}

/// Bad arguments are an argument error, not a ten-minute wait.
///
/// `--budget` and the delays are handed to `sleep` and to shell arithmetic; a
/// typo'd one would otherwise become a silently different budget on the job that
/// least wants a surprise.
#[test]
fn a_budget_that_is_not_a_number_is_refused_before_anything_is_installed() {
    let install = FakeInstall::new(1);
    let output = retry(&install, &["--budget", "ten minutes"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "a malformed budget did not exit 2: {}",
        stderr(&output)
    );
    assert_eq!(
        install.attempts(),
        0,
        "the install ran despite the malformed budget"
    );
    let reported = stderr(&output);
    assert!(
        reported.contains("--budget needs a whole number of seconds"),
        "the argument error does not name the option: {reported}"
    );
    assert!(
        reported.contains("ACTION: run 'retry-install.sh"),
        "the argument error does not show the usage: {reported}"
    );
}
