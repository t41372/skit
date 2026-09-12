# FakeHost / legacy model-walker migration ledger

Worktree: `/home/ubuntu/coding/skit-uiwalker-mouse-integration` (branch `integration/ui-walker-on-mouse`).
Read-only analysis; nothing in the repository was modified.

## Deletion set

| file | lines | `#[test]` |
|---|---|---|
| `crates/skit-tui/tests/model_walker.rs` | 26 | 0 (module wiring only) |
| `crates/skit-tui/tests/model_walker/asciicast.rs` | 1 | 0 (one `pub(super) use` re-export of `skit_tui_walker_support::asciicast::{AsciicastRecorder, FRAME_INTERVAL}`) |
| `crates/skit-tui/tests/model_walker/cast_projection.rs` | 199 | 4 |
| `crates/skit-tui/tests/model_walker/corpus.rs` | 1849 | 15 |
| `crates/skit-tui/tests/model_walker/driver.rs` | 4066 | 46 |
| `crates/skit-tui/tests/model_walker/fake_host.rs` | 6128 | 62 |
| `crates/skit-tui/tests/model_walker/fixtures.rs` | 1262 | 3 |
| `crates/skit-tui/tests/model_walker/invariants.rs` | 1387 | 25 |
| `crates/skit-tui/tests/model_walker/local_inventory.rs` | 523 | 8 |
| `crates/skit-tui/tests/model_walker/strategy.rs` | 647 | 8 |
| **total** | **16 088** | **171** |

Ground truth: `cargo test --locked -p skit-tui --test model_walker -- --list` reports exactly
**171 tests, 0 benchmarks**, matching the static enumeration. There are **no `proptest!` macro
tests, no `prop_*` functions and no rstest cases** — proptest is used through its API
(`TestRunner`, `Strategy`, `Config`, `collection::vec`) inside `driver.rs` and `strategy.rs`, so the
harness contributes no extra test functions. Named invariants live as uppercase error codes inside
`invariants.rs`'s `check_state` checker; they are catalogued as one infrastructure row in section 11.

**Closed 2026-09-06 (MIG-F).** All ten files are deleted. `crates/skit-tui/Cargo.toml` lost the
`proptest` and `skit-tui-walker-support` dev-dependencies with them, and `Cargo.lock` lost the same
two entries.

## Classification key

- **A** production `TuiHost` / real-host composition (`crates/skit-cli/src/cli/tui_host.rs`,
  `tui_real_host.rs`, `tests.rs`)
- **B** store / application invariant (`crates/skit-store`, `crates/skit-application`)
- **C** low-level fault test at an injected port
- **D** real reachable walker flow (`crates/skit-cli/src/cli/tui_real_walker.rs`,
  `tui_walker_bundle.rs`, `crates/skit-tui-walker-support`)
- **E** reducer / frontend-session level (`crates/skit-ui` pure `update`, or the non-fake
  `crates/skit-tui/tests/*.rs` session suites)
- **F** obsolete: asserts what the fake or the legacy harness does, not what the product does

`MM` = **MUST-MIGRATE** (section 5). Equivalence column: **yes** = a cited real-owner test asserts
the same thing; **partial** = a cited test covers part of it; **none** = nothing found in a bounded
grep of the real owners.

**Provenance of the equivalence column.** Every *legacy* row was read in full. Real-owner citations
were located by name and line from `awk`-extracted test lists over `tui_host.rs`, `tui_real_host.rs`,
`tui_real_walker.rs`, `tui_walker_bundle.rs`, `tui_real_sandbox_fs.rs`, `crates/skit-cli/src/cli/tests.rs`,
`crates/skit-tui-walker-support/src/*.rs`, `crates/skit-store/tests/*.rs`,
`crates/skit-application/tests/*.rs` and the surviving `crates/skit-tui/tests/*.rs`. The following
citations were additionally **body-verified**: `tests.rs:1615`, `:3690`, `:3952`, `:4388`, `:7344`,
`:9341`; `tui_real_walker.rs:633`, `:1183`, `:1676`, `:2192`, `:2396`, `:2417`, `:5838`, `:6466`,
`:6747`, `:7036`, `:7064`; `tui_walker_bundle.rs:2846`; `bundle_tests.rs:486`;
`contract_tests.rs:187`; `engine_tests.rs:829`, `:1120`; `mutation_refusals.rs:127`;
`skit-application/tests/form_state.rs:81`; `interactive_run_form.rs:585`; `render.rs:1301`;
`session.rs:443`, `:1579`, `:1666`, `:1726`, `:1782`. The remaining "yes" rows are **name-inferred**.
Treat a name-inferred "yes" as a strong hint, not a licence to delete without one confirming read.

---

## 1. Count summary

**Counting convention.** A few rows carry a compound class (`E+A`, `A+B`, `B+E`, `E + F`); the
tables below are tallied by the **first** letter in the cell. Equivalence cells are bucketed as
`yes` (including `replaced by …` and `conceptual — …`), `partial`, and `none` (including `n/a`).
Every figure in this section was produced by parsing the tables in sections 6-13, not by hand.

### By classification

| classification | rows | yes | partial | none |
|---|---|---|---|---|
| A real host | 39 | 30 | 9 | 0 |
| B store / application | 3 | 2 | 1 | 0 |
| C injected-port fault | 5 | 4 | 1 | 0 |
| D real walker flow | 54 | 39 | 9 | 6 |
| E reducer / session | 23 | 3 | 8 | 12 |
| F obsolete | 47 | 11 | 1 | 35 |
| **non-F total** | **124** | **78** | **28** | **18** |

Of the **124 non-F rows**, **78 already have a cited real-owner equivalent**, 28 are partially
covered, and 18 have none. **34 rows are flagged MUST-MIGRATE** (33 numbered items in section 5;
item 5 covers two rows and item 19 covers three).

### By file

| file | tests | A | B | C | D | E | F | MM |
|---|---|---|---|---|---|---|---|---|
| cast_projection.rs | 4 | 0 | 0 | 0 | 4 | 0 | 0 | 0 |
| corpus.rs | 15 | 0 | 0 | 1 | 13 | 0 | 1 | 0 |
| driver.rs | 46 | 0 | 0 | 3 | 31 | 5 | 7 | 10 |
| fake_host.rs | 62 | 39 | 3 | 1 | 0 | 7 | 12 | 15 |
| fixtures.rs | 3 | 0 | 0 | 0 | 0 | 1 | 2 | 1 |
| invariants.rs | 25 | 0 | 0 | 0 | 0 | 4 | 21 | 1 |
| local_inventory.rs | 8 | 0 | 0 | 0 | 0 | 6 | 2 | 5 |
| strategy.rs | 8 | 0 | 0 | 0 | 6 | 0 | 2 | 2 |
| **total** | **171** | **39** | **3** | **5** | **54** | **23** | **47** | **34** |

Two infrastructure rows sit outside this table because they are not `#[test]`s: the
`invariants::check_state` checker (section 11 row 0) and the `check_public_hit_parity` probe family
(section 8 row 0). Both are MUST-MIGRATE and are migration priorities 1 and 2.

Headline: **`corpus.rs` and `cast_projection.rs` are almost entirely redundant** (the
`skit-tui-walker-support` crate carries 155 tests over the same artifact contracts, and
`tui_walker_bundle.rs` carries 81 more). **`fake_host.rs` is ~80 % redundant** with
`crates/skit-cli/src/cli/tests.rs` (12 272 lines, 182 tests over real `TempDir` stores and real
services). The concentrated loss is in **`local_inventory.rs`, the pointer/parity tests inside
`driver.rs`, the `invariants::check_state` per-step checker, and the randomized property walk
itself**.

**Closed 2026-09-06 (MIG-F).** These counts are history. The 124 non-F rows have the owners that
section 5 lists, and the 47 class-F rows are dropped. The deletion removed all 171 legacy tests.

---

## 2. Production (non-test) code that exists only for FakeHost or the legacy walker

**Finding: none.** Every production seam the legacy owner touches is also consumed by the real-host
owner or by a surviving `crates/skit-tui/tests/*.rs` suite. Deleting `model_walker/` therefore
requires **no production-source change**.

| seam | file:line | also used by |
|---|---|---|
| `mod agent_review` + `#[doc(hidden)] pub use agent_review::{AGENT_REVIEW_SNAPSHOT_VERSION, AgentReviewSnapshot, AgentReviewSnapshotError, RATATUI_TEXTAREA_CRATE, RATATUI_TEXTAREA_VERSION}` | `crates/skit-tui/src/lib.rs:5`, `:30-33` | real owner — `crates/skit-cli/src/cli/tui_real_walker.rs:586`, `crates/skit-cli/src/cli/tui_real_host.rs:5816`, `:16728`, `:16960`, `:22937` |
| `TuiSession::agent_review_snapshot` | `crates/skit-tui/src/session.rs:1579` | real owner — `tui_real_walker.rs:586` (`RealFrontend::session_value`) |
| per-screen `agent_review_snapshot` accessors (12 sites) | `footer.rs:263`, `:516`; `screens/library.rs:91`; `screens/report.rs:41`; `screens/run_modal.rs:121`; `screens/management.rs:105`, `:502`, `:826`; `screens/picker.rs`; `screens/add.rs`; `screens/preferences.rs`; `screens/settings.rs` | reached through `TuiSession::agent_review_snapshot`, i.e. the real owner; each also has its own `#[cfg(test)] mod agent_review_tests` in the same file |
| `perturb_agent_review_state` (14 sites) | `crates/skit-tui/src/footer.rs:512`, `screens/library.rs:87`, `screens/modal.rs:113`/`:365`, `screens/report.rs:37`, `screens/run_modal.rs:117`, `screens/management.rs:101`/`:498`/`:822`, `screens/add.rs:288`, `screens/preferences.rs:438`, `screens/settings.rs:258` | **skit-tui's own** `#[cfg(test)]` test `every_deterministic_top_level_field_is_snapshot_sensitive` at `crates/skit-tui/src/session.rs:443-560`. Never referenced by `model_walker/`. Survives. |
| `TuiSession::try_fork` | `crates/skit-tui/src/session.rs:1666` (and the path-completion inner fork at `:1278`) | `crates/skit-tui/tests/interactive_run_form.rs`, `crates/skit-tui/tests/overlay_pointer_priority.rs`. **Not** used by `tui_real_*.rs` — the real walker never forks a session. Survives via the skit-tui suites; if the pointer-parity migration (section 5) lands in `tui_real_walker.rs`, that owner will need it too. |
| `TuiSession::advertised_command_bindings` | `crates/skit-tui/src/session.rs:1782` | `crates/skit-tui/tests/interactive_run_form.rs`. **Not** used by `tui_real_*.rs`. Survives. |
| `TuiSession::local_action_inventory` | `crates/skit-tui/src/session.rs:1726` | real owner — `tui_real_walker.rs:1433`, `:6765`; `tui_real_host.rs` |
| `#[doc(hidden)] pub use screen_target::{ScreenFocusInventory, ScreenTarget, ScreenTargetError, ScreenTargetHit, ScreenTargetInventory}` | `crates/skit-tui/src/lib.rs:41-44` | real owner — `tui_real_walker.rs` (the `ScreenFocus`/`ScreenHit` corpus operations) |
| `pub use local_action::{LocalActionInventory, LocalActionOutcome, LocalActionTarget, LocalAdvertisedAction, LocalKeyBinding}` | `crates/skit-tui/src/lib.rs:35-38` | real owner — `tui_real_walker.rs` (`LocalKeyboard`/`LocalHit` operations). `LocalKeyBinding` alone has no consumer outside `crates/skit-tui/src` — a pre-existing unused re-export, unrelated to this deletion. |
| `map_event` | `crates/skit-tui/src/lib.rs` / `session.rs` | real owner — `tui_real_walker.rs:1151`; also `crates/skit-tui/tests/render.rs` |

Two `#[doc(hidden)]` constants — `RATATUI_TEXTAREA_CRATE` and `RATATUI_TEXTAREA_VERSION`
(`crates/skit-tui/src/lib.rs:32`) — have no consumer anywhere outside `agent_review.rs` and the
re-export, **including the legacy walker**. They are pre-existing dead exports, not part of this
deletion set; flag separately if wanted.

No feature flags and no `cfg(test)` seams in `crates/skit-ui/src` serve the legacy walker. The one
`skit-ui` constructor the legacy tests lean on heavily,
`LibraryState::from_library_surface` (`crates/skit-ui/src/lib.rs:1807`, used at
`invariants.rs:941`, `:1018`, `:1376`), has a **production** caller at
`crates/skit-cli/src/cli/tui_host.rs:508` plus `crates/skit-benchmarks/src/tui_probe.rs:166`, `:403`
and `crates/skit-cli/src/cli/tests.rs:8288`. It survives.
`crates/skit-tui-walker-support` is a **shared** crate: the legacy walker aliases it as
`pub(crate) use skit_tui_walker_support as artifact` in `model_walker.rs:8`, and the real owner
depends on it through `crates/skit-cli/Cargo.toml`'s `[dev-dependencies] skit-tui-walker-support`.
The crate survives; only skit-tui's dev-dependency on it dies.

**Closed 2026-09-06 (MIG-F).** The deletion changed no production source, as this section says.
Every listed seam keeps a real consumer.

---

## 3. Entry point, dev-dependencies and the CI workflow

### `crates/skit-tui/tests/model_walker.rs` (26 lines)

Pure module wiring: a doc comment explaining why the walker uses plain proptest instead of a
reference state machine, `pub(crate) use skit_tui_walker_support as artifact;`, and nine
`#[path = "model_walker/…"] mod …;` declarations. It owns **no test**. Deleting the directory
deletes this file with it.

### `crates/skit-tui/Cargo.toml` dev-dependencies

Evidence from grepping every remaining skit-tui test file and `crates/skit-tui/src`:

| dev-dep | used outside `tests/model_walker/`? | verdict |
|---|---|---|
| `proptest` | **no** | **remove** — the only consumers are `model_walker/strategy.rs:1,234,243,246,251,451` and `model_walker/driver.rs:11-15,1806,1980,1991` |
| `skit-tui-walker-support` | **no** | **remove** — only `model_walker.rs:8` and its submodules. `crates/skit-cli/Cargo.toml` keeps its own dev-dependency, so the crate itself stays alive. |
| `glob` | yes | keep (`tests/preferences_workflow.rs`, `settings_screen.rs`, `overlay_pointer_priority.rs`, `port_test_tui_responsive.rs`, `render.rs`, `interactive_search.rs`, `interactive_run_form.rs`, `port_test_path_tui.rs`, plus `src/footer.rs`, `src/session.rs`) |
| `portable-pty` | yes | keep (`tests/terminal_pty.rs`) |
| `skit-language` | yes | keep (`tests/port_test_draft_and_reader_tui.rs`) |
| `skit-store` | yes | keep (`tests/port_test_path_tui.rs`, `tests/port_test_tui_edit.rs`) |
| `tempfile` | yes | keep (`tests/viewport_session_invariants.rs`, `terminal_pty.rs`, `port_test_path_tui.rs`, `interactive_run_form.rs`, plus `src/session.rs`, `src/screens/picker.rs`, `src/screens/run_modal.rs`) |
| `skit-i18n` | yes | keep (18 remaining test files) |

### `.github/workflows/ui-walker.yml`

The workflow invokes **exactly one selector**, twice:

```
cargo test --locked -p skit-tui --test model_walker driver::nightly_model_walk -- --exact --ignored --list
cargo test --locked -p skit-tui --test model_walker driver::nightly_model_walk -- --exact --ignored --nocapture
```

with `SKIT_WALKER_CASES=16`, `SKIT_WALKER_STEPS=100`, and `SKIT_WALKER_RECORD_SUCCESS` set to `1`
only for a labelled PR or a `workflow_dispatch` with `record_success`. It then:

- asserts the `--list` output contains exactly one line `driver::nightly_model_walk: test`
  (a guard that would silently start passing/failing wrongly if the test disappears);
- runs the walk under `timeout --kill-after=5m 65m` (job timeout 90 min);
- when `RECORD_SUCCESS=1`, requires exactly one
  `target/ui-walker-artifacts/success-*/success.cast`;
- downloads pinned `agg` v1.9.0 (SHA-256 checked) and renders every
  `target/ui-walker-artifacts/{failure-*/failure.cast,success-*/success.cast}` to a GIF;
- uploads `target/ui-walker-artifacts/` with `if-no-files-found: error`.

**`target/ui-walker-artifacts` is produced only by the legacy driver.** Grep across `crates/` and
`.github/` returns exactly three producers/consumers: `model_walker/driver.rs:60` (`ARTIFACT_DIR`),
`model_walker/driver.rs:64` (`REGRESSION_FILE` → `regressions.txt`, proptest's
`FileFailurePersistence::Direct` seed file), and the workflow itself. The real owner writes its
bundles through `install_bundle` into a **caller-supplied parent** (a `tempfile::TempDir` in every
test — `tui_walker_bundle.rs:2846 installs_real_smoke_bundle_and_matches_exact_layout`), names them
`bundle-<digest>` rather than `failure-*`/`success-*`, and has **no `#[ignore]`d test at all**
(`grep '#\[ignore' crates/skit-cli/src/cli/tui_real_walker.rs tui_walker_bundle.rs` → empty), so it
already runs inside the ordinary `cargo test --workspace` suite.

**Consequence:** deleting the legacy owner leaves `ui-walker.yml` with no target. It is **not a
selector swap** — there is no drop-in replacement that writes `failure.cast`/`success.cast` under
`target/ui-walker-artifacts/`. Three options, in order of cost:

1. **Retire the workflow** (delete `ui-walker.yml` and the `ui-walker-requested` label). The real
   walker's coverage already runs on every PR through the normal test job. Costs the nightly
   randomized sweep and the GIF artifact.
2. **Retarget with a new writer**: add an `#[ignore]`d entry point in `tui_real_walker.rs` that
   records the canonical corpus for the required profiles, installs the bundle under
   `target/ui-walker-artifacts/`, and emits a `success.cast`/`failure.cast` at the globbed paths;
   then change the selector, the package (`-p skit-cli-rs --lib`) and the env-var names. The
   `agg`/upload/label machinery can stay byte-for-byte.
3. **Keep the workflow, drop the cast render**: point it at a real-walker corpus test and delete the
   `agg` install, smoke-test and render steps plus the `RECORD_SUCCESS` branch.

Either way the three env vars `SKIT_WALKER_CASES`, `SKIT_WALKER_STEPS`, `SKIT_WALKER_RECORD_SUCCESS`
and the two other legacy env vars `SKIT_WALKER_AGENT_STEPS`, `SKIT_WALKER_REVIEW` /
`SKIT_WALKER_EXPECT_REVISION` / `SKIT_WALKER_REPRO` (driver.rs:2292, 2305, 2319, 2345) die with the
file. `SKIT_WALKER_ALLOCATOR_UMASK_CHILD` (`tui_real_host.rs:13421`) is unrelated and survives.

**Closed 2026-09-06 (MIG-F).** MIG-E took option 2. `.github/workflows/ui-walker.yml` and the pins
in `scripts/test_tooling_contracts.sh` name `cli::tui_real_random_walk::real_random_walk`, and
`crates/skit-cli/src/cli/tui_real_random_walk.rs` writes `target/ui-walker-artifacts/`. MIG-F
removed the two dev-dependencies this section names.

---

## 4. `driver::nightly_model_walk` — what it randomizes and what it checks

### The property walk

`driver.rs:2263-2287` reads `SKIT_WALKER_CASES` (default 16, CI 16) and `SKIT_WALKER_STEPS`
(default 100, CI 100), refuses zero for either, parses `SKIT_WALKER_RECORD_SUCCESS` as strictly
`0`/`1`, then calls `run_property_profile("nightly_model_walk", cases, steps, COMPLETE_PROFILES,
record_success)` (`driver.rs:1972-2065`).

`run_property_profile` builds
`Config { cases, max_shrink_iters: 512, failure_persistence: FileFailurePersistence::Direct("target/ui-walker-artifacts/regressions.txt"), rng_algorithm: RngAlgorithm::ChaCha, .. }`
and runs `collection::vec(operation_strategy(), 0..=steps)`.

**What is randomized** (`strategy.rs:224-268`, weighted `prop_oneof!`):

| family | weight | randomized payload |
|---|---|---|
| `AdvertisedKey { command: u8, binding: u8 }` | 12 | late-bound ordinals into the live enabled command registry (Quit excluded) |
| `PublicHit { ordinal: u8 }` | 10 | ordinal into the live non-empty, non-Quit `geometry.hits` |
| `LocalAdvertisedKey { action: u8, binding: u8 }` | 12 | ordinals into the live `LocalActionInventory` |
| `LocalHit { action: u8 }` | 10 | ordinal into the live inventory, must have a `hit` rect |
| `MouseCell { x_fraction: u8, y_fraction: u8, kind }` | 8 | any cell in the viewport × 8 `MouseKind`s (LeftDown/RightDown/MiddleDown/LeftUp/LeftDrag/Move/ScrollUp/ScrollDown) |
| `Resize { width, height }` | 3 | one of the 10 `RESIZE_CASES` |
| `Paste { value }` | 5 | 4:1 mix of the 7 `PASTE_CASES` and a `.{0,24}` regex string |
| `RawKey { key, kind }` | 8 | 19 `RawKey`s × 3 `KeyKind`s (Press/Repeat/Release) |
| `Focus { gained: bool }` | 1 | FocusGained / FocusLost |

Every generated vector is then replayed against **all 15 `COMPLETE_PROFILES`**
(`driver.rs:76-92`): En/ZhCn/ZhTw/Pseudo each at 1×1, 24×6 and 120×30, plus En 80×24,
ZhTw 40×40, Pseudo 120×12 and 120×30. A failure in any profile fails the case; proptest shrinks
(512 iterations), the minimal trace is replayed to confirm it still fails, and
`write_failure_artifacts` emits `failure-*/{repro.json,failure.cast}`. `catch_unwind` around both
construction and the walk converts a panic into a captured failure rather than losing the cast.

### What each checkpoint checks (`driver.rs:576-700`)

Every transition boundary — Initial, Session, UserAction, HostAction — runs, in order:

| # | check | real owner runs it? |
|---|---|---|
| 1 | `invariants::check_state(&self.state)` — a `LibraryState` JSON round-trip stability check followed by the full screen + modal invariant set (≈40 named codes, section 11) | **the round trip: yes. The ≈40 shape codes: NO.** `RealFrontend::observe_frontend` (`tui_real_walker.rs:633-655`) and `CorpusFrontend::observe` (`:1676`) call only `validate_styled_frame`. Separately, the trace **read-back** path runs `parse_library_state` (`tui_real_walker.rs:2417`), which decodes each reducer object and requires `json_value(&state) == value` — that is exactly `check_state`'s round-trip half — and `validate_reducer_action` (`:2396`), which replays the action through the production reducer and compares both the emitted effect and the resulting state. Neither inspects screen or modal shape. Verified by reading both functions and `validate_reducer_transitions` (`:2192`). **Gap: every named code in section 11 row 0.** |
| 2 | the frame renders without a Ratatui error, and a render error is reported together with any invariant error | **partial** — the real engine propagates the draw error (`tui_real_walker.rs:637`) but has no paired-error message |
| 3 | frame structure / cursor / style / width validation | **yes** — `validate_styled_frame` at `tui_real_walker.rs:648`; contract in `skit-tui-walker-support/src/lib.rs:723` and its tests at `lib.rs:2292`, `:2344`, `:2457` |
| 4 | `check_public_hit_parity` (`driver.rs:965-1300`) — for every non-empty geometry hit: the rect is inside the viewport (`PUBLIC_HIT_BOUNDS`); a forked-session primary click yields the expected typed action (`PUBLIC_SESSION_HIT`); the advertised keyboard binding for the same target reaches the same endpoint; `FocusField`/`ToggleField`/`SelectFieldOption` hits go through `check_field_hit_parity`, which compares the complete rendered endpoint (frame + geometry) after mouse and after keyboard | **NO.** The real owner validates only target *well-formedness and availability* (`validate_screen_inventory`, `tui_real_walker.rs:1183-1207`) and asserts parity for a **single named pair** in `corpus_keyboard_and_mouse_paths_reach_the_same_command_and_local_endpoint` (`tui_real_walker.rs:6466`). It never sweeps every hit on every frame. **Gap.** |
| 5 | `fork_probe` + `drain_host_effects(pending)` + `assert_liveness` — clone host and session, drain any pending host effect, then confirm ≤32 Escape/`y` events return the UI to a modal-free Library or to quit | **partial** — `skit-tui-walker-support/src/engine_tests.rs:939 host_liveness_uses_a_fresh_prefix_replay_without_cloning_the_main_host` and `:1136 final_liveness_uses_a_separate_phase_without_changing_the_successful_prefix` cover the mechanism, and `tui_real_walker.rs:6813 canonical_corpus_preflight_returns_the_real_host_to_the_library` covers one end-to-end run — but the real engine runs liveness **once at the end**, not after every checkpoint |
| 6 | presented / not-presented cast recording | **yes** — `tui_real_walker.rs:2475`, `:5838`; `skit-tui-walker-support/src/contract_tests.rs:187` |

**Summary of section 4:** the two invariants the real-host owner does **not** run are
`invariants::check_state` (item 1) and `check_public_hit_parity` (item 4). Item 5 is run once
instead of continuously. Everything else already has a real owner. The randomization itself
(16 × 100 random operations × 15 locale/size profiles, with shrinking and a persisted seed file)
has **no equivalent** — the real walker is a deterministic 100-operation corpus over 4 profiles.

**Closed 2026-09-06 (MIG-F).** `cli::tui_real_random_walk::real_random_walk` replaces this walk. It
draws the same nine operation families from `skit_tui_walker_model::model` and runs them against
the real host.

---

## 5. MUST-MIGRATE (34 rows, 33 numbered items — plus the two infrastructure rows)

Ordered by regression risk. Each names a non-obvious product rule that no cited real-owner test
asserts today.

**Pointer and parity rules (frontend)**

1. `driver.rs:3183 typed_run_controls_compare_complete_mouse_and_keyboard_endpoints` — **the second
   click on an open run picker closes it, producing the exact same rendered endpoint as Escape**
   (`:3402-3409`); the *first* click on a non-current picker both focuses and opens it, which is
   strictly more than `Action::FocusField` alone (`:3268`); a picker `Down` only **arms** the anchor
   and changes nothing (`:3334`); a release away from the anchor cancels the arm (`:3350`); and a
   later release *on* the anchor after that cancellation is inert (`:3369`).
   `crates/skit-tui/tests/interactive_run_form.rs:585` proves the paths exist but asserts none of
   these five rules. High risk — commit `0edf38c fix(tui): restore reliable pointer interaction` is
   in this exact area.
2. `driver.rs:3622 first_preferences_frame_advertises_only_keys_the_focused_widget_releases` —
   **a shared footer chord must not become reachable through a Tab prefix while another widget owns
   it** (`:3688-3715`: the same chord fails with a 0-step focus budget and succeeds with 64), the
   first Preferences frame advertises only `Tab` for `FocusNext` (never renders `Tab/↓`), and with
   the Editor field focused `ManageAgents`/`InstallAgentSkill` have no binding, no hit and no
   rendered label.
3. `driver.rs:3142 run_footer_previous_focus_mouse_matches_the_keyboard_endpoint` — `FocusPrevious`
   footer-chip click and its advertised key reach the same endpoint **on the Run screen**;
   `render.rs:1396` covers the Library footer only.
4. `driver.rs:3031 local_dispatch_checks_the_persistent_session_once_against_its_descriptor` — a
   local action's **advertised descriptor must equal its live endpoint**; a forged descriptor is
   refused with `LOCAL_ACTION_ENDPOINT` and never quits.
5. `driver.rs:2937 raw_left_down_remains_one_pointer_event_and_keeps_its_armed_state` — a raw
   `LeftDown` is one event and leaves `top_level_click.fields.pressed` non-null; it is never
   discarded or auto-released.
6. `local_inventory.rs:216 registry_empty_local_contexts_export_live_key_and_mouse_semantics` —
   for Add, Health, Runners, the Runners editor and the standalone RunnerEditor: **every advertised
   local action has a mouse hit and at least one key, and both reach the identical
   `EventHandling`** — product rule 2, enforced per screen. Nothing else asserts this.
7. `local_inventory.rs:485 runner_local_surfaces_are_bounded_and_unambiguous_in_every_tier_and_locale`
   — across 5 runner surfaces × 4 locales (En/ZhCn/ZhTw/Pseudo) × 3 viewports (1×1, 24×6, 120×30):
   every local rect is non-empty and inside the viewport, **no two rects with different targets
   overlap**, and key and mouse agree.
8. `local_inventory.rs:263 local_inventory_is_empty_for_hidden_overlays_and_clipped_cells` — a
   1×1 Add screen advertises **no** local action (no invented rect), and opening the path picker
   (Ctrl+O) empties the inventory for the occluded base screen.
9. `local_inventory.rs:301 every_advertised_alias_is_a_positive_session_path` — when one local
   action advertises several chords (the runner editor's Tab/↓ and BackTab/↑), **every** alias
   reaches the same endpoint as the mouse hit.
10. `local_inventory.rs:329 preferences_shared_focus_hits_return_typed_session_actions` — every
    `FocusNext`/`FocusPrevious` hit on Preferences returns an `Action::Preferences(_)`, not a
    generic focus action.

**Invariants and the walk itself**

11. `driver.rs:3998 initial_invariant_failure_keeps_a_replay_header_and_subject` — plus the whole
    `invariants::check_state` checker (section 11 row 0). This is the single largest loss: ~40
    reducer-state invariants run at every transition boundary, none of which the real engine runs.
12. `invariants.rs:788 rejects_the_e29db41_draft_subject_regression_shape` — a **named commit
    regression guard**: an Add draft-delete confirmation must never have a null `delete_candidate`.
13. `driver.rs:2263 nightly_model_walk` + `driver.rs:2223 bounded_smoke_walk_checks_every_transition_boundary`
    — the randomized property walk itself (section 4). No equivalent.
14. `driver.rs:2234 complete_profile_guarantees_common_shapes_without_a_full_cartesian_product` —
    the profile matrix is **exactly 15 entries**: every locale at 1×1, 24×6 and 120×30, plus En
    80×24, Pseudo 120×12, ZhTw 40×40. The comment `"new shapes must not expand into every locale and
    size pair"` is a deliberate combinatorial budget. The real walker uses 4 profiles, so the tiny
    1×1 and compact 24×6 tiers across all four locales are lost.
15. `strategy.rs:527 resize_cases_include_tiny_responsive_and_large_viewports` — the 10 required
    resize shapes `(1,1) (1,2) (2,1) (24,6) (40,40) (46,12) (80,24) (120,12) (120,30) (300,100)`.
    The 24×6 and 1×2 / 2×1 degenerate tiers are the compact-confirmation and one-column layouts.
16. `strategy.rs:545 paste_cases_cover_terminal_text_edges` — the 7 required paste payloads:
    `""`, `"界"` (wide), `"e\u{301}"` (combining), `"🙂"` (astral), `"one\ntwo"` (newline),
    `"a\tb"` (tab), `"\0"` (NUL).
17. `driver.rs:2829 consumed_and_ignored_events_still_cross_a_render_boundary` — a `Consumed`
    session event still emits a changed frame, and opening a local overlay **refreshes** the local
    inventory to empty. (Its path-leak half is already owned by
    `tui_walker_bundle.rs:4096`.)

**Host and reducer rules**

18. `fake_host.rs:3414 fixtures_cover_each_required_host_surface` — the **analyzer / original-file
    kind table**: python, shell, fish, js, ts have an analyzer; powershell, ruby, perl, lua, r,
    command do not. Every kind except `command` has an original file and preserves it.
19. `fake_host.rs:4362 health_rebuild_uses_current_needs_runners_and_mirror_state` — **`MissingNeeds`
    outranks `LaunchBlocked`**: an entry with both a missing runner and a missing need reports
    exactly one issue, the `MissingNeeds` one (`:4404-4414`). Also: mirror master off with a
    configured URL reports `MirrorHealth::Paused`, not `Off`.
20. `fake_host.rs:4763 preset_schema_is_authoritative_and_empty_schema_refuses_without_writing` —
    a preset is filtered by the declaration schema **as of the save**, not as of the form open; if
    a parameter becomes secret between the two, its value is dropped and the previously-secret one
    is written. An entry with no declarations refuses a preset save without writing.
21. `fake_host.rs:4674 prompt_interpolation_controls_only_effective_run_and_detail_declarations` —
    interpolation off ⇒ no run fields, no detail parameters, and `OpenRunPresetSave` is a no-op
    (no modal); switching it back on restores the `value:TOPIC` field.
22. `fake_host.rs:4542 add_prompt_candidate_projection_keeps_only_unmanaged_placeholders` — with
    interpolation on, a *selected* candidate becomes managed and leaves the candidate list
    (`selected=true ⇒ []`); with interpolation off, **every** placeholder stays a candidate
    (`["TOPIC","AUDIENCE"]`).
23. `fake_host.rs:4620 add_python_candidate_projection_redetects_an_unmanaged_source_binding` —
    deselecting a review candidate for a Python source re-detects it as an unmanaged binding, so it
    reappears in `settings.candidates`.
24. `fake_host.rs:4457 reducer_run_hidden_keys_round_trip_for_each_entry_surface` — a run submit
    always carries `_skit_args`, `_skit_save_preset` and `_skit_dry_run`; a parameterised entry also
    carries `_skit_preset`; a prompt entry also carries `_skit_runner`.
25. `fake_host.rs:5420 reducer_glob_requests_use_the_same_virtual_root_and_production_counts` — the
    11-pattern count table (`*`→6, `*.py`→2, `***.py`→1, `none-*.zzz`→1, `[ab]lpha.py`→1,
    `[broken`→1, `**/*.py`→3, `.hidden*.py`→1, `nested/*.py`→1, `nested/.*.py`→1, `unicodé-?.rs`→1),
    plus: a literal makes **no** glob request, and an unbalanced quote makes **no** request.
    `crates/skit-store/tests/glob_expander.rs` covers the adapter but not this table.
26. `fake_host.rs:3652 ordered_add_effects_apply_prefix_mutations_and_return_at_the_terminal_effect`
    — an ordered `AddEffect` list applies its prefix and **returns at the first terminal effect**;
    the `Complete("must-not-run")` after a `Commit` never executes.
27. `fake_host.rs:5191 prompt_run_saves_picked_runner_and_extra_arguments_atomically` — a picked
    runner becomes `last_runner` **and seeds the runner of the next Add prompt review**
    (`:5276-5278`); extra args round-trip through the host argv dialect; an unbalanced quote refuses
    regardless of whether the runner is valid.
28. `fake_host.rs:4926 run_state_after_run_and_protocol_only_effects_are_honest` —
    `after_run = Exit` makes a run return `Action::Quit`, and `Effect::None`, `Effect::Quit` and
    `PreferencesEffect::None` must **never reach the host** (each is an error, with no mutation).
29. `fake_host.rs:5737 settings_reducer_uses_production_python_validation_and_rolls_back_secret_scrub`
    — an invalid PEP 508 requirement or PEP 440 specifier refuses the whole settings save
    **including the already-staged secret scrub**; a Python copy entry offers no `INTERPRETER_KEY`
    field at all.
30. `fake_host.rs:5792 command_settings_reconcile_body_placeholders_and_environment_riders` — a
    command template's `{PLACEHOLDER}`s become `ParameterDelivery::Placeholder` declarations **in
    body order** (`SECOND`, `TARGET`), and an added parameter becomes a `ParameterDelivery::Env`
    rider appended **last** (`EXTRA`).
31. `fixtures.rs:1190 settings_inputs_serializes_all_twenty_seven_non_default_fields_exactly` —
    pins the complete 27-field `SettingsInputs` shape; adding a field without updating the
    projection breaks it. Nothing else enumerates the full set.
32. `fake_host.rs:5489 preferences_reducer_tokens_resolve_presets_custom_urls_and_paused_state` —
    **turning the mirror master off keeps every configured URL while reporting `enabled = false`.**
    `crates/skit-cli/src/cli/tests.rs:9341` was body-verified: it covers the preset tokens, both nju
    derivations and the every-axis-`off` case (which *clears* the URLs), but never the master-off
    state that keeps them.
33. `fake_host.rs:5914 changed_defaults_do_not_reuse_an_exact_last_run_as_remembered_prefill` — a
    submitted value equal to the current default is **not** remembered, so a later default change
    changes the prefill. (`crates/skit-application/tests/form_state.rs:81` owns the pure function;
    the end-to-end settings→run consequence is unique here — migrate as a thin real-host test.)

**Closed 2026-09-06 (MIG-F).** Every item above has a real owner:

| items | owner |
|---|---|
| 1, 3, 5 | `crates/skit-tui-walker-model/tests/run_pointer_rules.rs` (`cd2380a8`) |
| 2 | `crates/skit-tui-walker-model/tests/preferences_focus_rules.rs` (`cd2380a8`) |
| 4 | the `LOCAL_ACTION_ENDPOINT` refusal in `skit_tui_walker_model::parity`, asserted by `crates/skit-cli/src/cli/tui_real_walker.rs a_forged_local_descriptor_refuses_with_the_named_endpoint_rule` |
| 6, 7, 8, 9, 10 | `crates/skit-tui-walker-model/tests/local_action_parity.rs` (`cd2380a8`) |
| 11 | `skit_tui_walker_model::invariants::check_state`, wired at `crates/skit-cli/src/cli/tui_real_walker.rs:708`, plus `crates/skit-cli/src/cli/tui_real_random_walk.rs a_broken_initial_state_fails_the_walk_and_keeps_a_diagnostic_frame` (`c022b825`) |
| 12 | `crates/skit-tui-walker-model/src/invariants_tests.rs:195 rejects_the_e29db41_draft_subject_regression_shape` (`cd2380a8`) |
| 13 | `cli::tui_real_random_walk::real_random_walk` (`c022b825`) |
| 14, 15, 16 | `random_walk_profiles()`, `RESIZE_CASES` and `PASTE_CASES` in `skit_tui_walker_model::model`, with their exact-set tests in `src/model_tests.rs` (`cd2380a8`) |
| 17 | `crates/skit-cli/src/cli/tui_real_walker.rs a_consumed_operation_records_a_session_checkpoint_and_empties_the_local_inventory` (`c022b825`) |
| 18 to 30, 32, 33 | `crates/skit-cli/src/cli/tests/legacy_walker_rules.rs` (`af195e85`) |
| 31 | `crates/skit-ui/src/settings.rs:2655 settings_inputs_names_all_twenty_seven_fields_the_screen_reads` |
| section 8 row 0 | `skit_tui_walker_model::parity`, wired at `crates/skit-cli/src/cli/tui_real_walker.rs:726` (`c022b825`) |
| section 11 row 0 | `skit_tui_walker_model::invariants`, wired at `crates/skit-cli/src/cli/tui_real_walker.rs:708` (`c022b825`) |

The 15-profile matrix and the tiny and compact tiers of item 14 run nightly only. `ui-walker-migration.md`
records that as a deliberate loss.

---
## 6. `crates/skit-tui/tests/model_walker/cast_projection.rs` (4 rows)

Thin wrapper over `skit_tui_walker_support::bundle::rebuild_presented_cast`; the fixture uses the
legacy `corpus::ObjectStore` but the code under test is the shared support crate.

| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 118 | `complete_unpresented_objects_stay_out_of_cast_and_accumulate_time` | a `NotPresented` row emits no cast output but its 0.1 s interval still advances, so the next presented frame lands at 0.2 s; the diagnostic frame object stays readable in the store | D | **yes** — `crates/skit-cli/src/cli/tui_real_walker.rs:5838 not_presented_distinct_frame_is_absent_and_its_interval_reaches_the_next_frame`; `crates/skit-tui-walker-support/src/bundle_tests.rs:486-501` (the loader must not even read a non-presented frame) | drop |
| 145 | `diagnostic_resize_does_not_replace_the_last_presented_cast_frame` | the header uses the maximum canvas (6×3), each row emits its own `"r"` resize event, and a diagnostic resize does not replace the previous presented frame (the next presented frame arrives at delta 0.0) | D | **partial** — `tui_real_walker.rs:7036 stable_corpus_grow_resize_uses_the_timeline_maximum_canvas` and `bundle_tests.rs:446 maximum_canvas_is_component_wise_and_order_independent` cover the canvas; the *ordering* of a diagnostic resize against the last presented frame is not asserted | add one case to `bundle_tests.rs` asserting the `["r",…]` sequence and the 0.0 delta around a `NotPresented` resize row |
| 169 | `identical_presented_frames_dedupe_across_diagnostic_rows` | two identical presented frames separated by a diagnostic row produce only 2 cast events — the diagnostic frame does not force a redraw | D | **partial** — `bundle_tests.rs:461 cast_canvas_and_rebuild_use_all_rows_and_only_presented_frames` counts events but not this dedupe-across-diagnostic case | fold into the same `bundle_tests.rs` case |
| 190 | `validator_rejects_a_valid_cast_event_spliced_outside_the_timeline` | a well-formed cast event appended after the rebuilt bytes is rejected — the cast must be exactly what the timeline produces | D | **yes** — `crates/skit-cli/src/cli/tui_walker_bundle.rs:3513 read_back_refuses_each_cast_corruption` | drop |

**Closed 2026-09-06 (MIG-F).** No row here is must-migrate. Every cited owner is outside
`model_walker/`, so all four rows are dropped. The two `partial` migration notes stay open as
`skit-tui-walker-support` work. They are not a MIG-F precondition.

---

## 7. `crates/skit-tui/tests/model_walker/corpus.rs` (15 rows)

The legacy artifact store, trace writer, corpus finalizer and validator. Its production counterpart
is `crates/skit-cli/src/cli/tui_walker_bundle.rs` (6014 lines, 81 tests) plus
`crates/skit-tui-walker-support` (13 661 lines, 155 tests).

| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 1252 | `object_store_writes_one_canonical_object_for_repeated_complete_state` | the same complete state written twice yields one `ObjectRef` and one file; reading it back returns the exact value | D | **yes** — `tui_walker_bundle.rs:3096 repeated_objects_are_written_once` | drop |
| 1266 | `frame_store_writes_a_small_readable_view_beside_the_complete_object` | a stored frame also writes a human-readable view naming size, cursor position/visibility and each row's text, and the complete object round-trips | D | **yes** — `crates/skit-tui-walker-support/src/bundle_tests.rs:258 readable_object_and_frame_views_match_the_legacy_bytes`; `:205 paths_cover_every_kind_and_the_frame_view_asymmetry` | drop |
| 1293 | `logical_completed_frame_does_not_keep_a_wide_cell_backend_ghost` | after a CJK frame is redrawn with shorter ASCII, `readable_lines()` shows only the new text — no wide-cell ghost | D | **yes** — `crates/skit-tui-walker-support/src/lib.rs:2511 real_walker_frame_hides_stale_ascii_behind_a_cjk_cell`; `:2344 readable_frame_keeps_hidden_cursor_diff_options_and_halfwidth_voicing` | drop |
| 1316 | `stored_trace_keeps_two_unchanged_checkpoints_and_validates_every_reference` | two consecutive checkpoints may reuse the same object refs without dropping a timeline row, and validation fails as soon as any referenced object file is gone | D | **yes** — `crates/skit-tui-walker-support/src/lib.rs:2629 unchanged_checkpoint_objects_can_repeat_without_dropping_a_timeline_row`; `tui_walker_bundle.rs:3585 read_back_refuses_object_and_view_corruption` | drop |
| 1432 | `trace_capture_records_all_five_complete_checkpoint_objects` | one checkpoint stores exactly five objects — reducer, host, session, styled frame, geometry | D | **yes** — `crates/skit-tui-walker-support/src/bundle_tests.rs:684 bundle_layout_derives_the_exact_tree_from_profiles_and_rows`; `tui_walker_bundle.rs:3096` | drop |
| 1501 | `profile_writer_preserves_every_row_and_emits_small_review_chunks` | the profile manifest's `row_count` matches the timeline line count, chunks are non-empty, and each chunk view names the checkpoint index, its `presentation=`, its `cause:` and a `readable: views/` pointer | D | **yes** — `crates/skit-tui-walker-support/src/bundle_tests.rs:354 chunk_views_pin_markdown_and_keep_first_appearance_per_cursor`; `:770 chunk_views_cover_reducer_host_and_fallback_cause_summaries` | drop |
| 1588 | `review_guide_distinguishes_diagnostic_state_from_terminal_presentations` | the reviewer guide tells the reader that a `not_presented` row is causal evidence and must not be reported as flicker or a visible defect | D | **yes** — `crates/skit-tui-walker-support/src/aggregate_contract_tests.rs:196 aggregate_contract_exposes_exact_root_names_and_legacy_review_bytes` pins the guide byte-for-byte at `:204` | drop |
| 1598 | `product_semantics_validate_terminal_effects_and_live_locale_changes` | a reducer transition must declare the effect the reducer actually emitted and must change state; a host row may not silently change the host object; an effect chain must terminate in `Effect::None`; the trace locale may change only at a matching `PreferencesSaved` host response, to that exact locale | D | **yes** — `tui_walker_bundle.rs:3666 read_back_refuses_unterminated_effect_and_live_locale_corruption`, `:3785`-`:3853` (the seven `live_locale_*` rows), `:4062 reducer_replay_covers_initial_and_session_boundaries`; `crates/skit-tui-walker-support/src/lib.rs:3669 timeline_locale_changes_only_from_a_matching_host_response` | drop |
| 1667 | `corpus_finalizer_rejects_an_incomplete_profile_set` | finalizing with fewer than four review profiles fails, and neither corpus validation nor review validation then succeeds | D | **yes** — `crates/skit-tui-walker-support/src/lib.rs:3961 manifest_requires_the_exact_four_nonempty_unique_profiles`; `tui_walker_bundle.rs:1057 required_profile_coverage_refuses_missing_extra_static_and_zero_mutations` | drop |
| 1683 | `corpus_validator_rejects_tampered_readable_evidence` | four independent tampering channels are refused: appended cast bytes, a rewritten frame view, a rewritten chunk markdown, and a host view with duplicate JSON keys | D | **yes** — `tui_walker_bundle.rs:3513 read_back_refuses_each_cast_corruption`, `:3585 read_back_refuses_object_and_view_corruption`, `:3364 read_back_refuses_profile_timeline_and_chunk_corruption` | drop |
| 1727 | `corpus_validator_rejects_false_summaries_and_noncanonical_timeline` | a coverage summary that claims no profiles is refused; a trailing newline in `timeline.ndjson` is refused; `run.json` written with non-canonical key order is refused (and the canonical writer itself refuses to emit it) | D | **yes** — `tui_walker_bundle.rs:3202 read_back_refuses_root_document_corruption`, `:3364`; `crates/skit-tui-walker-support/src/coverage_tests.rs:48 coverage_decoder_rejects_shape_schema_and_noncanonical_bytes`, `:177` | drop |
| 1767 | `corpus_validator_rejects_undeclared_files_and_directories` | any extra file or directory in the bundle fails with `"exact declared set"` — the tree is an allowlist, not a floor | D | **yes** — `tui_walker_bundle.rs:3897 allowlist_refuses_extra_and_missing_entries` | drop |
| 1788 | `completed_review_binds_an_authored_report_and_every_profile_verdict` | a review claim over the unedited template is refused; a report edited after the digest was recorded is refused; a genuinely authored report with every chunk digest and every profile verdict passes | D | **yes** — `crates/skit-tui-walker-support/src/aggregate_contract_tests.rs:638 completed_review_claim_rejects_pending_template_digest_and_newline_errors`, `:668 completed_review_claim_accepts_no_findings_and_findings_verdicts`; `crates/skit-tui-walker-support/src/lib.rs:4089`, `:4121` | drop |
| 1821 | `corpus_validator_rejects_symlinks_even_when_the_target_is_declared` | a symlink inside the bundle is refused even when its target is a declared file (`"contains a symlink"`) | C | **yes** — `tui_walker_bundle.rs:3922 tree_walk_refuses_symlinks_including_one_to_a_declared_file`, `:3943 tree_walk_refuses_a_non_regular_entry` | drop |
| 1837 | `coverage_names_string_and_object_tagged_workflow_screens` | the legacy coverage helper reads `workflow.active` whether it is a bare string or a single-key object | F | n/a — the real coverage summary is built from the typed effect vocabulary, not by reading serialized screen tags (`tui_walker_bundle.rs:444 effect_coverage_classifier_covers_all_outer_request_and_form_variants`, `:1187 complete_timeline_builds_one_valid_required_profile_coverage_summary`) | drop |

**Closed 2026-09-06 (MIG-F).** No row here is must-migrate. Every cited owner is outside
`model_walker/`, so all 15 rows are dropped.

---
## 8. `crates/skit-tui/tests/model_walker/driver.rs` (46 rows)

The walker engine, the property runner, the parity probes and the artifact writers.

### Row 0 — the parity and dispatch probes (infrastructure, **MUST-MIGRATE**)

`driver.rs` carries a second oracle beside `invariants::check_state`: a family of probe functions
that run at every checkpoint (`driver.rs:686-700`, gated by `probe_checks`) and inside every local
dispatch. Like section 11 row 0 they are *not* `#[test]`s, so they do not appear in the 46-row table
below, but they carry ~30 named codes and are migration priority 2 (section 14).

| probe | file:line | named codes |
|---|---|---|
| `dispatch_local_event` | `:369` | `LOCAL_ACTION_KEY`, `LOCAL_ACTION_RECT`, `LOCAL_ACTION_EVENT`, `LOCAL_ACTION_PRESS`, `SEMANTIC_CLICK_EVENT` |
| `finish_local_endpoint` | `:433` | `LOCAL_ACTION_ENDPOINT` — the advertised descriptor must equal the live endpoint |
| `resolved_operation_value` | `:826` | `PUBLIC_HIT_EVENT`, `LOCAL_HIT_EVENT` |
| `check_public_hit_parity` | `:965` | `PUBLIC_HIT_BOUNDS` (every visible hit is inside the viewport), `PUBLIC_HIT_CONTEXT`, `PUBLIC_SESSION_HIT` (a primary click yields the expected typed action) |
| `expected_hit_action` | `:1458` | `RUN_OPTION_HIT`, `RUN_TOKEN_OWNER`, `RUN_TOKEN_OPTIONS`, `RUN_TOKEN_EFFECT`, `RUN_TOKEN_END`, `RUN_TOKEN_FILE_OPTION`, `RUN_TOKEN_FILE_POSITION` |
| `check_field_hit_parity` | `:1162` | `FIELD_PARITY_TARGET`, `FIELD_PARITY_OWNER`, `FIELD_PARITY_INTERNAL`, `FIELD_SESSION_PARITY`, `FIELD_SESSION_ACTION`, `FIELD_SESSION_EFFECT`, `FIELD_SESSION_IGNORED`, `RUN_FIELD_FOCUS_EFFECT` |
| `is_plain_focus_field` / `check_keyboard_focus_path` | `:1302`, `:1312` | `FIELD_TEXT_FOCUS_STATE`, `FIELD_TEXT_FOCUS_HIDDEN`, `FIELD_FOCUS_KEY_PATH`, `FIELD_FOCUS_EFFECT`, `FOCUS_PARITY_INTERNAL` |
| `field_activation_sequences` | `:1363` | (no codes — enumerates the bounded Tab/BackTab prefixes a chip's keyboard twin may use) |
| `session_action` / `session_primary_click` | `:1502`, `:1518` | `SESSION_ACTION`, `SESSION_CLICK_PRESS` |
| `browse_keyboard_action` | `:1550` | `RUN_BROWSE_CAPABILITY`, `RUN_BROWSE_ENDPOINT`, `RUN_BROWSE_PARITY` |
| `command_key_action` / `check_command_key_endpoint` | `:1612`, `:1688` | `COMMAND_KEY_PATH`, `COMMAND_ENDPOINT`, `COMMAND_MOUSE_EFFECT` |

Support helpers that must move with them: `apply_probe_handling` (`:1402`),
`render_probe_endpoint` (`:1420`) and `render_probe_session` (`:1441`) — the full-frame
mouse-vs-keyboard endpoint comparison (frame **and** geometry) that rows 3183 and 3622 depend on.

**No real owner runs any of this.** `tui_real_walker.rs:1183 validate_screen_inventory` checks only
that screen targets are well-formed and present in `available`; the single parity assertion in the
real owner is `tui_real_walker.rs:6466 corpus_keyboard_and_mouse_paths_reach_the_same_command_and_local_endpoint`,
for one named `Health` pair. Nothing sweeps every hit on every frame.

**Migration note:** move the probe family into `crates/skit-cli/src/cli/tui_real_walker.rs` (or a
shared helper module) and call `check_public_hit_parity` from `CorpusFrontend::observe`
(`tui_real_walker.rs:1676`) next to `validate_styled_frame`. It needs `TuiSession::try_fork`
(`crates/skit-tui/src/session.rs:1666`) and `TuiSession::advertised_command_bindings` (`:1782`),
both already public and today used only by the surviving skit-tui suites.


| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 2223 | `bounded_smoke_walk_checks_every_transition_boundary` | 8 cases × ≤40 random operations at En 80×24 keep every checkpoint invariant, hit parity and liveness probe green | D | **none** — the real walker runs one deterministic 100-operation corpus, never a randomized vector | **MM.** Only survives if a randomized entry point is rebuilt on `WalkerEngine` + `CorpusFrontend`; see section 4 |
| 2234 | `complete_profile_guarantees_common_shapes_without_a_full_cartesian_product` | `COMPLETE_PROFILES` is exactly 15 entries: every locale at 1×1, 24×6, 120×30, plus En 80×24, Pseudo 120×12, ZhTw 40×40 — a deliberate budget against a full cartesian product | D | **none** — `tui_real_walker.rs:6677 canonical_corpus_has_exact_operation_count_and_required_profile_order` and `tui_walker_bundle.rs:1218 four_profile_coverage_merge_is_exact_checked_and_stored_comparable` pin **four** profiles, not this 15-shape matrix | **MM.** Port the matrix as a `required_corpus_factories()` extension, or record the loss of the 1×1 / 24×6 tiers in all four locales |
| 2263 | `nightly_model_walk` (`#[ignore]`) | the CI entry point: reads `SKIT_WALKER_CASES`/`STEPS`/`RECORD_SUCCESS`, refuses zero or a non-`0`/`1` value, runs the property profile over all 15 shapes | D | **none** | **MM.** Section 3 and 4 — this is the selector `ui-walker.yml` invokes |
| 2291 | `local_agent_review_walk` (`#[ignore]`) | generates a complete local review corpus for the four review profiles from `SKIT_WALKER_AGENT_STEPS` operations | F | replaced by `tui_real_walker.rs:3881 recorded_corpus_requires_exact_shared_profiles_and_values` + `tui_walker_bundle.rs:2846 installs_real_smoke_bundle_and_matches_exact_layout` | drop |
| 2303 | `validate_local_agent_review` (`#[ignore]`) | validates a completed local review and reports its source revision, optionally against `SKIT_WALKER_EXPECT_REVISION` | F | replaced by the bundle read-back path (`tui_walker_bundle.rs:3202`+) and `:4006 git_identity_and_directory_name_checks_refuse_mismatches` | drop |
| 2343 | `replay_saved_model_walk` (`#[ignore]`) | replays a saved `repro.json` minimal trace | F | none needed once the randomized walk is gone | drop with the walk |
| 2361 | `initial_state_is_checked_and_rendered_before_the_first_event` | the walker opens with exactly one `Initial` checkpoint and exactly one cast output frame — the initial state is validated and drawn before any event | D | **yes** — `crates/skit-tui-walker-support/src/engine_tests.rs:845 main_trace_keeps_session_reducer_and_host_boundaries_in_order`, `:1225 each_checkpoint_observes_the_frontend_and_host_exactly_once` | drop |
| 2369 | `agent_review_trace_keeps_the_user_effect_before_the_host_response` | one advertised-key operation produces 4 rows: Initial, UserAction carrying the emitted `reload` effect, HostAction carrying the host response, then a FinalLiveness row with `LivenessResult::Passed` | D | **yes** — `engine_tests.rs:845`; `tui_real_walker.rs:4627 real_host_engine_records_and_replays_one_production_open` | drop |
| 2445 | `add_cancel_keeps_the_in_flight_frame_as_unpresented_causal_evidence` | cancelling Add yields Presented → NotPresented → Presented for one operation; the in-flight diagnostic frame object differs from the settled one and is kept in the timeline; the cast still shows only 3 output frames | D | **yes** — `crates/skit-tui-walker-support/src/contract_tests.rs:187 presentation_classifier_follows_the_complete_transition_table`; `tui_real_walker.rs:5838` | drop |
| 2509 | `nonreview_cast_keeps_in_flight_diagnostic_frames` | without a review capture the recorder emits every frame (4 for open+cancel), including in-flight ones | F | none — the real owner always records a review trace, so the non-review recorder mode does not exist | drop |
| 2519 | `public_click_trace_uses_two_dispatch_chains_for_one_operation` | one public-hit operation records two event chains (sequence 0 and 1) whose events are `Down` then `Up`, sharing one resolution; the final liveness row starts its own chain | D | **yes** — `crates/skit-tui-walker-support/src/lib.rs:3170 timeline_accepts_two_dispatch_chains_for_one_click_operation`, `:2916 click_plan_validation_keeps_its_exact_existing_contract` | drop |
| 2581 | `local_click_trace_uses_two_dispatch_chains_for_one_operation` | the same press/release chain rule for a **local** action hit | D | **yes** — same as above; `tui_real_walker.rs:6466` exercises `LocalHit` end to end | drop |
| 2635 | `noop_and_final_already_terminal_each_use_one_synthetic_dispatch` | an unresolvable operation records one synthetic `{"synthetic":"not_applicable"}` dispatch under its own chain, and a walk already at the Library records `{"synthetic":"already_terminal"}` for its liveness chain | D | **yes** — `crates/skit-tui-walker-support/src/lib.rs:3292 timeline_semantics_requires_the_noop_synthetic_event`, `:2657 final_liveness_rows_are_kept_without_impersonating_vector_operations` | drop |
| 2684 | `raw_resize_and_paste_each_use_one_exact_dispatch_chain` | a raw key, a resize and a paste each use exactly one chain with ascending sequence, and the resize's resolution names its exact `resize` event | D | **yes** — `tui_real_walker.rs:6953 diagnostic_stable_corpus_resize_replays_backend_terminal_event_and_viewport_bytes`; `crates/skit-tui-walker-support/src/lib.rs:2840`, `:2870` | drop |
| 2741 | `agent_review_trace_records_the_live_locale_after_preferences_save` | the row **before** the `PreferencesSaved` host response is still `en`; that row and every later row are `zh-TW` | D | **yes** — `tui_real_walker.rs:4391 real_frontend_switches_locale_only_when_it_consumes_preferences_saved`, `:4517 real_preferences_operation_switches_the_first_host_frame_and_replays_exactly` | drop |
| 2796 | `local_agent_review_vector_is_explicit_repeatable_and_exact_length` | the 100-operation review vector is deterministic, exactly 100 long, and byte-stable through JSON | D | **yes** — `tui_real_walker.rs:6677 canonical_corpus_has_exact_operation_count_and_required_profile_order`, `:7702 corpus_operations_have_stable_source_owned_shapes` | drop |
| 2808 | `local_agent_review_rejects_a_source_tree_change_during_generation` | the source fingerprint is taken before and after; if it changed, generation aborts and installs **nothing** (no `review-*` directory) | D | **yes** — `tui_walker_bundle.rs:4006 git_identity_and_directory_name_checks_refuse_mismatches`, `:3956 changed_fingerprint_removes_every_staged_and_installed_directory` | drop |
| 2829 | `consumed_and_ignored_events_still_cross_a_render_boundary` | a `Consumed` session event still adds a Session checkpoint and emits a changed cast frame; opening the Add path picker (Ctrl+O) empties the local inventory; the cast contains `/fixtures` but never the checkout path | D | **partial** — the path-leak half is `tui_walker_bundle.rs:4096 c3_leak_oracle_refuses_raw_root_in_every_zero_tolerance_channel`; the "every consumed event renders" and "the inventory refreshes for the occluded base screen" halves have no owner | **MM.** Add a `tui_real_walker.rs` case asserting a `Consumed` corpus operation still produces a Session checkpoint whose local inventory is empty for an open overlay |
| 2876 | `semantic_public_hits_dispatch_a_press_and_matching_release` | one `PublicHit` on Help adds exactly two checkpoints (Session for the press, UserAction for the release-borne action) and opens the Help modal | D | **yes** — `crates/skit-tui-walker-support/src/lib.rs:2916`, `:3170` | drop |
| 2907 | `agent_review_resolved_public_hit_names_both_pointer_events` | the recorded resolution declares `input.kind == "primary_click"` and lists both the `Down` and the `Up` event | D | **yes** — `tui_real_walker.rs:7570 corpus_screen_operations_and_resolutions_have_exact_outer_shapes` (`primary_click` shape) | drop |
| 2937 | `raw_left_down_remains_one_pointer_event_and_keeps_its_armed_state` | a raw `LeftDown` over a Help chip adds one checkpoint, opens no modal, and leaves `session.top_level_click.fields.pressed` non-null — it is neither discarded nor auto-released | D | **partial** — `crates/skit-tui/tests/interactive_run_form.rs:730 a_global_footer_click_cancels_an_armed_run_control` and `:754` cover top-level arming for content targets, not the raw-Down-only case | **done** — `crates/skit-tui-walker-model/tests/run_pointer_rules.rs` `a_lone_primary_press_arms_a_footer_chip_without_activating_it` |
| 3014 | `filtered_quit_operations_do_not_discard_the_trace_suffix` | a quit-filtered operation is still a checkpoint and does not set `quit`, so the walk keeps consuming its remaining operations | D | **yes** — `tui_real_walker.rs:6494 corpus_resolver_records_typed_refusals_and_real_input_plans` (`QuitFiltered`, `EventWouldQuit`), `:6728 corpus_refusal_sink_records_a_real_not_applicable_operation` | drop |
| 3031 | `local_dispatch_checks_the_persistent_session_once_against_its_descriptor` | dispatching a local key adds one checkpoint and refreshes the inventory; a **forged** descriptor whose `outcome` claims `Action::Quit` is rejected with `LOCAL_ACTION_ENDPOINT` and never quits | D | **partial** — `tui_real_walker.rs:6747 local_keyboard_uses_the_exact_key_when_one_add_target_has_two_chords` refuses an ambiguous duplicate but never compares descriptor to live endpoint | **MM.** Add the descriptor-vs-endpoint check to `resolve_corpus_operation_with_local_actions` and cover it in `tui_real_walker.rs` |
| 3071 | `local_hit_dispatches_the_advertised_press_release_endpoint` | a local hit dispatches press **and** release (2 checkpoints) and reaches the same endpoint as the key path | D | **yes** — `tui_real_walker.rs:6466 corpus_keyboard_and_mouse_paths_reach_the_same_command_and_local_endpoint` | drop |
| 3102 | `in_flight_effect_state_is_checked_before_the_fake_host_answers` | the last checkpoint with a pending host effect is a `UserAction` boundary whose Add stage is still `ConfirmDraftDelete` **with** its candidate; the settled `HostAction` checkpoint is back at `Source` | D | **yes** — `crates/skit-cli/src/cli/tui_real_host.rs:20891 checkpoint_capture_projects_the_real_add_delete_chain_in_every_effect_position`; `engine_tests.rs:845` | drop |
| 3129 | `resize_changes_the_backend_before_the_resize_event_is_rendered` | the backend is resized before the `Resize` event is dispatched, so the checkpoint records the new size | D | **yes** — `tui_real_walker.rs:6953 diagnostic_stable_corpus_resize_replays_backend_terminal_event_and_viewport_bytes` | drop |
| 3142 | `run_footer_previous_focus_mouse_matches_the_keyboard_endpoint` | on the **Run** screen, the `FocusPrevious` footer chip's click and its advertised key reach the same endpoint | E | **partial** — `crates/skit-tui/tests/render.rs:1396 every_library_footer_action_has_the_expected_mouse_mapping` covers the Library footer only | **done** — `crates/skit-tui-walker-model/tests/run_pointer_rules.rs` `the_run_previous_focus_chip_reaches_its_advertised_key_endpoint` |
| 3183 | `typed_run_controls_compare_complete_mouse_and_keyboard_endpoints` | checkbox and radio-option hits exist and are typed; the first click on a non-current picker focuses **and** opens (strictly more than `FocusField`); **the second click closes the dropdown and produces the same rendered endpoint as Escape**; a picker `Down` only arms; a release away cancels; a stale release on the anchor after cancellation is inert; whole-frame `check_public_hit_parity` holds before and after | E | **partial** — `interactive_run_form.rs:585 checkbox_radio_and_picker_have_keyboard_and_mouse_paths` proves the paths exist but asserts none of the five arm/close rules | **done** — `crates/skit-tui-walker-model/tests/run_pointer_rules.rs` `typed_run_picker_pointer_rules_match_their_keyboard_endpoints`, with both whole-frame `check_public_hit_parity` sweeps through `skit_tui_walker_model::parity` |
| 3414 | `direct_text_focus_accepts_a_keyboard_path_with_different_scroll_history` | reaching a Run text field by keyboard after a resize keeps a positive `FocusField(2)` hit and a non-zero `first_visible` — a scrolled form still publishes its focused row | D | **partial** — `interactive_run_form.rs:1630 focus_auto_scrolls_a_long_form_and_wheel_scroll_uses_the_shared_viewport`, `:1672 resize_only_keeps_the_focused_run_control_visible` | **done** — `crates/skit-tui-walker-model/tests/run_pointer_rules.rs` `a_scrolled_run_form_keeps_a_positive_hit_for_its_focused_row` |
| 3487 | `picker_focus_parity_uses_one_post_navigation_session` | after search → hit → `FocusPrevious` → wheel scroll, focus stays on field 0, `first_visible` becomes 1, and field 0 still has a positive hit | D | **partial** — same as above | **done** — folded into row 3414's test in `crates/skit-tui-walker-model/tests/run_pointer_rules.rs` |
| 3543 | `runner_editor_modal_does_not_publish_blocked_base_screen_hits` | an open runner-editor modal publishes **no** base-screen geometry hits at all while keeping its own local actions | E | **yes** — `crates/skit-tui/tests/overlay_pointer_priority.rs:409 runner_editor_blocks_the_underlying_run_field_command_hit` | drop |
| 3575 | `run_dropdown_does_not_publish_occluded_field_command_hits` | a shrunk regression trace: an open run dropdown must not publish occluded field-command hits, and every checkpoint still passes its liveness probe | D | **partial** — `overlay_pointer_priority.rs:409` covers the occlusion rule; the specific shrunk trace is a walker artefact | drop the trace; the rule is owned |
| 3596 | `preferences_agent_picker_does_not_publish_blocked_footer_hits` | with the agent-skill picker open, `SavePreferences` and `ClosePreferences` publish no hits | E | **yes** — `overlay_pointer_priority.rs:350 agent_picker_blocks_global_footer_but_keeps_its_owner` | drop |
| 3622 | `first_preferences_frame_advertises_only_keys_the_focused_widget_releases` | the first Preferences frame advertises only `Tab` for `FocusNext` and never renders `Tab/↓`; with the Editor field focused, `ManageAgents` and `InstallAgentSkill` have no binding, no hit and no rendered label; **their chord must not be reachable through a Tab prefix** (fails at 0 focus steps, succeeds at 64) | E | **partial** — `crates/skit-tui/tests/render.rs:1050 contextual_footer_only_advertises_commands_that_can_run_here` covers advertisement but not the Tab-prefix laundering rule | **done** — `crates/skit-tui-walker-model/tests/preferences_focus_rules.rs` `the_first_preferences_frame_advertises_only_keys_the_focused_widget_releases` |
| 3719 | `persistent_preferences_controls_do_not_occlude_the_shared_footer` | a shrunk regression trace: a focused persistent Preferences control must not occlude the shared footer; every checkpoint passes liveness | D | **partial** — `overlay_pointer_priority.rs` covers overlay occlusion; the persistent-control case rides on `check_public_hit_parity` | drop the trace; migrate the rule with row 3622 |
| 3739 | `liveness_uses_real_session_events_to_leave_a_dirty_workflow` | from a dirty Preferences screen, real `Esc` then `y` events return the UI to a modal-free Library — the liveness probe uses real events, not synthetic state resets | D | **yes** — `tui_real_walker.rs:6813 canonical_corpus_preflight_returns_the_real_host_to_the_library`; `engine_tests.rs:1136 final_liveness_uses_a_separate_phase_without_changing_the_successful_prefix` | drop |
| 3753 | `host_effect_cycles_fail_after_every_intermediate_boundary_is_checked` | a looping host is stopped after exactly 64 host actions (`"exceeded 64 actions"`), and every one of the 64 `HostAction` checkpoints was recorded with `pending_effect` set | C | **yes** — body-verified `crates/skit-tui-walker-support/src/engine_tests.rs:1120 host_effect_cycles_stop_at_the_engine_limit` (parameterised, exercised at limit 2). **Note a harness divergence, not a contract loss:** the legacy constant is `driver.rs:57 HOST_EFFECT_LIMIT = 64`; the real walker uses `tui_real_walker.rs:60 EFFECT_LIMIT = 16` | drop |
| 3788 | `exactly_the_host_effect_limit_may_settle_on_the_final_action` | a chain that settles on exactly the 64th action succeeds | C | **yes** — body-verified `engine_tests.rs:829 effect_chain_can_settle_exactly_at_the_limit` (same divergence note as row 3753) | drop |
| 3813 | `missing_effect_selectors_fail_before_the_fake_host_can_turn_them_into_status` | the fake's own sanity gate rejects an effect naming an absent selector before serving it | F | n/a — a fake-only guard; the real host resolves selectors through the production store | drop |
| 3824 | `quit_is_a_terminal_effect_and_never_reaches_the_host` | `Effect::Quit` sets the quit flag at a `UserAction` boundary and is never served to the host | D | **yes** — `engine_tests.rs:805 local_and_exact_limit_host_quit_are_successful_terminal_operations`; `tui_real_walker.rs:4339` (`effect_route(&Effect::Quit) == EffectRoute::Quit`) | drop |
| 3847 | `canvas_size_keeps_the_component_maximum_across_resize_order` | the replay canvas is the component-wise maximum and is order-independent; **the legacy resolver clamps a zero dimension to 1 yet the canvas still grows to that resize's other dimension** | D | **yes for the maximum** — `crates/skit-tui-walker-support/src/bundle_tests.rs:446 maximum_canvas_is_component_wise_and_order_independent`. **Deliberate divergence for zero:** `tui_real_walker.rs:7064 corpus_canvas_forecast_ignores_each_zero_dimension_resize` and `:7090` say the real corpus **ignores** a zero-dimension resize entirely | drop — the real behaviour is intentionally different and already owned |
| 3908 | `cast_header_uses_the_maximum_replay_canvas` | the cast header names the maximum canvas (300×100), not the initial viewport | D | **yes** — `tui_real_walker.rs:7036 stable_corpus_grow_resize_uses_the_timeline_maximum_canvas` | drop |
| 3929 | `failure_artifacts_keep_each_complete_bundle_separate` | each failure writes its own `failure-*/{failure.cast,repro.json}` bundle atomically (staged then renamed), and two failures never share one | F | **none** — the real owner installs `bundle-<digest>` directories into a caller-supplied parent, not `failure-*` under `target/ui-walker-artifacts` | drop with the walk; **but see section 3** — this is what `ui-walker.yml` globs |
| 3964 | `requested_success_artifact_contains_the_replay_and_cast_in_one_bundle` | a requested success writes `success-*/{success.cast,repro.json}` with `result: "passed"` | F | **none** — same | same as above |
| 3998 | `initial_invariant_failure_keeps_a_replay_header_and_subject` | an initial state that violates `LIBRARY_VISIBLE_ORDER` fails the walk **and** still produces a valid v3 cast header at the requested 1×1 size plus one non-empty diagnostic frame | D | **none** — the real engine has no `LibraryState` invariant, so this failure mode cannot arise there | **MM.** Depends on migrating `check_state` (section 11 row 0) into `RealFrontend::observe_frontend` |
| 4043 | `initial_host_panic_keeps_a_replay_header` | a host that panics during initialization is converted to a captured failure with a valid cast header, not a lost test | C | **partial** — `engine_tests.rs:491 into_parts_preserves_poisoned_adapter_state_after_the_primary_error` covers poisoning, not `catch_unwind` around a panicking host | drop unless a randomized entry point is rebuilt |

**Closed 2026-09-06 (MIG-F).** Row 0's probe family is `skit_tui_walker_model::parity`, wired at
`crates/skit-cli/src/cli/tui_real_walker.rs:726`. The ten must-migrate rows landed as section 5
lists. Every other cited owner is outside `model_walker/`, so those rows are dropped.

---
## 9. `crates/skit-tui/tests/model_walker/fake_host.rs` (62 rows)

`FakeHost` re-implements the whole host (entry CRUD, runner CAS, preferences, settings, the Add
workflow, drafts, presets, health) over an in-memory `HostSnapshot`. Its primary real owner is
**`crates/skit-cli/src/cli/tests.rs`** (12 272 lines, 182 tests over real `TempDir` stores,
`FileStore`, `LibraryService`, `FileConfigStore` and the real `tui_effect` seam), with
`tui_host.rs`, `tui_real_host.rs`, `crates/skit-store/tests/` and `crates/skit-application/tests/`
carrying the rest.

Note: several of these tests already run **production** logic through the fake — `skit_language::{detect_candidates, managed_params, placeholder_params, validate_pep440_specifiers, validate_pep508_requirement}`, `skit_application::runner_management::{split_editable_argv, join_editable_argv, validate_runner_argv}` and `skit_application::form_state::remembered_values` (`fake_host.rs:17-27`). Those parts already have real owners in `skit-application`/`skit-language`.

### 9a. Fake-snapshot projection contracts (rows 1-12) — classification F unless noted

| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 2563 | `agent_review_snapshot_is_complete_and_byte_stable_across_json_round_trips` | the fake's snapshot has exactly 18 named keys in sorted order and is byte-stable across a JSON round trip and across two fresh hosts | F | conceptual — `crates/skit-cli/src/cli/tui_real_host.rs:14635 observations_keep_full_ui_state_unknown_config_and_port_transcripts` | drop |
| 2604 | `agent_review_snapshot_changes_after_a_reachable_run_mutation` | a reachable run changes `last_runs`, `remembered_values` and `extra_args` in the snapshot | F | `tui_real_host.rs:14635`; `crates/skit-cli/src/cli/tests.rs:1144 tui_run_forms_preserve_saved_values_but_never_prefill_secrets` | drop |
| 2640 | `agent_review_snapshot_checks_only_typed_paths_and_non_finite_numbers` | a checkout path inside a **typed path field** (settings source, virtual file, saved `shell.bash_path`) is refused, but the same text inside a free-text description is allowed; a non-finite number is refused | C | **yes** — `crates/skit-cli/src/cli/tui_walker_bundle.rs:4096 c3_leak_oracle_refuses_raw_root_in_every_zero_tolerance_channel`, `:4489 c3_structural_checks_preserve_identity_text_and_unrelated_numbers`, `:4159 c3_run_metadata_masks_only_its_typed_root_fields` | drop |
| 2689 | `native_unix_paths_keep_backslash_filenames_distinct_from_child_paths` | `/fixtures/name\part` and `/fixtures/name/part` project to distinct sorted keys; `./`, `//` and `../` spellings are refused | F | **yes** — `tui_real_host.rs:18603 c2_session_native_paths_project_only_canonical_registered_objects`, `:23125 tree_observation_escapes_non_utf8_paths_without_dropping_bytes` | drop |
| 2716 | `every_explicit_path_string_has_the_same_posix_and_windows_projection` | the whole snapshot is byte-identical whether every path is spelled POSIX or `C:\`-Windows | F | **yes** — `tui_real_host.rs:18802 c2_windows_session_native_paths_project_only_canonical_registered_objects`; `tui_walker_bundle.rs:4871 c3_windows_roots_use_the_declared_separator` | drop |
| 2773 | `source_path_map_sorts_by_canonical_key_instead_of_host_pathbuf_order` | the source map sorts by canonical key (`/fixtures/a/first` before `/fixtures/a\first`), not by host `PathBuf` order | F | **partial** — `tui_real_host.rs:18603` | drop |
| 2800 | `canonical_paths_preserve_non_utf8_virtual_path_bytes` | a non-UTF-8 path projects as `{"unix_bytes": [...]}` with no byte loss | F | **yes** — `tui_real_host.rs:23125 tree_observation_escapes_non_utf8_paths_without_dropping_bytes`, `:18664 c2_real_non_utf_file_picker_entry_projects_its_lossy_name_and_native_path` | drop |
| 2815 | `every_host_snapshot_field_has_an_independent_json_contract` | each of the 18 snapshot fields has an independent JSON pointer — changing one changes only that subtree | F | n/a — a fake-serializer contract | drop |
| 2894 | `every_entry_fixture_field_has_an_independent_json_contract` | the 7 `EntryFixture` fields are independent | F | n/a | drop |
| 2945 | `entry_summary_and_detail_manual_projection_fields_have_value_oracles` | 17 manually-projected summary/detail pointers each carry the exact expected value | F | n/a | drop |
| 3135 | `draft_source_and_agent_manual_projection_fields_have_value_oracles` | draft, source and agent-target pointers each carry the exact expected value | F | n/a | drop |
| 3248 | `preferences_health_and_runner_facts_have_value_oracles` | preferences, health and runner-row pointers each carry the exact expected value | F | n/a | drop |

### 9b. Fixture-consistency contracts

| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 3406 | `portable_fake_inputs_follow_both_host_contracts` | a virtual workdir is absolute on both platforms; `split_editable_argv("\"unfinished")` errors in **both** POSIX and Windows dialects | B | **yes** — `crates/skit-cli/src/cli/tests.rs:6959 windows_argument_splitting_covers_padding_escapes_and_unclosed_quotes`; `crates/skit-application/tests/runner_management.rs` | drop |
| 3414 | `fixtures_cover_each_required_host_surface` | **the analyzer / original-file kind table**: python, shell, fish, js, ts have an analyzer; powershell, ruby, perl, lua, r, command do not. Every kind except `command` has an original file and preserves it. Plus: an empty command entry has a declared schema and no declarations | A | **partial** — `tests.rs:2165 tui_settings_offers_a_declared_row_exactly_the_axes_version_04_makes_editable`, `:7037 tui_settings_refuse_axes_that_do_not_apply_to_the_entry_kind` cover per-kind axes but not this analyzer/original-file table | **MM.** Port the kind table to `tests.rs` (or to `skit-language`, which owns `has_analyzer`) as an exhaustive per-kind assertion |
| 3498 | `preserved_sources_and_found_tools_exist_in_the_same_virtual_snapshot` | every preserved original and the discovered `uv` path exist in the same virtual file tree the picker walks | F | n/a — a fixture-consistency guard; the real host reads a real `TempDir` tree (`tui_real_host.rs:22750 g2c_file_picker_tree_is_exact_idempotent_and_external_only`) | drop |

### 9c. Host effect contracts

| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 3551 | `opens_every_screen_from_the_current_model` | every `HostRequest` maps to its screen — notably `Presets` → Settings and `Rename` → Form | A | **yes** — `crates/skit-cli/src/cli/tests.rs:3690 tui_host_opens_every_frontend_neutral_screen_and_handles_simple_effects` | drop |
| 3589 | `invalid_selector_and_request_pairs_do_not_mutate_the_model` | `Run` with a missing selector and `Add` with a selector both fail without any mutation | A | **yes** — `tests.rs:3690`; `:924 adapter_only_error_paths_do_not_require_process_global_configuration` | drop |
| 3611 | `remove_and_draft_delete_commit_only_after_exact_validation` | removing a missing selector is a typed status with no mutation; a draft delete removes it once, then reports `Err` for the second attempt with no further change; a valid remove really removes | A | **yes** — `tests.rs:1615 tui_add_maps_every_draft_cleanup_outcome_without_rolling_back_the_entry`; `:10133 owned_draft_consume_is_idempotent_and_requires_the_exact_snapshot` | drop |
| 3652 | `ordered_add_effects_apply_prefix_mutations_and_return_at_the_terminal_effect` | an ordered `AddEffect` list applies its prefix (RememberRunner, DraftKept, Commit) and **returns at the first terminal effect** — the trailing `Complete("must-not-run")` never executes | A | **partial** — `tests.rs:1390 tui_add_host_reads_exact_source_and_commits_through_the_reducer_seam` covers the commit, not the terminal-effect stop | **MM.** Add a `tests.rs` case dispatching a multi-effect `UiEffect::Add` with a trailing effect that must not run |
| 3678 | `stale_runner_cas_targets_refuse_without_mutation` | a `Named` save or remove whose `expected` identities or `expected_pinned_count` are stale returns `MutationFailed` with **no** mutation | A | **yes** — `tests.rs:3952 tui_runner_host_preserves_editor_input_on_stale_rows_and_rechecks_prompt_pins` (identical stale-token and pinned-count CAS over `FileConfigStore`) | drop |
| 3713 | `valid_runner_save_and_remove_update_every_projection` | a valid CAS save updates argv, and a valid remove drops the row **and** its name from the preferences snapshot | A | **yes** — `tests.rs:3952`, `:4069`; `crates/skit-store/tests/runner_management_cas.rs` | drop |
| 3769 | `preset_and_preferences_saves_persist_real_model_values` | a preset save drops secret-named values; a preferences save persists language and editor | A | **yes** — `tests.rs:4388 typed_run_feedback_and_preset_effects_use_application_and_store_ports`; `:4218 typed_preferences_effects_validate_atomically_and_install_only_after_selection` | drop |
| 3800 | `generic_submit_rejects_wrong_purpose_without_mutation` | submitting with the wrong `FormPurpose` for the selector errors with no mutation | A | **yes** — `tests.rs:4911 tui_host_submits_every_form_without_global_process_state` | drop |
| 3815 | `add_and_runner_refusals_return_to_the_exact_reducer_owner` | a source-inspection refusal echoes its exact request id and leaves the Add workflow's `problem()` set; a duplicate runner save keeps the manager editor open with `host_error()` set — no owner change, no mutation | A | **yes** — `tests.rs:4069 tui_runner_host_routes_success_to_the_exact_standalone_editor_owner`; `tui_real_host.rs:19863 c2_runner_failure_status_provenance_replays_and_success_values_stay_user_owned` | drop |
| 3898 | `settings_validate_the_complete_transaction_before_commit` | a 19-key settings save applies every axis at once (name, workdir, dependencies, python, needs, manage/unmanage, normalize, add/remove parameter, per-parameter type/choices/default/required/prompt/help, preset removal) and the reopened form shows the saved values; a later save carrying one unknown key rolls back the **whole** transaction | A | **yes** — `tests.rs:9762 a_python_settings_save_that_moves_one_control_leaves_every_other_axis_alone`; `:6035 tui_settings_source_refusal_does_not_commit_other_fields` | drop |
| 4043 | `draft_delete_checks_identity_content_and_request_before_mutation` | four typed outcomes: `Removed` for an exact match; `AlreadyMissing` when the source is gone; `Changed(refreshed)` returning a refreshed row when bytes/identity moved (file kept); `Err` with no mutation when the claim has no identity | A | **yes** — `tests.rs:1615 tui_add_maps_every_draft_cleanup_outcome_without_rolling_back_the_entry`; `:1722 tui_delete_changed_outcome_returns_a_refreshed_draft_claim`; `:10431 unsupported_platform_keeps_owned_drafts_without_an_identity` | drop |
| 4124 | `add_cleanup_failure_keeps_the_committed_entry_and_draft` | a `ConsumeDraft` that fails after the entry committed still completes with a `warning:` receipt; the entry exists, the draft survives, and `original_file_preserved` is true | A | **yes** — `tests.rs:1547 tui_add_cleanup_refusals_keep_the_committed_entry_and_the_replacement`; `:10257 cli_cleanup_receipts_keep_the_committed_entry_and_changed_draft_in_every_locale` | drop |
| 4174 | `add_cleanup_success_recomputes_the_original_source_projection` | a successful consume removes the draft source and flips `original_file_preserved` to false while `has_original_file` stays true | A | **yes** — `tests.rs:1615`; `:10151 owned_draft_consume_keeps_in_place_content_and_permission_changes` | drop |
| 4210 | `picker_tree_tracks_author_delete_consume_and_refused_cleanup` | the picker tree gains an authored draft, loses it on delete, loses a consumed draft, and is **unchanged** when a consume is refused for changed bytes | A | **yes** — `tui_real_host.rs:22750 g2c_file_picker_tree_is_exact_idempotent_and_external_only`, `:22893 g2c_file_picker_tree_drives_the_real_add_picker_with_relative_identity`; `tests.rs:1947 tui_add_authoring_uses_a_real_owned_draft_and_discards_only_unchanged_starters` | drop |
| 4289 | `dynamic_sources_drive_glob_counts_from_the_same_virtual_snapshot` | a glob count over the drafts directory tracks authored and deleted drafts live | A | **yes** — `tests.rs:4388 typed_run_feedback_and_preset_effects_use_application_and_store_ports` | drop |
| 4328 | `add_allocates_slug_suffixes_but_refuses_exact_display_name_duplicates` | committing `"Python tool"` when it exists is refused with no mutation; committing `"Python-tool"` succeeds and allocates `python-tool-2` | A | **yes** — `crates/skit-store/tests/mutation_refusals.rs:127 colliding_slug_bases_receive_deterministic_numeric_suffixes`, `:48 same_slug_renames_succeed_while_name_conflicts_and_blank_names_refuse`, `:288 an_unindexed_legacy_name_still_blocks_a_duplicate_create` (production at `crates/skit-store/src/mutations.rs:952 ensure_name_available`, `:969 allocate_slug`) | drop |
| 4362 | `health_rebuild_uses_current_needs_runners_and_mirror_state` | clearing `needs` clears the `MissingNeeds` issue; a missing runner raises `LaunchBlocked`; **an entry with both reports exactly one issue and it is `MissingNeeds`**; mirror master off with a configured URL reports `MirrorHealth::Paused` | A | **partial** — `tests.rs:3469 shared_health_inspector_reports_typed_entry_runner_and_rebuild_facts` covers the typed facts but not the one-issue precedence | **MM.** Add the precedence and the paused-mirror case to `tests.rs:3469` |
| 4436 | `run_refuses_unknown_parameter_and_reserved_keys_without_mutation` | an unknown `value:` key, an unknown `_skit_` key, and a non-boolean `_skit_dry_run` are each a typed status with no mutation | A | **yes** — `tests.rs:4911 tui_host_submits_every_form_without_global_process_state` | drop |
| 4457 | `reducer_run_hidden_keys_round_trip_for_each_entry_surface` | a run submit always carries `_skit_args`, `_skit_save_preset` and `_skit_dry_run`; a parameterised entry also carries `_skit_preset`; a prompt entry also carries `_skit_runner`; the run then marks the entry rerunnable | E | **partial** — `tests.rs:1144 tui_run_forms_preserve_saved_values_but_never_prefill_secrets`, `:1222 plain_run_form_projects_prompts_saved_values_and_runner_defaults` | **MM.** Add an exhaustive hidden-key-set assertion to `crates/skit-ui` (`RunFormView::submit`) or `tests.rs:1144` |
| 4506 | `entry_mutations_refresh_runner_pins_without_rewriting_raw_identity` | clearing an entry's runner, adding a pinned entry and removing entries all refresh `pinned_count` while a malformed raw row keeps its exact identity | A | **yes** — `tests.rs:3952` (rechecks prompt pins over the real store) | drop |
| 4542 | `add_prompt_candidate_projection_keeps_only_unmanaged_placeholders` | with interpolation on, an unselected placeholder stays a candidate (`["TOPIC"]`) and a **selected** one becomes managed and leaves the list (`[]`); with interpolation off, **every** placeholder stays a candidate (`["TOPIC","AUDIENCE"]`) | E | **none** | **MM.** Port to `crates/skit-cli/src/cli/tests.rs` as a real add-commit case (production logic is `skit_language::placeholder_params` + `managed_params`) |
| 4620 | `add_python_candidate_projection_redetects_an_unmanaged_source_binding` | deselecting a review candidate for a Python source re-detects it as an unmanaged binding, so `NAME` reappears in `settings.candidates` | E | **none** | **MM.** Pairs with row 4542 |
| 4674 | `prompt_interpolation_controls_only_effective_run_and_detail_declarations` | interpolation off ⇒ no detail parameters, no `value:TOPIC` run field, and `OpenRunPresetSave` opens **no modal**; the stored declaration is kept; switching it back on restores both | E | **none** | **MM.** Port to `tests.rs` alongside the prompt settings cases |
| 4739 | `prompt_without_a_runner_returns_the_owner_preserving_action` | opening Run for a prompt whose pinned runner was removed returns `Action::PromptRunnerRequired`, not a screen | A | **yes** — `tests.rs:5312 selected_prompt_runner_preflight_mapping_is_typed_and_scoped`; `crates/skit-cli/src/cli/tui_host.rs:3405 prompt_runner_refusals_preserve_pick_state_without_a_completed_run` | drop |
| 4763 | `preset_schema_is_authoritative_and_empty_schema_refuses_without_writing` | a preset is filtered by the declaration schema **as of the save**, not as of the form open — if `NAME` becomes secret and `TOKEN` stops being secret between the two, the written preset reflects the new schema; an entry with no declarations refuses a preset save with no write | A | **partial** — `tests.rs:4388`, `:6182 tui_selected_preset_replaces_unchanged_last_used_values` | **MM.** Add the schema-race case to `tests.rs:4388` |
| 4835 | `create_projection_runner_identity_run_glob_and_preferences_are_model_backed` | a committed entry projects workdir, runner, needs, parameters and `prompt_runner: Configured(..)`; a valid runner save leaves a malformed sibling row's identity untouched; an invalid `shell.bash_path` returns `PreferencesError::BashPathMissing` with no mutation; agent-target discovery over an empty set returns an empty list | A | **yes** — `tests.rs:3952`, `:4218`; `crates/skit-cli/src/cli/tui_host.rs:1961 preference_files_own_validation_discovery_and_agent_skill_installation` | drop |
| 4926 | `run_state_after_run_and_protocol_only_effects_are_honest` | `after_run = Exit` makes a run return `Action::Quit` while still persisting `last_runs`; a rerun likewise; and `Effect::None`, `Effect::Quit`, `PreferencesEffect::None` each **error** at the host with no mutation | A | **partial** — `crates/skit-cli/src/cli/tui_host.rs:1721 explicit_roots_and_locale_own_reload_open_remove_and_complete` covers the served set; the "protocol-only effects never reach the host" refusal is not asserted | **MM.** Add the three protocol-only effects as explicit refusals to `tui_host.rs` |
| 4958 | `standalone_runner_refusal_preserves_its_exact_modal_owner` | a Settings-owned standalone editor save that fails returns `RunnerEditorSaveFailed` with the **exact same** `RunnerEditorOwner::Settings { selector }` and keeps the modal open with `host_error()` | A | **yes** — `tests.rs:4069 tui_runner_host_routes_success_to_the_exact_standalone_editor_owner` | drop |
| 5020 | `raw_runner_identity_must_resolve_to_one_exact_row` | a `RawRow` removal whose identity matches two rows is refused with no mutation | A | **yes** — `crates/skit-store/tests/runner_management_cas.rs`; `tests.rs:11202` | drop |
| 5039 | `reducer_raw_runner_repair_refuses_a_valid_name_collision_without_mutation` | repairing a malformed row into an existing valid name emits a `RawRow` save target and is refused; the manager editor stays open; nothing is written | E+A | **partial** — `tests.rs:3952` covers stale-row editor preservation, not the raw-repair-to-existing-name collision | fold into `tests.rs:3952` |
| 5096 | `reducer_raw_runner_repair_refuses_a_malformed_named_row_collision` | the same refusal when the collision target is another **malformed named** row | E+A | **partial** — same | fold in with row 5039 |
| 5156 | `forged_invalid_runner_commands_preserve_each_typed_owner` | an invalid name and an empty argv element are refused, and the refusal action matches its owner exactly (`Manager` → `RunnerManagerAction::MutationFailed`, `Editor(owner)` → `RunnerEditorSaveFailed { owner }`) | A | **yes** — `tests.rs:4069` | drop |
| 5191 | `prompt_run_saves_picked_runner_and_extra_arguments_atomically` | a picked runner sets `_skit_runner_picked: true`, becomes `last_runner`, is echoed back on reopen, **and seeds the runner of the next Add prompt review**; extra args round-trip through `join_editable_argv`; an unbalanced quote refuses whether or not the runner is valid | A | **partial** — `tests.rs:1320 tui_add_opens_the_typed_workflow_with_owned_drafts_and_runner_history` covers runner history; `:6959` covers argv splitting; the *last-runner-seeds-the-Add-review* link is not asserted | **MM.** Add the seeding assertion to `tests.rs:1320` |
| 5310 | `rerun_refuses_a_missing_nonempty_prompt_pin_without_mutation` | rerunning a prompt whose pinned runner is missing is a typed status with no mutation | A | **yes** — `tests.rs:4836 tui_rerun_localizes_runner_failures_without_writing`; `tui_host.rs:3405` | drop |
| 5330 | `prompt_and_command_settings_update_their_own_surfaces_atomically` | a prompt settings save updates `interpolate`, clears the runner and flips detail to `PickOnRunForm`; a command save updates both `settings.template` and `detail.template`; a non-boolean resync value rolls the whole save back | A | **yes** — `tests.rs:9834 a_command_settings_save_that_moves_one_control_leaves_every_other_axis_alone`; `:9762` | drop |
| 5384 | `forged_preferences_globs_and_agent_targets_refuse_without_mutation` | an invalid locale and an unknown skills directory each return a typed status with no mutation | A | **yes** — `tests.rs:4218 typed_preferences_effects_validate_atomically_and_install_only_after_selection` | drop |
| 5420 | `reducer_glob_requests_use_the_same_virtual_root_and_production_counts` | a literal value makes **no** glob request; an unbalanced quote makes **no** request; the 11 glob patterns return exactly `*`→6, `*.py`→2, `***.py`→1, `none-*.zzz`→1, `[ab]lpha.py`→1, `[broken`→1, `**/*.py`→3, `.hidden*.py`→1, `nested/*.py`→1, `nested/.*.py`→1, `unicodé-?.rs`→1; and every request's `cwd` is the run context's `invoke_cwd` | B+E | **partial** — `crates/skit-store/tests/glob_expander.rs:11`,`:28`,`:39`,`:72`,`:99` own the adapter; `crates/skit-application/src/form_feedback.rs:24 glob_count_request` owns the request gate; `tests.rs:4388` owns one end-to-end count | **MM.** Port the 11-row count table (especially the no-match-falls-back-to-1 cases and the hidden/nested/unicode rows) into `crates/skit-store/tests/glob_expander.rs` |
| 5489 | `preferences_reducer_tokens_resolve_presets_custom_urls_and_paused_state` | mirror preset tokens resolve to exact store URLs (tsinghua pypi, nju github → **both** the python-install and uv-binary derivations, npmmirror); **master off keeps every URL but reports `enabled = false`**; a custom URL is normalized (trailing slash dropped for pypi/npm, kept as a prefix for github); choosing `Off` on every axis clears every URL | A+B | **partial** — body-verified: `tests.rs:9341 the_mirror_wizard_answers_reach_the_store_on_every_axis` covers the preset tokens, both nju derivations, and the every-axis-`off` clearing; `:9380 a_custom_mirror_url_is_one_token_and_github_demands_https` covers the custom-URL gate. **Not covered:** the master-off-keeps-the-URLs state (`SetMirrorMaster(false)` leaves `pypi`/`npm` populated while `enabled` is false) — the real test reaches "off" by setting each axis to `off`, which clears them | **MM.** Add the master-off case to `tests.rs:9341` |
| 5617 | `every_pypi_preset_token_from_the_reducer_resolves_to_its_store_value` | tsinghua/aliyun/ustc each resolve to their exact index URL | B | **yes** — `tests.rs:9341`, `:9328` | drop |
| 5641 | `reducer_preset_and_secret_transition_scrub_every_persistent_value_surface` | making a parameter secret scrubs it from every persistent surface at once — saved presets (dropping a preset that held only it), `last_runs`, `remembered_values` and the detail value — and the detail row keeps `secret: true` with an empty value; a secret value is never stored by a run in the first place | A | **yes** — `tests.rs:7344 tui_settings_source_secret_transition_purges_every_state_surface`, `:7412 tui_settings_noop_resync_still_purges_plaintext_for_an_existing_secret` | drop |
| 5737 | `settings_reducer_uses_production_python_validation_and_rolls_back_secret_scrub` | a Python **copy** entry offers no `INTERPRETER_KEY` field; an invalid PEP 508 requirement (`@@@`) and an invalid PEP 440 specifier (`not-a-version`) each refuse the whole save **including the already-staged secret scrub** | A | **partial** — `tests.rs:7344` covers the scrub; `:7072 tui_settings_accept_a_pinnable_interpreter_and_a_managed_binding` covers the interpreter axis; the scrub **rollback** on a later validation failure is not asserted | **MM.** Add the rollback case to `tests.rs:7344` |
| 5792 | `command_settings_reconcile_body_placeholders_and_environment_riders` | a command template's `{PLACEHOLDER}`s become `ParameterDelivery::Placeholder` declarations **in body order** (`SECOND`, `TARGET`), and an added parameter becomes a `ParameterDelivery::Env` rider appended **last** (`EXTRA`); the detail mirrors that order | A | **partial** — `tests.rs:9834`; `:2820 params_host_updates_managed_secrets_and_skips_an_invalid_environment_source_without_writes` | **MM.** Add the body-order + Env-rider-last assertion to `tests.rs:9834` |
| 5839 | `run_and_rerun_refresh_public_detail_and_reopen_an_unpinned_prompt` | a run refreshes the public detail's parameter value and `last_run`; reopening prefills it; a rerun of a prompt whose pin was cleared presents the Run screen instead of launching | A | **yes** — `tests.rs:6182 tui_selected_preset_replaces_unchanged_last_used_values`; `:4836 tui_rerun_localizes_runner_failures_without_writing` | drop |
| 5914 | `changed_defaults_do_not_reuse_an_exact_last_run_as_remembered_prefill` | a submitted value equal to the current default is recorded in `last_runs` but **not** in `remembered_values`, so changing the default later changes both the detail value and the run prefill | E | **partial** — `crates/skit-application/tests/form_state.rs:81 remembered_values_store_only_nondefault_intent_and_structurally_strip_secrets` owns the pure function; the settings→run end-to-end consequence is not asserted | **MM.** Add a short real-host case to `tests.rs` chaining a run, a default change and a reopen |
| 5982 | `settings_workdir_projects_into_run_context_and_glob_requests` | a relative workdir (`invoke`) resolves against the library root and appears in the run context and in every glob request's `cwd`; an absolute workdir is kept verbatim; an empty or relative-with-separator workdir refuses with no mutation | A | **yes** — `tests.rs:3163 doctor_launch_checks_cover_every_runtime_and_workdir_policy`; `:4388` | drop |
| 6049 | `rename_refusal_keeps_the_real_form_owner_and_lists_accept_whitespace` | a blank rename and a rename to a taken name both refuse, keep the Form screen and write nothing; a `needs` list accepts tab, double-space, comma and newline as separators (`"git\tcurl  make,\nrg"` → 4 items) | A+B | **yes** — `crates/skit-store/tests/mutation_refusals.rs:48 same_slug_renames_succeed_while_name_conflicts_and_blank_names_refuse`; the list-splitting half is `crates/skit-application/tests/parameter_edit.rs` / `run_inputs.rs` | drop |
| 6106 | `settings_name_collision_rolls_back_the_complete_transaction` | a settings save whose name is taken rolls back **everything**, including the co-submitted secret flag | A | **yes** — `tests.rs:5965 tui_settings_refuse_a_taken_name_before_javascript_dependency_cleanup`; `crates/skit-store/tests/mutation_refusals.rs:48` | drop |

**Closed 2026-09-06 (MIG-F).** The 15 must-migrate rows (section 5 items 18 to 30, 32 and 33) are
`crates/skit-cli/src/cli/tests/legacy_walker_rules.rs` (`af195e85`). Every other cited owner is
outside `model_walker/`, so those rows are dropped.

---
## 10. `crates/skit-tui/tests/model_walker/fixtures.rs` (3 rows)

A test-only portable projection of the fixture model into canonical JSON (POSIX and `C:\` paths
project identically; every explicit path is canonicalized and sorted by a byte key).

| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 998 | `explicit_path_dialects_are_injective_and_reject_ambiguous_spellings` | `/fixtures/name\part` stays literal under POSIX; `C:\fixtures\name\part` projects to `/fixtures/name/part`; any other drive letter is refused; `./`, `//` and `../` spellings are refused in both dialects | F | conceptual — `crates/skit-cli/src/cli/tui_real_host.rs:18603 c2_session_native_paths_project_only_canonical_registered_objects`, `:18802`, `:18653 c2_wide_unit_escape_is_injective` | drop |
| 1031 | `each_settings_input_field_changes_only_its_own_json_pointer` | each of the 27 `SettingsInputs` fields changes exactly one JSON pointer in the fixture projection | F | n/a — a fixture-serializer contract | drop |
| 1190 | `settings_inputs_serializes_all_twenty_seven_non_default_fields_exactly` | pins the **complete 27-field `SettingsInputs` shape** with an exact expected JSON object (selector, kind, name, description, source, reference_mode, workdir, interpreter, runner, supports_modes, has_original_file, has_stored_name, pinnable_interpreter, has_analyzer, declared_schema, reader_fields, managed, candidates, template, interpolate, dependency_flavor, effective_dependencies, effective_requires_python, needs, configured_runners, presets, revealed) | E | **none** — no test enumerates the full `SettingsInputs` field set; `crates/skit-ui/src/settings.rs` has 36 tests but none is an exhaustive field-shape oracle | **MM.** Port as a serde round-trip / field-completeness test in `crates/skit-ui/src/settings.rs` (no fixture serializer needed) |

**Closed 2026-09-06 (MIG-F).** Row `:1190` is
`crates/skit-ui/src/settings.rs:2655 settings_inputs_names_all_twenty_seven_fields_the_screen_reads`.
The other two rows are dropped.

---

## 11. `crates/skit-tui/tests/model_walker/invariants.rs` (25 rows + 1 infrastructure row)

### Row 0 — the checker itself (infrastructure, **MUST-MIGRATE**)

`check_state` (`invariants.rs:15-25`) is the per-checkpoint reducer-state oracle. It runs a
`LibraryState` JSON round trip (the state must be unchanged by it) and then three sub-checkers over
~40 named error codes. **No real owner runs any of it** — see section 4, item 1.

| checker | file:line | named codes |
|---|---|---|
| `check_library_indices` | `:27` | `LIBRARY_SCHEMA`, `LIBRARY_VISIBLE_TYPE`, `LIBRARY_VISIBLE_BOUNDS`, `LIBRARY_VISIBLE_ORDER`, `LIBRARY_SELECTED_TYPE`, `LIBRARY_SELECTED_EMPTY`, `LIBRARY_SELECTED_MISSING`, `LIBRARY_SELECTED_BOUNDS` |
| `check_screen` | `:82` | `HEALTH_SELECTION_PRESENCE`, `RUNNER_SELECTION_PRESENCE`, `PREFERENCES_FOCUS_HIDDEN`, `AGENT_TARGET_SELECTION_PRESENCE`, plus health/runner/action-row bounds |
| `check_add` | `:167` | `ADD_SCHEMA`, `ADD_DRAFT_SELECTION_BOUNDS`, `ADD_KIND_SUBJECT`, `ADD_KIND_SOURCE_LIFETIME`, `ADD_KIND_SOURCE_SCHEMA`, `ADD_KIND_SOURCE_REPLAY`, `ADD_DELETE_LIFETIME`, `ADD_DELETE_SUBJECT`, `ADD_DELETE_PENDING` |
| `check_focus` | `:270` | `RUN_FOCUS_BOUNDS`, `FORM_FOCUS_BOUNDS` |
| `check_runner_removal` | `:281` | `RUNNER_REMOVAL_SCHEMA`, `RUNNER_REMOVAL_TARGET` |
| `check_runner_editor` | `:377` | `RUNNER_EDITOR_SCHEMA`, `RUNNER_MANAGER_EDITOR_MODE`, `RUNNER_MANAGER_EDITOR_TARGET` |
| `check_modal` | `:454` | `REMOVE_SCHEMA`, `REMOVE_SUBJECT`, `REMOVE_PRESERVED_FACT`, `DISCARD_CLEAN_SETTINGS`, `RUN_PRESET_SUBJECT`, `RUN_TOKEN_OPTIONS`, `RUN_ENV_CAPABILITY`, `RUN_ENV_CONTEXT`, `RUN_ENV_SUBJECT`, `RUN_ENV_REPLAY`, `RUN_FILE_CAPABILITY`, `RUN_FILE_SUBJECT`, `RUN_FILE_REPLAY`, `RUN_MODAL_FOCUS`, `RUNNER_EDITOR_MODE`, `RUNNER_EDITOR_RUN_CAPABILITY`, `RUNNER_EDITOR_SETTINGS_CAPABILITY`, `RUNNER_EDITOR_SETTINGS_STATUS` |

Three of these are **replay** oracles, not shape checks — `ADD_KIND_SOURCE_REPLAY` (`:200-223`),
`RUN_ENV_REPLAY` (`:664-680`) and `RUN_FILE_REPLAY` (`:682-693`) rebuild the subject by re-running
the reducer from a clean state and compare, so a modal can never carry a stale subject. Those are
the highest-value rules in the file and the hardest to reconstruct.

**Migration note:** move `check_state` (and its helpers) into `crates/skit-ui` as a
`#[doc(hidden)] pub fn validate_library_state(&LibraryState) -> Result<(), String>` (or into
`skit-tui-walker-support`), then call it from `RealFrontend::observe_frontend`
(`crates/skit-cli/src/cli/tui_real_walker.rs:633`) next to `validate_styled_frame`. The 22 forged-state
`rejects_*` tests below can then be ported verbatim against the new public entry point.

### The 25 tests

Twenty-two feed **hand-forged JSON** to `check_state` and assert the exact refusal; three assert
that the **reducer produces** a state the checker accepts. The forged ones are tests of the checker
(F); the accepting ones are reducer contracts (E).

| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 780 | `accepts_the_in_flight_draft_delete_before_the_host_answers` | the reducer's in-flight draft-delete state (effect emitted, host not yet answered) satisfies every invariant | E | **yes** — `crates/skit-cli/src/cli/tui_real_host.rs:20891 checkpoint_capture_projects_the_real_add_delete_chain_in_every_effect_position` | drop |
| 788 | `rejects_the_e29db41_draft_subject_regression_shape` | a draft-delete confirmation with a null `delete_candidate` is refused — a **named commit regression guard** | F (checker) | none | **MM.** Port with the checker (row 0) |
| 799 | `rejects_a_run_modal_that_names_a_missing_field` | a run token menu naming field 999 is refused | F | none | port with row 0 |
| 815 | `rejects_a_runner_editor_owned_by_a_different_run_form` | a runner editor whose owner selector differs from the live run form is refused | F | none | port with row 0 |
| 827 | `rejects_nested_management_indices_that_name_no_row` | out-of-bounds runner and health selections are refused | F | none | port with row 0 |
| 869 | `rejects_a_discard_guard_for_clean_settings` | a discard-changes modal over clean Settings is refused (`DISCARD_CLEAN_SETTINGS`) | F | none | port with row 0 |
| 891 | `rejects_stale_run_modal_subjects` | a preset-name modal whose `existing` list is stale, and a token menu whose `options` are stale, are refused | F | none | port with row 0 |
| 917 | `rejects_stale_runner_removal_identity` | a removal confirmation whose row snapshot token changed after confirmation is refused (`RUNNER_REMOVAL_TARGET`) | F | none | port with row 0 |
| 938 | `rejects_remove_confirmation_with_stale_detail_facts` | a remove confirmation whose `original_file_preserved` disagrees with the live detail is refused | F | none | port with row 0 |
| 955 | `accepts_raw_removal_of_a_pinned_named_duplicate` | the reducer's raw removal of the invalid duplicate of a pinned named runner satisfies every invariant | E | **none** | port with row 0 |
| 979 | `rejects_jointly_truncated_named_runner_removal_cas` | truncating both rows' `key_identities` *and* the removal's `expected` in lockstep is still refused — the CAS is not self-consistent-by-construction | F | none | **high value**; port with row 0 |
| 1015 | `rejects_remove_confirmation_with_a_duplicate_or_unselected_selector` | a duplicated slug and a moved selection each refuse (`REMOVE_SUBJECT`) | F | none | port with row 0 |
| 1042 | `rejects_stale_or_wrong_add_delete_subjects` | a stale stage, a wrong candidate, and a wrong pending-delete draft each refuse | F | none | port with row 0 |
| 1071 | `rejects_an_add_draft_selection_that_names_no_draft` | an out-of-bounds `selected_draft` is refused | F | none | port with row 0 |
| 1083 | `rejects_a_nonempty_agent_picker_without_selection_or_a_hidden_focus` | a non-empty agent-target picker with no selection is refused, and Preferences focus on a hidden control is refused (`PREFERENCES_FOCUS_HIDDEN`) | F | none | port with row 0 |
| 1104 | `rejects_nonempty_management_surfaces_without_a_selection` | non-empty runner rows or health issues with a null selection are refused | F | none | port with row 0 |
| 1138 | `rejects_stale_manager_editor_targets` | an editor whose `expected` snapshot token is stale is refused | F | none | port with row 0 |
| 1152 | `rejects_a_kind_stage_without_its_inspected_source` | the reducer's Kind stage with its inspected source is accepted; dropping the stage, changing the inspected bytes, or nulling `pending_source` each refuse (`ADD_KIND_SOURCE_LIFETIME` / `ADD_KIND_SUBJECT`) — this is the **reducer-replay** oracle | E + F | **partial** — `tui_real_host.rs:20033 c2_every_add_effect_path_projects_in_all_checkpoint_cause_positions` covers projection, not the replay equality | port with row 0 |
| 1209 | `accepts_a_named_manager_editor_for_an_editable_invalid_duplicate` | the reducer's named editor over an editable invalid duplicate satisfies every invariant | E | **none** | port with row 0 |
| 1232 | `rejects_runner_overlays_not_owned_by_the_selected_row` | a removal whose selection moved, and a repair editor whose raw row identity went stale, each refuse | F | none | port with row 0 |
| 1267 | `rejects_a_settings_runner_editor_without_a_runner_section` | a Settings-owned runner editor on an entry with no runner section is refused (`RUNNER_EDITOR_SETTINGS_CAPABILITY`) | F | none | port with row 0 |
| 1295 | `rejects_out_of_bounds_workflow_focus` | run focus 999 and form focus 999 (on an empty field list) each refuse | F | none | port with row 0 |
| 1317 | `rejects_environment_and_file_picker_contract_drift` | an environment picker whose `visible` list drifts from a clean replay, and a file picker whose context workdir drifts, each refuse — the other **reducer-replay** oracles | F | none | **high value**; port with row 0 |
| 1343 | `rejects_runner_editor_mode_and_recovery_drift` | a run-owned runner editor carrying a cancel status while a picker exists is refused; an editor whose view target is not `New` is refused | F | none | port with row 0 |
| 1373 | `rejects_noncanonical_visible_library_indices` | a `visible` array that is not strictly ascending is refused (`LIBRARY_VISIBLE_ORDER`) | F | none | port with row 0 |

**Closed 2026-09-06 (MIG-F).** Row 0's checker is `skit_tui_walker_model::invariants::check_state`,
wired at `crates/skit-cli/src/cli/tui_real_walker.rs:708`. The 25 tests are
`crates/skit-tui-walker-model/src/invariants_tests.rs`.

---

## 12. `crates/skit-tui/tests/model_walker/local_inventory.rs` (8 rows)

**The highest-value file after `invariants.rs`.** It is the only place that enforces product rule 2
("keep each TUI action available by keyboard and mouse") over the **screen-local** action inventory.
`crates/skit-tui/tests/render.rs:1301 every_enabled_registry_key_and_rendered_chip_work_at_every_size_tier`
does the equivalent sweep for the **global command registry** only; `crates/skit-tui/src/footer.rs:1289
assert_local_action_inventory` checks the footer item lists, not the rendered inventory. Six of the
eight rows have **no** real owner.

The file uses `FakeHost` only to *reach* states (`open()` at `:30`), so migration means either
replacing those with a real host (`crates/skit-cli/src/cli/tui_real_host.rs` factories) or building
the states directly from `LibraryState` — the assertions themselves are pure frontend.

| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 216 | `registry_empty_local_contexts_export_live_key_and_mouse_semantics` | on Add, Health, Runners, the nested Runners editor and the standalone RunnerEditor: the inventory is non-empty; **every** advertised action has a non-empty mouse hit **and** at least one key; the mouse path arms on press and completes on release; and every key alias yields exactly the same `EventHandling` as the mouse | E | **none** | **done** — `crates/skit-tui-walker-model/tests/local_action_parity.rs` `every_advertised_local_action_has_a_hit_and_an_equal_key_endpoint` |
| 263 | `local_inventory_is_empty_for_hidden_overlays_and_clipped_cells` | a 1×1 Add screen advertises **no** local action (no invented rect); at 24×6 every action has a positive rect; after Ctrl+O opens the path picker the base screen's inventory is empty | E | **none** — `crates/skit-tui/tests/viewport_session_invariants.rs:31 zero_height_local_viewports_do_not_emit_positive_children_or_hits` is the closest, but covers geometry hits, not the local inventory | **done** — `crates/skit-tui-walker-model/tests/local_action_parity.rs` `local_inventory_is_empty_for_clipped_cells_and_open_overlays` |
| 301 | `every_advertised_alias_is_a_positive_session_path` | when one local action advertises several chords (the runner editor's Tab/↓ and BackTab/↑), **every** alias reaches the same endpoint as its mouse hit | E | **none** | **done** — `crates/skit-tui-walker-model/tests/local_action_parity.rs` `every_advertised_alias_reaches_the_mouse_endpoint` |
| 319 | `inventory_snapshot_is_tied_to_the_last_rendered_local_context` | the inventory is empty on the Library and non-empty after opening Health — it always describes the last rendered frame, never a stale one | E | **none** | **done** — `crates/skit-tui-walker-model/tests/local_action_parity.rs` `local_inventory_describes_only_the_last_rendered_screen` |
| 329 | `preferences_shared_focus_hits_return_typed_session_actions` | every `FocusNext`/`FocusPrevious` hit on Preferences returns an `Action::Preferences(_)`, not a generic focus action | E | **none** | **done** — `crates/skit-tui-walker-model/tests/local_action_parity.rs` `preferences_shared_focus_hits_return_typed_preferences_actions` |
| 359 | `local_operations_resolve_from_the_latest_rendered_session_inventory` | the legacy resolver binds `LocalAdvertisedKey`/`LocalHit` ordinals against the latest inventory and centres the click in the advertised rect | F | **yes** (equivalent concept) — `crates/skit-cli/src/cli/tui_real_walker.rs:1433` binds against `frontend.session.local_action_inventory().actions`; `:6637 corpus_clipped_hit_is_not_applicable_without_guessing_an_ordinal` | drop |
| 410 | `raw_and_random_pointer_events_cannot_select_the_live_quit_target` | Escape and every one of the 65 536 random pointer cells that lands on the live Quit chip resolve to a no-op — **both halves** of the click are removed, and the grid provably reaches Quit | F (walker safety) | **yes** — `tui_real_walker.rs:6494 corpus_resolver_records_typed_refusals_and_real_input_plans` (`QuitFiltered`, `EventWouldQuit`); `:1151` (`map_event(...) == Some(Action::Quit)`) | drop |
| 485 | `runner_local_surfaces_are_bounded_and_unambiguous_in_every_tier_and_locale` | across 5 runner surfaces × 4 locales (En, ZhCn, ZhTw, Pseudo) × 3 viewports (1×1, 24×6, 120×30): every local rect is non-empty and fully inside the viewport, **no two rects with different targets overlap**, and every key alias equals the mouse endpoint | E | **none** | **done** — `crates/skit-tui-walker-model/tests/local_action_parity.rs` `runner_local_surfaces_are_bounded_and_unambiguous_in_every_tier_and_locale` |

**Closed 2026-09-06 (MIG-F).** The six rows marked **done** are
`crates/skit-tui-walker-model/tests/local_action_parity.rs`. Rows `:359` and `:410` keep their cited
owners in `crates/skit-cli/src/cli/tui_real_walker.rs`; `:410` also ports with
`CorpusOperation::RawMouse`.

---

## 13. `crates/skit-tui/tests/model_walker/strategy.rs` (8 rows)

The proptest generator and the late-binding resolver. Most rows describe the generator, but two pin
input corpora that carry real product meaning.

| line | test | product behaviour asserted | class | equivalent in a real owner | migration note |
|---|---|---|---|---|---|
| 449 | `operation_strategy_generates_every_required_event_family` | over 4096 draws the strategy emits all nine families: AdvertisedKey, PublicHit, LocalAdvertisedKey, LocalHit, MouseCell, Resize, Paste, RawKey, Focus | D | **yes** (equivalent vocabulary) — `crates/skit-cli/src/cli/tui_real_walker.rs:403` defines the 11-variant `CorpusOperation`; `:7136 corpus_machine_mappings_are_total_and_stable` and `:7570 corpus_screen_operations_and_resolutions_have_exact_outer_shapes` pin it | drop |
| 486 | `public_hits_resolve_against_the_latest_geometry` | the same `PublicHit { ordinal: 0 }` resolves to different cells against different geometries — ordinals are late-bound, never pre-computed | D | **yes** — `tui_real_walker.rs:6637 corpus_clipped_hit_is_not_applicable_without_guessing_an_ordinal`; `:1183 validate_screen_inventory` | drop |
| 509 | `arbitrary_mouse_cells_stay_inside_the_current_viewport` | fraction 255 maps to the last addressable cell (`(23,5)` in a 24×6 viewport), never outside | F (generator) | n/a — the real corpus names targets, not fractions | drop |
| 527 | `resize_cases_include_tiny_responsive_and_large_viewports` | the required resize shapes are exactly `(1,1) (1,2) (2,1) (24,6) (40,40) (46,12) (80,24) (120,12) (120,30) (300,100)` — degenerate one-column/one-row tiers, the compact 24×6 tier, the responsive 46×12 tier, and an oversize 300×100 | D | **none** — `tui_real_walker.rs` resizes exist (`:6957`, `:7040`, `:7067-7099`) but no test asserts a required set | **MM.** Port the required-shape list into `tui_real_walker.rs` next to `canonical_corpus_operations()` |
| 545 | `paste_cases_cover_terminal_text_edges` | the required paste payloads are exactly `""`, `"界"` (wide), `"e\u{301}"` (combining), `"🙂"` (astral), `"one\ntwo"` (newline), `"a\tb"` (tab), `"\0"` (NUL) | D | **none** — the real corpus pastes only realistic user text (`tui_real_walker.rs:3538`, `:3627`, `:6613`) | **MM.** Port the required-payload list into `tui_real_walker.rs`; note `crates/skit-tui/tests/interactive_run_form.rs:1275` and `:1422` cover paste editing semantics but not this edge set |
| 552 | `semantic_traces_have_deterministic_json_replays` | an operation vector round-trips through JSON unchanged, so a saved trace replays exactly | D | **yes** — `tui_real_walker.rs:7702 corpus_operations_have_stable_source_owned_shapes`; `:7570` | drop |
| 572 | `raw_key_inventory_excludes_the_session_clock_dependent_ctrl_c_chord` | no `RawKey` in the inventory ever resolves to `Ctrl+C` — that chord's meaning depends on the session clock (double-press quit) and would make the walk non-deterministic | F (walker safety) | **yes** for the underlying rule — `crates/skit-tui/tests/render.rs:1614 central_session_ctrl_c_search_help_and_confirmation_priority_use_real_events` owns Ctrl+C semantics | drop |
| 595 | `model_operations_do_not_select_quit_from_shared_keys_or_hits` | no advertised-command ordinal ever produces Ctrl+C, and a `PublicHit` over a Quit chip resolves to a no-op | D | **yes** — `tui_real_walker.rs:6494 corpus_resolver_records_typed_refusals_and_real_input_plans` (`QuitFiltered` for `CommandKeyboard(Quit)` and `HitTarget::Command(Quit)`; `EventWouldQuit` for a raw `q`) | drop |

**Closed 2026-09-06 (MIG-F).** Rows `:527` and `:545` are `RESIZE_CASES` and `PASTE_CASES` in
`skit_tui_walker_model::model`, with their exact-set tests in `src/model_tests.rs`. The other six
rows keep their cited owners.

---

## 14. Recommended migration order

1. **`invariants::check_state` → `crates/skit-ui` (or `skit-tui-walker-support`), wired into
   `RealFrontend::observe_frontend`** (`crates/skit-cli/src/cli/tui_real_walker.rs:633`), then port
   the 25 `invariants.rs` tests against the new entry point. This alone recovers ~40 named
   invariants plus the three replay oracles, and unblocks `driver.rs:3998`.
2. **`check_public_hit_parity` → a reusable helper**, called per checkpoint from the real walker's
   `observe`. Recovers whole-frame keyboard/mouse parity (product rule 2) and subsumes
   `driver.rs:3142`, `:3183` (partially) and the `local_inventory.rs` sweeps.
3. **`local_inventory.rs` rows 216, 263, 301, 319, 329, 485 →
   `crates/skit-tui-walker-model/tests/local_action_parity.rs`.** Pure frontend, no host needed; the
   `FakeHost::open` helper became direct `LibraryState` construction.
4. **`driver.rs:2937`, `:3142`, `:3183`, `:3414`, `:3487` →
   `crates/skit-tui-walker-model/tests/run_pointer_rules.rs`, and `:3622` →
   `crates/skit-tui-walker-model/tests/preferences_focus_rules.rs`.** `render_probe_endpoint`,
   `render_probe_session` (`driver.rs:1420-1457`) and `command_key_action` with its focus budget
   (`driver.rs:1612-1735`) are the model crate's `parity` module.
5. **The 13 `fake_host.rs` MUST-MIGRATE rows → `crates/skit-cli/src/cli/tests.rs`.** Each is a small
   addition to an existing real-store test; none needs a new fixture.
6. **`strategy.rs:527` / `:545` and `driver.rs:2234` → `tui_real_walker.rs` corpus constants.**
7. **`fixtures.rs:1190` → `crates/skit-ui/src/settings.rs`.**
8. **Decide `ui-walker.yml`** (section 3): retire, retarget with a new artifact writer, or keep
   without the cast render. Do this **before** deleting `model_walker/`, or CI breaks on the next
   nightly.
9. **Delete `crates/skit-tui/tests/model_walker.rs` and `crates/skit-tui/tests/model_walker/`**, and
   remove `proptest` and `skit-tui-walker-support` from `crates/skit-tui/Cargo.toml`'s
   `[dev-dependencies]`. No production source change is required (section 2).

---

**Closed 2026-09-06 (MIG-F).** Steps 1 to 9 are done. Four destinations changed:

- Step 1 landed in the new dev-only crate `skit-tui-walker-model`, not in `skit-ui` or
  `skit-tui-walker-support`.
- Step 5 landed in `crates/skit-cli/src/cli/tests/legacy_walker_rules.rs`, not in `tests.rs`.
- Step 6 landed in `skit_tui_walker_model::model`, not in `tui_real_walker.rs`.
- Step 8 took option 2, the retarget, in `c022b825`.

Step 9 deleted `crates/skit-tui/tests/model_walker.rs` and `crates/skit-tui/tests/model_walker/`
(16 088 lines, 171 tests) and removed the `proptest` and `skit-tui-walker-support`
dev-dependencies. No production source changed.
