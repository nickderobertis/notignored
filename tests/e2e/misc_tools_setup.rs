//! `scripts/setup-misc-tools.sh`, executed — not read.
//!
//! The installer that provisions ShellCheck, llmlint, and maturin is what makes
//! `just bootstrap` enough to run this suite from a clean clone. Its pinning is
//! the part worth proving: `tests/e2e/packaging.rs` builds the wheel with
//! whatever `.dev/maturin` holds, so if this script stopped provisioning it — a
//! line lost in a merge, a pin the matcher no longer recognizes — the packaging
//! journey would either fail far from the cause or quietly build with something
//! else.
//!
//! These journeys run the **real** script over the real repository, which is the
//! state `just bootstrap` leaves and every CI leg starts from. Nothing is
//! stubbed and nothing reaches the network: with every tool already at its pin
//! the script installs nothing, which is exactly the path being asserted.
//!
//! The failure branches are not re-proven here. `read_pin`, `tool_binary`, and
//! `is_pinned` are one implementation shared with `scripts/setup-python-tools.sh`,
//! and [`python_tools_setup`](python_tools_setup.rs) drives every one of their
//! refusals for real.
//!
//! Unix only, matching its sibling: staging this script's environment on Windows
//! needs a shell that models `PATH` the same way. CI's `cross (windows-latest)`
//! leg still runs it for real through `just bootstrap` on every run.
#![cfg(unix)]

use std::process::Command;

use crate::support::{pinned_version, repo_root, tool_binary};

/// Every tool this installer owns. `just bootstrap` provisions all three, and
/// the packaging journey depends on the third.
const TOOLS: [&str; 3] = ["shellcheck", "llmlint", "maturin"];

/// A re-run over an already-provisioned tree changes nothing and says nothing.
///
/// This is the path `just bootstrap` takes on a warm machine and on every CI
/// leg, so a stray line of output here would land in every developer's log. It
/// is also what proves each tool's installed version is *recognized* as the pin:
/// a version the matcher could not read would send the script off to reinstall,
/// which is neither silent nor offline.
#[test]
fn the_script_is_silent_and_idempotent_once_the_pinned_tools_are_installed() {
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/setup-misc-tools.sh"))
        .current_dir(repo_root())
        .output()
        .expect("re-run setup-misc-tools.sh");

    assert!(
        output.status.success(),
        "a re-run over an already-provisioned tree failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "the script must be quiet on success"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "the script must be quiet on success"
    );
}

/// After the script runs, every tool it owns resolves at the version its pin
/// file names.
///
/// The assertion is on what running the installer *leaves behind*, not on what
/// its source says: `tool_binary` resolves each tool under `.dev/<tool>` and
/// asserts the binary reports the pin. On a cold checkout — every CI leg — the
/// run above is the only thing that could have put it there, so this fails when
/// the script stops provisioning one and when it provisions a version the repo
/// does not declare. maturin is the reason it exists: it is not a parity tool, so
/// nothing else in the suite would notice it going missing until a wheel came out
/// wrong.
#[test]
fn every_tool_the_script_owns_resolves_at_its_pin_afterwards() {
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/setup-misc-tools.sh"))
        .current_dir(repo_root())
        .output()
        .expect("run setup-misc-tools.sh");
    assert!(
        output.status.success(),
        "the installer failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for tool in TOOLS {
        // Panics with the `just bootstrap` action when it is missing or wrong.
        let binary = tool_binary(tool);
        assert!(
            binary.exists(),
            "{tool} is not installed at {}",
            binary.display()
        );
        assert!(
            !pinned_version(tool).is_empty(),
            ".{tool}-version is empty, so nothing pins {tool}"
        );
    }
}
