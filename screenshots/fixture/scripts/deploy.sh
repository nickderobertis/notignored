#!/usr/bin/env bash
# Ship the built artifact to the environment named by $1.
set -euo pipefail

environment="$1"
flags="$(cat "deploy/$environment.flags")"

# shellcheck disable=SC2086
rsync $flags ./dist/ "deploy@$environment:/srv/app/"

# The directive above is deliberately reason-less: an unjustified suppression is
# the finding this tool exists to surface, so the gallery has to show one.
# llmlint: ignore-file[suppressions_justified] fixture input; nothing here is run
