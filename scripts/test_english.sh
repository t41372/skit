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

legacy="$test_root/legacy.md"
printf '%s\n' "The environment variable X isn't set (needed by {env:X})." > "$legacy"
if bash scripts/check_english.sh "$legacy" >/dev/null 2>&1; then
  echo "English check accepted frozen copy outside its exact owner" >&2
  exit 1
fi

test -s scripts/english_contractions.allow || {
  echo "English contraction allowlist is missing" >&2
  exit 1
}

allowlist_probe_root="$test_root/allowlist-root"
allowlist_probe_path="crates/skit-cli/src/cli.rs"
mkdir -p "$allowlist_probe_root/$(dirname "$allowlist_probe_path")"
cp "$allowlist_probe_path" "$allowlist_probe_root/$allowlist_probe_path"
SKIT_ENGLISH_SOURCE_ROOT="$allowlist_probe_root" \
  bash scripts/check_english.sh "$allowlist_probe_path"
perl -ni -e 'print unless /doesn.t take package dependencies; only --need applies/' \
  "$allowlist_probe_root/$allowlist_probe_path"
if SKIT_ENGLISH_SOURCE_ROOT="$allowlist_probe_root" \
  bash scripts/check_english.sh "$allowlist_probe_path" >/dev/null 2>&1; then
  echo "English check accepted a missing frozen contraction" >&2
  exit 1
fi
cp "$allowlist_probe_path" "$allowlist_probe_root/$allowlist_probe_path"
printf '%s\n' \
  '            Message::new("{} doesn'"'"'t take package dependencies; only --need applies")' \
  >> "$allowlist_probe_root/$allowlist_probe_path"
if SKIT_ENGLISH_SOURCE_ROOT="$allowlist_probe_root" \
  bash scripts/check_english.sh "$allowlist_probe_path" >/dev/null 2>&1; then
  echo "English check accepted a duplicate frozen contraction" >&2
  exit 1
fi

crate_probe=$(mktemp crates/skit-domain/src/english-check.XXXXXX.rs)
docs_probe=$(mktemp docs/scripts/english-check.XXXXXX.mjs)
trap 'rm -rf -- "$test_root"; rm -f -- "$crate_probe" "$docs_probe"' EXIT

printf '%s\n' "// This isn't valid new English." > "$crate_probe"
if bash scripts/check_english.sh >/dev/null 2>&1; then
  echo "English check did not scan Rust source" >&2
  exit 1
fi
rm -f -- "$crate_probe"

printf '%s\n' "// This isn't valid new English." > "$docs_probe"
if bash scripts/check_english.sh >/dev/null 2>&1; then
  echo "English check did not scan documentation scripts" >&2
  exit 1
fi
