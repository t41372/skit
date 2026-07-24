# Design: the performance evaluation pipeline

Status: v5, review-converged (five adversarial rounds) · Target: one PR, no behavior change · Baseline: `main` @ `366d6d4`

## Why

skit wants to claim "lightweight" eventually, and wants to *stop regressions* immediately.
Today there is no reproducible evidence for startup cost, library-scale behavior, memory,
or install footprint — and the CI has no performance job at all. An exploratory analysis
(GPT 5.6 Pro, 2026-07, against PyPI 0.2.0) found `skit --version` ≈ `import skit.cli`
≈ 219 ms and `list --json` growing ~505 ms from 0 → 1,000 entries; directionally credible,
but not reproducible, not tied to a commit, and not tracked.

This PR builds the **measurement pipeline only**. Optimizations (CLI fast path, summary
index, parser caching) are future PRs that will be *judged by* this pipeline. Because the
whole point is to measure those future PRs against today, this PR deliberately changes no
runtime behavior: **nothing under `src/skit/` is touched.**

## Non-goals (this PR)

- No `src/skit` changes: no `__main__` dispatcher, no `EntrySummary` index, no
  `KindDescriptor`, no parser cache. The pipeline must first record the "before".
- No README performance claims, no badges, no "lightweight" wording anywhere. Claims come
  in a later PR, generated from this pipeline's output, after budgets are ratified.
- No SaaS onboarding (CodSpeed etc.). The pipeline is self-contained: pyperf + hyperfine +
  artifacts + a gh-pages history branch. CodSpeed can be added later without redesign; the
  reasons and tradeoffs are documented in `benchmarks/README.md`.
- No fixed-hardware runner. Hosted-runner wall-clock is treated as *advisory* by design
  (see budget tiers); hard time gates wait for stable hardware.
- Linux x86_64 is the only reference platform in v1. Suites that need `/proc`, strace, or
  `resource` guard themselves and record a skip elsewhere — never crash, never silently
  vanish.
- **No N=10,000 tier and no bulk-write dataset mode.** Generating 10k entries through the
  public API is quadratic in registry rewrites (hours), and a second "bulk" writer with a
  fidelity-witness test is complexity that serves no current decision. The scale grid tops
  out at 1,000 (the analysis' own headline scale). If a future decision needs 10k, that
  follow-up adds bulk mode *with* its witness test.
- No uv-bootstrap download measurement. `footprint` reports `uvman.UV_VERSION` as a string;
  downloading the artifact nightly to weigh it buys a number nobody is waiting on.

## Shape

Everything lives in a top-level `benchmarks/` package (dev tooling — not shipped in the
wheel, exempt from the i18n gate which scans `src/skit` only). One front door:

```bash
uv run python -m benchmarks datasets --n 1000 --out .bench/datasets/n1000
uv run python -m benchmarks run --profile pr --out .bench/
uv run python -m benchmarks summarize .bench/            # → results.json + results.md
uv run python -m benchmarks check .bench/results.json    # budgets gate (enforced tier)
uv run python -m benchmarks check --propose .bench/results.json  # print refreshed TOML
uv run python -m benchmarks compare base.json head.json  # A/B delta table
uv run python -m benchmarks export-gha .bench/results.json       # history format
```

```
benchmarks/
├── README.md            # methodology: cold/warm defs, run counts, how to read numbers
├── __init__.py
├── __main__.py          # thin argparse front door (all logic lives in covered modules)
├── envinfo.py           # host manifest: OS, kernel, CPU, RAM, python, uv, git commit+dirty
├── datasets.py          # deterministic library generator (seeded), public store API only
├── results.py           # typed result model; (de)serialization + validation — THE schema
├── budgets.py           # budgets.toml loader + evaluator (tiers, predicates, propose)
├── compare.py           # A/B delta report with noise thresholds
├── hyperfine.py         # pure command-set builder + JSON parser + metric-ID minting
│                        #   (no subprocess here)
├── envspec.py           # the constructed-environment contract (built, never inherited)
│                        #   and the pyperf inherit list — covered gate code
├── parsers.py           # pure parse/derive layer: sys.modules census, importtime top-20,
│                        #   strace -c tables, rss unit normalization, VmHWM, pyperf JSON —
│                        #   everything that turns tool output into metric values
├── pipeline.py          # pure run-plan logic: profile → suite plan, duration capture,
│                        #   skip collection, summarize merge + derived metrics,
│                        #   results.md render, export-gha conversion
├── budgets.toml         # the performance contract (see Budgets)
├── fixtures/            # noop.py/.sh/.js (benchmark subjects) + sources.py (seeded
│                        #   per-language analyzer-input generators — covered code)
├── suites/              # orchestration: spawns subprocesses under the env contract
│   ├── _env.py          # binary discovery + spawn wrappers (envspec applies here)
│   ├── startup.py       # hyperfine: python -c pass, import skit, import skit.cli,
│   │                    #   skit --version / --help / list / list --json (N=0)
│   ├── scale.py         # hyperfine: list/list --json/show --json at N ∈ profile grid
│   ├── run_overhead.py  # hyperfine: python noop vs uv run --no-project --script vs skit run
│   ├── rss.py           # peak RSS via getrusage(RUSAGE_CHILDREN) harness
│   ├── imports.py       # sys.modules census + -X importtime artifact for fast paths
│   ├── tui.py           # driver spawning tui_probe.py subprocesses
│   ├── tui_probe.py     # in-process: import span, first-idle span, search span, VmHWM
│   ├── syscalls.py      # strace -c: file-op counts + network-syscall count (Linux)
│   ├── footprint.py     # wheel bytes; clean-venv closure size, dist count, top-10 dists
│   └── micro.py         # orchestrates the pyperf scripts below
└── micro/               # pyperf Runner scripts — self-contained: import only
    ├── bench_store.py   #   skit + pyperf + stdlib, never the benchmarks package
    ├── bench_analyzers.py
    ├── bench_launch.py
    └── bench_render.py
```

### The environment contract (`benchmarks/envspec.py`, applied by `suites/_env.py`)

Benchmarked processes never inherit the developer's or runner's ambient environment; the
suite runner **constructs** the env dict and passes it to every child (hyperfine, probes,
harnesses — their children inherit it in turn, so no `env`-prefix trick is needed and the
`python -c pass` baseline stays uniform with every other lane):

- `SKIT_DATA_DIR` / `SKIT_STATE_DIR` / `SKIT_CONFIG_DIR` → the generated dataset
  (`src/skit/paths.py` reads these live per call — same isolation contract as
  `tests/conftest.py`).
- `HOME`, `XDG_*` → per-run scratch (belt and braces, mirroring conftest).
- `PATH` → **constructed, not scrubbed away**: `dirname(.venv/bin/skit)` + `dirname(which
  uv)` + `/usr/bin:/bin` (+ node's dir for the JS lane). PATH is load-bearing: without uv
  on it, `skit run` auto-downloads a private uv mid-benchmark
  (`src/skit/langs/launch.py:49`) and `skit doctor` exits 1 — hyperfine aborts on non-zero
  exits. The runner asserts `which uv` (and node, where that lane runs) *before* the suite
  and records a skip otherwise.
- `UV_CACHE_DIR` → a per-session scratch cache, warmed explicitly by the suite's warmup
  phase (so "warm uv cache" is a defined state, not whatever the runner happened to have).
- `SKIT_LANG=en`, `PYTHONUTF8=1`, `LC_ALL=C.UTF-8`, `TERM=dumb`, `COLUMNS=100`, `LINES=40`.
- Dropped entirely: `PYTHONPATH`, `FORCE_COLOR`/`NO_COLOR`/`CLICOLOR*`, `UV_*` mirror/index
  vars, `VIRTUAL_ENV`.

Two mechanisms need special care and are pinned here:

- **pyperf purifies worker environments** (verified against pyperf 2.10: workers receive
  only PATH/HOME/locale/PYTHONPATH unless told otherwise). The micro orchestrator passes
  `--inherit-environ SKIT_DATA_DIR,SKIT_STATE_DIR,SKIT_CONFIG_DIR,SKIT_LANG,PYTHONUTF8,LC_ALL`
  (plus any per-script fixture vars) to every script, and every micro script **asserts at startup that `SKIT_DATA_DIR` is set
  and non-default** — a missing dataset must die loudly, not benchmark an empty (or worse,
  the developer's real) library.
- pyperf re-execs the script file itself, so micro scripts are self-contained plain files
  (skit + pyperf + stdlib imports only; dataset root and fixture params arrive via env; no
  `benchmarks` package import, no cwd assumption).

Benchmarked binaries are `.venv/bin/skit` and `.venv/bin/python` directly — never
`uv run`, whose own overhead would pollute every number. The harness cwd for every lane is
a scratch directory **outside any uv project** (`uv run --script` behaves differently
inside one; see run_overhead).

### Output conventions

- Statistical outputs always record median, p95, stddev, n_runs; raw samples ship in the
  JSON artifact. Headline numbers are medians with p95 beside them.
- **No silent skips.** Anything a suite cannot run (no node, not Linux, no strace) is
  recorded in `results.skipped` with a reason and surfaced in the rendered summary; the
  count is itself a metric (`pipeline.skipped_count`) so the reference platform can budget
  it to zero (see Budgets).
- The pipeline measures itself: `pipeline.duration_s` (whole run) and per-suite durations
  are metrics, so bench-job creep is visible in the same history as everything else.

## Suites × profiles

| Suite        | pr profile                          | full profile (nightly)                  |
| ------------ | ----------------------------------- | --------------------------------------- |
| startup      | 3 warmup + 15 runs                  | 5 warmup + 40 runs                      |
| scale        | N ∈ {0, 100, 1000}                  | N ∈ {0, 10, 100, 1000} + `doctor --json`|
| run_overhead | python + shell lanes                | + JS lane (node preinstalled on runner) |
| rss          | 5 samples/case                      | 10 samples/case                         |
| imports      | full census (deterministic)         | same                                    |
| tui          | 5 probes × N ∈ {0, 100, 1000}       | 10 probes × same grid                   |
| micro        | pyperf `--fast`                     | pyperf default rigor                    |
| syscalls     | —                                   | list --json @1000 (strace apt-installed)|
| footprint    | wheel+sdist bytes only              | + clean-venv closure, dist count, top-10|

Dataset generation cost is paid once per profile run (N=1000 through the public API ≈
1–2 min; acceptable inside the job's 20-minute budget). N=100 is the *typical-library*
scale a launcher's real users feel; 1000 is the stress point the budgets quote — the
rendered summary shows both, always.

**startup** — hyperfine (`--shell=none`): `python -c pass` (interpreter baseline),
`python -c "import skit"`, `python -c "import skit.cli"`, `skit --version`, `skit --help`,
`skit list`, `skit list --json` on an empty library. Derived:
`startup.version.over_python_ms` (median − interpreter baseline median).

**scale** — same harness against pre-generated datasets: `skit list --json`, `skit list`,
`skit show <mid-slug> --json`; nightly adds `skit doctor --json`. Derived:
`scale.list_json.per_entry_us` = (median_ms(N=1000) − median_ms(N=0)) / 1000 × 1000 —
i.e. the ms delta *is* the per-entry µs figure; the unit conversion is stated so nobody
"fixes" it into a 1000× lie. `list --json` loads per-entry state (`argstate.load_state`
per row), so this metric depends on `state_fraction` and the missing-target fraction
exactly as it depends on the kind mix — all three are pinned in the generator-version
discontinuity clause (see Datasets).

**run_overhead** — three lanes, kept clean of download/install noise. Lane C uses a
**dedicated minimal library** (an otherwise-empty dataset into which the suite adds
exactly `noop-py`/`noop-sh`/`noop-js` via the public store API), so resolve cost doesn't
ride on the scale grid's N:
- A `python noop.py`
- B `uv run --no-project --script noop.py` — **the exact argv skit itself builds** for a
  python entry (`src/skit/langs/launch.py:91`; without `--no-project` uv attaches the
  script to any enclosing project, a different code path — also why cwd is pinned outside
  any uv project). `noop.py` carries a PEP 723 block with no dependencies: that is the
  product's canonical python entry, and it pins what B measures.
- C `skit run noop-py --no-input`; core overhead = C − B.

Shell lane: `bash noop.sh` vs `skit run noop-sh --no-input`. JS lane (nightly; skip
recorded if node is absent): `node noop.js` vs `skit run noop-js --no-input`. All lanes
are warmed by hyperfine's `--warmup` runs executing the real command (which also warms
the per-session `UV_CACHE_DIR`); there is no separate prepare step. Methodology note in
README: **C legitimately includes skit's post-run state persistence** (`save_last` +
`record_run`, two fsync'd atomic writes — a real per-run user cost; constant-size
rewrites, so iterations don't grow state), and warmup absorbs first-write file creation.
Cold dependency resolution and cold download journeys are out of scope for v1 and listed
as future suites.

**rss** — a stdlib harness: fork one child, wait, read
`getrusage(RUSAGE_CHILDREN).ru_maxrss`; fresh harness process per sample so maxima can't
bleed between samples; units normalized (Linux KiB / macOS bytes; Windows: suite skips —
no `resource` module). Targets: `skit --version`, `skit list --json` at N=0 and N=1000.
Report median and max.

**imports** — deterministic census, not timing: run the real CLI path
(`sys.argv=['skit','--version']; from skit.cli import app; app()` catching `SystemExit`)
and dump `sorted(sys.modules)`; record the count plus presence booleans for `typer`,
`rich`, `textual`, `tree_sitter*`. Same for `skit list --json` (N=0). Measured at design
time on main: `--version` = 298 modules on CPython 3.14 / 291 on 3.13 (the census is
python-version-dependent — which is exactly why the ratchet protocol pins the capture to
the CI python; these prose numbers are context, never the bound). typer+rich present,
textual+tree_sitter absent. Also captures `python -X importtime -c "import skit.cli"`
stderr and summarizes the top-20 cumulative offenders (3.12/3.13-compatible;
`-X importtime=2` is 3.14-only and not used).

**tui** — `tui_probe.py` spawned fresh per sample; inside one probe: span 1 =
`import skit.tui` (cold import share), span 2 = `MenuApp()` (no-arg constructor) →
`run_test(size=(120, 40))` → first `pilot.pause()` returns (mount + initial `_reload` +
message queue drained), span 3 = search: the table owns focus after mount and plain
letters are action keys (`a` opens Add), so the probe presses `/` (focus search) and
settles *first*, then measures `press(<probe char>)` → `pause()` (the char is
`datasets.SEARCH_PROBE_CHAR` — one definition, shared by probe and generator), and
**asserts the filter actually ran**: the search input's value is the probe char, and —
at N > 0, since an empty library has no rows to change — the visible row count strictly
drops (the generator guarantees a probe-char-free entry, so a full match can't mask a
dead filter) while at N ≥ 3 some rows must also SURVIVE (the generator guarantees a
matching long description), so the span measures a real filtered repaint, never the
degenerate filter-to-zero path. A silent no-op must die, not get recorded as a fast
search. Peak RSS needs `/proc/self/status`: its absence is a PRE-SPAWN skip recorded
once per run; a parse failure on a host where the file exists crashes. Documented as
*proxies* (headless Textual, not terminal paint) — stable, comparable, honest about
what they are.

**syscalls** (Linux, nightly; workflow apt-installs strace — it is not preinstalled on
runner images) — `strace -f -c` around `skit list --json` N=1000: counts of `openat`/
`stat`-family/`read` (the direct evidence for the future summary-index PR), plus a count
of network-family syscalls (`socket`, `connect`), expected 0 on warm read-only paths.

**footprint** — `uv build` → wheel and sdist bytes (current main wheel: ~451 KiB).
Nightly adds: clean `uv venv` + `uv pip install <wheel>` (network allowed; sizes need no
timing hermeticity): site-packages delta = dependency closure, `site-packages/skit*` =
skit itself, distribution count via `importlib.metadata`, top-10 largest distributions
(this shows exactly what the tree-sitter grammars cost — the input the "optionalize
parsers?" decision has been waiting for). Reports `uvman.UV_VERSION` as a string.

**micro** — pyperf scripts: `store.list_entries()` at N ∈ {0, 100, 1000};
`store.resolve()` first/mid/last; `argstate.load_state()`; warm per-language `analyze()`
(python, shell, js, ts) × source sizes {20, 200, 2000} lines from seeded fixture
generators; launch command assembly; prompt render. Cold first-import+parse per language
is measured as one-shot subprocess samples (×5), reported separately from warm loops —
never averaged together.

## Datasets

`datasets.py generate(root, n, seed=20260720, state_fraction=0.5)` builds a full skit home
(data/state/config trees) that suites point the `SKIT_*_DIR` variables at. **Public store
API only** (`store.add_python/add_script/add_prompt/add_command/add_exe`) — format
fidelity by construction; the per-add registry rewrite is O(N²) but fine at N ≤ 1000.

- Kind mix (sums to 100%): 30% python, 20% shell, 10% js, 5% ts, 10% command, 10% prompt,
  5% fish, 6% exe (reference-mode), 4% long tail rotating ruby → perl → lua → r. Kinds
  differ in meta/analyzer cost, so the mix is part of every `per_entry_us` definition
  (see the discontinuity clause below).
- Names: short/long ASCII with CJK and emoji sprinkled deterministically; empty, short,
  and long descriptions; scripts carry realistic param blocks.
- `state_fraction` of entries get last-run state (`argstate.save_last` +
  `argstate.record_run(at=…)`) with synthetic timestamps spread over a fixed range
  (drives the TUI's activity sort). ~10% of reference entries point at deliberately
  deleted targets (drives `target_missing`).
- Determinism: single `random.Random(seed)`; same (n, seed, state_fraction, generator
  version) → same structure. Store-stamped `added_at` values are not byte-stable and
  don't need to be.
- **Discontinuity clause:** the kind mix, `state_fraction`, and the missing-target
  fraction are all load-bearing inputs to every scale/tui metric; changing ANY of them
  bumps the generator version and is a history discontinuity.
- Search-probe invariant (both sides): every dataset contains at least one entry
  whose searchable text (name + description) lacks `SEARCH_PROBE_CHAR`, and — for
  n ≥ 3 — at least one that contains it (see the tui suite's filter assertions).
- Post-generate self-check: `store.list_entries()` must count exactly n, else the
  generator fails loudly.

`fixtures/` provides the noop scripts for run_overhead (noop.py with an empty PEP 723
block, noop.sh, noop.js) and `fixtures/sources.py`, the seeded per-language source
generators for analyzer benches (committed generators, not committed blobs; covered and
tested for determinism and requested line counts).

## Results model

`results.py` defines the schema as typed dataclasses — the single source of truth
(deliberately **no** parallel `schema.json`; a second schema document would be a
divergence waiting to happen; `benchmarks/README.md` documents the shape for humans).
Stable-ordered JSON:

```jsonc
{
  "schema_version": 1,
  "meta": {
    "generated_at": "…", "profile": "pr",
    "git": {"commit": "…", "dirty": false}, "skit_version": "0.2.1.dev0",
    "host": {"os": "…", "kernel": "…", "cpu": "…", "cpu_count": 8, "mem_total_mib": 16384,
              "ci_runner": "ubuntu-24.04|null", "platform_key": "linux-x86_64"},
    "python": "3.13.x", "uv": "0.11.x", "textual": "8.2.8"
  },
  "metrics": {   // flat, dotted, stable IDs — budgets and history key on these
    "startup.version.median_ms": {"value": 218.7, "unit": "ms", "p95": 231.0,
                                    "stddev": 4.1, "n": 40},
    "scale.list_json.n1000.median_ms": {…},
    "scale.list_json.per_entry_us": {…},
    "imports.version.modules": {"value": 291, "unit": "count", "n": 1},
    "imports.version.has_typer": {"value": 1, "unit": "bool", "n": 1},
    "footprint.wheel_bytes": {…}, "rss.list_json.n1000.peak_kib": {…},
    "tui.first_idle.n1000.median_ms": {…},
    "pipeline.skipped_count": {"value": 0, "unit": "count", "n": 1},
    "pipeline.duration_s": {…}, …
  },
  "skipped": [{"suite": "run_overhead", "case": "js", "reason": "node not found"}],
  "raw": {"hyperfine": {…}, "pyperf": {…}, "importtime_top": […]}  // full samples
}
```

`summarize` merges per-suite JSONs, computes derived metrics, validates via the dataclass
model, and renders `results.md` (the step-summary/human artifact).

## Budgets: a two-tier contract

`budgets.toml` is the performance contract. Every row:

```toml
[[budget]]
metric = "imports.version.modules"
max = 320                      # 291 measured at merge on CI's 3.13 (+ ~10% headroom)
tier = "enforced"              # or "target"
profiles = ["pr"]              # optional predicate vs meta.profile (list; empty = all)
platform = "linux-x86_64"      # optional predicate vs meta.host.platform_key
ci_only = true                 # optional predicate vs meta.host.ci_runner != null
context = { python = "3.13", commit = "…", date = "2026-07-…" }  # provenance, enforced rows
note = "ratchet: fast path may not get importier before the fast-path PR lands"
```

- **`enforced`** — deterministic or ratchet-safe metrics only; `check` exits non-zero on
  violation. Day-1 set (all pass on current main by construction):
  - `footprint.wheel_bytes ≤ 1 MiB` (current ~451 KiB; catches accidental data shipping).
  - `imports.version.modules` and `imports.list_json.modules` ratchets (measured + ~10%).
  - `pipeline.skipped_count = 0` twice — once with `profiles = ["pr"]` and once with
    `profiles = ["full"]` (the skip-prone suites — JS lane, syscalls, closure — run only
    nightly, so a pr-only row would never budget them), both with
    `platform = "linux-x86_64"`, `ci_only = true` — the pipeline may not silently decay
    *on the reference platform*; local macOS/Windows runs see these rows reported
    "not applicable".

`check`'s failure semantics are pinned, because each is a distinct decay channel:
- applicable enforced row, metric **over bound** → violation, exit non-zero;
- applicable enforced row, metric **absent from results.json** (crashed suite, renamed
  ID, merge bug) → "metric missing", exit non-zero — a ratchet must not evaporate the
  day its metric is renamed;
- enforced row whose **predicate references an absent or empty-string `meta` field** →
  "predicate unevaluable", exit non-zero — predicates may not rot into permanent
  not-applicable. (JSON `null` for `ci_runner` is *evaluable* — it legitimately means
  "not CI", and is how local runs see `ci_only` rows as not applicable. The CI value's
  source is explicit: the workflows export `BENCH_CI_RUNNER=ubuntu-24.04`, which envinfo
  reads; no GitHub-provided variable carries the `runs-on` label.)
- row genuinely not applicable on this host → reported as such, never silently dropped.

`check` prints an evaluated/applicable/passed/failed tally, and CI invokes it as
`check --require-enforced`, which additionally exits non-zero when zero applicable
enforced rows were evaluated — machine-checked, not grepped from prose output; local
non-reference runs omit the flag and stay usable.

Separately, `run`'s failure policy: a suite subprocess that exits non-zero fails the
whole pipeline run (distinct from a *recorded skip*, which is a suite's own deliberate,
reasoned decision before spawning work).
- **`target`** — the aspirational contract, reported as a ✓/△ table on every run, never
  failing CI until a future PR moves a row to `enforced` (wall-clock rows only on fixed
  hardware): `--version` ≤ python + 75 ms; `list(1000) − list(0)` ≤ 250 ms; TUI first
  idle @1000 ≤ 800 ms; CLI peak RSS ≤ 64 MiB; TUI peak RSS ≤ 96 MiB; closure ≤ 64 MiB;
  `imports.version.has_typer/rich/textual/tree_sitter = 0` (the fast-path PR's acceptance
  criteria, visible from day one); `syscalls.list_json.network = 0`.

**Ratchet protocol** (the part that keeps enforced numbers honest):

- Every enforced row carries `context` (python, commit, date) — provenance for the bound.
- Bench workflows pin Python 3.13 (same as the quality/coverage jobs), and **day-1 (and
  every later) ratchet values are proposed from the PR's own CI bench artifact** — never
  from a maintainer's local run: the module census is python-version-dependent (298 on
  3.14 vs 291 on 3.13 for the same tree), so a locally-proposed bound can be wrong on
  day 1. `check`'s python-provenance rule is scoped to where it means "config bug": on
  CI (`meta.host.ci_runner != null`), an enforced ratchet row whose `context.python`
  disagrees with `meta.python` (major.minor comparison) **fails** — a bound gating a
  census it wasn't set on;
  on a local host (`ci_runner` null), the same mismatch reports the row "not applicable
  (bound pinned to python 3.13; this host runs 3.14.x)" — a 3.14 laptop must not be
  permanently red for not being the CI matrix.
  Bootstrap is explicit, not circular: the pipeline PR ships budgets.toml with
  deliberately generous provisional ratchet bounds whose `context.python` is
  hand-written `"3.13"` — knowable a priori because the workflow pins it, and what keeps
  the interim CI runs green — and its final commit tightens the values via
  `check --propose` on the artifact those runs produced.
- A PR that intentionally moves an enforced metric (dependency bump changing the module
  census, deliberate feature cost) **updates budgets.toml in the same PR** —
  `check --propose` prints the exact refreshed TOML from a results.json, so the update is
  mechanical, reviewed, and diffed rather than hand-typed.
- `check` also warns when a measured value sits below 85% of its enforced bound: "ceiling
  is stale — tighten it" (so the fast-path PR's win gets locked in, prompted rather than
  remembered).

The tier split is the honest answer to "hosted runners are noisy": hard-gate only what
cannot flake; *display* the rest loudly on every run so drift is seen immediately.
`check` output states each row's tier and applicability.

## CI

Three workflows, matching house rules (SHA-pinned actions, `permissions: {}` top-level +
minimal per-job grants, `persist-credentials: false`, concurrency groups, `PYTHONUTF8`,
zizmor-clean). Runner: `ubuntu-24.04` pinned (not `-latest`) for comparability — a future
label change is a history discontinuity and gets annotated. Python pinned 3.13.
`timeout-minutes` on every bench job (20 for the pr job, 45 for nightly and
compare); the job's own duration is a recorded metric.

**The bench job is advisory by policy: it is never configured as a required status check
while path-filtered** (GitHub leaves path-skipped required checks "Pending", which would
block every docs-only PR). Its `check` step still fails the job red on enforced-budget
violations — visible shame, no merge lock.

1. **`benchmark.yml`** — `pull_request` + `push` to main, path-filtered to `src/**`,
   `benchmarks/**`, `pyproject.toml`, `uv.lock`, and the benchmark workflows. Runs the
   **pr profile**: datasets {0, 100, 1000} → suites → summarize →
   `check --require-enforced` → results table into `$GITHUB_STEP_SUMMARY` → upload
   `results.json` + `results.md` + importtime artifact. No PR comments (no extra
   permissions, no noise).
2. **`benchmark-nightly.yml`** — schedule (off-hour minute, house style) +
   `workflow_dispatch`. Full profile. Installs strace via apt. Then the same tail as the
   pr job — `summarize` → `check --require-enforced` (this is where the
   `profiles = ["full"]` enforced rows get their enforcement point; without it they'd be
   evaluated by no CI run, ever) → uploads artifacts; converts headline metrics via
   `export-gha` and publishes history with `benchmark-action/github-action-benchmark`
   (SHA-pinned; v1.22.x current) to the `gh-pages` branch under `bench/` —
   `customSmallerIsBetter`, `auto-push`, `fail-on-alert: false` initially; **alert
   thresholds get tuned after ~14 nightly points exist** (tracked in
   benchmarks/README.md). `contents: write` granted to exactly this job. One-time setup
   (merge checklist in the PR): create the `gh-pages` branch (empty orphan commit — the
   action documents pre-creating it), enable Pages so the chart is actually served, and
   **dispatch `benchmark-nightly` immediately after merge** to confirm the
   `profiles = ["full"]` enforced rows evaluate green — schedule-only workflows first run
   post-merge, so their first evaluation must be deliberate, not whenever the cron gets
   around to it.
3. **`benchmark-compare.yml`** — `workflow_dispatch(base, head)`; inputs reach only
   checkout `ref:` parameters and the job name, never `run:` interpolation. A/B semantics pinned: **the harness is always
   the invoking ref's `benchmarks/`; base and head supply only built+installed skit
   venvs** (each synced from its own ref's `uv.lock` — dependency changes are part of the
   measured diff, and the log says so — except pyperf, which the workflow installs into
   both side venvs itself: it is harness infrastructure, and a pre-pipeline base ref has
   no `bench` group, which must not blank the micro lane). Startup/scale/run_overhead
   compare via the CLI surface only (valid against any skit); micro (which imports skit
   internals) runs best-effort per side — an import failure records a skip carrying the
   actual exception text (never a canned label that misattributes the cause) for that
   side's benchmark instead of failing the workflow. `compare` flags |Δ| > max(5%, a
   per-unit noise floor: 2 ms for macro ms-metrics, 1 µs for micro µs-metrics) as
   notable (raw sample lists ship in each side's results.json for any deeper
   statistics), and a profile/platform/python mismatch between the sides renders a
   loud not-directly-comparable warning — warn-only either way. Compatibility floor: sides must postdate the prompt-kind store API
   (`725f11d`) — the dataset generator uses it, so older refs fail dataset generation
   before any suite runs; the workflow says so where it would bite. Results carry each
   SIDE's git identity (`--measured-repo`), never the harness ref's. This is the
   evidence tool future optimization PRs cite.

hyperfine is installed from a version-pinned GitHub release tarball; upstream publishes no
checksum assets, so the workflow hardcodes a sha256 computed once at pin time and verifies
against it.

## Interaction with the repo's gates

The trust rule: **code whose output gates or persuades gets gated itself; only
orchestration that needs external binaries is exempt, each exemption commented.**

- **Coverage (100% floor):** `[tool.coverage.run] source` gains `benchmarks`; an `omit`
  list excludes exactly `benchmarks/suites/*`, `benchmarks/micro/*`,
  `benchmarks/__main__.py`, and `benchmarks/fixtures/noop.py` — and *only* those:
  suites/micro after the parse layer is pulled out are genuinely spawn-and-wait
  orchestration around external binaries, `__main__.py` is a thin argparse shim, and
  `noop.py` is a benchmark *subject* (a script the benchmarks run, not harness code) —
  each omission commented in pyproject. The covered set is everything that computes,
  decides, or persuades: `results.py`, `budgets.py`, `compare.py`, `datasets.py`,
  `hyperfine.py` (pure builder/parser by design: no subprocess calls in it),
  `parsers.py` (every function that turns tool output — census dumps, strace tables,
  importtime stderr, VmHWM lines, pyperf JSON — into metric values; enforced metrics
  like the import ratchets are computed here, so this code IS gate code),
  `envspec.py` (the constructed-environment contract and the pyperf inherit list —
  a wrong environment produces wrong-but-plausible numbers, so building it is a
  decision, not orchestration), `pipeline.py`
  (plan/merge/derive/render/export logic), `fixtures/sources.py` (the seeded
  per-language analyzer-input generators — pure compute whose output the micro numbers
  stand on), and `envinfo.py` (budget predicates key on its `platform_key`/`ci_runner`
  output). envinfo is pure functions over injected `uname`/env/`os.sysconf` values; its
  module-level default wiring to the real host is the one seam, marked
  `# pragma: no cover` — the same statement-level mechanism the repo already uses. A
  repo whose identity is "everything is gated" does not grow an ungated gate.
- **Tests:** `tests/test_benchmarks_tooling.py` — hermetic, no hyperfine/pyperf binaries:
  dataset generation (small N; kind mix; state fraction; missing targets; the
  search-probe invariant; determinism under fixed seed; post-generate count check),
  results model round-trip + validation failures, budget evaluation (tiers, predicates,
  propose, stale-ceiling warning, context-python mismatch, enforced violation → failure
  exit, **missing-metric → failure exit, unevaluable predicate → failure exit,
  `--require-enforced` with zero applicable rows → failure exit**), compare thresholds,
  hyperfine command construction + JSON parsing against a fixture, every `parsers.py`
  derivation against fixture tool output, `pipeline.py` plan/merge/derive/render logic,
  `fixtures/sources.py` determinism + requested line counts, envinfo derivations from
  injected values, export-gha shape.
  `[tool.pytest.ini_options]` gains `pythonpath = ["."]` so `import benchmarks` works
  (and keeps working inside mutmut's `mutants/` tree — pytest resolves `pythonpath`
  relative to the inifile, which mutmut copies into `mutants/`).
- **mutmut:** `benchmarks/` added to `also_copy` (tests import it); `source_paths` stays
  `src/skit/` — mutation effort stays pointed at the product.
- **ruff/ty:** fully typed and linted like the rest of the repo; per-file-ignores follow
  the `scripts/` precedent (S603 fixed-arg subprocess).
- **Dependency groups:** `[dependency-groups] bench = ["pyperf>=2.9,<3"]` and
  `[tool.uv] default-groups = ["dev", "bench"]` — every `uv sync` (dev, CI quality, CI
  test) gets pyperf, so `ty check` (strictest: unresolved imports are errors) passes
  everywhere without overrides, and no "works in bench job only" split exists. pyperf is
  small, pure-Python, and never a runtime dependency.
- **i18n:** no impact — the gate scans `src/skit`; bench output is dev-facing English by
  the same machine-facing carve-out as `--json`.
- **zizmor:** the existing audit job covers the new workflows automatically.

## Docs

- `benchmarks/README.md`: full methodology — cold/warm definitions (process-cold = fresh
  process; fs-warm = post-warmup; cold-import vs warm-parse split; warm-uv-cache
  definition), profile grid (the table above), dataset definition + seed + generator
  versioning, the metric-ID grammar (`<suite>.<case>[.<subcase>].<stat>`, unit
  suffixes on statistical stats only; the headline set is
  `pipeline.HEADLINE_METRICS` — in code, so it can't drift), budget tiers + ratchet protocol + `--propose` workflow,
  hosted-runner noise policy, exact lane argvs for run_overhead, how to run locally
  (including "non-Linux hosts see skips; the skip budget applies only to reference CI"),
  how to add a suite, the gh-pages one-time setup checklist, and what would move
  wall-clock budgets to `enforced` (fixed hardware + observed noise distribution).
- `AGENTS.md`: add the bench commands to Commands and a short "Performance pipeline"
  section: *pipeline PRs measure; optimization PRs must attach `benchmark-compare`
  evidence; README claims must be generated from `results.json`, never hand-written.*
- Root READMEs: untouched in this PR (by design; see Non-goals).

## Risks & mitigations

- **Hosted-runner noise** → tier system; medians+p95 with raw samples retained; nightly
  trend line rather than single-run judgment; wall-clock hard gates deferred.
- **Harness bitrot** (the classic fate of benchmark suites) → contract modules under the
  100% coverage floor; smoke tests in the normal PR test job, so a refactor that breaks
  the harness fails CI the same day.
- **Wrong-but-plausible numbers** (the worst failure class) → micro scripts assert their
  dataset env; generator self-checks its count; skips are counted and budgeted; lane
  argvs documented and derived from what skit actually execs.
- **Textual first-idle proxy drift across versions** → metric documented as a proxy;
  textual version recorded in the manifest; history annotates dependency bumps.
- **`uv pip install` network flake in footprint (nightly)** → retries; wheel-only metric
  stays on the PR path.
- **Third-party history action** → SHA-pinned, `contents: write` on one job only,
  publishes to gh-pages only.

## Implementation order (within the single PR)

1. `results.py` + `envinfo.py` + `budgets.py` + `budgets.toml` + tests (the contract).
2. `datasets.py` + fixtures + tests (the workloads).
3. `parsers.py` + `hyperfine.py` + tests (the pure measurement layer), then
   `suites/_env.py` and the suites: imports/footprint/rss → startup/scale/run_overhead
   (hyperfine) → micro (pyperf) → tui (probe) → syscalls.
4. `pipeline.py` (+ `__main__.py` thin front door): profiles, `summarize`/`check`/
   `compare`/`export-gha` + tests.
5. Workflows + AGENTS.md + benchmarks/README.md.
6. Full gate run; then a baseline captured **from the PR's own CI bench artifact**
   (ratchet protocol: the census is python-dependent, so bounds come from the pinned CI
   python, via `check --propose` on the downloaded results.json) written into
   budgets.toml, and the results attached to the PR description (not committed —
   results are artifacts, `.bench/` is gitignored).
