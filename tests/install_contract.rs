//! Keeps every install surface on one asset-naming contract.
//!
//! `scripts/install.sh` constructs a release-asset name; `release.yml` produces
//! one. The moment the two spell it differently, the documented install
//! one-liner 404s — a failure local testing never catches because there is no
//! release to try. This test compares the two templates directly.
//!
//! Comparing them to *each other* is not enough on its own, and v0.1.11 is the
//! proof: both sides agreed on the archive stem, and the installer then asked for
//! the checksum as `<archive>.sha256` while every release has ever published it
//! as `<stem>.sha256`. So the rendered names are also held against
//! `tests/fixtures/release-assets/v0.1.11.txt` — a real release's real asset
//! listing, which no template in this repository can talk into agreeing with it.

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

/// A `# <MARKER>:` template `scripts/install.sh` records for a name it builds.
fn installer_marker(script: &str, marker: &str) -> String {
    let prefix = format!("# {marker}:");
    script
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| panic!("install.sh records a {marker} marker"))
}

/// The template `scripts/install.sh` records for the archive stem it builds.
fn installer_template(script: &str) -> String {
    installer_marker(script, "ASSET_NAME_TEMPLATE")
}

/// Every asset name a real release carried, from a listing this repository did
/// not generate. Comment lines let the fixture say where it came from.
fn published_asset_names() -> Vec<String> {
    read("tests/fixtures/release-assets/v0.1.11.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Fill a `$bin-$tag-$target` template in, the way both sides do at runtime.
fn render(template: &str, target: &str) -> String {
    template
        .replace("$bin", "notignored")
        .replace("$tag", "v0.1.11")
        .replace("$target", target)
}

/// The five targets `release.yml` builds, with the archive extension each one's
/// upload carries.
const PUBLISHED_TARGETS: &[(&str, &str)] = &[
    ("x86_64-unknown-linux-gnu", "tar.gz"),
    ("aarch64-unknown-linux-gnu", "tar.gz"),
    ("x86_64-apple-darwin", "tar.gz"),
    ("aarch64-apple-darwin", "tar.gz"),
    ("x86_64-pc-windows-msvc", "zip"),
];

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
        script.contains(r#"ASSET="$BIN-$VERSION-$TARGET""#),
        "install.sh no longer builds the asset stem from BIN/VERSION/TARGET"
    );
    assert!(
        script.contains(r#"ARCHIVE="$ASSET.$EXT""#),
        "install.sh must give the stem the archive's extension"
    );
    assert!(
        script.contains(r#"CHECKSUM="$ASSET.sha256""#),
        "install.sh must give the stem the `.sha256` extension, not append one \
         to the archive name — the release publishes the two as siblings"
    );
    assert!(
        script.contains(r#""$BASE_URL/$VERSION/$CHECKSUM""#),
        "install.sh must fetch the checksum name it built"
    );
    assert!(
        !script.contains("$ARCHIVE.sha256"),
        "install.sh appends `.sha256` to the archive name; no release has ever \
         published that name (see tests/fixtures/release-assets/v0.1.11.txt)"
    );
}

/// The checksum's name is the archive's *stem* plus `.sha256`, because that is
/// how `taiki-e/upload-rust-binary-action` derives it from the `archive:` input.
#[test]
fn the_checksum_template_is_the_asset_stem_plus_sha256() {
    let script = read("scripts/install.sh");
    let stem = installer_template(&script);
    let checksum = installer_marker(&script, "CHECKSUM_NAME_TEMPLATE");
    assert_eq!(
        checksum,
        format!("{stem}.sha256"),
        "the checksum template must extend the archive stem, not the archive name"
    );
}

/// The names the installer would actually request, against the names a release
/// actually carries. This is the check the v0.1.11 outage needed: an
/// installer-versus-workflow comparison passed throughout it, because both
/// spelled the stem the same and only the installer's checksum suffix was wrong.
#[test]
fn every_url_the_installer_builds_names_a_published_asset() {
    let script = read("scripts/install.sh");
    let stem = installer_template(&script);
    let checksum = installer_marker(&script, "CHECKSUM_NAME_TEMPLATE");
    let published = published_asset_names();
    assert_eq!(
        published.len(),
        PUBLISHED_TARGETS.len() * 2,
        "the release manifest should hold an archive and a checksum per target"
    );

    for (target, extension) in PUBLISHED_TARGETS {
        for name in [
            format!("{}.{extension}", render(&stem, target)),
            render(&checksum, target),
        ] {
            assert!(
                published.contains(&name),
                "install.sh would request {name}, which release v0.1.11 does not \
                 publish; its assets are {published:?}"
            );
        }
    }
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
