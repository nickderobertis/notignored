#!/usr/bin/env bash
# Strict type-check of the Python SDK: its sources AND its suite.
#
# The suite is in scope because it is the SDK's executable specification — a
# test that calls `scan()` with the wrong argument types is a test asserting
# something the API does not promise, and only a type checker catches that.
#
# The pinned mypy lives in its own venv (`.dev/mypy`) with nothing else in it,
# so on its own it cannot resolve pytest and would report missing stubs instead
# of real errors. `--python-executable` points it at the project's own
# interpreter for import resolution, which keeps `.mypy-version` the single pin
# rather than adding a second one to the project's dev group.
#
# Quiet on success; mypy's own diagnostics are the failure output.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT="$ROOT/python/notignored-sdk"

PYTHON=""
for candidate in "$PROJECT/.venv/bin/python" "$PROJECT/.venv/Scripts/python.exe"; do
  if [ -x "$candidate" ]; then
    PYTHON="$candidate"
    break
  fi
done

if [ -z "$PYTHON" ]; then
  echo "python-sdk-typecheck: the SDK's virtualenv is missing, so mypy cannot resolve pytest" >&2
  echo "ACTION: run 'just bootstrap' (or 'uv sync --locked' in python/notignored-sdk)" >&2
  exit 1
fi

cd "$PROJECT"
exec bash "$ROOT/scripts/dev-tool.sh" mypy --python-executable "$PYTHON" src tests
