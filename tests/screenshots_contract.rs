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

/// The capture must not inherit git's hook environment.
///
/// `.githooks/pre-push` runs `scripts/screenshots.sh`, and git exports `GIT_DIR`
/// (and friends) to the hooks it runs. Inherited, they aim the throwaway
/// repository the `diff` and `pr-comment` scenes are driven inside back at the
/// developer's checkout: `--diff` then reports "not inside a git work tree" and
/// both scenes capture that instead of the review case — so the guard blocks
/// every screenshot-relevant push on drift it created itself.
///
/// Asserted as text because the behaviour cannot be: reproducing it needs
/// `freeze`, a release build, and a real capture, all of which are out of the
/// gate for the reasons this file opens with.
#[test]
fn the_capture_drops_the_git_environment_a_hook_hands_it() {
    let script = read("scripts/screenshots.sh");
    let unset = script
        .find("unset GIT_DIR")
        .unwrap_or_else(|| panic!("scripts/screenshots.sh no longer unsets GIT_DIR:\n{script}"));
    // The statement is one logical line, wrapped with a backslash continuation.
    let mut statement = String::new();
    for line in script[unset..].lines() {
        statement.push_str(line);
        if !line.ends_with('\\') {
            break;
        }
    }
    for variable in ["GIT_WORK_TREE", "GIT_INDEX_FILE"] {
        assert!(
            statement.contains(variable),
            "scripts/screenshots.sh no longer unsets {variable}: {statement}"
        );
    }
    let init = script
        .find("git -c init.defaultBranch=main init")
        .expect("the diff scene no longer builds a throwaway repository");
    assert!(
        unset < init,
        "the git environment is dropped after the throwaway repository is built"
    );
}

/// The one value the rendered-comment capture cannot take from the tree.
///
/// Both capture scripts pin their permalinks to the same literal sha, because
/// neither can know the sha of the commit that adds a screenshot. Two scenes of
/// the same review case that disagreed about it would show a reader two
/// different commits for one change.
#[test]
fn both_captures_pin_their_permalinks_to_the_same_commit() {
    let literal = |script: &str| {
        let sha = read(script)
            .lines()
            .find_map(|line| line.trim().strip_prefix("permalink_sha=").map(str::trim))
            .map(|value| value.trim_matches('"').to_string())
            .unwrap_or_else(|| panic!("{script} no longer pins a permalink_sha"));
        // A permalink is only a permalink if it names a commit: `--github-sha`
        // rejects anything but a 40-hex id, so a script that drifted to a short
        // sha or an empty string would fail the capture rather than this test.
        assert!(
            sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
            "{script} pins a permalink_sha that is not a commit id: {sha:?}"
        );
        sha
    };
    assert_eq!(
        literal("scripts/screenshots.sh"),
        literal("scripts/pr-comment-body.sh"),
        "the `pr-comment` scene and the rendered-comment PNGs pin their \
         permalinks to different commits — they photograph one review case"
    );
}

/// The renderer that lays GitHub's markup over a body has to know every language
/// a fence can name, or a snippet loses its colour without anything failing.
#[test]
fn the_comment_renderer_knows_every_language_a_fence_can_name() {
    let tags: Vec<String> = read("src/cli/markdown.rs")
        .lines()
        .skip_while(|line| !line.contains("match Language::from_path"))
        .take_while(|line| !line.contains("Language::Unknown"))
        .filter_map(|line| {
            let (_, tag) = line.trim().split_once("=> ")?;
            Some(
                tag.trim()
                    .trim_end_matches(',')
                    .trim_matches('"')
                    .to_string(),
            )
        })
        .collect();
    assert!(
        tags.len() >= 5,
        "could not read the fence languages out of src/cli/markdown.rs: {tags:?}"
    );
    let renderer = read("scripts/comment-render/render.mjs");
    for tag in &tags {
        assert!(
            renderer.contains(&format!("\"{tag}\"")),
            "scripts/comment-render/render.mjs does not know the `{tag}` fence \
             src/cli/markdown.rs emits — its snippets would render uncoloured"
        );
    }
}

/// The rendered-comment capture, its installer, and the two PNGs it writes.
///
/// A raster is not byte-reproducible, so screencomp cannot gate these the way it
/// gates the SVGs — which leaves the README free to point at an image nobody
/// committed, or the recipe that makes one free to disappear. These read the
/// real files instead.
#[test]
fn the_rendered_comment_capture_and_the_images_it_writes_are_all_present() {
    for image in [
        "docs/screenshots/pr-comment-rendered.png",
        "docs/screenshots/pr-comment-rendered-dark.png",
    ] {
        assert!(
            repo_root().join(image).is_file(),
            "{image} is not committed — run `just screenshots-pr-comment`"
        );
    }
    for script in [
        "scripts/pr-comment-body.sh",
        "scripts/pr-comment-png.sh",
        "scripts/setup-comment-render.sh",
        "scripts/comment-render/render.mjs",
        "scripts/comment-render/comment.css",
        "scripts/comment-render/package-lock.json",
    ] {
        assert!(
            repo_root().join(script).is_file(),
            "{script} is gone — the rendered comment could no longer be regenerated"
        );
    }
    let justfile = read("justfile");
    for recipe in ["screenshots-pr-comment:", "screenshots-comment-tools:"] {
        assert!(
            justfile.contains(recipe),
            "the justfile no longer defines `{}`",
            recipe.trim_end_matches(':')
        );
    }
    assert!(
        read("scripts/pr-comment-png.sh").contains("scripts/setup-comment-render.sh"),
        "the capture no longer installs its toolchain on demand"
    );
    assert!(
        read("screenshots/AGENTS.md").contains("pr-comment-rendered"),
        "screenshots/AGENTS.md no longer records what the rendered-comment capture is"
    );
}

/// A picture nobody can see sells nothing, and a `<picture>` missing its dark
/// source reads harshly under the README's dark-themed hero.
#[test]
fn the_readme_leads_the_action_with_the_rendered_comment() {
    let readme = read("README.md");
    let dark = readme
        .find("srcset=\"docs/screenshots/pr-comment-rendered-dark.png\"")
        .expect("the README's <picture> offers a dark rendered-comment source");
    let light = readme
        .find("src=\"docs/screenshots/pr-comment-rendered.png\"")
        .expect("the README embeds the light rendered comment");
    let opened = readme[..dark]
        .rfind("<picture>")
        .expect("the dark source sits inside a <picture>");
    let closed = readme[opened..]
        .find("</picture>")
        .map(|offset| opened + offset)
        .expect("the <picture> is closed");
    assert!(
        light < closed,
        "the light rendered comment is outside the <picture> its dark source opens"
    );

    let img = readme[..light]
        .rfind("<img ")
        .expect("the light rendered comment is embedded as an <img>");
    let alt = readme[img..light]
        .split_once("alt=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(alt, _)| alt)
        .unwrap_or_default();
    assert!(
        alt.len() > 30,
        "docs/screenshots/pr-comment-rendered.png is embedded with no descriptive alt text: {alt:?}"
    );

    // Where it sits is the point: in the main text under the `--diff` scene, not
    // behind the fold the markdown *source* still lives in.
    let diff_scene = readme
        .find("](docs/screenshots/diff.svg)")
        .expect("the README embeds the --diff scene");
    let fold = readme
        .find("<details>")
        .expect("the README still folds the remaining scenes away");
    assert!(
        diff_scene < opened && closed < fold,
        "the rendered comment is no longer in the README's main text between the \
         --diff scene and the collapsed fold"
    );
}

/// The render toolchain is a browser download. It must never become something an
/// ordinary contributor has to get through to run `just check`.
#[test]
fn the_rendered_comment_toolchain_is_out_of_bootstrap_and_ci() {
    let root_manifest = read("package.json");
    for dependency in ["markdown-it", "shiki", "playwright"] {
        assert!(
            !root_manifest.contains(dependency),
            "{dependency} has leaked into the workspace package.json"
        );
    }
    let justfile = read("justfile");
    for recipe in ["bootstrap:", "_crate-bootstrap:", "check:"] {
        let body = justfile
            .lines()
            .skip_while(|line| !line.starts_with(recipe))
            .take_while(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !body.is_empty(),
            "the justfile no longer defines `{recipe}`"
        );
        assert!(
            !body.contains("comment-render") && !body.contains("pr-comment"),
            "`just {}` now needs the rendered-comment toolchain:\n{body}",
            recipe.trim_end_matches(':')
        );
    }
    assert!(
        !read("screencomp.toml").contains("comment-render"),
        "screencomp.toml's guard now watches the rendered-comment toolchain, whose \
         capture nothing gates"
    );
    let workflows = repo_root().join(".github/workflows");
    let listing = std::fs::read_dir(&workflows)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflows.display()));
    for entry in listing {
        let path = entry
            .unwrap_or_else(|error| panic!("read an entry of {}: {error}", workflows.display()))
            .path();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            !text.contains("comment-render") && !text.contains("screenshots-pr-comment"),
            "{} runs the rendered-comment capture — it is informational and \
             downloads a browser",
            path.display()
        );
    }
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
