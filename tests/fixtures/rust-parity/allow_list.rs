//! Several lints at once, including a tool-prefixed one, silenced by `allow`.

// llmlint: ignore[suppressions_justified] the missing reason is the point: this fixture is the input that proves notignored reports an unjustified suppression, and the parity test asserts its `reason` comes back null.
#[allow(dead_code, unused_variables, clippy::needless_return)]
fn unused_helper(value: u32) -> u32 {
    return 7;
}
