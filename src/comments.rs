//! Language-aware comment and attribute extraction.
//!
//! Tool parsers consume the output of this module rather than regexing raw
//! lines, so a `# noqa` *inside a string literal* is never mistaken for a
//! directive. The scanners are deliberately biased toward **missing** an exotic
//! comment over **inventing** one: a false comment becomes a false suppression
//! report, which is the failure mode that erodes trust in the tool.
//!
//! Known simplifications, all in the safe direction:
//!
//! * A JavaScript regex literal containing `//` (`/a\/\/b/`) is scanned as code,
//!   so its contents could read as a line comment. No tool directive is
//!   expressible there in practice.
//! * A `${…}` interpolation inside a template literal is scanned as string
//!   content, so a comment nested in one is skipped.
//! * Shell here-documents are scanned as code, so a `#` inside one reads as a
//!   comment.

use crate::source::Language;

/// Whether a comment was introduced by a line marker or a block delimiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    /// Runs from its marker to the end of the line (`#`, `//`).
    Line,
    /// Delimited, and possibly spanning several lines (`/* … */`).
    Block,
}

/// One comment, with a 1-based span.
///
/// `end_column` is **exclusive**: it is the column just past the comment's last
/// character, so a comment occupying columns 1..=6 has `column == 1` and
/// `end_column == 7`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Line or block.
    pub kind: CommentKind,
    /// The comment exactly as written, delimiters included.
    pub raw: String,
    /// The comment body with its delimiters stripped, otherwise untouched.
    pub text: String,
    /// 1-based line of the opening delimiter.
    pub line: u32,
    /// 1-based column of the opening delimiter.
    pub column: u32,
    /// 1-based line of the closing delimiter (or of the line end).
    pub end_line: u32,
    /// 1-based column just past the comment's last character.
    pub end_column: u32,
    /// True when only whitespace precedes the comment on its opening line.
    pub leading: bool,
}

/// One Rust attribute (`#[…]` or `#![…]`), with a 1-based span.
///
/// Attributes are extracted alongside comments because Rust's own suppressions
/// (`#[allow(dead_code)]`) are attributes, not comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// The attribute exactly as written, brackets included.
    pub raw: String,
    /// The contents between the brackets.
    pub text: String,
    /// True for the inner form (`#![…]`), which applies to the enclosing item.
    pub inner: bool,
    /// 1-based line of the leading `#`.
    pub line: u32,
    /// 1-based column of the leading `#`.
    pub column: u32,
    /// 1-based line of the closing `]`.
    pub end_line: u32,
    /// 1-based column just past the closing `]`.
    pub end_column: u32,
}

/// Everything one scan of a file produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extracted {
    /// Comments, in source order.
    pub comments: Vec<Comment>,
    /// Rust attributes, in source order. Empty for every other language.
    pub attributes: Vec<Attribute>,
}

/// Extract every comment (and, for Rust, every attribute) from `source`.
///
/// An unknown language yields nothing rather than guessing a grammar.
pub fn extract(source: &str, language: Language) -> Extracted {
    let chars: Vec<char> = source.chars().collect();
    match language {
        Language::Python | Language::Toml => scan_hash(&chars, HASH_PYTHON),
        Language::Shell | Language::Yaml => scan_hash(&chars, HASH_WORD),
        Language::Rust => scan_c_style(&chars, C_RUST),
        Language::JavaScript | Language::TypeScript => scan_c_style(&chars, C_SCRIPT),
        Language::Unknown => Extracted::default(),
    }
}

/// Dialect knobs for `#`-comment languages.
#[derive(Debug, Clone, Copy)]
struct HashSyntax {
    /// `#` only opens a comment at a word boundary — start of input, or after
    /// whitespace or a shell operator. True for shell and YAML, where `${x#y}`
    /// and `a#b` are ordinary words; false for Python and TOML.
    word_boundary: bool,
    /// Triple-quoted multi-line strings exist (Python, TOML).
    triple_quotes: bool,
    /// A backslash escapes the delimiter inside a single-quoted string. False
    /// for shell and YAML, where `'` runs literally to the next `'`.
    escape_in_single: bool,
}

const HASH_PYTHON: HashSyntax = HashSyntax {
    word_boundary: false,
    triple_quotes: true,
    escape_in_single: true,
};

const HASH_WORD: HashSyntax = HashSyntax {
    word_boundary: true,
    triple_quotes: false,
    escape_in_single: false,
};

/// Dialect knobs for `//` + `/* … */` languages.
#[derive(Debug, Clone, Copy)]
struct CSyntax {
    /// `/*` nests (Rust) rather than ending at the first `*/` (JS/TS).
    nested_block: bool,
    /// Backtick template literals exist (JS/TS).
    backtick_strings: bool,
    /// Rust raw strings (`r#"…"#`) exist.
    raw_strings: bool,
    /// `'` opens a char literal *or* a lifetime (Rust) rather than a string.
    char_literals: bool,
    /// `#[…]` / `#![…]` attributes exist (Rust).
    attributes: bool,
}

const C_RUST: CSyntax = CSyntax {
    nested_block: true,
    backtick_strings: false,
    raw_strings: true,
    char_literals: true,
    attributes: true,
};

const C_SCRIPT: CSyntax = CSyntax {
    nested_block: false,
    backtick_strings: true,
    raw_strings: false,
    char_literals: false,
    attributes: false,
};

/// A character cursor that tracks the 1-based line and column as it advances.
struct Cursor<'a> {
    chars: &'a [char],
    index: usize,
    line: u32,
    column: u32,
}

impl<'a> Cursor<'a> {
    fn new(chars: &'a [char]) -> Self {
        Cursor {
            chars,
            index: 0,
            line: 1,
            column: 1,
        }
    }

    fn eof(&self) -> bool {
        self.index >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index + offset).copied()
    }

    fn matches(&self, needle: &str) -> bool {
        needle
            .chars()
            .enumerate()
            .all(|(offset, expected)| self.peek_at(offset) == Some(expected))
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn bump_n(&mut self, count: usize) {
        for _ in 0..count {
            if self.bump().is_none() {
                break;
            }
        }
    }

    /// Consume a delimited run that started at the cursor, honouring backslash
    /// escapes when `escapes`. Stops after the closing delimiter, or at EOF for
    /// an unterminated literal.
    fn skip_delimited(&mut self, delimiter: &str, escapes: bool) {
        self.bump_n(delimiter.chars().count());
        while !self.eof() {
            if escapes && self.peek() == Some('\\') {
                self.bump_n(2);
                continue;
            }
            if self.matches(delimiter) {
                self.bump_n(delimiter.chars().count());
                return;
            }
            self.bump();
        }
    }
}

/// Collect a line comment starting at the cursor, whose marker is `marker`.
fn take_line_comment(cursor: &mut Cursor<'_>, marker: &str, leading: bool) -> Comment {
    let (line, column) = (cursor.line, cursor.column);
    let mut raw = String::new();
    while let Some(ch) = cursor.peek() {
        if ch == '\n' {
            break;
        }
        raw.push(ch);
        cursor.bump();
    }
    // A CRLF file leaves the `\r` on the comment; it is not part of the text.
    while raw.ends_with('\r') {
        raw.pop();
    }
    let text = raw[marker.len()..].to_string();
    let width = u32::try_from(raw.chars().count()).unwrap_or(u32::MAX);
    Comment {
        kind: CommentKind::Line,
        raw,
        text,
        line,
        column,
        end_line: line,
        end_column: column.saturating_add(width),
        leading,
    }
}

/// Collect a `/* … */` comment starting at the cursor. `nested` allows Rust's
/// nesting; an unterminated comment ends at EOF.
fn take_block_comment(cursor: &mut Cursor<'_>, nested: bool, leading: bool) -> Comment {
    let (line, column) = (cursor.line, cursor.column);
    let mut raw = String::from("/*");
    cursor.bump_n(2);
    let mut depth = 1usize;
    while !cursor.eof() {
        if nested && cursor.matches("/*") {
            depth += 1;
            raw.push_str("/*");
            cursor.bump_n(2);
            continue;
        }
        if cursor.matches("*/") {
            depth -= 1;
            raw.push_str("*/");
            cursor.bump_n(2);
            if depth == 0 {
                break;
            }
            continue;
        }
        if let Some(ch) = cursor.bump() {
            raw.push(ch);
        }
    }
    let text = raw
        .strip_prefix("/*")
        .and_then(|rest| rest.strip_suffix("*/").or(Some(rest)))
        .unwrap_or_default()
        .to_string();
    Comment {
        kind: CommentKind::Block,
        raw,
        text,
        line,
        column,
        end_line: cursor.line,
        end_column: cursor.column,
        leading,
    }
}

/// True when `#` here opens a comment rather than continuing a word.
fn at_word_boundary(previous: Option<char>) -> bool {
    match previous {
        None => true,
        Some(ch) => ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '<' | '>'),
    }
}

fn scan_hash(chars: &[char], syntax: HashSyntax) -> Extracted {
    let mut out = Extracted::default();
    let mut cursor = Cursor::new(chars);
    let mut previous: Option<char> = None;
    let mut line_has_content = false;

    while let Some(ch) = cursor.peek() {
        match ch {
            '\n' => {
                line_has_content = false;
                previous = cursor.bump();
            }
            '#' if !syntax.word_boundary || at_word_boundary(previous) => {
                let comment = take_line_comment(&mut cursor, "#", !line_has_content);
                out.comments.push(comment);
                line_has_content = true;
                previous = Some('#');
            }
            '\\' => {
                cursor.bump_n(2);
                line_has_content = true;
                previous = Some('\\');
            }
            '"' | '\'' => {
                let triple = if ch == '"' { "\"\"\"" } else { "'''" };
                let single = if ch == '"' { "\"" } else { "'" };
                let escapes = ch == '"' || syntax.escape_in_single;
                if syntax.triple_quotes && cursor.matches(triple) {
                    cursor.skip_delimited(triple, escapes);
                } else {
                    cursor.skip_delimited(single, escapes);
                }
                line_has_content = true;
                previous = Some(ch);
            }
            other => {
                if !other.is_whitespace() {
                    line_has_content = true;
                }
                previous = cursor.bump();
            }
        }
    }
    out
}

fn scan_c_style(chars: &[char], syntax: CSyntax) -> Extracted {
    let mut out = Extracted::default();
    let mut cursor = Cursor::new(chars);
    let mut line_has_content = false;
    let mut previous: Option<char> = None;

    while let Some(ch) = cursor.peek() {
        if ch == '\n' {
            line_has_content = false;
            previous = cursor.bump();
            continue;
        }
        if cursor.matches("//") {
            let comment = take_line_comment(&mut cursor, "//", !line_has_content);
            out.comments.push(comment);
            line_has_content = true;
            previous = Some('/');
            continue;
        }
        if cursor.matches("/*") {
            let comment = take_block_comment(&mut cursor, syntax.nested_block, !line_has_content);
            out.comments.push(comment);
            line_has_content = true;
            previous = Some('/');
            continue;
        }
        if syntax.attributes && ch == '#' && matches!(cursor.peek_at(1), Some('[') | Some('!')) {
            if let Some(attribute) = take_attribute(&mut cursor) {
                out.attributes.push(attribute);
                line_has_content = true;
                previous = Some(']');
                continue;
            }
        }
        if syntax.raw_strings
            && ch == 'r'
            && !is_ident_char(previous)
            && skip_raw_string(&mut cursor)
        {
            line_has_content = true;
            previous = Some('"');
            continue;
        }
        if syntax.raw_strings
            && ch == 'b'
            && cursor.peek_at(1) == Some('r')
            && !is_ident_char(previous)
        {
            cursor.bump();
            skip_raw_string(&mut cursor);
            line_has_content = true;
            previous = Some('"');
            continue;
        }
        if ch == '"' {
            cursor.skip_delimited("\"", true);
            line_has_content = true;
            previous = Some('"');
            continue;
        }
        if ch == '`' && syntax.backtick_strings {
            cursor.skip_delimited("`", true);
            line_has_content = true;
            previous = Some('`');
            continue;
        }
        if ch == '\'' {
            if syntax.char_literals {
                skip_char_literal(&mut cursor);
            } else {
                cursor.skip_delimited("'", true);
            }
            line_has_content = true;
            previous = Some('\'');
            continue;
        }
        if !ch.is_whitespace() {
            line_has_content = true;
        }
        previous = cursor.bump();
    }
    out
}

fn is_ident_char(previous: Option<char>) -> bool {
    previous.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Consume a Rust raw string (`r"…"`, `r#"…"#`, …) starting at the `r`.
///
/// Returns false without consuming anything when the `r` is just an identifier.
fn skip_raw_string(cursor: &mut Cursor<'_>) -> bool {
    let mut hashes = 0usize;
    while cursor.peek_at(1 + hashes) == Some('#') {
        hashes += 1;
    }
    if cursor.peek_at(1 + hashes) != Some('"') {
        return false;
    }
    cursor.bump_n(2 + hashes);
    let terminator: String = std::iter::once('"')
        .chain(std::iter::repeat_n('#', hashes))
        .collect();
    while !cursor.eof() {
        if cursor.matches(&terminator) {
            cursor.bump_n(terminator.chars().count());
            return true;
        }
        cursor.bump();
    }
    true
}

/// Consume a Rust char literal, or just the `'` when it opens a lifetime.
fn skip_char_literal(cursor: &mut Cursor<'_>) {
    if cursor.peek_at(1) == Some('\\') {
        cursor.skip_delimited("'", true);
        return;
    }
    if cursor.peek_at(2) == Some('\'') {
        cursor.bump_n(3);
        return;
    }
    cursor.bump();
}

/// Consume a Rust attribute starting at `#`, balancing nested brackets and
/// skipping string literals inside. Returns `None` when the `#` is not followed
/// by an attribute opener.
fn take_attribute(cursor: &mut Cursor<'_>) -> Option<Attribute> {
    let inner = cursor.peek_at(1) == Some('!');
    let bracket_offset = if inner { 2 } else { 1 };
    if cursor.peek_at(bracket_offset) != Some('[') {
        return None;
    }
    let (line, column) = (cursor.line, cursor.column);
    let mut raw = String::new();
    for _ in 0..bracket_offset {
        if let Some(ch) = cursor.bump() {
            raw.push(ch);
        }
    }
    let mut depth = 0usize;
    while !cursor.eof() {
        match cursor.peek() {
            Some('[') => {
                depth += 1;
                raw.push('[');
                cursor.bump();
            }
            Some(']') => {
                depth -= 1;
                raw.push(']');
                cursor.bump();
                if depth == 0 {
                    break;
                }
            }
            Some('"') => {
                let start = cursor.index;
                cursor.skip_delimited("\"", true);
                raw.extend(&cursor.chars[start..cursor.index]);
            }
            Some(_) => {
                if let Some(ch) = cursor.bump() {
                    raw.push(ch);
                }
            }
            None => break,
        }
    }
    let opener = if inner { "#![" } else { "#[" };
    let text = raw
        .strip_prefix(opener)
        .and_then(|rest| rest.strip_suffix(']').or(Some(rest)))
        .unwrap_or_default()
        .to_string();
    Some(Attribute {
        raw,
        text,
        inner,
        line,
        column,
        end_line: cursor.line,
        end_column: cursor.column,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comments(source: &str, language: Language) -> Vec<Comment> {
        extract(source, language).comments
    }

    #[test]
    fn hash_comments_carry_one_based_spans() {
        let found = comments("x = 1  # noqa: E501\n", Language::Python);
        assert_eq!(found.len(), 1);
        let comment = &found[0];
        assert_eq!(comment.kind, CommentKind::Line);
        assert_eq!(comment.raw, "# noqa: E501");
        assert_eq!(comment.text, " noqa: E501");
        assert_eq!((comment.line, comment.column), (1, 8));
        assert_eq!((comment.end_line, comment.end_column), (1, 20));
        assert!(!comment.leading);
    }

    #[test]
    fn a_comment_alone_on_its_line_is_leading() {
        let found = comments("   # ruff: noqa\nx = 1  # noqa\n", Language::Python);
        assert!(found[0].leading);
        assert_eq!(found[0].column, 4);
        assert!(!found[1].leading);
        assert_eq!(found[1].line, 2);
    }

    #[test]
    fn python_string_literals_are_not_comments() {
        let source = "a = \"# noqa\"\nb = '# noqa'\nc = \"\"\"\n# noqa\n\"\"\"\nd = 1  # real\n";
        let found = comments(source, Language::Python);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].raw, "# real");
        assert_eq!(found[0].line, 6);
    }

    #[test]
    fn python_escaped_quotes_do_not_end_a_string_early() {
        let found = comments("a = \"\\\"# noqa\"\nb = 2  # real\n", Language::Python);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].raw, "# real");
    }

    #[test]
    fn an_unterminated_python_string_swallows_the_rest() {
        let found = comments("a = \"oops\nb = 1  # noqa\n", Language::Python);
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn shell_needs_a_word_boundary_before_a_hash() {
        let source = "echo ${x#prefix}\nrm -f a#b   # real\n";
        let found = comments(source, Language::Shell);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].raw, "# real");
        assert_eq!(found[0].line, 2);
    }

    #[test]
    fn shell_single_quotes_ignore_backslashes() {
        let found = comments("echo '\\' # real\n", Language::Shell);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].raw, "# real");
    }

    #[test]
    fn yaml_comments_and_scalars_are_distinguished() {
        let found = comments("key: value#notacomment # real\n", Language::Yaml);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].raw, "# real");
    }

    #[test]
    fn toml_triple_quoted_strings_hide_hashes() {
        let found = comments("a = \"\"\"\n# not\n\"\"\"  # real\n", Language::Toml);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].raw, "# real");
    }

    #[test]
    fn escaped_hashes_outside_strings_are_skipped() {
        let found = comments("echo \\# not a comment  # real\n", Language::Shell);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].raw, "# real");
    }

    #[test]
    fn line_comments_in_c_style_languages_carry_spans() {
        let found = comments(
            "const a = 1; // eslint-disable-line\n",
            Language::JavaScript,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].raw, "// eslint-disable-line");
        assert_eq!(found[0].text, " eslint-disable-line");
        assert_eq!((found[0].line, found[0].column), (1, 14));
        assert_eq!(found[0].end_column, 36);
    }

    #[test]
    fn multi_line_block_comments_span_lines_and_columns() {
        let source = "  /* one\n     two */ const a = 1;\n";
        let found = comments(source, Language::TypeScript);
        assert_eq!(found.len(), 1);
        let comment = &found[0];
        assert_eq!(comment.kind, CommentKind::Block);
        assert_eq!(comment.raw, "/* one\n     two */");
        assert_eq!(comment.text, " one\n     two ");
        assert_eq!((comment.line, comment.column), (1, 3));
        assert_eq!((comment.end_line, comment.end_column), (2, 12));
        assert!(comment.leading);
    }

    #[test]
    fn an_unterminated_block_comment_ends_at_eof() {
        let found = comments("/* forever\nand ever\n", Language::JavaScript);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end_line, 3);
        assert!(found[0].raw.ends_with("and ever\n"));
    }

    #[test]
    fn javascript_strings_and_templates_are_not_comments() {
        let source =
            "const a = \"// no\";\nconst b = '/* no */';\nconst c = `// no ${x}`;\n// real\n";
        let found = comments(source, Language::JavaScript);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].raw, "// real");
        assert_eq!(found[0].line, 4);
    }

    #[test]
    fn rust_block_comments_nest() {
        let source = "/* outer /* inner */ still outer */\nlet a = 1; // real\n";
        let found = comments(source, Language::Rust);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].raw, "/* outer /* inner */ still outer */");
        assert_eq!(found[1].raw, "// real");
    }

    #[test]
    fn javascript_block_comments_do_not_nest() {
        let source = "/* outer /* inner */\n// real\n";
        let found = comments(source, Language::JavaScript);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].raw, "/* outer /* inner */");
    }

    #[test]
    fn rust_raw_strings_and_lifetimes_do_not_confuse_the_scanner() {
        let source = concat!(
            "let a = r#\"// no\"#;\n",
            "let b = br#\"/* no */\"#;\n",
            "let c = r\"// no\";\n",
            "fn f<'a>(x: &'a str) -> char { '/' }\n",
            "let d = '\\'';\n",
            "let raw = 1; // real\n",
        );
        let found = comments(source, Language::Rust);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].raw, "// real");
        assert_eq!(found[0].line, 6);
    }

    #[test]
    fn an_unterminated_rust_raw_string_swallows_the_rest() {
        let found = comments("let a = r#\"oops\n// no\n", Language::Rust);
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn rust_attributes_are_extracted_with_spans() {
        let source = "#![allow(dead_code)]\n\n#[allow(clippy::all, unused)]\nfn f() {}\n";
        let extracted = extract(source, Language::Rust);
        assert!(extracted.comments.is_empty());
        assert_eq!(extracted.attributes.len(), 2);

        let inner = &extracted.attributes[0];
        assert!(inner.inner);
        assert_eq!(inner.raw, "#![allow(dead_code)]");
        assert_eq!(inner.text, "allow(dead_code)");
        assert_eq!((inner.line, inner.column), (1, 1));
        assert_eq!((inner.end_line, inner.end_column), (1, 21));

        let outer = &extracted.attributes[1];
        assert!(!outer.inner);
        assert_eq!(outer.text, "allow(clippy::all, unused)");
        assert_eq!(outer.line, 3);
    }

    #[test]
    fn attributes_survive_nested_brackets_and_strings() {
        let source = "#[doc = \"has ] bracket\"]\n#[cfg_attr(test, allow(a[0]))]\nfn f() {}\n";
        let extracted = extract(source, Language::Rust);
        assert_eq!(extracted.attributes.len(), 2);
        assert_eq!(extracted.attributes[0].text, "doc = \"has ] bracket\"");
        assert_eq!(extracted.attributes[1].text, "cfg_attr(test, allow(a[0]))");
    }

    #[test]
    fn a_bare_hash_in_rust_is_not_an_attribute() {
        let extracted = extract("let a = 1; # oops\n", Language::Rust);
        assert!(extracted.attributes.is_empty());
        assert!(extracted.comments.is_empty());
    }

    #[test]
    fn an_unterminated_attribute_ends_at_eof() {
        let extracted = extract("#[allow(dead_code)\n", Language::Rust);
        assert_eq!(extracted.attributes.len(), 1);
        assert_eq!(extracted.attributes[0].raw, "#[allow(dead_code)\n");
    }

    #[test]
    fn crlf_line_endings_do_not_leak_into_comment_text() {
        let found = comments("x = 1  # noqa\r\ny = 2\r\n", Language::Python);
        assert_eq!(found[0].raw, "# noqa");
    }

    #[test]
    fn unknown_languages_yield_nothing() {
        assert_eq!(extract("# noqa\n", Language::Unknown), Extracted::default());
    }
}
