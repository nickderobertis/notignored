#!/usr/bin/env bash
# Install the pinned eslint / biome / tsc the e2e parity suite drives.
#
# Same contract as scripts/setup-python-tools.sh: the parity tests run the REAL linters,
# so a stub would prove nothing. `tests/js-toolchain/package-lock.json` is the
# single source of truth for the versions, read here via `npm ci` and asserted by
# the tests against each tool's `--version`, so a stray global eslint can never
# stand in for the pinned one.
#
# Installs into a project-local tree (`.dev/js`) rather than beside the manifest
# so no `node_modules` ever appears under `tests/`. Idempotent and quiet on
# success.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MANIFEST="tests/js-toolchain"
VENV=".dev/js"

if ! command -v npm >/dev/null 2>&1; then
  echo "setup-js: npm not found; cannot install the pinned eslint/biome/tsc" >&2
  echo "ACTION: install Node.js 20+ (https://nodejs.org/) and re-run 'just bootstrap'" >&2
  exit 1
fi

# `npm ci` reinstalls from scratch every time, which is slow enough to notice on
# a warm tree. Skip it when the installed tree already matches the lockfile we
# are about to copy in.
if [ -d "$VENV/node_modules" ] \
  && cmp -s "$MANIFEST/package-lock.json" "$VENV/package-lock.json" \
  && cmp -s "$MANIFEST/package.json" "$VENV/package.json"; then
  exit 0
fi

mkdir -p "$VENV"
cp "$MANIFEST/package.json" "$MANIFEST/package-lock.json" "$VENV/"
# --ignore-scripts: nothing in this tree needs a postinstall, and the pinned
# linters are dev inputs, not code we run in production. Least privilege.
if ! (cd "$VENV" && npm ci --silent --no-audit --no-fund --ignore-scripts); then
  # A half-written tree would make the skip above lie on the next run.
  rm -f "$VENV/package-lock.json" "$VENV/package.json"
  echo "setup-js: 'npm ci' failed in $ROOT/$VENV" >&2
  echo "ACTION: check network access to the npm registry, then re-run 'just setup-js'" >&2
  exit 1
fi
