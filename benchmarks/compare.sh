#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: benchmarks/compare.sh BASE-RESULTS HEAD-RESULTS" >&2
  exit 2
fi

jq -r -s '
  .[0].metrics as $base |
  .[1].metrics as $head |
  "## skit benchmark comparison\n\n| Metric | Base | Head | Delta |\n| --- | ---: | ---: | ---: |\n" +
  ([$head | keys[]] | map(
    . as $key |
    ($base[$key] // 0) as $before |
    ($head[$key] // 0) as $after |
    "| \($key) | \($before) | \($after) | \($after - $before) |"
  ) | join("\n"))
' "$1" "$2"

