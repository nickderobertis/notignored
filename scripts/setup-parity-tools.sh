#!/usr/bin/env bash
# Install the pinned linters the e2e parity suite drives.
#
# The parity tests prove that what notignored reports is what each tool actually
# suppresses, so they run the REAL tools — never a stub. Pinning makes that proof
# reproducible: the `.<tool>-version` files are the single source of truth, read
# here and asserted by the tests against each binary's own `--version`, so a
# stray system install can never silently stand in for the pinned one.
#
# Every tool ships as a PyPI wheel for Linux, macOS, and Windows, so one
# installer covers every platform CI runs. Each lands in its own project-local
# venv (`.dev/<tool>`) so the path is the same on every machine, one tool's
# dependencies can never constrain another's, and nothing leaks into the user's
# global tools. Idempotent and quiet on success.
#
# The Rust half of the suite needs no entry here: `rust-toolchain.toml` pins the
# toolchain, and rustup resolves `clippy-driver` from it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# tool | version file | PyPI requirement name
TOOLS=(
  "ruff|.ruff-version|ruff"
  "shellcheck|.shellcheck-version|shellcheck-py"
  "llmlint|.llmlint-version|llmlint-cli"
)

need_uv() {
  command -v uv >/dev/null 2>&1 && return 0
  echo "setup-parity-tools: uv not found; cannot install the pinned linters" >&2
  echo "ACTION: install uv (https://docs.astral.sh/uv/) and re-run 'just bootstrap'" >&2
  exit 1
}

install_tool() {
  local tool="$1" version_file="$2" package="$3"
  local pin venv bin

  pin="$(tr -d '[:space:]' < "$version_file")" || {
    echo "setup-parity-tools: cannot read $ROOT/$version_file" >&2
    echo "ACTION: restore it from git ('git checkout -- $version_file')" >&2
    exit 1
  }
  # The pin feeds a package requirement, so validate its shape before use rather
  # than letting arbitrary file contents reach the resolver. Three or four
  # numeric components, nothing else: a glob alone would let a trailing
  # requirement specifier through.
  if ! printf '%s' "$pin" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)?$'; then
    echo "setup-parity-tools: $version_file must hold a version like 0.16.1" >&2
    echo "ACTION: write a published $package version (https://pypi.org/project/$package/#history)" >&2
    exit 1
  fi

  venv=".dev/$tool"
  if [ -x "$venv/bin/$tool" ]; then
    bin="$venv/bin/$tool"
  elif [ -x "$venv/Scripts/$tool.exe" ]; then
    bin="$venv/Scripts/$tool.exe"
  else
    bin=""
  fi
  if [ -n "$bin" ] && installed_version_matches "$bin" "$pin"; then
    return 0
  fi

  need_uv
  rm -rf "$venv" || {
    echo "setup-parity-tools: cannot remove the stale venv at $ROOT/$venv" >&2
    echo "ACTION: delete it by hand (it may be in use), then re-run 'just setup-parity-tools'" >&2
    exit 1
  }
  uv venv --quiet "$venv" || {
    echo "setup-parity-tools: cannot create the venv at $venv" >&2
    echo "ACTION: check disk space and that 'uv --version' works, then re-run 'just setup-parity-tools'" >&2
    exit 1
  }
  uv pip install --quiet --python "$venv" "$package==$pin" || {
    echo "setup-parity-tools: cannot install $package==$pin" >&2
    echo "ACTION: check network access to PyPI, or set $version_file to a published release" >&2
    exit 1
  }
}

# A tool's own reported version is the PyPI pin minus any packaging suffix
# (`shellcheck-py` 0.11.0.1 ships ShellCheck 0.11.0), so accept either the whole
# pin or its first three components — and only as a whole version number, so
# 0.11.0 never matches a reported 0.11.0.2.
installed_version_matches() {
  local bin="$1" pin="$2" reported short
  reported="$("$bin" --version 2>/dev/null)" || return 1
  short="$(printf '%s' "$pin" | cut -d. -f1-3)"
  printf '%s' "$reported" \
    | grep -qE "(^|[^0-9.])(${pin//./\\.}|${short//./\\.})([^0-9.]|\$)"
}

for entry in "${TOOLS[@]}"; do
  IFS='|' read -r tool version_file package <<< "$entry"
  install_tool "$tool" "$version_file" "$package"
done
