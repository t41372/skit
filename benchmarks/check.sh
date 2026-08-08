#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo "usage: benchmarks/check.sh RESULTS BUDGETS [--require-enforced]" >&2
  exit 2
fi

bench_results=$1
bench_budgets=$2
bench_require=${3:-}
bench_script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bench_required=$bench_script_dir/required-metrics.txt
[[ -f "$bench_results" ]] || { echo "results file does not exist: $bench_results" >&2; exit 2; }
[[ -f "$bench_budgets" ]] || { echo "budget file does not exist: $bench_budgets" >&2; exit 2; }
[[ -f "$bench_required" ]] || { echo "required metric list does not exist: $bench_required" >&2; exit 2; }

bench_rows=$(awk '
  function emit() {
    if (metric != "") print metric "\t" maximum "\t" tier
  }
  /^\[\[budget\]\]$/ { emit(); metric=""; maximum=""; tier=""; next }
  /^metric = / { value=$0; sub(/^metric = "/, "", value); sub(/"$/, "", value); metric=value; next }
  /^max = / { value=$0; sub(/^max = /, "", value); maximum=value; next }
  /^tier = / { value=$0; sub(/^tier = "/, "", value); sub(/"$/, "", value); tier=value; next }
  END { emit() }
' "$bench_budgets")

bench_failed=0
bench_enforced=0
bench_skipped=0
while IFS= read -r bench_metric; do
  [[ -n $bench_metric ]] || continue
  if ! jq -e --arg metric "$bench_metric" \
    '(.metrics[$metric] | type) == "number"' "$bench_results" >/dev/null; then
    echo "missing required metric: $bench_metric" >&2
    bench_skipped=$((bench_skipped + 1))
  fi
done < "$bench_required"
if ! bench_reported_skips=$(jq -er '.metrics["pipeline.skipped_count"] | numbers' "$bench_results"); then
  echo "missing metric: pipeline.skipped_count" >&2
  bench_failed=1
elif [[ $bench_reported_skips -ne $bench_skipped ]]; then
  echo "skipped metric count mismatch: reported $bench_reported_skips, found $bench_skipped" >&2
  bench_failed=1
fi
while IFS=$'\t' read -r bench_metric bench_max bench_tier; do
  [[ -n "$bench_metric" ]] || continue
  if ! bench_value=$(jq -er --arg metric "$bench_metric" '.metrics[$metric]' "$bench_results"); then
    echo "missing metric: $bench_metric" >&2
    [[ $bench_tier == enforced ]] && bench_failed=1
    continue
  fi
  if [[ $bench_tier == enforced ]]; then
    bench_enforced=$((bench_enforced + 1))
  fi
  if awk -v value="$bench_value" -v maximum="$bench_max" 'BEGIN { exit !(value > maximum) }'; then
    echo "$bench_tier budget exceeded: $bench_metric = $bench_value, max = $bench_max" >&2
    [[ $bench_tier == enforced ]] && bench_failed=1
  else
    echo "$bench_tier budget passed: $bench_metric = $bench_value, max = $bench_max"
  fi
done <<< "$bench_rows"

if [[ $bench_require == --require-enforced && $bench_enforced -eq 0 ]]; then
  echo "no enforced budget was evaluated" >&2
  bench_failed=1
elif [[ -n $bench_require && $bench_require != --require-enforced ]]; then
  echo "unknown option: $bench_require" >&2
  exit 2
fi
exit "$bench_failed"
