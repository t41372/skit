# The skit performance evaluation pipeline

The [design record](../docs/design/benchmarks.md) defines the measurement contract. This file is
the operating manual. The pipeline measures skit. It does not change product behavior. Generate
all performance claims from `results.json`; do not write claims by hand.

## Quick start

```bash
cargo build --locked --release -p skit-cli-rs -p skit-benchmarks
bash benchmarks/test.sh
bash benchmarks/run.sh pr .bench target/release/skit
bash benchmarks/check.sh .bench/results.json benchmarks/budgets.toml --require-enforced
cat .bench/results.md
cargo bench --locked -p skit-benchmarks
```

The shell files are thin front doors. All parsing, validation, orchestration, statistics, and
reporting live in the typed `skit-benchmarks` crate. The pipeline does not contain a Python
implementation or Python tooling. Python files are permitted only as measured script subjects.

The macro suites need a POSIX host, Python, uv, and Hyperfine. CI installs Hyperfine 1.20.0 from
the checksum-pinned repository action. The full profile also uses Node and `strace`. An optional
tool that is absent causes a recorded skip. It never causes a silent missing metric. Linux x86_64
on `ubuntu-24.04` is the reference platform.

## Front door

Use the binary directly when a shell wrapper is not suitable:

```bash
target/release/skit-bench datasets --n 1000 --out .bench/datasets/n1000
target/release/skit-bench run --profile pr --out .bench \
  --repo . --skit-binary target/release/skit
target/release/skit-bench summarize .bench
target/release/skit-bench check .bench/results.json --require-enforced
target/release/skit-bench check .bench/results.json --propose
target/release/skit-bench compare base.json head.json
```

The `pr`, `full`, and `compare` profiles are stable machine values. Usage errors exit with code 2.
Operational failures and failed enforced budgets exit with code 1.

## Implementation map

| Piece | Responsibility |
| --- | --- |
| `crates/skit-benchmarks/src/lib.rs` | Result schema, suite plans, strict merge, and derivation |
| `budget.rs` and `benchmarks/budgets.toml` | Two-tier budget contract and ratchet proposals |
| `compare.rs` | Warn-only A/B delta report |
| `dataset.rs` | Deterministic libraries made through public product APIs |
| `environment.rs` | Constructed child environment and host manifest |
| `hyperfine.rs` | Hyperfine argv and full-sample export parsing |
| `parsers.rs` | RSS and `strace` output parsing |
| `process.rs` | Bounded process-tree execution, timeout, kill, and reap |
| `report.rs` | Completed-run validation and atomic JSON and Markdown reports |
| `sources.rs` | Seeded analyzer benchmark subjects |
| `stats.rs` | Median, nearest-rank p95, and sample standard deviation |
| `suites/` | Macro and micro suite implementations |
| `tui_probe.rs` | Fresh-process Ratatui interaction probe |
| `benches/core.rs` | Criterion and CodSpeed-compatible benchmark subjects |

The executable Rust source stays under the workspace coverage, Clippy, Rustdoc, test, and mutation
gates. Suite orchestration is not exempt from those gates.

## Measurement definitions

- **Process-cold, filesystem-warm:** each macro sample starts a new process. Warmup runs warm the
  page cache and the per-run uv cache. Cold disk and first-download journeys are separate future
  measurements.
- **Median and p95:** headline values use the median. p95 uses nearest rank. `results.json` retains
  every raw Hyperfine sample and every independent Rust probe sample.
- **Import compatibility metrics:** a native Rust binary has no Python module import graph. The
  pipeline retains every latest-main `imports.*` metric ID with exact zero values. The raw payload
  records the architecture reason. This preserves the historical namespace and makes a regression
  to a Python implementation measurable.
- **Cold analysis and warm analysis:** `micro.analyze_cold.*` starts a new harness process for each
  sample. `micro.analyze.*` calls the parser-backed product API repeatedly in one process. Do not
  combine the two measurements.
- **Broken analysis:** `micro.analyze_broken.*` measures the same line count with an incomplete last
  line. Read it beside its valid twin. Faster error recovery is not a parser speedup.
- **TUI spans:** a fresh process scans the real generated store, creates real `LibraryState`, and
  renders with Ratatui `TestBackend` at 120 by 40. It measures first idle, one real selection move,
  one real search update, and Linux `VmHWM`. Assertions prove that selection moved and search both
  removed and retained rows. These values are headless interaction proxies, not terminal paint.
- **Run overhead:** Python compares direct Python, the exact `uv run --no-project --script` lane,
  and `skit run --no-input`. Shell compares direct Bash with skit. Full also compares Node with
  skit. The skit lane includes post-run state persistence because users pay that cost.
- **Scale per entry:** `scale.list_json.per_entry_us` is the median millisecond difference between
  N=1000 and N=0. Division by 1000 entries and conversion to microseconds cancel.
- **Library footprint:** `footprint.library_bytes` measures user entries.
  `footprint.library_state_bytes` measures remembered form and run state.
  `footprint.library_bytes_per_entry` divides their total by N. Absolute source paths make these
  values host-dependent, so they are for same-host A/B use and not budgets.
- **Pipeline cost:** every suite records `duration_seconds`; merge publishes
  `pipeline.<suite>.duration_s` and `pipeline.duration_s`.

## Profiles

| Suite | `pr` | `full` | `compare` |
| --- | --- | --- | --- |
| startup | 3 warmups + 15 runs | 5 + 40 | 3 + 15 |
| scale | N in {0, 100, 1000} | {0, 10, 100, 1000} and doctor | {0, 100, 1000} |
| run overhead | Python and shell | Python, shell, and JavaScript | Python and shell |
| RSS | 5 samples | 10 | 5 |
| imports | N in {0, 100} | same | same |
| TUI | 5 samples at N in {0, 100, 1000} | 10 at the same sizes | 5 at the same sizes |
| micro | fast adaptive sampling | rigorous adaptive sampling | fast adaptive sampling |
| syscalls | absent | `list --json` at N=1000 | absent |
| footprint | wheel, sdist, binary, repository, and libraries | plus installed closure | absent |

N=100 represents a typical library. N=1000 is the stress point. The full profile adds N=10 only
where it helps show scale shape.

Criterion and CodSpeed use the same source generator for Python, shell, JavaScript, and TypeScript
at 20, 200, and 2000 lines. It consumes a CPython-compatible random stream. Golden SHA-256 tests pin
the exact latest-main bytes for all 12 default subjects. Store subjects use two public-API N=200
libraries: the mixed kind contract below and a 100% reference-mode command library that exposes the
index worst case. Both shapes measure full entry reads, summary reads, and resolution. The crate also
keeps the added Ratatui render and reducer-filter subjects.

## Datasets

The generator uses `LibraryService<FileStore>` and `FormStateService`. It does not write a private
store format. The default inputs are seed `20260720`, state fraction `0.5`, and generator version
1. A manifest permits reuse only when every input and the writing skit version match. A corrupt or
incomplete manifest fails with a delete-and-rerun remedy. The pipeline only removes a directory at
the exact expected generated path, and it refuses symlinks and non-directories.

One CPython-compatible `Random` stream drives the kind shuffle, names, descriptions, and state
selection in the exact latest-main order. Golden contracts pin the kind grid, representative
entries, selected state, and analyzer source bytes. Do not split or reseed that stream.

The deterministic 100-slot kind mix is 30 Python, 20 shell, 10 JavaScript, 5 TypeScript, 10 command,
10 prompt, 5 fish, 6 executable, and one each of Ruby, Perl, Lua, and R. Names and descriptions
include ASCII, CJK, and emoji. Entries include realistic parameter declarations. Half receive
synthetic remembered values and run times. Every tenth reference entry has its target removed.

The search probe uses `o`. Latest Python main guarantees that entry zero's name and description do
not contain it. Search also includes slug and kind in the Rust product. The generator retains the
latest-main sequence instead of moving a kind slot to manufacture a new invariant. The live TUI
probe uses the real input path and rejects each planned non-empty dataset if filtering removes no
rows or every row.

The kind mix, state fraction, missing-target rate, probe character, random-call order, fixed runner
label, and major UI dependency versions are load-bearing inputs. A generated-content change must
increment `GENERATOR_VERSION` and declare a benchmark-history discontinuity.

## Constructed environment

Measured processes do not inherit the developer environment. Each child receives a complete map:

- absolute `SKIT_DATA_DIR`, `SKIT_STATE_DIR`, and `SKIT_CONFIG_DIR` for one dataset;
- per-run `HOME`, `XDG_*`, and `UV_CACHE_DIR` paths in external scratch storage;
- a de-duplicated `PATH` made from the skit, uv, and Node directories plus `/usr/bin:/bin`;
- `SKIT_LANG=en`, `PYTHONUTF8=1`, `LC_ALL=C.UTF-8`, `TERM=dumb`, `COLUMNS=100`, and `LINES=40`;
- resolved toolchain `CARGO` and `RUSTC` binaries only for wheel builds.

The runner resolves rustup proxies to the selected toolchain before it changes `HOME`. It does not
inherit `PYTHONPATH`, virtual-environment state, color flags, registry mirrors, or arbitrary `UV_*`
variables. Every child has a timeout. A timed-out process tree is killed and reaped through an OS
process group or job object. A script cannot leave a descendant that holds a capture pipe open.

## Result contract

`results.json` is schema version 1. The Rust types are the only schema source. It contains a host
and Git manifest, a flat map of stable dotted metrics, explicit skips, and raw suite payloads:

```jsonc
{
  "schema_version": 1,
  "meta": {
    "generated_at": "2026-08-08T12:00:00Z",
    "profile": "pr",
    "git": {"commit": "...", "dirty": false, "pr": "29"},
    "skit_version": "0.5.0",
    "host": {
      "os": "Linux", "kernel": "...", "cpu": "...", "cpu_count": 8,
      "mem_total_mib": 16384, "platform_key": "linux-x86_64",
      "ci_runner": "ubuntu-24.04", "ci_image_version": "..."
    },
    "python": "3.13.x", "uv": "0.12.x",
    "textual": "not-applicable", "pyperf": "rust-harness-v1"
  },
  "metrics": {
    "startup.version.median_ms": {
      "value": 18.7, "unit": "ms", "n": 15, "p95": 20.1, "stddev": 0.9
    }
  },
  "skipped": [{"suite": "run_overhead", "case": "js", "reason": "node not found"}],
  "raw": {"startup": {"times_s": {}}}
}
```

`run.json` is the completion stamp. It is written only after every planned suite succeeds.
`summarize` requires an exact match between its suite set and the suite JSON files. It validates all
finite values and duplicate metric IDs, derives metrics, and writes `results.json` and `results.md`
atomically. A process crash is a run failure. A skip is a deliberate precondition decision with a
reason.

On pull requests, `meta.git.commit` is the exact ephemeral merge commit that ran. `meta.git.pr` is
the durable evidence anchor used in ratchet context. Other runs use the measured commit.

## Budgets

`budgets.toml` has two tiers. An `enforced` row fails the command. A `target` row reports an
aspirational bound and does not fail. Predicates can restrict a row by profile, platform, and CI.
Every enforced row carries provenance context.

The checker treats these conditions as different hard failures:

- an applicable enforced metric exceeds its bound;
- an applicable enforced metric is missing;
- an enforced predicate refers to missing or empty metadata;
- a CI ratchet's Python context does not match the measured Python;
- `--require-enforced` evaluates no applicable enforced row.

A genuine predicate mismatch is reported as not applicable. Every recorded skip contributes to
`pipeline.skipped_count`, so the reference CI profiles can require zero decay.

For a ratchet refresh, use a clean CI artifact:

```bash
bash benchmarks/check.sh ci-results.json benchmarks/budgets.toml --propose > proposed.toml
```

Proposal refuses local artifacts, dirty artifacts, and wider bounds. Use `--allow-regression` only
for a reviewed regression and explain it in the row note. Change the bound in the same pull request
as the metric change. The checker asks for a tighter bound when a value falls below 85% of its
ceiling.

## CI and comparisons

- `benchmark.yml` runs the PR profile on relevant pull requests and main pushes. It stays advisory
  because path-filtered required checks remain pending on skipped pull requests.
- `benchmark-nightly.yml` runs the full profile with Node and `strace` and keeps the complete
  artifact for 90 days.
- `benchmark-compare.yml` builds and measures two refs on one runner. It uploads both complete
  results and a warn-only delta report.
- CodSpeed runs the Criterion subjects for hardware-independent instruction trends. It does not
  replace the macro pipeline or its byte, RSS, import, skip, and syscall contracts.

The Rust harness links product crates for micro and TUI measurements. The compare workflow must
therefore build each side's own `skit-bench` binary. A fixed head harness cannot honestly call the
base ref's in-process Rust APIs. A schema-v1 Python ref uses its own checked-out benchmark harness
and isolated environment for the same reason. This keeps latest Python main available as a migration
baseline without adding Python tooling to the v0.5 tree. The invoking Rust ref only renders the
final report. Both side artifacts must use schema version 1 and the `compare` profile. The report
marks different harness versions as incomparable context instead of presenting a false clean A/B.

## Add a suite or metric

1. Write a failing contract test. Define stable dotted IDs in the form
   `<suite>.<case>[.<subcase>].<stat>`.
2. Put parsing or statistics in a covered typed module. Put process orchestration in `suites/`.
3. Add the suite to every applicable profile. Add a headline only when the summary needs it.
4. Decide whether each unavailable prerequisite is a recorded skip. A child error must remain an
   error.
5. Add or update budget rows. Test all applicable, missing, not-applicable, and unevaluable paths.
6. Update both this operating manual and the design record.

Do not add an untyped script, a second schema, a private dataset writer, or a required-metrics text
list. The plan, result types, skips, and budget rows already define the complete contract.
