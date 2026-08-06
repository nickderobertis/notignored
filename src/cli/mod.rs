//! The `notignored` command line.
//!
//! Kept free of parsing logic: this layer selects files, calls
//! [`crate::scan`], and renders. A future `--diff [--diff-base REF]` adds one
//! branch to `Cli::select_files` — asking git for the changed files instead of
//! walking the tree — and touches nothing below it.

use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::model::{Report, Tool};
use crate::scan::{self, ScanError, ScanOptions};

mod render;

pub use render::{narrate_errors, render_human, render_json};

/// The scan completed and nothing forced a failure.
pub const EXIT_OK: u8 = 0;
/// `--fail-if-found` was given and at least one suppression was reported.
pub const EXIT_FOUND: u8 = 1;
/// The scan could not complete: a path was unreadable or a file could not be
/// parsed as source.
pub const EXIT_ERROR: u8 = 2;

/// How to render the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// One line per suppression, for a person reading a terminal.
    Human,
    /// The full report envelope as JSON, for machines.
    Json,
}

/// Report every lint/type-check suppression comment in a source tree.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "notignored",
    version,
    about = "Report every lint and type-check suppression comment in a source tree.",
    long_about = "Report every lint and type-check suppression comment in a source tree.\n\n\
                  Suppressions are parsed natively — the tools whose rules are being silenced are \
                  never invoked — so this is fast enough to run on every pull request.\n\n\
                  Exit codes:\n  \
                  0  the scan completed\n  \
                  1  --fail-if-found was given and at least one suppression was reported\n  \
                  2  the scan could not complete (an unreadable path or file)"
)]
pub struct Cli {
    /// Files and/or directories to scan. Directories are walked recursively,
    /// honouring .gitignore.
    #[arg(value_name = "PATHS", default_value = ".")]
    pub paths: Vec<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    pub format: Format,

    /// Only report this tool. Repeat to allow several; omit for all of them.
    #[arg(long = "tool", value_name = "NAME", value_enum)]
    pub tools: Vec<Tool>,

    /// Exit 1 when any suppression is reported.
    #[arg(long)]
    pub fail_if_found: bool,
}

impl Cli {
    /// The files this invocation should scan.
    ///
    /// The single seam where file selection is decided; `--diff` will branch
    /// here.
    fn select_files(&self) -> Result<Vec<PathBuf>, ScanError> {
        scan::discover(&self.paths)
    }

    /// The scan configuration this invocation implies.
    fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            tools: self.tools.clone(),
        }
    }
}

/// Run the command, writing the report to `out` and diagnostics to `err`.
///
/// Returns the process exit code rather than exiting, so tests can drive it.
pub fn run(cli: &Cli, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let files = match cli.select_files() {
        Ok(files) => files,
        Err(error) => {
            let _ = writeln!(err, "notignored: {error}");
            let _ = writeln!(
                err,
                "hint: pass paths that exist, or omit them to scan the current directory"
            );
            return EXIT_ERROR;
        }
    };

    let report = scan::scan_files(&files, &cli.scan_options());
    // Errors first, so a human sees what went wrong above the summary and a JSON
    // run still gets them on the terminal when stdout is redirected.
    let _ = narrate_errors(&report, err);
    let rendered = match cli.format {
        Format::Human => render_human(&report, out, err),
        Format::Json => render_json(&report, out),
    };
    // A downstream consumer that stops reading (`| head`, `| grep -q`) closes the
    // pipe. That is its prerogative, not our failure — report the scan's verdict
    // quietly, the way every other Unix filter does, instead of a write error.
    if let Err(error) = rendered {
        if error.kind() != io::ErrorKind::BrokenPipe {
            let _ = writeln!(err, "notignored: cannot write report: {error}");
            return EXIT_ERROR;
        }
    }
    exit_code(&report, cli.fail_if_found)
}

/// The exit code a finished report implies.
fn exit_code(report: &Report, fail_if_found: bool) -> u8 {
    if !report.errors.is_empty() {
        EXIT_ERROR
    } else if fail_if_found && !report.ignores.is_empty() {
        EXIT_FOUND
    } else {
        EXIT_OK
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use clap::CommandFactory;

    use super::*;
    use crate::model::{IgnoreDirective, ReportError, Scope, Suppressed};

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("notignored").chain(args.iter().copied())).unwrap()
    }

    fn run_in(dir: &std::path::Path, args: &[&str]) -> (u8, String, String) {
        let mut argv = vec![dir.to_string_lossy().to_string()];
        argv.extend(args.iter().map(|a| a.to_string()));
        let cli =
            Cli::try_parse_from(std::iter::once("notignored".to_string()).chain(argv)).unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&cli, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("app.py"),
            "u = URL  # noqa: E501  # long URL\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn defaults_scan_the_current_directory_in_human_format() {
        let cli = parse(&[]);
        assert_eq!(cli.paths, vec![PathBuf::from(".")]);
        assert_eq!(cli.format, Format::Human);
        assert!(cli.tools.is_empty());
        assert!(!cli.fail_if_found);
    }

    #[test]
    fn flags_are_parsed_and_repeatable() {
        let cli = parse(&[
            "a.py",
            "b/",
            "--format",
            "json",
            "--tool",
            "ruff",
            "--tool",
            "mypy",
            "--fail-if-found",
        ]);
        assert_eq!(cli.paths, vec![PathBuf::from("a.py"), PathBuf::from("b/")]);
        assert_eq!(cli.format, Format::Json);
        assert_eq!(cli.tools, vec![Tool::Ruff, Tool::Mypy]);
        assert!(cli.fail_if_found);
        assert_eq!(cli.scan_options().tools, vec![Tool::Ruff, Tool::Mypy]);
    }

    #[test]
    fn an_unknown_tool_is_rejected_by_the_parser() {
        let error = Cli::try_parse_from(["notignored", "--tool", "flake8"]).unwrap_err();
        assert!(error.to_string().contains("flake8"), "{error}");
    }

    #[test]
    fn the_command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn a_clean_run_exits_zero_and_reports_the_finding() {
        let dir = fixture();
        let (code, out, err) = run_in(dir.path(), &[]);
        assert_eq!(code, EXIT_OK);
        assert!(
            out.contains("app.py:1:10 ruff E501 (line) -- long URL"),
            "{out}"
        );
        assert!(err.contains("1 ignore in 1 file"), "{err}");
    }

    #[test]
    fn fail_if_found_turns_a_finding_into_exit_one() {
        let dir = fixture();
        let (code, _, _) = run_in(dir.path(), &["--fail-if-found"]);
        assert_eq!(code, EXIT_FOUND);
    }

    #[test]
    fn fail_if_found_still_exits_zero_with_nothing_to_report() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("app.py"), "x = 1\n").unwrap();
        let (code, out, _) = run_in(dir.path(), &["--fail-if-found"]);
        assert_eq!(code, EXIT_OK);
        assert!(out.is_empty(), "{out}");
    }

    #[test]
    fn a_missing_path_exits_two_with_a_hint() {
        let cli = parse(&["does/not/exist"]);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&cli, &mut out, &mut err);
        assert_eq!(code, EXIT_ERROR);
        let err = String::from_utf8(err).unwrap();
        assert!(err.contains("does/not/exist"), "{err}");
        assert!(err.contains("hint:"), "{err}");
        assert!(out.is_empty());
    }

    #[test]
    fn an_unreadable_source_file_exits_two() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("broken.py"), [b'x', 0xff, b'\n']).unwrap();
        let (code, _, err) = run_in(dir.path(), &[]);
        assert_eq!(code, EXIT_ERROR);
        assert!(err.contains("broken.py"), "{err}");
    }

    /// A stdout that always fails with `kind`.
    struct Failing(io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "write failed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(self.0, "flush failed"))
        }
    }

    #[test]
    fn a_failing_writer_exits_two() {
        let dir = fixture();
        let cli = parse(&[&dir.path().to_string_lossy(), "--format", "json"]);
        let mut err = Vec::new();
        let code = run(&cli, &mut Failing(io::ErrorKind::StorageFull), &mut err);
        assert_eq!(code, EXIT_ERROR);
        assert!(String::from_utf8(err)
            .unwrap()
            .contains("cannot write report"));
    }

    #[test]
    fn a_closed_pipe_is_not_an_error() {
        let dir = fixture();
        let mut err = Vec::new();
        let cli = parse(&[&dir.path().to_string_lossy(), "--format", "json"]);
        assert_eq!(
            run(&cli, &mut Failing(io::ErrorKind::BrokenPipe), &mut err),
            EXIT_OK
        );
        assert!(
            String::from_utf8(err.clone()).unwrap().is_empty(),
            "{err:?}"
        );

        // The scan's own verdict still stands: `--fail-if-found | head` exits 1.
        let cli = parse(&[&dir.path().to_string_lossy(), "--fail-if-found"]);
        assert_eq!(
            run(&cli, &mut Failing(io::ErrorKind::BrokenPipe), &mut err),
            EXIT_FOUND
        );
    }

    #[test]
    fn report_errors_outrank_fail_if_found() {
        let mut report = Report::new();
        report.errors.push(ReportError {
            path: "a.py".into(),
            message: "boom".into(),
        });
        report.ignores.push(IgnoreDirective {
            tool: Tool::Ruff,
            scope: Scope::Line,
            rules: vec![],
            reason: None,
            path: "a.py".into(),
            line: 1,
            end_line: 1,
            column: 1,
            raw: "# noqa".into(),
            suppressed: Suppressed {
                start_line: 1,
                end_line: Some(1),
            },
        });
        assert_eq!(exit_code(&report, true), EXIT_ERROR);
        assert_eq!(exit_code(&report, false), EXIT_ERROR);

        report.errors.clear();
        assert_eq!(exit_code(&report, true), EXIT_FOUND);
        assert_eq!(exit_code(&report, false), EXIT_OK);
    }
}
