#!/usr/bin/env bash
set -euo pipefail

test_root=$(mktemp -d)
trap 'rm -rf -- "$test_root"' EXIT

cat > "$test_root/complete.json" <<'EOF'
{
  "metrics": {
    "startup.version.mean_ms": 1,
    "startup.list_empty.mean_ms": 1,
    "binary.release_bytes": 1,
    "repository.python_implementation_files": 0,
    "pipeline.skipped_count": 0
  }
}
EOF
bash benchmarks/check.sh "$test_root/complete.json" benchmarks/budgets.toml --require-enforced >/dev/null

jq 'del(.metrics["startup.list_empty.mean_ms"])' \
  "$test_root/complete.json" > "$test_root/missing.json"
if bash benchmarks/check.sh \
  "$test_root/missing.json" benchmarks/budgets.toml --require-enforced >/dev/null 2>&1; then
  echo "benchmark check accepted a skipped required metric" >&2
  exit 1
fi

jq '.metrics["pipeline.skipped_count"] = 1' \
  "$test_root/missing.json" > "$test_root/reported.json"
if bash benchmarks/check.sh \
  "$test_root/reported.json" benchmarks/budgets.toml --require-enforced >/dev/null 2>&1; then
  echo "benchmark check accepted a pipeline with one skipped metric" >&2
  exit 1
fi

mkdir "$test_root/bin" "$test_root/run-output"
cat > "$test_root/bin/hyperfine" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
while [[ $# -gt 0 ]]; do
  if [[ $1 == --export-json ]]; then
    output=$2
    break
  fi
  shift
done
printf '%s\n' '{"results":[{"command":"version","mean":0.001}]}' > "$output"
EOF
chmod +x "$test_root/bin/hyperfine"
PATH="$test_root/bin:$PATH" SKIT_BENCH_REPO=. \
  bash benchmarks/run.sh pr "$test_root/run-output" /bin/true >/dev/null
jq -e '.metrics["pipeline.skipped_count"] == 1' \
  "$test_root/run-output/results.json" >/dev/null
