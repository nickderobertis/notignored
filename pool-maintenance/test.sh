#!/usr/bin/env bash
# Journeys for measure.sh: the real script, the real `pool-maintenance` recipe
# from the root justfile, and the pinned cargo-prune-superseded, over a target/
# that a double of cargo fills. Only cargo is replaced: a real gate build takes
# minutes and cannot be made to leave a cache-aged tree or a build that never
# settles, which are the states the measurement must refuse or converge from.
#
# The double mirrors what matters to the pruner: a unit is its name, and a
# fresh unit's files are left exactly as old as they are. `dual` is a crate the
# gate builds twice in one profile (two current hashes of one unit), `app` an
# ordinary one; under CARGO_PROFILE_DEV_DEBUG every unit gets a new hash.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fail() {
  printf 'pool-maintenance test: %s\n' "$1" >&2
  [ -z "${2:-}" ] || { printf -- '--- measure.sh output ---\n' >&2; cat "$2" >&2; }
  printf 'ACTION: %s\n' "${3:-fix measure.sh or the staging it does, then run: just test pool-maintenance}" >&2
  exit 1
}

command -v cargo-prune-superseded >/dev/null 2>&1 ||
  fail "cargo-prune-superseded is not on PATH" "" "run 'just bootstrap'"

# The recipe exactly as the root justfile declares it.
recipe="$(awk '/^pool-maintenance:$/ { on = 1 } on && /^$/ { exit } on' "$ROOT/justfile")"
[ -n "$recipe" ] || fail "the root justfile declares no pool-maintenance recipe" "" \
  "restore the recipe, or point this test at where it moved"

# One checkout per journey: measure.sh linked in, so the file run is the real one.
checkout() {
  local dir="$SCRATCH/$1"
  mkdir -p "$dir/pool-maintenance" "$dir/bin" "$dir/state"
  ln -s "$HERE/measure.sh" "$dir/pool-maintenance/measure.sh"
  printf '%s\n' "$recipe" >"$dir/justfile"
  cat >"$dir/bin/cargo" <<'CARGO'
#!/usr/bin/env bash
set -euo pipefail
profile=target/llvm-cov-target/debug
mkdir -p "$profile/deps" "$profile/.fingerprint"
unit() {
  [ -e "$profile/deps/lib$1-$2.rlib" ] && return 0
  printf 'build\n' >"$profile/deps/lib$1-$2.rlib"
  mkdir -p "$profile/.fingerprint/$1-$2"
  printf 'fingerprint\n' >"$profile/.fingerprint/$1-$2/lib-$1"
}
if [ -n "${CARGO_PROFILE_DEV_DEBUG:-}" ]; then
  if [ "${FAKE_MODE:-}" = alternative-orphan ]; then
    unit orphan 000000000000000a
  else
    unit dual 00000000000000a1
    unit dual 00000000000000a2
    unit app 00000000000000a3
  fi
  : >"$FAKE_STATE/alternative-built"
  exit 0
fi
unit dual 0000000000000001
unit dual 0000000000000002
unit app 0000000000000003
case "${FAKE_MODE:-}" in
  never-settles)
    count=$(($(cat "$FAKE_STATE/count" 2>/dev/null || echo 0) + 1))
    echo "$count" >"$FAKE_STATE/count"
    find "$profile" -path '*drift*' -exec touch -h -d 2000-01-01 {} +
    unit drift "$(printf '%016x' "$count")"
    ;;
  supersedes-late)
    [ ! -e "$FAKE_STATE/alternative-built" ] || unit late 0000000000000005
    ;;
esac
CARGO
  chmod +x "$dir/bin/cargo"
  printf '%s\n' "$dir"
}

# Every entry under the profile set to one old instant, as a restored CI cache
# leaves them: older than the window, with no unit's hashes told apart by age.
age() { find "$1/target" -mindepth 1 -exec touch -h -d '2000-06-01T00:00:00Z' {} +; }

measure() {
  local dir="$1" mode="$2"
  (cd "$dir" && PATH="$dir/bin:$PATH" FAKE_STATE="$dir/state" FAKE_MODE="$mode" \
    ONEBUDGETSPEC_RESULT="$dir/state/result.json" bash pool-maintenance/measure.sh) \
    >"$dir/state/log" 2>&1
}

expect_ok() {
  local dir="$1" rounds="$2" log="$1/state/log"
  grep -qx 'measure: the timed pass removed 6 of the alternative build'"'"'s 6 entries; untimed backlog passes .*; lists in target/pool-maintenance' "$log" ||
    fail "$3: the timed pass did not remove exactly the alternative build's 6 entries" "$log"
  local passes
  passes="$(sed -n 's/^measure: .*untimed backlog passes \(.*\); lists in .*/\1/p' "$log" | awk -F' [+] ' '{ print NF }')"
  [ "$passes" = "$rounds" ] || fail "$3: expected $rounds untimed backlog passes, the log reports $passes" "$log"
  # shellcheck disable=SC2016 # JavaScript, whose `${...}` templates are for node to expand
  node -e '
    const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
    if (typeof r.value !== "number" || !(r.value >= 0)) throw new Error(`no seconds figure: ${JSON.stringify(r)}`);
    if (!/builds removed: 3;/.test(r.detail)) throw new Error(`the pruner'"'"'s own detail is gone: ${r.detail}`);
    if (!/; untimed backlog passes beforehand: [0-9.-]+s( [+] [0-9.-]+s)* \(not counted\)$/.test(r.detail))
      throw new Error(`the backlog passes are missing from the detail: ${r.detail}`);
  ' "$dir/state/result.json" 2>>"$log" || fail "$3: the result file is not the timed pass's figure" "$log"
}

expect_refusal() {
  local dir="$1" message="$2" log="$1/state/log"
  grep -qF "measure: $message" "$log" || fail "$3: expected the refusal '$message'" "$log"
  grep -q '^ACTION: ' "$log" || fail "$3: the refusal names no next action" "$log"
}

# A fresh checkout is already steady: one backlog round, which removes nothing.
dir="$(checkout fresh)"
measure "$dir" "" || fail "a fresh checkout refused" "$dir/state/log"
expect_ok "$dir" 1 "a fresh checkout"

# A cache-aged checkout: one pass drops the older half of `dual`, the rebuild
# makes the other half the superseded one, and only the third round settles.
dir="$(checkout aged)"
PATH="$dir/bin:$PATH" FAKE_STATE="$dir/state" bash -c "cd '$dir' && cargo" || fail "the cargo double failed"
age "$dir"
measure "$dir" "" || fail "a cache-aged checkout refused" "$dir/state/log"
expect_ok "$dir" 3 "a cache-aged checkout"
[ -s "$dir/target/pool-maintenance/backlog-2-removed.txt" ] ||
  fail "a cache-aged checkout: round 2 removed nothing, so the aged half was not the one at stake" "$dir/state/log"

# A build that supersedes one of its own units every time never settles.
dir="$(checkout never-settles)"
PATH="$dir/bin:$PATH" FAKE_STATE="$dir/state" FAKE_MODE=never-settles bash -c "cd '$dir' && cargo" ||
  fail "the cargo double failed"
measure "$dir" never-settles && fail "a build that never settles was timed" "$dir/state/log"
expect_refusal "$dir" "4 backlog rounds each removed builds, so the ordinary build never became current" \
  "a build that never settles"

# The timed pass removed an entry the alternative build did not create.
dir="$(checkout supersedes-late)"
mkdir -p "$dir/target/llvm-cov-target/debug/deps"
printf 'build\n' >"$dir/target/llvm-cov-target/debug/deps/liblate-0000000000000004.rlib"
age "$dir"
measure "$dir" supersedes-late && fail "a pass over a second superseded build was timed" "$dir/state/log"
expect_refusal "$dir" "the timed pass removed 1 entries the alternative build did not create" \
  "an ordinary build that supersedes another"

# The timed pass removed nothing.
dir="$(checkout alternative-orphan)"
measure "$dir" alternative-orphan && fail "a pass that removed nothing was timed" "$dir/state/log"
expect_refusal "$dir" "the timed pass removed nothing, so it did not measure the workload" \
  "an alternative build that supersedes nothing"

echo "pool-maintenance test: 5 measure.sh journeys passed"
