#!/usr/bin/env bash
set -euo pipefail

bench_script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bench_repo_root=$(cd -- "$bench_script_dir/.." && pwd)
bench_test_root=$(mktemp -d)
trap 'rm -rf -- "$bench_test_root"' EXIT

cargo test \
  --locked \
  --manifest-path "$bench_repo_root/Cargo.toml" \
  -p skit-benchmarks

cargo run \
  --locked \
  --manifest-path "$bench_repo_root/Cargo.toml" \
  -p skit-benchmarks \
  --bin skit-bench \
  -- datasets --n 3 --out "$bench_test_root/n3"

SKIT_DATA_DIR="$bench_test_root/n3/data" \
SKIT_STATE_DIR="$bench_test_root/n3/state" \
SKIT_CONFIG_DIR="$bench_test_root/n3/config" \
SKIT_LANG=en \
cargo run \
  --locked \
  --manifest-path "$bench_repo_root/Cargo.toml" \
  -p skit-benchmarks \
  --bin skit-bench \
  -- probe tui --entries 3 --probe-char o >/dev/null

cargo run \
  --locked \
  --manifest-path "$bench_repo_root/Cargo.toml" \
  -p skit-benchmarks \
  --bin skit-bench \
  -- probe analyze --kind python --source "$bench_script_dir/fixtures/noop.py"
