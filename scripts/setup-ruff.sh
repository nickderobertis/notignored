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
# `.ruff-version` feeds a package requirement, so validate its shape before use
# rather than letting arbitrary file contents reach the resolver.
# Reject anything outside digits and dots before checking the X.Y.Z shape, so no
# extra requirement specifier or shell metacharacter can reach the resolver.
case "$PIN" in
  *[!0-9.]* | *..*) PIN="" ;;
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) PIN="" ;;
esac
if [ -z "$PIN" ]; then
  echo "setup-ruff: .ruff-version must hold a version like 0.16.1" >&2
  echo "ACTION: write a published ruff version (see https://pypi.org/project/ruff/#history)" >&2
  exit 1
fi
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
uv venv --quiet "$VENV" || {
  echo "setup-ruff: cannot create the venv at $VENV" >&2
  echo "ACTION: check disk space and that 'uv --version' works, then re-run 'just setup-ruff'" >&2
  exit 1
}
uv pip install --quiet --python "$VENV" "ruff==$PIN" || {
  echo "setup-ruff: cannot install ruff==$PIN" >&2
  echo "ACTION: check network access to PyPI, or set .ruff-version to a published release" >&2
  exit 1
}
