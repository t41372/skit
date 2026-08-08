# Performance pipeline

The pipeline measures the Rust application. It does not change runtime behavior.

Build the release binary, run the macro measurements, and check the two-tier budget:

```bash
cargo build --locked --release -p skit-cli-rs
bash benchmarks/test.sh
bash benchmarks/run.sh pr .bench target/release/skit
bash benchmarks/check.sh .bench/results.json benchmarks/budgets.toml --require-enforced
cargo bench --locked --workspace --all-features
```

`results.json` records the commit, date, host, profile, raw hyperfine samples, and stable
metric IDs. `results.md` is generated from that file. Do not write README performance claims by
hand.

`required-metrics.txt` contains each required measurement ID. The run script calculates
`pipeline.skipped_count` from this list. The check script calculates the count again and rejects a
different reported value.

The `enforced` budget tier fails the pipeline. It contains deterministic limits, such as binary
size and the Python implementation-file count. The `target` tier reports timing goals but does not
fail on shared hosted hardware.

PR jobs use the `pr` profile and remain advisory because the workflow has path filters. The nightly
job uses the `full` profile. The compare workflow builds two refs on one runner and calls
`bash benchmarks/compare.sh`; optimization pull requests must attach that artifact.

Criterion subjects are in `crates/skit-benchmarks`. CodSpeed runs the same subjects in simulation
mode. The subjects cover parser-backed analysis, a 1,000-entry reducer filter, and a 1,000-entry
Ratatui render.
