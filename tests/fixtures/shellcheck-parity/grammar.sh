#!/usr/bin/env bash
# shellcheck disable=SC2086,SC2046
echo hi
# shellcheck disable=SC2000-SC2100
echo $1
# shellcheck disable=all
echo $(echo $2)
echo '# shellcheck disable=SC2148 inside a string, not a directive'
