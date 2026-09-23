#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: benchmarks/run.sh PROFILE OUTPUT-DIRECTORY SKIT-BINARY" >&2
  exit 2
fi

bench_script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bench_repo_root=$(cd -- "$bench_script_dir/.." && pwd)
bench_args=(
  run
  --profile "$1"
  --out "$2"
  --budgets "$bench_script_dir/budgets.toml"
  --repo "$bench_repo_root"
  --skit-binary "$3"
)
if [[ -n ${SKIT_BENCH_REPO:-} ]]; then
  bench_args+=(--measured-repo "$SKIT_BENCH_REPO")
fi

exec cargo run \
  --locked \
  --release \
  --manifest-path "$bench_repo_root/Cargo.toml" \
  -p skit-benchmarks \
  --bin skit-bench \
  -- "${bench_args[@]}"
