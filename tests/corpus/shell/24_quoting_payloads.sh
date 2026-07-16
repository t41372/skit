#!/usr/bin/env bash
# Every quoting shape a const RHS can take, plus a read: the injector normalizes all of them to
# single-quoted literals, so an injected `$(touch pwned)` / `'; rm -rf ~; echo '` stays inert text.
BARE=plain
RAW='single quoted'
DOUBLE="double quoted"
NUMBER=42
read -p "Payload: " PAYLOAD
echo "$BARE|$RAW|$DOUBLE|$NUMBER|$PAYLOAD"
