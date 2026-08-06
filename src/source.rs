//! Source files: the unit a [`ToolParser`](crate::tools::ToolParser) consumes.
//!
//! A [`SourceFile`] extracts its comments (and, for Rust, its attributes) **once**
//! at construction. Parsers read that extraction; they never re-scan raw lines,
//! which is what keeps string-literal contents from being mistaken for
//! directives.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::comments::{self, Attribute, Comment, Extracted};

/// A source language, as far as comment syntax is concerned.
///
/// Languages are distinguished where the tool registry needs to tell them apart
/// (a TypeScript file is judged by `tsc`, a JavaScript one is not), even when
/// their comment syntax is identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// Python (`.py`, `.pyi`, `.pyw`).
    Python,
    /// Rust (`.rs`).
    Rust,
    /// JavaScript (`.js`, `.jsx`, `.mjs`, `.cjs`).
    JavaScript,
    /// TypeScript (`.ts`, `.tsx`, `.mts`, `.cts`).
    TypeScript,
    /// POSIX-ish shell (`.sh`, `.bash`, `.zsh`, `.ksh`).
    Shell,
    /// YAML (`.yml`, `.yaml`).
    Yaml,
    /// TOML (`.toml`).
    Toml,
    /// Anything we have no comment grammar for; scanned as having no comments.
    Unknown,
}

impl Language {
    /// Guess the language from a file extension.
    pub fn from_path(path: &Path) -> Language {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "py" | "pyi" | "pyw" => Language::Python,
            "rs" => Language::Rust,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "ts" | "tsx" | "mts" | "cts" => Language::TypeScript,
            "sh" | "bash" | "zsh" | "ksh" => Language::Shell,
            "yml" | "yaml" => Language::Yaml,
            "toml" => Language::Toml,
            _ => Language::Unknown,
        }
    }

    /// Whether this language has a comment grammar we can scan.
    pub fn is_scannable(self) -> bool {
        self != Language::Unknown
    }
}

/// Render a path the way reports do: `/`-separated and without a leading `./`.
///
/// Report paths are a wire contract compared across platforms, so a Windows
/// `src\app.py` and a Unix `./src/app.py` must both come out as `src/app.py`.
pub fn display_path(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    raw.strip_prefix("./").unwrap_or(&raw).to_string()
}

/// A file to scan, with its comments already extracted.
#[derive(Debug, Clone)]
pub struct SourceFile {
    path: PathBuf,
    display_path: String,
    language: Language,
    source: String,
    extracted: Extracted,
}

impl SourceFile {
    /// Build a source file from contents already in memory.
    pub fn new(path: impl Into<PathBuf>, source: String) -> Self {
        let path = path.into();
        let language = Language::from_path(&path);
        let extracted = comments::extract(&source, language);
        SourceFile {
            display_path: display_path(&path),
            path,
            language,
            source,
            extracted,
        }
    }

    /// Read a file from disk and extract its comments.
    ///
    /// Returns the underlying [`io::Error`] for unreadable files (including
    /// non-UTF-8 contents, reported as [`io::ErrorKind::InvalidData`]) so the
    /// caller can record a report error rather than abort the scan.
    pub fn read(path: &Path) -> io::Result<Self> {
        let source = fs::read_to_string(path)?;
        Ok(SourceFile::new(path.to_path_buf(), source))
    }

    /// The path as given.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The path as it appears in reports: `/`-separated, no leading `./`.
    pub fn display_path(&self) -> &str {
        &self.display_path
    }

    /// The detected language.
    pub fn language(&self) -> Language {
        self.language
    }

    /// The full file contents.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Every comment in the file, in source order.
    pub fn comments(&self) -> &[Comment] {
        &self.extracted.comments
    }

    /// Every Rust attribute in the file, in source order. Empty for every other
    /// language.
    pub fn attributes(&self) -> &[Attribute] {
        &self.extracted.attributes
    }

    /// Number of lines in the file (a trailing newline does not add one).
    pub fn line_count(&self) -> u32 {
        if self.source.is_empty() {
            return 0;
        }
        let newlines = self.source.matches('\n').count();
        let trailing = u32::from(!self.source.ends_with('\n'));
        u32::try_from(newlines)
            .unwrap_or(u32::MAX)
            .saturating_add(trailing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_map_to_languages() {
        let cases = [
            ("a.py", Language::Python),
            ("a.PYI", Language::Python),
            ("a.rs", Language::Rust),
            ("a.mjs", Language::JavaScript),
            ("a.tsx", Language::TypeScript),
            ("a.bash", Language::Shell),
            ("a.yaml", Language::Yaml),
            ("a.toml", Language::Toml),
            ("a.md", Language::Unknown),
            ("Makefile", Language::Unknown),
        ];
        for (name, expected) in cases {
            assert_eq!(Language::from_path(Path::new(name)), expected, "{name}");
        }
        assert!(Language::Python.is_scannable());
        assert!(!Language::Unknown.is_scannable());
    }

    #[test]
    fn report_paths_are_slash_separated_without_a_dot_prefix() {
        assert_eq!(display_path(Path::new("./src/app.py")), "src/app.py");
        assert_eq!(display_path(Path::new("src\\app.py")), "src/app.py");
        assert_eq!(display_path(Path::new("src/app.py")), "src/app.py");
    }

    #[test]
    fn a_source_file_extracts_its_comments_once() {
        let file = SourceFile::new("a.py", "x = 1  # noqa\n".to_string());
        assert_eq!(file.language(), Language::Python);
        assert_eq!(file.comments().len(), 1);
        assert!(file.attributes().is_empty());
        assert_eq!(file.display_path(), "a.py");
        assert_eq!(file.path(), Path::new("a.py"));
        assert_eq!(file.source(), "x = 1  # noqa\n");
        assert_eq!(file.line_count(), 1);
    }

    #[test]
    fn line_count_handles_empty_and_unterminated_files() {
        assert_eq!(SourceFile::new("a.py", String::new()).line_count(), 0);
        assert_eq!(SourceFile::new("a.py", "a\nb".to_string()).line_count(), 2);
        assert_eq!(
            SourceFile::new("a.py", "a\nb\n".to_string()).line_count(),
            2
        );
    }

    #[test]
    fn reading_a_missing_file_surfaces_the_io_error() {
        let err = SourceFile::read(Path::new("does/not/exist.py")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
