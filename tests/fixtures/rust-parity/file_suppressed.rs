#![allow(clippy::needless_return)]
//! An inner attribute exempts the whole crate root.

pub fn early() -> u32 {
    return 1;
}

pub fn later() -> u32 {
    return 2;
}
