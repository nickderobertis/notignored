//! Grouping fixture: one decision written twice, over spans of different
//! lengths and with different spacing inside the attribute. They group.

#[allow(dead_code, reason = "kept for the plugin API")]
fn register() {}

#[allow( dead_code , reason = "kept for the plugin API" )]
fn unregister() {
    let _ = register;
}
