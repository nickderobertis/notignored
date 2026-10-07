#!/usr/bin/env bash
# Affected-only selection, keyed off an explicitly derived merge base.
#
# Two modes:
#   scripts/nx-affected.sh -t check            run a target over the affected projects
#   scripts/nx-affected.sh --affects NAME...   print `true` if any NAME is affected,
#                                              else `false`
#
# The base is the merge base with NOTIGNORED_NX_BASE_REF (or a pull request's
# GITHUB_BASE_REF), unless NOTIGNORED_NX_BASE_SHA names the base commit outright.
# That one wins over any ref: a push build has no base branch to fork from, so CI
# hands it the pushed range's base commit instead (`github.event.before`).
#
# Both **fail closed**: when the merge base cannot be derived — a shallow clone,
# a missing base branch, a detached build — this runs everything and says so on
# stderr rather than reporting a scoped pass as a full one. Affected selection is
# a speed optimisation, and a speed optimisation that can silently skip a check
# is a correctness hole.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || {
  echo "nx-affected: cannot enter the repository root $ROOT" >&2
  echo "ACTION: run this from a checkout whose directories are readable" >&2
  exit 1
}

# The base branch as GitHub names it on a pull request, or the local default.
#
# `GITHUB_BASE_REF` is workflow-controlled rather than attacker-controlled, but it
# reaches `git fetch` as a refspec, so its shape is validated at the boundary
# instead of trusted: a branch name is what a branch name may look like.
#
# In CI its absence is meaningful rather than missing: a push build is *on* the
# base branch, so scoping against it would find nothing changed and skip every
# check. There is no base there, and no base means run everything.
base_branch() {
  local ref="${NOTIGNORED_NX_BASE_REF:-${GITHUB_BASE_REF:-}}"
  if [ -z "$ref" ]; then
    if [ -n "${CI:-}" ]; then
      echo "nx-affected: no base branch — this is not a pull-request build" >&2
      return 1
    fi
    printf 'main'
    return 0
  fi
  if ! printf '%s' "$ref" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9._/-]*$'; then
    echo "nx-affected: '$ref' is not a usable branch name" >&2
    return 1
  fi
  printf '%s' "$ref"
}

# The commit NOTIGNORED_NX_BASE_SHA names, or a refusal when it names none. It
# is never "close enough": a value that does not resolve selects everything
# rather than falling back to some other base the caller did not ask for.
explicit_base() {
  local sha="$NOTIGNORED_NX_BASE_SHA"
  # Matched as one string — grep would accept a value whose first line is hex.
  case "$sha" in
  "" | *[!0-9a-fA-F]*)
    echo "nx-affected: NOTIGNORED_NX_BASE_SHA '$sha' is not a commit id" >&2
    echo "ACTION: set it to a hexadecimal commit id, or unset it to use the merge base" >&2
    return 1
    ;;
  esac
  if ! git rev-parse --verify --quiet "$sha^{commit}" 2>/dev/null; then
    echo "nx-affected: NOTIGNORED_NX_BASE_SHA '$sha' is not a commit in this checkout" >&2
    echo "ACTION: fetch that commit (a full-depth checkout has it), or correct the variable" >&2
    return 1
  fi
}

# The merge base this branch forked from, or nothing when it cannot be derived.
resolve_base() {
  if [ -n "${NOTIGNORED_NX_BASE_SHA:-}" ]; then
    explicit_base
    return
  fi
  local branch
  branch="$(base_branch)" || return 1
  # A PR runner's checkout has the base branch only as a remote-tracking ref if
  # it was fetched; fetch it before asking for the merge base so detection does
  # not depend on how deep the checkout happened to be.
  if [ -n "${CI:-}" ]; then
    git fetch --no-tags --quiet origin \
      "+refs/heads/$branch:refs/remotes/origin/$branch" 2>/dev/null || true
  fi
  git merge-base "origin/$branch" HEAD 2>/dev/null || return 1
}

case "${1:-}" in
--affects)
  shift
  [ "$#" -gt 0 ] || set -- ""
  for name in "$@"; do
    case "$name" in
    "" | -* | *[!A-Za-z0-9_.-]*)
      echo "nx-affected: --affects needs project names; '$name' is not one" >&2
      echo "ACTION: name one or more, e.g. 'scripts/nx-affected.sh --affects notignored'" >&2
      exit 2
      ;;
    esac
  done
  wanted="$*"
  if ! base="$(resolve_base)"; then
    echo "nx-affected: no merge base — treating '$wanted' as affected" >&2
    echo "ACTION: none needed to stay safe; to scope, fetch the base branch or set NOTIGNORED_NX_BASE_SHA" >&2
    printf 'true\n'
    exit 0
  fi
  # Read for Nx's answer, so the wrapper must not fold it into a summary line.
  if ! projects="$(NOTIGNORED_NX_SHOW_OUTPUT=1 bash scripts/nx.sh show projects --affected --base="$base" --head=HEAD --json)"; then
    echo "nx-affected: Nx could not list the affected projects — treating '$wanted' as affected" >&2
    echo "ACTION: run 'just nx show projects --affected --json' to see Nx's own error" >&2
    printf 'true\n'
    exit 0
  fi
  # Matched as a parsed JSON array element rather than by grepping the text: a
  # project whose name is a substring of another's would otherwise answer for it.
  # Exit 0 is "affected", 1 is "not"; anything else — output that is not an array
  # of names, a node that would not run — is an answer nobody read, so it selects.
  # llmlint: ignore-block[changed_behavior_has_e2e] only a substitute for Nx could
  # print something other than its JSON list, and the journeys drive the real Nx
  # rather than a stub; tests/e2e/nx_workspace.rs proves the parsed path's true and
  # false answers, and every other way this script cannot decide fails closed too.
  verdict=0
  # shellcheck disable=SC2016 # JavaScript, whose `${...}` templates are for node to expand
  printf '%s' "$projects" | node -e '
    const fs = require("node:fs");
    let affected;
    try {
      affected = JSON.parse(fs.readFileSync(0, "utf8"));
    } catch (error) {
      console.error(`nx-affected: Nx printed no JSON: ${error.message}`);
      process.exit(2);
    }
    if (!Array.isArray(affected) || !affected.every((name) => typeof name === "string")) {
      console.error("nx-affected: Nx printed something other than an array of project names");
      process.exit(2);
    }
    process.exit(process.argv.slice(1).some((name) => affected.includes(name)) ? 0 : 1);
  ' "$@" || verdict=$?
  case "$verdict" in
  0) printf 'true\n' ;;
  1) printf 'false\n' ;;
  *)
    echo "nx-affected: could not read Nx's affected list — treating '$wanted' as affected" >&2
    echo "ACTION: run 'just nx show projects --affected --json' to see what Nx printed" >&2
    printf 'true\n'
    ;;
  esac
  # llmlint: ignore-end[changed_behavior_has_e2e]
  ;;
*)
  [ "$#" -gt 0 ] || {
    echo "nx-affected: pass the Nx arguments to run, e.g. '-t check'" >&2
    exit 2
  }
  if ! base="$(resolve_base)"; then
    echo "nx-affected: no merge base — running every project instead of the affected ones" >&2
    exec bash scripts/nx.sh run-many "$@"
  fi
  exec bash scripts/nx.sh affected --base="$base" --head=HEAD "$@"
  ;;
esac
