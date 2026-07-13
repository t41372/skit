#!/usr/bin/env bash
set -euo pipefail
OUTPUT_DIR=/tmp/out
MAX_RETRIES=3
: "${LOG_LEVEL:=info}"
read -p "Deploy to production? " confirm
echo "$OUTPUT_DIR $MAX_RETRIES $LOG_LEVEL $confirm"
