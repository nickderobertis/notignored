#!/usr/bin/env bash
# A directive below the first command applies to the command that follows it.
echo hi
# shellcheck disable=SC2086  # the caller passes a pre-split argument list
echo $1

# llmlint: ignore-file[tool_output_is_signal] fixture input, not a script this
# project runs: ShellCheck reads it and nothing executes it, and the unquoted
# expansions it prints are the very violations the parity test needs it to
# have.
