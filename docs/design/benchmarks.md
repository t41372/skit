# Rust benchmark contract

The performance pipeline measures the Rust product. It does not change runtime behavior.

## Surfaces

- `crates/skit-benchmarks/benches/core.rs` measures parser-backed analysis, reducer filtering, and
  Ratatui rendering with Criterion and CodSpeed compatibility.
- `benchmarks/run.sh` measures release-binary startup with Hyperfine.
- `benchmarks/check.sh` checks deterministic enforced budgets and advisory timing targets.
- `benchmarks/compare.sh` renders base and head deltas from two `results.json` files.

## Commands

```bash
cargo build --locked --release -p skit-cli-rs
bash benchmarks/run.sh pr .bench target/release/skit
bash benchmarks/check.sh .bench/results.json benchmarks/budgets.toml --require-enforced
cargo bench --locked --workspace --all-features
```

The `pr` and `compare` profiles use 15 measured Hyperfine runs after three warmups. The `full`
profile uses 40 runs after five warmups.

## Result contract

`results.json` records the schema, commit, UTC date, host, profile, dirty state, CI state, metrics,
and raw Hyperfine samples. `results.md` is generated from that file. Do not write performance claims
by hand.

Stable macro metrics are:

- `startup.version.mean_ms`;
- `startup.list_empty.mean_ms`;
- `binary.release_bytes`;
- `repository.python_implementation_files`;
- `pipeline.skipped_count`.

## Budgets

`benchmarks/budgets.toml` has two tiers:

- `enforced` fails CI. It covers the release binary size, zero Python implementation files, and
  zero skipped required measurements.
- `target` reports timing goals. It does not fail on a shared hosted runner.

Refresh an enforced value only from a CI artifact. Change the bound in the same pull request that
changes the metric.

## Workflows

- The pull-request workflow measures the `pr` profile and uploads artifacts. It remains advisory
  because path filters can skip it.
- The nightly workflow measures the `full` profile.
- The manual compare workflow builds two refs on one runner and uploads base, head, and delta data.
- CodSpeed runs the same Criterion subjects in simulation mode.

An optimization pull request must include compare-workflow evidence. Hand-run results are useful for
development, but they are not release evidence.
