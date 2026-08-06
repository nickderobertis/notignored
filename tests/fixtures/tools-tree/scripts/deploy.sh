#!/usr/bin/env bash
# shellcheck disable=SC2086  # every expansion here is a pre-split argument list
set -eu
echo $1
# shellcheck disable=SC2046,SC2000-SC2100
echo $(id -u)
# shellcheck disable=all
echo $2

# llmlint: ignore-file[tool_output_is_signal, suppressions_justified] fixture
# input, not a script this project runs: ShellCheck reads it and nothing
# executes it, so the noisy expansions are the violations the parity test needs,
# and the reason-less directives are the form it asserts comes back with a null
# `reason`.
