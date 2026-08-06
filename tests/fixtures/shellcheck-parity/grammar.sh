#!/usr/bin/env bash
# shellcheck disable=SC2086,SC2046
echo hi
# shellcheck disable=SC2000-SC2100
echo $1
# shellcheck disable=all
echo $(echo $2)
echo '# shellcheck disable=SC2148 inside a string, not a directive'

# llmlint: ignore-file[tool_output_is_signal, suppressions_justified] fixture
# input, not a script this project runs: ShellCheck reads it and nothing
# executes it, so the noisy expansions are the violations the parity test needs,
# and the reason-less directives are the form it asserts comes back with a null
# `reason`.
