//! End-to-end journeys: the compiled binary, real files, and the real linters
//! and type checkers it claims parity with.
//!
//! Split into its own test target so `just test-e2e` can run the slow binary
//! journeys alone — but `just check` runs them either way. Nothing here is
//! `#[ignore]`d: an opt-in e2e is coverage that quietly stops happening.

mod support;

mod action_comment;
mod action_scan;
mod biome_parity;
mod cli;
mod diff;
mod eslint_parity;
mod examples;
mod installer;
mod js_tools_setup;
mod llmlint_parity;
mod markdown;
mod misc_tools_setup;
mod packaging;
mod polyglot;
mod publish_npm;
mod python_tools_setup;
mod python_types_parity;
mod ruff_grammar;
mod ruff_parity;
mod rust_parity;
mod shellcheck_parity;
mod symlinked_root;
mod typescript_parity;
