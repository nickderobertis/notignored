//! A reader for the YAML subset this repo's workflows and composite action are
//! written in — block mappings, block sequences, block scalars, and one-line
//! `[a, b]` flow sequences.
//!
//! `action.yml`, `.github/workflows/notignored.yml`, and
//! `.github/workflows/release.yml` are only ever exercised for real inside
//! GitHub Actions, where a mistake costs a red release and a round trip. The
//! contract tests read them the way the runner does — as structure, not as text
//! — so a renamed input, a step that forgot its shell, a job that lost its
//! `needs`, or an untrusted event value spliced into a script fails the build
//! instead of a run.
//!
//! The parser is deliberately strict: anything it cannot make sense of is a
//! panic naming the line, which is the syntax check a hand-written workflow
//! otherwise never gets. That strictness is what makes adding a file here
//! structural validation rather than a `contains` check.
//!
//! Included with `#[path]` by every contract test that needs it, so there is one
//! parser rather than one per test binary.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// The repository root (the directory holding `Cargo.toml`).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A repository file, or a panic naming the one that could not be read.
pub fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// A parsed YAML node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Scalar(String),
    Map(BTreeMap<String, Node>),
    List(Vec<Node>),
}

impl Node {
    /// The value at `key`, or a panic naming what was actually there.
    pub fn get(&self, key: &str) -> &Node {
        match self {
            Node::Map(entries) => entries
                .get(key)
                .unwrap_or_else(|| panic!("no `{key}` here; found {:?}", entries.keys())),
            other => panic!("`{key}` was looked up in a {other:?}"),
        }
    }

    /// The value at `key`, if there is one.
    pub fn find(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(entries) => entries.get(key),
            _ => None,
        }
    }

    /// This node's keys, in order.
    pub fn keys(&self) -> Vec<&str> {
        match self {
            Node::Map(entries) => entries.keys().map(String::as_str).collect(),
            other => panic!("expected a mapping, got {other:?}"),
        }
    }

    pub fn scalar(&self) -> &str {
        match self {
            Node::Scalar(value) => value,
            other => panic!("expected a scalar, got {other:?}"),
        }
    }

    pub fn list(&self) -> &[Node] {
        match self {
            Node::List(items) => items,
            other => panic!("expected a sequence, got {other:?}"),
        }
    }
}

/// A line that carries content, with its indentation.
struct Line<'a> {
    indent: usize,
    text: &'a str,
    number: usize,
}

/// Every line that is neither blank nor a comment.
fn meaningful(text: &str) -> Vec<Line<'_>> {
    text.lines()
        .enumerate()
        .filter_map(|(index, raw)| {
            let trimmed = raw.trim_start();
            (!trimmed.is_empty() && !trimmed.starts_with('#')).then(|| Line {
                indent: raw.len() - trimmed.len(),
                text: trimmed.trim_end(),
                number: index + 1,
            })
        })
        .collect()
}

/// Parse the YAML subset these files use.
pub fn parse(text: &str) -> Node {
    let lines = meaningful(text);
    let mut cursor = 0;
    let node = parse_block(
        &lines,
        &mut cursor,
        lines.first().map_or(0, |line| line.indent),
    );
    assert_eq!(
        cursor,
        lines.len(),
        "line {} is not part of the document",
        lines[cursor].number
    );
    node
}

/// The block starting at `cursor`, made of every line indented by `indent`.
fn parse_block(lines: &[Line<'_>], cursor: &mut usize, indent: usize) -> Node {
    if lines
        .get(*cursor)
        .is_some_and(|line| line.text.starts_with("- ") || line.text == "-")
    {
        let mut items = Vec::new();
        while lines
            .get(*cursor)
            .is_some_and(|line| line.indent == indent && line.text.starts_with("- "))
        {
            let inline = lines[*cursor].text[2..].trim().to_string();
            *cursor += 1;
            items.push(parse_entries(lines, cursor, indent + 2, Some(inline)));
        }
        return Node::List(items);
    }
    parse_entries(lines, cursor, indent, None)
}

/// A mapping — optionally opened by `inline`, the text that followed a `- `.
fn parse_entries(
    lines: &[Line<'_>],
    cursor: &mut usize,
    indent: usize,
    inline: Option<String>,
) -> Node {
    let mut entries = BTreeMap::new();
    if let Some(first) = inline {
        // A sequence item that is a plain scalar rather than a mapping.
        let Some((key, rest)) = split_entry(&first) else {
            return Node::Scalar(unquote(&first));
        };
        insert(&mut entries, key, rest, lines, cursor, indent);
    }
    while let Some(line) = lines.get(*cursor) {
        if line.indent < indent || line.text.starts_with("- ") {
            break;
        }
        assert_eq!(
            line.indent, indent,
            "line {} is indented {} where {indent} was expected",
            line.number, line.indent
        );
        let text = line.text.to_string();
        let (key, rest) = split_entry(&text)
            .unwrap_or_else(|| panic!("line {} is not a `key: value` entry", line.number));
        *cursor += 1;
        insert(&mut entries, key, rest, lines, cursor, indent);
    }
    Node::Map(entries)
}

/// Record one mapping entry, reading its nested block when it has one.
fn insert(
    entries: &mut BTreeMap<String, Node>,
    key: String,
    rest: String,
    lines: &[Line<'_>],
    cursor: &mut usize,
    indent: usize,
) {
    let value = if rest.is_empty() {
        match lines.get(*cursor) {
            // A sequence may sit at its parent's own indentation.
            Some(line) if line.indent > indent => parse_block(lines, cursor, line.indent),
            Some(line) if line.indent == indent && line.text.starts_with("- ") => {
                parse_block(lines, cursor, indent)
            }
            _ => Node::Scalar(String::new()),
        }
    } else if matches!(rest.as_str(), "|" | ">" | "|-" | ">-" | "|+" | ">+") {
        Node::Scalar(block_scalar(lines, cursor, indent))
    } else if let Some(items) = flow_sequence(&rest) {
        Node::List(items)
    } else {
        Node::Scalar(unquote(&rest))
    };
    assert!(
        entries.insert(key.clone(), value).is_none(),
        "`{key}` is defined twice"
    );
}

/// A one-line `[a, b, c]` sequence, or `None` when the value is a plain scalar.
///
/// Workflows write short lists inline — `types: [published]`, `needs: [guard,
/// test]` — and a reader that returned those as the string `"[published]"` would
/// make a test comparing them to a list fail for the wrong reason. Only scalar
/// items are supported; nothing in this repo nests inside a flow sequence.
fn flow_sequence(value: &str) -> Option<Vec<Node>> {
    let inner = value.strip_prefix('[')?.strip_suffix(']')?;
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    Some(
        inner
            .split(',')
            .map(|item| Node::Scalar(unquote(item)))
            .collect(),
    )
}

/// The indented text under a `|` or `>` marker, joined with spaces.
fn block_scalar(lines: &[Line<'_>], cursor: &mut usize, indent: usize) -> String {
    let mut parts = Vec::new();
    while let Some(line) = lines.get(*cursor) {
        if line.indent <= indent {
            break;
        }
        parts.push(line.text.to_string());
        *cursor += 1;
    }
    parts.join(" ")
}

/// Split `key: value`, leaving a `://` inside a value alone.
fn split_entry(text: &str) -> Option<(String, String)> {
    let colon = text.find(':')?;
    let rest = &text[colon + 1..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((unquote(&text[..colon]), rest.trim().to_string()))
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// Every `run:` step in a composite action or a workflow job.
pub fn run_steps(steps: &[Node]) -> Vec<&Node> {
    steps
        .iter()
        .filter(|step| step.find("run").is_some())
        .collect()
}
