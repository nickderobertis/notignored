//! Turning paths into a [`Report`].
//!
//! File selection ([`discover`]) is deliberately separate from parsing
//! ([`scan_files`]): a future `--diff` mode swaps the selection step for "the
//! files this branch changed" and reuses everything below it unchanged.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::model::{Report, ReportError, Tool};
use crate::source::{display_path, Language, SourceFile};
use crate::tools::llmlint::LlmlintParser;
use crate::tools::registry_for;

/// What to look for in the selected files.
#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Only report these tools. Empty means every registered parser.
    pub tools: Vec<Tool>,
}

/// A path the scan could not even begin on.
///
/// Problems *within* a walk (an unreadable file) become [`ReportError`] entries
/// instead, so one bad file cannot abort a whole scan.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// An input path does not exist or could not be walked.
    #[error("cannot read {path}: {message}")]
    Path {
        /// The offending path, as the user wrote it.
        path: String,
        /// The underlying reason.
        message: String,
    },
}

/// Collect every scannable file under `paths`.
///
/// Directories are walked recursively, honouring `.gitignore` (and `.ignore`)
/// rules whether or not the tree is inside a git repository, and skipping hidden
/// files. Files named explicitly are taken as given. The result is sorted and
/// de-duplicated so two overlapping inputs cannot report the same file twice.
pub fn discover(paths: &[PathBuf]) -> Result<Vec<PathBuf>, ScanError> {
    let mut found = BTreeSet::new();
    for path in paths {
        if !path.exists() {
            return Err(ScanError::Path {
                path: display_path(path),
                message: "no such file or directory".to_string(),
            });
        }
        let walk = WalkBuilder::new(path)
            // Honour `.gitignore` outside a git repository too: users expect
            // "respects .gitignore" to mean the file, not the repo.
            .require_git(false)
            .sort_by_file_path(Path::cmp)
            .build();
        for entry in walk {
            let entry = entry.map_err(|error| ScanError::Path {
                path: display_path(path),
                message: error.to_string(),
            })?;
            if entry.file_type().is_some_and(|kind| kind.is_file()) {
                found.insert(entry.into_path());
            }
        }
    }
    Ok(found.into_iter().collect())
}

/// Parse every directive out of `files`.
///
/// Files in a language we have no grammar for are skipped; files that cannot be
/// read — and directives a parser recognized but could not resolve — become
/// [`Report::errors`] entries.
pub fn scan_files(files: &[PathBuf], options: &ScanOptions) -> Report {
    let parsers = registry_for(&options.tools);
    let mut report = Report::new();
    for path in files {
        if !Language::from_path(path).is_scannable() {
            continue;
        }
        match SourceFile::read(path) {
            Ok(file) => {
                for parser in &parsers {
                    if !parser.applies_to(&file) {
                        continue;
                    }
                    // `ToolParser::parse` hands back directives and nothing
                    // else — that trait is a fixed contract. llmlint is the one
                    // syntax whose directive can be malformed in a way an
                    // `IgnoreDirective` cannot express (an `ignore-block` left
                    // open), so its parser keeps the richer result as an
                    // inherent method and the scan integrates it here instead.
                    if parser.tool() == Tool::Llmlint {
                        let scanned = LlmlintParser.scan(&file);
                        report.ignores.extend(scanned.directives);
                        report.errors.extend(scanned.errors);
                    } else {
                        report.ignores.extend(parser.parse(&file));
                    }
                }
            }
            Err(error) => report.errors.push(ReportError {
                path: display_path(path),
                message: error.to_string(),
            }),
        }
    }
    report.sort();
    report
}

/// Discover and scan in one step.
pub fn scan(paths: &[PathBuf], options: &ScanOptions) -> Result<Report, ScanError> {
    let files = discover(paths)?;
    Ok(scan_files(&files, options))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("build")).unwrap();
        fs::write(root.join(".gitignore"), "build/\n").unwrap();
        fs::write(root.join("src/app.py"), "x = 1  # noqa: E501\n").unwrap();
        fs::write(root.join("src/notes.md"), "# noqa\n").unwrap();
        fs::write(root.join("build/gen.py"), "y = 2  # noqa\n").unwrap();
        dir
    }

    #[test]
    fn discovery_walks_recursively_and_honours_gitignore() {
        let dir = tree();
        let found = discover(&[dir.path().to_path_buf()]).unwrap();
        let names: Vec<String> = found
            .iter()
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert!(names.contains(&"src/app.py".to_string()), "{names:?}");
        assert!(names.contains(&"src/notes.md".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("build/")), "{names:?}");
        assert!(!names.contains(&".gitignore".to_string()), "{names:?}");
    }

    #[test]
    fn overlapping_inputs_report_each_file_once() {
        let dir = tree();
        let found = discover(&[dir.path().to_path_buf(), dir.path().join("src/app.py")]).unwrap();
        let hits = found.iter().filter(|p| p.ends_with("app.py")).count();
        assert_eq!(hits, 1, "{found:?}");
    }

    #[test]
    fn a_missing_input_path_is_an_error() {
        let error = discover(&[PathBuf::from("does/not/exist")]).unwrap_err();
        assert!(error.to_string().contains("does/not/exist"), "{error}");
        assert!(error.to_string().contains("no such file"), "{error}");
    }

    #[test]
    fn scanning_reports_directives_and_skips_unscannable_files() {
        let dir = tree();
        let report = scan(&[dir.path().to_path_buf()], &ScanOptions::default()).unwrap();
        assert_eq!(report.ignores.len(), 1, "{report:#?}");
        assert_eq!(report.ignores[0].rules, vec!["E501"]);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn a_tool_filter_selects_parsers() {
        let dir = tree();
        let options = ScanOptions {
            tools: vec![Tool::Eslint],
        };
        let report = scan(&[dir.path().to_path_buf()], &options).unwrap();
        assert!(report.ignores.is_empty(), "{report:#?}");

        let options = ScanOptions {
            tools: vec![Tool::Ruff],
        };
        let report = scan(&[dir.path().to_path_buf()], &options).unwrap();
        assert_eq!(report.ignores.len(), 1);
    }

    #[test]
    fn an_unreadable_file_becomes_a_report_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.py");
        // Invalid UTF-8 is the portable stand-in for "cannot be read as source".
        fs::write(&path, [b'x', b' ', 0xff, 0xfe, b'\n']).unwrap();
        let report = scan_files(&[path], &ScanOptions::default());
        assert!(report.ignores.is_empty());
        assert_eq!(report.errors.len(), 1, "{report:#?}");
        assert!(report.errors[0].path.ends_with("broken.py"));
        assert!(!report.errors[0].message.is_empty());
    }
}
