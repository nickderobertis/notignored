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
# The gate has no warnings-only mode, and these are the crate's test builds. Set
# here rather than per recipe so every tier and the report's rebuild compile with
# the same flags — a binary built with others would not match its profiles.
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
case "$TARGET_DIR" in /*) ;; *) TARGET_DIR="$ROOT/$TARGET_DIR" ;; esac
readonly COV_DIR="$TARGET_DIR/llvm-cov-target"
# Each tier's directory under this is its Nx `test` target's declared output;
# tests/e2e/nx_workspace.rs holds the two to the same path.
readonly STORE="$ROOT/target/coverage-profiles"
readonly LOCK="$STORE/.lock"
readonly RECLAIM="$STORE/.lock-reclaim"
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
  echo "$$" >"$1/pid"
}

# The process id a lock directory records, or nothing when it records none a
# probe could trust — a holder that has made the directory but not yet written
# to it, or a torn write — which is then waited on like a live one, never taken
# over on a guess.
holder_of() {
  local holder
  holder="$(cat "$1/pid" 2>/dev/null || true)"
  case "$holder" in
  "" | 0* | *[!0-9]*) holder="" ;;
  esac
  printf '%s' "$holder"
}

# Reclaim the lock from `$1`, the dead holder this contender saw. Every
# contender that queued behind the same dead run sees the same pid, so the
# decision and the removal happen under a second mkdir mutex, and the holder is
# read again inside it: a contender that lost the race finds a live holder — or
# no lock at all — and removes nothing. Returns non-zero when the mutex is busy,
# and sets `took_over` to the dead pid when this contender removed its lock.
took_over=""
reclaim() {
  local seen="$1"
  mkdir "$RECLAIM" 2>/dev/null || return 1
  write_pid "$RECLAIM" || {
    # Best effort: the refusal below names the directory either way.
    rm -rf "$RECLAIM" 2>/dev/null || true
    die "could not record this run in $RECLAIM" \
      "check that $STORE is writable by you, delete $RECLAIM if it remains, then re-run"
  }
  if [ "$(holder_of "$LOCK")" = "$seen" ]; then
    rm -rf "$LOCK" || {
      # Best effort: the refusal below names both directories either way.
      rm -rf "$RECLAIM" 2>/dev/null || true
      die "could not remove the stale lock $LOCK" \
        "check that $STORE is writable by you, or delete $LOCK and $RECLAIM, then re-run"
    }
    took_over="$seen"
  fi
  must "release $RECLAIM" rm -rf "$RECLAIM"
}

# A reclaim mutex is held for a moment. One whose holder has exited was
# abandoned mid-reclaim, and is never cleared on a guess — clearing it is the same
# check-then-act race the mutex exists to close — so the run stops and names it.
refuse_dead_reclaim() {
  local holder
  holder="$(holder_of "$RECLAIM")"
  if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then
    die "pid $holder exited while reclaiming $LOCK, leaving $RECLAIM behind" \
      "if no coverage run is in progress, delete $RECLAIM and re-run"
  fi
}

# Release the lock on exit only while it is still this run's.
release() {
  [ "$(holder_of "$LOCK")" = "$$" ] || return 0
  rm -rf "$LOCK" || echo "coverage: could not remove $LOCK; delete it before the next run" >&2
}

# mkdir is the one atomic test-and-set every platform's shell has. A lock whose
# holder is gone — a run killed before its trap — is reclaimed, not waited on;
# one whose holder is alive is waited on, for a bounded time.
# llmlint: ignore-block[tool_output_is_signal] a run that pauses behind another,
# or overrides a dead run's lock, says so once on stderr: silence would read as a
# hang, and a takeover is exactly what a reader debugging a lost profile needs.
lock() {
  must "create $STORE" mkdir -p "$STORE"
  local waited=0 holder
  until mkdir "$LOCK" 2>/dev/null; do
    # Gone already — released between that attempt and this look — is
    # contention: try again at once. Still impossible to make is an error.
    if [ ! -d "$LOCK" ]; then
      mkdir "$LOCK" 2>/dev/null && break
      [ -d "$LOCK" ] || die "cannot create the lock $LOCK" \
        "check that $STORE is writable by you, then re-run"
    fi
    holder="$(holder_of "$LOCK")"
    if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then
      reclaim "$holder" || refuse_dead_reclaim
      if [ -n "$took_over" ]; then
        echo "coverage: took over a lock left by pid $took_over, which had exited" >&2
        took_over=""
      fi
      [ -d "$RECLAIM" ] || continue
    elif [ "$waited" -eq 0 ]; then
      echo "coverage: waiting for another tier's run (pid ${holder:-unknown}) to finish" >&2
    fi
    if [ "$waited" -ge "$LOCK_WAIT_SECONDS" ]; then
      die "pid ${holder:-unknown} has held $LOCK for over ${LOCK_WAIT_SECONDS}s" \
        "if no coverage run is in progress, delete $LOCK and $RECLAIM, then re-run"
    fi
    waited=$((waited + 1))
    sleep 1
  done
  trap release EXIT
  must "record this run in $LOCK" write_pid "$LOCK"
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
  # A tier Nx replayed from cache brings back its profiles but not the binaries
  # they count, and the target directory may have been rebuilt from other sources
  # since. So rebuild every test binary from the sources as they are — a no-op
  # when the tiers just ran — selecting no test, so nothing runs.
  local build
  if ! build="$(cargo llvm-cov --no-report nextest --locked --tests -E 'none()' \
    --no-tests=pass --status-level none --final-status-level none 2>&1)"; then
    printf '%s\n' "$build" >&2
    die "could not build the instrumented test binaries the report reads" \
      "fix the build error above, then re-run"
  fi
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
      echo "ACTION: if a coverage table printed above, cover the lines it counts as missed;"
      echo "        if it did not, the report failed to build — fix the error above, or"
      echo "        re-record the profiles it names with: just nx run <project>:test --skip-nx-cache"
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
