#![allow(clippy::needless_return)]
//! The vendored grammar tables, kept in the shape upstream generates them.

// llmlint: ignore-file[comments_earn_their_place, suppressions_justified] a generated
// table: every name here is upstream's own, and the bare attributes below are the
// reason-less form the golden report pins as `reason: null`.

#[expect(
    dead_code,
    reason = "the next-token table is generated in full, and the parser only
              reaches half of it until error recovery lands"
)]
struct Tables {
    next: [u8; 4],
}

#[allow(dead_code, clippy::needless_collect)]
fn widths() -> usize {
    let seen: Vec<u32> = (0..3).collect();
    seen.len()
}

pub const DECOY: &str = "#[allow(dead_code)] inside a string literal";
