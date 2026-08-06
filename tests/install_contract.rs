//! Keeps every install surface on one asset-naming contract.
//!
//! `scripts/install.sh` constructs a release-asset name; `release.yml` produces
//! one. The moment the two spell it differently, the documented install
//! one-liner 404s — a failure local testing never catches because there is no
//! release to try. This test compares the two templates directly.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// The `archive:` template the release workflow hands to
/// `taiki-e/upload-rust-binary-action`.
fn release_workflow_template(workflow: &str) -> String {
    workflow
        .lines()
        .find_map(|line| line.trim().strip_prefix("archive:"))
        .map(|value| value.trim().to_string())
        .expect("release.yml declares an `archive:` template")
}

/// The template `scripts/install.sh` records for the name it builds.
fn installer_template(script: &str) -> String {
    script
        .lines()
        .find_map(|line| line.trim().strip_prefix("# ASSET_NAME_TEMPLATE:"))
        .map(|value| value.trim().to_string())
        .expect("install.sh records an ASSET_NAME_TEMPLATE marker")
}

#[test]
fn the_installer_and_the_release_workflow_agree_on_the_archive_name() {
    let workflow = release_workflow_template(&read(".github/workflows/release.yml"));
    let installer = installer_template(&read("scripts/install.sh"));
    assert_eq!(
        installer, workflow,
        "install.sh and release.yml disagree on the release-asset name template; \
         one of them would 404"
    );
    assert_eq!(workflow, "$bin-$tag-$target");
}

#[test]
fn the_installer_builds_the_name_its_marker_promises() {
    let script = read("scripts/install.sh");
    // `$bin-$tag-$target` in the workflow is `$BIN-$VERSION-$TARGET` in the shell.
    assert!(
        script.contains(r#"ARCHIVE="$BIN-$VERSION-$TARGET.$EXT""#),
        "install.sh no longer builds the archive name from BIN/VERSION/TARGET"
    );
    assert!(
        script.contains(r#""$BASE_URL/$VERSION/$ARCHIVE.sha256""#),
        "install.sh must fetch the `.sha256` the release workflow publishes"
    );
}

#[test]
fn the_release_workflow_publishes_checksums_for_every_target() {
    let workflow = read(".github/workflows/release.yml");
    assert!(
        workflow.contains("checksum: sha256"),
        "release.yml must publish SHA-256 checksums"
    );
    for target in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ] {
        assert!(
            workflow.contains(target),
            "release.yml does not build {target}"
        );
        assert!(
            read("scripts/install.sh").contains(target_suffix(target)),
            "install.sh cannot construct a name for {target}"
        );
    }
}

/// The portion of a triple `install.sh` assembles from `uname` output.
fn target_suffix(target: &str) -> &str {
    target.split_once('-').expect("a target triple").1
}

#[test]
fn the_installer_refuses_to_install_without_verifying() {
    let script = read("scripts/install.sh");
    assert!(
        script.contains("refusing to install unverified"),
        "install.sh must abort when it cannot verify the download"
    );
    assert!(
        script.contains("checksum mismatch"),
        "install.sh must abort on a checksum mismatch"
    );
}

#[test]
fn the_readme_documents_the_installer_it_ships() {
    let readme = read("README.md");
    assert!(
        readme.contains("scripts/install.sh"),
        "the README must document the installer CI and users rely on"
    );
    assert!(
        Path::new(&repo_root().join("scripts/install.sh")).exists(),
        "the documented installer is missing"
    );
}
