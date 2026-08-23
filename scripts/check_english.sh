#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source_root="${SKIT_ENGLISH_SOURCE_ROOT:-$(cd -- "$script_dir/.." && pwd)}"
source_root="$(cd -- "$source_root" && pwd)"
allowlist="$script_dir/english_contractions.allow"

if [[ $# -eq 0 ]]; then
  full_scan=1
  english_files=(
    README.md
    CONTRIBUTING.md
    benchmarks/README.md
    skills/skit/SKILL.md
  )
  # Translated files are not English sources. Their locale suffix excludes them.
  while IFS= read -r english_file; do
    case $english_file in
      *.zh-CN.*|*.zh-TW.*) continue ;;
    esac
    english_files+=("$english_file")
  done < <(
    find "$source_root/docs/app" "$source_root/docs/components" "$source_root/docs/content" \
      "$source_root/docs/design" "$source_root/docs/lib" "$source_root/docs/public" -type f \
      \( -name '*.html' -o -name '*.md' -o -name '*.mdx' -o -name '*.mjs' \
        -o -name '*.ts' -o -name '*.tsx' \) \
      -print | sed "s#^$source_root/##" | sort
    find "$source_root/docs" -maxdepth 1 -type f \
      \( -name '*.html' -o -name '*.md' -o -name '*.mdx' -o -name '*.mjs' \
        -o -name '*.ts' -o -name '*.tsx' \) \
      -print | sed "s#^$source_root/##" | sort
    find "$source_root/docs/scripts" -type f \
      \( -name '*.mjs' -o -name '*.ts' -o -name '*.tsx' \) \
      -print | sed "s#^$source_root/##" | sort
    find "$source_root/crates" -type f -name '*.rs' -print | sed "s#^$source_root/##" | sort
  )
else
  full_scan=0
  english_files=("$@")
fi

english_contraction="(^|[^[:alpha:]])((aren|can|couldn|didn|doesn|don|hadn|hasn|haven|isn|mustn|needn|shan|shouldn|wasn|weren|won|wouldn)['’]t|(I|you|we|they)['’](d|ll|re|ve)|(he|she|it)['’](d|ll)|(it|that|there|here|what|who)['’]s)([^[:alpha:]]|$)"
english_failed=0

declare -a allowed_keys=()
declare -A allowed_seen=()
declare -A allowed_category=()
declare -A allowed_expected=()
declare -A allowed_actual=()
declare -A allowed_path=()
declare -A allowed_fragment=()
declare -A allowed_rationale=()

if [[ ! -s $allowlist ]]; then
  echo "English contraction allowlist does not exist or is empty: $allowlist" >&2
  exit 1
fi

while IFS=$'\t' read -r category expected path fragment rationale extra; do
  [[ -z $category || $category == \#* ]] && continue
  if [[ -n ${extra:-} || -z $path || -z $fragment || -z $rationale ]]; then
    echo "English contraction allowlist row is incomplete: $category $path" >&2
    exit 1
  fi
  case $category in
    A|D) ;;
    *)
      echo "English contraction allowlist category is not A or D: $category" >&2
      exit 1
      ;;
  esac
  if [[ ! $expected =~ ^[1-9][0-9]*$ ]]; then
    echo "English contraction allowlist count is not positive: $path: $expected" >&2
    exit 1
  fi
  if [[ $path = /* || $path == *'..'* ]]; then
    echo "English contraction allowlist path is not repository-relative: $path" >&2
    exit 1
  fi
  key="$path"$'\x1f'"$fragment"
  if [[ -n ${allowed_seen[$key]:-} ]]; then
    echo "English contraction allowlist has a duplicate raw owner: $path: $fragment" >&2
    exit 1
  fi
  allowed_seen[$key]=1
  allowed_keys+=("$key")
  allowed_category[$key]=$category
  allowed_expected[$key]=$expected
  allowed_actual[$key]=0
  allowed_path[$key]=$path
  allowed_fragment[$key]=$fragment
  allowed_rationale[$key]=$rationale
done < "$allowlist"

declare -A scanned_files=()

for english_file in "${english_files[@]}"; do
  case $english_file in
    *.zh-CN.*|*.zh-TW.*) continue ;;
  esac
  if [[ $english_file = /* ]]; then
    physical_file=$english_file
    case $physical_file in
      "$source_root"/*) relative_file=${physical_file#"$source_root"/} ;;
      *) relative_file=$physical_file ;;
    esac
  else
    relative_file=$english_file
    physical_file="$source_root/$english_file"
  fi
  if [[ -n ${scanned_files[$relative_file]:-} ]]; then
    echo "English source was listed more than once: $relative_file" >&2
    english_failed=1
    continue
  fi
  scanned_files[$relative_file]=1
  [[ -f $physical_file ]] || {
    echo "English source does not exist: $relative_file" >&2
    english_failed=1
    continue
  }
  while IFS=: read -r line_number source_line; do
    [[ -n $line_number ]] || continue
    trimmed_line=$source_line
    trimmed_line="${trimmed_line#"${trimmed_line%%[![:space:]]*}"}"
    trimmed_line="${trimmed_line%"${trimmed_line##*[![:space:]]}"}"
    raw_count="$(printf '%s\n' "$source_line" | grep -Eio "$english_contraction" | wc -l)"
    raw_count="${raw_count//[[:space:]]/}"
    matched_key=
    match_count=0
    for key in "${allowed_keys[@]}"; do
      if [[ ${allowed_path[$key]} == "$relative_file" \
        && ${allowed_fragment[$key]} == "$trimmed_line" ]]; then
        matched_key=$key
        ((match_count += 1))
      fi
    done
    if ((match_count > 1)); then
      echo "$relative_file:$line_number: allowlist match is ambiguous: $trimmed_line" >&2
      english_failed=1
    elif ((match_count == 1)); then
      allowed_actual[$matched_key]=$((allowed_actual[$matched_key] + raw_count))
    else
      echo "$relative_file:$line_number:$source_line: use the full English form" >&2
      english_failed=1
    fi
  done < <(grep -Ein "$english_contraction" "$physical_file" || true)
done

for key in "${allowed_keys[@]}"; do
  path=${allowed_path[$key]}
  if [[ -z ${scanned_files[$path]:-} ]]; then
    if ((full_scan)); then
      echo "$path: allowlist owner was not scanned: ${allowed_fragment[$key]}" >&2
      english_failed=1
    fi
    continue
  fi
  expected=${allowed_expected[$key]}
  actual=${allowed_actual[$key]}
  if ((actual != expected)); then
    echo "$path: allowlisted contraction count changed: expected $expected, found $actual: ${allowed_fragment[$key]} (${allowed_category[$key]}: ${allowed_rationale[$key]})" >&2
    english_failed=1
  fi
done

exit "$english_failed"
