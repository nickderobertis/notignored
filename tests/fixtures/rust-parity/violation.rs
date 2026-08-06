//! Both lints the parity fixtures hinge on fire here, unsuppressed:
//! `clippy::needless_return` on `early`, and `dead_code` on `unused_helper`.

pub fn early() -> u32 {
    return 1;
}

fn unused_helper() -> u32 {
    2
}
