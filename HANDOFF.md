# skit Rust rewrite — session handoff (2026-08-10)

**Read this first, then `docs/design/python-test-port-ledger.md` (the authoritative per-module
record + fix-list) and the memory at `~/.claude/projects/-home-ubuntu-coding-skit/memory/`.**
This supersedes the stale `CLAUDE_HANDOFF.md` (codex's; delete it at Phase 5).

Branch: `rewrite/rust-ratatui-complete-20260808-codex`. Oracle checkout: `/home/ubuntu/coding/skit-oracle`
(pinned `origin/main@206f9ef`, v0.4.1.dev0). Rust workspace: `/home/ubuntu/coding/skit`.

---

## 0. One-line status

**The Python behavior port is COMPLETE and green (2840 pass / 0 fail / 1179 ignored, tree clean, 81
`port_test_*.rs` files). The impl-fix pass has STARTED (i18n cluster done). ~85 v0.4 divergences
remain to fix.** The user chose plan **A**: finish the whole port FIRST (done), THEN one comprehensive
impl-fix pass (in progress).

---

## 1. The mission (do not lose this)

v0.5.0 must be a **strict superset** of the v0.4 oracle `origin/main@206f9ef`. 可以多，不能少 (more
allowed, less never). The Rust rewrite must be a **faithful TRANSLATION** of the Python
implementation — read the Python impl AND the Python tests, transcribe the behavior. Do NOT reinvent
Rust logic. The user was repeatedly furious about exactly that: inventing fresh Rust instead of
copying Python's reviewed behavior. When in doubt, read `skit-oracle/src/skit/*.py` and match it.

## 2. How the work is structured (two phases, two modes)

- **Port phase (DONE): large-scale fan-out.** One Python test module → one Rust
  `crates/<crate>/tests/port_test_<module>.rs`, produced by a **tests-only** subagent (writes tests,
  reports gaps, NEVER edits production — so it structurally cannot reinvent), paired with an
  independent **adversarial verify** subagent that tries to refute "faithful". Driver:
  `Workflow` tool, script at `/tmp/.../scratchpad/port_wave.js` (persisted per-invocation; the path
  is printed by the tool). 13 waves ran this way.
- **Fix phase (IN PROGRESS): serialized implementation subagents, NOT fan-out.** Production edits
  overlap and need review, so run ONE focused implementation subagent per cluster (or 2 in parallel
  only if clearly-separate crates). Each **translates the oracle impl**, then **deletes the
  `#[ignore]` line(s)** from the corresponding FAILING CONTRACT test(s) so they go green — that is the
  proof the fix matches the oracle. The supervisor (you) re-runs and verifies every delivery; never
  trust a subagent's self-report.

### The fix-loop (the exact procedure for each divergence)
1. Read the oracle impl (`skit-oracle/src/skit/*.py`) for the behavior + the FAILING CONTRACT test's
   `#[ignore]` reason (it has the oracle line refs + what Rust does).
2. Translate the behavior into the Rust production code.
3. Delete the `#[ignore = "FAILING CONTRACT (divergence): ..."]` line (leave the test body intact).
4. Run the crate's whole suite `cargo test --locked -p <pkg> --all-targets --all-features` — the
   un-ignored test now PASSES and nothing else broke. If a SIBLING test asserted the OLD (divergent)
   behavior, it was encoding the divergence — correct it to v0.4 and note it.
5. Gates: `cargo fmt --check -p <pkg>`, `cargo clippy --locked -p <pkg> --all-targets --all-features
   -- -D warnings`, `RUSTDOCFLAGS='-D warnings' cargo doc --locked -p <pkg> --no-deps`.
6. Supervisor verifies: re-run, confirm the un-ignore, run the FULL workspace test (the aggregate
   should show ignored dropping by exactly the number un-ignored, 0 failed), commit.

## 3. Completed work (all committed; `git log 052dcd3..HEAD`)

- **Store data-safety translation** (`2aebe6f`, `c04395c`): S1 resolve→NotFound for corrupt/missing
  meta (exit 127); S2 corrupt registry.toml → `.corrupt` backup; A2 opportunistic read-path
  self-heal (`_repair_rows` under a non-blocking lock; `resolve` never self-heals); A1 replace-retry
  test seam. **A2 reversed codex's "pure reads" design to match v0.4** — `rust-contract-matrix.md`
  rescoped (reads-never-migrate = user data; the registry cache is the oracle-defined self-heal
  exception). Reversible if the user objects.
- **13 fan-out waves → 81 port_test files, 2840 passing.** Every module verified against the oracle,
  gaps adjudicated, gates green before commit. The adversarial verify stage caught a weakened/
  mislabeled assertion in **8 modules** (declared_params, js_deps×6, review_fixes,
  add_validation_contracts, editor, config_cmd, prompt_kind, prompt_cli, prompt_tui, path_tui,
  tui_responsive, settings_atomicity, reset_default_ui, add_feedback_contracts, cli) — every one
  corrected against the oracle before commit. This is the whole point: the verify stage is what stops
  the reinvention/weakening the user hated.
- **Tooling adjudicated N/A**: `benchmarks_tooling` (Rust `skit-benchmarks` crate has 92 native
  tests) + `mutation_gate` (Rust uses cargo-mutants). Not portable 1:1.
- **First fix-pass change** (`5574ff1`, task #9 done): restored the v0.4 Library term (工具库/工具庫,
  not codex's 程序库/程式庫) across the catalog + `detect_locale` zh negotiation (zh-MY/zh-XX/bare-zh →
  ZhCn). Un-ignored 5 failing contracts; also fixed a latent defect (Ratatui `TestBackend`
  interleaves a blank cell after each wide glyph, so `contains("工具库")` never matched — assertions
  now use the spaced form `"工 具 库"`).

## 4. In-flight / needs immediate attention

- **launcher `describe` totality fix — NOT STARTED (agent stalled with no changes; tree is clean).**
  Do this next. DIVERGENCE: oracle `launcher.describe_command` (launcher.py:117-133) is total/
  side-effect-free — for an unknown kind returns `meta.template` (usually ""), never raises. Rust
  `build_launch_preview` (crates/skit-runtime/src/launch.rs) returns `Err(LaunchError::UnknownKind)`.
  FIX: make ONLY the preview/describe path total (return the stored template as the preview) — do NOT
  touch `build_launch_plan` (the run path stays refusing). Un-ignore
  `test_unknown_kind_describe_returns_template_and_never_raises` in
  crates/skit-cli/tests/port_test_langs.rs.
- **review-lane Ctrl+O / Ctrl+E fix — was RUNNING when the session ended; CHECK `git status` for
  uncommitted changes and either verify+commit or discard+redo.** Two divergences in the add/review
  TUI (crates/skit-tui/src/screens/add.rs, session.rs): (A) Ctrl+O opens the candidate picker
  UNCONDITIONALLY — oracle (tui_add.py:1471-1472) makes it a no-op when detected placeholders ≤
  LIST_PREVIEW_LIMIT; add the gate. (B) Ctrl+E binds → EditSource with NO focus check
  (add.rs:363/374) — oracle: Ctrl+E in a focused Input is end-of-line, never opens the editor; gate
  it on focus. Un-ignore `test_review_choose_variables_key_is_harmless_for_a_short_prompt` and
  `test_review_ctrl_e_in_input_is_end_of_line_not_editor` in port_test_prompt_tui.rs.
  **NOTE: these A/B are REAL production defects the port surfaced (see the fix agents' handoff notes).**

## 5. Not-yet-done: the fix-pass backlog (~85 divergences, by cluster)

Each has oracle line refs in the FAILING CONTRACT `#[ignore]` reason + the ledger adjudication log.
Recommended order: small/clear first (bank the loop), then the big clusters, `edit_declared` last.

- **launcher describe** (§4) + **doctor uv exit** (skit-cli: uv "not required" for a pure library →
  exit 0, not 1; healthcheck.py; un-ignores langs #18/#20, healthcheck) — small, clear.
- **review-lane** (§4) — Ctrl+O gate, Ctrl+E focus.
- **config-display** (skit-cli cli.rs): the `config` command lost its human display layer — default
  sentinels ("default ($VISUAL / $EDITOR)", "auto (deno > bun > node)"), padded list columns,
  "k = v" (space-padded) set confirmation, errors that NAME the valid choices, the paused-axis
  "switched off" notice. cli.py:5301-5326, 5509, 5523. (port_test_config_cmd.rs, ~10 divergences.)
- **add-lane (#15, big cluster, skit-cli cli.rs)**: the drafts-boundary guard is ABSENT (refuse
  `--exe/--ref/--kind exe` on a kept draft — cli.py:1894-1933); a resumed draft is NOT consumed on
  success (cli.py:258-266); the editor lane validates AFTER opening $EDITOR (should validate first —
  cli.py:309-329); `kind_for_draft` shebang-first classifier is MISSING (a .py draft with a bash
  shebang stores kind=python not shell — registry.py:442-473); `--python -/none/blank` normalization
  (case-insensitive, blank→auto — cli.py:279-281); unknown-kind refusal voices ("isn't a script..."
  / "names no interpreter..." vs the generic "could not infer" — cli.py:2040-2070); the interactive
  deps/python re-ask loop is ABSENT (cli.py:224-261). Spread across port_test_add_lane_contracts.rs,
  port_test_add_validation_contracts.rs, port_test_add_feedback_contracts.rs,
  port_test_draft_inference_and_reader_cli.rs (all deduped to the same shapes).
- **prompt (#14 + more, skit-cli)**: add name derivation keeps `.prompt` (`p.prompt.md`→`p-prompt`
  not `p`, store.py:571); stdin `add -` with no name defaults to 'stdin' vs requiring --name; stdin
  whitespace body accepted vs "Nothing arrived on stdin"; `{{目标}}` unicode placeholders undetected →
  empty fields (**task #14**); deleted prompt body → run exits 2 "invalid entry mutation" not 127;
  126 unknown-runner lists no names; empty-runner-list run lacks "No agents configured" + recovery;
  edit surfaces no reconcile hint; the params HUMAN read view drops the unmanaged/"(use --manage...)"
  listing (only --json carries it). **Plus a genuine internal bug the port found: the add lane trims
  `--runner " claude "` but the run lane does NOT** — make them consistent. (port_test_prompt_cli.rs
  ~10, port_test_prompt_kind.rs.)
- **params/edit (#16, big — reimplementation-scale)**: `edit_declared` (params.py:352-472) is a whole
  pure WARN-AND-CONTINUE batch editor returning `DeclEditResult{decls, warnings}` with 9 closed
  warning codes (already-declared, not-declared, bad-delivery/type/default, choice-without-choices,
  not-a-placeholder, env-source-not-secret, bool-flag-on-by-default), reverting a bad row but keeping
  the batch — ABSENT in Rust, which fail-fast-aborts on the first bad row (cli.rs:3762-3973).
  `reconcile.edit_specs` (--resync/--remove/--secret/--prompt) is inlined private in cli.rs — expose
  it. The params/edit CLI exits 2 (Usage) where v0.4 warns+exits 0 (bad --type/--prompt) or refuses
  with exit 1 (--resync on a reference entry; edit a non-editable kind). `[[parameters]]` unknown-key
  preservation: oracle `to_toml_dict` passes raw param dicts through (keep-unknown-fields,
  models.py:112-113); Rust's typed `to_meta_map` always adds `type` + drops unmodeled keys
  (parameters.rs:340-349). (port_test_params_edit.rs 36 absent, port_test_declared_params.rs,
  port_test_edit.rs.)
- **data-safety**: shim writes the plaintext-secret injected copy to `entry_dir` unconditionally;
  oracle writes OS-temp-first (rewrite.py:176-180) so a crash never persists a secret in the store —
  fix `stage_injected_source` (command.rs:686-693). settings-save npm-clear atomic refusal is ABSENT
  (tui_submit_settings never clears node_modules; only the deps command does). `deps`-clear must
  sweep node_modules (js_deps).
- **launcher/interpreters**: bun `run` subcommand + refusal-message wording (interpreters);
  interpreters DETECTION half (shebang_program/infer_kind) still OWED as a skit-language port (58
  cross-crate stubs point there — a coverage obligation, not a divergence).
- **misc wording**: kindnames picker labels ("A program (run it directly)" / "A prompt for an AI
  agent" — kindnames.py:50-52); PEP 440/508 error wording; name-conflict "taken" vs "already
  exists"; etc.
- **uvman orphan-pin completeness** → re-home to a skit-runtime white-box unit test (private
  CHECKSUMS unenumerable from an integration test).

## 6. After the fix pass (Phase 3–5, do NOT start early)

- Phase 3 gates: 100% executable-source line coverage (`cargo llvm-cov` + `scripts/check_coverage.sh`
  — do NOT relax it), i18n completeness (3 locales), ASD-STE100 English, cargo deny/audit, zizmor,
  docs build, benchmark budget, Maturin wheel + `uv tool install` smoke.
- **User's hands-on test (task #19)** fits AFTER the fix pass restores behavior, BEFORE mutation.
- Mutation (`cargo mutants`, task #20) is **BLOCKED on explicit user approval** — it invalidates on
  any change, so it runs only after behavior is frozen; ~4.5h local, likely on another box/modal.com.
- Phase 4: multi-round read-only independent review. Phase 5: re-record demo assets (main's
  `docs/assets/demo/{demo,shots}.tape` are the product spec — the acceptance harness runs them
  unchanged; the 8 tui-*.png + demo-mouse.gif were deleted on this branch and must be restored),
  delete CLAUDE_HANDOFF.md + this HANDOFF.md + mutation artifacts, open a non-draft PR.

## 7. Working mode (the user's rules — follow exactly)

- **Translate the oracle, never reinvent.** Read `skit-oracle/src/skit/*.py`. Match its behavior,
  messages, edge cases, exit codes.
- **Verify through the composition root.** A skit-ui/skit-tui test green while skit-cli never wires
  the feature = false pass. Grep for a production caller. (Memory: skit-verify-through-composition-root.)
- **Supervisor re-runs and adjudicates every subagent delivery.** Never trust self-reports. The
  verify stage catches weakenings; when it flags one, fix it against the oracle (do not accept a
  softened/tautological/no-op assertion, a gutted divergence body, or a wrong-evidence stub).
- **A divergence keeps its FULL asserting body** under `#[ignore = "FAILING CONTRACT (divergence):
  <oracle evidence>"]`. Stubs only for genuinely-absent-API or off-crate. `#[ignore]` NEVER hides a
  mismatch.
- **Fan-out subagent safety (learned the hard way):** a subagent once ran `rm -rf .locks values` in
  the repo root on a guess (harmless — skit's own scratch — but a real risk). The port_wave contract
  now forbids deleting anything the agent didn't create AND requires sandboxing the real `skit`
  binary's SKIT_DATA/STATE/CONFIG_DIR to temp on every call. Keep those rules on any shell-capable
  fan-out.
- **Commits: NO trailers** in this repo (no Co-Authored-By / Claude-Session). `git commit -F -` with a
  heredoc. Commit fix and its un-ignore together; keep the tree green.
- **Two policy items keep their oracle-matching defaults** (user did not object): the store self-heal
  reversal (`c04395c`) and fixing shim secret crash-safety. Reversible if the user changes their mind.

## 8. Hard-won knowledge / gotchas (save the next agent hours)

- **The CLI crate's package name is `skit-cli-rs`, NOT `skit-cli`** (dir is crates/skit-cli). `-p
  skit-cli --all-features` errors "packages outside of workspace". Always `-p skit-cli-rs`.
- **Ratatui `TestBackend` interleaves a blank cell after each wide (CJK) glyph** — assert `"工 具 库"`
  not `"工具库"`. A no-space `contains` silently never matches (and makes negative assertions
  vacuous). See render.rs:170.
- **An empty filter needle does NOT run `apply_filter`** (falls back to the default sort) — to test a
  comparator you must use a non-empty needle. (path_tui tiebreak.)
- **~1 in 8 subagents stalls mid-stream (API error).** The Workflow reports which; the fix is a
  1-module relaunch, not a full re-run. A failed agent may have left NO changes (check `git status`).
- **Reducer/render driving (skit-tui):** present a screen with `state.update(Action::Present(...))`;
  persistent session across events uses one `TuiSession`; `render_with_session(...)` calls
  `begin_render`(sync); reachable geometry = `ViewGeometry.hits`, `ChoicePickerGeometry` (Done/rows),
  `FilePickerGeometry`, `AddScreenGeometry` (skit_tui re-exports `AddScreenSession::focused()`), and
  buffer contents. UNreachable (use `// private-render:` / `// render-model:` notes): widget region
  heights, `run_modal::picked_path_text`, LineInput cursor, Textual rich-markup.
- **Verify a FAILING CONTRACT is real:** `cargo test ... -- --ignored --exact <name>` must fail at the
  REAL last assertion (not setup).
- **The full-workspace aggregate command** (use it to confirm a fix): pipe `cargo test --locked
  --workspace --all-targets --all-features` through the awk one-liner that sums passed/failed/ignored
  (see the session log). Baseline after the port: **2840 / 0 / 1179**.
- **`skit-benchmarks/src/suites/micro.rs` and `dataset.rs`** had the two compile errors codex left;
  already fixed early this session.

## 9. Pointers

- **Ledger (authoritative):** `docs/design/python-test-port-ledger.md` — every module, its crate/file,
  status, and the adjudication log with per-divergence oracle refs. Wave blocks + the "Wave N" and
  per-module adjudication entries are the detailed record.
- **Memory:** `~/.claude/projects/-home-ubuntu-coding-skit/memory/` — MEMORY.md index +
  skit-python-test-port.md (workstream), skit-few-long-lived-subagents.md (fan-out vs serialized +
  safety), skit-rust-rewrite-superset-rule.md, skit-verify-through-composition-root.md,
  skit-review-principles.md, skit-commit-trailers.md.
- **Contract matrix:** `docs/design/rust-contract-matrix.md` (22 rows, still "In progress" — a release
  blocker until Complete with evidence).
- **AGENTS.md / CLAUDE.md:** product rules, trust model (skit is a launcher, NOT a sandbox — do not
  add sanitization/threat-mitigation for trusted local content), architecture, gates.
- Commands to run gates are in AGENTS.md "## Commands".
