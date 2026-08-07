//! `scripts/update-major-tag.sh`: the floating `v0` the README tells action
//! consumers to write.
//!
//! The tag is what every consumer of the GitHub Action resolves, and the only
//! place it is ever moved for real is the last job of a release — where a
//! mistake is public before anyone can read the log. So the whole decision is
//! driven here over **real** repositories: a bare repo standing in for origin, a
//! clone pushing real tags to it, and the release job's own script deciding what
//! `v0` should point at. Nothing about git is simulated; only the runner is.
//!
//! The four decisions that are not visible from reading the script:
//!
//!   * the newest release wins *numerically* — `v0.1.10` is newer than `v0.1.9`,
//!     which the lexical order `sort` and `git tag` default to gets backwards;
//!   * the major is derived, so `v1.0.0` starts moving `v1` with no edit, and
//!     without disturbing `v0`;
//!   * re-cutting an older release leaves the floating tag alone, rather than
//!     walking every consumer backwards;
//!   * a pre-release, and a tag that is not a version at all, move nothing.

use std::path::Path;
use std::process::{Command, Output};

use crate::support::{bash_program, commit, git, git_repo, git_stdout, repo_root, write};

/// A bare repository standing in for origin, and a clone of it that pushes.
///
/// The push is what the script actually has to get right — a local `git tag`
/// that never reached a remote would look identical from inside the clone — so
/// every assertion below reads the *bare* repository, not the clone.
struct Origin {
    bare: tempfile::TempDir,
    clone: tempfile::TempDir,
}

impl Origin {
    fn new() -> Self {
        let bare = tempfile::tempdir().expect("tempdir");
        git(bare.path(), &["init", "--bare", "-q"]);
        let clone = git_repo();
        git(
            clone.path(),
            &["remote", "add", "origin", &bare.path().to_string_lossy()],
        );
        write(clone.path(), "README.md", "# fixture\n");
        commit(clone.path(), "chore: initial");
        git(clone.path(), &["push", "-q", "origin", "main"]);
        Self { bare, clone }
    }

    /// Commit, tag, and push a release the way release-plz does — the tag on
    /// origin is the only thing the script is allowed to work from.
    ///
    /// **Annotated**, because that is what release-plz creates: an annotated tag
    /// names a tag object, not a commit, so anything that read it without
    /// peeling would point the floating major at an object no consumer can check
    /// out.
    fn release(&self, tag: &str) -> String {
        write(self.clone.path(), "CHANGELOG.md", &format!("## {tag}\n"));
        commit(self.clone.path(), &format!("chore: release {tag}"));
        git(self.clone.path(), &["tag", "-a", tag, "-m", tag]);
        git(self.clone.path(), &["push", "-q", "origin", "main"]);
        git(
            self.clone.path(),
            &["push", "-q", "origin", &format!("refs/tags/{tag}")],
        );
        self.commit_of(tag).expect("the release tag was pushed")
    }

    /// The commit a tag on origin points at, or `None` when origin has no such
    /// tag.
    fn commit_of(&self, tag: &str) -> Option<String> {
        let output = Command::new("git")
            .args(["rev-list", "--max-count=1", tag])
            .current_dir(self.bare.path())
            .output()
            .expect("run git");
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// The kind of object a ref on origin names — `commit` for the floating tag,
    /// `tag` for the annotated release tags it is derived from.
    fn object_type(&self, reference: &str) -> String {
        git_stdout(self.bare.path(), &["cat-file", "-t", reference])
            .trim()
            .to_string()
    }

    /// Run the release job's script against this origin, from the clone.
    fn update(&self, tag: &str) -> Output {
        run(self.clone.path(), &["--tag", tag])
    }
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(bash_program())
        .arg(repo_root().join("scripts/update-major-tag.sh"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run scripts/update-major-tag.sh")
}

/// The script's own stdout, for the assertions that care what a release log says.
fn succeeds(output: &Output) -> String {
    assert!(
        output.status.success(),
        "update-major-tag.sh failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn fails(output: &Output) -> String {
    assert!(
        !output.status.success(),
        "update-major-tag.sh succeeded when it should not have: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The first release creates the floating tag; a later one moves it.
///
/// `v0.1.9` is tagged *after* `v0.1.10` on purpose: creation order and lexical
/// order both name it the newest, and only the numeric comparison the script
/// makes gets `v0.1.10` right. Without it, a release would silently decline to
/// move the tag it exists to move.
#[test]
fn the_floating_major_tag_follows_the_newest_release() {
    let origin = Origin::new();
    let first = origin.release("v0.1.9");
    origin.update("v0.1.9");
    assert_eq!(origin.commit_of("v0").as_deref(), Some(first.as_str()));

    let newest = origin.release("v0.1.10");
    let log = succeeds(&origin.update("v0.1.10"));
    assert_eq!(
        origin.commit_of("v0").as_deref(),
        Some(newest.as_str()),
        "v0 did not move to the newest release; 0.1.9 sorts above 0.1.10 lexically"
    );
    assert!(
        log.contains("v0 -> v0.1.10"),
        "a release log has to name what moved: {log}"
    );
    // Release tags are annotated, so the release commit is one dereference down
    // from the tag ref. `@v0` has to name the commit itself.
    assert_eq!(origin.object_type("v0.1.10"), "tag");
    assert_eq!(
        origin.object_type("v0"),
        "commit",
        "v0 names the annotated tag object rather than the commit it points at"
    );
}

/// 1.0.0 starts maintaining `v1` with nothing to edit, and leaves `v0` where the
/// last 0.x release put it — the whole point of a floating *major*.
#[test]
fn the_major_is_derived_from_the_release_and_leaves_the_others_alone() {
    let origin = Origin::new();
    let last_zero = origin.release("v0.1.10");
    origin.update("v0.1.10");

    let one = origin.release("v1.0.0");
    let log = succeeds(&origin.update("v1.0.0"));
    assert!(
        log.contains("v1 -> v1.0.0"),
        "the major has to be derived from the release tag: {log}"
    );
    assert_eq!(origin.commit_of("v1").as_deref(), Some(one.as_str()));
    assert_eq!(
        origin.commit_of("v0").as_deref(),
        Some(last_zero.as_str()),
        "a 1.0.0 release must not drag v0 consumers onto a breaking change"
    );
}

/// Re-cutting an older release — a re-run, or a patch off an old branch — must
/// not walk every `@v0` consumer backwards.
#[test]
fn an_older_release_leaves_the_floating_tag_where_it_is() {
    let origin = Origin::new();
    origin.release("v0.1.9");
    let newest = origin.release("v0.1.10");
    origin.update("v0.1.10");

    let log = succeeds(&origin.update("v0.1.9"));
    assert_eq!(
        origin.commit_of("v0").as_deref(),
        Some(newest.as_str()),
        "v0 moved backwards onto an older release"
    );
    assert!(
        log.contains("v0.1.10 is a newer v0 release than v0.1.9"),
        "declining to move has to say why: {log}"
    );
}

/// `@v1` means the newest stable 1.x, so a pre-release cut from the same major
/// leaves it alone — including the very first one, where the tag does not exist
/// yet and a careless move would create it.
#[test]
fn a_pre_release_never_becomes_the_floating_major() {
    let origin = Origin::new();
    let stable = origin.release("v1.0.0");
    origin.update("v1.0.0");

    origin.release("v1.1.0-rc.1");
    let log = succeeds(&origin.update("v1.1.0-rc.1"));
    assert_eq!(
        origin.commit_of("v1").as_deref(),
        Some(stable.as_str()),
        "a pre-release became what @v1 resolves to"
    );
    assert!(
        log.contains("pre-release"),
        "declining to move has to say why: {log}"
    );

    let origin = Origin::new();
    origin.release("v2.0.0-rc.1");
    succeeds(&origin.update("v2.0.0-rc.1"));
    assert_eq!(
        origin.commit_of("v2"),
        None,
        "a pre-release created a floating major tag with no stable release behind it"
    );
}

/// A Release can be cut by hand with any tag at all, and that tag reaches git as
/// a revision and origin as a refspec. Anything but a version fails loudly and
/// moves nothing.
#[test]
fn a_tag_that_is_not_a_version_moves_nothing() {
    let origin = Origin::new();
    origin.release("v0.1.10");
    origin.update("v0.1.10");

    for tag in ["nightly", "v1x.2y.3z", "v0.1", "release/v0.2.0"] {
        let error = fails(&origin.update(tag));
        assert!(
            error.contains("is not a vX.Y.Z version tag") && error.contains("ACTION:"),
            "'{tag}' was rejected without saying what to do: {error}"
        );
    }
    assert_eq!(
        origin.commit_of("v1"),
        None,
        "a rejected tag still created a floating major tag"
    );
}

/// The newest-release comparison is only as good as the tags this clone can see,
/// so an origin it cannot reach is an error rather than a tag moved on a guess.
#[test]
fn an_unreachable_origin_fails_instead_of_guessing() {
    let origin = Origin::new();
    let released = origin.release("v0.1.10");
    origin.update("v0.1.10");
    origin.release("v0.2.0");

    let error = fails(&run(
        origin.clone.path(),
        &["--tag", "v0.2.0", "--remote", "nowhere"],
    ));
    assert!(
        error.contains("cannot fetch tags from 'nowhere'") && error.contains("ACTION:"),
        "an unreachable remote was reported without saying what to do: {error}"
    );
    assert_eq!(
        origin.commit_of("v0").as_deref(),
        Some(released.as_str()),
        "the tag moved despite the fetch failing"
    );
}

/// Outside a repository there is nothing to tag — the one failure that is a
/// wrong invocation rather than a wrong release.
#[test]
fn a_directory_that_is_not_a_repository_is_an_error() {
    let elsewhere = tempfile::tempdir().expect("tempdir");
    // `git rev-parse` walks up to any enclosing repository, so the check only
    // means anything somewhere that has none above it.
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(elsewhere.path())
        .output()
        .expect("run git");
    if output.status.success() {
        return;
    }
    let error = fails(&run(elsewhere.path(), &["--tag", "v0.1.10"]));
    assert!(
        error.contains("not inside a git repository") && error.contains("ACTION:"),
        "{error}"
    );
}

/// Argument handling: a missing value must not silently become an empty setting.
#[test]
fn an_argument_without_its_value_is_a_usage_error() {
    let origin = Origin::new();
    for args in [vec!["--tag"], vec!["--remote"], vec![], vec!["--nope"]] {
        let error = fails(&run(origin.clone.path(), &args));
        assert!(
            error.contains("update-major-tag.sh --tag vX.Y.Z"),
            "{args:?} was rejected without the usage: {error}"
        );
    }
    assert!(succeeds(&run(origin.clone.path(), &["--help"])).contains("--tag vX.Y.Z"));
}

/// The tag the release job moves is the one the dogfooded repository would push:
/// this asserts the script is the only thing that decides it, by running it from
/// a checkout whose HEAD is not the release commit at all.
#[test]
fn the_tag_points_at_the_release_commit_not_the_checked_out_head() {
    let origin = Origin::new();
    let released = origin.release("v0.1.10");
    write(origin.clone.path(), "src.rs", "// unreleased work\n");
    commit(origin.clone.path(), "feat: after the release");
    let head = git_stdout(origin.clone.path(), &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    assert_ne!(head, released);

    succeeds(&origin.update("v0.1.10"));
    assert_eq!(
        origin.commit_of("v0").as_deref(),
        Some(released.as_str()),
        "v0 followed the checkout's HEAD instead of the release tag"
    );
}
