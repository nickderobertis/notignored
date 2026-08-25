//! What a change touched: which files, and which lines it added to them.
//!
//! `--diff` narrows a report to the suppressions a change *introduced* — the
//! pull-request review case, where the historical inventory is noise. Selection
//! happens here so [`scan`](crate::scan) is only ever handed the files a change
//! actually touched, which is what keeps a diff run cheap on a large repository.
//!
//! Selection alone cannot tell an author who *wrote* a suppression from one who
//! rewrote an existing one's justification: both touch a line the directive
//! occupies. [`classify`] answers that, by reading each changed file's contents
//! at the base out of git and pairing what it finds there with what the head
//! scan reported. It only ever labels — the set of reported directives is
//! exactly what [`retain_new`] left, in the order it left it.
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

use crate::model::{Change, IgnoreDirective, Report, Tool};
use crate::scan::{scan_source, ScanOptions};
use crate::source::{display_path, SourceFile};

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
    /// The object name of the file's contents at the base, or `None` when it had
    /// none — a file the change created, or a diff with nothing to compare
    /// against.
    ///
    /// This is why the change list is read as `--raw` rather than
    /// `--name-status`: git names the source blob in every base this crate
    /// supports, so [`Diff::pre_image`] needs no revision arithmetic of its own.
    pub base_blob: Option<BlobId>,
}

/// A git object name, as git wrote it and as it is handed back to git.
///
/// A newtype rather than a `String` because it crosses back over the boundary
/// it arrived at: [`Diff::pre_image`] passes it to `git cat-file`, and the only
/// thing that makes that safe is that [`BlobId::parse`] accepted nothing but hex
/// digits. A plain `String` would let any text reach that argument from anywhere
/// in the crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobId(String);

impl BlobId {
    /// Read an object name git wrote, or `None` when it is not one.
    ///
    /// The all-zero id is `None` too: that is not an object, it is git saying
    /// this side of the record has no contents.
    fn parse(oid: &str) -> Option<BlobId> {
        if oid.is_empty() || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        (!oid.chars().all(|c| c == '0')).then(|| BlobId(oid.to_string()))
    }

    /// The name, as git spells it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A changed file's contents at the diff's base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseImage {
    /// The path the file had at the base — a rename's *source*, where there is
    /// one, because that is the file whose language and directives these are.
    pub path: PathBuf,
    /// The file as it read then, which is what the pre-image scan parses.
    pub text: String,
}

/// Everything one `git diff --raw` named.
///
/// The files a change touched, and — kept apart from them — the paths this build
/// cannot name. A file path is bytes to git and a `String` in the report
/// contract, so a name that is not valid UTF-8 has no faithful spelling here:
/// the lossy one is a *different* path, and handing it back to git as a pathspec
/// would match nothing and silently drop the file from the review.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changed {
    /// The files to read, in git's order.
    pub files: Vec<ChangedFile>,
    /// The lossy spelling of each path that could not be represented, in git's
    /// order. These become [`Report::errors`] entries.
    pub undecodable: Vec<String>,
}

/// One hunk of a unified diff: the base lines it replaced, and the new lines it
/// wrote in their place.
///
/// Both sides are kept because both are needed. The new side decides which
/// directives a change *touched*, and the base side is where their counterparts
/// lived — the pairing [`classify`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hunk {
    /// Inclusive 1-based `(first, last)` on the base side, or `None` for a pure
    /// insertion, which replaced nothing.
    base: Option<(u32, u32)>,
    /// The same on the new side, or `None` for a pure deletion, which wrote
    /// nothing.
    new: Option<(u32, u32)>,
}

/// Whether the inclusive span `start..=end` overlaps the inclusive `range`.
fn overlaps(range: (u32, u32), start: u32, end: u32) -> bool {
    range.0 <= end && start <= range.1
}

/// What a change did to one file: the hunks of its patch, and — once the caller
/// has asked git for it — the file as it was before them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileChange {
    /// The hunks, in file order.
    hunks: Vec<Hunk>,
    /// The file's contents at the base, when this build could read them.
    base: Option<BaseImage>,
}

impl FileChange {
    /// Whether the change added no line at all to this file.
    pub fn is_empty(&self) -> bool {
        self.hunks.iter().all(|hunk| hunk.new.is_none())
    }

    /// Whether any line of the inclusive span `start..=end` was added.
    ///
    /// A span, not a single line, so a directive written across several lines
    /// counts as new when the change added *any* of them.
    pub fn intersects(&self, start: u32, end: u32) -> bool {
        self.hunks
            .iter()
            .filter_map(|hunk| hunk.new)
            .any(|new| overlaps(new, start, end))
    }

    /// Record the file's contents at the base, as [`Diff::pre_image`] read them.
    pub fn set_base(&mut self, base: Option<BaseImage>) {
        self.base = base;
    }

    /// Read the hunks out of a unified diff.
    ///
    /// Only hunk headers (`@@ -12,0 +13,4 @@`) are parsed — both of their sides,
    /// which is the whole of what a `--unified=0` patch has to say. The `+`
    /// counts are in the *new* file, which is what a report's line numbers refer
    /// to; the `-` counts are in the base, which is where a counterpart is
    /// looked for. Body lines are ignored, so a source line that itself looks
    /// like a header (it arrives prefixed with `+`, `-`, or a space) can never
    /// be mistaken for one.
    /// A header that *is* one and cannot be read is an error, not a skip: the
    /// lines it covers would otherwise be silently treated as unchanged.
    fn parse(patch: &str, command: &str) -> Result<Self, DiffError> {
        let mut hunks = Vec::new();
        for line in patch.lines() {
            let Some(header) = line.strip_prefix("@@ ") else {
                continue;
            };
            let malformed = || DiffError::Malformed {
                command: command.to_string(),
                detail: format!("unreadable hunk header {line:?}"),
            };
            let side = |sign: char| -> Result<Option<(u32, u32)>, DiffError> {
                let field = header
                    .split_whitespace()
                    .find_map(|field| field.strip_prefix(sign))
                    .ok_or_else(malformed)?;
                // A missing count means one line; a zero count means this side
                // holds nothing — a pure insertion has no base, a pure deletion
                // no new text.
                let (first, count) = match field.split_once(',') {
                    Some((first, count)) => (first, count),
                    None => (field, "1"),
                };
                let first: u32 = first.parse().map_err(|_| malformed())?;
                let count: u32 = count.parse().map_err(|_| malformed())?;
                Ok((count > 0).then(|| (first, first.saturating_add(count - 1))))
            };
            hunks.push(Hunk {
                base: side('-')?,
                new: side('+')?,
            });
        }
        Ok(FileChange { hunks, base: None })
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
    pub fn changed_files(&self) -> Result<Changed, DiffError> {
        // `-z` makes the records NUL-separated, so a path holding a space or a
        // quote needs no unquoting; `--relative` reports paths the way this run's
        // report does — relative to the directory it was invoked from. The output
        // is read as bytes because a path *is* bytes: decoding it lossily here
        // would invent a name for a file and then fail to find it.
        //
        // `--raw` carries the status letter `--name-status` would have, and the
        // source object id besides; `--abbrev=40` asks for the object name
        // unabbreviated rather than the handful of characters `--raw` prints by
        // default.
        let args = [
            "diff",
            "--raw",
            "-z",
            "--abbrev=40",
            "--relative",
            self.base.arg(),
        ];
        let output = self.git_bytes(&args)?;
        parse_raw(&output, &format!("git {}", args.join(" ")))
    }

    /// The file's contents at the base, when this build can read them as text.
    ///
    /// `None` is the answer for every file with no comparable previous content —
    /// one the change created, a diff with nothing to compare against — and for
    /// one whose previous bytes are not text. It is never an error: a pre-image
    /// this build cannot read is a file it knows nothing about, and "nothing to
    /// compare against" is the same answer `--diff` gave before there was
    /// anything to compare. Failing here would turn a reviewable change into no
    /// review at all.
    pub fn pre_image(&self, file: &ChangedFile) -> Option<BaseImage> {
        let blob = file.base_blob.as_ref()?;
        let bytes = self.git_bytes(&["cat-file", "blob", blob.as_str()]).ok()?;
        Some(BaseImage {
            // A rename's pre-image is the *source* path's blob, and its language
            // and directives are that file's.
            path: file
                .renamed_from
                .clone()
                .unwrap_or_else(|| file.path.clone()),
            text: String::from_utf8(bytes).ok()?,
        })
    }

    /// The hunks this change made to `file`.
    pub fn file_change(&self, file: &ChangedFile) -> Result<FileChange, DiffError> {
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
        FileChange::parse(&patch, &format!("git {}", args.join(" ")))
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
    ///
    /// Patches and revisions are text; only [`changed_files`](Self::changed_files)
    /// needs the raw bytes, and it goes through [`Self::git_bytes`].
    fn git(&self, args: &[&str]) -> Result<String, DiffError> {
        // A diff's body is read lossily for the same reason report paths are:
        // one undecodable byte must not abort a scan.
        Ok(String::from_utf8_lossy(&self.git_bytes(args)?).into_owned())
    }

    /// Run git in the diff's root, returning its stdout as the bytes git wrote.
    fn git_bytes(&self, args: &[&str]) -> Result<Vec<u8>, DiffError> {
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
        Ok(output.stdout)
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

/// Read `git diff --raw -z` output.
///
/// Records are NUL-separated: an info field —
/// `:<src-mode> <dst-mode> <src-oid> <dst-oid> <status>` — then the path it
/// applies to, or two paths, source then destination, when the status is a
/// rename or a copy.
///
/// The source object id is why this is `--raw` and not `--name-status`: it names
/// the file's contents at the base for *every* base [`Diff`] supports — the work
/// tree against `HEAD`, the index against the empty tree, a resolved revision, a
/// caller's own `A..B` range — so nothing here has to parse a range or recompute
/// a merge base to read what a file used to say. An all-zero id means the file
/// had no previous contents.
///
/// A path whose bytes are not valid UTF-8 is set aside rather than decoded
/// lossily: the replacement characters would name a file that does not exist, so
/// the scan would look for nothing and the report would call the change clean.
/// It is reported instead — see [`Changed::undecodable`].
fn parse_raw(output: &[u8], command: &str) -> Result<Changed, DiffError> {
    let mut fields = output.split(|byte| *byte == 0).filter(|f| !f.is_empty());
    let mut changed = Changed::default();
    while let Some(info) = fields.next() {
        // A record cut short, one this build cannot read, or one whose status it
        // does not know means git described a change in terms we cannot act on.
        // Answering from the records that did arrive would report on a change we
        // only partly know, so each is a failure instead.
        let malformed = |detail: String| DiffError::Malformed {
            command: command.to_string(),
            detail,
        };
        let info = String::from_utf8_lossy(info);
        let Some((base_blob, status)) = parse_raw_info(&info) else {
            return Err(malformed(format!("unreadable record {info:?}")));
        };
        let Some(kind) = status_kind(status) else {
            return Err(malformed(format!("unknown status {status:?}")));
        };
        let truncated = || malformed(format!("record {info:?} names no path"));
        let first = decode_path(fields.next().ok_or_else(truncated)?);
        let (path, renamed_from) = match kind {
            StatusKind::OnePath => (first, None),
            StatusKind::Paired => (
                decode_path(fields.next().ok_or_else(truncated)?),
                Some(first),
            ),
        };
        match (path, renamed_from) {
            (Ok(path), None) => changed.files.push(ChangedFile {
                path,
                renamed_from: None,
                base_blob,
            }),
            (Ok(path), Some(Ok(source))) => changed.files.push(ChangedFile {
                path,
                renamed_from: Some(source),
                base_blob,
            }),
            // Either end of a rename is enough to lose it: git only detects one
            // when the pathspec admits the source too.
            (Err(lossy), _) | (Ok(_), Some(Err(lossy))) => changed.undecodable.push(lossy),
        }
    }
    Ok(changed)
}

/// The source object id and the status letter of one `--raw` info field, or
/// `None` when it is not one.
///
/// The id is `None` when git wrote the all-zero one, which is how it says the
/// file had no contents at the base.
fn parse_raw_info(info: &str) -> Option<(Option<BlobId>, &str)> {
    let fields: Vec<&str> = info.strip_prefix(':')?.split_whitespace().collect();
    let [_src_mode, _dst_mode, src_oid, _dst_oid, status] = fields[..] else {
        return None;
    };
    // The all-zero id is a valid record with no pre-image; anything that is not
    // an object name at all is a format this build cannot read, rather than a
    // name to hand back to git and find out.
    let base_blob = BlobId::parse(src_oid);
    if base_blob.is_none() && !src_oid.chars().all(|c| c == '0') {
        return None;
    }
    Some((base_blob, status))
}

/// The path a `-z` record names, or the lossy spelling of one whose bytes this
/// build cannot represent.
fn decode_path(field: &[u8]) -> Result<PathBuf, String> {
    std::str::from_utf8(field)
        .map(PathBuf::from)
        .map_err(|_| String::from_utf8_lossy(field).into_owned())
}

/// How many paths a `--raw` record carries after its info field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    /// The status applies to the one path that follows it.
    OnePath,
    /// A rename or copy: a source path, then a destination path.
    Paired,
}

/// How many paths a `--raw` record's status letter introduces, or `None` when
/// git named a status this build does not know.
///
/// Only `R`/`C` carry a similarity score and a second path; `X` is git's own
/// "this is a bug" marker and is not something to report a change from.
fn status_kind(status: &str) -> Option<StatusKind> {
    let (letter, score) = status.split_at(
        status
            .char_indices()
            .nth(1)
            .map_or(status.len(), |(i, _)| i),
    );
    match letter {
        "R" | "C" if score.chars().all(|c| c.is_ascii_digit()) => Some(StatusKind::Paired),
        "A" | "D" | "M" | "T" | "U" if score.is_empty() => Some(StatusKind::OnePath),
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
pub fn retain_new(report: &mut Report, changes: &BTreeMap<String, FileChange>) {
    report.ignores.retain(|directive| {
        changes
            .get(&directive.path)
            .is_some_and(|change| change.intersects(directive.line, directive.end_line))
    });
}

/// Say of every reported directive whether the change wrote it or rewrote the
/// justification of one that was already there.
///
/// Only a `--diff` run has a base to answer that against, so this is the only
/// place [`crate::IgnoreDirective::change`] is ever set — a whole-tree scan
/// leaves it `None`, which is the honest answer for an inventory.
///
/// It **labels, and nothing else**: no directive is added, removed, or
/// reordered here. That is a safety property rather than a convenience — the
/// worst a mis-pairing can produce is a wrong word on an entry that is still in
/// front of the reviewer, never a suppression missing from the review.
///
/// A directive is [`Change::JustificationEdited`] when the file's pre-image held
/// a directive of the same tool, in the same hunk, silencing the same rules over
/// the same scope, whose stated reason differs — a reason reworded, one written
/// where there was none, one removed entirely. Everything else is
/// [`Change::Added`], including a directive that was already there whose rules
/// or scope the change altered: that one silences something its base version did
/// not, and calling it a justification edit would assert something untrue.
///
/// Where there is no pre-image to compare against there is no edit, and every
/// directive in that file is [`Change::Added`].
pub fn classify(report: &mut Report, changes: &BTreeMap<String, FileChange>, tools: &[Tool]) {
    // The pairing is per file, and each pre-image is parsed once for all of it.
    let mut by_path: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, directive) in report.ignores.iter().enumerate() {
        by_path
            .entry(directive.path.as_str())
            .or_default()
            .push(index);
    }
    let mut verdicts = vec![Change::Added; report.ignores.len()];
    for (path, reported) in by_path {
        // No pre-image is no counterpart, and every directive in the file keeps
        // the `added` it started with.
        let Some((change, base)) = changes
            .get(path)
            .and_then(|change| Some((change, change.base.as_ref()?)))
        else {
            continue;
        };
        let previous = scan_source(
            &SourceFile::new(base.path.clone(), base.text.clone()),
            &ScanOptions {
                tools: tools.to_vec(),
            },
        );
        pair(
            report,
            &reported,
            &previous.ignores,
            &change.hunks,
            &mut verdicts,
        );
    }
    for (directive, verdict) in report.ignores.iter_mut().zip(verdicts) {
        directive.change = Some(verdict);
    }
}

/// Pair one file's reported directives with the ones its pre-image held, hunk by
/// hunk, writing a verdict for each.
///
/// Both sides are in file order, so where several directives of one tool sit in
/// one hunk on each side they pair in that order. A directive is "in" a hunk
/// when the hunk touches any line it occupies — the same relation on the base
/// side as on the new one, which is what lets a wrapped justification pair when
/// only its continuation line moved.
fn pair(
    report: &Report,
    reported: &[usize],
    previous: &[IgnoreDirective],
    hunks: &[Hunk],
    verdicts: &mut [Change],
) {
    let mut head_paired = vec![false; reported.len()];
    let mut base_used = vec![false; previous.len()];
    for hunk in hunks {
        let (Some(base_range), Some(new_range)) = (hunk.base, hunk.new) else {
            continue;
        };
        for (slot, &index) in reported.iter().enumerate() {
            if head_paired[slot] {
                continue;
            }
            let head = &report.ignores[index];
            if !overlaps(new_range, head.line, head.end_line) {
                continue;
            }
            let counterpart = previous.iter().enumerate().find(|(other, candidate)| {
                !base_used[*other]
                    && candidate.tool == head.tool
                    && overlaps(base_range, candidate.line, candidate.end_line)
            });
            let Some((other, candidate)) = counterpart else {
                continue;
            };
            base_used[other] = true;
            head_paired[slot] = true;
            if candidate.rules == head.rules
                && candidate.scope == head.scope
                && candidate.reason != head.reason
            {
                verdicts[index] = Change::JustificationEdited;
            }
        }
    }
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

    /// The hunks of a patch git is standing in for.
    fn hunks_of(patch: &str) -> FileChange {
        FileChange::parse(patch, "git diff").expect("a readable patch")
    }

    fn changed(diff: &Diff) -> Vec<(String, Option<String>)> {
        diff.changed_files()
            .expect("changed files")
            .files
            .into_iter()
            .map(|file| {
                (
                    display_path(&file.path),
                    file.renamed_from.as_deref().map(display_path),
                )
            })
            .collect()
    }

    fn change_for(diff: &Diff, path: &str) -> FileChange {
        diff.file_change(&changed_file(diff, path))
            .expect("the file's hunks")
    }

    /// The record `git diff --raw` produced for `path`.
    fn changed_file(diff: &Diff, path: &str) -> ChangedFile {
        diff.changed_files()
            .expect("changed files")
            .files
            .into_iter()
            .find(|file| display_path(&file.path) == path)
            .unwrap_or_else(|| panic!("{path} is not in the diff"))
    }

    #[test]
    fn hunk_headers_become_the_hunks_a_change_made() {
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
        let added = hunks_of(patch);
        assert!(added.intersects(2, 2) && added.intersects(4, 4));
        assert!(!added.intersects(1, 1) && !added.intersects(5, 11));
        assert!(added.intersects(12, 12));
        // A pure deletion adds nothing at the line it removed.
        assert!(!added.intersects(24, 24));
        assert!(!added.is_empty());
    }

    #[test]
    fn a_span_counts_when_any_of_its_lines_was_added() {
        let added = hunks_of("@@ -1,0 +5,2 @@\n+a\n+b\n");
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
            let error = FileChange::parse(patch, "git diff").unwrap_err();
            assert!(matches!(error, DiffError::Malformed { .. }), "{error:?}");
            assert!(error.to_string().contains("hunk header"), "{error}");
            assert!(error.hint().contains("--diff"), "{}", error.hint());
        }

        // A body line that merely looks like a header is body, not a header: a
        // patch of a patch still reads as one hunk of added text.
        let added = hunks_of("@@ -1,0 +9,2 @@\n+@@ -1,0 +3,3 @@\n+ @@ -1,0 +3,3 @@\n");
        assert!(added.intersects(9, 10) && !added.intersects(3, 3));
        assert!(FileChange::default().is_empty());
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
        assert!(change_for(&diff, "a.py").intersects(2, 2));
        assert!(!change_for(&diff, "a.py").intersects(1, 1));
    }

    #[test]
    fn an_unborn_head_diffs_the_index_instead_of_fataling() {
        let dir = repo();
        let root = dir.path();
        write(root, "a.py", "x = 1  # noqa\n");
        git(root, &["add", "a.py"]);

        let diff = Diff::open(root, None).expect("open diff");
        assert_eq!(changed(&diff), vec![("a.py".to_string(), None)]);
        assert!(change_for(&diff, "a.py").intersects(1, 1));
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
        assert!(change_for(&two_dot, "base_only.py").intersects(1, 1));
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
        assert!(change_for(&diff, "new.py").is_empty());
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

    /// A `--raw` record's info field, as git writes it.
    ///
    /// The mode pair and the destination id are real values this parser does not
    /// read, spelled the way git spells them so a record here is one git could
    /// have produced.
    const OLD: &str = "de980441c3ab03a8c07dda1ad27b8a11f39deb1e";
    const NEW: &str = "3e757656cf36eca53338e520d134963a44f793f8";

    fn info(status: &str) -> String {
        format!(":100644 100644 {OLD} {NEW} {status}\0")
    }

    /// A record: an info field carrying `status`, then its path(s).
    fn record(status: &str, paths: &[&[u8]]) -> Vec<u8> {
        let mut out = info(status).into_bytes();
        for path in paths {
            out.extend_from_slice(path);
            out.push(0);
        }
        out
    }

    /// A path git named in bytes this build cannot spell is set aside rather
    /// than decoded into a name that points at nothing.
    #[test]
    fn a_path_that_is_not_utf8_is_set_aside_rather_than_guessed_at() {
        let command = "git diff --raw";
        let mut output = record("M", &[b"caf\xe9.py"]);
        output.extend(record("A", &[b"plain.py"]));
        let changed = parse_raw(&output, command).unwrap();
        assert_eq!(
            changed.files,
            vec![ChangedFile {
                path: PathBuf::from("plain.py"),
                renamed_from: None,
                base_blob: BlobId::parse(OLD),
            }],
            "the file git *could* name is still part of the change"
        );
        assert_eq!(changed.undecodable, vec!["caf\u{fffd}.py"]);

        // Either end of a rename losing its name loses the pairing, so the whole
        // record is one this build cannot act on.
        for paths in [
            &[&b"caf\xe9.py"[..], b"new.py"],
            &[&b"old.py"[..], b"caf\xe9.py"],
        ] {
            let output = record("R100", paths);
            let changed = parse_raw(&output, command).unwrap();
            assert!(changed.files.is_empty(), "{paths:?}");
            assert_eq!(changed.undecodable, vec!["caf\u{fffd}.py"], "{paths:?}");
        }
    }

    #[test]
    fn truncated_raw_output_is_an_error_not_a_shorter_answer() {
        let command = "git diff --raw";
        assert_eq!(parse_raw(b"", command).unwrap(), Changed::default());
        for truncated in [record("M", &[]), record("R100", &[b"old.py"])] {
            let error = parse_raw(&truncated, command).unwrap_err();
            assert!(matches!(error, DiffError::Malformed { .. }), "{error:?}");
            assert!(error.to_string().contains("names no path"), "{error}");
        }

        // A status this build does not know — git's own "unknown" marker, or a
        // field that is not a status at all — is not read as a plain change.
        for unknown in ["X", "M100", "a.py"] {
            let output = record(unknown, &[b"a.py"]);
            let error = parse_raw(&output, command).unwrap_err();
            assert!(error.to_string().contains("unknown status"), "{error}");
        }

        // An info field this build cannot read at all — one git never wrote, or
        // one whose source id is not an object name to hand back to it — is a
        // failure rather than a record answered from its remaining fields.
        for unreadable in [
            &b"M\0a.py\0"[..],
            b":100644 100644 M\0a.py\0",
            b":100644 100644 not-an-oid 3e75765 M\0a.py\0",
            b":100644 100644  3e75765 M\0a.py\0",
        ] {
            let error = parse_raw(unreadable, command).unwrap_err();
            assert!(
                error.to_string().contains("unreadable record"),
                "{unreadable:?}: {error}"
            );
        }

        // Every status git does define is understood.
        for (status, paths, renamed_from) in [
            ("A", &[&b"a.py"[..]][..], None),
            ("D", &[b"a.py"], None),
            ("T", &[b"a.py"], None),
            ("U", &[b"a.py"], None),
            ("C75", &[b"old.py", b"a.py"], Some("old.py")),
            ("R", &[b"old.py", b"a.py"], Some("old.py")),
        ] {
            let output = record(status, paths);
            let changed = parse_raw(&output, command).unwrap();
            assert_eq!(changed.files.len(), 1, "{status}");
            assert_eq!(changed.files[0].path, PathBuf::from("a.py"), "{status}");
            assert_eq!(
                changed.files[0].renamed_from,
                renamed_from.map(PathBuf::from),
                "{status}"
            );
            assert!(changed.undecodable.is_empty(), "{status}");
        }

        let mut output = record("M", &[b"a.py"]);
        output.extend(record("A", &[b"b.py"]));
        assert_eq!(
            parse_raw(&output, command)
                .unwrap()
                .files
                .iter()
                .map(|file| display_path(&file.path))
                .collect::<Vec<_>>(),
            vec!["a.py", "b.py"]
        );
    }

    /// A file with no contents at the base carries no source blob: git writes
    /// the all-zero object id, and that is "there is nothing to compare".
    #[test]
    fn an_all_zero_source_id_means_the_file_had_no_pre_image() {
        let zeros = "0".repeat(40);
        let output = format!(":000000 100644 {zeros} {NEW} A\0new.py\0");
        let changed = parse_raw(output.as_bytes(), "git diff --raw").unwrap();
        assert_eq!(changed.files[0].base_blob, None);

        let output = format!(":100644 100644 {OLD} {NEW} M\0old.py\0");
        let changed = parse_raw(output.as_bytes(), "git diff --raw").unwrap();
        assert_eq!(
            changed.files[0].base_blob.as_ref().map(BlobId::as_str),
            Some(OLD)
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
            change: None,
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

        let mut changes = BTreeMap::new();
        changes.insert("a.py".to_string(), hunks_of("@@ -6,0 +7,1 @@\n+x\n"));
        retain_new(&mut report, &changes);

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
        let mut changes = BTreeMap::new();
        changes.insert("a.py".to_string(), hunks_of("@@ -7,0 +8,1 @@\n+  reason\n"));
        retain_new(&mut report, &changes);
        assert_eq!(report.ignores.len(), 1, "{report:#?}");

        // ...and one that ends before the first added line does not.
        let mut report = Report::new();
        report.ignores.push(directive("a.py", 4, 6));
        retain_new(&mut report, &changes);
        assert!(report.ignores.is_empty(), "{report:#?}");
    }

    /// The base side of every hunk header is read too, and a side with no lines
    /// on it is `None` rather than a range nothing can be inside.
    #[test]
    fn hunk_headers_carry_the_base_side_as_well_as_the_new_one() {
        let change = hunks_of(concat!(
            "@@ -2 +2 @@\n",
            "-old\n+new\n",
            "@@ -9,0 +10,2 @@\n",
            "+a\n+b\n",
            "@@ -20,2 +21,0 @@\n",
            "-gone\n-gone\n",
        ));
        assert_eq!(
            change.hunks,
            vec![
                Hunk {
                    base: Some((2, 2)),
                    new: Some((2, 2)),
                },
                // A pure insertion replaced nothing, so no base directive can be
                // inside it.
                Hunk {
                    base: None,
                    new: Some((10, 11)),
                },
                Hunk {
                    base: Some((20, 21)),
                    new: None,
                },
            ]
        );
        // Selection is unchanged by any of it: only the new side decides.
        assert!(change.intersects(2, 2) && change.intersects(10, 11));
        assert!(!change.intersects(21, 21));
    }

    /// A base side this build cannot read is an error for the same reason the
    /// new side is: the lines it covers would silently read as unchanged.
    #[test]
    fn an_unreadable_base_side_is_an_error_rather_than_a_guess() {
        for patch in [
            "@@ +1,2 @@\n",
            "@@ -not-a-number +1 @@\n",
            "@@ -1,x +1 @@\n",
        ] {
            let error = FileChange::parse(patch, "git diff").unwrap_err();
            assert!(matches!(error, DiffError::Malformed { .. }), "{error:?}");
            assert!(error.to_string().contains("hunk header"), "{error}");
        }
    }

    #[test]
    fn a_pre_image_is_the_files_contents_at_the_base() {
        let dir = repo();
        let root = dir.path();
        write(root, "a.py", "x = 1  # noqa: E501  # the old reason\n");
        commit(root, "baseline");
        write(root, "a.py", "x = 1  # noqa: E501  # the new reason\n");

        let diff = Diff::open(root, None).expect("open diff");
        let base = diff
            .pre_image(&changed_file(&diff, "a.py"))
            .expect("a pre-image");
        assert_eq!(base.path, PathBuf::from("a.py"));
        assert_eq!(base.text, "x = 1  # noqa: E501  # the old reason\n");
    }

    /// A rename's pre-image is the *source* path's blob, under the source's own
    /// name: that is the file whose language and directives these are.
    #[test]
    fn a_renamed_files_pre_image_is_read_under_its_old_name() {
        let dir = repo();
        let root = dir.path();
        // Long enough that git still sees a rename once the one line moves:
        // a rename it cannot see reads as a whole-file addition instead.
        let body = "a = 1\nb = 2\nc = 3\nd = 4\ne = 5\nf = 6\n";
        write(
            root,
            "old.py",
            &format!("{body}x = 1  # noqa: E501  # before\n"),
        );
        commit(root, "baseline");
        git(root, &["mv", "old.py", "new.py"]);
        write(
            root,
            "new.py",
            &format!("{body}x = 1  # noqa: E501  # after\n"),
        );

        let diff = Diff::open(root, None).expect("open diff");
        let file = changed_file(&diff, "new.py");
        assert_eq!(file.renamed_from, Some(PathBuf::from("old.py")));
        let base = diff.pre_image(&file).expect("a pre-image");
        assert_eq!(base.path, PathBuf::from("old.py"));
        assert_eq!(base.text, format!("{body}x = 1  # noqa: E501  # before\n"));
    }

    /// No previous contents, and previous contents that are not text, are both
    /// "nothing to compare against" — never a failure.
    #[test]
    fn a_file_with_no_readable_pre_image_answers_none_rather_than_failing() {
        let dir = repo();
        let root = dir.path();
        write(root, "kept.py", "x = 1\n");
        fs::write(root.join("binary.py"), [b'x', b' ', 0xff, 0xfe, b'\n']).unwrap();
        commit(root, "baseline");
        write(root, "created.py", "y = 2\n");
        write(root, "binary.py", "y = 2  # noqa: E501\n");
        git(root, &["add", "-A"]);

        let diff = Diff::open(root, None).expect("open diff");
        assert_eq!(diff.pre_image(&changed_file(&diff, "created.py")), None);
        assert_eq!(diff.pre_image(&changed_file(&diff, "binary.py")), None);
    }

    /// One file's change: the hunks of its patch, and what it used to say.
    fn change_of(patch: &str, base: Option<&str>) -> FileChange {
        let mut change = hunks_of(patch);
        change.set_base(base.map(|text| BaseImage {
            path: PathBuf::from("a.py"),
            text: text.to_string(),
        }));
        change
    }

    /// Classify a one-file report and read back the word on each record.
    fn verdicts(source: &str, patch: &str, base: Option<&str>) -> Vec<(u32, Change)> {
        let mut report = crate::scan::scan_source(
            &SourceFile::new("a.py", source.to_string()),
            &ScanOptions::default(),
        );
        let mut changes = BTreeMap::new();
        changes.insert("a.py".to_string(), change_of(patch, base));
        retain_new(&mut report, &changes);
        classify(&mut report, &changes, &[]);
        report
            .ignores
            .iter()
            .map(|directive| {
                (
                    directive.line,
                    directive.change.expect("a --diff run classifies"),
                )
            })
            .collect()
    }

    #[test]
    fn a_rewritten_reason_is_a_justification_edit_and_a_rewritten_rule_set_is_not() {
        // The same directive, the same rules, a different reason.
        assert_eq!(
            verdicts(
                "x = 1  # noqa: E501  # the new reason\n",
                "@@ -1 +1 @@\n",
                Some("x = 1  # noqa: E501  # the old reason\n"),
            ),
            vec![(1, Change::JustificationEdited)]
        );
        // A reason written where there was none, and one taken away.
        assert_eq!(
            verdicts(
                "x = 1  # noqa: E501  # now justified\n",
                "@@ -1 +1 @@\n",
                Some("x = 1  # noqa: E501\n"),
            ),
            vec![(1, Change::JustificationEdited)]
        );
        assert_eq!(
            verdicts(
                "x = 1  # noqa: E501\n",
                "@@ -1 +1 @@\n",
                Some("x = 1  # noqa: E501  # was justified\n"),
            ),
            vec![(1, Change::JustificationEdited)]
        );
        // The rule set moved: this now silences something its base version did
        // not, whatever happened to the words next to it.
        assert_eq!(
            verdicts(
                "x = 1  # noqa: E501,F401  # the new reason\n",
                "@@ -1 +1 @@\n",
                Some("x = 1  # noqa: E501  # the old reason\n"),
            ),
            vec![(1, Change::Added)]
        );
        // So did the scope.
        assert_eq!(
            verdicts(
                "# ruff: noqa: E501  # a file-wide reason\n",
                "@@ -1 +1 @@\n",
                Some("x = 1  # noqa: E501  # a line-wide reason\n"),
            ),
            vec![(1, Change::Added)]
        );
    }

    #[test]
    fn a_directive_with_no_counterpart_is_added() {
        // Nothing was there: the change wrote it.
        assert_eq!(
            verdicts(
                "x = 1\ny = 2  # noqa: E501  # brand new\n",
                "@@ -1,0 +2 @@\n",
                Some("x = 1\n"),
            ),
            vec![(2, Change::Added)]
        );
        // A file with no pre-image at all — one the change created, or one this
        // build cannot read as text.
        assert_eq!(
            verdicts("x = 1  # noqa: E501  # why\n", "@@ -0,0 +1 @@\n", None),
            vec![(1, Change::Added)]
        );
        // A counterpart of another tool is not a counterpart.
        assert_eq!(
            verdicts(
                "x = 1  # noqa: E501  # a ruff reason\n",
                "@@ -1 +1 @@\n",
                Some("x = 1  # type: ignore  # a mypy reason\n"),
            ),
            vec![(1, Change::Added)]
        );
    }

    /// Several directives of one tool in one hunk pair in file order.
    #[test]
    fn directives_in_one_hunk_pair_in_file_order() {
        assert_eq!(
            verdicts(
                "a = 1  # noqa: E501  # first, reworded\nb = 2  # noqa: E501  # second\n",
                "@@ -1,2 +1,2 @@\n",
                Some("a = 1  # noqa: E501  # first\nb = 2  # noqa: E501  # second\n"),
            ),
            vec![(1, Change::JustificationEdited), (2, Change::Added)]
        );
    }

    /// Classification labels and nothing else: the same records, in the same
    /// order, whatever the pre-image says.
    #[test]
    fn classification_never_moves_a_record() {
        let source = "a = 1  # noqa: E501  # reworded\nb = 2  # noqa: F401  # new\n";
        let mut changes = BTreeMap::new();
        changes.insert(
            "a.py".to_string(),
            change_of("@@ -1,2 +1,2 @@\n", Some("a = 1  # noqa: E501  # was\n")),
        );
        let mut report = crate::scan::scan_source(
            &SourceFile::new("a.py", source.to_string()),
            &ScanOptions::default(),
        );
        retain_new(&mut report, &changes);
        let before = report.clone();
        classify(&mut report, &changes, &[]);

        assert_eq!(report.ignores.len(), before.ignores.len());
        for (after, before) in report.ignores.iter().zip(&before.ignores) {
            assert_eq!(
                IgnoreDirective {
                    change: None,
                    ..after.clone()
                },
                *before
            );
        }
        assert_eq!(report.errors, before.errors);
    }
}
