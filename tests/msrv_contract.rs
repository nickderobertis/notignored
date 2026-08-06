//! One MSRV, two files that have to agree on it.
//!
//! `Cargo.toml`'s `rust-version` is the source of truth: `just msrv` reads it,
//! and so does CI's msrv job. `clippy.toml` restates it — it has to, because
//! clippy takes its own `msrv` key and reads no manifest — and a stale copy
//! there is silent: clippy simply keeps suggesting APIs the declared floor does
//! not have, and the floor still builds. This is the reconciling check.

use std::path::Path;

fn source(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// The value of a top-level `key = "value"` line, ignoring comments.
fn declared(text: &str, key: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| line.strip_prefix(key))
        .and_then(|rest| rest.trim_start().strip_prefix('='))
        .map(|value| value.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| panic!("no `{key}` is declared"))
}

#[test]
fn clippy_lints_against_the_msrv_the_manifest_declares() {
    let manifest = declared(&source("Cargo.toml"), "rust-version");
    let clippy = declared(&source("clippy.toml"), "msrv");
    assert!(!manifest.is_empty(), "Cargo.toml declares no rust-version");
    assert_eq!(
        clippy, manifest,
        "clippy.toml's msrv drifted from Cargo.toml's rust-version; raise both, \
         or clippy lints against a floor this crate no longer promises"
    );
}
