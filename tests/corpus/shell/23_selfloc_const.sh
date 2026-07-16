#!/usr/bin/env bash
# $0 AND an injectable const: a const rewrite runs from a temp copy, so the script would see the
# copy's directory here. The injector warns; --normalize is the way out (env delivery, no copy).
HERE=$(dirname "$0")
OUTPUT_DIR=/tmp/out
RETRIES=3
echo "$HERE $OUTPUT_DIR $RETRIES"
