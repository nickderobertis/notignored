#![allow(clippy::needless_return)]
//! The Rust attribute forms, as the README advertises them.

#[allow(dead_code, clippy::needless_collect)]
fn collected() -> usize {
    let seen: Vec<u32> = (0..3).collect();
    seen.len()
}

#[expect(
    dead_code,
    reason = "a justification long enough that it wraps
              across two lines of the attribute"
)]
struct Shim {
    field: u32,
}

pub const DECOY: &str = "#[allow(dead_code)] inside a string literal";

// llmlint: ignore-file[suppressions_justified] the missing reason is the
// point: this fixture is the input that proves notignored reports an
// unjustified suppression, and the parity test asserts its `reason` comes back
// null.
