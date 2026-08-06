#!/usr/bin/env bash
# shellcheck disable=SC2086  # every expansion here is a pre-split list
echo $1
echo $2

# llmlint: ignore-file[tool_output_is_signal] fixture input, not a script this
# project runs: ShellCheck reads it and nothing executes it, and the unquoted
# expansions it prints are the very violations the parity test needs it to
# have.
