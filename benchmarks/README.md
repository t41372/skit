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
| `pipeline.py` | profiles, merge, derived metrics, results.md render |
| `datasets.py` | deterministic library generator (public store API only) |
| `hyperfine.py` | hyperfine argv building + export parsing (no subprocess) |
| `envinfo.py` | host manifest; budget predicates key on its output |
| `compare.py` | A/B delta report (warn-only) |
| `suites/` | orchestration: spawns processes under the env contract |
| `micro/` | self-contained pyperf scripts (skit + pyperf + stdlib only) |
| `fixtures/` | noop benchmark subjects + seeded analyzer-source generators |

Trust split: everything that computes, decides, or persuades sits under the repo's
100% coverage floor; only `suites/*`, `micro/*`, and `fixtures/noop.py` are exempt
(spawn-and-wait orchestration and benchmark subjects —
the exact list, with reasons, is in `pyproject.toml`'s coverage `omit`).

## Definitions (what the numbers mean)

- **process-cold, filesystem-warm** — every macro sample is a fresh process, after
  warmup runs have warmed the page cache and (where uv is involved) `UV_CACHE_DIR`.
  Cold-filesystem / first-ever-install journeys are separate future suites, never
  mixed into these numbers.
- **the import census is N-dependent** — resolving an entry's kind builds its `LangSpec`,
  which imports that language's grammar, so `imports.list_json` is measured once per N.
  `n0` is the CLI floor (no kind resolved); `n100` is what a real library pays, and the
  only tier where `has_tree_sitter` can be anything but 0. `imports.version.*` carries no
  N — `--version` never reads the library.
- **cold import vs warm parse** — `micro.analyze_cold.*` is a one-shot subprocess
  (first import + first parse); `micro.analyze.*` is pyperf's warm in-process loop.
  Never averaged together.
- **`micro.analyze_broken.*` is only meaningful beside its twin** — the same 2000-line
  source with its last line left half-written, which is what a launcher parses while
  its user is still editing. It comes out markedly FASTER than the valid twin, because
  the analyzer bails out (Python raises, the grammars enter error recovery) and
  returns no parameters — a number that looks like a speed-up and is nothing of the
  kind. The pair `analyze.<lang>.l2000` / `analyze_broken.<lang>.l2000` is the
  measurement; the actual ratio lives in each run's `results.json`, never here.
- **median / p95** — headline values are medians; p95 is nearest-rank
  (`ceil(0.95·n)`); raw samples ship in `results.json` under `raw`.
- **TUI spans are proxies** — headless Textual (`run_test`, 120×40), not terminal
  paint: span 1 `import skit.tui`, span 2 App() → first `pilot.pause()` returns,
  span 3 press `down` while the table still owns focus (cursor moves one row, the
  detail pane re-renders), span 4 focus search (`/`), settle, then measure
  `press(<probe char>)` → settle (the char is `datasets.SEARCH_PROBE_CHAR`). The probe
  asserts the row count matches the library, that the cursor actually moved, that the
  filter really dropped rows (the dataset guarantees a probe-char-free entry), and —
  at 3+ entries — that some rows survive (a matching entry is guaranteed too), so the
  span never degenerates to filter-to-zero. `tui.select` is absent below N=2: there is
  no second row to move to, and a span that measured nothing is worse than none.
- **run overhead** — lane A `python noop.py`; lane B `uv run --no-project --script
  noop.py` (the EXACT argv skit builds — `src/skit/langs/launch.py`); lane C
  `skit run noop-py --no-input`. Core overhead = C − B. C legitimately includes
  skit's post-run state persistence (two fsync'd, constant-size writes). Lanes run
  in a dedicated 3-entry library, cwd outside any uv project.
- **`scale.list_json.per_entry_us`** — (median_ms(N=1000) − median_ms(N=0)) is the
  total ms for 1000 entries; numerically that IS the per-entry µs figure (÷1000
  entries × 1000 µs/ms cancel). Stated so nobody "fixes" it into a 1000× lie.
- **`footprint.library_*` measures the USER, not the tool** — every other footprint
  metric is what installing skit costs (wheel, sdist, closure). `library_bytes` is
  what the user's own entries weigh in the store, `library_state_bytes` what their
  remembered values and presets weigh, `library_total_bytes` their sum, and
  `library_bytes_per_entry` that sum divided by N — the per-entry figure divides into
  the TOTAL, not into the store figure it is printed beside.
  **Host-dependent, so never budget material**: every meta.toml records its entry's
  absolute source path, so the same seeded library totals different bytes under a CI
  workdir and a local /tmp one. It compares one host against itself, which is what an
  A/B run does. Per-script `node_modules` is deliberately NOT in here: materializing it
  needs npm and the network, which the pr profile must not touch.

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
`GENERATOR_VERSION` and is a history discontinuity (say so in the PR that does it).
The same applies to the fixed runner label (`ubuntu-24.04`) when it eventually
EOLs, and to its periodically refreshed image build (recorded as
`meta.host.ci_image_version`), and to major dependency bumps (textual above all —
the TUI proxy rides on it; the manifest records versions for exactly this reason).

## Profiles

| Suite | pr | full (nightly) | compare (A/B) |
| --- | --- | --- | --- |
| startup | 3 warmup + 15 runs | 5 + 40 | 3 + 15 |
| scale | N ∈ {0, 100, 1000} | {0, 10, 100, 1000} + doctor | {0, 100, 1000} |
| run_overhead | python + shell | + JS lane | python + shell |
| rss | 5 samples | 10 | 5 |
| imports | census at N ∈ {0, 100} (deterministic) | same | same |
| tui | 5 probes × {0, 100, 1000} | 10 × same | 5 × same |
| micro | pyperf `--fast` | full rigor | `--fast` |
| syscalls | — | list --json @1000 | — |
| footprint | wheel+sdist+library | + closure, dist sizes | — (would measure the harness ref) |

N=100 is the *typical-library* scale; 1000 is the stress point the budgets quote.
The rendered summary always shows both.

## Budgets: the two-tier contract

`budgets.toml` rows are `enforced` (deterministic/ratchet-safe only — `check` exits
non-zero) or `target` (the aspirational contract — reported loudly, never failing CI
until a future PR deliberately promotes a row; wall-clock rows only ever on fixed
hardware). Optional predicates (`profiles`, `platform`, `ci_only`) scope rows;
non-matching hosts see "not applicable", and every distinct decay channel is its own
hard failure: violation, **metric missing** (renamed ID / crashed suite),
**predicate unevaluable** (absent/empty meta field), **python mismatch on CI**
(a ratchet bound gating a census it wasn't set on — the module census differs across
python versions). CI runs `check --require-enforced`, which also fails when zero
applicable enforced rows were evaluated.

**Ratchet protocol:** ratchet rows (`ratchet = true`) bound a measured value +
headroom. Refresh them ONLY from a CI artifact — `uv run python -m benchmarks check
<ci-results.json> --propose` prints the exact replacement file. Both halves of that
rule are enforced, not advisory: propose **refuses** a local or dirty-tree artifact
(the census is platform- and python-dependent, so a laptop's number must never become
an enforced ceiling), and it **refuses to widen** a bound — a regression is what makes
`check` fail, and rewriting the failing bound to fit it turns a red gate into a rubber
stamp. Bounds do legitimately loosen sometimes (a dependency bump that really does add
modules); `--allow-regression` says so out loud, and the row's `note` should say why. A PR that
intentionally moves an enforced metric updates budgets.toml in the same PR. When a
measured value sits under 85% of its ceiling, `check` nags to tighten. `--propose`
stamps each refreshed row's `context` from the artifact it read: python, date, and
either `pr` (PR artifacts — a PR run's HEAD is GitHub's ephemeral merge ref, which no
clone resolves and squash-merge deletes) or `commit` (everything else). On CI a ratchet
row whose `context.python` disagrees with the running python **fails** — a bound may
not gate a census it was never set on.

## CI

- **benchmark.yml** (PR + main pushes, path-filtered): pr profile → check → step
  summary + artifacts. **Advisory by policy** — never make it a required status
  check while path-filtered (GitHub leaves path-skipped required checks Pending,
  blocking docs-only PRs). Red = visible shame, not a merge lock.
- **benchmark-nightly.yml** (02:43 UTC + dispatch): full profile → check (the
  `profiles = ["full"]` enforced rows' only enforcement point) → step summary →
  artifacts (`results.json` and the full per-suite output, 90-day retention).
- **benchmark-compare.yml** (dispatch: base, head): the A/B evidence tool. The
  harness is ALWAYS the invoking ref's `benchmarks/`; each side is its own
  built venv from its own lockfile (pyperf injected as harness infrastructure), and
  the harness runs *under that side's python*, so the benchmarked venv is the side's
  while the harness code is fixed. Results carry each side's own git identity
  (`--measured-repo`). The compare profile carries `compare_mode` on every
  SuitePlan, so micro scripts that can't import an older side's API record
  per-script skips carrying the actual error — including on a local
  `run --profile compare`. Compatibility
  floor: sides must postdate the prompt-kind store API (`725f11d`) — the dataset
  generator uses it, so older refs fail dataset generation before any suite runs.

Hosted-runner wall clock is **advisory**: medians move with the neighbors' noisy
workloads. Trend lines and A/B-on-one-runner are meaningful; single absolute numbers
are not. Wall-clock budget rows stay `target`-tier until fixed hardware exists and
its noise distribution is measured.

### Why there is no trend chart here

An earlier draft published headline metrics to a `gh-pages` branch via
`benchmark-action/github-action-benchmark`, and the setup checklist said to point
Settings → Pages at that branch. That was wrong twice over. A repository has exactly
one Pages deployment and this one belongs to the documentation site, which publishes
by artifact upload (`docs.yml`, `build_type = workflow`) — following the checklist
would have taken https://t41372.github.io/skit/ down. And the chart was never the
mechanism: what stops a regression is `budgets.toml` plus `check --require-enforced`,
a bound in the repo that fails CI, not a line on a page nobody is watching.

Trends over time are CodSpeed's job (see below), and for the metrics CodSpeed cannot
measure — wheel bytes, closure bytes, module censuses, syscall counts — a bound beats
a curve: they are deterministic, they are `enforced`, and their history is the git log
of `budgets.toml`, where every ratchet carries a commit message explaining the move.

### One-time setup (merge checklist)

1. Dispatch `benchmark (nightly)` once and confirm the `profiles = ["full"]` enforced
   rows evaluate green — schedule-only workflows never run pre-merge.
2. Re-propose the ratchet bounds if the first main-push run's census differs from the
   PR's (`check --propose`) — squashing changes no imports, so it normally won't.

## Adding a suite

1. Decide the metric IDs (dotted, stable — they are budget keys and history names).
   IDs read `<suite>.<case>[.<subcase>].<stat>`: statistical stats carry their unit
   suffix (`median_ms`, `peak_kib`), deterministic counts don't (`modules`,
   `file_ops`, `distributions`). The headline set is `pipeline.HEADLINE_METRICS` —
   in code, so it can't drift.
2. Pure parsing/derivation goes in `parsers.py` (covered, tested against fixture
   output); spawning goes in a new `suites/<name>.py` exposing
   `run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput`.
3. Add the suite to the profile table in `pipeline.build_plan` and, if headline, to
   `HEADLINE_METRICS`.
4. Skips are pre-spawn decisions with reasons; a crash must crash. Anything the
   suite cannot run on the reference CI platform will trip the skip budget — that is
   the point.
5. Update the profile table here and the design doc.

## Why no CodSpeed / SaaS *here*

The pipeline is self-contained (pyperf + hyperfine + artifacts) so it works without
accounts, tokens, or third-party availability, and A/B evidence stays reproducible from
the repo alone. That property is the point, and it does not change.

CodSpeed is adopted alongside it (#32) for the one thing this pipeline structurally
cannot do: its simulation mode measures CPU instructions rather than wall clock, which
is hardware-independent, so a PR-time timing regression can actually fail instead of
drowning in whichever CPU the runner drew. The micro layer is plain callables, so
`pytest-codspeed` wraps it without redesign.

One rule its benchmarks inherit from this pipeline: **a store benchmark states its kind
mix.** `benchmarks/codspeed/test_bench_store.py` runs each read path over two libraries
— 200 command templates (100% reference mode, the index worst case, cheap to build) and
the seeded generator's own mix (~84% copy mode, what a real library looks like). The
index carries per-row fields that only reference entries need, so the two shapes give
materially different answers; reporting either alone would have been a half-truth.

What stays here:
`budgets.toml` remains the contract, and every metric CodSpeed's instruments cannot take
— wheel bytes, closure bytes, module censuses, syscall counts. Codecov is the repo's
SaaS precedent and the bar anything else must clear: optional, never a merge gate,
degrades to nothing.
