//! One suppression in a second language, so the smoke cannot pass on a single
//! parser.

#[expect(dead_code, reason = "the smoke fixture only has to parse, not link")]
pub fn unused() {}
