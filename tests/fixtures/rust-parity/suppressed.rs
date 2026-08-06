//! The same two lints, each silenced by the attribute form under test.

#[allow(clippy::needless_return)]
pub fn early() -> u32 {
    return 1;
}

#[expect(dead_code, reason = "kept for the 1.0 surface, wired up next release")]
fn unused_helper() -> u32 {
    2
}

// llmlint: ignore-file[suppressions_justified] the missing reason is the
// point: this fixture is the input that proves notignored reports an
// unjustified suppression, and the parity test asserts its `reason` comes back
// null.
