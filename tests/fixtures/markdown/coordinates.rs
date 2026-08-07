// llmlint: ignore[invalid_states_unrepresentable] fixture input, not production code: this directive has its own line, so the comment body must show the derive below it (tests/e2e/markdown.rs)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}
