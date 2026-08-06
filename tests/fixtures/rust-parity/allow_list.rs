//! Several lints at once, including a tool-prefixed one, silenced by `allow`.

#[allow(dead_code, unused_variables, clippy::needless_return)]
fn unused_helper(value: u32) -> u32 {
    return 7;
}
