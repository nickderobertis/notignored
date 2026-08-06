//! The same helper, with the lint expected rather than fixed.

#[expect(dead_code, reason = "kept until the C API lands")]
fn unused_helper() -> u32 {
    7
}
