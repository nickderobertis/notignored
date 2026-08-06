//! End-to-end journeys: the compiled binary, real files, and the real linters
//! and type checkers it claims parity with.
//!
//! Split into its own test target so `just test-e2e` can run the slow binary
//! journeys alone — but `just check` runs them either way. Nothing here is
//! `#[ignore]`d: an opt-in e2e is coverage that quietly stops happening.

mod support;

mod biome_parity;
mod cli;
mod diff;
mod eslint_parity;
mod installer;
mod markdown;
mod python_tools_setup;
mod python_types_parity;
mod ruff_grammar;
mod ruff_parity;
mod rust_parity;
mod typescript_parity;
