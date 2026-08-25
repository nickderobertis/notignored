#!/usr/bin/env bash
# Capture the rendered pull-request comment the README's marketing passage shows
# (docs/screenshots/pr-comment-rendered.png and -dark.png).
#
# It is the `pr-comment` scene's sibling: that one photographs the markdown
# SOURCE `--format markdown` emits, this one what a reviewer meets once GitHub
# has rendered it. The body comes from scripts/pr-comment-body.sh — the same
# review case, driven by the real binary — so only the STYLING is a local mimic
# (scripts/comment-render/, and screenshots/AGENTS.md for why).
#
# Informational: not hash-gated, not in the gate, not in CI. Regenerate on demand
# and commit the result.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

docs_dir="$repo_root/docs/screenshots"

if ! command -v node >/dev/null 2>&1; then
  echo "pr-comment-png: node not found; the render needs Node.js 20+" >&2
  echo "ACTION: install Node.js (https://nodejs.org/) and re-run 'just screenshots-pr-comment'" >&2
  exit 1
fi
bash "$repo_root/scripts/setup-comment-render.sh"

mkdir -p "$docs_dir"
bash "$repo_root/scripts/pr-comment-body.sh" \
  | node "$repo_root/scripts/comment-render/render.mjs" \
    "$docs_dir/pr-comment-rendered.png" \
    "$docs_dir/pr-comment-rendered-dark.png"

echo "pr-comment-png: wrote docs/screenshots/pr-comment-rendered{,-dark}.png" >&2
