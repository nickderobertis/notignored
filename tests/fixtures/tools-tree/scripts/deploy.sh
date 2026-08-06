#!/usr/bin/env bash
# shellcheck disable=SC2086  # every expansion here is a pre-split argument list
set -eu
echo $1
# shellcheck disable=SC2046,SC2000-SC2100
echo $(id -u)
# shellcheck disable=all
echo $2
