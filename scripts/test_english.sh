#!/usr/bin/env bash
set -euo pipefail

test_root=$(mktemp -d)
trap 'rm -rf -- "$test_root"' EXIT

printf '%s\n' 'Use the command. It does not change global data.' > "$test_root/clean.md"
bash scripts/check_english.sh "$test_root/clean.md"

printf '%s\n' "Don't change global data." > "$test_root/contraction.md"
if bash scripts/check_english.sh "$test_root/contraction.md" >/dev/null 2>&1; then
  echo "English check accepted a contraction" >&2
  exit 1
fi
