//! End-to-end journeys: the compiled binary, real files, and the real linters.
//!
//! Split into its own test target so `just test-e2e` can run the slow binary
//! journeys alone — but `just check` runs them either way. Nothing here is
//! `#[ignore]`d: an opt-in e2e is coverage that quietly stops happening.

mod support;

mod cli;
mod installer;
mod llmlint_parity;
mod ruff_grammar;
mod ruff_parity;
mod rust_parity;
mod shellcheck_parity;
