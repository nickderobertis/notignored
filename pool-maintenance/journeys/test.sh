#!/usr/bin/env bash
# Journeys for pool-maintenance/measure.sh: the real script, the real
# `pool-maintenance` recipe from the root justfile, and the pinned
# cargo-prune-superseded, over a target/ that a double of cargo fills. Only
# cargo is replaced: a real gate build takes minutes and cannot be made to leave
# a cache-aged tree or a build that never settles, which are the states the
# measurement must converge from or refuse.
#
# The double mirrors what matters to the pruner: a unit is its name, and a
# fresh unit's files are left exactly as old as they are. `dual` is a crate the
# gate builds twice in one profile (two current hashes of one unit), `app` an
# ordinary one; under CARGO_PROFILE_DEV_DEBUG every unit gets a new hash.
set -euo pipefail

readonly NEXT="fix measure.sh or the staging it does, then run: just nx run pool-maintenance-journeys:test"

fail() {
  printf 'pool-maintenance journeys: %s\n' "$1" >&2
  if [ -n "${2:-}" ]; then
    printf -- '--- measure.sh output ---\n' >&2
    cat "$2" >&2 || printf '(the output at %s could not be read)\n' "$2" >&2
  fi
  printf 'ACTION: %s\n' "${3:-$NEXT}" >&2
  exit 1
}

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)" ||
  fail "cannot resolve this script's directory" "" "run it from a readable checkout"
readonly MEASURE="$HERE/../measure.sh" JUSTFILE="$HERE/../../justfile"
[ -f "$MEASURE" ] || fail "no measure.sh beside the journeys at $MEASURE" "" \
  "restore pool-maintenance/measure.sh, or point MEASURE at where it moved"
SCRATCH="$(mktemp -d)" || fail "cannot create a scratch directory" "" "check that \$TMPDIR is writable"
trap 'rm -rf "$SCRATCH" || printf "pool-maintenance journeys: could not remove %s; delete it by hand\n" "$SCRATCH" >&2' EXIT

command -v cargo-prune-superseded >/dev/null 2>&1 ||
  fail "cargo-prune-superseded is not on PATH" "" "run 'just bootstrap'"

# The recipe exactly as the root justfile declares it.
recipe="$(awk '/^pool-maintenance:$/ { on = 1 } on && /^$/ { exit } on' "$JUSTFILE")" ||
  fail "cannot read the root justfile at $JUSTFILE" "" "check that the checkout's justfile is readable"
[ -n "$recipe" ] || fail "the root justfile declares no pool-maintenance recipe" "" \
  "restore the recipe, or point this test at where it moved"

# One checkout per journey, its path left in $dir: measure.sh is linked in, so
# the file each journey runs is the real one.
unwritable() { fail "cannot write $1 under $SCRATCH" "" "check that \$TMPDIR is writable and has space"; }

checkout() {
  dir="$SCRATCH/$1"
  mkdir -p "$dir/pool-maintenance" "$dir/bin" "$dir/state" || unwritable "the $1 checkout"
  ln -s "$MEASURE" "$dir/pool-maintenance/measure.sh" || unwritable "the $1 checkout's measure.sh link"
  printf '%s\n' "$recipe" >"$dir/justfile" || unwritable "the $1 checkout's justfile"
  cat >"$dir/bin/cargo" <<'CARGO' || unwritable "the $1 checkout's cargo double"
#!/usr/bin/env bash
set -euo pipefail
profile=target/llvm-cov-target/debug
broken() { echo "cargo double: cannot write $1 under $PWD" >&2; exit 1; }
mkdir -p "$profile/deps" "$profile/.fingerprint" || broken "$profile"
unit() {
  [ -e "$profile/deps/lib$1-$2.rlib" ] && return 0
  printf 'build\n' >"$profile/deps/lib$1-$2.rlib" || broken "lib$1-$2.rlib"
  mkdir -p "$profile/.fingerprint/$1-$2" || broken "$1-$2"
  printf 'fingerprint\n' >"$profile/.fingerprint/$1-$2/lib-$1" || broken "$1-$2/lib-$1"
}
if [ -n "${CARGO_PROFILE_DEV_DEBUG:-}" ]; then
  if [ "${FAKE_MODE:-}" = alternative-orphan ]; then
    unit orphan 000000000000000a
  else
    unit dual 00000000000000a1
    unit dual 00000000000000a2
    unit app 00000000000000a3
  fi
  : >"$FAKE_STATE/alternative-built" || broken "$FAKE_STATE/alternative-built"
  exit 0
fi
unit dual 0000000000000001
unit dual 0000000000000002
unit app 0000000000000003
case "${FAKE_MODE:-}" in
  never-settles)
    count=$(($(cat "$FAKE_STATE/count" 2>/dev/null || echo 0) + 1))
    echo "$count" >"$FAKE_STATE/count" || broken "$FAKE_STATE/count"
    find "$profile" -path '*drift*' -exec touch -h -d 2000-01-01 {} + || broken "the drift unit's age"
    unit drift "$(printf '%016x' "$count")"
    ;;
  supersedes-late)
    [ ! -e "$FAKE_STATE/alternative-built" ] || unit late 0000000000000005
    ;;
esac
CARGO
  chmod +x "$dir/bin/cargo" || unwritable "the $1 checkout's cargo double as executable"
}

# The cargo double run once outside measure.sh, to seed a checkout's target/.
seed_build() {
  (cd "$dir" && PATH="$dir/bin:$PATH" FAKE_STATE="$dir/state" FAKE_MODE="${1:-}" cargo) ||
    fail "the cargo double could not seed the checkout at $dir" "" "check that \$TMPDIR is writable and has space"
}

# Every entry under target/ set to one old instant, as a restored CI cache
# leaves them: older than the window, with no unit's hashes told apart by age.
age() {
  find "$dir/target" -mindepth 1 -exec touch -h -d '2000-06-01T00:00:00Z' {} + ||
    fail "cannot age the fixture under $dir/target" "" "check that GNU touch and find are installed"
}

measure() {
  (cd "$dir" && PATH="$dir/bin:$PATH" FAKE_STATE="$dir/state" FAKE_MODE="${1:-}" \
    ONEBUDGETSPEC_RESULT="$dir/state/result.json" bash pool-maintenance/measure.sh) \
    >"$dir/state/log" 2>&1
}

expect_ok() {
  local rounds="$1" journey="$2" log="$dir/state/log" passes
  grep -qx 'measure: the timed pass removed 6 of the alternative build'"'"'s 6 entries; untimed backlog passes .*; lists in target/pool-maintenance' "$log" ||
    fail "$journey: the timed pass did not remove exactly the alternative build's 6 entries" "$log"
  passes="$(sed -n 's/^measure: .*untimed backlog passes \(.*\); lists in .*/\1/p' "$log" | awk -F' [+] ' '{ print NF }')" ||
    fail "$journey: cannot read the backlog passes from the log" "$log"
  [ "$passes" = "$rounds" ] || fail "$journey: expected $rounds untimed backlog passes, the log reports $passes" "$log"
  # shellcheck disable=SC2016 # JavaScript, whose `${...}` templates are for node to expand
  node -e '
    const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
    if (typeof r.value !== "number" || !(r.value >= 0)) throw new Error(`no seconds figure: ${JSON.stringify(r)}`);
    if (!/builds removed: 3;/.test(r.detail)) throw new Error(`the pruner'"'"'s own detail is gone: ${r.detail}`);
    if (!/; untimed backlog passes beforehand: [0-9.-]+s( [+] [0-9.-]+s)* \(not counted\)$/.test(r.detail))
      throw new Error(`the backlog passes are missing from the detail: ${r.detail}`);
  ' "$dir/state/result.json" 2>>"$log" || fail "$journey: the result file is not the timed pass's figure" "$log"
}

expect_refusal() {
  local message="$1" journey="$2" log="$dir/state/log"
  grep -qF "measure: $message" "$log" || fail "$journey: expected the refusal '$message'" "$log"
  grep -q '^ACTION: ' "$log" || fail "$journey: the refusal names no next action" "$log"
}

# A fresh checkout is already steady: one backlog round, which removes nothing.
# The lists a longer earlier run left are cleared, so none can be read as this
# run's evidence.
checkout fresh
mkdir -p "$dir/target/pool-maintenance" || unwritable "the fresh checkout's evidence directory"
printf 'target/llvm-cov-target/debug/deps/libstale-00000000000000ff.rlib\n' \
  >"$dir/target/pool-maintenance/backlog-4-removed.txt" || unwritable "an earlier run's backlog list"
measure || fail "a fresh checkout refused" "$dir/state/log"
expect_ok 1 "a fresh checkout"
[ ! -e "$dir/target/pool-maintenance/backlog-4-removed.txt" ] ||
  fail "a fresh checkout: an earlier run's backlog-4-removed.txt survived beside this run's lists" "$dir/state/log"

# A cache-aged checkout: one pass drops the older half of `dual`, the rebuild
# makes the other half the superseded one, and only the third round settles.
checkout aged
seed_build
age
measure || fail "a cache-aged checkout refused" "$dir/state/log"
expect_ok 3 "a cache-aged checkout"
[ -s "$dir/target/pool-maintenance/backlog-2-removed.txt" ] ||
  fail "a cache-aged checkout: round 2 removed nothing, so the aged half was not the one at stake" "$dir/state/log"

# A build that supersedes one of its own units every time never settles.
checkout never-settles
seed_build never-settles
measure never-settles && fail "a build that never settles was timed" "$dir/state/log"
expect_refusal "4 backlog rounds each removed builds, so the ordinary build never became current" \
  "a build that never settles"

# The timed pass removed an entry the alternative build did not create.
checkout supersedes-late
mkdir -p "$dir/target/llvm-cov-target/debug/deps" || unwritable "the late unit's deps directory"
printf 'build\n' >"$dir/target/llvm-cov-target/debug/deps/liblate-0000000000000004.rlib" ||
  unwritable "the late unit's superseded build"
age
measure supersedes-late && fail "a pass over a second superseded build was timed" "$dir/state/log"
expect_refusal "the timed pass removed 1 entries the alternative build did not create" \
  "an ordinary build that supersedes another"

# The timed pass removed nothing.
checkout alternative-orphan
measure alternative-orphan && fail "a pass that removed nothing was timed" "$dir/state/log"
expect_refusal "the timed pass removed nothing, so it did not measure the workload" \
  "an alternative build that supersedes nothing"

echo "pool-maintenance journeys: 5 measure.sh journeys passed"
