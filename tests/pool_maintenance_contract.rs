//! Keep the maintenance paths aligned with Cargo's checked-in configuration.

use std::path::Path;
use std::process::Command;

#[test]
fn maintenance_covers_the_configured_cargo_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tracked = Command::new("git")
        .args([
            "ls-files",
            "--",
            "**/.cargo/config.toml",
            ".cargo/config.toml",
            "**/.cargo/config",
            ".cargo/config",
        ])
        .current_dir(root)
        .output()
        .expect("list tracked Cargo configurations");
    assert!(tracked.status.success(), "git ls-files failed");
    let paths = String::from_utf8(tracked.stdout).expect("UTF-8 configuration paths");
    assert_eq!(
        paths.lines().collect::<Vec<_>>(),
        [".cargo/config.toml"],
        "Cargo configuration changed: resolve every target-dir and update maintenance and this gate"
    );
    let config =
        std::fs::read_to_string(root.join(".cargo/config.toml")).expect("read Cargo configuration");
    let settings: Vec<_> = config
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    // Pin the small build section's shape so a new spelling or setting fails
    // closed instead of silently evading this path contract.
    let build = settings
        .iter()
        .position(|line| *line == "[build]")
        .expect("build section");
    let end = settings[build + 1..]
        .iter()
        .position(|line| line.starts_with('['))
        .map_or(settings.len(), |offset| build + 1 + offset);
    assert_eq!(&settings[build + 1..end], ["target-dir = \"target\""]);

    let justfile = std::fs::read_to_string(root.join("justfile")).expect("read justfile");
    let recipe = justfile
        .split("\npool-maintenance:\n")
        .nth(1)
        .expect("pool-maintenance recipe")
        .split("\n\n")
        .next()
        .expect("recipe body");
    assert_eq!(
        recipe.trim(),
        "@cargo-prune-superseded --keep-within 2d target"
    );
    let measurement = std::fs::read_to_string(root.join("pool-maintenance/measure.sh"))
        .expect("read measurement script");
    assert!(
        measurement
            .lines()
            .any(|line| line == "readonly TARGET=\"target\""),
        "measurement must inventory the same Cargo output as the declared recipe"
    );
}
