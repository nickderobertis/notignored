#!/usr/bin/env bash
# shellcheck disable=SC2086  # every expansion here is a pre-split argument list
set -eu

# llmlint: ignore-block[tool_output_is_signal] the release log is the artifact this script ships
echo "releasing $1"
echo $(id -u)
# llmlint: ignore-end[tool_output_is_signal]

# shellcheck disable=SC2046,SC2116
echo $(echo "$2")
