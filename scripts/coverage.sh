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

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || {
  echo "coverage: cannot resolve the repository root from ${BASH_SOURCE[0]}" >&2
  echo "ACTION: run it as 'bash scripts/coverage.sh' from a readable checkout" >&2
  exit 1
}
cd "$ROOT" || {
  echo "coverage: cannot enter the repository root $ROOT" >&2
  echo "ACTION: run this from a checkout whose directories are readable" >&2
  exit 1
}

readonly FLOOR=95
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
case "$TARGET_DIR" in /*) ;; *) TARGET_DIR="$ROOT/$TARGET_DIR" ;; esac
readonly COV_DIR="$TARGET_DIR/llvm-cov-target"
# Each tier's directory under this is its Nx `test` target's declared output;
# tests/e2e/nx_workspace.rs holds the two to the same path.
readonly STORE="$ROOT/target/coverage-profiles"
readonly LOCK="$STORE/.lock"
# Long enough for the slowest tier to finish ahead of a queued one.
readonly LOCK_WAIT_SECONDS="${NOTIGNORED_COVERAGE_LOCK_WAIT:-3600}"

die() {
  printf 'coverage: %s\n' "$1" >&2
  printf 'ACTION: %s\n' "$2" >&2
  exit 1
}

case "$LOCK_WAIT_SECONDS" in
"" | *[!0-9]*)
  die "NOTIGNORED_COVERAGE_LOCK_WAIT is '$LOCK_WAIT_SECONDS', not a number of seconds" \
    "unset it, or set it to a whole number of seconds"
  ;;
esac

must() {
  local what="$1"
  shift
  "$@" || die "could not $what" \
    "check that $STORE and $COV_DIR are writable by you, or delete them, then re-run"
}

# A project name becomes a directory name, so it is held to what Nx names are.
valid_project() {
  case "$1" in
  "" | -* | *[!a-z0-9-]*) return 1 ;;
  *) return 0 ;;
  esac
}

write_pid() {
  echo "$$" >"$LOCK/pid"
}

# mkdir is the one atomic test-and-set every platform's shell has. A lock whose
# holder is gone — a run killed before its trap — is taken over, not waited on;
# one whose holder is alive is waited on, for a bounded time.
# llmlint: ignore-block[tool_output_is_signal] a run that pauses behind another,
# or overrides a dead run's lock, says so once on stderr: silence would read as a
# hang, and a takeover is exactly what a reader debugging a lost profile needs.
lock() {
  must "create $STORE" mkdir -p "$STORE"
  local waited=0 holder
  until mkdir "$LOCK" 2>/dev/null; do
    [ -d "$LOCK" ] || die "cannot create the lock $LOCK" \
      "check that $STORE is writable by you, then re-run"
    holder="$(cat "$LOCK/pid" 2>/dev/null || true)"
    # Only a positive process id is probed. Anything else — a holder that has
    # made the lock but not yet written to it, or a torn write — is waited on
    # like a live one, never taken over on a guess.
    case "$holder" in
    "" | 0* | *[!0-9]*) holder="" ;;
    esac
    if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then
      echo "coverage: taking over a lock left by pid $holder, which has exited" >&2
      must "remove the stale lock $LOCK" rm -rf "$LOCK"
      continue
    fi
    if [ "$waited" -eq 0 ]; then
      echo "coverage: waiting for another tier's run (pid ${holder:-unknown}) to finish" >&2
    fi
    if [ "$waited" -ge "$LOCK_WAIT_SECONDS" ]; then
      die "pid ${holder:-unknown} has held $LOCK for over ${LOCK_WAIT_SECONDS}s" \
        "if no coverage run is in progress, delete $LOCK and re-run"
    fi
    waited=$((waited + 1))
    sleep 1
  done
  trap 'rm -rf "$LOCK" || echo "coverage: could not remove $LOCK; delete it before the next run" >&2' EXIT
  must "record this run in $LOCK" write_pid
}
# llmlint: ignore-end[tool_output_is_signal]

# The raw profiles at the top of cargo-llvm-cov's directory — the only place it
# reads them from, and the only place it writes them.
loose_profiles() {
  [ -d "$COV_DIR" ] || return 0
  must "list the profiles in $COV_DIR" find "$COV_DIR" -maxdepth 1 -type f -name '*.profraw'
}

clear_loose_profiles() {
  [ -d "$COV_DIR" ] || return 0
  must "clear earlier profiles from $COV_DIR" find "$COV_DIR" -maxdepth 1 -type f \
    \( -name '*.profraw' -o -name '*.profdata' -o -name '*-profraw-list' \) -delete
}

stored_profiles() {
  [ -d "$STORE/$1" ] || return 0
  must "list $1's stored profiles" find "$STORE/$1" -maxdepth 1 -type f -name '*.profraw'
}

tier() {
  local project="${1:-}"
  valid_project "$project" || die "'$project' is not a project name" \
    "call it as: scripts/coverage.sh tier <project> <cargo llvm-cov nextest args...>"
  shift
  lock
  local profiles="$STORE/$project"
  must "remove $project's previous profiles" rm -rf "$profiles"
  # Leftovers from a run killed mid-way would otherwise be filed as this one's.
  clear_loose_profiles
  local status=0
  cargo llvm-cov --no-report nextest --locked \
    --status-level fail --final-status-level fail "$@" || status=$?
  must "create $profiles" mkdir -p "$profiles"
  local found file
  found="$(loose_profiles)"
  while IFS= read -r file; do
    [ -z "$file" ] || must "move $file into $profiles" mv "$file" "$profiles/"
  done <<EOF
$found
EOF
  if [ "$status" -ne 0 ]; then
    echo "coverage: $project's tests failed — fix what the summary above names" >&2
    exit "$status"
  fi
}

report() {
  [ "$#" -gt 0 ] || die "report needs the tiers to combine" \
    "call it as: scripts/coverage.sh report <project>..."
  local project
  # Refused before the lock is taken: a caller that already holds it — a test
  # inside a running tier — gets its answer instead of waiting on itself.
  for project in "$@"; do
    valid_project "$project" || die "'$project' is not a project name" \
      "pass the Nx project names whose tiers this report combines"
    [ -n "$(stored_profiles "$project")" ] ||
      die "$project has no recorded profiles, so the report would cover less than the suite" \
        "run its tier first: just nx run $project:test"
  done
  lock
  clear_loose_profiles
  must "create $COV_DIR" mkdir -p "$COV_DIR"
  local found file
  for project in "$@"; do
    found="$(stored_profiles "$project")"
    # Linked, or copied across filesystems: the store is the cache's copy and
    # must survive the report.
    while IFS= read -r file; do
      [ -z "$file" ] || ln -f "$file" "$COV_DIR/" 2>/dev/null ||
        must "stage $file for the report" cp "$file" "$COV_DIR/"
    done <<EOF
$found
EOF
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
