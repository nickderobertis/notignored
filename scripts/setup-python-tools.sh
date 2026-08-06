#!/usr/bin/env bash
# Install the pinned Python tools the e2e parity suites drive.
#
# The parity tests prove that what notignored reports is what each tool actually
# suppresses, so they run the REAL tools — never a stub. Pinning them makes that
# proof reproducible: `.<tool>-version` is the single source of truth, read here
# and asserted by the tests against `<tool> --version`, so a stray system install
# can never silently stand in for the pinned one.
#
# Each tool gets its own project-local venv (`.dev/<tool>`) so the path is the
# same on every machine, one tool's dependencies cannot resolve against another's,
# and nothing leaks into the user's global tools. Idempotent and quiet on success.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || {
  echo "$(basename "${BASH_SOURCE[0]}"): cannot enter the repository root $ROOT" >&2
  echo "ACTION: run this from a checkout whose directories are readable, then re-run 'just bootstrap'" >&2
  exit 1
}

# `<tool> <pip requirement>`. The venv directory, the binary, and the pin file
# are all named after the tool; only the requirement can differ.
#
# pyright's PyPI package is a wrapper that drives the real (Node) pyright: the
# `nodejs` extra vendors a Node build, so `just bootstrap` needs no system Node
# and CI cannot drift onto whatever version a runner happens to ship.
TOOLS=(
  "ruff ruff"
  "mypy mypy"
  "pyright pyright[nodejs]"
  "ty ty"
)

# The wrapper otherwise queries PyPI for a newer release on every run, which
# would put a network call — and a stray line of output — inside the gate.
export PYRIGHT_PYTHON_IGNORE_WARNINGS=1

# The pin as written in `.<tool>-version`, validated before it reaches uv.
read_pin() {
  local tool="$1" pin
  pin="$(tr -d '[:space:]' < ".$tool-version")" || {
    echo "setup-python-tools: cannot read $ROOT/.$tool-version" >&2
    echo "ACTION: restore it from git ('git checkout -- .$tool-version')" >&2
    exit 1
  }
  # The pin feeds a package requirement, so validate its shape before use rather
  # than letting arbitrary file contents reach the resolver. Exactly three
  # numeric components: a glob alone would let `1.2.3.4` — or a trailing
  # requirement specifier — through.
  if ! printf '%s' "$pin" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "setup-python-tools: .$tool-version must hold a version like 0.16.1" >&2
    echo "ACTION: write a published $tool version (see https://pypi.org/project/$tool/#history)" >&2
    exit 1
  fi
  printf '%s' "$pin"
}

# The tool's binary inside its venv, or nothing when it is not installed.
tool_binary() {
  local venv="$1"
  if [ -x "$venv/bin/${2}" ]; then
    printf '%s' "$venv/bin/${2}"
  elif [ -x "$venv/Scripts/${2}.exe" ]; then
    printf '%s' "$venv/Scripts/${2}.exe"
  fi
}

# True when the installed binary already reports the pinned version. Tools pad
# their version line differently (`mypy 2.3.0 (compiled: yes)`), so match the
# leading `<tool> <pin>` token rather than the whole line.
is_pinned() {
  local binary="$1" tool="$2" pin="$3" reported
  reported="$("$binary" --version 2>/dev/null | head -1)" || return 1
  case "$reported" in
  "$tool $pin" | "$tool $pin "*) return 0 ;;
  *) return 1 ;;
  esac
}

for entry in "${TOOLS[@]}"; do
  read -r TOOL REQUIREMENT <<<"$entry"
  PIN="$(read_pin "$TOOL")"
  VENV=".dev/$TOOL"

  BIN="$(tool_binary "$VENV" "$TOOL")"
  if [ -n "$BIN" ] && is_pinned "$BIN" "$TOOL" "$PIN"; then
    continue
  fi

  if ! command -v uv >/dev/null 2>&1; then
    echo "setup-python-tools: uv not found; cannot install the pinned $TOOL ($PIN)" >&2
    echo "ACTION: install uv (https://docs.astral.sh/uv/) and re-run 'just bootstrap'" >&2
    exit 1
  fi

  # llmlint: ignore[changed_behavior_has_e2e] forcing `rm -rf` to fail needs a
  # directory the test process cannot write, which root — the user in many CI
  # containers — is exempt from; the test would silently stop proving anything
  # there. The branch it guards is one echo pair, and every other failure path in
  # this script is driven for real by tests/e2e/python_tools_setup.rs.
  rm -rf "$VENV" || {
    echo "setup-python-tools: cannot remove the stale venv at $ROOT/$VENV" >&2
    echo "ACTION: delete it by hand (it may be in use), then re-run 'just setup-python-tools'" >&2
    exit 1
  }
  uv venv --quiet "$VENV" || {
    echo "setup-python-tools: cannot create the venv at $VENV" >&2
    echo "ACTION: check disk space and that 'uv --version' works, then re-run 'just setup-python-tools'" >&2
    exit 1
  }
  uv pip install --quiet --python "$VENV" "$REQUIREMENT==$PIN" || {
    echo "setup-python-tools: cannot install $REQUIREMENT==$PIN" >&2
    echo "ACTION: check network access to PyPI, or set .$TOOL-version to a published release" >&2
    exit 1
  }

  # llmlint: ignore[changed_behavior_has_e2e] this fires only when uv reports a
  # successful install of `$TOOL==$PIN` that then reports a different version —
  # reachable only by serving a mislabelled wheel from a fake index, which would
  # prove the fake index. It is the backstop for exactly the case the rest of the
  # script cannot detect, so it stays.
  BIN="$(tool_binary "$VENV" "$TOOL")"
  if [ -z "$BIN" ] || ! is_pinned "$BIN" "$TOOL" "$PIN"; then
    echo "setup-python-tools: $TOOL $PIN installed but does not report that version" >&2
    echo "ACTION: remove $ROOT/$VENV and re-run 'just setup-python-tools'" >&2
    exit 1
  fi
done
