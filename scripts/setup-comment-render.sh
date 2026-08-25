#!/usr/bin/env bash
# Install the pinned markdown / syntax-highlighting / browser toolchain that
# `just screenshots-pr-comment` renders the pull-request comment PNGs with.
#
# Same shape as scripts/setup-js.sh: `scripts/comment-render/package-lock.json`
# is the single source of truth for the versions, installed into a project-local
# tree (`.dev/comment-render`) so nothing ever lands in the repository's own
# node_modules. Chromium goes to `.dev/comment-render/browsers` for the same
# reason — a screenshot toolchain must not write into a developer's shared
# ~/.cache.
#
# Deliberately NOT wired into `just bootstrap`, `just check`, or any CI
# workflow: the capture is informational (see screenshots/AGENTS.md), and a
# browser download must never be something an ordinary contributor has to make
# it through to run the gate. `just screenshots-pr-comment` calls this on demand.
#
# Idempotent and quiet on success.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || {
  echo "$(basename "${BASH_SOURCE[0]}"): cannot enter the repository root $ROOT" >&2
  echo "ACTION: run this from a checkout whose directories are readable, then re-run 'just screenshots-comment-tools'" >&2
  exit 1
}

MANIFEST="scripts/comment-render"
TOOLCHAIN=".dev/comment-render"
export PLAYWRIGHT_BROWSERS_PATH="$ROOT/$TOOLCHAIN/browsers"

if ! command -v npm >/dev/null 2>&1; then
  echo "setup-comment-render: npm not found; cannot install the pinned markdown-it/shiki/playwright" >&2
  echo "ACTION: install Node.js 20+ (https://nodejs.org/) and re-run 'just screenshots-comment-tools'" >&2
  exit 1
fi
# The pinned packages are ESM-first and Playwright's CLI needs a modern runtime,
# so "npm exists" is not the requirement — a specific floor is. `|| true` because
# `pipefail` would otherwise make a missing or broken `node` abort the script
# under `set -e`, silently, before the branch below can say which it was.
node_major="$(node --version 2>/dev/null | sed -n 's/^v\([0-9][0-9]*\).*/\1/p' || true)"
if [ -z "$node_major" ] || [ "$node_major" -lt 20 ]; then
  echo "setup-comment-render: Node.js 20+ is required (found ${node_major:-none})" >&2
  echo "ACTION: install or select Node.js 20+ (https://nodejs.org/) and re-run 'just screenshots-comment-tools'" >&2
  exit 1
fi

# `npm ci` reinstalls from scratch every time, and the browser download is worse
# still. Skip both when the installed tree already matches the lockfile we are
# about to copy in and a browser is already unpacked beside it.
if [ -d "$TOOLCHAIN/node_modules" ] \
  && [ -d "$TOOLCHAIN/browsers" ] \
  && cmp -s "$MANIFEST/package-lock.json" "$TOOLCHAIN/package-lock.json" \
  && cmp -s "$MANIFEST/package.json" "$TOOLCHAIN/package.json"; then
  exit 0
fi

mkdir -p "$TOOLCHAIN" || {
  echo "setup-comment-render: cannot create $ROOT/$TOOLCHAIN" >&2
  echo "ACTION: check the directory is writable (or remove a stale file at that path), then re-run 'just screenshots-comment-tools'" >&2
  exit 1
}
cp "$MANIFEST/package.json" "$MANIFEST/package-lock.json" "$TOOLCHAIN/" || {
  echo "setup-comment-render: cannot copy the pinned manifest from $MANIFEST into $ROOT/$TOOLCHAIN" >&2
  echo "ACTION: check that $MANIFEST/package.json and package-lock.json exist, then re-run 'just screenshots-comment-tools'" >&2
  exit 1
}
# llmlint: ignore-block[changed_behavior_has_e2e] both branches below are the
# network install itself — a test of either would download the browser the gate
# is deliberately kept clear of (screenshots/AGENTS.md).
# --ignore-scripts: playwright's postinstall would fetch a browser into the
# ambient cache. We fetch it ourselves, below, into the project-local tree.
if ! (cd "$TOOLCHAIN" && npm ci --silent --no-audit --no-fund --ignore-scripts); then
  # A half-written tree would make the skip above lie on the next run. `|| true`
  # so a cleanup that itself fails cannot pre-empt the message that says what to
  # do about the install.
  rm -f "$TOOLCHAIN/package-lock.json" "$TOOLCHAIN/package.json" || true
  echo "setup-comment-render: 'npm ci' failed in $ROOT/$TOOLCHAIN" >&2
  echo "ACTION: check network access to the npm registry, then re-run 'just screenshots-comment-tools'" >&2
  exit 1
fi

if ! (cd "$TOOLCHAIN" && node node_modules/playwright/cli.js install chromium >/dev/null); then
  rm -f "$TOOLCHAIN/package-lock.json" "$TOOLCHAIN/package.json" || true
  echo "setup-comment-render: could not download Chromium into $ROOT/$TOOLCHAIN/browsers" >&2
  echo "ACTION: check network access to https://cdn.playwright.dev, then re-run 'just screenshots-comment-tools'" >&2
  exit 1
fi

# llmlint: ignore-end[changed_behavior_has_e2e]
