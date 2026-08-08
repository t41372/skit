#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 0 ]]; then
  english_files=(
    README.md
    CONTRIBUTING.md
    benchmarks/README.md
    docs/mutation-ledger.md
    skills/skit/SKILL.md
  )
  # Translated files are not English sources. Their locale suffix excludes them.
  while IFS= read -r english_file; do
    case $english_file in
      *.zh-CN.*|*.zh-TW.*) continue ;;
    esac
    english_files+=("$english_file")
  done < <(
    find docs/app docs/components docs/content docs/design docs/lib docs/public -type f \
      \( -name '*.html' -o -name '*.md' -o -name '*.mdx' -o -name '*.mjs' \
        -o -name '*.ts' -o -name '*.tsx' \) \
      -print | sort
    find docs -maxdepth 1 -type f \
      \( -name '*.html' -o -name '*.md' -o -name '*.mdx' -o -name '*.mjs' \
        -o -name '*.ts' -o -name '*.tsx' \) \
      -print | sort
  )
else
  english_files=("$@")
fi

english_contraction="(^|[^[:alpha:]])((aren|can|couldn|didn|doesn|don|hadn|hasn|haven|isn|mustn|needn|shan|shouldn|wasn|weren|won|wouldn)['’]t|(I|you|we|they)['’](d|ll|re|ve)|(he|she|it)['’](d|ll)|(it|that|there|here|what|who)['’]s)([^[:alpha:]]|$)"
english_failed=0
for english_file in "${english_files[@]}"; do
  [[ -f $english_file ]] || { echo "English source does not exist: $english_file" >&2; english_failed=1; continue; }
  if english_matches=$(grep -Ein "$english_contraction" "$english_file"); then
    printf '%s\n' "$english_matches" | while IFS= read -r english_match; do
      echo "$english_file:$english_match: use the full English form" >&2
    done
    english_failed=1
  fi
done

exit "$english_failed"
