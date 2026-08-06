#!/usr/bin/env bash
# Ship the built artifact to the environment named by $1.
set -euo pipefail

environment="$1"
flags="$(cat "deploy/$environment.flags")"

# shellcheck disable=SC2086  # the flags file is ours, and has to split into separate arguments
rsync $flags ./dist/ "deploy@$environment:/srv/app/"
