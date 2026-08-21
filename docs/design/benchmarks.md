# Design: the Rust performance evaluation pipeline

Status: restored from latest Python `main` at `206f9ef946fc45835cb2479593794431f2620c32`
Contract: v0.5.0 is a behavior and metric superset of that benchmark product

## Decision

skit keeps the complete latest-main performance product after the Rust migration. The harness is a
Rust workspace crate, not a reduced startup script. It preserves the metric namespace, profile
shape, deterministic datasets, constructed environment, raw evidence, skip policy, budget
predicates, ratchet rules, comparison report, and CI artifact flow.

The migration changes the implementation where the product architecture requires it:

- Rust types replace Python dataclasses as the one result schema.
- Adaptive Rust sampling plus `statrs` replaces pyperf for in-process microbenchmarks.
- Ratatui `TestBackend` replaces Textual `run_test` for headless interaction spans.
- Native process execution replaces Python import execution. Historical import metrics remain and
  report exact zeros with an explicit architecture note.
- Each A/B side supplies its own linked Rust harness because in-process Rust APIs cannot be loaded
  from another checkout at runtime.
- A schema-v1 Python side supplies its own checked-out harness and environment. This retains latest
  Python main as a migration baseline without adding Python benchmark tooling to v0.5.

These changes preserve the questions that every metric answers. They do not add Python tooling or a
Python implementation.

## Goals

- Measure startup, scale, launcher overhead, RSS, TUI interaction, parser and store hot paths,
  install footprint, loaded-language architecture, and system calls.
- Keep process-cold and warm in-process measurements separate.
- Keep complete raw evidence and reproducibility metadata.
- Fail on crashes, schema drift, missing enforced metrics, rotten predicates, and silent skips.
- Generate data through public product APIs, so the benchmark exercises supported formats.
- Make benchmark execution safe for user files. Only exact benchmark output and generated dataset
  paths can be replaced.
- Keep hosted wall-clock data advisory while hard-gating deterministic and ratchet-safe metrics.

## Non-goals

- The harness is not a product runtime path and does not change launcher behavior.
- It does not measure first-ever uv downloads or cold disks.
- It does not create an N=10000 tier. Public per-entry persistence makes that tier expensive and no
  current product decision needs it.
- It does not publish hand-written performance claims or a Pages trend site.
- It does not treat hosted-runner wall time as a deterministic merge gate.
- It does not use a private store writer, Python orchestration, or an external schema file.

## Architecture

`skit-benchmarks` owns the typed benchmark product:

```text
src/
├── bin/skit-bench.rs     command front door and hidden fresh-process probes
├── lib.rs                profiles, schema, validation, merge, and derivation
├── budget.rs             strict budget evaluation and proposal
├── compare.rs            A/B deltas and provenance warnings
├── dataset.rs            public-API deterministic library generation
├── environment.rs        constructed environments and host metadata
├── hyperfine.rs          pinned-tool argv and export parser
├── parsers.rs            GNU/BSD time, VmHWM, and strace parsers
├── pipeline.rs           dataset reuse, suite execution, and completion stamp
├── process.rs            bounded child lifecycle
├── report.rs             strict summary and atomic artifacts
├── runner.rs             discovered tools and per-suite context
├── sources.rs            exact-line seeded analyzer inputs
├── stats.rs              descriptive statistics
├── tui_probe.rs          real reducer and TestBackend interaction probe
└── suites/
    ├── imports.rs
    ├── footprint.rs
    ├── rss.rs
    ├── startup.rs
    ├── scale.rs
    ├── run_overhead.rs
    ├── micro.rs
    ├── tui.rs
    └── syscalls.rs
```

The shell files in `benchmarks/` only locate the repository and invoke this binary. This keeps the
command convenient without putting policy in shell.

## Execution model

`run` performs these ordered operations:

1. Resolve all command paths to absolute paths.
2. remove stale result and suite JSON files from the exact output directory;
3. build the profile plan and generate or verify every dataset manifest;
4. create scratch storage outside the repository and output directory;
5. discover optional tools and resolve rustup proxies to real toolchain binaries;
6. collect host, product, and measured-checkout metadata with bounded probes;
7. execute each suite and atomically persist its validated output;
8. write `run.json` only after every planned suite succeeds;
9. require the exact planned suite set, merge all metrics, derive totals, evaluate budgets, and
   atomically publish `results.json` and `results.md`.

A rerun preserves verified datasets and replaces derived output. An incomplete prior dataset that
has no manifest can be removed only at the expected generated path. A symlink or another filesystem
object is refused. An existing manifest with different inputs is never overwritten; the error asks
the operator to remove it explicitly.

Every process gets an argv vector, a complete environment, a working directory, and a timeout.
Timeout handling kills and reaps the complete tree through an OS process group or job object.
Hyperfine uses `--shell=none`; the harness quotes command
strings only for Hyperfine's command parser and validates the exported case names and full samples.

## Environment contract

Measured children receive only the following classes of values:

- one generated dataset through absolute `SKIT_DATA_DIR`, `SKIT_STATE_DIR`, and
  `SKIT_CONFIG_DIR`;
- external scratch paths for `HOME`, all `XDG_*` locations, and `UV_CACHE_DIR`;
- an ordered, de-duplicated PATH for skit, uv, Node, and system binaries;
- pinned English, UTF-8, plain-terminal, and 100 by 40 process terminal inputs;
- resolved `CARGO` and `RUSTC` executables when Maturin builds the binary wheel.

The environment does not inherit Python paths, virtual environments, color settings, uv registry
settings, or arbitrary developer values. Tool discovery happens before construction. The runner
resolves rustup proxies while the real home and repository toolchain file are available, then gives
Maturin the selected binaries. This prevents the scratch `HOME` from changing the Rust toolchain.

## Profiles

The plan is executable data, and tests pin every field:

| Suite | PR | Full | Compare |
| --- | --- | --- | --- |
| imports | N=0,100 | same | same |
| footprint | N=0,1000 | plus clean installed closure | omitted |
| RSS | 5 samples at N=0,1000 | 10 samples | 5 samples |
| startup | 3 warmups, 15 runs | 5, 40 | 3, 15 |
| scale | N=0,100,1000 | N=0,10,100,1000 and doctor | PR grid |
| run overhead | Python and shell | plus JavaScript | Python and shell |
| micro | fast adaptive samples | rigorous adaptive samples | fast adaptive samples |
| TUI | 5 samples at N=0,100,1000 | 10 samples | 5 samples |
| syscalls | omitted | N=1000 | omitted |

Compare mode omits footprint because building a package describes its harness checkout, not only the
selected binary. Per-case product API incompatibilities can be recorded as explicit skips in compare
mode; normal profiles fail them.

## Suite contracts

### Startup

Hyperfine measures a Python `-c pass` compatibility baseline and fresh-process `skit --version`,
`--help`, `list`, and `list --json` on N=0. It also preserves the latest-main Python import startup
IDs as not-applicable architecture values where required. Merge derives version overhead above the
Python baseline. The baseline is compatibility context, not part of the native launch path.

### Scale

Hyperfine measures `list`, `list --json`, and `show <middle-slug> --json` at the profile grid. Full
adds `doctor --json`. Merge derives `scale.list_json.per_entry_us` from N=1000 and N=0. The show
selector comes from the generated manifest, not a guessed filename.

### Run overhead

A dedicated three-entry library is created through `LibraryService`: Python, shell, and JavaScript.
It uses exact no-op subjects and a working directory outside all uv projects. Python measures direct
Python, exact uv script execution, and skit. Shell and JavaScript compare their runtime with skit.
The result includes skit's real post-run state writes.

### RSS

Each sample is a new `/usr/bin/time` process. The parser accepts GNU `%M` and BSD maximum-resident
set output and normalizes values to KiB. Cases cover version and `list --json` at N=0 and N=1000.
Malformed output on a supported host fails instead of becoming zero.

### Imports

The native application imports zero Python modules. The suite executes the real native version and
list paths to prove that they work, then publishes the complete historical `imports.version.*` and
`imports.list_json.n{0,100}.*` namespace as exact zeros. This includes module count and the Typer,
Rich, Textual, and tree-sitter flags. Raw output explains that the native Rust binary has no Python
import graph. Keeping the IDs preserves budgets and detects an accidental Python runtime dependency.

### Footprint

uv builds one binary wheel and one source distribution from the measured repository. The suite
records wheel, sdist, release binary, and repository Python implementation-file counts. Allowed
Python subjects under the corpus, demo, and no-op fixture are excluded from the implementation
census.

The suite also measures generated library data, state, total, and per-entry bytes. Full creates a
clean venv, installs the wheel with bounded retries, walks site-packages and the installed executable,
reads wheel `RECORD` files, and reports closure size, skit installed bytes, distribution count, and
the ten largest distributions.

### Micro

The Rust harness uses `statrs` and deterministic adaptive sampling. Calibration selects iterations;
fast and rigorous profiles choose different sample counts. Every subject uses `black_box` and calls
the same public product path used by the application.

Subjects include full store list, summary list, first/middle/last resolution, remembered-state load,
valid and incomplete Python/shell/JavaScript/TypeScript analysis at 20, 200, and 2000 exact lines,
fresh-process cold analysis, launch-plan construction, and raw prompt rendering. The generated
sources consume the same CPython-compatible random stream as latest Python main. Golden SHA-256
tests pin every default source byte at each language and size. The generator preserves the requested
final newline and line count.

### TUI

Each independent sample starts `skit-bench probe tui`. The probe scans a real `FileStore`, builds
real serializable UI state, and uses Ratatui `TestBackend` at 120 by 40. It measures initial render,
selection update plus render, and search input plus filtered render. Assertions check row count,
selection movement, query state, removed rows, and surviving rows. `/proc/self/status` supplies
`VmHWM` on Linux. A missing platform facility is a pre-spawn recorded skip; bad data is a crash.

The old `tui.import` metric remains zero with a native-architecture explanation. Selection is absent
below two entries because no valid movement exists.

### Syscalls

Full on Linux uses `strace -f -c` around `skit list --json` at N=1000. The strict parser records the
total and groups open/stat/read file operations and socket/connect network operations. Missing Linux
or `strace` is a recorded skip. Invalid output fails.

### Criterion and CodSpeed

The workspace bench target preserves the latest-main analyzer grid for Python, shell, JavaScript,
and TypeScript at 20, 200, and 2000 lines. It uses the shared seeded source generator. Store read
subjects measure full entry list, summary list, and resolution over two N=200 libraries. The mixed
library uses the standard generator. The command-only library uses public `LibraryService::add`
calls and puts every entry in reference mode, which is the registry-row worst case. Reporting both
prevents either the typical shape or the worst shape from being presented as the whole result.

The existing Rust-only parser, reducer filter, and N=1000 Ratatui render subjects remain as additive
coverage. CodSpeed uses the Criterion compatibility layer, so local and hosted subjects do not drift.

## Dataset contract

`dataset::generate(root, n, seed, state_fraction)` creates full data, state, config, and source
trees. It uses `LibraryService<FileStore>` and `FormStateService<FileFormStateStore>`. It never
serializes internal store documents itself.

The 100-slot mix is:

| Kind | Slots |
| --- | ---: |
| Python | 30 |
| shell | 20 |
| JavaScript | 10 |
| TypeScript | 5 |
| command | 10 |
| prompt | 10 |
| fish | 5 |
| executable | 6 |
| Ruby, Perl, Lua, R | 1 each |

The generator uses deterministic ASCII, CJK, and emoji names and varied descriptions. Scripts carry
real parameter declarations. Half of entries receive remembered values and runs across a fixed
synthetic timeline. Every tenth reference entry loses its target after insertion. A final public
list must return exactly N entries. One CPython-compatible `Random` stream drives the kind shuffle,
names, descriptions, and state selection in the same order as latest Python main. Golden tests pin
the kind grid, representative entries, selected state, and every analyzer source digest.

Latest Python main reserves `o` as the probe and makes entry zero's name and description free of
that character. Version 0.5 also searches slug and kind. The Rust generator does not reorder the
kind grid to hide this product change. For each planned non-empty TUI dataset, the live probe applies
the real input path and rejects a filter that removes no rows or every row.

Any change to the mix, state fraction, missing rate, probe character, random-call order, or other
load-bearing generated content must increment the generator version. Reusing an older version is
forbidden. The pull request must state that historical data has a discontinuity.

## Results and merge rules

The schema uses ordered maps and stable JSON. A metric has a finite value, stable unit, positive
sample count, and optional finite p95 and standard deviation. A skip has suite, case, and reason.
Each suite output has one label, duration, unique metrics, skips, and raw payloads.

Merge rejects:

- a duplicate suite output;
- a duplicate metric ID across suites;
- a skip whose suite does not match its output;
- a non-finite or invalid sample value;
- a missing planned output during summary.

Merge adds suite duration, total duration, and skip count. It derives differences and per-entry
values only when all source metrics exist and have compatible units. A result artifact can be loaded
and validated independently before checking or comparing it.

## Budget contract

Each TOML row names one metric, maximum, tier, optional profile list, optional platform, optional
CI-only predicate, optional ratchet marker, provenance context, and note.

Enforced evaluation distinguishes these states:

1. passed;
2. violated;
3. metric missing;
4. not applicable;
5. predicate unevaluable;
6. Python context mismatch.

Only genuine predicate mismatches are not applicable. Missing metadata cannot make a row disappear.
`--require-enforced` also fails when no enforced row was evaluated. Target rows always remain
advisory.

Ratchet proposal reads a schema-validated result and produces a complete deterministic TOML file. It
requires a CI runner, a clean measured tree, and usable Git or pull-request identity. It stamps
Python, date, and durable evidence identity. It refuses to widen a bound unless
`--allow-regression` is explicit. It retains non-ratchet rows and all predicates and notes. A value
under 85% of its ceiling creates a tighten notice.

The day-one Rust contract hard-gates the wheel and binary size, zero tracked Python implementation
files, the inherited import-count ratchets, zero native import flags, and zero skipped cases on the
applicable reference profiles. A clean CI proposal can tighten the count ratchets to the native zero
baseline. Timing, RSS, closure, library, and syscall goals remain target rows until their hardware
variance supports enforcement.

## A/B contract

Comparison requires schema-compatible artifacts. It excludes `pipeline.*` metrics because harness
duration does not describe product performance. Exact count, byte, and boolean metrics show absolute
and percentage deltas. Timing metrics use an explicit 5% noise floor. The report lists metrics and
skips that exist on only one side and warns about profile, platform, CI image, Python major/minor,
Rust-harness version, and metric-unit differences.

For Python, a fixed harness could import one side at a time. A Rust executable cannot dynamically
replace its linked `skit-*` crates. The workflow therefore runs each ref's own schema-v1 harness and
`compare` plan. A Rust side builds its CLI and linked harness. A Python side creates its isolated
environment and runs its checked-out harness. The root checkout only validates both schema-v1
artifacts and renders the report. A cross-generation report warns about the harness provenance
difference. Refs before schema version 1 remain below the compatibility floor; inventing mixed
linked code would produce misleading numbers.

## CI policy

The PR workflow is path-filtered and advisory. GitHub can leave a path-skipped required workflow in
pending state, so it must not be a required check. It builds the release CLI and harness, runs
Criterion, runs the PR macro profile, checks enforced budgets, adds Markdown to the step summary,
and uploads all raw data.

Nightly runs the full profile with Node and `strace`. It is the enforcement point for full-only skip
rows and installed closure. The compare workflow uses the pull request event's fixed base and head
SHAs. This keeps untrusted benchmark code in the pull request cache scope instead of executing a
caller-selected ref in the default-branch scope.

CodSpeed remains complementary. Its simulation-mode instruction counts are stable across hardware,
while the self-contained macro pipeline records process behavior, bytes, memory, imports, skips, and
system calls. `budgets.toml` remains the repository contract. No workflow publishes to GitHub Pages;
the documentation site owns that deployment.

## Verification

Changes to this product run:

```bash
cargo test --locked -p skit-benchmarks --all-targets
cargo clippy --locked -p skit-benchmarks --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked -p skit-benchmarks --all-features --no-deps
cargo fmt --all --check
bash benchmarks/test.sh
cargo build --locked --release -p skit-cli-rs -p skit-benchmarks
bash benchmarks/run.sh pr .bench target/release/skit
bash benchmarks/check.sh .bench/results.json benchmarks/budgets.toml --require-enforced
cargo bench --locked -p skit-benchmarks
```

Workspace coverage, mutation, dependency, and workflow security gates also cover this crate. Do not
weaken those gates for orchestration code.
