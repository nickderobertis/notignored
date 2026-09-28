//! The composite action's install step, run against a release served on
//! loopback.
//!
//! `uses: nickderobertis/notignored@vX.Y.Z` with no `version:` is meant to be the
//! whole pin: the step reads the crate version from the `Cargo.toml` at
//! `$GITHUB_ACTION_PATH` — the action's own tree at the ref the caller named —
//! and installs the Release `v<that version>`. These journeys run the real step,
//! lifted out of `action.yml` the way [`crate::action_scan`] lifts the scan step,
//! with `GITHUB_ACTION_PATH` at this repository, so the real `scripts/install.sh`
//! downloads the real compiled `notignored` from the server
//! [`crate::installer`] runs. The version is read through
//! [`cargo_version`], never spelled, so a release PR's bump cannot turn this red.
//!
//! POSIX-only, for the reason given in [`crate::installer`].
#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Output};

use crate::action_scan::step_script;
use crate::installer::{publish, Release};
use crate::support::{cargo_version, repo_root};

/// A tag no build of this crate carries, for the `latest` document to name so a
/// fall-back to `latest` would be visible in what got installed and requested.
const SOME_OTHER_RELEASE: &str = "v0.0.1";

/// Run the install step with `version` as its input and `action_path` as the
/// action's own checkout, against `release`.
fn install(release: &Release, action_path: &Path, version: &str, temp: &Path) -> Output {
    let script = temp.join("install-step.sh");
    std::fs::write(&script, step_script("Install notignored")).expect("write the step script");
    let outputs = temp.join("outputs.txt");
    std::fs::write(&outputs, "").expect("create the step output file");

    Command::new("bash")
        .arg(&script)
        .current_dir(temp)
        .env("VERSION", version)
        .env("GITHUB_ACTION_PATH", action_path)
        .env("RUNNER_TEMP", temp)
        .env("GITHUB_OUTPUT", &outputs)
        .env("NOTIGNORED_RELEASE_BASE_URL", &release.base_url)
        .env("NOTIGNORED_RELEASE_API_URL", &release.base_url)
        .env_remove("GITHUB_TOKEN")
        .env_remove("NOTIGNORED_VERSION")
        .env_remove("NOTIGNORED_INSTALL_DIR")
        .output()
        .expect("run the install step")
}

/// Point the served `releases/latest` document at `tag`.
fn latest_names(release: &Release, tag: &str) {
    std::fs::write(
        release
            .dir
            .join("releases/repos/nickderobertis/notignored/releases/latest"),
        format!("{{\n  \"tag_name\": \"{tag}\"\n}}\n"),
    )
    .expect("rewrite the latest document");
}

fn text(output: &Output) -> String {
    format!(
        "--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The binary the step installed, as its `bin` output names it.
fn installed(temp: &Path) -> Command {
    let outputs = std::fs::read_to_string(temp.join("outputs.txt")).expect("read the outputs");
    let bin = outputs
        .lines()
        .find_map(|line| line.strip_prefix("bin="))
        .unwrap_or_else(|| panic!("the step set no bin output:\n{outputs}"));
    Command::new(bin)
}

/// The default: an empty `version` installs the release named by the action's
/// own crate version, and asks nothing about `latest`.
#[test]
fn an_empty_version_installs_the_release_the_actions_own_version_names() {
    let tag = format!("v{}", cargo_version());
    let release = publish(&tag, false, true);
    latest_names(&release, SOME_OTHER_RELEASE);
    let temp = tempfile::tempdir().expect("a runner temp directory");

    let output = install(&release, &repo_root(), "", temp.path());
    assert!(output.status.success(), "{}", text(&output));

    let requested = release.requested_paths();
    assert!(
        !requested.is_empty()
            && requested
                .iter()
                .all(|path| path.starts_with(&format!("/{tag}/"))),
        "an empty version must download {tag} and nothing else: {requested:?}"
    );

    let run = installed(temp.path())
        .arg("--version")
        .output()
        .expect("run the installed binary");
    assert!(run.status.success(), "{}", text(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        format!("notignored {}", cargo_version())
    );
}

/// A `Cargo.toml` the step cannot read fails the step, names the input to set,
/// and never falls back to `latest` — which the server here would have answered.
#[test]
fn an_unreadable_manifest_fails_naming_the_version_input() {
    let release = publish(SOME_OTHER_RELEASE, false, true);
    let temp = tempfile::tempdir().expect("a runner temp directory");

    // An action checkout that carries the installer but no manifest to read.
    let action = tempfile::tempdir().expect("an action checkout");
    std::fs::create_dir_all(action.path().join("scripts")).expect("create scripts/");
    std::fs::copy(
        repo_root().join("scripts/install.sh"),
        action.path().join("scripts/install.sh"),
    )
    .expect("copy the installer");

    // No manifest; one with no [package] version; one whose version is not the
    // X.Y.Z a release tag is made of.
    for manifest in [
        None,
        Some("[workspace]\nmembers = []\n"),
        Some("[package]\nname = \"notignored\"\nversion = \"1.2\"\n"),
    ] {
        if let Some(contents) = manifest {
            std::fs::write(action.path().join("Cargo.toml"), contents)
                .expect("write a manifest with no [package] version");
        }
        let output = install(&release, action.path(), "", temp.path());
        assert!(!output.status.success(), "{}", text(&output));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("::error::") && stderr.contains("set the version input"),
            "the failure does not name the version input:\n{}",
            text(&output)
        );
        assert!(
            release.requested_paths().is_empty(),
            "the step reached for a release it was never told to install: {:?}",
            release.requested_paths()
        );
        assert!(
            !temp.path().join("notignored/bin/notignored").exists(),
            "a binary was installed anyway"
        );
    }
}

/// `latest` and an explicit tag keep their meaning beside the new default.
#[test]
fn latest_and_an_explicit_tag_install_what_they_name() {
    let release = publish("v9.9.9", false, true);

    let temp = tempfile::tempdir().expect("a runner temp directory");
    let output = install(&release, &repo_root(), "v9.9.9", temp.path());
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("notignored v9.9.9 installed"),
        "{}",
        text(&output)
    );
    assert!(
        !release
            .requested_paths()
            .iter()
            .any(|path| path.ends_with("/releases/latest")),
        "an explicit tag asked for latest"
    );

    let temp = tempfile::tempdir().expect("a runner temp directory");
    let output = install(&release, &repo_root(), "latest", temp.path());
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        release
            .requested_paths()
            .iter()
            .any(|path| path.ends_with("/releases/latest")),
        "`latest` did not resolve the latest release"
    );
    assert!(installed(temp.path())
        .arg("--version")
        .status()
        .unwrap()
        .success());
}
