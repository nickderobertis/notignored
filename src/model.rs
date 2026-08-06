//! The public record contract: what a suppression looks like once parsed.
//!
//! These types are a **versioned wire contract** consumed by downstream tooling
//! (review bots, CI jobs, the planned GitHub Action). Field names, the [`Scope`]
//! variants, and the [`Report`] envelope shape are fixed: changing one is a
//! breaking change that bumps [`REPORT_VERSION`] and moves the checked-in golden
//! report in the same commit. New fields must be optional and round-trip.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Version of the [`Report`] envelope.
///
/// Bump this whenever the serialized shape changes, and update
/// `tests/golden/report.json` in the same change.
pub const REPORT_VERSION: u32 = 1;

/// A lint or type-check tool whose suppression comments we understand.
///
/// The full planned set is fixed here so the registry, the `--tool` filter, and
/// the README table all agree; [`Tool::is_implemented`] reports which ones have
/// a parser today.
///
/// `ValueEnum` is derived here rather than in the CLI layer so `--tool --help`
/// lists the tools straight from the contract and cannot drift from it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    /// ESLint (`// eslint-disable-next-line`, …).
    Eslint,
    /// Biome (`// biome-ignore lint/…`).
    Biome,
    /// Ruff (`# noqa`, `# ruff: noqa`).
    Ruff,
    /// The TypeScript compiler (`// @ts-ignore`, `// @ts-expect-error`).
    Typescript,
    /// mypy (`# type: ignore`).
    Mypy,
    /// Pyright (`# pyright: ignore`).
    Pyright,
    /// ty (`# ty: ignore`).
    Ty,
    /// The Rust compiler and clippy (`#[allow(…)]`, `#[expect(…)]`).
    Rust,
    /// ShellCheck (`# shellcheck disable=SC2086`).
    Shellcheck,
    /// llmlint (its inline `ignore[rule] reason` directive).
    Llmlint,
}

impl Tool {
    /// Every tool in the contract, in a stable order.
    pub const ALL: [Tool; 10] = [
        Tool::Eslint,
        Tool::Biome,
        Tool::Ruff,
        Tool::Typescript,
        Tool::Mypy,
        Tool::Pyright,
        Tool::Ty,
        Tool::Rust,
        Tool::Shellcheck,
        Tool::Llmlint,
    ];

    /// The tool's name as it appears in reports and on the `--tool` flag.
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::Eslint => "eslint",
            Tool::Biome => "biome",
            Tool::Ruff => "ruff",
            Tool::Typescript => "typescript",
            Tool::Mypy => "mypy",
            Tool::Pyright => "pyright",
            Tool::Ty => "ty",
            Tool::Rust => "rust",
            Tool::Shellcheck => "shellcheck",
            Tool::Llmlint => "llmlint",
        }
    }

    /// Whether a parser for this tool is registered today.
    ///
    /// Planned tools stay in [`Tool::ALL`] (and in the README table) so the
    /// contract is visible before the parser lands.
    pub fn is_implemented(self) -> bool {
        crate::tools::registry().iter().any(|p| p.tool() == self)
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error returned when a `--tool` value names no known tool.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown tool {name:?} (known tools: {})", known_tools())]
pub struct ParseToolError {
    /// The unrecognized name, as the user wrote it.
    pub name: String,
}

fn known_tools() -> String {
    Tool::ALL
        .iter()
        .map(|t| t.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl FromStr for Tool {
    type Err = ParseToolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Tool::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| ParseToolError {
                name: s.to_string(),
            })
    }
}

/// How far a directive's suppression reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// Silences rules on the line the directive sits on.
    Line,
    /// Silences rules on the line *after* the directive.
    NextLine,
    /// Silences rules for the whole file.
    File,
    /// Silences rules over an explicitly delimited region.
    Block,
}

impl Scope {
    /// The scope's name as it appears in reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Line => "line",
            Scope::NextLine => "next-line",
            Scope::File => "file",
            Scope::Block => "block",
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The best-effort range of source lines a directive silences.
// llmlint: ignore[invalid_states_unrepresentable] plain 1-based coordinates are the fixed public contract; see "The ignore-record contract" in AGENTS.md for why they are not newtyped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suppressed {
    /// First 1-based line the directive silences.
    pub start_line: u32,
    /// Last 1-based line, or `None` when the range runs to end-of-file or is
    /// unterminated.
    pub end_line: Option<u32>,
}

/// One parsed suppression comment.
///
/// Serializes to the documented record shape; see the module docs before
/// touching field names or order.
// llmlint: ignore[invalid_states_unrepresentable] plain 1-based coordinates are the fixed public contract; see "The ignore-record contract" in AGENTS.md for why they are not newtyped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoreDirective {
    /// The tool whose rules are being silenced.
    pub tool: Tool,
    /// How far the suppression reaches.
    pub scope: Scope,
    /// Rule names/codes exactly as written. Empty means a blanket suppression
    /// of every rule the tool would apply.
    pub rules: Vec<String>,
    /// The stated justification, or `None` when none was given. Whitespace is
    /// collapsed to single spaces and trimmed.
    pub reason: Option<String>,
    /// Path to the file, relative to the invocation directory, `/`-separated.
    pub path: String,
    /// 1-based line the directive starts on.
    pub line: u32,
    /// 1-based line the directive ends on (equal to `line` unless the directive
    /// spans several lines).
    pub end_line: u32,
    /// 1-based column the directive starts at.
    pub column: u32,
    /// The directive exactly as it appears in the source, delimiters included.
    pub raw: String,
    /// The range of lines this directive silences.
    pub suppressed: Suppressed,
}

/// A file that could not be read, or a directive that could not be parsed.
///
/// Malformed input is reported, never panicked on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportError {
    /// Path the problem was found at, `/`-separated.
    pub path: String,
    /// What went wrong, in one line.
    pub message: String,
}

/// The report envelope: everything one scan produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Envelope version; see [`REPORT_VERSION`].
    ///
    /// Rejected at the trust boundary when a report claims a version this build
    /// does not understand: a newer envelope may carry fields these types drop,
    /// and silently parsing it would hand the caller a truncated report.
    #[serde(deserialize_with = "deserialize_version")]
    pub version: u32,
    /// Every directive found, ordered by path, then line, then column.
    pub ignores: Vec<IgnoreDirective>,
    /// Files that could not be read and directives that could not be parsed.
    pub errors: Vec<ReportError>,
}

fn deserialize_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = u32::deserialize(deserializer)?;
    if version > REPORT_VERSION {
        return Err(serde::de::Error::custom(format!(
            "report version {version} is newer than this build understands \
             ({REPORT_VERSION}); upgrade notignored"
        )));
    }
    Ok(version)
}

impl Report {
    /// An empty report stamped with the current [`REPORT_VERSION`].
    pub fn new() -> Self {
        Report {
            version: REPORT_VERSION,
            ignores: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Sort directives and errors into the documented, deterministic order.
    pub fn sort(&mut self) {
        self.ignores.sort_by(|a, b| {
            (&a.path, a.line, a.column, a.tool.as_str()).cmp(&(
                &b.path,
                b.line,
                b.column,
                b.tool.as_str(),
            ))
        });
        self.errors
            .sort_by(|a, b| (&a.path, &a.message).cmp(&(&b.path, &b.message)));
    }
}

impl Default for Report {
    fn default() -> Self {
        Report::new()
    }
}

/// Collapse runs of whitespace to single spaces and trim.
///
/// Reasons may be written across several lines of a block comment; this is how
/// they are joined into the single-line `reason` field.
pub fn normalize_reason(text: &str) -> Option<String> {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_round_trip_through_from_str() {
        for tool in Tool::ALL {
            assert_eq!(Tool::from_str(tool.as_str()), Ok(tool));
            assert_eq!(tool.to_string(), tool.as_str());
        }
    }

    #[test]
    fn unknown_tool_names_are_rejected_with_the_known_set() {
        let err = Tool::from_str("flake8").unwrap_err();
        assert_eq!(err.name, "flake8");
        let msg = err.to_string();
        assert!(msg.contains("flake8"), "{msg}");
        assert!(msg.contains("ruff"), "{msg}");
    }

    #[test]
    fn only_the_registered_tools_report_as_implemented() {
        assert!(Tool::Ruff.is_implemented());
        assert!(!Tool::Eslint.is_implemented());
    }

    #[test]
    fn scope_renders_kebab_case_in_json_and_display() {
        assert_eq!(
            serde_json::to_string(&Scope::NextLine).unwrap(),
            "\"next-line\""
        );
        assert_eq!(Scope::NextLine.to_string(), "next-line");
        assert_eq!(Scope::Line.as_str(), "line");
        assert_eq!(Scope::File.as_str(), "file");
        assert_eq!(Scope::Block.as_str(), "block");
    }

    #[test]
    fn reasons_collapse_whitespace_and_drop_empties() {
        assert_eq!(
            normalize_reason("  long   wrapped\n URL "),
            Some("long wrapped URL".into())
        );
        assert_eq!(normalize_reason("   \n  "), None);
    }

    #[test]
    fn a_future_envelope_version_is_rejected_at_the_boundary() {
        let future = format!(
            r#"{{"version": {}, "ignores": [], "errors": []}}"#,
            REPORT_VERSION + 1
        );
        let error = serde_json::from_str::<Report>(&future).unwrap_err();
        assert!(
            error.to_string().contains("newer than this build"),
            "{error}"
        );

        // The current version, and any older one, still parse.
        let current = format!(r#"{{"version": {REPORT_VERSION}, "ignores": [], "errors": []}}"#);
        assert_eq!(
            serde_json::from_str::<Report>(&current).unwrap(),
            Report::new()
        );
    }

    #[test]
    fn sort_orders_by_path_then_position() {
        let mut report = Report::default();
        report.ignores.push(directive("b.py", 1, 1));
        report.ignores.push(directive("a.py", 9, 1));
        report.ignores.push(directive("a.py", 2, 7));
        report.ignores.push(directive("a.py", 2, 3));
        report.errors.push(ReportError {
            path: "z.py".into(),
            message: "b".into(),
        });
        report.errors.push(ReportError {
            path: "a.py".into(),
            message: "a".into(),
        });
        report.sort();

        let order: Vec<_> = report
            .ignores
            .iter()
            .map(|d| (d.path.as_str(), d.line, d.column))
            .collect();
        assert_eq!(
            order,
            vec![
                ("a.py", 2, 3),
                ("a.py", 2, 7),
                ("a.py", 9, 1),
                ("b.py", 1, 1)
            ]
        );
        assert_eq!(report.errors[0].path, "a.py");
        assert_eq!(report.version, REPORT_VERSION);
    }

    fn directive(path: &str, line: u32, column: u32) -> IgnoreDirective {
        IgnoreDirective {
            tool: Tool::Ruff,
            scope: Scope::Line,
            rules: vec![],
            reason: None,
            path: path.to_string(),
            line,
            end_line: line,
            column,
            raw: "# noqa".to_string(),
            suppressed: Suppressed {
                start_line: line,
                end_line: Some(line),
            },
        }
    }
}
