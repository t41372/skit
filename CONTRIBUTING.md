# Contributing to skit

Thank you for contributing. skit is a Rust workspace. The released `skit` command and all product
logic are Rust code. Python files in the repository are analyzer inputs, demo inputs, or benchmark
subjects. They are not implementation code.

## Prerequisites

Install the pinned Rust toolchain from `rust-toolchain.toml`. Cargo selects it when you enter the
repository. Install these development tools when you need their gates:

```bash
cargo install cargo-llvm-cov --version 0.8.7 --locked
cargo install cargo-mutants --version 27.1.0 --locked
cargo install cargo-audit --version 0.22.2 --locked
cargo install cargo-deny --version 0.20.2 --locked
cargo install zizmor --version 1.29.0 --locked
```

The documentation site needs Node.js 26.7.0 and npm 12.0.2 or later. The macro benchmark needs
`hyperfine` and `jq`. Wheel builds use `uvx --from maturin==1.14.1`; the product does not need a
Python runtime.

## Start development

```bash
git clone https://github.com/t41372/skit
cd skit
cargo build --locked
cargo run -p skit-cli-rs -- --help
cargo test --locked --workspace --all-targets --all-features
```

## Architecture

The dependency direction is strict:

```text
skit-domain
    ↑
skit-application  ←  skit-language / skit-form
    ↑                         ↑
skit-store / skit-runtime   skit-ui
             ↑                ↑
             └──── skit-cli + skit-tui

future: skit-tauri → skit-application + skit-ui
```

`skit-application` defines use cases and ports. It does not depend on Clap, Ratatui, Crossterm,
Tauri, TOML, or a concrete filesystem. `skit-ui` is a serializable reducer with no terminal code.
The Ratatui adapter renders that reducer. A future Tauri adapter must use the same reducer and
application ports.

## TDD workflow

Use this order for each behavior change:

1. Add a contract test that fails for the expected reason.
2. Add the smallest implementation that passes the test.
3. Refactor while the test stays green.
4. Add refusal, corruption, and concurrency tests where the boundary can fail.

Use real temporary directories for filesystem contracts. Use fake process probes before the final
spawn boundary. UI tests must drive the frontend-neutral reducer and the Ratatui `TestBackend`.

## Required gates

Run the complete local gate before a pull request:

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
cargo test --locked --workspace --all-targets --all-features
cargo llvm-cov --locked --workspace --all-targets --all-features --lcov --output-path lcov.info
bash scripts/test_coverage.sh
bash scripts/check_coverage.sh lcov.info
cargo mutants --workspace --all-features --cargo-arg=--locked --jobs 2 --minimum-test-timeout 20
cargo deny --locked check
cargo audit --deny warnings
zizmor .github/workflows
```

The coverage script merges records from all test binaries. It rejects every uncovered executable
source line. It ignores only LLVM mappings for structural Rust lines, such as braces, attributes,
and function signatures. Mutation testing remains a separate hard gate and must report zero
survivors.

## Internationalization

Every user-visible string must have English, Simplified Chinese, and Traditional Chinese entries in
`crates/skit-i18n/src/lib.rs`. English source text, new comments, and new documentation must follow
ASD-STE100. Machine JSON keys and `skills/skit/SKILL.md` remain English-only stable contracts.

Run these focused gates after a copy change:

```bash
cargo test --locked -p skit-i18n -p skit-cli-rs
bash scripts/test_english.sh
bash scripts/check_english.sh
```

## Analyzer corpus

Files in `tests/corpus/` are byte-exact analyzer inputs. Do not change line endings, trailing
spaces, final newlines, or unusual Unicode unless the test contract requires that change. The
pre-commit hooks exclude this directory from automatic text fixes.

## Agent Skill

`skills/skit/SKILL.md` is the source of truth. The Rust binary embeds it at compile time and writes
it with `skit agent install`. Tests verify its command examples against the real CLI tree.

## Documentation

Edit public pages in `docs/content/docs/`. The landing page includes `README.md`. Verify the site:

```bash
cd docs
npm ci
npm run types:check
npm run build
```

The screenshot and video pipeline is `bash scripts/record_demo.sh`. It needs Docker. Regenerate the
assets when TUI copy or layout changes. VHS does not record a mouse pointer, so the generated demo
does not include a mouse pointer.

## Benchmarks

```bash
cargo build --locked --release -p skit-cli-rs
bash benchmarks/test.sh
bash benchmarks/run.sh pr .bench target/release/skit
bash benchmarks/check.sh .bench/results.json benchmarks/budgets.toml --require-enforced
cargo bench --locked --workspace --all-features
```

Do not write performance claims by hand. Generate them from `results.json`. An optimization pull
request must include base and head evidence from the compare workflow.

## Dependencies and releases

Use current supported dependencies. Add Rust dependencies through workspace entries, run
`cargo update`, and commit `Cargo.lock`. Pin third-party GitHub Actions to full commit hashes. Run
`zizmor` after any workflow change.

A release tag must match the workspace and PyPI metadata versions. The release workflow builds
platform wheels and a source archive with Maturin, then publishes through PyPI trusted publishing.

## License

Contributions are licensed under the MIT License.
