#!/usr/bin/env bash
set -euo pipefail

expect_text() {
  local file=$1
  local text=$2
  if ! grep -Fq -- "$text" "$file"; then
    echo "$file does not contain the required text: $text" >&2
    return 1
  fi
}

expect_text Cargo.toml 'tree-sitter = "0.26.12"'
expect_text .github/workflows/ci.yml 'taiki-e/install-action@6c6fd71fe4fb72c3697d269963d0e15df8adedad # v2.85.10'
expect_text .github/workflows/mutation.yml 'taiki-e/install-action@6c6fd71fe4fb72c3697d269963d0e15df8adedad # v2.85.10'
expect_text .github/workflows/codspeed.yml 'CodSpeedHQ/action@4296e51e7041e24dadb86d1d6e8b9320d223dbe8 # v5.0.3'
expect_text .github/workflows/codspeed.yml 'taiki-e/install-action@6c6fd71fe4fb72c3697d269963d0e15df8adedad # v2.85.10'
expect_text .github/workflows/codspeed.yml 'tool: cargo-codspeed@5.0.1'
expect_text .github/workflows/codspeed.yml 'cargo codspeed build -m simulation --locked --workspace --all-features'
expect_text .github/workflows/codspeed.yml 'run: cargo codspeed run'
expect_text .github/workflows/release.yml 'pypa/gh-action-pypi-publish@dc37677b2e1c63e2034f94d8a5b11f265b73ba33 # v1.14.2'
expect_text pyproject.toml '{ path = "tests/corpus/**/*", format = "sdist" }'
expect_text .github/workflows/ci.yml 'cargo test --locked -p skit-language --test corpus'
expect_text CONTRIBUTING.md 'Node.js 26.7.0 and npm 12.0.2 or later'
expect_text AGENTS.md 'cargo mutants --workspace --all-features --cargo-arg=--locked --jobs 2 --minimum-test-timeout 20 --timeout-multiplier 3.0'
expect_text .github/workflows/mutation.yml 'cargo mutants --workspace --all-features --cargo-arg=--locked --jobs 2 --minimum-test-timeout 20 --timeout-multiplier 3.0'
expect_text .github/workflows/ci.yml 'zizmor .github/workflows .github/actions/install-hyperfine/action.yml'

while IFS= read -r use; do
  echo "GitHub action is not pinned to a full commit: $use" >&2
  exit 1
done < <(
  sed -nE 's/^[[:space:]]*uses:[[:space:]]*([^[:space:]#]+).*/\1/p' .github/workflows/*.yml |
    grep -v '^\./' |
    grep -Ev '@[0-9a-f]{40}$' || true
)
