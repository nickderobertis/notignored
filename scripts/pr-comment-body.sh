#!/usr/bin/env bash
# Print the comment body the rendered-comment capture photographs.
#
# Separate from scripts/pr-comment-png.sh so this half — the review case, and
# therefore every character the picture shows — can be driven directly by
# `tests/e2e/pr_comment.rs`, the way scripts/action/counts.sh is. The half it
# leaves out needs a browser, which the gate must never.
#
# It builds the same throwaway review repository scripts/screenshots.sh builds
# for its `diff` and `pr-comment` scenes: the committed screenshots/fixture/ tree
# committed as the base, then the screenshots/change/ overlay laid on top as the
# uncommitted work a reviewer is looking at.
#
# NOTIGNORED_BIN drives a binary other than the release build (the suite points
# it at the one cargo already compiled); SCREENSHOTS_NO_BUILD skips the build
# when the release binary is already there.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

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
if [ -z "${NOTIGNORED_BIN:-}" ] \
  && { [ -z "${SCREENSHOTS_NO_BUILD:-}" ] || [ ! -x "$bin" ]; }; then
  cargo build --release --locked --bin notignored >&2
fi

tmp_state="$(mktemp -d)"
trap 'rm -rf "$tmp_state"' EXIT

diff_repo="$tmp_state/review"
mkdir -p "$diff_repo"
cp -R "$repo_root/screenshots/fixture/." "$diff_repo/"
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
) >/dev/null 2>&1
cp -R "$repo_root/screenshots/change/." "$diff_repo/"

body="$tmp_state/body.md"
(cd "$diff_repo" && "$bin" . --diff --format markdown \
  --github-repo nickderobertis/notignored --github-sha "$permalink_sha") >"$body"
if [ ! -s "$body" ]; then
  {
    echo "pr-comment-body: the binary produced no comment body."
    echo "ACTION: run '$bin . --diff --format markdown' inside a copy of"
    echo "        screenshots/fixture/ with screenshots/change/ laid over it and see"
    echo "        what it prints; an empty body means the change added no suppression."
  } >&2
  exit 1
fi
cat "$body"
