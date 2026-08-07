#!/usr/bin/env bash
# Ship the built artifact to the environment named by $1.
set -euo pipefail

environment="$1"
flags="$(cat "deploy/$environment.flags")"

# shellcheck disable=SC2086  # the flags file is ours, and has to split into separate arguments
rsync $flags ./dist/ "deploy@$environment:/srv/app/"

# These nine lines are sized to show one realistic shellcheck suppression, not
# the error handling an operational script owes.
# llmlint: ignore-file[tool_output_is_signal] example input the README quickstart scans, not a script this project runs

