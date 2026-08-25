#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: benchmarks/check.sh RESULTS BUDGETS [OPTIONS]" >&2
  exit 2
fi

bench_results=$1
bench_budgets=$2
shift 2
bench_script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bench_repo_root=$(cd -- "$bench_script_dir/.." && pwd)

exec cargo run \
  --locked \
  --release \
  --manifest-path "$bench_repo_root/Cargo.toml" \
  -p skit-benchmarks \
  --bin skit-bench \
  -- check "$bench_results" --budgets "$bench_budgets" "$@"
