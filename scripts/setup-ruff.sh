#!/usr/bin/env bash
# Install the pinned `ruff` the e2e parity suite drives.
#
# The parity tests prove that what notignored reports is what ruff actually
# suppresses, so they run the REAL ruff — never a stub. Pinning it makes that
# proof reproducible: `.ruff-version` is the single source of truth, read here
# and asserted by the tests against `ruff --version`, so a stray system ruff can
# never silently stand in for the pinned one.
#
# Installs into a project-local venv (`.dev/ruff`) so the path is the same on
# every machine and nothing leaks into the user's global tools. Idempotent and
# quiet on success.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PIN="$(tr -d '[:space:]' < .ruff-version)"
VENV=".dev/ruff"
if [ -x "$VENV/bin/ruff" ]; then
  BIN="$VENV/bin/ruff"
elif [ -x "$VENV/Scripts/ruff.exe" ]; then
  BIN="$VENV/Scripts/ruff.exe"
else
  BIN=""
fi

if [ -n "$BIN" ] && "$BIN" --version 2>/dev/null | grep -qx "ruff $PIN"; then
  exit 0
fi

if ! command -v uv >/dev/null 2>&1; then
  echo "setup-ruff: uv not found; cannot install the pinned ruff ($PIN)" >&2
  echo "ACTION: install uv (https://docs.astral.sh/uv/) and re-run 'just bootstrap'" >&2
  exit 1
fi

rm -rf "$VENV"
uv venv --quiet "$VENV"
uv pip install --quiet --python "$VENV" "ruff==$PIN"
