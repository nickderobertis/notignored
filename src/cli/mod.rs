//! The `notignored` command line.
//!
//! Kept free of parsing logic: this layer selects files, calls
//! [`crate::scan`], and renders. `--diff` is one branch in `Cli::select` —
//! asking git for the changed files instead of walking the tree, then narrowing
//! the finished report to the directives the change added and saying of each
//! whether the change wrote it — and touches nothing below it.

use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, ValueEnum};

use crate::diff::{self, Diff, DiffError, FileChange};
use crate::model::{Report, ReportError, Tool};
use crate::scan::{self, ScanError, ScanOptions};
use crate::source::{display_path, Language};

mod markdown;
mod render;

pub use markdown::{render_markdown, MarkdownOptions, DEFAULT_MAX_ENTRIES, MARKER};
pub use render::{narrate_errors, render_human, render_human_colored, render_json, ChangeCounts};

/// The scan completed and nothing forced a failure.
pub const EXIT_OK: u8 = 0;
/// `--fail-if-found` was given and at least one suppression was reported.
pub const EXIT_FOUND: u8 = 1;
/// The scan could not complete: a path was unreadable or a file could not be
/// parsed as source. Clap exits with the same code when an argument is invalid,
/// so 2 means "nothing was reported, and why" either way.
pub const EXIT_ERROR: u8 = 2;

/// How to render the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// One line per suppression, for a person reading a terminal.
    Human,
    /// The full report envelope as JSON, for machines.
    Json,
    /// A pull-request comment body, for the GitHub Action.
    Markdown,
}

/// When to apply ANSI color to the `human` report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    /// Color only when stdout is a terminal and color is not suppressed.
    #[default]
    Auto,
    /// Always emit color, even through a pipe.
    Always,
    /// Never emit color.
    Never,
}

impl ColorChoice {
    /// Resolve to a concrete on/off decision.
    ///
    /// `Auto` colors an interactive terminal that has not asked for plain
    /// output; `Always` overrides both signals, because an explicit flag is how
    /// a user forces color through a pager or into a screenshot, and `Never`
    /// overrides nothing because it already means off.
    pub fn resolve(self, is_tty: bool, no_color: bool) -> bool {
        match self {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => is_tty && !no_color,
        }
    }
}

/// Whether the environment asks for plain output: the
/// [`NO_COLOR`](https://no-color.org) convention (present and non-empty) or a
/// terminal that cannot render styling (`TERM=dumb`).
fn no_color_env() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
        || std::env::var("TERM").is_ok_and(|term| term == "dumb")
}

/// Accept an `owner/repo` slug and nothing else.
///
/// It is interpolated into a permalink, so anything that is not two plain path
/// segments — a full URL, a `..`, a query string — is rejected here rather than
/// rendered into a link that goes somewhere unintended.
fn github_repo(value: &str) -> Result<String, String> {
    let segment_ok = |segment: &str| {
        !segment.is_empty()
            && segment != ".."
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    match value.split_once('/') {
        Some((owner, repo)) if segment_ok(owner) && segment_ok(repo) => Ok(value.to_string()),
        _ => Err(format!("expected owner/repo, got {value:?}")),
    }
}

/// Accept a hexadecimal commit id and nothing else.
///
/// The permalinks pin source to a commit; a branch name would move under the
/// reader and a fragment of one would resolve to something else entirely.
fn github_sha(value: &str) -> Result<String, String> {
    if (7..=64).contains(&value.len()) && value.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "expected a commit sha of 7 to 64 hex digits, got {value:?}"
        ))
    }
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
                  With --diff, only the suppressions the change added are reported, which is the \
                  review case: `notignored --diff --diff-base main`.\n\n\
                  --format markdown renders that report as a pull-request comment body; pass \
                  --github-repo and --github-sha to link each suppression to its source, and \
                  --max-entries to change how many it lists.\n\n\
                  Exit codes:\n  \
                  0  the scan completed\n  \
                  1  --fail-if-found was given and at least one suppression was reported\n  \
                  2  the scan could not run or could not complete (a bad argument, \
                  or an unreadable path or file)"
)]
pub struct Cli {
    /// Files and/or directories to scan. Directories are walked recursively,
    /// honouring .gitignore.
    #[arg(value_name = "PATHS", default_value = ".")]
    pub paths: Vec<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    pub format: Format,

    /// When to colorize the `human` report: `auto` colors only an interactive
    /// terminal with NO_COLOR unset, `always` forces it (through a pipe, a
    /// pager, or into a screenshot), `never` disables it. The `json` and
    /// `markdown` formats are never colorized.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,

    /// Only report this tool. Repeat to allow several; omit for all of them.
    #[arg(long = "tool", value_name = "NAME", value_enum)]
    pub tools: Vec<Tool>,

    /// Exit 1 when any suppression is reported.
    #[arg(long)]
    pub fail_if_found: bool,

    /// The `owner/repo` the `markdown` format links suppressions to. Without it
    /// (or --github-sha) locations render as plain `path:line` text.
    #[arg(long = "github-repo", value_name = "OWNER/REPO", value_parser = github_repo)]
    pub github_repo: Option<String>,

    /// The commit the `markdown` format pins its permalinks to.
    #[arg(long = "github-sha", value_name = "SHA", value_parser = github_sha)]
    pub github_sha: Option<String>,

    /// Most suppressions the `markdown` format lists before it closes with a
    /// line counting the rest. Must be at least 1.
    #[arg(
        long = "max-entries",
        value_name = "N",
        value_parser = clap::value_parser!(u32).range(1..),
        default_value_t = DEFAULT_MAX_ENTRIES,
    )]
    pub max_entries: u32,

    // llmlint: ignore-block[invalid_states_unrepresentable] the pair is a field-per-flag mirror of the command line, and clap rejects a base without --diff at the boundary (`requires = "diff"`, exit 2, covered by an e2e); folding them into an enum would move flag parsing into a custom value parser, which is exactly what this layer keeps out.
    /// Report only the suppressions this change added: those on lines the diff
    /// added. Compares the work tree against HEAD unless --diff-base says
    /// otherwise, and parses only the files the change touched.
    #[arg(long)]
    pub diff: bool,

    /// Base the --diff comparison is taken from: any git revision or range. A
    /// plain ref uses three-dot / merge-base semantics — the comparison starts
    /// where this branch forked, so later base-branch commits are never reported
    /// as this branch's own changes — while an explicit A..B range is passed to
    /// git as-is.
    #[arg(long, value_name = "REF", requires = "diff")]
    pub diff_base: Option<String>,
    // llmlint: ignore-end[invalid_states_unrepresentable]
}

/// What a run decided to look at.
struct Selection {
    /// The files to parse, in report order.
    files: Vec<PathBuf>,
    /// In `--diff` mode, what the change did to each selected file, keyed by
    /// the path the report uses. `None` when every directive is reported and
    /// there is no base to have changed anything against.
    changes: Option<BTreeMap<String, FileChange>>,
    /// What selection itself could not do: a file the change touched under a
    /// name this build cannot represent. Carried into the report's `errors` so a
    /// file that cannot be scanned is never counted as clean.
    errors: Vec<ReportError>,
}

/// Why a run could not decide what to look at.
#[derive(Debug, thiserror::Error)]
enum SelectError {
    /// A path could not be walked.
    #[error(transparent)]
    Scan(#[from] ScanError),
    /// The change itself could not be determined.
    #[error("cannot diff: {0}")]
    Diff(#[from] DiffError),
}

impl SelectError {
    /// The concrete next action for this failure.
    fn hint(&self) -> &'static str {
        match self {
            SelectError::Scan(_) => {
                "pass paths that exist, or omit them to scan the current directory"
            }
            SelectError::Diff(error) => error.hint(),
        }
    }
}

impl Cli {
    /// What this invocation should scan, and how to tell which directives are
    /// new.
    ///
    /// The single seam where file selection is decided: without `--diff` the
    /// tree is walked, with it git names the changed files.
    fn select(&self) -> Result<Selection, SelectError> {
        if !self.diff {
            return Ok(Selection {
                files: scan::discover(&self.paths)?,
                changes: None,
                errors: Vec::new(),
            });
        }
        // The diff is taken where the command was run, so its paths line up with
        // the ones the report prints.
        let diff = Diff::open(Path::new("."), self.diff_base.as_deref())?;
        let changed = diff.changed_files()?;
        // A path this build cannot name cannot be narrowed by PATHS either — the
        // spelling a selector would be compared against is the one we do not
        // have — so it is reported however the run was scoped.
        let errors = changed
            .undecodable
            .iter()
            .map(|path| ReportError {
                path: path.clone(),
                message: "path is not valid UTF-8, so it cannot be scanned".to_string(),
            })
            .collect();

        let mut files = Vec::new();
        let mut changes = BTreeMap::new();
        for file in self.narrow_to_paths(&changed.files)? {
            // A file the change deleted is part of the diff but has no source
            // left to read; skipping it is the answer, not an error.
            if !file.path.exists() {
                continue;
            }
            // Asking git for the lines of a file no parser could read would
            // spend a process on a file the scan skips anyway.
            if !Language::from_path(&file.path).is_scannable() {
                continue;
            }
            let mut change = diff.file_change(file)?;
            // Nothing added means nothing new to find, so the file is never read
            // — and neither is what it used to say.
            if change.is_empty() {
                continue;
            }
            change.set_base(diff.pre_image(file));
            changes.insert(display_path(&file.path), change);
            files.push(file.path.clone());
        }
        Ok(Selection {
            files,
            changes: Some(changes),
            errors,
        })
    }

    /// The changed files this run's `PATHS` select.
    ///
    /// `PATHS` narrow a change the same way they narrow a tree scan, and an
    /// empty intersection is simply an empty report. A path that neither exists
    /// nor appears in the change is a typo rather than a smaller run, and gets
    /// the same error a tree scan gives it.
    fn narrow_to_paths<'a>(
        &self,
        changed: &'a [diff::ChangedFile],
    ) -> Result<Vec<&'a diff::ChangedFile>, ScanError> {
        let mut selected: Vec<&diff::ChangedFile> = Vec::new();
        for path in &self.paths {
            let mut matched = changed
                .iter()
                .filter(|file| diff::path_selects(path, &file.path))
                .peekable();
            if matched.peek().is_none() && !path.exists() {
                return Err(ScanError::Path {
                    path: display_path(path),
                    message: "no such file or directory".to_string(),
                });
            }
            selected.extend(matched);
        }
        // Two overlapping inputs must not report the same file twice.
        selected.sort_by(|a, b| a.path.cmp(&b.path));
        selected.dedup_by(|a, b| a.path == b.path);
        Ok(selected)
    }

    /// The scan configuration this invocation implies.
    fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            tools: self.tools.clone(),
        }
    }

    /// Whether this invocation's human report is colorized.
    ///
    /// The one place `--color` meets the live process: the flag is resolved
    /// against the real stdout and environment here, so [`run`] and the process
    /// shell that wraps stdout for Windows always agree on the same answer.
    pub fn color_enabled(&self) -> bool {
        self.color
            .resolve(io::stdout().is_terminal(), no_color_env())
    }

    /// Where this invocation says the scanned source lives.
    fn markdown_options(&self) -> MarkdownOptions {
        MarkdownOptions {
            repo: self.github_repo.clone(),
            sha: self.github_sha.clone(),
            max_entries: self.max_entries,
        }
    }
}

/// Run the command, writing the report to `out` and diagnostics to `err`.
///
/// Returns the process exit code rather than exiting, so tests can drive it.
pub fn run(cli: &Cli, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let selection = match cli.select() {
        Ok(selection) => selection,
        Err(error) => {
            let _ = writeln!(err, "notignored: {error}");
            let _ = writeln!(err, "hint: {}", error.hint());
            return EXIT_ERROR;
        }
    };

    let mut report = scan::scan_files(&selection.files, &cli.scan_options());
    if let Some(changes) = &selection.changes {
        diff::retain_new(&mut report, changes);
        diff::classify(&mut report, changes, &cli.tools);
    }
    if !selection.errors.is_empty() {
        report.errors.extend(selection.errors);
        report.sort();
    }
    // Errors first, so a human sees what went wrong above the summary and a JSON
    // run still gets them on the terminal when stdout is redirected.
    let _ = narrate_errors(&report, err);
    let rendered = match cli.format {
        Format::Human => render_human_colored(&report, out, err, cli.color_enabled()),
        Format::Json => render_json(&report, out),
        Format::Markdown => render_markdown(&report, &cli.markdown_options(), out),
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
    fn color_defaults_to_auto_and_takes_the_three_documented_values() {
        assert_eq!(parse(&[]).color, ColorChoice::Auto);
        for (flag, expected) in [
            ("auto", ColorChoice::Auto),
            ("always", ColorChoice::Always),
            ("never", ColorChoice::Never),
        ] {
            assert_eq!(parse(&["--color", flag]).color, expected);
        }
        let error = Cli::try_parse_from(["notignored", "--color", "sometimes"]).unwrap_err();
        assert!(error.to_string().contains("sometimes"), "{error}");
    }

    /// `always` is how a user forces color through a pipe, a pager, or into a
    /// screenshot, so neither a redirected stdout nor `NO_COLOR` may veto it;
    /// `never` is unconditional the other way.
    #[test]
    fn an_explicit_color_choice_overrides_the_terminal_and_no_color() {
        for is_tty in [true, false] {
            for no_color in [true, false] {
                assert!(ColorChoice::Always.resolve(is_tty, no_color));
                assert!(!ColorChoice::Never.resolve(is_tty, no_color));
            }
        }
    }

    #[test]
    fn auto_colors_only_an_interactive_terminal_that_did_not_ask_for_plain() {
        assert!(ColorChoice::Auto.resolve(true, false));
        // Piped or redirected — the case every test and every `| grep` hits.
        assert!(!ColorChoice::Auto.resolve(false, false));
        // A terminal, but the NO_COLOR convention wins.
        assert!(!ColorChoice::Auto.resolve(true, true));
        assert!(!ColorChoice::Auto.resolve(false, true));
    }

    /// stdout is not a terminal under a test runner, so `auto` — the default —
    /// is the plain rendering every other assertion in this suite relies on.
    #[test]
    fn the_default_choice_is_plain_when_output_is_captured() {
        assert!(!parse(&[]).color_enabled());
        assert!(parse(&["--color", "always"]).color_enabled());
        assert!(!parse(&["--color", "never"]).color_enabled());
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
    fn diff_flags_are_parsed_and_diff_base_requires_diff() {
        let cli = parse(&["--diff", "--diff-base", "origin/main"]);
        assert!(cli.diff);
        assert_eq!(cli.diff_base.as_deref(), Some("origin/main"));

        let bare = parse(&["--diff"]);
        assert!(bare.diff && bare.diff_base.is_none());

        // A base with nothing to compare is a mistake, not a whole-tree scan.
        let error = Cli::try_parse_from(["notignored", "--diff-base", "main"]).unwrap_err();
        assert!(error.to_string().contains("--diff"), "{error}");
    }

    #[test]
    fn the_markdown_flags_are_parsed_into_render_options() {
        let cli = parse(&[
            "--format",
            "markdown",
            "--github-repo",
            "acme/widgets",
            "--github-sha",
            "0123456789abcdef0123456789abcdef01234567",
            "--max-entries",
            "5",
        ]);
        assert_eq!(cli.format, Format::Markdown);
        assert_eq!(
            cli.markdown_options(),
            MarkdownOptions {
                repo: Some("acme/widgets".into()),
                sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
                max_entries: 5,
            }
        );
        // Omitted, the permalink flags simply render locations as plain text and
        // the cap falls back to the documented default.
        assert_eq!(parse(&[]).markdown_options(), MarkdownOptions::default());
        assert_eq!(parse(&[]).max_entries, DEFAULT_MAX_ENTRIES);
    }

    /// The cap is a number the action passes through from a workflow input, so a
    /// value that would render a different body than the caller asked for has to
    /// stop the run rather than fall back.
    #[test]
    fn a_max_entries_that_is_not_a_positive_number_is_rejected() {
        for bad in ["0", "-1", "twenty", "", "4.5"] {
            let error = Cli::try_parse_from(["notignored", "--max-entries", bad])
                .expect_err("{bad} was accepted");
            assert_eq!(error.exit_code(), i32::from(EXIT_ERROR), "{bad}: {error}");
        }
        assert_eq!(parse(&["--max-entries", "1"]).max_entries, 1);
    }

    #[test]
    fn a_repo_that_is_not_owner_slash_repo_is_rejected() {
        for bad in [
            "widgets",
            "acme/widgets/extra",
            "https://github.com/acme/widgets",
            "acme/../secrets",
            "/widgets",
            "acme/",
        ] {
            assert!(github_repo(bad).is_err(), "{bad} was accepted");
        }
        assert_eq!(github_repo("acme/widgets.rs"), Ok("acme/widgets.rs".into()));

        let error = Cli::try_parse_from(["notignored", "--github-repo", "not-a-slug"]).unwrap_err();
        assert!(error.to_string().contains("owner/repo"), "{error}");
    }

    #[test]
    fn a_sha_that_is_not_a_commit_id_is_rejected() {
        for bad in ["main", "abc", "", &"a".repeat(65), "0123456z"] {
            assert!(github_sha(bad).is_err(), "{bad} was accepted");
        }
        assert_eq!(github_sha("0123abc"), Ok("0123abc".into()));

        let error = Cli::try_parse_from(["notignored", "--github-sha", "main"]).unwrap_err();
        assert!(error.to_string().contains("hex digits"), "{error}");
    }

    #[test]
    fn markdown_format_writes_a_comment_body_to_stdout() {
        let dir = fixture();
        let (code, out, _) = run_in(dir.path(), &["--format", "markdown"]);
        assert_eq!(code, EXIT_OK);
        assert!(out.starts_with(MARKER), "{out}");
        assert!(out.contains("- **ruff E501** — _long URL_ — "), "{out}");
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
            change: None,
        });
        assert_eq!(exit_code(&report, true), EXIT_ERROR);
        assert_eq!(exit_code(&report, false), EXIT_ERROR);

        report.errors.clear();
        assert_eq!(exit_code(&report, true), EXIT_FOUND);
        assert_eq!(exit_code(&report, false), EXIT_OK);
    }
}
