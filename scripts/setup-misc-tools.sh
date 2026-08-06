#!/usr/bin/env bash
# Install the pinned ShellCheck and llmlint the e2e parity suites drive.
#
# The parity tests prove that what notignored reports is what each tool actually
# suppresses, so they run the REAL tools — never a stub. Pinning makes that proof
# reproducible: `.<tool>-version` is the single source of truth, read here and
# asserted by the tests against each binary's own `--version`, so a stray system
# install can never silently stand in for the pinned one.
#
# Both ship a PyPI wheel for Linux, macOS, and Windows, so one installer covers
# every platform CI runs on. Each lands in its own project-local venv
# (`.dev/<tool>`) so the path is the same on every machine, one tool's
# dependencies can never constrain another's, and nothing leaks into the user's
# global tools. Idempotent and quiet on success.
#
# Companion installers own the other pins: `scripts/setup-python-tools.sh` the
# Python linters, and `rust-toolchain.toml` the toolchain rustup resolves
# `clippy-driver` from — the Rust half of the suite needs no entry here.
#
# This is NOT `scripts/setup-llmlint.sh`, which provisions the harness-backed
# `llmlint` this repo lints *itself* with. The pin below is a test fixture.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# `<tool> <pip requirement>`. The venv directory, the binary, and the pin file
# are all named after the tool; only the requirement can differ.
TOOLS=(
  "shellcheck shellcheck-py"
  "llmlint llmlint-cli"
)

# The pin as written in `.<tool>-version`, validated before it reaches uv.
read_pin() {
  local tool="$1" package="$2" pin
  pin="$(tr -d '[:space:]' < ".$tool-version")" || {
    echo "setup-misc-tools: cannot read $ROOT/.$tool-version" >&2
    echo "ACTION: restore it from git ('git checkout -- .$tool-version')" >&2
    exit 1
  }
  # The pin feeds a package requirement, so validate its shape before use rather
  # than letting arbitrary file contents reach the resolver. Three or four
  # numeric components — `shellcheck-py` adds a fourth for its own packaging
  # revision — and nothing else: a glob alone would let a trailing requirement
  # specifier through.
  if ! printf '%s' "$pin" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)?$'; then
    echo "setup-misc-tools: .$tool-version must hold a version like 0.11.0.1" >&2
    echo "ACTION: write a published $tool version (see https://pypi.org/project/$package/#history)" >&2
    exit 1
  fi
  printf '%s' "$pin"
}

# The tool's binary inside its venv, or nothing when it is not installed.
tool_binary() {
  local venv="$1" tool="$2"
  if [ -x "$venv/bin/$tool" ]; then
    printf '%s' "$venv/bin/$tool"
  elif [ -x "$venv/Scripts/$tool.exe" ]; then
    printf '%s' "$venv/Scripts/$tool.exe"
  fi
}

# True when the installed binary already reports the pinned version. A tool's own
# reported version is the PyPI pin minus any packaging suffix (`shellcheck-py`
# 0.11.0.1 ships ShellCheck 0.11.0), so accept either the whole pin or its first
# three components — and only as a whole version number, so 0.11.0 never matches
# a reported 0.11.0.2. Tools word their `--version` output differently, so this
# searches it rather than matching a fixed line.
is_pinned() {
  local binary="$1" pin="$2" reported short
  reported="$("$binary" --version 2>/dev/null)" || return 1
  short="$(printf '%s' "$pin" | cut -d. -f1-3)"
  printf '%s' "$reported" \
    | grep -qE "(^|[^0-9.])(${pin//./\\.}|${short//./\\.})([^0-9.]|\$)"
}

for entry in "${TOOLS[@]}"; do
  read -r TOOL REQUIREMENT <<<"$entry"
  PIN="$(read_pin "$TOOL" "$REQUIREMENT")"
  VENV=".dev/$TOOL"

  BIN="$(tool_binary "$VENV" "$TOOL")"
  if [ -n "$BIN" ] && is_pinned "$BIN" "$PIN"; then
    continue
  fi

  if ! command -v uv >/dev/null 2>&1; then
    echo "setup-misc-tools: uv not found; cannot install the pinned $TOOL ($PIN)" >&2
    echo "ACTION: install uv (https://docs.astral.sh/uv/) and re-run 'just bootstrap'" >&2
    exit 1
  fi

  rm -rf "$VENV" || {
    echo "setup-misc-tools: cannot remove the stale venv at $ROOT/$VENV" >&2
    echo "ACTION: delete it by hand (it may be in use), then re-run 'just setup-misc-tools'" >&2
    exit 1
  }
  uv venv --quiet "$VENV" || {
    echo "setup-misc-tools: cannot create the venv at $VENV" >&2
    echo "ACTION: check disk space and that 'uv --version' works, then re-run 'just setup-misc-tools'" >&2
    exit 1
  }
  uv pip install --quiet --python "$VENV" "$REQUIREMENT==$PIN" || {
    echo "setup-misc-tools: cannot install $REQUIREMENT==$PIN" >&2
    echo "ACTION: check network access to PyPI, or set .$TOOL-version to a published release" >&2
    exit 1
  }

  BIN="$(tool_binary "$VENV" "$TOOL")"
  if [ -z "$BIN" ] || ! is_pinned "$BIN" "$PIN"; then
    echo "setup-misc-tools: $TOOL $PIN installed but does not report that version" >&2
    echo "ACTION: remove $ROOT/$VENV and re-run 'just setup-misc-tools'" >&2
    exit 1
  fi
done
