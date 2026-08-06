//! llmlint scans raw lines, so it reads the directive inside this string
//! literal and rejects the unknown rule it names. notignored extracts comments
//! first, so it never sees a directive here at all.
pub fn decoy() -> &'static str {
    "// llmlint: ignore[not_a_rule] inside a string literal"
}
