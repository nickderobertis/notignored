//! Drift gate for the screenshot machinery.
//!
//! The capture itself is out of `just check` on purpose — it needs `freeze`, and
//! CI's `Visual docs` workflow owns the comparison. But the *configuration*
//! around it is spread over five files that have to agree, and every way they can
//! disagree fails somewhere expensive and late:
//!
//! - the `freeze` pin lives in `justfile`, in `visual-docs.yml`'s
//!   `capture-command`, and in the release URL that command downloads. Two
//!   versions of a text renderer reflow every shot, so a drifted pin turns the
//!   strict gate into "CI is red and nobody knows why".
//! - `[guard].paths` names files that decide what a scene shows. A path that has
//!   been renamed away silently stops guarding, and the local pre-push guard goes
//!   quiet rather than failing.
//! - the committed baseline and the committed README images are two halves of one
//!   capture; a baseline blessed without the images (or vice versa) publishes a
//!   README showing output the gate is no longer checking.
//!
//! So these read the files as text, the way `tests/action_contract.rs` reads
//! `action.yml`, and fail the build instead.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// The `freeze` version the justfile pins.
fn pinned_freeze() -> String {
    read("justfile")
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("freeze-version := ")
                .map(str::trim)
        })
        .and_then(|value| value.trim_matches('"').to_string().into())
        .expect("the justfile declares a `freeze-version`")
}

/// Every scene name the committed baseline carries.
fn baseline_scenes() -> Vec<String> {
    let baseline: serde_json::Value = serde_json::from_str(&read("shots/baseline/x86_64.json"))
        .expect("the committed baseline is valid JSON");
    let mut names: Vec<String> = baseline["shots"]
        .as_array()
        .expect("the baseline lists shots")
        .iter()
        .map(|shot| {
            shot["name"]
                .as_str()
                .expect("every shot is named")
                .to_string()
        })
        .collect();
    names.sort();
    names
}

/// CI renders the shots with the version the justfile installs, or the strict
/// gate fails on a reflow nobody asked for.
#[test]
fn ci_captures_with_the_freeze_version_the_justfile_pins() {
    let pinned = pinned_freeze();
    let workflow = read(".github/workflows/visual-docs.yml");
    let expected_release = format!(
        "https://github.com/charmbracelet/freeze/releases/download/v{pinned}/\
         freeze_{pinned}_Linux_x86_64.tar.gz"
    );
    assert!(
        workflow.contains(&expected_release),
        "visual-docs.yml downloads a freeze other than the pinned {pinned}"
    );
    assert!(
        workflow.contains(&format!("freeze_{pinned}_Linux_x86_64/freeze")),
        "visual-docs.yml installs from a directory other than the pinned {pinned}'s"
    );
    assert!(
        read("scripts/screenshots.sh").contains("just screenshots-tools"),
        "the capture script no longer names the recipe that installs freeze"
    );
}

/// The capture builds the crate, so the container has to carry the toolchain the
/// repository pins — a floating `rust:latest` would eventually stop compiling it.
#[test]
fn the_capture_container_is_the_pinned_toolchain() {
    let channel = read("rust-toolchain.toml")
        .lines()
        .find_map(|line| line.trim().strip_prefix("channel = "))
        .map(|value| value.trim().trim_matches('"').to_string())
        .expect("rust-toolchain.toml pins a channel");
    let workflow = read(".github/workflows/visual-docs.yml");
    assert!(
        workflow.contains(&format!("container: rust:{channel}-bookworm")),
        "visual-docs.yml captures in a container other than the pinned {channel}"
    );
    assert!(
        workflow.contains(&format!("RUSTUP_TOOLCHAIN={channel}")),
        "visual-docs.yml builds with a toolchain other than the pinned {channel}"
    );
}

/// A guard path that no longer exists guards nothing, and does so silently.
#[test]
fn every_guarded_path_still_exists() {
    let config = read("screencomp.toml");
    let listed: Vec<String> = config
        .lines()
        .skip_while(|line| line.trim() != "paths = [")
        .skip(1)
        .take_while(|line| line.trim() != "]")
        .filter_map(|line| {
            let trimmed = line.trim().trim_end_matches(',');
            trimmed
                .strip_prefix('"')?
                .strip_suffix('"')
                .map(str::to_string)
        })
        .collect();
    assert!(
        listed.len() >= 5,
        "screencomp.toml's [guard].paths looks empty: {listed:?}"
    );
    for entry in &listed {
        // A `**` glob guards a directory; anything else names a file.
        let target = entry.trim_end_matches("/**").trim_end_matches("**");
        let path = repo_root().join(target.trim_end_matches('/'));
        assert!(
            path.exists(),
            "screencomp.toml guards {entry:?}, which no longer exists — a renamed \
             file stops the pre-push guard silently"
        );
    }
    // The things that decide what a scene shows: both renderers, the CLI surface,
    // the capture machinery, and the vendored font (under screenshots/).
    for required in [
        "src/cli/render.rs",
        "src/cli/markdown.rs",
        "src/cli/mod.rs",
        "scripts/screenshots.sh",
        "screenshots/**",
    ] {
        assert!(
            listed.iter().any(|entry| entry == required),
            "screencomp.toml no longer guards {required}"
        );
    }
}

/// One capture produces both halves. A baseline blessed without refreshing the
/// README images publishes screenshots the gate is not checking.
#[test]
fn the_committed_images_are_exactly_the_scenes_the_baseline_gates() {
    let scenes = baseline_scenes();
    assert!(!scenes.is_empty(), "the committed baseline gates nothing");

    let docs = repo_root().join("docs/screenshots");
    let mut committed: Vec<String> = std::fs::read_dir(&docs)
        .unwrap_or_else(|error| panic!("read {}: {error}", docs.display()))
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".svg").map(str::to_string)
        })
        .collect();
    committed.sort();
    assert_eq!(
        committed, scenes,
        "docs/screenshots/ and the committed baseline disagree — re-run \
         `just screenshots-bless` and commit both"
    );
}

/// A gallery nobody can see documents nothing: every captured scene has to be
/// embedded in the README, with alt text, and so does the hero GIF.
#[test]
fn the_readme_embeds_every_scene_and_the_hero_gif() {
    let readme = read("README.md");
    for scene in baseline_scenes() {
        let target = format!("](docs/screenshots/{scene}.svg)");
        let embed = readme
            .lines()
            .find(|line| line.contains(&target))
            .unwrap_or_else(|| panic!("the README does not embed docs/screenshots/{scene}.svg"));
        let alt = embed
            .split_once("![")
            .and_then(|(_, rest)| rest.split_once(']'))
            .map(|(alt, _)| alt)
            .unwrap_or_default();
        assert!(
            alt.len() > 30,
            "docs/screenshots/{scene}.svg is embedded with no descriptive alt text: {embed}"
        );
    }

    let hero = readme
        .lines()
        .find(|line| line.contains("](docs/screenshots/demo.gif)"))
        .expect("the README embeds the hero GIF");
    assert!(
        Path::new(&repo_root().join("docs/screenshots/demo.gif")).exists(),
        "the README embeds a hero GIF that is not committed — run `just screenshots-gif`"
    );
    let hero_line = readme.lines().position(|line| line == hero).unwrap_or(0);
    assert!(
        hero_line < 10,
        "the hero GIF is no longer at the top of the README (line {hero_line})"
    );
}

/// Screenshots are informational: a capture in the gate would make `just check`
/// need `freeze`, a network fetch, and a release build.
#[test]
fn capturing_is_not_part_of_the_gate() {
    let justfile = read("justfile");
    let gate = justfile
        .lines()
        .skip_while(|line| !line.starts_with("check:"))
        .take_while(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!gate.is_empty(), "the justfile no longer defines `check`");
    assert!(
        !gate.contains("screenshot"),
        "`just check` now captures screenshots:\n{gate}"
    );
    let project: serde_json::Value =
        serde_json::from_str(&read("project.json")).expect("project.json is valid JSON");
    let targets = project["targets"].to_string();
    assert!(
        !targets.contains("screenshot"),
        "an Nx target now captures screenshots: {targets}"
    );
}
