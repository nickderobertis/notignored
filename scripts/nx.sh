#!/usr/bin/env bash
# The one entry point to this workspace's Nx.
#
# Nx lives in `node_modules/.bin`, which a fresh clone does not have, so every
# invocation heals through a locked install first. That is what lets `just check`
# work from a clean clone without a separate "install the orchestrator" step, and
# what keeps one recipe from failing with `nx: command not found` while another
# quietly repaired it.
#
# Nx orchestrates targets; it is never a runtime dependency of the scripts it
# runs. Each target shells out to the project's own language-native tool.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || {
  echo "nx: cannot enter the repository root $ROOT" >&2
  echo "ACTION: run this from a checkout whose directories are readable" >&2
  exit 1
}

# The daemon is a long-lived background process per workspace root that buys
# about a tenth of a second here; it is not worth a resident process the gate
# never reaps. `NX_DAEMON=true` still turns it back on for anyone who wants it.
export NX_DAEMON="${NX_DAEMON-false}"
# Keep a daemon that *is* turned back on from fetching its own private `nx@latest`
# for housekeeping: this workspace's pinned Nx is the only one that may run.
export NX_USE_LOCAL=true

if [ ! -e node_modules/.bin/nx ] && [ ! -e node_modules/.bin/nx.cmd ]; then
  if ! command -v npm >/dev/null 2>&1; then
    echo "nx: npm not found; cannot install the pinned Nx the project graph needs" >&2
    echo "ACTION: install Node.js 20+ (https://nodejs.org/) and re-run 'just bootstrap'" >&2
    exit 1
  fi
  # Installer chatter is not this command's output: `nx show projects` and
  # friends are read for their stdout, so anything that is not Nx's answer goes
  # to stderr.
  if ! npm ci --silent --no-audit --no-fund >&2; then
    echo "nx: 'npm ci' failed in $ROOT" >&2
    echo "ACTION: check network access to the npm registry, then re-run 'just bootstrap'" >&2
    exit 1
  fi
fi

# The npm-written shim rather than a path inside the package: Nx has moved its
# bin entry between releases, and the shim is the one name that cannot.
if [ -e node_modules/.bin/nx ]; then
  exec node_modules/.bin/nx "$@"
fi
exec node_modules/.bin/nx.cmd "$@"
