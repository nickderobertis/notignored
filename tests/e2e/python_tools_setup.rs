//! `scripts/setup-python-tools.sh`, executed — not read.
//!
//! `just bootstrap` runs this script on every machine and every CI leg, so its
//! happy path is continuously proven. What that never reaches is the recovery
//! advice: each failure branch stops the gate and prints the one action that
//! clears it, and advice nobody has run is advice nobody has checked. These
//! journeys run the real script over a real repo layout in a scratch directory
//! and read exactly what a developer would see.
//!
//! Nothing here reaches the network. The one install that is allowed to start is
//! forced to fail offline, so the deterministic gate stays offline.
//!
//! Unix only: the failure paths are staged by handing the script a `PATH` built
//! from symlinks, which Windows' shell does not model the same way. The script
//! still runs for real on Windows — CI's `cross (windows-latest)` leg bootstraps
//! with it on every run, which is a stronger proof of the success path than a
//! simulation would be.
// llmlint: ignore-file[changed_behavior_has_e2e] the Windows leg of this script is
// exercised for real by CI's `cross (windows-latest)` bootstrap step rather than here;
// staging its failure branches on a Unix host would need a stubbed `command -v`, which
// would test the stub.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::support::repo_root;

/// Every tool the script provisions, in the order it walks them. Only the first
/// one can decide a failing run, so the fixtures below only ever pin `ruff`.
const FIRST_TOOL: &str = "ruff";

/// A scratch directory laid out like the repo root: the **real** script under
/// `scripts/`, plus whichever `.<tool>-version` pins a journey wants it to read.
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("scratch repo root");
        let scripts = dir.path().join("scripts");
        fs::create_dir_all(&scripts).expect("scripts dir");
        fs::copy(script_path(), scripts.join("setup-python-tools.sh")).expect("copy the script");
        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Write `.<tool>-version`, exactly as a developer would.
    fn pin(&self, tool: &str, version: &str) -> &Self {
        fs::write(self.path().join(format!(".{tool}-version")), version).expect("write a pin");
        self
    }

    /// Run the script, with `env` layered onto a PATH that still has `uv`.
    fn run(&self, env: &[(&str, &str)]) -> Output {
        let mut command = Command::new(bash());
        command
            .arg(self.path().join("scripts/setup-python-tools.sh"))
            .current_dir(self.path());
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().expect("run setup-python-tools.sh")
    }
}

fn script_path() -> PathBuf {
    repo_root().join("scripts/setup-python-tools.sh")
}

/// The absolute path of a program on the current PATH.
///
/// Resolved up front so the journeys can hand the script a PATH of their own
/// without losing the ability to launch anything themselves.
fn resolve(program: &str) -> PathBuf {
    let output = Command::new("/bin/sh")
        .args(["-c", &format!("command -v {program}")])
        .output()
        .unwrap_or_else(|error| panic!("look up {program}: {error}"));
    assert!(
        output.status.success(),
        "{program} is not on PATH, so these journeys cannot run the script"
    );
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
}

fn bash() -> PathBuf {
    resolve("bash")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Assert the script refused the run, said each of `needles`, and left the
/// developer an action. Every failure branch owes all three.
fn assert_refused(output: &Output, needles: &[&str]) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "the script should stop with exit 1\n{}",
        stderr_of(output)
    );
    let stderr = stderr_of(output);
    for needle in needles {
        assert!(
            stderr.contains(needle),
            "the failure never told the developer {needle:?}:\n{stderr}"
        );
    }
    assert!(
        stderr.contains("ACTION:"),
        "a failure with no suggested next action:\n{stderr}"
    );
}

/// The gate's own entry point: with every tool already at its pin, a second run
/// must change nothing and say nothing.
///
/// This is the path `just bootstrap` takes on a warm machine, and the one place
/// a stray line of output would land in every developer's and every CI leg's log.
#[test]
fn the_script_is_silent_and_idempotent_once_the_pinned_tools_are_installed() {
    // The rest of the e2e suite already depends on these being installed, so a
    // re-run here is the second half of an idempotence check, not a fresh one.
    let output = Command::new(bash())
        .arg(script_path())
        .current_dir(repo_root())
        .env("PYRIGHT_PYTHON_IGNORE_WARNINGS", "1")
        .output()
        .expect("re-run setup-python-tools.sh");

    assert!(
        output.status.success(),
        "a re-run over an already-provisioned tree failed:\n{}",
        stderr_of(&output)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "the script must be quiet on success"
    );
    assert_eq!(
        stderr_of(&output),
        "",
        "the script must be quiet on success"
    );
}

/// The pin feeds a package requirement, so its shape is a trust boundary: a file
/// that does not hold a plain version never reaches the resolver.
#[test]
fn a_pin_that_is_not_a_version_is_refused_before_it_reaches_the_resolver() {
    for bad in [
        "latest",
        "0.16",
        "0.16.1.2",
        // The one that matters: a pin that would otherwise smuggle an argument
        // into `uv pip install`.
        "0.16.1 --index-url http://example.invalid/simple",
    ] {
        let sandbox = Sandbox::new();
        sandbox.pin(FIRST_TOOL, bad);
        assert_refused(
            &sandbox.run(&[]),
            &[
                ".ruff-version must hold a version like",
                "https://pypi.org/project/ruff/",
            ],
        );
    }
}

/// A pin file that cannot be read at all names the file and the command that
/// brings it back.
#[test]
fn a_missing_pin_file_is_refused_with_the_command_that_restores_it() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[]);
    assert_refused(
        &output,
        &[
            "cannot read",
            ".ruff-version",
            "git checkout -- .ruff-version",
        ],
    );
}

/// Without `uv` there is nothing to install with, and the message has to say so
/// rather than fail somewhere deeper with a resolver error.
#[test]
fn without_uv_the_script_names_uv_and_stops() {
    let sandbox = Sandbox::new();
    sandbox.pin(FIRST_TOOL, "0.16.1");

    // Everything the script shells out to, and nothing else — so `command -v uv`
    // inside it genuinely finds nothing while the rest still works.
    let bin = sandbox.path().join("no-uv-bin");
    fs::create_dir_all(&bin).expect("a PATH directory");
    for program in ["dirname", "tr", "grep", "head", "rm", "cat"] {
        std::os::unix::fs::symlink(resolve(program), bin.join(program)).expect("link a utility");
    }
    let path = bin.to_str().expect("a UTF-8 scratch path");

    // Prove the stage before trusting it: a PATH that still finds uv would make
    // this journey assert the wrong branch and pass anyway.
    let probe = Command::new(bash())
        .args(["-c", "command -v uv"])
        .env("PATH", path)
        .output()
        .expect("probe for uv");
    assert!(
        !probe.status.success(),
        "the scratch PATH still finds uv at {}, so this cannot test its absence",
        String::from_utf8_lossy(&probe.stdout).trim()
    );

    assert_refused(
        &sandbox.run(&[("PATH", path)]),
        &[
            "uv not found",
            "cannot install the pinned ruff (0.16.1)",
            "https://docs.astral.sh/uv/",
        ],
    );
}

/// When the venv cannot be created the message has to point at the path, because
/// the cause (a stale file in the way, a full disk) is outside the script.
#[test]
fn a_venv_that_cannot_be_created_reports_the_path_and_what_to_check() {
    let sandbox = Sandbox::new();
    sandbox.pin(FIRST_TOOL, "0.16.1");
    // `.dev` is where every tool's venv goes. A regular file there is something
    // `rm -rf .dev/ruff` shrugs at and `uv venv` cannot work around.
    fs::write(sandbox.path().join(".dev"), "not a directory").expect("stage .dev as a file");

    assert_refused(
        &sandbox.run(&[]),
        &["cannot create the venv at .dev/ruff", "uv --version"],
    );
}

/// An install that fails has to name the requirement it could not get, or the
/// developer cannot tell a bad pin from a bad network.
#[test]
fn an_unresolvable_pin_reports_the_requirement_and_both_of_its_causes() {
    let sandbox = Sandbox::new();
    // A shape-valid pin that no index publishes, resolved with the network shut
    // off: the install fails the same way on a laptop and on an air-gapped runner.
    sandbox.pin(FIRST_TOOL, "9.9.9");

    assert_refused(
        &sandbox.run(&[("UV_OFFLINE", "1")]),
        &[
            "cannot install ruff==9.9.9",
            "network access to PyPI",
            ".ruff-version to a published release",
        ],
    );
}
