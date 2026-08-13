# skit Rust rewrite — session handoff (2026-08-12)

**Read this first, then `docs/design/python-test-port-ledger.md` (the authoritative per-module
record + fix-list).** This file is SELF-CONTAINED: it does not depend on any `.claude` memory (that
is machine-local and may be gone on a new box). Everything you need is here or in the repo.
Supersedes the stale `CLAUDE_HANDOFF.md` (codex's; delete it at Phase 5).

Branch: `rewrite/rust-ratatui-complete-20260808-codex`. The oracle is this repo pinned at
`origin/main@206f9ef` (v0.4.1.dev0). Any `/home/ubuntu/coding/...` path below is this machine's —
**see §0.5 to recreate the oracle checkout and the rest on a new box.**

---

## 0. One-line status

**Port COMPLETE. Impl-fix pass WELL UNDERWAY: 43 fix commits landed, 109 FAILING CONTRACTs closed
(107 removed/un-ignored + 2 re-labeled white-box), 2 stubs promoted, 2 owed white-box units added.
The last fully green recorded baseline was workspace 2887 pass / 0 fail / 1134 ignored. Five
reviewed PR #44 waves added 96 executable parity tests plus 8 completeness-manifest tests.
177 FAILING CONTRACT attributes remain (§5 has the per-file map). JS deps and responsive
implementation divergences are closed; next continue prompt/add clusters.** The user
chose plan **A**:
finish the whole port FIRST (done), THEN one comprehensive impl-fix pass (in progress).

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
- **The v0.4 zh translations** live IN the oracle worktree at
  `skit-oracle/src/skit/locales/{zh_CN,zh_TW}/LC_MESSAGES/skit.po`. Every zh string you add to
  `crates/skit-i18n/src/lib.rs` MUST be copied verbatim from there (msgid lookup), never invented —
  codex-invented rows are themselves divergences (several were found and fixed). Extraction
  one-liner pattern (regex over msgid/msgstr pairs) is in the session log; a python3 heredoc with
  `re.finditer(r'msgid ((?:"..."\s*)+)msgstr (...)')` + unicode_escape decoding works.
- **The demo harness** (`/home/ubuntu/coding/skit-harness`, Phase 5 ONLY): a VHS/Docker frame-compare
  harness. The product SPEC — the tapes — lives IN this repo at `docs/assets/demo/{demo,shots}.tape`
  (`shots.tape` has deterministic `Screenshot` points; `demo.tape` is the keystroke choreography).
  Phase 5 replays them UNCHANGED against the Rust binary and diffs frames. Operating knowledge from
  the prior demo work: Docker volume paths must be ABSOLUTE; chain build `&&` record (a failed build
  silently records the PREVIOUS image); the image build fails under contention with a host
  `cargo test` (don't run them together). The 8 `tui-*-{en,zh}.png` + `demo-mouse.gif` + mp4s were
  deleted on this branch and must be re-recorded at Phase 5.
- **Devbox caveat:** this box was MISSING some interpreters (fish / pwsh / deno / ruby / lua / R).
  Interpreter-dependent tests SKIP rather than fail — a skip is NOT a pass. Install them, or expect
  skips (and don't chase a "green" that is really a skip). This box HAS `/usr/bin/vi` — see §8 for
  the vi-hang trap that follows from that.

## 1. The mission (do not lose this)

v0.5.0 must be a **strict superset** of the v0.4 oracle `origin/main@206f9ef`. 可以多，不能少 (more
allowed, less never). The Rust rewrite must be a **faithful TRANSLATION** of the Python
implementation — read the Python impl AND the Python tests, transcribe the behavior. Do NOT reinvent
Rust logic. The user was repeatedly furious about exactly that: inventing fresh Rust instead of
copying Python's reviewed behavior. When in doubt, read `skit-oracle/src/skit/*.py` and match it.

## 2. The fix-loop (the exact procedure — it works; 13 commits ran it verbatim)

1. Read the oracle impl (`skit-oracle/src/skit/*.py`) for the behavior + the FAILING CONTRACT
   test's `#[ignore]` reason (it has the oracle line refs + what Rust does).
2. Verify the contract is real: `cargo test ... -- --ignored --exact <name>` must fail at the REAL
   last assertion (not setup).
3. Translate the behavior into the Rust production code. If a user-visible string changes, pull the
   zh rows verbatim from the oracle `.po` (§0.5) and update `crates/skit-i18n/src/lib.rs` (remove
   replaced rows — the catalog is a linear-scan array, thematically grouped, no order/dup test).
4. Delete the `#[ignore = "FAILING CONTRACT (divergence): ..."]` line (leave the test body intact).
5. Run the crate suite `cargo test --locked -p <pkg> --all-targets --all-features`. **Expect 1-3
   SIBLINGS to fail** — every cluster this session had siblings asserting the OLD divergent
   behavior, including tests whose NAME claims "v0.4" (`doctor_keeps_the_v040_fresh_install_uv_check`
   asserted the divergence itself). Correct them to the oracle and say so in the commit.
6. Gates: `cargo fmt --check -p <pkg>`, `cargo clippy --locked -p <pkg> --all-targets
   --all-features -- -D warnings`, `RUSTDOCFLAGS='-D warnings' cargo doc --locked -p <pkg>
   --no-deps`.
7. Full workspace aggregate (`cargo test --locked --workspace --all-targets --all-features` piped
   through the awk sum, §8). **The ignored count must drop by EXACTLY the number un-ignored; passed
   grows by that plus any new tests. Do the arithmetic every time** — a discrepancy means a sibling
   changed silently. Takes ~7-9 min; background it or `timeout 550`.
8. Commit fix + un-ignores + sibling corrections together (message format: what diverged, the
   oracle refs, what was un-ignored, which siblings were corrected and why, the new aggregate).
   Then update the ledger row(s) in `docs/design/python-test-port-ledger.md` (`**X FIXED <sha>**`
   convention) as a separate `docs(ledger):` commit.

## 3. Fix-pass work COMPLETED (all committed; sequence `git log 052dcd3..HEAD`)

Pre-session (previous agents): store data-safety (`2aebe6f`,`c04395c`), 13 port waves, i18n Library
term + zh negotiation (`5574ff1`), review-lane Ctrl+O/Ctrl+E (`22b9773`).

This session (2026-08-11/12), in order — each closed the named contracts:

| Commit | What | Contracts |
|---|---|---|
| `8af2d92` | launch preview total for unknown kind (returns template; display byte-matches flows.py:915 env-prefix line) | 1 |
| `a8e2480` | doctor uv rules: exit 0 for non-empty no-python library (`UvHealth::NotRequired` now produced); python never launch-blocked over uv | 3 (+2 siblings) |
| `3bb4fbf` | config command display layer: localized unset sentinels, padded `{k:<16}` list in v0.4 key order (`CONFIG_KEYS` exported from skit-store), `k = v` confirmation, paused-axis stderr notice, choice-naming refusals (store `normalize_setting` voices) | 10 |
| `8633128` | params read-view empty/unmanaged voices: "<name> has no managed parameters." (analyzer-less OR reader-driven → early return; ref/copy advice split), "Detected but not yet managed: ..." tail voices | 8 |
| `dc58131` | kind-picker labels: `kind_choice_label` in skit-i18n ("A program (run it directly)" / "A prompt for an AI agent"); badge `kind_label` unchanged | 2 |
| `19e5ab8` | bun gets `run` subcommand; JS refusal = oracle text naming candidates + `skit config js.runner` hint (`JsRuntimeMissing`, exit 126, pinned-missing takes same voice) | 2 |
| `7977386` | uv checksum error carries expected+actual digests; dir-fsync swallowed post-rename; unpinned triple = typed `NoPinnedChecksum` (white-box unit) not a panic | 1 (+2 re-labeled) |
| `76aaab1` | owed orphan-pin completeness white-box unit (CHECKSUMS == producible triples, digest shape) | — |
| `c5e84ea` | PEP 440/508 refusal wording: "<value> isn't a package requirement (e.g. ...)" / "... isn't a Python version constraint (e.g. ...)", raw value, parser reason dropped | 6 |
| `592d236` | name-conflict voices: create = "The name X is already taken — pick another name." (`Conflict{name}`), rename = "The name X is already taken." (new `RenameConflict{name}`); slug no longer leaked | 2 |
| `1cac4a4` | editor resolution total (config > $VISUAL > $EDITOR > vi/notepad, blank falls through, unbalanced-quote → raw argv[0]); edit-lane voices ("Saved X." + drift hint, reference-gone guard BEFORE launch, "no editable source ... run as-is" exit 1, "Could not launch the editor (...)" exit 1, editor rc IGNORED on edit lane) | 6 |
| `c1964dc` | promoted 2 now-observable editor stubs (whitespace-config→VISUAL, all-blank→vi via PATH shim) | — |
| `54bf6c6` | JavaScript dependency inputs: preserve first-seen import order; skip empty requirements; correct three sibling assertions that pinned sorted output | 6 |
| `c3e02b8` | mirror environment truthiness: empty npm, Python index, and Python install values are unset; any nonempty alias still defers | 3 |
| `6aedb9b` | reference JS/TS add skips implicit dependency scanning; explicit `--dep` still refuses before create; one opposite sibling corrected | 1 |
| `05e8b9d` | JavaScript installer commands: exact npm/bun/deno argv and unknown runner fallback to npm | 2 |
| `dc770fc` | reference-mode dependency refusals: distinct oracle add/deps voices with exact EN/zh-CN/zh-TW rows | 3 |
| `f8f80f5` | add `--python` normalization: trim explicit values; case-insensitive `-`/`none` remain explicit automatic constraints | 2 |
| `8cf75fe` | enabled four composition-root PEP refusal contracts that `c5e84ea` had already restored; removed stale divergence notes | 4 |
| `cc0ead1` | JS deps cleanup: transactionally clear manifest/lock/node_modules without an ownership heuristic; sweep legacy `.injected-*` only when older than one hour; preserve fresh/cutoff files and symlink targets | 4 |
| `d7575c5` | dependency-free JS/TS modules: exact minimal private/type manifest, same bytes do not rewrite, invalid/different bytes replace transactionally, no installer or stamp | 3 |
| `2c1f6ef` | prompt runner picks: trim explicit override, remember only successful actual picks, preserve last state for unknown/default/pinned lanes across plain and enhanced forms | 1 |
| `f1dc3c7` | i18n replacement: unique reachable English keys and oracle Library wording in both Chinese catalogs | replacement/fix |
| `7e53da4` | config recovery: exact empty language clear; Python-compatible backup-directory copy; recoverable backup failure continues with exact three-language warning | replacement/fix |
| `07dfbce` | prompt placeholders: Unicode XID scan/render with byte-preserved decomposed names; command template identifiers remain ASCII | 3 |
| `a9e1452` | add flag refusals: exact v0.4 shell/command `--dep` and stdin `--ref` voices with EN/zh-CN/zh-TW end-to-end coverage | 3 |
| `462aa9e` | responsive detail pin: Tab/key/mouse actions carry actual rendered visibility; reducer pins the requested open/closed state across tiers and resizes | 2 |
| `219a136` | JS install artifacts: exact manifest/order/parser, real entry cwd, node_modules SHA-256 marker, fresh/resolve ordering, persistent-lock backup transaction and rollback | 4 |
| `ef7c1ef` | responsive compact controls: short env picker keeps its input visible; focused Add fields scroll their full control span into view | 2 active-red tests |
| `061d29c` | responsive Search: short terminals use a one-row borderless input without losing filtering or cursor behavior | 1 active-red test |
| `9238165` | responsive Preferences: narrow mirror choices stack while wide choices remain horizontal | 1 active-red test |
| `3d43a40` | add-time prompt runner validation: exact unknown-runner voice lists configured runners before draft/editor/entry writes | 3 |
| `157f9b7` | bare add pipe lanes: exact three-language v0.4 advice names only stdin/prompt/template lanes that work without a terminal | 2 |
| `c2187b7` | editor add lane: exact three-language `--edit --no-input` refusal explains the conflict and gives the working stdin spelling | 1 |
| `affa5b2` | add Python pins: exact three-language note announces only shebang-derived `requires-python`; explicit and PEP 723 pins stay silent | 4 |
| `c2ec8b6` | PowerShell defaults: preserve runtime scalar literal types independently from the static-type fallback; unknown types remain degraded | 1 PR #44 active-red + additive matrix |
| `45516b3` | prompt token grammar: scanner and renderer exclude brace-adjacent/triple-stache tokens and reserved `prompt` names | 3 |
| `059e24b` | prompt runner diagnostics: preserve a blank raw runner name for inspection while valid runners and frontend identities still exclude it | 1 |
| `0996295` | raw run conflicts: exact three-language v0.4 guidance for `--set`, `--preset`, and `--save-preset` | 3 |
| `796a5d1` | field-less preset save: name the entry and reuse the exact standalone preset refusal before any state write | 1 |
| `18ec1a0` | run `--set`: trim only the parameter name while preserving the value bytes after the first equals sign | 1 |
| `0b8bf2d` | reused run arguments: emit the exact three-language notice only for an implicit non-raw replay | 2 |
| `27bbb4b` | run `--set` validation: report all malformed and unknown names, list valid names or `—`, and mutate values only after complete validation | 3 |

PR #44 is actively continuing. The last corrected integrated accounting is 64/84 behavior modules
and 1318/3018 Python contracts. Its merge ancestry and complete test snapshot are preserved on
`integration/pr44-20260812` at corrected tip `cdae0a9` (PR pin `af05498`); do not use the earlier `0b36bf7` result,
which inherited an upstream denominator error (60 instead of 78 `test_store.py` functions and
3000 instead of 3018 total contracts). The integration workspace passes
`cargo test --locked --workspace --all-targets --all-features --no-run`. Reviewed green waves on
this branch include parser mutation
contracts (`184726d`), argstate filesystem contracts (`40b6087`), atomic state contracts
(`817f14c`), two non-duplicate boolean parameter-edit guards (`7fcc177`), and packaging distribution
contracts (`606c716`). The i18n replacement is green after production fixes (`f1dc3c7`..`5d3c303`).

Two policy items keep their oracle-matching defaults (user did not object): the store self-heal
reversal (`c04395c`) and shim secret crash-safety (§5 data-safety, still to implement). Reversible.

## 4. Verified baseline (re-verify on arrival)

```
git status --short          # only stray .coverage (untracked, leave it)
cargo test --locked --workspace --all-targets --all-features | <awk aggregate, §8>
# => 2887 passed / 0 failed / 1134 ignored
grep -rh '#\[ignore = "FAILING CONTRACT' crates --include='*.rs' | wc -l   # => 177
```

The full-workspace benchmark target previously had one intermittent timing failure in
`process_timeout_terminates_the_complete_descendant_tree`; 126 exact/parallel/full-binary reruns
all passed. `08046bd` hardened the test with separate 3-second pipe-holder and 250ms marker-writer
descendants, a 20ms product timeout, and a diagnostic 1-second return budget. Production did not
change; exact stress, full benchmark crate, format, and Clippy passed.
The product workspace excluding
`skit-benchmarks` most recently passed 2878 / 0 / 1134 before the six JS-deps contracts were
un-ignored. The language/runtime suites and `port_test_js_deps` are green at `81c99e7`.

## 5. REMAINING work — 177 FAILING CONTRACTs by file (fix-pass backlog)

Recommended: keep banking coherent clusters, one commit per cluster. Biggest-first is fine now that
the loop is proven; `edit_declared` (params/edit) last as before. Counts are exact as of `27bbb4b`.

- **44 port_test_prompt_cli.rs + 3 port_test_prompt_kind.rs + 9 port_test_prompt_utf8.rs — the
  prompt cluster (#14).** Add name derivation keeps `.prompt` (`p.prompt.md`→slug `p`, store.py:571
  removesuffix); stdin `add -` no name → defaults 'stdin'; stdin whitespace body accepted; `{{目标}}`
  unicode placeholders undetected → empty fields (**the #14 analyzer defect** — likely in
  skit-language's prompt placeholder scan, check `\w`-class vs oracle regex); deleted prompt body →
  run exits 2 "invalid entry mutation" not 127; exit-126 unknown-runner must list configured names;
  empty-runner-list run needs "No agents configured" + recovery; `_reconcile_prompt_after_edit`
  (the edit-lane prompt half — the generic drift hint is DONE `1cac4a4`, the prompt reconcile
  offer is NOT); params human read view for prompts (placeholder table via `_show_command_params`);
  **genuine internal bug: add lane trims `--runner " claude "`, run lane does not — make
  consistent**. Oracle: cli.py prompt lanes, store.py:571, langs/prompt/*.
- **0 port_test_js_deps.rs implementation divergences — JS dependency materialization.** First-seen scanner order and empty
  requirements are fixed in `54bf6c6`; mirror empty-value precedence is fixed in `c3e02b8`;
  reference implicit scanning, installer argv/fallback, and refusal voices are fixed in
  `6aedb9b`, `05e8b9d`, and `dc770fc`. Transactional explicit cleanup and the aged legacy injected
  sweep are fixed in `cc0ead1`; minimal dependency-free module manifests are fixed in `d7575c5`.
  Add-lane refusal voices are fixed in `a9e1452`. The full manifest/stamp/install transaction is
  fixed in `219a136`; the 88 remaining ignores in that file are classified architecture,
  cross-crate, private-helper, or absent-public-seam ports rather than `FAILING CONTRACT` markers.
  Oracle: langs/javascript/deps.py.
- **16 port_test_cli.rs — mixed add/File-not-found voices.** "File not found" phrase (oracle
  `_require_exists`/`_require_file`, cli.py:420-428) vs Rust "could not resolve ...os error 2";
  plus add-lane overlaps (see add-lane cluster).
- **16 port_test_add_validation_contracts.rs + 9 port_test_add_lane_contracts.rs + 6
  port_test_add_feedback_contracts.rs + 6 port_test_draft_inference_and_reader_cli.rs + 13
  port_test_add_no_source.rs + ~8 of port_test_editor.rs (the `-e` lane) — the add-lane cluster
  (#15), the biggest.** Drafts-boundary guard (refuse `--exe/--ref/--kind exe` on a kept draft,
  cli.py:1894-1933); resumed draft not consumed on success (cli.py:258-266); editor lane must
  validate BEFORE opening $EDITOR (cli.py:309-329); `kind_for_draft` shebang-first classifier
  (registry.py:442-473 — a `.py` draft with a bash shebang stores kind=shell); `--python -/none/blank`
  normalization (case-insensitive, blank→auto, cli.py:279-281); unknown-kind refusal voices
  ("isn't a script..." / "names no interpreter..." — cli.py:2040-2070); interactive deps/python
  re-ask loop (cli.py:224-261); `-e` lane name prompt/interactivity gate; untouched-draft exit 0
  "Nothing was added" + unlink vs Rust's exit-2 keep; awk-shebang → ask, not python-fallback;
  `_offer_create_in_editor` decline = clean exit 0 (Rust: 130 Aborted).
- **14 port_test_dependency_command_contracts.rs** — deps belt refusals before entry existence etc.
- **14 port_test_declared_params.rs + 3 port_test_edit.rs + 2 of port_test_editor.rs (params
  resync) + port_test_params_edit.rs (36 tests currently ABSENT-stubbed) — the params/edit cluster
  (#16), reimplementation-scale, LAST.** `edit_declared` (params.py:352-472): pure warn-and-continue
  batch editor returning `DeclEditResult{decls, warnings}` with 9 closed warning codes, reverting a
  bad row but keeping the batch (Rust fail-fast-aborts, cli.rs:3762-3973 region); expose
  `reconcile.edit_specs`; exits: warn+0 (bad --type/--prompt), refuse 1 (--resync on reference,
  non-editable kind) vs Rust's blanket Usage 2; `[[parameters]]` unknown-key preservation
  (models.py:112-113 pass-through vs typed `to_meta_map` dropping unmodeled keys,
  parameters.rs:340-349).
- **2 port_test_run_set.rs** — run --set lane divergences.
- **6 port_test_config.rs** — config store-level leftovers (distinct from the DONE config_cmd).
- **Data-safety (in js_deps + elsewhere):** shim writes the plaintext-secret injected copy to
  `entry_dir` unconditionally; oracle stages OS-temp-first (rewrite.py:176-180) so a crash never
  persists a secret — fix `stage_injected_source` (crates/skit-cli/src/run/command.rs:686-693
  region). settings-save npm-clear atomic refusal (tui_submit_settings never clears node_modules);
  `deps`-clear must sweep node_modules.
- **TUI: 5 path_tui + 4 tui_nav + 2 tui_responsive + 2 draft_and_reader_tui + 1 reset_default_ui.**
- **Small:** 2 agent_install, 2 default_semantics_review_fixes, 1 default_name_resolution, 1 show,
  1 entrypoint, 1 dependency_write_validation (unclassifiable-file-outside-drafts, ties add-lane),
  1 flows.
- **OWED (not divergences): the interpreters DETECTION half** — port the oracle's
  shebang_program/infer_kind test module against `skit-language` (58 cross-crate stubs in
  port_test_interpreters.rs point there; tests-only coverage work, could be a fan-out subagent job
  per the §7 port mode).

Each contract's `#[ignore]` reason carries the oracle line refs + what Rust does. The ledger has
the per-module adjudication log.

## 6. After the fix pass (Phase 3–5, do NOT start early)

- Phase 3 gates: 100% executable-source line coverage (`cargo llvm-cov` + `scripts/check_coverage.sh`
  — do NOT relax it), i18n completeness (3 locales), ASD-STE100 English, cargo deny/audit, zizmor,
  docs build, benchmark budget, Maturin wheel + `uv tool install` smoke.
- **User's hands-on test** fits AFTER the fix pass restores behavior, BEFORE mutation.
- Mutation (`cargo mutants`) is **BLOCKED on explicit user approval** — it invalidates on any
  change, so it runs only after behavior is frozen; ~4.5h local, likely on another box/modal.com.
  (Local mutants runs show ~9-15% survivors on ANY branch including pristine main; the CI nightly
  on 3.13/hosted runner is the authoritative zero-survivor gate.)
- Phase 4: multi-round read-only independent review. Phase 5: re-record demo assets (main's
  `docs/assets/demo/{demo,shots}.tape` are the product spec — the acceptance harness runs them
  unchanged; the 8 tui-*.png + demo-mouse.gif were deleted on this branch and must be restored),
  delete CLAUDE_HANDOFF.md + this HANDOFF.md + mutation artifacts, open a non-draft PR.

## 7. Working mode (the user's rules — follow exactly)

- **Translate the oracle, never reinvent.** Read `skit-oracle/src/skit/*.py` at the pin
  `origin/main@206f9ef` (NOT the `v0.4.0` tag — an earlier audit used the tag and had to be
  redone). Match its behavior, messages, edge cases, exit codes. Cost is not a factor: "永遠遵守長
  期最優解和最佳架構，任何技術決策只考慮用戶體驗和實現優雅，不考慮實現成本，不接受最小或最簡單修
  復。可以多，不能少。" Prefer a maintained crate over hand-writing a widget. Feature loss is a
  RELEASE BLOCKER, tracked in `docs/design/rust-contract-matrix.md`.
- **Implementation happens in the MAIN context, serially** (the user's later preference confirmed:
  plan → adversarial review → main-context implementation; no blind implementer fan-out). This
  session ran 13 clusters that way with zero rework. Fan-out is ONLY for mechanical tests-only work
  (like the owed detection port), each paired with an adversarial verify subagent.
- **Verify through the composition root — layer-local green is NOT proof.** PROVEN CASE:
  `LibraryEntryDetail` had complete impls + passing tests but skit-cli never fed it (dead in the
  binary). For any restored capability, grep for a production caller outside `tests/`. End-to-end
  proof is the VHS tape harness (Phase 5).
- **Review principles:** (1) assert what the user gets, not what the code drew; (2) resolve against
  the current set on every read (cached keys are as stale as cached indexes); (3) when you find a
  defect, ask where else it lives (this session: the same invented-zh-translation defect existed in
  "No such file", the PEP rows, AND the conflict rows); (4) stop when a slice stops being coherent
  and hand back a GREEN tree.
- **A divergence keeps its FULL asserting body** under `#[ignore = "FAILING CONTRACT (divergence):
  <oracle evidence>"]`. Stubs only for genuinely-absent-API or off-crate. `#[ignore]` NEVER hides a
  mismatch. When an impl fix makes a stubbed behavior observable, PROMOTE the stub to a real test
  (precedent: `c1964dc`).
- **Commits: NO trailers** (NO `Co-Authored-By:`, NO `Claude-Session:`). `git commit -F -` with a
  heredoc, plain subject/blank/body. ASD-STE100-ish English. Commit a fix and its un-ignores
  together; keep the tree green. Ledger updates as separate `docs(ledger):` commits.
- Subagent safety (if you fan out): forbid deleting anything the agent didn't create; sandbox
  `SKIT_DATA/STATE/CONFIG_DIR` to temp on every real-binary call; expect ~1 in 8 to stall
  (relaunch just that one).

## 8. Hard-won knowledge / gotchas (save the next agent hours)

- **THE vi-HANG TRAP (new, cost this session an hour):** editor resolution now falls back to `vi`
  (v0.4 behavior). Any test that invokes `edit`/`add --edit` WITHOUT pinning an editor now launches
  the REAL `/usr/bin/vi`, which HANGS the suite under `.output()` capture (stdin is null but vi
  retries reads). Every such test must set `EDITOR`/`VISUAL`/config editor, or empty the `PATH`
  (`env("PATH", empty_tempdir)`) when the vi-fallback itself is under test. If the full suite
  hangs: run per-test-binary with `timeout 100 cargo test --test <name>`, bisect to the file, then
  `-- --exact` per test. Kill leftovers with `pkill -9 -f target/debug`.
- **zh rows come from the oracle .po, verbatim** (§0.5). If a catalog row you're replacing has no
  .po counterpart, suspect the ENGLISH text is also invented — find the oracle msgid first.
- **The CLI crate's package name is `skit-cli-rs`, NOT `skit-cli`** (dir is crates/skit-cli).
- **Ratatui `TestBackend` interleaves a blank cell after each wide (CJK) glyph** — assert
  `"工 具 库"` not `"工具库"`. A no-space `contains` silently never matches.
- **An empty filter needle does NOT run `apply_filter`** (path_tui tiebreak tests need a non-empty
  needle).
- **Multi-line `#[ignore]` attributes exist** (backslash continuations in port_test_interpreters,
  uvman). Line-based un-ignore scripts must handle them (regex over the whole attribute, or match
  the first line and skip until `"]`).
- **The full-workspace aggregate command:**
  ```
  cargo test --locked --workspace --all-targets --all-features 2>&1 \
    | grep -E "^test result" \
    | awk -F'[ .;]+' '{p+=$4; f+=$6; i+=$8} END {print "passed="p" failed="f" ignored="i}'
  ```
  ~7-9 min. Run per-crate suites during the loop; full workspace before each commit.
- **Verify a FAILING CONTRACT is real before fixing:** `cargo test ... -- --ignored --exact <name>`
  fails at the REAL last assertion.
- **Reducer/render driving (skit-tui):** `state.update(Action::Present(...))`; one `TuiSession`
  across events; `render_with_session` calls `begin_render`; reachable geometry =
  `ViewGeometry.hits`, `ChoicePickerGeometry`, `FilePickerGeometry`, `AddScreenGeometry`, buffer
  contents. UNreachable: widget region heights, `run_modal::picked_path_text`, LineInput cursor.
- **skit-store `settings()`/`get()` return RAW machine tokens**; the HUMAN display layer
  (sentinels) lives in skit-cli `config_display_value`. Tests reading raw values go through
  `--json`.
- **`resolve_editor_argv` / `launch_editor` / `report_saved_edit`** (cli.rs) are the edit-lane
  seams now; `open_editor_in` (add lane) still refuses editor rc≠0 — the add-lane cluster will
  adjudicate that against cli.py:309-329.
- **Message hole order:** all oracle zh translations so far keep the En `{}` hole order, so
  positional holes suffice. If you hit a translation that reorders holes, that's the deferred
  "positional→named format holes" work item — surface it, don't bodge it.

## 9. Pointers

- **Ledger (authoritative):** `docs/design/python-test-port-ledger.md` — every module, its
  crate/file, status (`**X FIXED <sha>**` convention), and the adjudication log with per-divergence
  oracle refs.
- **Contract matrix:** `docs/design/rust-contract-matrix.md` (22 rows, still "In progress" — a
  release blocker until Complete with evidence).
- **AGENTS.md / CLAUDE.md:** product rules, trust model (skit is a launcher, NOT a sandbox — do not
  add sanitization/threat-mitigation for trusted local content), architecture, gates. Commands to
  run gates are in AGENTS.md "## Commands".
- **Machine-local `.claude` memory is NOT a dependency** — everything essential is inlined here.
