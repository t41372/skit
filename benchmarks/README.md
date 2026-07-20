# The skit performance evaluation pipeline

Design & decision record: [`docs/design/benchmarks.md`](../docs/design/benchmarks.md)
(review-converged; this README is the operating manual). The pipeline measures — it
changes nothing about skit itself. Optimization PRs are judged by these numbers;
README claims must be generated from `results.json`, never hand-written.

## Quick start

```bash
uv sync                                                  # bench group is default-installed
uv run python -m benchmarks run --profile pr --out .bench
uv run python -m benchmarks check .bench/results.json    # budget contract (see below)
cat .bench/results.md
```

Datasets under `.bench/datasets/` are reused across runs when every generation input
matches (including the writing skit's version). When switching between branches that
share a version string, delete `.bench/datasets` — the stamp can't see same-version
store-layout changes.

Requirements: a POSIX host (Windows is not supported — the venv layout and
`resource`-based harnesses assume POSIX); Linux x86_64 is the reference platform;
`hyperfine` on PATH for the macro suites (CI installs a pinned 1.20.0 — the pin's
single source of truth is `benchmarks/hyperfine.py`, and a sync test holds every
workflow to it), `strace` for the nightly syscalls suite. macOS and missing tools
produce **recorded skips**, never crashes — but the numbers that matter are the
reference platform's.

## Layout

| Piece | What it is |
| --- | --- |
| `results.py` | THE schema (typed dataclasses; deliberately no schema.json twin) |
| `budgets.py` + `budgets.toml` | the two-tier performance contract |
| `parsers.py` | everything that turns tool output into metric values |
| `envspec.py` | the constructed-environment contract (built, never inherited) |
| `pipeline.py` | profiles, merge, derived metrics, results.md, history export |
| `datasets.py` | deterministic library generator (public store API only) |
| `hyperfine.py` | hyperfine argv building + export parsing (no subprocess) |
| `envinfo.py` | host manifest; budget predicates key on its output |
| `compare.py` | A/B delta report (warn-only) |
| `suites/` | orchestration: spawns processes under the env contract |
| `micro/` | self-contained pyperf scripts (skit + pyperf + stdlib only) |
| `fixtures/` | noop benchmark subjects + seeded analyzer-source generators |

Trust split: everything that computes, decides, or persuades sits under the repo's
100% coverage floor; only `suites/*`, `micro/*`, `__main__.py`, and
`fixtures/noop.py` are exempt (spawn-and-wait orchestration and benchmark subjects —
the exact list, with reasons, is in `pyproject.toml`'s coverage `omit`).

## Definitions (what the numbers mean)

- **process-cold, filesystem-warm** — every macro sample is a fresh process, after
  warmup runs have warmed the page cache and (where uv is involved) `UV_CACHE_DIR`.
  Cold-filesystem / first-ever-install journeys are separate future suites, never
  mixed into these numbers.
- **cold import vs warm parse** — `micro.analyze_cold.*` is a one-shot subprocess
  (first import + first parse); `micro.analyze.*` is pyperf's warm in-process loop.
  Never averaged together.
- **median / p95** — headline values are medians; p95 is nearest-rank
  (`ceil(0.95·n)`); raw samples ship in `results.json` under `raw`.
- **TUI spans are proxies** — headless Textual (`run_test`, 120×40), not terminal
  paint: span 1 `import skit.tui`, span 2 App() → first `pilot.pause()` returns,
  span 3 focus search (`/`), settle, then measure `press(<probe char>)` → settle
  (the char is `datasets.SEARCH_PROBE_CHAR`). The probe asserts the row count matches
  the library, that the filter really dropped rows (the dataset guarantees a
  probe-char-free entry), and — at 3+ entries — that some rows survive (a matching
  entry is guaranteed too), so the span never degenerates to filter-to-zero.
- **run overhead** — lane A `python noop.py`; lane B `uv run --no-project --script
  noop.py` (the EXACT argv skit builds — `src/skit/langs/launch.py`); lane C
  `skit run noop-py --no-input`. Core overhead = C − B. C legitimately includes
  skit's post-run state persistence (two fsync'd, constant-size writes). Lanes run
  in a dedicated 3-entry library, cwd outside any uv project.
- **`scale.list_json.per_entry_us`** — (median_ms(N=1000) − median_ms(N=0)) is the
  total ms for 1000 entries; numerically that IS the per-entry µs figure (÷1000
  entries × 1000 µs/ms cancel). Stated so nobody "fixes" it into a 1000× lie.

## Datasets

`datasets.py generate(root, n, seed=20260720, state_fraction=0.5)` — public
`store.add_*` only (format fidelity by construction; the O(N²) registry rewrite is
fine at N ≤ 1000, and the 10k tier + bulk writer is an explicit non-goal). Kind mix
(sums to 100): 30 python / 20 shell / 10 js / 5 ts / 10 command / 10 prompt / 5 fish
/ 6 exe / 4 long-tail (ruby→perl→lua→r), shuffled deterministically; CJK/emoji names
sprinkled; half the entries carry last-run state over a fixed synthetic time range;
every 10th reference entry's target is deliberately deleted.

**Discontinuity clause:** the kind mix, `state_fraction`, and the missing-target
fraction are inputs to every scale/tui metric. Changing ANY of them bumps
`GENERATOR_VERSION` and is a history discontinuity (annotate the gh-pages chart).
The same applies to the pinned runner label (`ubuntu-24.04`) when it eventually
EOLs, and to major dependency bumps (textual above all — the TUI proxy rides on it;
the manifest records versions for exactly this reason).

## Profiles

| Suite | pr | full (nightly) | compare (A/B) |
| --- | --- | --- | --- |
| startup | 3 warmup + 15 runs | 5 + 40 | 3 + 15 |
| scale | N ∈ {0, 100, 1000} | {0, 10, 100, 1000} + doctor | {0, 100, 1000} |
| run_overhead | python + shell | + JS lane | python + shell |
| rss | 5 samples | 10 | 5 |
| imports | census (deterministic) | same | same |
| tui | 5 probes × {0, 100, 1000} | 10 × same | 5 × same |
| micro | pyperf `--fast` | full rigor | `--fast` |
| syscalls | — | list --json @1000 | — |
| footprint | wheel+sdist only | + closure, dist sizes | — (would measure the harness ref) |

N=100 is the *typical-library* scale; 1000 is the stress point the budgets quote.
The rendered summary always shows both.

## Budgets: the two-tier contract

`budgets.toml` rows are `enforced` (deterministic/ratchet-safe only — `check` exits
non-zero) or `target` (the aspirational contract — reported loudly, never failing CI
until a future PR deliberately promotes a row; wall-clock rows only ever on fixed
hardware). Optional predicates (`profile`, `platform`, `ci_only`) scope rows;
non-matching hosts see "not applicable", and every distinct decay channel is its own
hard failure: violation, **metric missing** (renamed ID / crashed suite),
**predicate unevaluable** (absent/empty meta field), **python mismatch on CI**
(a ratchet bound gating a census it wasn't set on — the module census differs across
python versions). CI runs `check --require-enforced`, which also fails when zero
applicable enforced rows were evaluated.

**Ratchet protocol:** ratchet rows (`ratchet = true`) bound a measured value +
headroom. Refresh them ONLY from a CI artifact — `uv run python -m benchmarks check
<ci-results.json> --propose` prints the exact replacement file. A PR that
intentionally moves an enforced metric updates budgets.toml in the same PR. When a
measured value sits under 85% of its ceiling, `check` nags to tighten. Provisional
bootstrap bounds carry hand-written `context.python = "3.13"` (the workflow's pin).

## CI

- **benchmark.yml** (PR + main pushes, path-filtered): pr profile → check → step
  summary + artifacts. **Advisory by policy** — never make it a required status
  check while path-filtered (GitHub leaves path-skipped required checks Pending,
  blocking docs-only PRs). Red = visible shame, not a merge lock.
- **benchmark-nightly.yml** (02:43 UTC + dispatch): full profile → check (the
  `profile = "full"` enforced rows' only enforcement point) → artifacts → history
  push to `gh-pages` under `bench/` via github-action-benchmark
  (`customSmallerIsBetter`; names are the stable metric IDs from
  `pipeline.HEADLINE_METRICS`).
- **benchmark-compare.yml** (dispatch: base, head): the A/B evidence tool. The
  harness is ALWAYS the invoking ref's `benchmarks/`; each side is its own
  built venv from its own lockfile (pyperf injected as harness infrastructure), and
  the harness runs *under that side's python*, so the benchmarked venv is the side's
  while the harness code is fixed. Results carry each side's own git identity
  (`--measured-repo`). Micro scripts that can't import an older side's API record
  per-script skips carrying the actual error (`BENCH_COMPARE=1`). Compatibility
  floor: sides must postdate the prompt-kind store API (`725f11d`) — the dataset
  generator uses it, so older refs fail dataset generation before any suite runs.

Hosted-runner wall clock is **advisory**: medians move with the neighbors' noisy
workloads. Trend lines and A/B-on-one-runner are meaningful; single absolute numbers
are not. Wall-clock budget rows stay `target`-tier until fixed hardware exists and
its noise distribution is measured.

### One-time setup (merge checklist)

1. Create the history branch: `git switch --orphan gh-pages && git commit
   --allow-empty -m "bench history" && git push origin gh-pages` (the action
   documents pre-creating it).
2. Repo Settings → Pages → deploy from `gh-pages` so the chart is served.
3. Dispatch `benchmark (nightly)` once and confirm the `profile = "full"` enforced
   rows evaluate green — schedule-only workflows never run pre-merge.
4. Tighten the provisional ratchet bounds from the merged PR's CI artifact
   (`check --propose`).
5. After ~14 nightly points: pick alert thresholds for github-action-benchmark
   (currently `fail-on-alert: false` — trend line first).

## Adding a suite

1. Decide the metric IDs (dotted, stable — they are budget keys and history names).
2. Pure parsing/derivation goes in `parsers.py` (covered, tested against fixture
   output); spawning goes in a new `suites/<name>.py` exposing
   `run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput`.
3. Add the suite to the profile table in `pipeline.build_plan` and, if headline, to
   `HEADLINE_METRICS`.
4. Skips are pre-spawn decisions with reasons; a crash must crash. Anything the
   suite cannot run on the reference CI platform will trip the skip budget — that is
   the point.
5. Update the profile table here and the design doc.

## Why no CodSpeed / SaaS

The pipeline is self-contained (pyperf + hyperfine + artifacts + gh-pages) so it
works without accounts, tokens, or third-party availability, and A/B evidence stays
reproducible from the repo alone. CodSpeed's simulation mode would add stable
PR-regression signal later without redesign: the micro layer is plain callables, so
`pytest-codspeed` wrappers can be added alongside pyperf if that trade ever earns
its onboarding. Codecov is the repo's one SaaS precedent; anything added here must
match its property: optional, never a merge gate, degrades to nothing.
