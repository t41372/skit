#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: scripts/check_coverage.sh LCOV-FILE" >&2
  exit 2
fi

coverage_input=$1
[[ -f "$coverage_input" ]] || { echo "coverage file does not exist: $coverage_input" >&2; exit 2; }

coverage_zero=$(mktemp)
trap 'rm -f -- "$coverage_zero"' EXIT

awk '
  /^SF:/ { file=substr($0, 4); next }
  /^DA:/ {
    split(substr($0, 4), row, ",")
    key=file SUBSEP row[1]
    hits[key]+=row[2]
    files[key]=file
    lines[key]=row[1]
  }
  END {
    for (key in hits) {
      if (hits[key] == 0) print files[key] "\t" lines[key]
    }
  }
' "$coverage_input" | sort > "$coverage_zero"

coverage_failed=0
coverage_function_re='^(pub\([^)]*\)[[:space:]]+|pub[[:space:]]+)?(const[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]][^{}]*(\{[[:space:]]*)?$'
coverage_signature_end_re='^\)[[:space:]]*(->[[:space:]]*[^{}]+)?[[:space:]]*\{$'
while IFS=$'\t' read -r coverage_file coverage_line; do
  [[ -n "$coverage_file" ]] || continue
  coverage_source=$(sed -n "${coverage_line}p" "$coverage_file")
  coverage_trimmed=${coverage_source#"${coverage_source%%[![:space:]]*}"}
  coverage_trimmed=${coverage_trimmed%"${coverage_trimmed##*[![:space:]]}"}
  coverage_syntax=$(printf '%s' "$coverage_trimmed" | sed 's/[][{}(),;?]//g')

  if [[ -z "$coverage_trimmed" \
    || "$coverage_trimmed" == //* \
    || "$coverage_trimmed" == \#\[* \
    || -z "$coverage_syntax" \
    || "$coverage_trimmed" =~ $coverage_function_re \
    || "$coverage_trimmed" =~ $coverage_signature_end_re ]]; then
    continue
  fi

  echo "uncovered executable source: ${coverage_file}:${coverage_line}: ${coverage_trimmed}" >&2
  coverage_failed=1
done < "$coverage_zero"

if [[ $coverage_failed -ne 0 ]]; then
  exit 1
fi

echo "complete executable-source line coverage"
