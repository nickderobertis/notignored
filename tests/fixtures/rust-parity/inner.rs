//! A crate-wide exemption: the inner form applies to the whole file.
// llmlint: ignore[suppressions_justified] the missing reason is the point: this fixture is the input that proves notignored reports an unjustified suppression, and the parity test asserts its `reason` comes back null.
#![allow(dead_code)]

fn unused_helper() -> u32 {
    7
}
