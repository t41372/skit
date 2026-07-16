#!/usr/bin/env bash
: "${GREETING:-hello}"
echo "${TIMEOUT:=30}"
echo "${LEVEL-info}"
echo "${RETRIES=3}"
