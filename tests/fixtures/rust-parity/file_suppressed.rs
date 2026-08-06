#![allow(clippy::needless_return)]
//! An inner attribute exempts the whole crate root.

pub fn early() -> u32 {
    return 1;
}

pub fn later() -> u32 {
    return 2;
}

// llmlint: ignore-file[suppressions_justified] the missing reason is the point: this fixture is the input that proves notignored reports an unjustified suppression, and the parity test asserts its `reason` comes back null.
