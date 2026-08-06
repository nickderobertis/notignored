#!/usr/bin/env bash
# ShellCheck rejects both of these, so neither suppresses anything.
echo hi
# shellcheck disable=SC2086 the reason needs its own '#' marker
echo $1
echo $2 # shellcheck disable=SC2086
