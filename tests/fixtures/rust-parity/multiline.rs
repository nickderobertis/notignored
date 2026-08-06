//! The same suppression, written across several lines.

#[expect(
    dead_code,
    reason = "kept until the C API lands"
)]
fn unused_helper() -> u32 {
    7
}
