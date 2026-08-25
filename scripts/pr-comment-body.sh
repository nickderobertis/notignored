#!/usr/bin/env bash
# Print the comment body the rendered-comment capture photographs.
#
# Separate from the render half so this one — the review case, and therefore
# every character the picture shows — can be driven directly by
# `tests/e2e/pr_comment.rs`, the way scripts/action/counts.sh is. The half it
# leaves out (scripts/comment-render/render.mjs, which `just
# screenshots-pr-comment` pipes this into) needs a browser, which the gate must
# never.
#
# It builds the same throwaway review repository scripts/screenshots.sh builds
# for its `diff` and `pr-comment` scenes: the committed screenshots/fixture/ tree
# committed as the base, then the screenshots/change/ overlay laid on top as the
# uncommitted work a reviewer is looking at.
#
# It does not build: `just screenshots-pr-comment` does that first, the way
# `just screenshots-gif` does. NOTIGNORED_BIN drives a binary other than the
# release one (the suite points it at the one cargo already compiled).
set -euo pipefail

# llmlint: ignore-block[changed_behavior_has_e2e] reaching this means the
# checkout disappearing under the script mid-run, which a test would have to do
# to the tree the rest of the suite is reading.
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root" || {
  echo "pr-comment-body: cannot enter the repository root $repo_root" >&2
  echo "ACTION: run this from a checkout whose directories are readable, then re-run 'just screenshots-pr-comment'" >&2
  exit 1
}
# llmlint: ignore-end[changed_behavior_has_e2e]

# Same reason as scripts/screenshots.sh: git exports GIT_DIR (and friends) to the
# hooks it runs, and inherited they would aim the throwaway repository built
# below — and the `--diff` run driven inside it — back at this checkout.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_PREFIX GIT_COMMON_DIR \
  GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_QUARANTINE_PATH

# The commit the permalinks are pinned to. The same literal scripts/screenshots.sh
# uses, for the same reason — a capture cannot know the sha of the commit that
# adds it — and `tests/screenshots_contract.rs` fails the build if the two ever
# stop agreeing.
permalink_sha="0123456789abcdef0123456789abcdef01234567"

bin="${NOTIGNORED_BIN:-$repo_root/target/release/notignored}"
if [ ! -x "$bin" ]; then
  echo "pr-comment-body: no notignored binary at $bin" >&2
  echo "ACTION: run 'just screenshots-pr-comment', which builds the release binary first" >&2
  exit 1
fi

tmp_parent="${TMPDIR:-/tmp}"
tmp_state="$(mktemp -d "$tmp_parent/notignored-pr-comment.XXXXXX")" || {
  echo "pr-comment-body: cannot create a scratch directory beneath TMPDIR=$tmp_parent" >&2
  echo "ACTION: check \$TMPDIR exists and has space (df -h), then re-run 'just screenshots-pr-comment'" >&2
  exit 1
}
trap 'rm -rf "$tmp_state"' EXIT

# llmlint: ignore-block[changed_behavior_has_e2e] every branch below reports the
# host breaking under the script mid-run — an unreadable fixture, a git that will
# not init, a report the binary declines to produce — and reaching one from a
# test means sabotaging the checkout the rest of the suite is reading.
diff_repo="$tmp_state/review"
mkdir -p "$diff_repo" && cp -R "$repo_root/screenshots/fixture/." "$diff_repo/" || {
  echo "pr-comment-body: cannot stage screenshots/fixture/ into $diff_repo" >&2
  echo "ACTION: check that screenshots/fixture/ is readable and \$TMPDIR is writable, then re-run 'just screenshots-pr-comment'" >&2
  exit 1
}
git_log="$tmp_state/git.log"
(
  cd "$diff_repo"
  # The git config is neutralized and the identity fixed, so a developer's own
  # gitconfig — signing, hooks, autocrlf, a default branch name — cannot change
  # what is captured.
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
  export GIT_AUTHOR_NAME=notignored GIT_AUTHOR_EMAIL=notignored@example.invalid
  export GIT_COMMITTER_NAME=notignored GIT_COMMITTER_EMAIL=notignored@example.invalid
  git -c init.defaultBranch=main init -q
  git add -A
  git commit -q -m "the tree as it stood before this change"
) >"$git_log" 2>&1 || {
  # Captured rather than discarded: git says exactly what went wrong (a missing
  # binary, a refused hook, an unwritable object store) and the guess this
  # script could make instead would send a reader somewhere else.
  echo "pr-comment-body: cannot build the throwaway review repository in $diff_repo" >&2
  cat "$git_log" >&2
  echo "ACTION: fix what git reports above; it runs here with an empty config, so the" >&2
  echo "        cause is the host rather than your gitconfig. Then re-run" >&2
  echo "        'just screenshots-pr-comment'." >&2
  exit 1
}
cp -R "$repo_root/screenshots/change/." "$diff_repo/" || {
  echo "pr-comment-body: cannot lay screenshots/change/ over the base tree" >&2
  echo "ACTION: check that screenshots/change/ is readable, then re-run 'just screenshots-pr-comment'" >&2
  exit 1
}

body="$tmp_state/body.md"
(cd "$diff_repo" && "$bin" . --diff --format markdown \
  --github-repo nickderobertis/notignored --github-sha "$permalink_sha") >"$body" || {
  echo "pr-comment-body: '$bin --diff --format markdown' failed over the review repository" >&2
  echo "ACTION: run it by hand in a copy of screenshots/fixture/ with screenshots/change/ laid over it and read what it reports" >&2
  exit 1
}
if [ ! -s "$body" ]; then
  {
    echo "pr-comment-body: the binary produced no comment body."
    echo "ACTION: run '$bin . --diff --format markdown' inside a copy of"
    echo "        screenshots/fixture/ with screenshots/change/ laid over it and see"
    echo "        what it prints; an empty body means the change added no suppression."
  } >&2
  exit 1
fi
cat "$body" || {
  echo "pr-comment-body: cannot write the comment body to stdout" >&2
  echo "ACTION: check what this script is piped into — 'just screenshots-pr-comment' pipes it into the renderer" >&2
  exit 1
}

# llmlint: ignore-end[changed_behavior_has_e2e]
