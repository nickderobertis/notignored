#!/usr/bin/env bash
# ShellCheck rejects both of these, so neither suppresses anything.
echo hi
# shellcheck disable=SC2086 the reason needs its own '#' marker
echo $1
echo $2 # shellcheck disable=SC2086

# llmlint: ignore-file[tool_output_is_signal] fixture input, not a script this
# project runs: ShellCheck reads it and nothing executes it, and the unquoted
# expansions it prints are the very violations the parity test needs it to
# have.
