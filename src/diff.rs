//! What a change touched: which files, and which lines it added to them.
//!
//! `--diff` narrows a report to the suppressions a change *introduced* — the
//! pull-request review case, where the historical inventory is noise. Selection
//! happens here so [`scan`](crate::scan) is only ever handed the files a change
//! actually touched, which is what keeps a diff run cheap on a large repository.
//!
//! Git is shelled out to. The rule this crate lives by — never invoke the tool
//! whose rule is being silenced — is about *linters*; git is infrastructure, and
//! re-implementing its rename detection and merge-base arithmetic would be a
//! worse contract than asking it.
//!
//! The semantics mirror llmlint's `--diff` / `--diff-base` exactly, so a project
//! already running llmlint in CI can predict what notignored selects: a plain
//! ref is compared from the **merge base** (three-dot, like a pull request's
//! "Files changed"), while an explicit `A..B` range is handed to git untouched.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::model::Report;
use crate::source::display_path;

/// A `--diff` run that could not be decided.
///
/// The user asked for the changed files, so a fault here is an error rather than
/// a silently empty — and therefore falsely clean — report.
#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    /// No `git` on PATH.
    #[error("git not found on PATH")]
    GitMissing,
    /// A `git` that could not be started (not executable, and so on).
    #[error("cannot run git: {message}")]
    Spawn {
        /// The underlying reason.
        message: String,
    },
    /// The directory is not inside a git work tree (a bare repository, say).
    #[error("not inside a git work tree: {path}")]
    NotAWorkTree {
        /// The directory the diff was asked for.
        path: String,
    },
    /// A git invocation failed; its own message says why.
    #[error("`{command}` failed: {message}")]
    Git {
        /// The command as it was run.
        command: String,
        /// git's stderr, trimmed.
        message: String,
    },
    /// git succeeded but printed something this build cannot read.
    ///
    /// Skipping the unreadable part would answer "this change added no
    /// suppression" without having looked, so it is a failure instead.
    #[error("cannot read `{command}` output: {detail}")]
    Malformed {
        /// The command whose output could not be read.
        command: String,
        /// What could not be read, quoted.
        detail: String,
    },
}

impl DiffError {
    /// The concrete next action for this failure.
    pub fn hint(&self) -> &'static str {
        match self {
            DiffError::GitMissing => "install git, or drop --diff to scan the whole tree",
            DiffError::Spawn { .. } => {
                "make sure the git on PATH is executable, or drop --diff to scan the whole tree"
            }
            DiffError::NotAWorkTree { .. } => {
                "run --diff from inside a git work tree, or drop --diff to scan the whole tree"
            }
            DiffError::Git { .. } => {
                "pass a --diff-base this repository can resolve: a branch, tag, commit, or A..B range"
            }
            DiffError::Malformed { .. } => {
                "this git speaks a diff format notignored cannot read — report it, \
                 or drop --diff to scan the whole tree"
            }
        }
    }
}

/// A file a change touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// The path git reported, relative to the directory the diff was taken in.
    pub path: PathBuf,
    /// Where git paired the file as renamed (or copied) from.
    ///
    /// Kept so the per-file diff can name **both** paths: git only detects a
    /// rename when the pathspec admits the source too, and a rename it cannot
    /// see reads as a whole-file addition — every suppression in a moved file
    /// would then look new.
    pub renamed_from: Option<PathBuf>,
}

/// The 1-based lines a change added to one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddedLines {
    /// Inclusive `(first, last)` runs, in file order.
    runs: Vec<(u32, u32)>,
}

impl AddedLines {
    /// Whether the change added no line at all to this file.
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// Whether any line of the inclusive span `start..=end` was added.
    ///
    /// A span, not a single line, so a directive written across several lines
    /// counts as new when the change added *any* of them.
    pub fn intersects(&self, start: u32, end: u32) -> bool {
        self.runs
            .iter()
            .any(|(first, last)| *first <= end && start <= *last)
    }

    /// Read the added-line runs out of a unified diff.
    ///
    /// Only hunk headers (`@@ -12,0 +13,4 @@`) are parsed, and only their `+`
    /// side: those counts are in the *new* file, which is what a report's line
    /// numbers refer to. Body lines are ignored, so a source line that itself
    /// looks like a header (it arrives prefixed with `+`, `-`, or a space) can
    /// never be mistaken for one.
    /// A header that *is* one and cannot be read is an error, not a skip: the
    /// lines it covers would otherwise be silently treated as unchanged.
    fn parse(patch: &str, command: &str) -> Result<Self, DiffError> {
        let mut runs = Vec::new();
        for line in patch.lines() {
            let Some(header) = line.strip_prefix("@@ ") else {
                continue;
            };
            let malformed = || DiffError::Malformed {
                command: command.to_string(),
                detail: format!("unreadable hunk header {line:?}"),
            };
            let new_side = header
                .split_whitespace()
                .find_map(|field| field.strip_prefix('+'))
                .ok_or_else(malformed)?;
            // A missing count means one line; a zero count is a pure deletion,
            // which adds nothing.
            let (first, count) = match new_side.split_once(',') {
                Some((first, count)) => (first, count),
                None => (new_side, "1"),
            };
            let first: u32 = first.parse().map_err(|_| malformed())?;
            let count: u32 = count.parse().map_err(|_| malformed())?;
            if count > 0 {
                runs.push((first, first.saturating_add(count - 1)));
            }
        }
        Ok(AddedLines { runs })
    }
}

/// What a diff is taken against.
///
/// The cases git accepts here are different decisions, not interchangeable
/// strings, so each is spelled out: the default `HEAD`, the index (all a
/// repository with no commit yet can be compared to), a revision this crate
/// resolved (a `--diff-base` ref's merge base with `HEAD`, or the ref itself
/// when there is no merge base), and a range the caller spelled, which is the
/// one case passed to git untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Base {
    /// The work tree against `HEAD`.
    Head,
    /// The index against the empty tree.
    Index,
    /// A resolved revision.
    Revision(String),
    /// An `A..B` range exactly as the caller wrote it.
    Range(String),
}

impl Base {
    /// The single argument this base becomes on a `git diff` command line.
    fn arg(&self) -> &str {
        match self {
            Base::Head => "HEAD",
            Base::Index => "--cached",
            Base::Revision(rev) | Base::Range(rev) => rev,
        }
    }
}

/// A prepared `git diff` of one work tree against one base.
#[derive(Debug, Clone)]
pub struct Diff {
    /// Directory the diff is taken in; paths are reported relative to it.
    root: PathBuf,
    /// What the work tree is compared against.
    base: Base,
    /// The `git` binary. A field so tests can point it at one that is missing or
    /// cannot be started.
    git: String,
}

impl Diff {
    /// Prepare a diff of the work tree at `root` against `base`.
    ///
    /// `base` is any git revision or range, or `None` for the default `HEAD`.
    /// A plain ref is resolved to its **merge base** with `HEAD`, giving the
    /// three-dot semantics a pull request's "Files changed" shows: the
    /// comparison starts where this branch forked, so commits that landed on the
    /// base branch afterwards are never reported as this branch's own changes.
    /// An explicit `A..B` range is the caller's choice of semantics and is
    /// passed to git untouched.
    pub fn open(root: &Path, base: Option<&str>) -> Result<Self, DiffError> {
        let mut diff = Diff {
            root: root.to_path_buf(),
            base: Base::Head,
            git: "git".to_string(),
        };
        // Validate the boundary once, so a bare repository or a directory
        // outside version control fails with one clear message instead of an
        // empty report that reads as "nothing changed".
        match diff.git(&["rev-parse", "--is-inside-work-tree"]) {
            Ok(inside) if inside.trim() == "true" => {}
            // A bare repository answers "false"; anywhere outside a repository
            // git fails, and its message says the same thing this one does.
            Ok(_) | Err(DiffError::Git { .. }) => {
                return Err(DiffError::NotAWorkTree {
                    path: display_path(root),
                })
            }
            Err(error) => return Err(error),
        }
        diff.base = diff.resolve_base(base);
        Ok(diff)
    }

    /// Every file this change touched, in git's order.
    pub fn changed_files(&self) -> Result<Vec<ChangedFile>, DiffError> {
        // `-z` makes the records NUL-separated, so a path holding a space or a
        // quote needs no unquoting; `--relative` reports paths the way this run's
        // report does — relative to the directory it was invoked from.
        let args = ["diff", "--name-status", "-z", "--relative", self.base.arg()];
        let output = self.git(&args)?;
        parse_name_status(&output, &format!("git {}", args.join(" ")))
    }

    /// The lines this change added to `file`.
    pub fn added_lines(&self, file: &ChangedFile) -> Result<AddedLines, DiffError> {
        let path = display_path(&file.path);
        let renamed_from = file.renamed_from.as_deref().map(display_path);
        // `--unified=0` keeps the patch to its hunk headers and changed lines:
        // context we would only skip past.
        let mut args = vec![
            "diff",
            "--no-color",
            "--unified=0",
            "--relative",
            self.base.arg(),
            "--",
            &path,
        ];
        if let Some(source) = &renamed_from {
            args.push(source);
        }
        let patch = self.git(&args)?;
        AddedLines::parse(&patch, &format!("git {}", args.join(" ")))
    }

    /// What `base` means as something to compare against.
    fn resolve_base(&self, base: Option<&str>) -> Base {
        match base {
            // A range is the caller's own choice of semantics.
            Some(range) if range.contains("..") => Base::Range(range.to_string()),
            // A plain ref gets three-dot semantics. When no merge base exists
            // (disjoint histories) or none can be computed, fall back to the ref
            // itself: an unrelated base stays diffable, and a ref that does not
            // resolve still surfaces git's own error at the diff step rather than
            // being swallowed into a silently different comparison.
            Some(rev) => Base::Revision(self.merge_base(rev).unwrap_or_else(|| rev.to_string())),
            None if self.rev_exists("HEAD") => Base::Head,
            // A repository with no commit yet has no HEAD to diff against, so
            // compare the index with the empty tree instead of fataling: a staged
            // new file is still a change someone is about to review.
            None => Base::Index,
        }
    }

    /// The divergence point of `rev` and `HEAD`, or `None` when git cannot name
    /// one.
    fn merge_base(&self, rev: &str) -> Option<String> {
        let found = self.git(&["merge-base", rev, "HEAD"]).ok()?;
        let found = found.trim().to_string();
        (!found.is_empty()).then_some(found)
    }

    /// Whether `rev` resolves to a commit.
    fn rev_exists(&self, rev: &str) -> bool {
        self.git(&["rev-parse", "--verify", "--quiet", rev]).is_ok()
    }

    /// Run git in the diff's root, returning its stdout.
    fn git(&self, args: &[&str]) -> Result<String, DiffError> {
        let output = git_command(&self.git, &self.root)
            .args(args)
            .output()
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    DiffError::GitMissing
                } else {
                    DiffError::Spawn {
                        message: error.to_string(),
                    }
                }
            })?;
        if !output.status.success() {
            return Err(DiffError::Git {
                command: format!("git {}", args.join(" ")),
                message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        // Paths and diffs are read lossily for the same reason report paths are:
        // one undecodable byte must not abort a scan.
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// The environment variables that tell git which repository to operate on, and
/// which `-C <dir>` does **not** outrank.
///
/// Every git hook runs with `GIT_DIR` exported for the repository it fired in,
/// and `pre-push` also exports `GIT_INDEX_FILE`. A `--diff` run from inside one
/// would silently report that repository instead of the path it was given —
/// and a hook, or a CI step git itself invoked, is exactly where this tool is
/// meant to run.
const AMBIENT_REPOSITORY_VARS: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
];

/// A `git` invocation rooted at `dir` and nowhere else.
///
/// Clearing [`AMBIENT_REPOSITORY_VARS`] leaves `-C` as the only thing that
/// decides which repository answers.
fn git_command(program: &str, dir: &Path) -> Command {
    let mut command = Command::new(program);
    command.arg("-C").arg(dir);
    for name in AMBIENT_REPOSITORY_VARS {
        command.env_remove(name);
    }
    command
}

/// Read `git diff --name-status -z` output.
///
/// Records are NUL-separated: a status field, then the path it applies to — or
/// two paths, source then destination, when the status is a rename or a copy.
fn parse_name_status(output: &str, command: &str) -> Result<Vec<ChangedFile>, DiffError> {
    let mut fields = output.split('\0').filter(|field| !field.is_empty());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        // A record cut short, or one whose status this build does not know,
        // means git described a change in terms we cannot act on. Answering from
        // the records that did arrive would report on a change we only partly
        // know, so either is a failure instead.
        let malformed = |detail: String| DiffError::Malformed {
            command: command.to_string(),
            detail,
        };
        let paired = match status_kind(status) {
            Some(paired) => paired,
            None => return Err(malformed(format!("unknown status {status:?}"))),
        };
        let truncated = || malformed(format!("record {status:?} names no path"));
        let first = fields.next().ok_or_else(truncated)?;
        if !paired {
            files.push(ChangedFile {
                path: PathBuf::from(first),
                renamed_from: None,
            });
            continue;
        }
        let destination = fields.next().ok_or_else(truncated)?;
        files.push(ChangedFile {
            path: PathBuf::from(destination),
            renamed_from: Some(PathBuf::from(first)),
        });
    }
    Ok(files)
}

/// Whether a `--name-status` status field names a two-path (rename or copy)
/// change, or `None` when git named a status this build does not know.
///
/// Only `R`/`C` carry a similarity score and a second path; `X` is git's own
/// "this is a bug" marker and is not something to report a change from.
fn status_kind(status: &str) -> Option<bool> {
    let (letter, score) = status.split_at(
        status
            .char_indices()
            .nth(1)
            .map_or(status.len(), |(i, _)| i),
    );
    match letter {
        "R" | "C" if score.chars().all(|c| c.is_ascii_digit()) => Some(true),
        "A" | "D" | "M" | "T" | "U" if score.is_empty() => Some(false),
        _ => None,
    }
}

/// Whether `selector` names `candidate` or a directory holding it.
///
/// This is how positional `PATHS` narrow a change: both sides are read relative
/// to the directory the diff was taken in, and a selector pointing outside it
/// (an absolute path elsewhere, a `../` escape) selects nothing.
pub fn path_selects(selector: &Path, candidate: &Path) -> bool {
    let Some(prefix) = relative_prefix(selector) else {
        return false;
    };
    if prefix.is_empty() {
        return true;
    }
    let candidate = display_path(candidate);
    candidate == prefix
        || candidate
            .strip_prefix(&prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// A path's `/`-separated form relative to the current directory, or `None` when
/// it points outside it. The empty string is the current directory itself.
fn relative_prefix(path: &Path) -> Option<String> {
    let relative = if path.is_absolute() {
        path.strip_prefix(std::env::current_dir().ok()?)
            .ok()?
            .to_path_buf()
    } else {
        path.to_path_buf()
    };
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => parts.push(part),
            // A `..` that walks above the diff root leaves the tree git reported.
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let joined: PathBuf = parts.iter().collect();
    Some(display_path(&joined))
}

/// Drop every directive the change did not introduce.
///
/// A directive is **new** when at least one line it occupies (`line..=end_line`)
/// is a line the change added. That is the directive's own span, not the span it
/// suppresses: a pre-existing `# ruff: noqa` at the top of a file does not become
/// new because lines below it were edited. Report errors are left alone — a file
/// that could not be read still has to be reported.
pub fn retain_new(report: &mut Report, added: &BTreeMap<String, AddedLines>) {
    report.ignores.retain(|directive| {
        added
            .get(&directive.path)
            .is_some_and(|lines| lines.intersects(directive.line, directive.end_line))
    });
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::model::{IgnoreDirective, ReportError, Scope, Suppressed, Tool};

    use super::*;

    /// Run git in `dir`, panicking with the command that failed.
    ///
    /// Through [`git_command`] for the same reason the production path is: the
    /// gate runs inside a `pre-push` hook, and a scratch repository built with
    /// `GIT_DIR` inherited is not a scratch repository at all.
    fn git(dir: &Path, args: &[&str]) {
        let output = git_command("git", dir)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("run git {args:?}: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A repository on `main` with committed contents, independent of the
    /// developer's own git configuration.
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "Tester"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        git(dir.path(), &["checkout", "-q", "-b", "main"]);
        dir
    }

    fn write(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).expect("write file");
    }

    fn commit(dir: &Path, message: &str) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", message]);
    }

    /// The added lines of a patch git is standing in for.
    fn added_lines(patch: &str) -> AddedLines {
        AddedLines::parse(patch, "git diff").expect("a readable patch")
    }

    fn changed(diff: &Diff) -> Vec<(String, Option<String>)> {
        diff.changed_files()
            .expect("changed files")
            .into_iter()
            .map(|file| {
                (
                    display_path(&file.path),
                    file.renamed_from.as_deref().map(display_path),
                )
            })
            .collect()
    }

    fn added(diff: &Diff, path: &str) -> AddedLines {
        let file = diff
            .changed_files()
            .expect("changed files")
            .into_iter()
            .find(|file| display_path(&file.path) == path)
            .unwrap_or_else(|| panic!("{path} is not in the diff"));
        diff.added_lines(&file).expect("added lines")
    }

    #[test]
    fn hunk_headers_become_added_line_runs() {
        let patch = concat!(
            "diff --git a/a.py b/a.py\n",
            "--- a/a.py\n",
            "+++ b/a.py\n",
            "@@ -1,0 +2,3 @@ def f():\n",
            "+one\n+two\n+three\n",
            "@@ -9 +12 @@\n",
            "-old\n+new\n",
            "@@ -20,2 +24,0 @@\n",
            "-gone\n-gone\n",
        );
        let added = added_lines(patch);
        assert!(added.intersects(2, 2) && added.intersects(4, 4));
        assert!(!added.intersects(1, 1) && !added.intersects(5, 11));
        assert!(added.intersects(12, 12));
        // A pure deletion adds nothing at the line it removed.
        assert!(!added.intersects(24, 24));
        assert!(!added.is_empty());
    }

    #[test]
    fn a_span_counts_when_any_of_its_lines_was_added() {
        let added = added_lines("@@ -1,0 +5,2 @@\n+a\n+b\n");
        // Straddling the run from either side, and containing it whole.
        assert!(added.intersects(3, 5));
        assert!(added.intersects(6, 9));
        assert!(added.intersects(1, 20));
        assert!(!added.intersects(1, 4));
        assert!(!added.intersects(7, 9));
    }

    #[test]
    fn an_unreadable_hunk_header_is_an_error_rather_than_a_guess() {
        for patch in [
            "@@ no plus side @@\n",
            "@@ -1,0 +not-a-number,2 @@\n",
            "@@ -1,0 +5,not-a-number @@\n",
        ] {
            let error = AddedLines::parse(patch, "git diff").unwrap_err();
            assert!(matches!(error, DiffError::Malformed { .. }), "{error:?}");
            assert!(error.to_string().contains("hunk header"), "{error}");
            assert!(error.hint().contains("--diff"), "{}", error.hint());
        }

        // A body line that merely looks like a header is body, not a header: a
        // patch of a patch still reads as one hunk of added text.
        let added = added_lines("@@ -1,0 +9,2 @@\n+@@ -1,0 +3,3 @@\n+ @@ -1,0 +3,3 @@\n");
        assert!(added.intersects(9, 10) && !added.intersects(3, 3));
        assert!(AddedLines::default().is_empty());
    }

    #[test]
    fn the_default_base_is_the_working_tree_against_head() {
        let dir = repo();
        let root = dir.path();
        write(root, "a.py", "x = 1\n");
        write(root, "b.py", "y = 2\n");
        commit(root, "baseline");
        write(root, "a.py", "x = 1\nz = 3  # noqa: E501\n");

        let diff = Diff::open(root, None).expect("open diff");
        assert_eq!(changed(&diff), vec![("a.py".to_string(), None)]);
        assert!(added(&diff, "a.py").intersects(2, 2));
        assert!(!added(&diff, "a.py").intersects(1, 1));
    }

    #[test]
    fn an_unborn_head_diffs_the_index_instead_of_fataling() {
        let dir = repo();
        let root = dir.path();
        write(root, "a.py", "x = 1  # noqa\n");
        git(root, &["add", "a.py"]);

        let diff = Diff::open(root, None).expect("open diff");
        assert_eq!(changed(&diff), vec![("a.py".to_string(), None)]);
        assert!(added(&diff, "a.py").intersects(1, 1));
    }

    #[test]
    fn a_plain_ref_is_compared_from_the_merge_base() {
        let dir = repo();
        let root = dir.path();
        write(root, "app.py", "x = 1\n");
        write(root, "base_only.py", "value = 1  # noqa: E501\n");
        commit(root, "fork point");
        git(root, &["checkout", "-q", "-b", "feature"]);
        write(root, "app.py", "x = 1\nimport os  # noqa: F401\n");
        commit(root, "feature change");
        // main moves on, rewriting the line the feature branch still carries.
        git(root, &["checkout", "-q", "main"]);
        write(root, "base_only.py", "value = 1\n");
        commit(root, "base drift");
        git(root, &["checkout", "-q", "feature"]);

        let three_dot = Diff::open(root, Some("main")).expect("open diff");
        assert_eq!(changed(&three_dot), vec![("app.py".to_string(), None)]);

        // The same comparison spelled as a raw two-dot range keeps git's own
        // semantics: the base branch's later edit reads as this branch's change.
        let two_dot = Diff::open(root, Some("main..HEAD")).expect("open diff");
        let paths: Vec<String> = changed(&two_dot).into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, vec!["app.py", "base_only.py"]);
        assert!(added(&two_dot, "base_only.py").intersects(1, 1));
    }

    #[test]
    fn a_base_with_no_common_history_falls_back_to_a_two_dot_diff() {
        let dir = repo();
        let root = dir.path();
        write(root, "a.py", "x = 1\n");
        commit(root, "main baseline");
        git(root, &["checkout", "-q", "--orphan", "orphan"]);
        write(root, "a.py", "x = 2  # noqa\n");
        commit(root, "orphan baseline");

        let diff = Diff::open(root, Some("main")).expect("open diff");
        assert_eq!(changed(&diff), vec![("a.py".to_string(), None)]);
    }

    #[test]
    fn a_rename_is_paired_so_the_moved_lines_are_not_added_lines() {
        let dir = repo();
        let root = dir.path();
        write(
            root,
            "old.py",
            "a = 1\nb = 2  # noqa: E501\nc = 3\nd = 4\ne = 5\nf = 6\n",
        );
        commit(root, "baseline");
        git(root, &["mv", "old.py", "new.py"]);

        let diff = Diff::open(root, None).expect("open diff");
        assert_eq!(
            changed(&diff),
            vec![("new.py".to_string(), Some("old.py".to_string()))]
        );
        assert!(added(&diff, "new.py").is_empty());
    }

    #[test]
    fn a_ref_that_does_not_resolve_surfaces_gits_own_error() {
        let dir = repo();
        let root = dir.path();
        write(root, "a.py", "x = 1\n");
        commit(root, "baseline");

        let diff = Diff::open(root, Some("no-such-ref")).expect("open diff");
        let error = diff.changed_files().unwrap_err();
        assert!(matches!(error, DiffError::Git { .. }), "{error:?}");
        assert!(error.to_string().contains("no-such-ref"), "{error}");
        assert!(error.hint().contains("--diff-base"), "{}", error.hint());
    }

    #[test]
    fn a_directory_outside_version_control_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = Diff::open(dir.path(), None).unwrap_err();
        assert!(matches!(error, DiffError::NotAWorkTree { .. }), "{error:?}");
        assert!(error.hint().contains("git work tree"), "{}", error.hint());
    }

    #[test]
    fn a_bare_repository_is_not_a_work_tree() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "--bare"]);
        let error = Diff::open(dir.path(), None).unwrap_err();
        assert!(matches!(error, DiffError::NotAWorkTree { .. }), "{error:?}");
        assert!(error.to_string().contains("not inside a git work tree"));
    }

    #[test]
    fn a_missing_git_is_an_error_with_a_way_out() {
        let dir = tempfile::tempdir().unwrap();
        let diff = Diff {
            root: dir.path().to_path_buf(),
            base: Base::Head,
            git: "notignored-no-such-git".to_string(),
        };
        let error = diff.changed_files().unwrap_err();
        assert!(matches!(error, DiffError::GitMissing), "{error:?}");
        assert!(error.to_string().contains("git not found"), "{error}");
        assert!(error.hint().contains("install git"), "{}", error.hint());
    }

    #[cfg(unix)]
    #[test]
    fn a_git_that_cannot_be_started_is_an_error_with_a_way_out() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("git");
        fs::write(&fake, "#!/bin/sh\n").unwrap();
        let diff = Diff {
            root: dir.path().to_path_buf(),
            base: Base::Head,
            git: fake.to_string_lossy().into_owned(),
        };
        let error = diff.changed_files().unwrap_err();
        assert!(matches!(error, DiffError::Spawn { .. }), "{error:?}");
        assert!(error.hint().contains("executable"), "{}", error.hint());
    }

    #[test]
    fn truncated_name_status_output_is_an_error_not_a_shorter_answer() {
        let command = "git diff --name-status";
        assert!(parse_name_status("", command).unwrap().is_empty());
        for truncated in ["M\0", "R100\0old.py\0"] {
            let error = parse_name_status(truncated, command).unwrap_err();
            assert!(matches!(error, DiffError::Malformed { .. }), "{error:?}");
            assert!(error.to_string().contains("names no path"), "{error}");
        }

        // A status this build does not know — git's own "unknown" marker, or a
        // field that is not a status at all — is not read as a plain change.
        for unknown in ["X\0a.py\0", "M100\0a.py\0", "a.py\0M\0"] {
            let error = parse_name_status(unknown, command).unwrap_err();
            assert!(error.to_string().contains("unknown status"), "{error}");
        }
        // Every status git does define is understood.
        for (record, renamed_from) in [
            ("A\0a.py\0", None),
            ("D\0a.py\0", None),
            ("T\0a.py\0", None),
            ("U\0a.py\0", None),
            ("C75\0old.py\0a.py\0", Some("old.py")),
            ("R\0old.py\0a.py\0", Some("old.py")),
        ] {
            let files = parse_name_status(record, command).unwrap();
            assert_eq!(files.len(), 1, "{record:?}");
            assert_eq!(files[0].path, PathBuf::from("a.py"), "{record:?}");
            assert_eq!(
                files[0].renamed_from,
                renamed_from.map(PathBuf::from),
                "{record:?}"
            );
        }
        assert_eq!(
            parse_name_status("M\0a.py\0A\0b.py\0", command).unwrap(),
            vec![
                ChangedFile {
                    path: PathBuf::from("a.py"),
                    renamed_from: None
                },
                ChangedFile {
                    path: PathBuf::from("b.py"),
                    renamed_from: None
                },
            ]
        );
    }

    #[test]
    fn positional_paths_select_a_file_or_the_directory_holding_it() {
        let candidate = Path::new("src/app.py");
        for selector in [".", "./", "src", "src/", "./src", "src/app.py"] {
            assert!(
                path_selects(Path::new(selector), candidate),
                "{selector} should select {candidate:?}"
            );
        }
        for selector in ["srcs", "src/app", "docs", "/elsewhere/src"] {
            assert!(
                !path_selects(Path::new(selector), candidate),
                "{selector} should not select {candidate:?}"
            );
        }
    }

    #[test]
    fn a_selector_outside_the_diff_root_selects_nothing() {
        let candidate = Path::new("app.py");
        assert!(!path_selects(Path::new("../sibling"), candidate));
        // An absolute path under the current directory still selects; one
        // elsewhere cannot.
        let cwd = std::env::current_dir().unwrap();
        assert!(path_selects(&cwd.join("app.py"), candidate));
        assert!(!path_selects(Path::new("/definitely/elsewhere"), candidate));
        // A `..` that comes back down again stays inside.
        assert!(path_selects(Path::new("docs/../app.py"), candidate));
    }

    fn directive(path: &str, line: u32, end_line: u32) -> IgnoreDirective {
        IgnoreDirective {
            tool: Tool::Ruff,
            scope: Scope::Line,
            rules: vec![],
            reason: None,
            path: path.to_string(),
            line,
            end_line,
            column: 1,
            raw: "# noqa".to_string(),
            suppressed: Suppressed {
                start_line: line,
                end_line: Some(end_line),
            },
        }
    }

    #[test]
    fn only_directives_on_added_lines_survive() {
        let mut report = Report::new();
        report.ignores.push(directive("a.py", 1, 1)); // pre-existing
        report.ignores.push(directive("a.py", 7, 7)); // added
        report.ignores.push(directive("b.py", 7, 7)); // file not in the diff
        report.errors.push(ReportError {
            path: "c.py".into(),
            message: "unreadable".into(),
        });

        let mut added = BTreeMap::new();
        added.insert("a.py".to_string(), added_lines("@@ -6,0 +7,1 @@\n+x\n"));
        retain_new(&mut report, &added);

        assert_eq!(
            report
                .ignores
                .iter()
                .map(|d| (d.path.as_str(), d.line))
                .collect::<Vec<_>>(),
            vec![("a.py", 7)]
        );
        // A file that could not be read is still reported: the run is incomplete
        // either way.
        assert_eq!(report.errors.len(), 1);
    }

    #[test]
    fn a_directive_spanning_lines_survives_on_a_partial_overlap() {
        let mut report = Report::new();
        // A block-comment directive opened before the change and closed inside
        // it: the change is what made it say what it now says.
        report.ignores.push(directive("a.py", 4, 8));
        let mut added = BTreeMap::new();
        added.insert(
            "a.py".to_string(),
            added_lines("@@ -7,0 +8,1 @@\n+  reason\n"),
        );
        retain_new(&mut report, &added);
        assert_eq!(report.ignores.len(), 1, "{report:#?}");

        // ...and one that ends before the first added line does not.
        let mut report = Report::new();
        report.ignores.push(directive("a.py", 4, 6));
        retain_new(&mut report, &added);
        assert!(report.ignores.is_empty(), "{report:#?}");
    }
}
