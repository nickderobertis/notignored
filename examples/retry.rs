//! A retry budget, and the attribute an intentionally-unused helper earns.

/// How many attempts a call gets before it gives up.
pub const MAX_ATTEMPTS: u32 = 3;

#[expect(dead_code, reason = "the scheduler starts calling this once backoff lands")]
fn backoff_ms(attempt: u32) -> u64 {
    u64::from(attempt) * 250
}
