#!/usr/bin/env bash
set -euo pipefail

flags="$(cat deploy.flags)"

# llmlint: ignore[no_debug_prints] the rsync progress below is the operator's only view
# shellcheck disable=SC2086
rsync $flags ./dist/ deploy@host:/srv/app/
