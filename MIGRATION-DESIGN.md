# Legacy walker migration design, revision 2 (2026-09-04)

Scope: retire `crates/skit-tui/tests/model_walker/` (16,088 lines, 171 tests, `FakeHost`) after every
useful contract has a real owner. `docs/design/fakehost-migration-ledger.md` classifies all 171
tests: 124 carry product meaning, 78 already have a real owner, 28 have a partial owner, 18 have
none, and 34 rows are must-migrate. No production source exists only for the legacy owner, so
deletion changes no production code.

Revision 2 folds in two advisor reviews (a full-context Fable fork and a fresh-context Opus
reviewer). The decisive change: the product-coupled oracles get a dev-only, mutation-visible crate
instead of skit-cli private test modules.

Ordering constraint: MIG-D wiring, MIG-E's real-host driver, and the `RawMouse` resolver edit
`crates/skit-cli/src/cli/tui_real_walker.rs`, which G4 owns until G4 commits. Everything else starts
now in disjoint files.

## M0. The gates that shape every placement decision

- Every line in a `#[cfg(test)]` module under `crates/*/src/` is coverage-gated by
  `scripts/check_coverage.sh` (no path filter; an `#[ignore]` body is uncovered lines). Files under
  `crates/*/tests/**` get no lcov record. A workspace coverage run merges hits from every test
  binary, so a dev-only crate's lines count when skit-cli's tests execute them.
- `cargo mutants` skips `#[cfg(test)]` items, so skit-cli private modules are mutation-blind while a
  separate crate is mutation-visible. AGENTS.md requires zero survivors.
- `skit-tui-walker-support` must stay product-independent and must not depend on skit-ui,
  skit-tui, skit-cli, or proptest.

Therefore product-coupled oracle code that is a rule (state invariants, parity probes, the
operation model and its tables) lives in a new dev-only crate; product-independent driver code that
needs proptest also lives there; skit-cli keeps only what needs its private real host.

## M1. New crate `skit-tui-walker-model`

`crates/skit-tui-walker-model`, `publish = false`, workspace member, `[lints] workspace = true`.
Dependencies: `skit-ui`, `skit-tui`, `skit-tui-walker-support`, `serde`, `serde_json`, `proptest`,
`ratatui-core`. Dev-dependency of `skit-cli-rs`. It is never a dependency of `skit-tui` (no
dev-dependency cycle, no duplicate crate instances); the pure-frontend suites that need its helpers
live in its own `tests/` directory (M4, M5). Modules:

- `invariants`: `pub fn check_state(state: &LibraryState) -> Result<(), String>` and its helpers,
  moved code and named codes intact from the legacy `invariants.rs` (about 40 codes in
  `check_library_indices`, `check_screen`, `check_add`, `check_focus`, `check_runner_removal`,
  `check_runner_editor`, `check_modal`, plus the reducer-replay oracles `ADD_KIND_SOURCE_REPLAY`,
  `RUN_ENV_REPLAY`, `RUN_FILE_REPLAY`, which re-run the production reducer from a clean state). The
  25 legacy tests port verbatim as `invariants_tests.rs` (22 forged-JSON refusals, 3 reducer
  acceptances).
- `parity`: the probe family (`check_public_hit_parity`, `check_field_hit_parity`,
  `expected_hit_action`, `check_keyboard_focus_path`, `field_activation_sequences`,
  `session_action`, `session_primary_click`, `browse_keyboard_action`, `command_key_action`,
  `check_command_key_endpoint`, `apply_probe_handling`, `render_probe_endpoint`,
  `render_probe_session`; about 30 named codes). It uses only public skit-tui API
  (`TuiSession::try_fork`, `advertised_command_bindings`, `screen_target_inventory`,
  `local_action_inventory`, `ViewGeometry`) and the production reducer; a probe forks the frontend
  session, applies one pointer or keyboard path, renders, compares the complete endpoint (frame and
  geometry), and drops the fork. `expected_hit_action` is the public hit contract only (a Command
  hit yields its command action, `FocusField(i)` yields `Action::FocusField(i)`, run tokens yield
  their option payload); it never grows host or reducer expectations. Every named code gets a
  refusal test in `parity_tests.rs` reached through a forged session or geometry (this is what makes
  the `Err` arms covered and the mutants killable). The `try_fork` failure arm: `try_fork` returns
  `None` only while a path-completion worker owns a channel, which the walker never has; write the
  arm as a one-line `ok_or_else` and pin the reason in a comment.
- `model`: the random operation model. The nine weighted families with late-bound ordinals
  (`AdvertisedKey`, `PublicHit`, `LocalAdvertisedKey`, `LocalHit`, `MouseCell`, `Resize`, `Paste`,
  `RawKey`, `Focus`), the ten resize shapes, the seven paste payloads, the 15-profile matrix
  (`random_walk_profiles()`), the proptest `Strategy`, and the late binder that turns an ordinal
  into a semantic target against a live command registry, `ViewGeometry.hits`,
  `LocalActionInventory`, and `ScreenTargetInventory`. Exact-set tests for the shapes, payloads,
  and matrix, plus a direct unit test per family (deterministic seed) so no line depends on a coin
  flip. The binder returns a product-independent `ResolvedInput` that skit-cli maps to its private
  `CorpusOperation`.
- `artifacts`: failure and success bundle writers (`repro.json` plus a cast; staged then renamed,
  two failures never share a bundle), and the branch-free env readers for `SKIT_WALKER_CASES`,
  `SKIT_WALKER_STEPS`, `SKIT_WALKER_PROFILES` (`bounded`|`complete`), `SKIT_WALKER_RECORD_SUCCESS`
  (`0`|`1`), `SKIT_WALKER_LIVENESS_EVERY`, and `SKIT_WALKER_REPRO`. Strictness without branch lines:
  `["0", "1"].iter().position(|v| *v == raw).expect("SKIT_WALKER_RECORD_SUCCESS must be 0 or 1") == 1`;
  positive integers as `parse().ok().filter(positive).unwrap_or(default)` followed by
  `assert!(value > 0)`; the workflow's shell guards refuse zero before cargo runs.

## M2. Per-checkpoint oracles in the real engine (skit-cli)

`RealFrontend::observe_frontend` calls `check_state(&self.state)` before the draw (legacy order) and
`check_public_hit_parity(...)` after `validate_styled_frame`; `CorpusFrontend::observe` delegates to
it and so inherits both. A violation is an observation error that poisons the engine like a
styled-frame violation. Cost (from the legacy implementation): one fork plus render per non-empty
hit, and for form fields the activation sequences (7 per text field, `2*options+2` per choice
field); about 50 ms per form checkpoint, roughly +15 s per profile on the corpus. The sweep is not
bounded: endpoints depend on session state, not only on geometry. Measure and record the preflight
and corpus deltas in the handoff; they also raise every mutant's suite time (issue #47).

The corpus resolver's local dispatch gains the descriptor-versus-live-endpoint refusal
(`LOCAL_ACTION_ENDPOINT`, ledger `driver.rs:3031`) tested through the existing `local_actions`
parameter of `resolve_corpus_operation_with_local_actions` with a forged descriptor.
`driver.rs:3998` becomes a real-engine test through `frontend_for_host_with_initial_state`: an
initial state violating `LIBRARY_VISIBLE_ORDER` fails `engine.start` while the sink keeps a valid
cast header and one diagnostic frame. `driver.rs:2829` becomes a corpus case: a `Consumed` operation
still records a Session checkpoint and an open overlay leaves the local inventory empty.

## M3. The randomized walk rebuilt on the real host (skit-cli)

`crates/skit-cli/src/cli/tui_real_random_walk.rs` (private `#[cfg(test)]`) owns only what needs
the private host: it maps the model's `ResolvedInput` to `CorpusOperation`, spawns one fresh
random-mode real host per (case, profile) through `RealWalkerFactory::create`, runs the
`WalkerEngine` with a refusal sink, and runs final liveness once per case through the existing
engine phase. Liveness per checkpoint on a real host needs a fresh prefix replay; the default runs
none, and `SKIT_WALKER_LIVENESS_EVERY=k` (nightly only) replays the prefix on a fresh host and runs
final liveness every k operations. A frontend-only probe is rejected: `route_effect` sends every
effect except `None` and `Quit` to the host, so "the reducer emitted an effect" proves intent, not
arrival. This is the one deliberate loss versus the legacy per-checkpoint probe; the handoff
records it.

- `CorpusOperation::RawMouse { column, row, kind: CorpusMouseKind }` with the eight legacy kinds
  and exact serde pinned in `corpus_machine_mappings_are_total_and_stable` and
  `corpus_operations_have_stable_source_owned_shapes`; the canonical vector bytes do not change.
  Its quit filter is session-level: the resolver forks the session and feeds the same single raw
  event that it will dispatch. It refuses only an event that emits `Quit`. An unarmed release
  stays inert, and a primary press can remain armed until the next input. Semantic target clicks
  still send Down followed by Up.
- One non-ignored test `real_random_walk` reads the env values through the model's readers with
  defaults 4 cases, 24 steps, `bounded` (en 80x24 only), `0`, no liveness sampling, and a fixed
  ChaCha seed with `FileFailurePersistence::Off`; CI's normal job runs it at the defaults and covers
  every line. `complete` mode uses proptest's entropy seed, `FileFailurePersistence::Direct` under
  `target/ui-walker-artifacts/regressions.txt`, prints the seed in the failure message, shrinks on
  the failing profile only with a measured `max_shrink_iters` cap, replays the minimal vector on a
  fresh host, and writes `failure-<n>/{repro.json, failure.cast}`. Initial vectors contain exactly
  the configured step count; shrinking reduces operation values within that length. A requested success writes
  `success-<n>/{repro.json, success.cast}`. `catch_unwind` around host construction and the walk
  keeps the cast for a panic; both the invariant-failure path and the panic path are covered through
  `frontend_for_host_with_initial_state` (a forged initial state; a panicking closure). `repro.json`
  holds the resolved `CorpusOperation` vector; `SKIT_WALKER_REPRO` replays it through the same
  adapter.
- CI: `.github/workflows/ui-walker.yml` changes both selectors to
  `cargo test --locked -p skit-cli-rs --lib cli::tui_real_random_walk::real_random_walk -- --exact --nocapture`
  (and `--list`, expecting `cli::tui_real_random_walk::real_random_walk: test`, no `--ignored`),
  sets `SKIT_WALKER_PROFILES: "complete"`, and keeps the `agg`, upload, and label machinery. The
  exact pins in `scripts/test_tooling_contracts.sh` (`:406-407` env values, `:449-451` selectors,
  `:450` the list guard, `:455` the timeout string) change in the same commit. Budget: a real host
  costs about 22 s for 100 operations before the M2 sweep; 16 cases x 15 profiles is about 90
  minutes against the 65-minute timeout. Measure one (case, profile) with M2 enabled on this host,
  then choose the nightly values (start at 4 cases x `complete`), and record them in the pins.
- The 15-profile matrix (every locale at 1x1, 24x6, 120x30; plus en 80x24, pseudo 120x12,
  zh-TW 40x40, pseudo 120x30) runs only in `complete` mode; the bounded default runs en 80x24. The
  handoff records that the tiny and compact tiers across all four locales are nightly-only.

## M4. Screen-local inventory contracts (pure frontend)

`local_inventory.rs` rows 216, 263, 301, 319, 329, 485 become
`crates/skit-tui-walker-model/tests/local_action_parity.rs`. States come from
`LibraryState::from_library_surface` plus `TuiSession` events; no host. Row 485 needs the nested
Runners editor and the standalone RunnerEditor; verify both are reachable from surface plus events
before committing, else the row takes a real-host factory in skit-cli. MIG-A is implementing these
under `crates/skit-tui/tests/` with local helpers; the files move into the model crate's `tests/`
directory when the crate exists, and their local helpers are replaced by the crate's `parity` API.

## M5. Specific pointer rules (pure frontend)

Same destination directory. `driver.rs:3183` (run picker: first click focuses and opens; second
click closes with the same rendered endpoint as Escape; Down only arms; release away cancels; a
stale release is inert), `:3142` (Run screen `FocusPrevious` chip equals its key), `:2937` (a raw
`LeftDown` is one event and keeps the arm), `:3414`/`:3487` (a scrolled form keeps a positive hit
on the focused row), and `:3622` (a shared footer chord is unreachable through a Tab prefix while
another widget owns it; the first Preferences frame advertises only `Tab`; Editor focus hides
`ManageAgents`/`InstallAgentSkill`). They use the crate's `render_probe_endpoint` and
`command_key_action` instead of private copies.

## M6. Host and reducer rules (real stores)

The 15 `fake_host.rs` must-migrate rows (ledger items 18–30, 32, 33) become tests in
`crates/skit-cli/src/cli/tests/legacy_walker_rules.rs` (declared from `cli/tests.rs`) against real
`TempDir` stores and the production `tui_effect` seam; `fixtures.rs:1190` (27-field
`SettingsInputs` pin) becomes a `crates/skit-ui` settings test.

## M7. Constants with product meaning

The ten resize shapes, seven paste payloads, and the 15-profile matrix are the model crate's tables
(M1 `model`) with the legacy exact-set tests ported, mutation-visible.

## M8. Deletion

After M1–M7 land with review: delete `crates/skit-tui/tests/model_walker.rs` and
`crates/skit-tui/tests/model_walker/`; remove `proptest` and `skit-tui-walker-support` from
`crates/skit-tui/Cargo.toml` dev-dependencies; update `docs/` references; the ledger records every
row's destination. The CI retarget happens in MIG-E, before deletion, so the tree is never red
between commits.

**Done (MIG-F, 2026-09-06).** The ten files are deleted (16 088 lines, 171 tests), the two
dev-dependencies and their two `Cargo.lock` entries are gone, and
`docs/design/fakehost-migration-ledger.md` closes every section with the row's real owner. No
production source changed.

## Commit ladder

- MIG-A `test(walker): own screen-local parity and pointer rules` (M4, M5; lands after MIG-C so
  the files can live in the model crate).
- MIG-B `test(cli): own host rules from the legacy walker` (M6).
- MIG-C `feat(walker): add the model crate with state invariants` (M1 crate scaffold, `invariants`,
  its 25 tests; no skit-cli wiring yet).
- MIG-D `feat(walker): check invariants and parity at every real checkpoint` (M1 `parity` with its
  refusal tests; M2 wiring and the three real-engine tests). After G4 commits.
- MIG-E `feat(walker): rebuild the random walk on the real host` (M1 `model` and `artifacts`; M3;
  M7; `RawMouse`; CI retarget and tooling pins). After G4 commits.
- MIG-F `chore(tui): delete the FakeHost walker` (M8). **Done 2026-09-06.**

Each unit gets a fresh-context adversarial review at 0/0/0 before commit and a Codex review when the
quota returns.

## Deliberate losses, recorded

- The two whole-frame `check_public_hit_parity` sweeps of legacy `driver.rs:3183` (before and
  after the picker rules) are absent from the MIG-A port while its files live under
  `crates/skit-tui/tests/`; when the files move into the model crate's `tests/` directory (after
  MIG-D part 1), `typed_run_picker_pointer_rules_match_their_keyboard_endpoints` regains both
  sweeps through the crate's `parity` API. Until then the rule is owned by nothing but the legacy
  test, which is why MIG-F cannot precede that move.
  **Resolved (`cd2380a8`).** The move happened, and
  `crates/skit-tui-walker-model/tests/run_pointer_rules.rs:303` and `:477` carry both sweeps.

- Per-checkpoint liveness on real hosts is replaced by final liveness per case plus nightly
  sampled prefix replay.
  **Stands.** `SKIT_WALKER_LIVENESS_EVERY` samples the prefix replay and is nightly only.
- The 15-profile matrix runs nightly, not on every PR.
  **Stands.** `SKIT_WALKER_PROFILES=complete` selects the matrix; the default is `bounded`.
- The random walk's effect limit is the real walker's 16, not the legacy 64.
  **Stands.** `EFFECT_LIMIT` is 16 in `crates/skit-cli/src/cli/tui_real_walker/corpus.rs`.
