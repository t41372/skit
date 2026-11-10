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

expect_text Cargo.toml 'tree-sitter = "0.27.2"'
setup_uv_pin='astral-sh/setup-uv@68559e99cc807c1eb7e9238fd43a15bba460eec2 # v10.3.3'
test "$(grep -rF "$setup_uv_pin" .github/workflows | wc -l)" -eq 7 || {
  echo 'every setup-uv use must share the pinned v10.3.3 action' >&2
  exit 1
}
if grep -rE 'astral-sh/setup-uv@' .github/workflows |
  grep -qv '68559e99cc807c1eb7e9238fd43a15bba460eec2'; then
  echo 'a workflow uses a stale or unpinned setup-uv action' >&2
  exit 1
fi
expect_text .github/workflows/ci.yml 'taiki-e/install-action@cfdb7ed4ccc7c7dce37759eae89dc5674b469b39 # v2.90.3'
expect_text .github/workflows/ci.yml 'workflow_dispatch:'
expect_text .github/workflows/codspeed.yml 'CodSpeedHQ/action@af75f77f17a054769abefaa42fe8dc2975469b5f # v5.6.3'
expect_text .github/workflows/codspeed.yml 'taiki-e/install-action@cfdb7ed4ccc7c7dce37759eae89dc5674b469b39 # v2.90.3'
expect_text .github/workflows/codspeed.yml 'tool: cargo-codspeed@6.0.0'
expect_text .github/workflows/codspeed.yml 'cargo codspeed build -m simulation --locked --workspace --all-features'
expect_text .github/workflows/codspeed.yml 'run: cargo codspeed run --workspace'
if grep -Eq '^[[:space:]]*run:[[:space:]]*cargo codspeed run[[:space:]]*$' \
  .github/workflows/codspeed.yml; then
  echo 'CodSpeed build and run must select the same workspace packages' >&2
  exit 1
fi
for benchmark_workflow in benchmark.yml benchmark-nightly.yml benchmark-compare.yml; do
  # No benchmark job installs a package with uv, so uv never makes its cache directory and the
  # setup-uv post step fails on the absent path.
  expect_text ".github/workflows/$benchmark_workflow" 'enable-cache: false'
  if grep -Eq 'enable-cache:[[:space:]]*true' ".github/workflows/$benchmark_workflow"; then
    echo "$benchmark_workflow must not ask setup-uv to cache a directory that stays absent" >&2
    exit 1
  fi
done
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
expect_text .github/workflows/release.yml 'pypa/gh-action-pypi-publish@7245906fb61fa33a13554f9d5fb856fe61758086 # v1.14.3'
expect_text .github/workflows/release.yml 'workflow_dispatch:'
expect_text .github/workflows/release.yml "if: github.event_name == 'push'"
expect_text .github/workflows/release.yml "if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')"
expect_text .github/workflows/release.yml 'name: Smoke-test the native Linux wheel'
# Each Linux wheel runs on its own platform before the upload, not only the x86_64 glibc wheel.
expect_text .github/workflows/release.yml 'runner: ubuntu-24.04-arm'
# The cross image cannot build ring for aarch64 glibc, so that wheel builds on an Arm runner.
expect_text .github/workflows/release.yml 'runner: ubuntu-22.04-arm'
expect_text .github/workflows/release.yml 'name: Smoke-test the musl wheel in Alpine'
expect_text .github/workflows/release.yml 'needs: [linux, linux-smoke, windows, macos, sdist]'
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
expect_text .github/workflows/ci.yml 'fish-actions/install-fish@4bb37a71daebb76c6fb7d83771cff31b8a78c28e # v1.2.0'
expect_text .github/workflows/ci.yml 'actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0'
expect_text .github/workflows/ci.yml 'node-version: "26.13.0"'
expect_text .github/workflows/ci.yml 'run: node --version'
expect_text .github/workflows/ci.yml 'echo "SKIT_REQUIRE_NODE_RUNTIME=1" >> "$GITHUB_ENV"'
expect_text .github/workflows/docs.yml 'run: npm install --global npm@12.2.2'
expect_text .github/workflows/ci.yml 'uv::private_tests::test_extract_uv_skips_dir_fsync_on_windows -- --exact --ignored'
expect_text crates/skit-runtime/src/uv.rs 'uv::private_tests::test_extract_uv_skips_dir_fsync_on_windows -- --exact --ignored'
expect_text .github/workflows/ci.yml "throw 'the native Windows uv gate ran zero tests'"
expect_text .github/workflows/ci.yml 'test_the_preamble_runs_on_every_supported_dialect -- --exact --ignored'
expect_text .github/workflows/ci.yml 'the complete POSIX shell gate ran zero tests'
expect_text .github/workflows/ci.yml 'the CPython 3.13 compile gate ran zero tests'
fish_action_line="$(
  grep -Fn 'uses: fish-actions/install-fish@4bb37a71daebb76c6fb7d83771cff31b8a78c28e # v1.2.0' \
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
  grep -Fn 'uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0' \
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
  grep -Fn 'uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0' \
    .github/workflows/docs.yml | cut -d: -f1
)"
docs_npm_line="$(grep -Fn 'run: npm install --global npm@12.2.2' \
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

test "$(grep -lF 'activate-environment: true' \
  .github/workflows/benchmark.yml \
  .github/workflows/benchmark-nightly.yml \
  .github/workflows/benchmark-compare.yml | wc -l)" -eq 3 || {
  echo 'every benchmark workflow must activate its pinned Python environment' >&2
  exit 1
}
test "$(grep -lF 'sys.version_info[:2] == (3, 13)' \
  .github/workflows/benchmark.yml \
  .github/workflows/benchmark-nightly.yml \
  .github/workflows/benchmark-compare.yml | wc -l)" -eq 3 || {
  echo 'every benchmark workflow must verify Python 3.13 before measurement' >&2
  exit 1
}
expect_text CONTRIBUTING.md 'Node.js 26.13.0 and npm 12.2.2 or later'
expect_text .github/workflows/ci.yml 'zizmor .github/workflows .github/actions/install-hyperfine/action.yml'
# A release download can fail for a moment (HTTP 504 on the 2026-11-08 nightly), so curl tries
# again. The digest check stays the gate on what gets installed.
hyperfine_action=.github/actions/install-hyperfine/action.yml
expect_text "$hyperfine_action" 'curl -sSfL --retry 5 --retry-all-errors --retry-delay 5 -o hyperfine.tgz'
expect_text "$hyperfine_action" '  hyperfine.tgz" | sha256sum -c -'

# The complete UI walk is expensive, so pull requests opt in with one label. Scheduled and manual
# runs always execute. A failed walk must keep its replay data even when GIF rendering also fails.
ui_walker_workflow=.github/workflows/ui-walker.yml
test -f "$ui_walker_workflow" || {
  echo 'the UI walker workflow is missing' >&2
  exit 1
}
expect_text "$ui_walker_workflow" 'types: [opened, synchronize, reopened, labeled]'
expect_text "$ui_walker_workflow" 'schedule:'
expect_text "$ui_walker_workflow" 'workflow_dispatch:'
expect_text "$ui_walker_workflow" '      record_success:'
expect_text "$ui_walker_workflow" '      review_corpus:'
expect_text "$ui_walker_workflow" "if: \${{ github.event_name == 'workflow_dispatch' && inputs.review_corpus }}"
expect_text "$ui_walker_workflow" 'cargo test --locked -p skit-cli-rs --lib cli::tui_walker_corpus_tests::generates_and_reinstalls_the_stable_review_corpus -- --exact --ignored --nocapture'
expect_text "$ui_walker_workflow" "grep -Fq 'test result: ok. 1 passed; 0 failed; 0 ignored;' target/ui-walker-artifacts/corpus.log"
expect_text "$ui_walker_workflow" 'name: ui-walker-review-corpus'
review_corpus_input="$(awk '
  $0 == "      review_corpus:" { capture = 1; print; next }
  capture && $0 !~ /^        / { exit }
  capture { print }
' "$ui_walker_workflow")"
for contract in '        required: false' '        type: boolean' '        default: false'; do
  if ! printf '%s\n' "$review_corpus_input" | grep -Fqx "$contract"; then
    echo "the review_corpus input must contain: $contract" >&2
    exit 1
  fi
done
expect_text "$ui_walker_workflow" "SKIT_WALKER_RECORD_SUCCESS: \${{ ((github.event_name == 'pull_request' && contains(github.event.pull_request.labels.*.name, 'ui-walker-requested')) || (github.event_name == 'workflow_dispatch' && inputs.record_success)) && '1' || '0' }}"
record_success_input="$(awk '
  $0 == "      record_success:" { capture = 1; print; next }
  capture && $0 !~ /^        / { exit }
  capture { print }
' "$ui_walker_workflow")"
for contract in '        required: false' '        type: boolean' '        default: false'; do
  if ! printf '%s\n' "$record_success_input" | grep -Fqx "$contract"; then
    echo "the record_success input must contain: $contract" >&2
    exit 1
  fi
done
if grep -Eq '^  push:' "$ui_walker_workflow"; then
  echo 'the UI walker must not run for an ordinary branch push' >&2
  exit 1
fi
ui_walker_gate="github.event_name != 'pull_request' || (github.event.action == 'labeled' && github.event.label.name == 'ui-walker-requested') || (github.event.action != 'labeled' && contains(github.event.pull_request.labels.*.name, 'ui-walker-requested'))"
test "$(grep -cF "$ui_walker_gate" "$ui_walker_workflow")" -eq 1 || {
  echo 'the UI walker job must reject unrelated label events but honor an existing opt-in label' >&2
  exit 1
}
expect_text "$ui_walker_workflow" 'runs-on: ubuntu-24.04'
expect_text "$ui_walker_workflow" 'permissions: {}'
expect_text "$ui_walker_workflow" '      contents: read'
if grep -Eq '^[[:space:]]+[A-Za-z-]+:[[:space:]]+write([[:space:]]|$)' "$ui_walker_workflow"; then
  echo 'the UI walker does not need a write permission' >&2
  exit 1
fi
expect_text "$ui_walker_workflow" 'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1'
expect_text "$ui_walker_workflow" 'actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2'
expect_text "$ui_walker_workflow" 'SKIT_WALKER_CASES: "2"'
expect_text "$ui_walker_workflow" 'SKIT_WALKER_STEPS: "100"'
expect_text "$ui_walker_workflow" 'SKIT_WALKER_PROFILES: "complete"'
expect_text "$ui_walker_workflow" 'timeout-minutes: 90'
expect_text "$ui_walker_workflow" 'https://github.com/asciinema/agg/releases/download/v1.9.0/agg-x86_64-unknown-linux-gnu'
expect_text "$ui_walker_workflow" 'f111e315cd71056b116302342553dd765b7297579ed511f111d0cedb442aeda6'
expect_text "$ui_walker_workflow" '"$RUNNER_TEMP/agg"'
if grep -Eq 'cargo install([[:space:]]|.*)agg|sudo .*agg|/usr/(local/)?bin/agg' \
  "$ui_walker_workflow"; then
  echo 'the UI walker must not install agg globally or through Cargo' >&2
  exit 1
fi
expect_text "$ui_walker_workflow" 'agg-smoke.cast'
expect_text "$ui_walker_workflow" 'agg-smoke.gif'
expect_text "$ui_walker_workflow" 'test -s "$RUNNER_TEMP/agg-smoke.gif"'
expect_text "$ui_walker_workflow" 'agg 1.9.0 ignores resize events'

ui_walker_step() {
  local name=$1
  awk -v marker="      - name: $name" '
    $0 == marker { capture = 1 }
    capture && $0 ~ /^      - name:/ && $0 != marker { exit }
    capture { print }
  ' "$ui_walker_workflow"
}

expect_ui_step_text() {
  local name=$1
  local text=$2
  local block
  block="$(ui_walker_step "$name")"
  if ! printf '%s\n' "$block" | grep -Fq -- "$text"; then
    echo "the '$name' step does not contain: $text" >&2
    return 1
  fi
}

walker_step='Run the complete UI model walk'
expect_ui_step_text "$walker_step" 'id: walker'
expect_ui_step_text "$walker_step" 'continue-on-error: true'
expect_ui_step_text "$walker_step" 'test "$SKIT_WALKER_CASES" -gt 0'
expect_ui_step_text "$walker_step" 'test "$SKIT_WALKER_STEPS" -gt 0'
expect_ui_step_text "$walker_step" 'test "$SKIT_WALKER_RECORD_SUCCESS" = 0'
expect_ui_step_text "$walker_step" 'test "$SKIT_WALKER_RECORD_SUCCESS" = 1'
expect_ui_step_text "$walker_step" 'cargo test --locked -p skit-cli-rs --lib cli::tui_real_random_walk::real_random_walk -- --exact --list'
expect_ui_step_text "$walker_step" "grep -cFx 'cli::tui_real_random_walk::real_random_walk: test'"
expect_ui_step_text "$walker_step" 'cargo test --locked -p skit-cli-rs --lib cli::tui_real_random_walk::real_random_walk -- --exact --nocapture'
expect_ui_step_text "$walker_step" "-path 'target/ui-walker-artifacts/success-*/success.cast'"
expect_ui_step_text "$walker_step" 'test "$success_count" -eq 1'
expect_ui_step_text "$walker_step" 'tee target/ui-walker-artifacts/walker.log'
expect_ui_step_text "$walker_step" 'timeout --signal=TERM --kill-after=5m 65m'

render_step='Render captured casts'
expect_ui_step_text "$render_step" 'id: render'
expect_ui_step_text "$render_step" "hashFiles('target/ui-walker-artifacts/failure-*/failure.cast') != ''"
expect_ui_step_text "$render_step" "hashFiles('target/ui-walker-artifacts/success-*/success.cast') != ''"
expect_ui_step_text "$render_step" 'continue-on-error: true'
expect_ui_step_text "$render_step" "-path 'target/ui-walker-artifacts/failure-*/failure.cast'"
expect_ui_step_text "$render_step" "-o -path 'target/ui-walker-artifacts/success-*/success.cast'"
expect_ui_step_text "$render_step" 'bundle="${cast%/*}"'
expect_ui_step_text "$render_step" 'stem="${stem%.cast}"'
expect_ui_step_text "$render_step" '"$bundle/$stem.gif"'
expect_ui_step_text "$render_step" '"$bundle/agg.log"'
expect_ui_step_text "$render_step" '-print0 > "$RUNNER_TEMP/ui-walker-casts"'
expect_ui_step_text "$render_step" 'done < "$RUNNER_TEMP/ui-walker-casts"'
render_block="$(ui_walker_step "$render_step")"
if printf '%s\n' "$render_block" | grep -Fq 'done < <('; then
  echo 'the renderer must not discard the find command exit status' >&2
  exit 1
fi
if printf '%s\n' "$render_block" | grep -Eq 'ui-walker-artifacts/(failure\.(cast|gif)|agg\.log)'; then
  echo 'the renderer must keep each output in its atomic failure bundle' >&2
  exit 1
fi

upload_step='Upload UI walker artifacts'
expect_ui_step_text "$upload_step" 'if: ${{ !cancelled() }}'
expect_ui_step_text "$upload_step" 'actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2'
expect_ui_step_text "$upload_step" 'path: target/ui-walker-artifacts/'
expect_ui_step_text "$upload_step" 'if-no-files-found: error'

fail_step='Fail after artifact capture'
expect_ui_step_text "$fail_step" "if: \${{ !cancelled() && (steps.walker.outcome == 'failure' || steps.render.outcome == 'failure') }}"
expect_ui_step_text "$fail_step" 'exit 1'
ui_render_line="$(grep -nF -- '- name: Render captured casts' "$ui_walker_workflow" | cut -d: -f1)"
ui_upload_line="$(grep -nF -- '- name: Upload UI walker artifacts' "$ui_walker_workflow" | cut -d: -f1)"
ui_fail_line="$(grep -nF -- '- name: Fail after artifact capture' "$ui_walker_workflow" | cut -d: -f1)"
test -n "$ui_render_line" && test -n "$ui_upload_line" && test -n "$ui_fail_line" &&
  test "$ui_render_line" -lt "$ui_upload_line" && test "$ui_upload_line" -lt "$ui_fail_line" || {
  echo 'the UI walker must render, upload, and then fail in that order' >&2
  exit 1
}

# Every workflow job needs a time bound. Without one a stuck job runs to the six-hour default, and
# the run is cancelled before the log flushes, so the failure teaches nothing about where it stuck.
while IFS= read -r unbounded; do
  echo "workflow job has no timeout-minutes: $unbounded" >&2
  exit 1
done < <(
  for workflow in .github/workflows/*.yml; do
    awk -v file="$workflow" '
      /^jobs:[[:space:]]*$/ { in_jobs = 1; next }
      /^[^[:space:]#]/ { if (in_jobs && job != "" && !bounded) print file ":" job; in_jobs = 0 }
      in_jobs && /^  [A-Za-z0-9_-]+:[[:space:]]*$/ {
        if (job != "" && !bounded) print file ":" job
        job = $1
        sub(/:$/, "", job)
        bounded = 0
      }
      in_jobs && /^    timeout-minutes:/ { bounded = 1 }
      END { if (in_jobs && job != "" && !bounded) print file ":" job }
    ' "$workflow"
  done
)

# Windows runners check out with autocrlf=true, which rewrites line endings. Every byte-exact
# contract — tests/corpus, the four CRLF fixtures in it, and the rendered budgets file — needs the
# committed bytes to survive the checkout, so the repository must turn that rewrite off.
test -f .gitattributes || {
  echo '.gitattributes must exist so a checkout keeps the committed bytes' >&2
  exit 1
}
expect_text .gitattributes '* -text'

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

# A repository action is not a third-party action. The `$/` self-repository form loads it from
# the commit that runs, and the `./` form loads it from the checked-out files.
while IFS= read -r use; do
  echo "GitHub action is not pinned to a full commit: $use" >&2
  exit 1
done < <(
  sed -nE 's/^[[:space:]]*uses:[[:space:]]*([^[:space:]#]+).*/\1/p' .github/workflows/*.yml |
    grep -Ev '^(\./|\$/)' |
    grep -Ev '@[0-9a-f]{40}$' || true
)
