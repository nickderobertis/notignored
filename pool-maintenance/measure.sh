#!/usr/bin/env bash
# The measurement of `notignored-pool-maintenance-seconds` (budgets.yaml): one
# steady-state pass of the declared pool maintenance, `just pool-maintenance`,
# timed and reported by the pruner itself through onebudgetspec.
#
# A regular pass faces one run's worth of superseded builds, never weeks of
# history, so that state is made before anything is timed:
#   1. the declared maintenance once, untimed, clears any backlog;
#   2. the gate's test build under an alternative profile — debuginfo on, which
#      gives every unit a new metadata hash in the same profile directory — and
#      exactly the entries that build created backdated past the 2-day window,
#      standing in for a run's builds that have since been superseded;
#   3. the ordinary gate build, so each unit's newest build is the ordinary one;
#   4. one declared pass, whose own report is the figure.
# The pass must then have removed something, and nothing the alternative build
# did not create, or the figure would describe a different workload; either is a
# failure, never a figure. The entry lists stay in $EVIDENCE.
#
# It builds the crate twice, so only the release pull request's sweep runs it
# (`just budgets-release`), never a per-change gate.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || {
  echo "measure: cannot enter the repository root $ROOT" >&2
  echo "ACTION: run this from a checkout whose directories are readable" >&2
  exit 1
}

readonly TARGET="target"
readonly EVIDENCE="$TARGET/pool-maintenance"
# Older than any build a real checkout holds, so the ordinary build always stays
# each unit's newest whatever the age of the slot it runs in.
readonly SUPERSEDED_AT="2000-01-01T00:00:00Z"

die() {
  printf 'measure: %s\n' "$1" >&2
  printf 'ACTION: %s\n' "$2" >&2
  exit 1
}

command -v cargo-prune-superseded >/dev/null 2>&1 ||
  die "cargo-prune-superseded is not on PATH" "run 'just bootstrap'"

# Every build entry the pruner selects among: the children of each profile's
# `deps/`, `.fingerprint/`, `build/` and `incremental/`.
entries() {
  [ -d "$TARGET" ] || return 0
  find "$TARGET" -type d \( -name deps -o -name .fingerprint -o -name build -o -name incremental \) \
    -prune -print | while IFS= read -r dir; do
    find "$dir" -mindepth 1 -maxdepth 1
  done | LC_ALL=C sort
}

# The gate's own build of every test binary (scripts/coverage.sh `report`),
# selecting no test so nothing runs.
gate_build() {
  RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings" \
    cargo llvm-cov --no-report nextest --locked --tests -E 'none()' \
    --no-tests=pass --status-level none --final-status-level none >&2 ||
    die "the gate's test build failed" "fix the build error above, then re-run"
}

seconds_now() { date +%s.%N; }

mkdir -p "$EVIDENCE" || die "cannot create $EVIDENCE" "check that $TARGET is writable by you"

# 1. The backlog, untimed: its figure is recorded only as evidence.
started="$(seconds_now)"
env -u ONEBUDGETSPEC_RESULT just pool-maintenance >&2 ||
  die "the untimed backlog pass failed" "fix what the pruner reported above, then re-run"
backlog_seconds="$(awk -v a="$started" -v b="$(seconds_now)" 'BEGIN { printf "%.2f", b - a }')"

# 2. One run's worth of builds, then superseded.
entries >"$EVIDENCE/before-alternative.txt"
CARGO_PROFILE_DEV_DEBUG=line-tables-only gate_build
entries >"$EVIDENCE/after-alternative.txt"
LC_ALL=C comm -13 "$EVIDENCE/before-alternative.txt" "$EVIDENCE/after-alternative.txt" \
  >"$EVIDENCE/alternative.txt"
[ -s "$EVIDENCE/alternative.txt" ] ||
  die "the alternative build created no entries, so there is nothing to supersede" \
    "check that CARGO_PROFILE_DEV_DEBUG still changes the units' metadata hashes"
while IFS= read -r entry; do
  find "$entry" -exec touch -h -d "$SUPERSEDED_AT" {} +
done <"$EVIDENCE/alternative.txt"

# 3. The ordinary build, each unit's newest.
gate_build
entries >"$EVIDENCE/before-pass.txt"

# 4. The timed pass: the pruner reports its own seconds when ONEBUDGETSPEC_RESULT
# names a file.
just pool-maintenance >&2 || die "the timed pass failed" "fix what the pruner reported above, then re-run"
entries >"$EVIDENCE/after-pass.txt"
LC_ALL=C comm -23 "$EVIDENCE/before-pass.txt" "$EVIDENCE/after-pass.txt" >"$EVIDENCE/removed.txt"
LC_ALL=C comm -23 "$EVIDENCE/removed.txt" "$EVIDENCE/alternative.txt" >"$EVIDENCE/unexpected.txt"

[ -s "$EVIDENCE/removed.txt" ] ||
  die "the timed pass removed nothing, so it did not measure the workload" \
    "compare $EVIDENCE/alternative.txt with $EVIDENCE/before-pass.txt"
[ ! -s "$EVIDENCE/unexpected.txt" ] ||
  die "the timed pass removed $(wc -l <"$EVIDENCE/unexpected.txt") entries the alternative build did not create" \
    "read $EVIDENCE/unexpected.txt: the ordinary build was not each unit's newest"

echo "measure: the timed pass removed $(wc -l <"$EVIDENCE/removed.txt") of the alternative build's $(wc -l <"$EVIDENCE/alternative.txt") entries; untimed backlog pass ${backlog_seconds}s; lists in $EVIDENCE" >&2

# The backlog pass is evidence beside the figure, never part of it.
if [ -n "${ONEBUDGETSPEC_RESULT:-}" ]; then
  # shellcheck disable=SC2016 # JavaScript, whose `${...}` templates are for node to expand
  node -e '
    const fs = require("fs");
    const [file, backlog] = process.argv.slice(1);
    const result = JSON.parse(fs.readFileSync(file, "utf8"));
    const note = `untimed backlog pass beforehand: ${backlog} s (not counted)`;
    result.detail = result.detail ? `${result.detail}; ${note}` : note;
    fs.writeFileSync(file, JSON.stringify(result));
  ' "$ONEBUDGETSPEC_RESULT" "$backlog_seconds" ||
    die "cannot add the backlog pass to $ONEBUDGETSPEC_RESULT" "check that Node.js runs, then re-run"
fi
