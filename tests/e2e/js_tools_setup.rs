//! `scripts/setup-js.sh`, executed — not read.
//!
//! `just bootstrap` runs this script on every machine and every CI leg, so its
//! happy path is continuously proven. What that never reaches is the recovery
//! advice: each failure branch stops the gate and prints the one action that
//! clears it, and advice nobody has run is advice nobody has checked.
//!
//! The script resolves its root from its own path, so each journey below runs it
//! through a symlink in a scratch tree — the real file's bytes, over a layout the
//! test controls. Nothing here reaches the network: both branches stop before
//! `npm ci`.
//!
//! Unix only: the staging is a symlink, and the script still runs for real on
//! Windows — CI's `cross (windows-latest)` leg bootstraps with it on every run.
#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use crate::support::repo_root;

/// The `npm` the script needs to get past its first guard. Missing, the journey
/// fails with the fix rather than proving a different branch by accident.
fn require_npm() {
    let found = Command::new("npm").arg("--version").output();
    assert!(
        found.is_ok_and(|output| output.status.success()),
        "npm is not installed\nACTION: install Node.js 20+ (https://nodejs.org/), which \
         `just bootstrap` needs anyway"
    );
}

/// A scratch directory laid out like the repo root, with the **real** script
/// linked in under `scripts/`.
fn sandbox(with_manifest: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("scratch repo root");
    let scripts = dir.path().join("scripts");
    fs::create_dir_all(&scripts).expect("scripts dir");
    std::os::unix::fs::symlink(
        repo_root().join("scripts/setup-js.sh"),
        scripts.join("setup-js.sh"),
    )
    .expect("link the real script");

    if with_manifest {
        let manifest = dir.path().join("tests/js-toolchain");
        fs::create_dir_all(&manifest).expect("manifest dir");
        for name in ["package.json", "package-lock.json"] {
            fs::copy(
                repo_root().join("tests/js-toolchain").join(name),
                manifest.join(name),
            )
            .expect("copy the pinned manifest");
        }
    }
    dir
}

fn run(root: &Path) -> Output {
    require_npm();
    Command::new("bash")
        .arg(root.join("scripts/setup-js.sh"))
        .output()
        .expect("run setup-js.sh")
}

/// A stale file where the toolchain tree belongs stops the install, and says
/// what to clear.
#[test]
fn a_toolchain_directory_that_cannot_be_created_names_the_fix() {
    let root = sandbox(true);
    fs::create_dir_all(root.path().join(".dev")).expect("the .dev dir");
    // `mkdir -p` cannot make a directory where a regular file already sits.
    fs::write(root.path().join(".dev/js"), "stale").expect("occupy the path");

    let output = run(root.path());
    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot create"), "{stderr}");
    assert!(stderr.contains(".dev/js"), "{stderr}");
    assert!(stderr.contains("ACTION:"), "{stderr}");
}

/// Without the pinned manifest there is nothing to install, and `npm ci` must
/// never be the thing that says so.
#[test]
fn a_missing_pinned_manifest_names_the_files_it_wanted() {
    let root = sandbox(false);

    let output = run(root.path());
    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot copy the pinned manifest"),
        "{stderr}"
    );
    assert!(stderr.contains("tests/js-toolchain"), "{stderr}");
    assert!(stderr.contains("ACTION:"), "{stderr}");
    // It stopped before the install, so nothing was fetched.
    assert!(
        !root.path().join(".dev/js/node_modules").exists(),
        "{stderr}"
    );
}
