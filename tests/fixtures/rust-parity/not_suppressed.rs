//! Attributes that silence nothing here: `deny` raises the lint rather than
//! hiding it, and a `cfg_attr` allow is inactive outside `cfg(test)`.

#[deny(dead_code)]
#[cfg_attr(test, allow(dead_code))]
fn unused_helper() -> u32 {
    7
}
