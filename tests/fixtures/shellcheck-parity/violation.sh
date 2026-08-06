#!/usr/bin/env bash
# SC2086 fires here: the expansion is unquoted on purpose.
echo hi
echo $1

# llmlint: ignore-file[tool_output_is_signal] fixture input, not a script this
# project runs: ShellCheck reads it and nothing executes it, and the unquoted
# expansions it prints are the very violations the parity test needs it to
# have.
