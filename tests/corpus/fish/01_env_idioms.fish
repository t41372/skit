#!/usr/bin/env fish
# Canonical env-default idioms: one-line `; or`, newline-continued `or`, int/float/str.

set -q PORT; or set PORT 8080
set -q RATE; or set RATE 2.5

set -q REGION
or set REGION us-east-1

# -gx on the guarded set half still preserves an inherited value (the `or` only fires when unset).
set -q LOG_DIR; or set -gx LOG_DIR /var/log/app

echo "listening on $PORT in $REGION at $RATE, logging to $LOG_DIR"
