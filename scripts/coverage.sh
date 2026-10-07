#!/usr/bin/env bash
# Line coverage measured per test tier and enforced once, over their union.
#
#   scripts/coverage.sh tier <project> <cargo llvm-cov nextest args...>
#       run one tier's tests instrumented, keeping its raw profiles
#   scripts/coverage.sh report <project>...
#       merge those tiers' profiles and enforce the floor over the crate's code
#
# The crate's tests are three Nx projects (unit, integration, e2e) so a change
# pays only for the tiers it reaches, but the 95% floor is a property of `src/`
# as *every* tier exercises it: the e2e journeys alone cover lines no unit test
# reaches. So no tier reports. Each runs `cargo llvm-cov --no-report`, and its
# raw profiles are moved out of cargo-llvm-cov's shared directory into one of
# their own — `target/coverage-profiles/<project>`, which is that Nx target's
# declared output, so a cached tier replays exactly the profiles its last real
# run wrote. `report` stages every tier's set back and reports over the union,
# which is the same measurement one `cargo llvm-cov nextest` over the whole
# suite made.
#
# cargo-llvm-cov names every profile after the workspace directory, whatever
# tier wrote it, so nothing but *when* it was written says whose it is. Every
# mode therefore holds one lock for its whole run, and Nx running two tiers at
# once queues the second rather than letting it sweep up the first's profiles.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || {
  echo "coverage: cannot enter the repository root $ROOT" >&2
  exit 1
}

readonly FLOOR=95
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
case "$TARGET_DIR" in /*) ;; *) TARGET_DIR="$ROOT/$TARGET_DIR" ;; esac
readonly COV_DIR="$TARGET_DIR/llvm-cov-target"
readonly STORE="$ROOT/target/coverage-profiles"
readonly LOCK="$STORE/.lock"

die() {
  printf 'coverage: %s\n' "$1" >&2
  printf 'ACTION: %s\n' "$2" >&2
  exit 1
}

# A project name becomes a directory name, so it is held to what Nx names are.
valid_project() {
  printf '%s' "$1" | grep -Eq '^[a-z0-9][a-z0-9-]*$'
}

# mkdir is the one atomic test-and-set every platform's shell has. A lock whose
# holder is gone — a run killed before its trap — is taken over, not waited on.
lock() {
  mkdir -p "$STORE"
  local waited=0 holder
  until mkdir "$LOCK" 2>/dev/null; do
    holder="$(cat "$LOCK/pid" 2>/dev/null || true)"
    if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then
      rm -rf "$LOCK"
      continue
    fi
    if [ "$waited" -eq 0 ]; then
      echo "coverage: waiting for another tier's run (pid ${holder:-unknown}) to finish" >&2
    fi
    waited=1
    sleep 1
  done
  echo "$$" >"$LOCK/pid"
  trap 'rm -rf "$LOCK"' EXIT
}

# The raw profiles at the top of cargo-llvm-cov's directory — the only place it
# reads them from, and the only place it writes them.
loose_profiles() {
  [ -d "$COV_DIR" ] || return 0
  find "$COV_DIR" -maxdepth 1 -type f -name '*.profraw'
}

clear_loose_profiles() {
  [ -d "$COV_DIR" ] || return 0
  find "$COV_DIR" -maxdepth 1 -type f \
    \( -name '*.profraw' -o -name '*.profdata' -o -name '*-profraw-list' \) -delete
}

tier() {
  local project="${1:-}"
  valid_project "$project" || die "'$project' is not a project name" \
    "call it as: scripts/coverage.sh tier <project> <cargo llvm-cov nextest args...>"
  shift
  lock
  local profiles="$STORE/$project"
  rm -rf "$profiles"
  # Leftovers from a run killed mid-way would otherwise be filed as this one's.
  clear_loose_profiles
  local status=0
  cargo llvm-cov --no-report nextest --locked \
    --status-level fail --final-status-level fail "$@" || status=$?
  mkdir -p "$profiles"
  loose_profiles | while IFS= read -r file; do mv "$file" "$profiles/"; done
  if [ "$status" -ne 0 ]; then
    echo "coverage: $project's tests failed — fix what the summary above names" >&2
    exit "$status"
  fi
}

report() {
  [ "$#" -gt 0 ] || die "report needs the tiers to combine" \
    "call it as: scripts/coverage.sh report <project>..."
  local project count
  # Refused before the lock is taken: a caller that already holds it — a test
  # inside a running tier — gets its answer instead of waiting on itself.
  for project in "$@"; do
    valid_project "$project" || die "'$project' is not a project name" \
      "pass the Nx project names whose tiers this report combines"
    count="$(find "$STORE/$project" -maxdepth 1 -type f -name '*.profraw' 2>/dev/null | wc -l)"
    [ "$count" -gt 0 ] || die "$project has no recorded profiles, so the report would cover less than the suite" \
      "run its tier first: just nx run $project:test"
  done
  lock
  clear_loose_profiles
  mkdir -p "$COV_DIR"
  for project in "$@"; do
    # Linked, or copied across filesystems: the store is the cache's copy and
    # must survive the report.
    find "$STORE/$project" -maxdepth 1 -type f -name '*.profraw' |
      while IFS= read -r file; do
        ln -f "$file" "$COV_DIR/" 2>/dev/null || cp "$file" "$COV_DIR/"
      done
  done
  local status=0
  cargo llvm-cov report --fail-under-lines "$FLOOR" || status=$?
  clear_loose_profiles
  if [ "$status" -ne 0 ]; then
    {
      echo "coverage: below ${FLOOR}% lines over the tiers $*, or the report could not be built"
      echo "ACTION: cover the lines the table above counts as missed. If a tier replayed from"
      echo "        cache onto a target directory rebuilt since, re-run it: NX_SKIP_NX_CACHE=true just test"
    } >&2
    exit "$status"
  fi
}

case "${1:-}" in
tier)
  shift
  tier "$@"
  ;;
report)
  shift
  report "$@"
  ;;
*)
  die "unknown mode '${1:-}'" "use 'tier <project> <args...>' or 'report <project>...'"
  ;;
esac
