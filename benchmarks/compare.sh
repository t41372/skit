#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: benchmarks/compare.sh BASE-RESULTS HEAD-RESULTS" >&2
  exit 2
fi

bench_script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bench_repo_root=$(cd -- "$bench_script_dir/.." && pwd)

exec cargo run \
  --locked \
  --release \
  --manifest-path "$bench_repo_root/Cargo.toml" \
  -p skit-benchmarks \
  --bin skit-bench \
  -- compare "$1" "$2"
