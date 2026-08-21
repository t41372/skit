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
expect_text .github/workflows/ci.yml 'workflow_dispatch:'
expect_text .github/workflows/mutation.yml 'taiki-e/install-action@6c6fd71fe4fb72c3697d269963d0e15df8adedad # v2.85.10'
expect_text .github/workflows/codspeed.yml 'CodSpeedHQ/action@4296e51e7041e24dadb86d1d6e8b9320d223dbe8 # v5.0.3'
expect_text .github/workflows/codspeed.yml 'taiki-e/install-action@6c6fd71fe4fb72c3697d269963d0e15df8adedad # v2.85.10'
expect_text .github/workflows/codspeed.yml 'tool: cargo-codspeed@5.0.1'
expect_text .github/workflows/codspeed.yml 'cargo codspeed build -m simulation --locked --workspace --all-features'
expect_text .github/workflows/codspeed.yml 'run: cargo codspeed run'
expect_text .github/workflows/benchmark-compare.yml 'enable-cache: false'
expect_text .github/workflows/benchmark-compare.yml 'ref: ${{ github.event.pull_request.base.sha }}'
expect_text .github/workflows/benchmark-compare.yml 'ref: ${{ github.event.pull_request.head.sha }}'
if grep -Eq 'workflow_dispatch:|inputs\.(base|head)' .github/workflows/benchmark-compare.yml; then
  echo 'the comparison workflow must not execute caller-selected refs in the default-branch cache scope' >&2
  exit 1
fi
if grep -Eq 'enable-cache:[[:space:]]*true|uses:[[:space:]]*actions/cache@' \
  .github/workflows/benchmark-compare.yml; then
  echo 'the ref-selectable benchmark workflow must not save a cache after running either side' >&2
  exit 1
fi
expect_text .github/workflows/release.yml 'pypa/gh-action-pypi-publish@dc37677b2e1c63e2034f94d8a5b11f265b73ba33 # v1.14.2'
expect_text .github/workflows/release.yml 'workflow_dispatch:'
expect_text .github/workflows/release.yml "if: github.event_name == 'push'"
expect_text .github/workflows/release.yml "if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')"
expect_text .github/workflows/release.yml 'name: Smoke-test the native Linux wheel'
expect_text .github/workflows/release.yml 'name: Smoke-test the native Windows wheel'
expect_text .github/workflows/release.yml 'name: Smoke-test the native macOS wheel'
expect_text .github/workflows/release.yml 'export UV_TOOL_BIN_DIR="$RUNNER_TEMP/skit-bin"'
expect_text .github/workflows/release.yml 'Join-Path $env:RUNNER_TEMP'
expect_text .github/workflows/release.yml 'name: Verify the complete distribution set'
expect_text .github/workflows/release.yml 'test "$wheel_count" -eq 8'
expect_text .github/workflows/release.yml 'test "$sdist_count" -eq 1'
expect_text .github/workflows/release.yml 'unzip -t "$wheel"'
expect_text .github/workflows/release.yml "grep -Eq '/skills/skit/SKILL[.]md$'"
expect_text .github/workflows/release.yml "grep -Eq '/tests/corpus/'"
expect_text .github/workflows/release.yml 'needs: [verify-artifacts]'
expect_text pyproject.toml '{ path = "tests/corpus/**/*", format = "sdist" }'
expect_text .github/workflows/ci.yml 'cargo test --locked -p skit-language --test corpus'
expect_text .github/workflows/ci.yml 'fish-actions/install-fish@d6d9d26231a15f8d9a6b3e74b3db45512440e3e8 # v1.1.0'
expect_text .github/workflows/ci.yml 'actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7.0.0'
expect_text .github/workflows/ci.yml 'node-version: "26.7.0"'
expect_text .github/workflows/ci.yml 'run: node --version'
expect_text .github/workflows/ci.yml 'echo "SKIT_REQUIRE_NODE_RUNTIME=1" >> "$GITHUB_ENV"'
expect_text .github/workflows/docs.yml 'run: npm install --global npm@12.0.2'
expect_text .github/workflows/ci.yml 'uv::private_tests::test_extract_uv_skips_dir_fsync_on_windows -- --exact --ignored'
expect_text crates/skit-runtime/src/uv.rs 'uv::private_tests::test_extract_uv_skips_dir_fsync_on_windows -- --exact --ignored'
expect_text .github/workflows/ci.yml "throw 'the native Windows uv gate ran zero tests'"
expect_text .github/workflows/ci.yml 'test_the_preamble_runs_on_every_supported_dialect -- --exact --ignored'
expect_text .github/workflows/ci.yml 'the complete POSIX shell gate ran zero tests'
expect_text .github/workflows/ci.yml 'the CPython 3.13 compile gate ran zero tests'
fish_action_line="$(
  grep -Fn 'uses: fish-actions/install-fish@d6d9d26231a15f8d9a6b3e74b3db45512440e3e8 # v1.1.0' \
    .github/workflows/ci.yml | cut -d: -f1
)"
test "$(printf '%s\n' "$fish_action_line" | sed '/^$/d' | wc -l)" -eq 1 || {
  echo '.github/workflows/ci.yml must install Fish exactly once' >&2
  exit 1
}
fish_step_start=$((fish_action_line - 2))
sed -n "${fish_step_start},${fish_action_line}p" .github/workflows/ci.yml |
  grep -Fq "if: runner.os != 'Windows'" || {
    echo 'the pinned Fish action must be limited to supported non-Windows test hosts' >&2
    exit 1
  }
expect_text .github/workflows/ci.yml 'echo "SKIT_REQUIRE_FISH_RUNTIME=1" >> "$GITHUB_ENV"'
fish_flag_line="$(
  grep -Fn 'echo "SKIT_REQUIRE_FISH_RUNTIME=1" >> "$GITHUB_ENV"' .github/workflows/ci.yml |
    cut -d: -f1
)"
test "$(printf '%s\n' "$fish_flag_line" | sed '/^$/d' | wc -l)" -eq 1 || {
  echo '.github/workflows/ci.yml must require the Fish runtime owner exactly once' >&2
  exit 1
}
fish_flag_step_start=$((fish_flag_line - 3))
sed -n "${fish_flag_step_start},${fish_flag_line}p" .github/workflows/ci.yml |
  grep -Fq "if: runner.os != 'Windows'" || {
    echo 'the Fish runtime requirement must be limited to non-Windows test hosts' >&2
    exit 1
  }
fish_test_line="$(
  grep -Fn 'run: cargo test --locked --workspace --all-targets --all-features' \
    .github/workflows/ci.yml | cut -d: -f1
)"
test "$fish_action_line" -lt "$fish_flag_line" && test "$fish_flag_line" -lt "$fish_test_line" || {
  echo 'Fish must be installed before the workspace test step' >&2
  exit 1
}
node_action_line="$(
  grep -Fn 'uses: actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7.0.0' \
    .github/workflows/ci.yml | cut -d: -f1
)"
node_check_line="$(grep -Fn 'run: node --version' .github/workflows/ci.yml | cut -d: -f1)"
test "$(printf '%s\n' "$node_action_line" | sed '/^$/d' | wc -l)" -eq 1 || {
  echo '.github/workflows/ci.yml must install Node.js exactly once in the test matrix' >&2
  exit 1
}
test "$(printf '%s\n' "$node_check_line" | sed '/^$/d' | wc -l)" -eq 1 || {
  echo '.github/workflows/ci.yml must verify Node.js exactly once in the test matrix' >&2
  exit 1
}
test "$node_action_line" -lt "$node_check_line" && test "$node_check_line" -lt "$fish_test_line" || {
  echo 'Node.js must be installed and verified before the workspace test step' >&2
  exit 1
}
docs_node_line="$(
  grep -Fn 'uses: actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7.0.0' \
    .github/workflows/docs.yml | cut -d: -f1
)"
docs_npm_line="$(grep -Fn 'run: npm install --global npm@12.0.2' \
  .github/workflows/docs.yml | cut -d: -f1)"
docs_ci_line="$(grep -Fn 'run: npm ci' .github/workflows/docs.yml | cut -d: -f1)"
test "$(printf '%s\n' "$docs_npm_line" | sed '/^$/d' | wc -l)" -eq 1 &&
  test "$docs_node_line" -lt "$docs_npm_line" && test "$docs_npm_line" -lt "$docs_ci_line" || {
  echo 'the documented npm release must be installed once before docs dependencies' >&2
  exit 1
}
windows_uv_line="$(
  grep -Fn 'uv::private_tests::test_extract_uv_skips_dir_fsync_on_windows -- --exact --ignored' \
    .github/workflows/ci.yml | cut -d: -f1
)"
test "$(printf '%s\n' "$windows_uv_line" | sed '/^$/d' | wc -l)" -eq 1 || {
  echo '.github/workflows/ci.yml must run the native Windows uv gate exactly once' >&2
  exit 1
}
sed -n "$((windows_uv_line - 2)),$((windows_uv_line - 1))p" .github/workflows/ci.yml |
  grep -Fq "if: runner.os == 'Windows'" || {
    echo 'the uv directory-sync gate must run only on Windows' >&2
    exit 1
  }
shell_install_line="$(grep -Fn 'sudo apt-get update && sudo apt-get install --yes zsh' \
  .github/workflows/ci.yml | cut -d: -f1)"
shell_gate_line="$(
  grep -Fn 'test_the_preamble_runs_on_every_supported_dialect -- --exact --ignored' \
    .github/workflows/ci.yml | cut -d: -f1
)"
shell_gate_step_line="$(
  grep -Fn -- '- name: Run every supported POSIX shell' .github/workflows/ci.yml | cut -d: -f1
)"
test "$(printf '%s\n' "$shell_install_line" | sed '/^$/d' | wc -l)" -eq 1 &&
  test "$(printf '%s\n' "$shell_gate_step_line" | sed '/^$/d' | wc -l)" -eq 1 &&
  test "$(printf '%s\n' "$shell_gate_line" | sed '/^$/d' | wc -l)" -eq 1 &&
  test "$shell_install_line" -lt "$shell_gate_line" || {
  echo 'the Linux shell matrix must be installed before its one native gate' >&2
  exit 1
}
sed -n "${shell_gate_step_line},${shell_gate_line}p" .github/workflows/ci.yml |
  grep -Fq "if: runner.os == 'Linux'" || {
    echo 'the complete POSIX shell gate must run on Linux' >&2
    exit 1
  }
expect_text CONTRIBUTING.md 'Node.js 26.7.0 and npm 12.0.2 or later'
expect_text AGENTS.md 'cargo mutants --workspace --all-features --cargo-arg=--locked --jobs 2 --minimum-test-timeout 20 --timeout-multiplier 3.0'
expect_text .github/workflows/mutation.yml 'cargo mutants --workspace --all-features --cargo-arg=--locked --jobs 2 --minimum-test-timeout 20 --timeout-multiplier 3.0'
expect_text .github/workflows/ci.yml 'zizmor .github/workflows .github/actions/install-hyperfine/action.yml'

for demo in \
  'README.md en' \
  'README.zh-CN.md zh-CN' \
  'README.zh-TW.md zh-TW'; do
  read -r readme locale <<<"$demo"
  for screen in library form add settings; do
    asset="docs/assets/tui-$screen-$locale.png"
    expect_text "$readme" "$asset"
    test -s "$asset" || {
      echo "$readme references a missing or empty demo image: $asset" >&2
      exit 1
    }
  done
done

while IFS= read -r use; do
  echo "GitHub action is not pinned to a full commit: $use" >&2
  exit 1
done < <(
  sed -nE 's/^[[:space:]]*uses:[[:space:]]*([^[:space:]#]+).*/\1/p' .github/workflows/*.yml |
    grep -v '^\./' |
    grep -Ev '@[0-9a-f]{40}$' || true
)
