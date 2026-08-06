#!/usr/bin/env bash
# A directive below the first command applies to the command that follows it.
echo hi
# shellcheck disable=SC2086  # the caller passes a pre-split argument list
echo $1
