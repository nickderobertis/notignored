#!/usr/bin/env bash
# Decide ci.yml's `required` job: the one context branch protection requires.
#
# A job skipped by its `if:` reports `skipped`, and a *matrix* job skipped that
# way reports one `skipped` check run under its bare name — its per-variant
# contexts (`cross (macos-latest)`) are never emitted, so protection naming them
# waits forever. `required` needs every other job and runs `if: always()`, so it
# always reports; this script is what it reports.
#
# Accepting every `skipped` would be as wrong as accepting none: `gate` skipped,
# or `cross` skipped while `changes` said the crate is affected, is a check that
# never ran. So a skip passes only for the reason the job's own `if:` gives,
# decided from what `toJSON(needs)` carries (each job's `result` and `outputs`)
# and the event name (three conditions turn on it, and `needs` does not say it).
# Everything else fails closed, naming the job.
#
# Reads REQUIRED_NEEDS (the `toJSON(needs)` payload) and REQUIRED_EVENT
# (`github.event_name`). Needs `node`, which `just bootstrap` already requires
# and every GitHub-hosted runner carries.
set -euo pipefail

# The rules, one per job `required` covers: the job id, then the exact `if:` it
# carries in ci.yml, or `(none)` when it has none. The condition is recorded
# verbatim rather than paraphrased so tests/ci_contract.rs can hold every row
# against the workflow; a job added there without a row here, or a condition
# changed there and not here, fails the build rather than this job.
#
# What each condition lets a skip mean is decided in `skip_allowed` below.
rules="$(
  cat <<'RULES'
changes | (none)
gate | (none)
cross | needs.changes.outputs.crate == 'true'
msrv | needs.changes.outputs.crate == 'true'
deny | needs.changes.outputs.crate == 'true'
install | needs.changes.outputs.crate == 'true'
install-documented | github.event_name == 'push'
pr-title | github.event_name == 'pull_request'
llmlint | github.event_name == 'pull_request'
RULES
)"

die() {
  printf 'ci-required: %s\n' "$1" >&2
  printf 'ACTION: %s\n' "$2" >&2
  exit 1
}

# Every row's condition has to be one `skip_allowed` can read, checked before any
# payload is, so a row this script cannot interpret fails every run — the
# journey's included — instead of only the one where that job happens to skip.
while IFS= read -r rule; do
  case "${rule#* | }" in
    "(none)" | "needs.changes.outputs.crate == 'true'" | "github.event_name == '"*"'") ;;
    *) die "the rule '$rule' records a condition this script cannot judge a skip under" \
      "teach skip_allowed in scripts/ci-required.sh what a skip under that \`if:\` means" ;;
  esac
done <<EOF
$rules
EOF

event="${REQUIRED_EVENT:-}"
needs="${REQUIRED_NEEDS:-}"
[ -n "$event" ] || die "REQUIRED_EVENT is empty or unset, so no event-conditioned skip can be judged" \
  "pass \`github.event_name\` to this script as REQUIRED_EVENT"
[ -n "$needs" ] || die "REQUIRED_NEEDS is empty or unset, so no job's result is known" \
  "pass \`toJSON(needs)\` to this script as REQUIRED_NEEDS"
command -v node >/dev/null 2>&1 || die "node is not on PATH; it is what reads the \`needs\` payload" \
  "install Node.js (just bootstrap requires it), then re-run"

# The payload as one line per job: id, result, and `changes`' crate output (the
# only output a rule reads) as JSON, or `absent`. Its shape is checked here, so a
# truncated or reshaped payload is a named failure rather than zero jobs.
if ! table="$(
  # shellcheck disable=SC2016 # JavaScript, whose `${...}` templates are node's to expand
  node -e '
    let payload;
    try {
      payload = JSON.parse(process.env.REQUIRED_NEEDS);
    } catch (error) {
      console.error(`the needs payload is not JSON: ${error.message}`);
      process.exit(1);
    }
    if (payload === null || typeof payload !== "object" || Array.isArray(payload)) {
      console.error("the needs payload is not an object of jobs");
      process.exit(1);
    }
    for (const [job, entry] of Object.entries(payload)) {
      if (!/^[A-Za-z0-9_-]+$/.test(job)) {
        console.error(`the needs payload names ${JSON.stringify(job)}, which is not a job id`);
        process.exit(1);
      }
      const record = entry !== null && typeof entry === "object" ? entry : {};
      const result = typeof record.result === "string" ? record.result : "";
      // Every field lands in a tab-and-newline table, so one that could carry
      // either is refused here rather than allowed to forge a row. `crate` is
      // written as JSON, which escapes both.
      if (!/^[A-Za-z_]*$/.test(result)) {
        console.error(`${job} has result ${JSON.stringify(result)}, which is not a job result`);
        process.exit(1);
      }
      const outputs = record.outputs !== null && typeof record.outputs === "object" ? record.outputs : {};
      const crate = Object.hasOwn(outputs, "crate") ? JSON.stringify(outputs.crate) : "absent";
      console.log(`${job}\t${result}\t${crate}`);
    }
  ' 2>&1
)"; then
  die "$table" "hand this script \`toJSON(needs)\` unaltered, as REQUIRED_NEEDS"
fi

tab="$(printf '\t')"

payload_line() {
  printf '%s\n' "$table" | grep "^$1$tab" || true
}

rule_condition() {
  printf '%s\n' "$rules" | sed -n "s/^$1 | //p"
}

# Whether a skipped job was skipped for the reason its condition gives. Prints
# why not, and fails, when it was not.
skip_allowed() {
  local condition="$1" changes_line changes_result crate
  case "$condition" in
    "(none)")
      echo "it has no \`if:\`, so it never skips legitimately"
      return 1
      ;;
    "needs.changes.outputs.crate == 'true'")
      changes_line="$(payload_line changes)"
      changes_result="$(printf '%s' "$changes_line" | cut -f2)"
      crate="$(printf '%s' "$changes_line" | cut -f3)"
      if [ "$changes_result" = success ] && [ "$crate" = '"false"' ]; then
        return 0
      fi
      echo "its \`if:\` is $condition, and changes reported result '${changes_result:-absent}' with crate output ${crate:-absent} rather than succeeding with \"false\""
      return 1
      ;;
    "github.event_name == '"*"'")
      local wanted="${condition#github.event_name == \'}"
      wanted="${wanted%\'}"
      if [ "$event" != "$wanted" ]; then
        return 0
      fi
      echo "its \`if:\` is $condition, and this run's event is '$event'"
      return 1
      ;;
    *)
      echo "no reading of \`if: $condition\` allows a skip"
      return 1
      ;;
  esac
}

failures=0
fail() {
  printf 'ci-required: %s: %s\n' "$1" "$2" >&2
  failures=$((failures + 1))
}

while IFS= read -r rule; do
  job="${rule%% | *}"
  condition="${rule#* | }"
  line="$(payload_line "$job")"
  if [ -z "$line" ]; then
    fail "$job" "has a rule but is absent from the needs payload — add it to \`required\`'s needs: in ci.yml"
    continue
  fi
  result="$(printf '%s' "$line" | cut -f2)"
  case "$result" in
    success) ;;
    skipped)
      if ! why="$(skip_allowed "$condition")"; then
        fail "$job" "result skipped — $why"
      fi
      ;;
    "") fail "$job" "has no result in the needs payload" ;;
    *) fail "$job" "result $result" ;;
  esac
done <<EOF
$rules
EOF

while IFS="$tab" read -r job result _; do
  [ -n "$job" ] || continue
  if [ -z "$(rule_condition "$job")" ]; then
    fail "$job" "result ${result:-absent} — no rule in scripts/ci-required.sh covers it; add its row with the \`if:\` it carries in ci.yml"
  fi
done <<EOF
$table
EOF

if [ "$failures" -gt 0 ]; then
  echo "ACTION: fix the job(s) named above, or — if a skip was legitimate — the rule that refused it" >&2
  exit 1
fi
echo "ci-required: every covered job succeeded or was skipped for its own condition"
