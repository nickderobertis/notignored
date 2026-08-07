#!/usr/bin/env bash
# Smoke-test a `notignored` that is already on PATH against the checked-in
# assets, and name the install that broke when it does not agree with them.
#
# One script, one fixture tree, one golden. `release.yml`'s verify jobs and
# `published-smoke.yml` run this over a binary they installed from PyPI or npm;
# `tests/e2e/smoke.rs` runs the identical file over the binary this repo just
# compiled. That is what stops a workflow's idea of "it works" from drifting
# from the parser that actually ships — a `grep -q '"E501"'` inlined in a
# workflow keeps passing after the record around it changes shape.
#
# Deliberately toolchain-free: bash, diff, and the installed binary. The
# scheduled sweep runs every week on four runners, and anything it had to
# install first would be a second thing that can rot.
set -euo pipefail

# Collation decides the glob order below, which decides the order the records
# come back in. Fix it so the golden is the same file on every runner.
export LC_ALL=C

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(dirname "$here")"

fixtures="$root/tests/fixtures/smoke"
expected="$root/tests/golden/smoke.json"
expect_version=""
label="installed notignored"

fail() {
  echo "::error::$label: $1" >&2
  echo "ACTION: $2" >&2
  exit 1
}

# Every option takes a value, so a missing one is an argument error rather than
# a silently empty setting.
need_value() {
  if [ "$#" -lt 2 ]; then
    echo "$1 needs a value" >&2
    echo "ACTION: pass it as '$1 <value>'" >&2
    exit 2
  fi
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --fixtures) need_value "$@"; fixtures="$2"; shift 2 ;;
    --expected) need_value "$@"; expected="$2"; shift 2 ;;
    --expect-version) need_value "$@"; expect_version="$2"; shift 2 ;;
    # What installed the binary, so a red matrix leg names the platform and the
    # registry rather than only the assertion that failed.
    --label) need_value "$@"; label="$2"; shift 2 ;;
    *)
      echo "unknown option $1" >&2
      echo "ACTION: run 'smoke-published.sh [--fixtures DIR] [--expected FILE] [--expect-version V] [--label TEXT]'" >&2
      exit 2
      ;;
  esac
done

if ! command -v notignored >/dev/null 2>&1; then
  fail "no 'notignored' on PATH" \
    "install it first — 'pip install notignored-cli' or 'npm install -g notignored-cli'"
fi
[ -d "$fixtures" ] || fail "no fixture directory at $fixtures" "check out the repository first"
[ -f "$expected" ] || fail "no golden report at $expected" "check out the repository first"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Windows ships the same bytes with CRLF once anything touches them, so strip CR
# everywhere rather than let a line ending decide the verdict.
if ! notignored --version >"$work/version.txt" 2>"$work/version.err"; then
  cat "$work/version.err" >&2
  fail "'notignored --version' exited non-zero" "the installed binary cannot run on this platform"
fi
reported="$(tr -d '\r' <"$work/version.txt")"
if [ -n "$expect_version" ] && [ "$reported" != "notignored $expect_version" ]; then
  fail "reports '$reported', not 'notignored $expect_version'" \
    "the install resolved a different build than the version that was published"
fi

# The fixture tree is the argument list: adding a file to it and re-blessing is
# the whole of adding a case, and no list of names has to be kept in step.
cd "$fixtures"
sources=()
for name in *; do
  if [ -f "$name" ]; then
    sources+=("$name")
  fi
done
[ "${#sources[@]}" -gt 0 ] || fail "no fixture files in $fixtures" "restore the smoke fixture tree"

# Bare file names, run from inside the tree: the report's paths are then spelled
# the same on Windows as on POSIX, so the golden needs no per-platform variant.
if ! notignored "${sources[@]}" --format json >"$work/raw.json" 2>"$work/scan.err"; then
  cat "$work/scan.err" >&2
  fail "the scan exited non-zero" "the installed binary could not read the fixture tree"
fi
tr -d '\r' <"$work/raw.json" >"$work/actual.json"
tr -d '\r' <"$expected" >"$work/expected.json"
if ! diff -u "$work/expected.json" "$work/actual.json"; then
  fail "the report differs from $expected (diff above)" \
    "if the published build is fine and the record shape moved, re-bless with 'just bless'"
fi

echo "$label: $reported reported the golden records for ${#sources[@]} fixture files"
