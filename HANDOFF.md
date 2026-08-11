# skit Rust rewrite — session handoff (2026-08-10)

**Read this first, then `docs/design/python-test-port-ledger.md` (the authoritative per-module
record + fix-list).** This file is SELF-CONTAINED: it does not depend on any `.claude` memory (that
is machine-local and may be gone on a new box). Everything you need is here or in the repo.
Supersedes the stale `CLAUDE_HANDOFF.md` (codex's; delete it at Phase 5).

Branch: `rewrite/rust-ratatui-complete-20260808-codex`. The oracle is this repo pinned at
`origin/main@206f9ef` (v0.4.1.dev0). Any `/home/ubuntu/coding/...` path below is this machine's —
**see §0.5 to recreate the oracle checkout and the rest on a new box.**

---

## 0. One-line status

**The Python behavior port is COMPLETE and green (workspace 2842 pass / 0 fail / 1177 ignored, tree
clean, 81 `port_test_*.rs` files). The impl-fix pass has STARTED — 2 clusters done (i18n
`5574ff1`, review-lane `22b9773`); ~80 v0.4 divergences remain. Next: launcher `describe` (§4).** The user chose plan **A**: finish the whole port FIRST (done), THEN one comprehensive
impl-fix pass (in progress).

---

## 0.5 Environment — recreate local checkouts FIRST on a new machine

Local absolute paths in this doc were `/home/ubuntu/coding/...`; on a new box they won't exist.
Recreate them (all are derivable from this one git repo):

- **The Rust workspace** = THIS repo, branch `rewrite/rust-ratatui-complete-20260808-codex`. Wherever
  you clone it, that is what this doc calls `<repo>` / `/home/ubuntu/coding/skit`. `cd` there.
- **The oracle** (the v0.4 Python source you translate FROM) is a **git worktree of this same repo**
  at the pinned commit `206f9ef946fc45835cb2479593794431f2620c32` (it IS in this repo's history:
  "docs(readme): Add text to avoid scaring off CLI-newbies"). Recreate it:
  ```
  git -C <repo> worktree add ../skit-oracle 206f9ef946fc45835cb2479593794431f2620c32
  ```
  After that, the Python impl is at `../skit-oracle/src/skit/*.py` and the tests at
  `../skit-oracle/tests/test_*.py`. **Everywhere this doc writes `skit-oracle/...` or
  `/home/ubuntu/coding/skit-oracle/...`, it means that worktree.** (Fallback if worktrees are
  awkward: `git clone` the repo elsewhere and `git checkout 206f9ef...` in it.)
- **The demo harness** (`/home/ubuntu/coding/skit-harness`, Phase 5 ONLY): a VHS/Docker frame-compare
  harness. The product SPEC — the tapes — lives IN this repo at `docs/assets/demo/{demo,shots}.tape`
  (`shots.tape` has deterministic `Screenshot` points; `demo.tape` is the keystroke choreography).
  Phase 5 replays them UNCHANGED against the Rust binary and diffs frames. Operating knowledge from
  the prior demo work: Docker volume paths must be ABSOLUTE; chain build `&&` record (a failed build
  silently records the PREVIOUS image); the image build fails under contention with a host
  `cargo test` (don't run them together). The 8 `tui-*-{en,zh}.png` + `demo-mouse.gif` + mp4s were
  deleted on this branch and must be re-recorded at Phase 5.
- **The port fan-out script** `port_wave.js` lived in a session scratchpad (gone). You do NOT need it
  for the fix pass (that uses serialized subagents, not fan-out — §7). If you ever re-port modules,
  rewrite it per the §2 contract: a `Workflow`-tool script, one tests-only subagent per module
  (reads oracle impl + test, writes `crates/<crate>/tests/port_test_<module>.rs`, reports gaps) each
  paired with an adversarial verify subagent.
- **Devbox caveat:** this box was MISSING some interpreters (fish / pwsh / deno / ruby / lua / R).
  Interpreter-dependent tests SKIP rather than fail — a skip is NOT a pass. Install them, or expect
  skips (and don't chase a "green" that is really a skip).

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
  `Workflow` tool with a script (the port ran one called `port_wave.js`; the `Workflow` tool persists
  each invocation's script and prints its path — the session copy is gone, see §0.5). 13 waves ran
  this way.
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

## 4. Fix pass done so far / next up

- **DONE — i18n cluster** (`5574ff1`): Library term + zh negotiation (§3).
- **DONE — review-lane Ctrl+O / Ctrl+E** (`22b9773`): Ctrl+O now no-ops for a short prompt (gated on
  PROMPT_LIST_PREVIEW_LIMIT); Ctrl+E in a focused review Input is end-of-line, EditSource only when
  no Input owns focus. Un-ignored both port_test_prompt_tui tests; a sibling `add_workflow.rs` seam
  test was adapted to a capped (21-hole) prompt so it still exercises the picker. skit-tui+skit-ui
  391/0.
- **NEXT (not started) — launcher `describe` totality.** DIVERGENCE: oracle
  `launcher.describe_command` (launcher.py:117-133) is total/side-effect-free — for an unknown kind
  returns `meta.template` (usually ""), never raises. Rust `build_launch_preview`
  (crates/skit-runtime/src/launch.rs) returns `Err(LaunchError::UnknownKind)`. FIX: make ONLY the
  preview/describe path total (return the stored template as the preview) — do NOT touch
  `build_launch_plan` (the run path stays refusing). Un-ignore
  `test_unknown_kind_describe_returns_template_and_never_raises` in
  crates/skit-cli/tests/port_test_langs.rs. (A subagent stalled on this before writing anything; tree
  was clean.) Then continue §5.

## 5. Not-yet-done: the fix-pass backlog (~85 divergences, by cluster)

Each has oracle line refs in the FAILING CONTRACT `#[ignore]` reason + the ledger adjudication log.
Recommended order: small/clear first (bank the loop), then the big clusters, `edit_declared` last.

`#N` below are the (machine-local, will-be-gone) task-list IDs, kept only as shorthand — the
descriptions here + in the ledger are authoritative. Legend: **#14** = prompt analyzer defects
(incl. `{{unicode}}` placeholders undetected); **#15** = refuse the add-lane inputs v0.4 refuses
(drafts-boundary guard etc.); **#16** = restore params batch fault tolerance (`edit_declared` warn-
and-continue); **#17** = give the English/ASD-STE100 gate an oracle-derived exemption; **#18** = let
a translation reorder its `{}` placeholders (positional→named format holes); **#19** = prepare a
build for the user's hands-on test; **#20** = run mutation testing (BLOCKED on explicit user
approval).

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

## 7. Working mode (the user's rules — follow exactly; inlined so nothing is lost)

- **Translate the oracle, never reinvent.** Read `skit-oracle/src/skit/*.py` at the pin
  `origin/main@206f9ef` (NOT the `v0.4.0` tag — an earlier audit used the tag and had to be redone).
  Match its behavior, messages, edge cases, exit codes. Cost is not a factor: "永遠遵守長期最優解和最
  佳架構，任何技術決策只考慮用戶體驗和實現優雅，不考慮實現成本，不接受最小或最簡單修復。可以多，不能
  少。" Prefer adding a maintained crate over hand-writing a widget — hand-rolled components from this
  assistant have been consistently broken. Feature loss is a RELEASE BLOCKER, tracked in
  `docs/design/rust-contract-matrix.md`.
- **Verify through the composition root — layer-local green is NOT proof.** skit's layering
  (`skit-ui` reducer → `skit-tui` renderer → `skit-cli` composition root) lets a capability be fully
  implemented and fully tested at the UI layer while the composition root never feeds it. PROVEN
  CASE: `LibraryEntryDetail` / `LibraryState::from_surface` / `Action::ReplaceSurface` had complete
  impls + passing tests, but skit-cli called `LibraryState::from_scan(scan)` (no `details`) — so the
  detail pane, the missing-target `⚠`, activity sort, and remove-confirm were all DEAD in the running
  binary. This class is invisible to per-crate tests, coverage, AND mutation. For any restored
  capability, **grep for a production caller outside `tests/`** before believing it works. The
  end-to-end proof is the VHS tape harness (Phase 5).
- **Review principles (each from a defect that passing tests missed):**
  1. **Assert what the user gets, not what the code drew** (a "scrollbar exists" test passed while
     the focused control wasn't rendered; assert the focused control's rect is inside the viewport).
  2. **Resolve against the current set on every read** — a cached key is as stale as a cached index
     (runner-select value-keyed, preset checkboxes name-keyed, settings focus field-keyed, positional
     `{}` format holes — same bug in four domains: don't assume hole/key order is stable).
  3. **When you find a defect, ask where else it lives** (the viewport-follow bug existed on two
     screens; the second was found only because the question was asked).
  4. **Stop when a slice stops being coherent, and say so** — park it and hand back a GREEN tree
     rather than bending tests to fit a half-finished change (a half-landed feature "looks finished in
     a diff" — that is exactly how this branch accumulated its regressions).
- **Supervisor re-runs and adjudicates every subagent delivery.** Never trust self-reports; re-run,
  grep the composition root, adjudicate against the oracle. When the verify stage flags a weakening,
  fix it (never accept a softened/tautological/no-op assertion, a gutted divergence body, or a
  wrong-evidence stub). Welcome being overturned — the implementation agent corrected the supervisor
  five times and was right each time.
- **A divergence keeps its FULL asserting body** under `#[ignore = "FAILING CONTRACT (divergence):
  <oracle evidence>"]`. Stubs only for genuinely-absent-API or off-crate. `#[ignore]` NEVER hides a
  mismatch.
- **Subagent modes:** mechanical, non-overlapping, tests-only work → large-scale fan-out (the
  `Workflow` tool + `port_wave.js`), tests-only + adversarial verify. Production edits / anything
  touching a shared file → FEW long-lived serialized subagents, supervisor verifies (the codex
  disaster was 4 agents editing one worktree). **Fan-out safety (learned the hard way):** a subagent
  once ran `rm -rf .locks values` in the repo root on a guess (harmless — skit's own scratch — but a
  real risk); forbid deleting anything the agent didn't create, and sandbox the real `skit` binary's
  `SKIT_DATA/STATE/CONFIG_DIR` to temp on every call. Expect ~1 in 8 subagents to stall (API); relaunch
  just that one.
- **Commits: NO trailers** in this repo (NO `Co-Authored-By:`, NO `Claude-Session:`). `git commit -F -`
  with a heredoc, plain subject/blank/body. If a trailer slips in, `git commit --amend`. Commit a fix
  and its un-ignore together; keep the tree green.
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
- **Machine-local `.claude` memory is NOT a dependency** — its essential content (superset rule,
  composition-root verification + proof case, review principles, subagent modes + fan-out safety,
  commit-trailer rule, environment rebuild) is inlined into §0.5 and §7 of THIS file. Do not rely on
  it existing.
- **Contract matrix:** `docs/design/rust-contract-matrix.md` (22 rows, still "In progress" — a release
  blocker until Complete with evidence).
- **AGENTS.md / CLAUDE.md:** product rules, trust model (skit is a launcher, NOT a sandbox — do not
  add sanitization/threat-mitigation for trusted local content), architecture, gates.
- Commands to run gates are in AGENTS.md "## Commands".
