//! The same two lints, each silenced by the attribute form under test.

#[allow(clippy::needless_return)]
pub fn early() -> u32 {
    return 1;
}

#[expect(dead_code, reason = "kept for the 1.0 surface, wired up next release")]
fn unused_helper() -> u32 {
    2
}
