#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: benchmarks/run.sh PROFILE OUTPUT-DIRECTORY SKIT-BINARY" >&2
  exit 2
fi

bench_profile=$1
bench_output=$2
bench_binary=$3
bench_repo=${SKIT_BENCH_REPO:-.}
bench_script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bench_required=$bench_script_dir/required-metrics.txt

case "$bench_profile" in
  pr | compare)
    bench_warmup=3
    bench_runs=15
    ;;
  full)
    bench_warmup=5
    bench_runs=40
    ;;
  *)
    echo "unknown benchmark profile: $bench_profile" >&2
    exit 2
    ;;
esac

command -v hyperfine >/dev/null
command -v jq >/dev/null
[[ -x "$bench_binary" ]] || { echo "skit binary is not executable: $bench_binary" >&2; exit 2; }

mkdir -p "$bench_output"
bench_tmp=$(mktemp -d)
trap 'rm -rf -- "$bench_tmp"' EXIT
mkdir -p "$bench_tmp/data" "$bench_tmp/state" "$bench_tmp/config"

printf -v bench_binary_q '%q' "$bench_binary"
printf -v bench_data_q '%q' "$bench_tmp/data"
printf -v bench_state_q '%q' "$bench_tmp/state"
printf -v bench_config_q '%q' "$bench_tmp/config"

hyperfine \
  --warmup "$bench_warmup" \
  --runs "$bench_runs" \
  --shell bash \
  --command-name version "$bench_binary_q --version" \
  --command-name list-empty \
  "env SKIT_DATA_DIR=$bench_data_q SKIT_STATE_DIR=$bench_state_q SKIT_CONFIG_DIR=$bench_config_q $bench_binary_q list --json" \
  --export-json "$bench_output/hyperfine.json"

bench_commit=$(git -C "$bench_repo" rev-parse HEAD)
bench_date=$(date -u +%Y-%m-%dT%H:%M:%SZ)
bench_platform="$(uname -s)-$(uname -m)"
bench_dirty=false
[[ -z "$(git -C "$bench_repo" status --porcelain)" ]] || bench_dirty=true
bench_ci=false
[[ ${GITHUB_ACTIONS:-false} == true ]] && bench_ci=true
bench_binary_bytes=$(wc -c < "$bench_binary" | tr -d ' ')
bench_python_files=$(
  git -C "$bench_repo" ls-files '*.py' \
    | while IFS= read -r bench_path; do
        [[ -f "$bench_repo/$bench_path" ]] || continue
        case "$bench_path" in
          tests/corpus/* | docs/assets/demo/scripts/* | benchmarks/fixtures/noop.py) continue ;;
        esac
        printf '%s\n' "$bench_path"
      done \
    | wc -l | tr -d ' '
)

jq \
  --arg profile "$bench_profile" \
  --arg commit "$bench_commit" \
  --arg date "$bench_date" \
  --arg platform "$bench_platform" \
  --argjson dirty "$bench_dirty" \
  --argjson ci "$bench_ci" \
  --argjson binary_bytes "$bench_binary_bytes" \
  --argjson python_files "$bench_python_files" \
  '{
    schema: 1,
    meta: {
      profile: $profile,
      commit: $commit,
      date: $date,
      platform: $platform,
      dirty: $dirty,
      ci: $ci
    },
    metrics: {
      "startup.version.mean_ms": (
        [.results[] | select(.command == "version") | .mean][0] as $mean
        | if $mean == null then null else $mean * 1000 end
      ),
      "startup.list_empty.mean_ms": (
        [.results[] | select(.command == "list-empty") | .mean][0] as $mean
        | if $mean == null then null else $mean * 1000 end
      ),
      "binary.release_bytes": $binary_bytes,
      "repository.python_implementation_files": $python_files
    },
    raw: .results
  }' "$bench_output/hyperfine.json" > "$bench_tmp/results.json"

bench_skipped=0
while IFS= read -r bench_metric; do
  [[ -n $bench_metric ]] || continue
  if ! jq -e --arg metric "$bench_metric" \
    '(.metrics[$metric] | type) == "number"' "$bench_tmp/results.json" >/dev/null; then
    bench_skipped=$((bench_skipped + 1))
  fi
done < "$bench_required"
jq --argjson skipped "$bench_skipped" \
  '.metrics["pipeline.skipped_count"] = $skipped' \
  "$bench_tmp/results.json" > "$bench_output/results.json"

{
  echo "## skit benchmark results"
  echo
  echo "Profile: \`$bench_profile\` · commit: \`$bench_commit\` · host: \`$bench_platform\`"
  echo
  echo "| Metric | Value |"
  echo "| --- | ---: |"
  jq -r '.metrics | to_entries[] | "| \(.key) | \(.value) |"' "$bench_output/results.json"
} > "$bench_output/results.md"
