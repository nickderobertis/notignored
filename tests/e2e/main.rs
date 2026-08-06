//! End-to-end journeys: the compiled binary, real files, and the real linters
//! and type checkers it claims parity with.
//!
//! Split into its own test target so `just test-e2e` can run the slow binary
//! journeys alone — but `just check` runs them either way. Nothing here is
//! `#[ignore]`d: an opt-in e2e is coverage that quietly stops happening.

mod support;

mod cli;
mod installer;
mod python_types_parity;
mod ruff_grammar;
mod ruff_parity;
