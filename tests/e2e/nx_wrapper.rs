//! `scripts/nx.sh`: what it says, and what it keeps.
//!
//! Every recipe in the gate goes through this wrapper, so its output *is* the
//! output of `just check` — and treating all command output as context the next
//! agent has to read means a green run owes a line, not Nx's whole task log. The
//! catch is that a wrapper which simply swallowed that log would make a failure
//! undiagnosable, so the output is not discarded: it is preserved at a path the
//! failure message names and a reader can `tail -f` while the run is still going.
//!
//! Three properties have to hold together, and none of them is visible from
//! reading the script:
//!
//! * a successful run is one line, and Nx's own chatter is not in it;
//! * a failed run prints that chatter in full, plus where to find it again;
//! * a nested run never truncates the log an enclosing one is still writing —
//!   which this repository does on every gate, because `just check` runs the e2e
//!   suite *through* Nx and these journeys spawn the wrapper again from inside it.
//!
//! Nothing here is stubbed: the real script, the real Nx, the real workspace.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::support::{bash_program, repo_root};

/// The log directory, spelled the way `preserved_log_open` spells it.
///
/// Asked of the script rather than assembled here, because a claim is compared
/// as a string and the two spellings have to be byte-identical. On Windows they
/// are not obviously so: the script canonicalizes through Git Bash and answers
/// `D:/a/...` where Rust's own path is `D:\a\...`, and a claim that missed would
/// quietly stop these journeys from nesting — which is the one thing keeping
/// them off the log `just check` is writing around them.
///
/// Probed under a throwaway label for that same reason: asking about `nx` itself
/// would truncate the real log.
fn logs_dir() -> String {
    let output = Command::new(bash_program())
        .args([
            "-c",
            r#". scripts/preserved-log.sh; preserved_log_open "$PWD" nxprobe >/dev/null; printf '%s' "$PRESERVED_LOG""#,
        ])
        .current_dir(repo_root())
        .env_remove("NOTIGNORED_PRESERVED_LOGS")
        .output()
        .expect("ask scripts/preserved-log.sh for its canonical log path");
    assert!(
        output.status.success(),
        "preserved_log_open failed:\n{}",
        stderr(&output)
    );
    let probe = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // The probe file is left where it is. Removing it would race the journeys
    // running beside this one — each opens the same throwaway label, and one that
    // deleted the file between another's create and its `chmod` made
    // `preserved_log_open` fail and answer with nothing. Truncating it
    // concurrently is harmless, deleting it is not, and `.logs/` is ignored
    // scratch either way.
    probe
        .strip_suffix("/nxprobe.log")
        .unwrap_or_else(|| {
            panic!(
                "preserved_log_open answered {probe:?}, not a path ending in \
                 `nxprobe.log`\n\
                 ACTION: it returns its path in PRESERVED_LOG; an empty answer \
                 means one of its own guards failed"
            )
        })
        .to_string()
}

/// The stable log every un-nested invocation writes.
fn stable_log() -> String {
    format!("{}/nx.log", logs_dir())
}

/// Run `scripts/nx.sh` as a *nested* invocation — one whose enclosing run has
/// already claimed the stable log.
///
/// Every journey below runs nested on purpose. It is the shape the gate actually
/// produces, and it is also the only safe one here: an un-nested run would
/// truncate the very log that `just check` is writing while it runs this suite.
fn nx_nested(args: &[&str]) -> Output {
    let mut command = Command::new(bash_program());
    command
        .arg("scripts/nx.sh")
        .args(args)
        .current_dir(repo_root())
        .env("NOTIGNORED_PRESERVED_LOGS", stable_log())
        .env_remove("NOTIGNORED_NX_SHOW_OUTPUT");
    command
        .output()
        .unwrap_or_else(|error| panic!("run scripts/nx.sh {args:?}: {error}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The log path an invocation reported, taken out of whichever stream it spoke
/// on. The wrapper names it in both the success line and the failure advice,
/// because a preserved log nobody can find is a discarded one.
fn reported_log(output: &Output) -> PathBuf {
    let combined = format!("{}{}", stdout(output), stderr(output));
    let path = combined
        .split_once("full output: ")
        .and_then(|(_, rest)| rest.split(')').next())
        .unwrap_or_else(|| {
            panic!(
                "scripts/nx.sh named no preserved log:\nstdout:\n{}\nstderr:\n{}",
                stdout(output),
                stderr(output)
            )
        })
        .trim()
        .to_string();
    PathBuf::from(path)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read the preserved log {}: {error}", path.display()))
}

/// A green run owes one line. `show projects` is the sharpest probe available:
/// it is read-only, and the JSON it prints is unmistakable in a stream that
/// should not carry it.
#[test]
fn a_successful_run_says_one_line_and_keeps_the_rest_out_of_it() {
    let output = nx_nested(&["show", "projects", "--json"]);
    assert!(
        output.status.success(),
        "`nx show projects` failed:\n{}",
        stderr(&output)
    );

    let said = stdout(&output);
    assert_eq!(
        said.lines().count(),
        1,
        "a successful run printed {} lines, not one:\n{said}\n\
         ACTION: scripts/nx.sh must preserve Nx's output and report a single line",
        said.lines().count()
    );
    assert!(
        !said.contains("notignored-sdk-python"),
        "Nx's own output reached stdout on a successful run:\n{said}\n\
         ACTION: preserve it to the log instead"
    );
}

/// ...and the rest is kept, not dropped. Quiet on success is only acceptable
/// while the output is still somewhere a reader can reach.
#[test]
fn a_successful_run_preserves_the_output_it_did_not_print() {
    let output = nx_nested(&["show", "projects", "--json"]);
    let log = reported_log(&output);
    assert!(
        read(&log).contains("notignored-sdk-python"),
        "the preserved log at {} does not hold Nx's output\n\
         ACTION: a wrapper that is quiet on success must still keep what it swallowed",
        log.display()
    );
}

/// A failure is the case the whole design answers to: everything Nx said, on
/// stderr, plus where to read it again. Losing either half is what makes a
/// captured run undiagnosable.
#[test]
fn a_failed_run_prints_everything_nx_said_and_names_the_log() {
    let output = nx_nested(&["run", "notignored:no-such-target"]);
    assert!(
        !output.status.success(),
        "running a target that does not exist should fail"
    );

    let said = stderr(&output);
    assert!(
        said.contains("no-such-target"),
        "the failure did not carry Nx's own diagnostic:\n{said}\n\
         ACTION: stream the preserved log to stderr when the command fails"
    );
    assert!(
        said.contains("scripts/nx.sh") || said.contains("nx:"),
        "the failure named no next action:\n{said}\n\
         ACTION: say what to run to clear it, not just that it broke"
    );
    assert!(
        read(&reported_log(&output)).contains("no-such-target"),
        "the preserved log does not hold the failing run's output"
    );
}

/// The recovery path a captured run depends on: the evidence outlives the
/// process, so a failure can be read after the fact rather than only as it
/// scrolled past.
#[test]
fn a_failed_runs_evidence_is_readable_after_the_process_is_gone() {
    let output = nx_nested(&["run", "notignored:no-such-target"]);
    let log = reported_log(&output);
    assert!(
        log.is_file(),
        "{} does not exist after the run ended\n\
         ACTION: the log must not be a mktemp file an EXIT trap removes",
        log.display()
    );
    assert!(
        read(&log).contains("no-such-target"),
        "the preserved log lost the failing run's output"
    );
}

/// What makes the stable path safe. `just check` runs this suite *through* Nx,
/// so an inner wrapper that truncated `.logs/nx.log` would erase the enclosing
/// gate's evidence exactly while a reader needs it.
///
/// Asserted without ever writing the shared path: the sentinel below stands in
/// for the enclosing run's log, and the whole point is that it comes back
/// untouched.
#[test]
fn a_nested_run_diverts_rather_than_truncating_the_enclosing_log() {
    let claimed = format!("{}/nx-enclosing-fixture.log", logs_dir());
    let sentinel = "evidence from the enclosing run\n";
    std::fs::write(&claimed, sentinel).expect("seed the enclosing run's log");

    // Both paths are claimed: the fixture because this journey is about it, and
    // the stable log because leaving it unclaimed would make this run the
    // *enclosing* one, free to prune the diverted logs its sibling journeys are
    // still writing.
    let claims = format!("{}\n{}", stable_log(), claimed);
    let output = Command::new(bash_program())
        .arg("scripts/nx.sh")
        .args(["show", "projects", "--json"])
        .current_dir(repo_root())
        .env("NOTIGNORED_PRESERVED_LOGS", claims)
        .env_remove("NOTIGNORED_NX_SHOW_OUTPUT")
        .output()
        .expect("run scripts/nx.sh nested");
    assert!(output.status.success(), "{}", stderr(&output));

    assert_ne!(
        reported_log(&output),
        PathBuf::from(&claimed),
        "the nested run took the log its enclosing run is still writing"
    );
    assert_eq!(
        read(Path::new(&claimed)),
        sentinel,
        "the nested run truncated the enclosing run's log\n\
         ACTION: divert to a distinct path when the stable one is already claimed"
    );
    std::fs::remove_file(&claimed).ok();
}

/// Everything the wrapper preserves goes through this filter first. Driven
/// directly rather than by hoping a credential turns up in Nx's own output: what
/// has to be proven is that the filter removes one when it does, and a journey
/// that only ever fed it clean text would pass while doing nothing.
fn redacted(input: &str, env: &[(&str, &str)]) -> String {
    let mut command = Command::new(bash_program());
    command
        .args(["-c", ". scripts/preserved-log.sh; redact_secrets"])
        .current_dir(repo_root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("run redact_secrets");
    std::io::Write::write_all(
        child.stdin.as_mut().expect("the filter's stdin"),
        input.as_bytes(),
    )
    .expect("feed the filter");
    let output = child
        .wait_with_output()
        .expect("collect the filter's output");
    assert!(output.status.success(), "{}", stderr(&output));
    stdout(&output)
}

/// A terminal forgets; a preserved log does not. One failing command that echoed
/// its environment would record a live token on disk, so the value is masked
/// from the only place it is known.
#[test]
fn a_credential_value_never_reaches_the_preserved_log() {
    let secret = "s3cret-value-not-in-any-log";
    let masked = redacted(
        &format!("nx run failed\nAUTHORIZATION: Bearer {secret}\ndone\n"),
        &[("NOTIGNORED_FIXTURE_TOKEN", secret)],
    );
    assert!(
        !masked.contains(secret),
        "a credential value survived redaction:\n{masked}\n\
         ACTION: mask credential-shaped environment values on the way into the log"
    );
    assert!(
        masked.contains("<redacted:NOTIGNORED_FIXTURE_TOKEN>"),
        "the redacted line does not say what was removed:\n{masked}"
    );
    // The surrounding evidence is what the log exists for; masking may not eat it.
    assert!(
        masked.contains("nx run failed") && masked.contains("done"),
        "redaction damaged the evidence around the credential:\n{masked}"
    );
}

/// Masking is keyed on the variable's *name*, not on how the value looks. A
/// filter that hid anything token-shaped would corrupt the very output a reader
/// opened the log for — project names, paths, and hashes are not secrets.
#[test]
fn an_ordinary_environment_value_is_left_alone() {
    let value = "notignored-sdk-python";
    let masked = redacted(
        &format!("nx run {value}:test\n"),
        &[("NOTIGNORED_FIXTURE_PROJECT", value)],
    );
    assert_eq!(
        masked.trim_end(),
        format!("nx run {value}:test"),
        "a non-credential environment value was masked out of the log"
    );
}

/// The escape hatch every graph journey depends on: `tests/e2e/nx_workspace.rs`
/// reads Nx's stdout, which only exists while this mode passes it straight
/// through. Without this the quiet default would silently break those.
#[test]
fn show_output_mode_streams_nx_stdout_unchanged() {
    let output = Command::new(bash_program())
        .arg("scripts/nx.sh")
        .args(["show", "projects", "--json"])
        .current_dir(repo_root())
        .env("NOTIGNORED_NX_SHOW_OUTPUT", "1")
        .env("NOTIGNORED_PRESERVED_LOGS", stable_log())
        .output()
        .expect("run scripts/nx.sh in show-output mode");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("notignored-sdk-python"),
        "show-output mode did not pass Nx's stdout through:\n{}",
        stdout(&output)
    );
}
