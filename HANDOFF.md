# skit Rust rewrite — session handoff (2026-08-21)

**Read this first, then `docs/design/python-test-port-ledger.md` (the authoritative per-module
record + fix-list).** This file is SELF-CONTAINED: it does not depend on any `.claude` memory (that
is machine-local and may be gone on a new box). Everything you need is here or in the repo.
Supersedes the stale `CLAUDE_HANDOFF.md` (codex's; delete it at Phase 5).

Branch: `rewrite/rust-ratatui-complete-20260808-codex`. The oracle is this repo pinned at
`origin/main@206f9ef` (v0.4.1.dev0). Any `/home/ubuntu/coding/...` path below is this machine's —
**see §0.5 to recreate the oracle checkout and the rest on a new box.**

---

## 0. One-line status

**The broad Rust port and the implementation-fix pass are almost complete. The last fully certified
candidate passed 4,054 workspace tests with 0 failures and 526 classified ignores. A fresh
committed-state LCOV run at that checkpoint covered every target and feature, and
`scripts/check_coverage.sh` returned
`complete executable-source line coverage`; no checker rule or exclusion changed. Workspace Clippy
and Rustdoc pass with warnings denied, and 0 `FAILING CONTRACT` attributes remain. Multi-round
oracle/PR/main review found and closed the late transaction, parser, runtime, plain-form, and i18n
defects instead of treating lower-layer tests as product proof. The fixed PR #44 head was audited
as a diff; only stronger owners were folded into the consolidated targets. Its raw split files and
manifests were rejected.

Phase 5 has re-recorded and visually checked 12 localized 1280x780 screenshots (four screens in
English, Simplified Chinese, and Traditional Chinese). Supply-chain, workflow, English, i18n, and
documentation, packaging, and benchmark gates pass locally. A final authority audit then fixed
wrong-typed runtime metadata, zero-test CI gates, benchmark occurrence accounting, and benchmark
workflow cache isolation. Their owning suites and static workflow gates pass; the successor must
rerun the fresh workspace LCOV/full/static checkpoint on the pushed SHA. Remaining release blockers
are that checkpoint, the user hands-on check, native macOS/Windows and declared-runtime CI evidence,
and the mutation result. The contract matrix is 20 Complete / 1 In progress. The user explicitly
authorized the handoff push on 2026-08-21; that push starts PR #45 CI and mutation.** The user
chose plan **A**:
finish the broad port first, then keep the implementation-fix and review passes open until the
release evidence is final.

### Immediate successor checklist

1. Treat the pushed PR #45 head as the only candidate. Record its exact SHA and do not reuse the
   stale `865e568` check rollup.
2. Run the full workspace test and fresh workspace LCOV commands, then run the coverage checker.
   The previous 4,054/0/526 count predates the final metadata and benchmark-manifest owners.
3. Run workspace Clippy and Rustdoc with warnings denied. Focused green evidence already exists for
   `skit-store`, `port_test_prompt_kind`, `runner_management_transaction`, and all benchmark targets.
4. Watch the three-platform CI, declared-runtime gates, release build-only workflow, and mutation
   workflow. The POSIX shell, CPython 3.13, and Windows uv exact gates now reject zero-test success;
   Node and Fish owners become mandatory only after their pinned setup steps.
5. Ask the user to complete the hands-on terminal checklist. Do not mark the last matrix row
   Complete until native/runtime CI, hands-on, and mutation are green.
6. If product code stays unchanged, rerun the local release package and benchmark commands only if
   the final binary or evidence policy requires a new exact-SHA receipt. The prior package, budget,
   docs, supply-chain, and demo receipts remain useful historical evidence, not proof for a changed
   SHA.

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
  `cargo test` (don't run them together). Phase 5 recorded 12 tracked PNGs:
  `{library,form,add,settings}` x `{en,zh-CN,zh-TW}`. The three MP4 recordings are valid but remain
  deliberately untracked. The old hand-recorded `demo-mouse.gif` is not generated by the current
  tapes and must not be restored as if it were Rust UI evidence.
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

## 2.9 Mutation matrix verdict and the timeout root cause (2026-08-23)

MUTATION RUN 32655362435 (head `67f2f21`, 48 shards): 48/48 shards FAILED. The aggregate
counts are 91 missed, 5,719 timeouts, and two infrastructure deaths. The timeouts are ONE
defect, not thousands.

ROOT CAUSE (primary-source verified): cargo-mutants 27.1.0 with `test_workspace = true` and
`--shard` calibrates each shard's automatic test timeout from a baseline that tests ONLY the
shard's mutated package (observed: `cargo test --package=skit-runtime` in shard 29's
baseline.log, 1 s), while every mutant then runs the FULL workspace suite (observed:
`cargo test --workspace` in the same shard's mutant logs). The docs promise the opposite
("all tests from the workspace are run for the baseline and against each mutant" when
test_workspace is set) — an upstream defect in sharded runs; no fixed release exists
(27.1.0 is current). The derived budget max(20, 3.0 x ~1 s) = 20 s then times out honest
full-suite runs, which need ~95 s on these runners (measured from completed MISSED runs:
80-96 s). Kill latency depends on the mutated crate's position in `cargo test`'s serial
binary order, which explains the exact block boundary: shards 0-16 (application, benchmarks,
cli — killing tests run early) mostly caught; shards 17-47 (domain, form, i18n, language,
runtime, store, tui, ui — killing tests sit after skit-cli's long suite) avalanched with
156-198 timeouts of 206. Even the "healthy" shards' budgets (90-102 s) sat BELOW the honest
95 s suite, so most of their timeouts were the same artifact. The local 25-mutant smoke run
(14/25 timeouts) was the same signature, visible months earlier.

CONSEQUENCE: roughly 65% of the workspace's 9,877 mutants have NEVER been honestly
adjudicated. The relaunch is not a formality; expect the missed list to grow well past 91,
concentrated in the eight crates the avalanche covered.

THE FIX (this commit):
- `--timeout 300` (explicit, ~3x the honest 95 s suite) replaces the automatic calibration.
  cargo-mutants has no config key for it, so it lives on the command line in mutation.yml
  and AGENTS.md; the contracts script pins both.
- `minimum_test_timeout` and `timeout_multiplier` left `.cargo/mutants.toml`: with an
  explicit `--timeout` they govern nothing, and a knob that does not govern misreads as
  policy. A new contract rejects their return.
- The baseline STAYS (no `--baseline=skip`): with the explicit timeout its miscalibration
  is inert, and it remains the only in-environment check that the suite passes inside the
  mutants tree copy. Skipping it would convert any environmental breakage into a silent
  false kill of every mutant in the shard — fake zero survivors the fail-closed aggregate
  cannot see.
- 48 -> 64 shards and `timeout-minutes: 300 -> 360`. Shard-17 artifact data: root-crate
  (domain) mutants pay 136 s mean workspace relink each; at 206 mutants and an honest
  ~80 s test, the projection is 5.5 h — an 8% margin against the 6 h hard ceiling with a
  fail-closed aggregate on the line. At 64 shards the worst class projects ~4.1 h (31%
  margin). The contracts script derives matrix/`--shard`/`SHARD_COUNT` consistency, so the
  count lives in one place.

VALIDATION (receipts): Before/after pair on three
avalanche-class mutants (delete match arm "python"/"shell"/"fish" in
build_launch_plan_inner, skit-runtime/src/launch.rs):
- BEFORE (old flags, local): auto-set test timeout 20 s from a package-scoped baseline
  (`cargo test --package=skit-runtime`, 1 s); all three mutants TIMEOUT while the full
  workspace suite was progressing honestly (receipts: session scratchpad valA/mutants.out,
  debug.log "Auto-set test timeout to 20s", per-mutant logs killed mid-suite).
- AFTER (explicit --timeout, local, 900 s to absorb this host's slower suite): all three
  CAUGHT, 0 timeouts, 18 minutes total (receipts: scratchpad valB/mutants.out, caught.txt=3,
  debug.log "test: Some(900s)"). The CI budget stays 300 s, calibrated to the measured
  80-96 s honest suite on the runners.

CLASSIFICATIONS:
- The 91 MISSED are REAL survivors (46 skit-benchmarks, 45 skit-cli/src/cli.rs). Every
  spot-checked one shows Build Success + Test Success with the full suite completing in
  80-95 s: the tests ran, passed, and the mutant lived. Killing owners are the next wave;
  the full list is in the shard artifacts (missed.txt) and the session scratchpad
  (missed-91.txt).
- Shard 8: runner eviction at 1h46m ("The runner has received a shutdown signal") after the
  same avalanche pattern — rerun noise, no separate defect.
- Shard 40: the unmutated baseline itself failed — skit-tui `terminal_pty` lifecycle owners
  panicked instantly ("PTY output closed before the generic form appeared") inside the
  cargo-mutants tree copy, while shards 41-46 ran the identical baseline green. A
  nondeterministic environment sensitivity in that owner. It goes on the killing-owner
  backlog with this note: under mutation, a flaky FAILURE is a false KILL (it catches every
  mutant in whatever shard it lands on), so this flake is a correctness risk to the gate,
  not just rerun noise.
- Genuine-hang mutants (the skit-benchmarks python_random bit-op class) will still be
  TIMEOUT under any honest budget, and the aggregate fails on non-empty timeout.txt. The
  resolution is killing tests that fail BEFORE the hang — killing-owner wave work, not a
  budget question.

RELAUNCH PLAN AND COST: label stays attached; the next push to the PR re-queues the 64-shard
matrix. Projected ~155 runner-hours per full run (64 shards x ~2.4 h average; worst class
~4.1 h), matrix wall ~8 h at 20-way concurrency. If margins run tighter than projected, the
next lever is a dedicated build profile without debuginfo to cut the 136 s root-crate
relink — noted, not implemented; the failure was calibration, not build speed.

NOTE: the `mutation-requested` label is REMOVED from PR #45 (user decision: do not re-run
the matrix yet — too slow). Re-add the label when the full 64-shard adjudication should run.
The 91-survivor killing-owner wave waits for that full honest list.
## 2.10 Walkthrough-findings wave (branch `fix/tui-findings-20260823`, base 67f2f21)

Commit `9e4fab9` recorded this wave as complete. Independent verification found four live defects
and reclassified the discard question. This section supersedes that record.

Round 1 made these changes:

| Commit | Result after verification |
|---|---|
| `2aa692a` | Fixed the `?` Help key. `UiBinding::accepts` now ignores Shift for character keys, and the binding tests drive both terminal event shapes. |
| `32ea213` | Fixed one clipped Settings control by drawing it into a scratch buffer. It did not fix top-clipped copy in Settings, top-clipped rows in Add, or top-clipped rows in Preferences. The scratch design also allocated the complete hidden control. |
| `b2b0344` | Added only a test. The test supplied slug-sorted input, so it did not expose the missing production tie-break. The library order and initial selection were still nondeterministic for equal activity values. |
| `820c583` | Fixed draft deletion. Focus now reports the highlighted draft to the reducer, as version 0.4 does. |
| `2a9bee5` | Corrected seven add-receipt strings. It did not test or correct the plaintext-agent sentence at the end of the same oracle block. |
| `2ba714b` | Removed the third discard-question copy from the panel title and corrected the three discard translations. The body and the shared screen header still show two copies by design. |

The supervisor correction round makes these changes:

| Finding | Correction |
|---|---|
| Library activity tie | `replace_surface` now sorts by activity descending and then slug ascending. The reducer contract starts with reversed rows, repeats with shuffled rows and complete details, and holds both the order and selected slug. |
| Top clipping in Settings, Add, and Preferences | One `RowClip` helper paints only the visible band. Wrapped paragraphs use Ratatui paragraph scrolling. Option lists iterate only visible source rows. The Settings scratch allocation and blit are gone. Each screen has a short-viewport test that asserts surviving later-row content. |
| Chinese add receipt | The final plaintext-agent sentence now matches the zh-CN and zh-TW `.po` files byte for byte. The catalog test covers every receipt msgid from `.po` lines 595-632. |
| Discard question | The two-copy rendering and its test stay unchanged. `docs/design/rust-contract-matrix.md` records the screen-header deviation from version 0.4. |

Classified faithful, no change: Esc on the library quits (`back_or_quit`, tui.py:323/676-684,
`show=False` so unadvertised is correct); Esc leaves search keeping the filter (only refocuses,
same `Back to list` chip, tui.py:463). The en-vs-zh final-frame difference is the proven VHS
raster-lag artifact class — key mapping never reads the locale.

## 3. Fix-pass work COMPLETED (all committed; sequence `git log 052dcd3..HEAD`)

Pre-session (previous agents): store data-safety (`2aebe6f`,`c04395c`), 13 port waves, i18n Library
term + zh negotiation (`5574ff1`), review-lane Ctrl+O/Ctrl+E (`22b9773`).

This session (2026-08-11 through 2026-08-14), in order — each closed the named contracts:

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
| `33d9a4a` | typed run values: reuse the exact form-validation voice across run, prompt, and extra-argument paths | 3 |
| `7bc5c5a` | malformed runner-container recovery: exact localized human voice while machine tokens remain stable | 1 |
| `b439a5e` | dry-run injection transparency: show only masked values and prove no state, source, or staged-file write | 1 |
| `6189684` | eager completion: show/install completion actions take priority over a simultaneous version flag | 1 |
| `19cc2ff` | prompt add names: derive `thing` from the compound `thing.prompt.md` suffix without changing source storage | 2 |
| `0f1fdd9` | missing prompt bodies: classify exact missing payload/read failures as localized Not Found (127) before dry-run output | 2 |
| `9105a6d` | prompt intake UTF-8: reject malformed file/stdin snapshots before onboarding or repository writes, with exact source and byte offset | 4 |
| `41f2ec0` | prompt argv validation: exact localized NUL-byte and over-platform-limit refusals without leaking the rejected prompt | 3 |
| `f38666b` | prompt edit UTF-8: retain the authored invalid bytes transactionally for repair, but refuse the edit and omit the success voice | 2 |
| `4d2c052` | agent skill install: share exact CLI/TUI success and destination-write failure voices while retaining the nested store cause | 2 |
| `076d04c` | runner removal recovery: quote the stable-name command path without changing the rejected duplicate rows | 1 |
| `959e35f` | prompt UTF-8 reads: one typed path+offset decoder for run, params, and doctor launch-blocked diagnostics | 3 |
| `085a4b3` | prompt runner refusals: distinct localized no-selection, empty-config, and unknown-runner diagnostics with stable known-name lists | 4 |
| `2087afa` | prompt add summary: explain disabled insertion and flood-cap auto-management outcomes | 2 |
| `d92e264` | runner blank names: use the shared required-name refusal before any configuration seeding | 2 |
| `53bdb1c` | prompt stdin boundary: require an explicit name and a nonblank decoded body before draft/store writes | 2 |
| `e64407a` | runner command validation: exact four-way argv reason voices while preserving typed pre-write rejection | 1 |
| `eeed834` | prompt params policy: operation-failure exits and exact runner/interpolation recovery, including the cleared-pin read voice | 4 |
| `22016c2` | add source preflight: exact localized File-not-found diagnostics before prompt confirmation, kind selection, or writes | 3 |
| `c134c80` | add path classification: unknown shebangs name the interpreter gap and offer explicit kind or executable recovery | 2 |
| `4fc1659` | typed add sources: reject directories with exact localized Not-a-file diagnostics while preserving explicit executable handling | 2 |
| `de754cf` | unreadable add sources: exact localized read failure before any store write, without changing missing or directory classification | 1 |
| `5d0d888` | enabled four complete editor contracts that `1cac4a4` had already restored; removed stale divergence markers | 4 |
| `62a7df8` | post-editor Python flags: exact draft-kind refusal while retaining the short kept-draft notice and no-write behavior | 2 |
| `c6d14ec` | bare Markdown add: non-interactive recovery names the prompt lane without changing the generic shebangless cases | 1 |
| `7c4cb53` | generic shebangless path recovery: exact basename-based script/program guidance while preserving stdin, shebang, explicit-kind, and kept-draft lanes | 4 |
| `7c3d615` | edit unknown entry: an explicit interactive decline exits cleanly without launching the editor or writing an entry; EOF remains abort | 1 |
| `ceece3b` | JavaScript Python-constraint refusals: exact shared add/deps voice for dash, none, and empty spellings without changing validation order or writes | 3 |
| `ea4389c` | add-editor name conflicts: explicit names are rejected through the PTY before editor or draft side effects; the final atomic conflict gate remains | 1 |
| `0335ed1` | secret parameter transitions: report the sorted names of purged plaintext values in human mode while JSON remains a single machine document | 2 |
| `ea80ec4` | add-editor explicit flags: reject malformed Python constraints and dependencies before editor or draft side effects | 2 |
| `0e6d826` | doctor malformed-runner recovery: exact three-language repair command, stable multi-row order, and pure JSON output | 1 |
| `ab901c4` | reclassified two duplicate Python store-boundary names as architecture closures; kept their ignored bodies and stronger CLI owners | — (2 re-labeled) |
| `f0dfcff` | runner removal targets: callback-level exactly-one validation, negative-row handling, pre-write refusal, and semantically equivalent zsh completion | 1 |
| `41a8146` | source-default reconciliation: publish only values coercible to the declared type while preserving the original typed value | 2 (integration + private unit translation) |
| `f6516fa` | TUI add input: Ctrl+D deletes the next focused text character; draft shortcut, footer click, and confirm-before-delete stay active | 1 |
| `7555a0e` | editor draft classification: unknown shebangs refuse with exact recovery-before-kept order, preserve the draft, and do not create an entry | 1 |
| `98b6bc8` | file picker navigation: PageUp/PageDown share Home/End semantics without closing or unfiltering the modal | 1 |
| `b8cd947` | Python match captures: count single-segment `case NAME:` bindings without treating qualified value patterns as captures | 1 |
| `add3c47` | params resync: refuse a source operation when a copy entry has no stored payload, before any metadata or state write | 1 |
| `39f9bd0` | run passthrough: let only explicit current extra arguments satisfy an empty required flag; remembered arguments still validate | 1 |
| `a5710d9` | moved the config axis-display exact contract from the raw store projection to its executable private CLI owner | 1 translated stale-green |
| `669abd1` | human params output: mask present secret defaults and last values while JSON and missing values keep their machine meaning | 2 |
| `06b4c99` | doctor entry counts: exact singular/plural taxonomy and oracle zh-CN/zh-TW translations; JSON stays numeric | 1 |
| `52ef759` | prompt params read view: disabled insertion returns the exact localized recovery notice before candidate or item rendering | 1 |
| `0fd6d85` | picked-path glob spelling: match Python `glob.escape` for `[`, keep Replace literal, and prove downstream re-glob selects only the picked file | 1 |
| `4a8aa74` | prompt params schema edits: refuse all metadata-schema operations while insertion is off, before body reads or writes | 1 |
| `3d9c8ba` | declared env sources: warn and preserve bytes for public values; apply same-batch secret transitions before the trimmed env source | 1 |
| `7f7be0d` | untouched editor drafts: safely unlink owned empty script/prompt drafts and report the exact typed success notice | 3 |
| `4746c36` | params source management: classify reference and command no-copy refusals as operation failures before any source, metadata, or state write | 2 |
| `623989f` | TUI mouse routing: ignore movement, button release, and drag events over Preferences, Run, and generic Form click targets while preserving left-button and wheel paths | regression fix (+2 tests) |
| `ef5c5a2` | runner removal confirmation: real PTY yes/no and two confirmation-window CAS races; runner n/EOF use frozen failure exit 1 while entry removal remains abort 130 | 4 cross-crate owners + production parity |
| `a219125` | JavaScript removal locking: move two exact owners to `skit-store`; prove wait-before-delete, persistent lock inode, and byte-exact no-write on lock-open failure | 2 cross-crate owners |
| `f5423d4` | inject field projection: move three exact owners to `RunFormView::from_declarations` and retain typed control/default/secret/env/binding assertions | 3 cross-crate owners |
| `0e9f082` | filesystem glob expansion: move two exact owners to the real store adapter; retain sorted/native/hidden assertions and add literal plus recursive `**` coverage | 2 cross-crate owners |
| `24e9510` | prompt editor lifecycle: ask and validate the name before authoring, share the three-language starter, clean untouched drafts, and refuse collisions before editor launch | 5 cross-crate owners + production parity |
| `f8091da` | prompt TUI public events: real mouse runner selection plus pinned/unpinned rerun through the live host, real child markers, and last-value replay | 2 cross-crate owners + 1 stronger owner |
| `0f76635` | selected prompt runner preflight: scoped ProgramNotFound returns from the form to Library with the exact localized error and no child/state write | 1 cross-crate owner + production parity |
| `c9d78af` | noninteractive prompt edit reconciliation: report body-order unmanaged placeholders with the frozen 20-item preview and localized tail, using the validated snapshot | 2 divergences |
| `949a51a` | zero-runner prompt TUI: open the shared runner editor, keep normal Ctrl+N cancel semantics, and re-enter the same form after a successful save | 2 cross-crate owners + production parity |
| `ccec2c6` | malformed prompt tweaks: warn and continue for missing `=` or an empty name without writing payload, metadata, or state; other malformed axes stay fatal | 2 divergences |
| `0c64921` | source-managed parameter edits: exact post-commit receipt for resync/manage/unmanage/shared tweaks, byte-preserving no-op resync, JSON/normalize exclusions, and frozen three-language copy | 1 divergence |
| `8eb9b03` | Add Review navigation: true Up/Down and Tab/Shift+Tab focus movement plus bidirectional clickable footer targets, without stealing arrows from Kind or open selects | 1 divergence + stronger PR body fold |
| `7c37d80` | first managed-reader transition: announce when a modeled reader form is set aside after the first successful human `--manage`; keep JSON, dynamic readers, and later edits silent | 1 divergence |
| `afeae66` | Preferences navigation: expose shared forward/back commands through the real TUI event fallback and reducer while Input, Radio, and open Select controls keep their own keys | 1 divergence + stronger PR body fold |
| `0d1a294` | bare-directory intake: noninteractive unknown directories get the frozen `--exe` recovery before reads or writes; typed directories, explicit executables, and interactive consent remain separate | 1 divergence |
| `9718a1a` | current-directory picker row: private pinned selection, keyboard and mouse acceptance, filter/empty behavior, exact three-language label, and a real root `.` observable | 2 divergences + stronger PR body folds |
| `4ba75ec` | prompt editor `--no-input`: replace the non-TTY surrogate with a true PTY owner and the exact three-language pipe recovery without editor or store writes | 1 divergence + stronger PR body fold |
| `8253219` | show JSON schema: retain the frozen 21-key body as a v0.5 strict-superset closure and pin the exact 25-key union, metadata types, identity, snapshot hash, and field shape in the active owner | 1 re-labeled version closure |
| `878826b` | declared parameter edits: exact post-commit human receipt after purge, JSON purity, empty-list dash, and separation from source-managed receipts | 1 divergence |
| `6f193e6` | template riders: new command/prompt names outside the real placeholder set default to environment delivery while real placeholders, other kinds, and explicit delivery keep their paths | 2 divergences |
| `fa1e9f3` | Settings live defaults: move the exact owner to the real CLI/TUI host, reconcile only display clones, use source-only Python repr, and prove open/cancel byte-no-write | 1 divergence + stronger PR body fold |
| `22103c2` | malformed runner containers: use shared typed row issues for add/update/remove and prove both malformed shapes leave config bytes unchanged | 1 divergence + stronger PR body fold |
| `ec4fbbc` | input assembly stages: retain the frozen post-validation direct-stage body as a stage-fusion closure while active owners reject invalid stale values before launch/write | 1 re-labeled architecture closure |
| `4801539` | dependency Python-constraint receipts: exact set/clear human rows after successful commit, with JSON/state preservation and no spill into dependency/needs paths | 2 divergences |
| `7fd57de` | editor authoring preflight: true non-TTY refusal, prompt-synchronized missing/blank name handling, exact three-language copy, and honest PTY conversion of legacy active siblings | 3 divergences + stronger PR body folds |
| `659be98` | bare add menu: show the complete `[1/2/3/4] (1)` input prompt through a private Dialoguer theme without replacing validation or dispatch | 1 divergence |
| `929e547` | bash-path persistence: keep the low-level store scalar literal and move file validation to direct CLI and legacy/typed Preferences write doors | 2 divergences |
| `7d94289` | params human view: keep every stored managed declaration visible, overlay only sound live defaults, preserve literal markup data, and leave JSON/run/form/write paths unchanged | 1 divergence |
| `ae955c3` | declared parameter roundtrip: move the exact owner from a wrong CLI surrogate to a real FileStore create/resolve/update/meta/registry transaction | 1 cross-crate owner |
| `046bd88` | directory consent: interactive unknown directories use a default-yes prompt and rejoin the executable reference lane; no/noninteractive paths remain pre-write | 2 divergences + 1 absent owner promoted |
| `8802cb7` | Settings arrows: distinguish multiline interior movement/selection from a true vertical boundary before yielding to shared form navigation | 1 divergence |
| `8f21ed4` | prompt params human view: complete stored/unreadable/env/unmanaged/gone rendering, secret masking, cap-20 tails, pseudo-localization, and byte-no-write | 4 divergences |
| `617f3ee` | Add Source navigation: remove Browse from the field ring, keep it discoverable by stage-scoped Ctrl+O and mouse, and restore bidirectional field keys/footer targets | 1 divergence + stronger PR body fold |
| `d057578` | unset prompt runner: retain the frozen private empty-string row as an unmapped representation closure while public show/params JSON remains null | 1 re-labeled closure |
| `744b3c4` | dependency receipts: one post-commit three-axis reporter with fixed Dependencies/Python/Needs order, JSON purity, no-op stability, and two needs owners rehomed from runtime | 3 divergences + 2 cross-crate owners |
| `1755354` | Add dispatch conflicts: replace Clap conflict interception with one typed preflight priority matrix, exact three-language voices, and a constrained five-line Zsh completion delta | 4 divergences |
| `0c847ea` | PowerShell reader riders: merge static reader fields with declared Flag/Env riders only for reader-only PowerShell, keeping Python declared-first semantics | 1 cross-crate divergence owner |
| `7510454` | placeholder metadata layering: separate implicit `params` names from explicit `parameters` rows across CLI/UI creation, params edit, and Settings touched-row projection | 4 divergences |
| `2d78043` | help taxonomy: exact root plus eight subcommand descriptions in all three locales, backed by 27 real-binary probes and an exact command-tree matrix | 1 divergence |
| `8f0c038` | raw/typed metadata rows: reclassify the frozen mixed seam as architecture-closed and strengthen executable raw projection, typed writer, and store no-rewrite owners | 1 re-labeled closure |
| `d1a6678` | unknown-kind selector: one typed plain-CLI adapter, 8 public PTY owners, 5 production helper owners, exact three-language layout, invalid/cancel/race/TUI separation, and no surrogate reducer ownership | 8 divergences + 5 absent owners promoted |
| `7ce033e` | declared-edit domain API: typed request/context/result/warnings, exact add/remove/order/type semantics, and 38 frozen domain bodies translated from empty stubs | first declared-engine stage |
| `0c8b975` | declared-edit semantics: all tweak axes, partial success, row rollback, bool hygiene, and application helper convergence | second declared-engine stage |
| `bcc1eae` | declared row extensions: merge unknown keys on authorized typed writes, preserve reads byte-for-byte, and keep fresh-meta CAS semantics | third declared-engine stage |
| `aad6f42` | declared CLI adapter: collect malformed inputs, render nine typed warnings in three locales, write once, purge only after CAS, preserve JSON purity, and activate path type | final declared-engine stage; 2 divergences + 40 absent/cross-crate owners promoted |
| `99a4a7c` | draft foundation: typed cross-platform source identity plus shared prompt/shebang-first draft-kind inference | owned-draft stage A |
| `98e75e9` | owned-draft boundary: exact explicit/inferred guard priority, three-language recovery, explicit language/prompt escape, and no-write contracts | owned-draft stage B |
| `7aab925` | post-commit consume: initial claim through commit, open-handle identity checks, atomic quarantine, no-clobber restore, symlink/outside/race protection, and localized non-rollback outcomes | owned-draft stage C |
| `6b5c6f0` | terminal workflow: carry snapshot and delete identities through CLI/TUI effects, refresh Changed rows, remove direct unlink surrogates, and prove the real live-host draft resume | owned-draft stage D |
| `7f163eb` | consume claim cleanup: group identity/modified/permissions/source facts in one private claim and satisfy hard Clippy gates | owned-draft refinement |
| `86f54a4` | cleanup warning localization: compose the localized warning prefix with the localized changed-draft message and satisfy the catalog scanner | final fix-pass cleanup |
| `4832197` / `cd15b26` / `40fa612` | editor process root: activate all four frozen helper owners at public CLI/PTY boundaries; pass the real stored copy path; accept ordinary nonzero editor status in CLI/TUI authoring; preserve the exact launch/read errors; finalize in-place copy edits through a repository-owned claim and identity/meta CAS; return the locked current bytes used for hash, validation, and reporting without replacing user bytes | 4 absent owners promoted + transaction race regressions |

PR #44 is complete upstream at fixed head `005bc9b7365fca1cfa7173acb61a2e8629f03bc9`.
Review only the diff from the previous pin `38260ff881420fbd06f95b5b9243e0caa610e370`;
do not replay its 500+ commits or merge its 198 split test/support paths. The previous ancestry
snapshot remains on `integration/pr44-20260812` at `a6e0513`, but it is not the final PR head.

Remote PR status was re-verified on 2026-08-21 after the handoff push. The stale `865e568`
paragraph that stood here is retired: PR #45 is an open draft whose remote head IS the current
branch head `8306c165e782c67ca422f969a9585a3a0f27f19d`, and GitHub reports it mergeable. The
complete check rollup at that head has 10 green checks — CodeQL (aggregate and all three language
analyses), CodeRabbit, CodSpeed, Docs build (deploy correctly skipped on a PR), the benchmark A/B
pull request comparison, dependency and workflow audit, and PyPI plus `uv tool` compatibility — and
7 red checks with three distinct roots (see the failure inventory below). PR #44 remains an open
draft at the fixed head above with a conflicting merge state; it is historical evidence only.

The 2026-08-21 failure inventory at `8306c16` (runs 32538327971 CI, 32538327972 benchmark,
32538327970 mutation):

- `test_rename_survives_doctor_rebuild` (`port_test_rename.rs:276`) fails the ubuntu test job, the
  format/lint/documentation job, the coverage job, and the mutation baseline. Root: the CI runners
  have no `uv` on PATH; with one python entry, v0.4 `doctor` exits 1 when uv is missing
  (`cli.py` `_uv_required` + `typer.Exit(0 if uv or not _uv_required(entries) else 1)`), and the
  Rust product matches (`cli.rs` `UvHealth::Missing` -> exit 1). The product is correct; the test
  is not hermetic — the same class `b32dc70` fixed with a private uv probe in other doctor owners.
- macOS fails 7 `cli::tests` owned-draft owners on `/private/var` vs `/var` canonical temp paths.
  One panic ("refusing to remove a file outside skit's drafts directory") shows the product's
  draft claim/containment seam mixes canonicalized and literal paths — a real product defect when
  the data directory sits behind a symlink, plus test expectations that assume literal paths.
- Windows fails one benchmark unit
  (`suites::footprint::tests::record_distribution_sizes_count_an_external_binary_exactly_once`,
  3 vs 8 bytes): the fixture writes a POSIX `venv/bin/skit` external binary that the Windows
  discovery path does not see. Fixture portability, not product.
- The benchmark PR-profile job completes every real step (Python 3.13 verification, release build,
  Criterion, PR profile, budgets, upload) and fails only in the setup-uv v10 post step: the cache
  path does not exist because the workflow installs no packages. `benchmark.yml` and
  `benchmark-nightly.yml` still set `enable-cache: true`; `benchmark-compare.yml` already disables
  it. Fix: `enable-cache: false` (keep the v10.0.1 commit pin; do not downgrade).

The three platform test jobs are fail-fast at the target level, so each platform list is a lower
bound: ubuntu stopped at `port_test_rename`, macOS inside the `skit-cli` lib tests, Windows inside
`skit-benchmarks`. Before the next push, run the full workspace suite twice locally: once with uv
hidden from PATH (simulates the runner) and once with `TMPDIR` behind a symlink (simulates macOS
`/private/var`). Do not push merely to refresh checks while fixes are still changing. A push
starts the mutation workflow and invalidates its result on the next source change.

The 2026-08-22 fix wave closed every root above plus four latent failures the fail-fast masking
hid. Seven code commits follow the docs correction `3797ceb`:

- `257f0a5` — the footprint fixture models the running platform's venv script layout
  (`Scripts\skit.exe` on Windows, `bin/skit` elsewhere). Explicit negative finding: no product
  bug — `venv_skit` already reads the real Windows layout.
- `8f712d7` — `benchmark.yml` and `benchmark-nightly.yml` set `enable-cache: false` (the jobs
  install no packages, so the setup-uv v10 post step failed on the absent cache dir);
  `benchmark-compare.yml`, `release.yml`, and `ci.yml` already disable it. The tooling contract
  now fails closed on `enable-cache: true` in the three benchmark workflows (RED: flipping
  `benchmark.yml` back fails the gate). The v10.0.1 commit pin is unchanged.
- `732329c` — the editor launch-failure guard replaces the random scratch prefix before its
  "XX"-sentinel read; the oracle's guarded message (`code --wait`, test_editor.py:218) holds no
  random text, and one full-suite run drew a `.tmpXXxWk9` directory. The exact-message assertion
  is untouched.
- `e665966` — `test_rename_survives_doctor_rebuild` installs a private uv probe
  (`<data>/bin/uv`, `uv.exe` on Windows); RED reproduced the CI failure byte-for-byte with a
  uv-hidden shadow PATH. The same helper in `port_test_prompt_cli.rs` now also writes `uv.exe`
  on Windows — its probe was invisible there (a latent Windows failure no Linux sweep can see).
  Two new `product_contract.rs` owners close the previously unowned `--rebuild` exit cells:
  a clean rebuilt report still exits 1 without uv for a python and for an empty library (human
  and JSON), and a command-only library exits 0 (a forced `code = 0` mutation fails the new
  owner). One new store owner proves scalar `needs`/`parameters` hide only their own entry and
  rewrite no byte, completing the P3 scalar-container verification.
- `195587e` — product fix: the owned-draft seam kept two spellings of the drafts directory and
  compared them asymmetrically, so a data directory behind a symlink (every macOS temp path)
  made skit refuse a legitimate draft cleanup ("refusing to remove a file outside skit's drafts
  directory"). Ownership checks now resolve both sides; rows, claims, quarantine, and
  user-visible text keep the caller's spelling; `source_record` provenance stays resolved
  (store.py:329). RED: all 7 macOS failures reproduced on Linux under a symlinked `TMPDIR`,
  plus an 8th latent one (`owned_draft_restore_and_cleanup_failures_never_clobber_or_rollback`).
  The 7 original tests pass unchanged; one added owner proves the symlinked-data-dir case under
  a normal `TMPDIR`. The consume guard keeps literal comparisons for both spellings so a
  `drafts/../drafts/skit-x` lexical detour is still refused.
- `2af1e3d` — product fix: the CLI `doctor --rebuild` receipt used the invented one-off
  "Registry rebuilt: {}" while the TUI face already used the v0.4 ngettext pair. The CLI now
  prints "Index rebuilt: {} entry" / "Index rebuilt: {} entries"; both catalog rows repeat the
  shipped `.po` verbatim (`索引已重建:{} 条` / `索引已重建:{} 筆`, half-width colon); the
  invented key's row is removed. Singular, plural, and all three locales have owners. Known
  residuals deliberately not reopened: the OK/ERROR/WARN prefixes, the missing install-hint
  tail on the uv line, the rebuild line's position, and the Library line format are adjudicated
  design, recorded here for the release reviewer.
- `f274fea` — nine port-test sandboxes resolve their root at creation, as pytest's `tmp_path`
  does (the oracle fixtures compared resolved with resolved). This closes the 13 next-wave
  macOS failures that the lib-test failures had masked. Shared helper:
  `crates/skit-cli/tests/support/temp_root.rs`. No test name, assertion, or ledger row changed.

The wave was pushed on 2026-08-22 as PR #45 head `7ce111d03709ed86d87e7366822b17cc0c5f80b5`
(with `e3edce7` docs and `7ce111d` mutation-probe owner on top). The remote result validated
every fix: benchmark run 32561430103, benchmark-compare 32561430063, CodSpeed 32561430087,
Docs 32561430081, and CodeQL 32561428241 are green, and every previous test-failure class is
gone from CI run 32561430067. That run then unmasked exactly three NEW latent failures (each
platform is fail-fast, so they had never executed): the completion-detection test got a
PowerShell script on a bash host, macOS spelled the SIGINT name "Interrupt: 2", and the
Windows checkout rewrote `benchmarks/budgets.toml` line endings. The mutation run 32561430085
died on the first of these in its baseline.

The 2026-08-22 second wave closed all three, each adjudicated against the oracle:

- `19ff59e` — product fix: `detect_shell` asked for `PSModulePath` before `SHELL`, so any host
  that exports PowerShell modules (every GitHub Linux runner) got a PowerShell completion
  script for a bash login. v0.4's chain (Typer -> shellingham) walks the parent process tree
  and reads `PSModulePath` on no platform. `SHELL` now answers first; `PSModulePath` is the
  match's last arm (PowerShell sets no `SHELL`, so Windows is unchanged). A new
  `edge_workflows` owner pins bash/zsh winning over an exported `PSModulePath`; a reverse
  probe (re-inserting the old early return) fails it, so the plain suite kills the reordering
  mutant. The pre-existing fallback and error-arm owners survive untouched.
- `b0eabe0` — the Ctrl-C owner accepts both host spellings of the SIGINT name ("Interrupt"
  and "Interrupt: N") while still refusing a normal exit. A workspace sweep found no other
  exact signal-string assertion (the other signal owners assert numeric `128+N` codes).
- `ec3a8fa` — the repository had no `.gitattributes`, and Windows runners check out with
  `core.autocrlf=true`, which breaks every byte-exact contract (four corpus fixtures carry
  CRLF on purpose; `benchmarks/budgets.toml` must match the renderer). A root `* -text` rule
  now keeps the committed bytes of every path on every platform; `git check-attr` over all
  611 tracked paths reports `text: unset`. The tooling contract fails closed if the file or
  the rule goes missing. Windows proof is by-construction plus the next CI run.

A GHA-simulating full-workspace sweep (`SHELL` unset, `PSModulePath`/`CI`/`GITHUB_ACTIONS`
set) and the plain suite both hold at 4062 / 0 / 525 after the second wave; fmt, workspace
Clippy `-D warnings`, tooling contracts, Actionlint, and Zizmor pass.

- `24fe85c` — local instrumented runs flaked twice on the 6-second mirror-PTY child-exit
  deadline (`cli/tests.rs` `finish()`, panic at tests.rs:8015): instrumented children flush
  coverage profiles on exit and need more headroom under parallel load. Both child-exit-wait
  deadlines (the `finish()` loop and the same-purpose sibling in
  `skit-tui/tests/terminal_pty.rs`) are now 30 seconds, per the `08046bd` precedent — a
  harness budget, never a product timeout. Six consecutive instrumented runs pass. The
  per-needle output budgets are unchanged; two other exit-wait constants were surveyed and
  deliberately left (`port_test_add_no_source.rs:244` asserts on its own expiry flag, so a
  bump would change test meaning — flagged as a latent instrumented-flake candidate).

A fresh committed-state workspace `cargo llvm-cov --locked --workspace --all-targets
--all-features` run after the second wave (with `24fe85c`) passed 4062 / 0 / 525 and
`scripts/check_coverage.sh` returned `complete executable-source line coverage`; no checker
rule or exclusion changed.

The second wave was pushed as PR #45 head `43c187568f33341d65d93512ef476efe2dbaa266` (runs:
CI 32563580462, benchmark 32563580469, compare 32563580485, CodSpeed 32563580457, Docs
32563580456, CodeQL 32563578519, mutation 32563580465). Benchmark, compare, CodSpeed, Docs,
and CodeQL stayed green, and the ubuntu test job went green for the first time on this
branch. The residual reds decomposed into six small clusters, all fixed in the third wave
(2026-08-22, `0f50deb`..`994387f`):

- `0f50deb` — the lint job died on `rg: command not found`: `test_tooling_contracts.sh` used
  ripgrep four times and GHA runners do not ship it (latent since `9114203`). All four are now
  portable grep; two counts tightened to `-F` literals. Green with and without rg on PATH.
- `d81fa45` — the macOS `test_deps_need_replaces_whole_list` failure was NOT a product bug:
  the raw meta held the correct replaced list, and the bare `contains("old")` guard matched
  the recorded source path — macOS `$TMPDIR` sits under `/var/folders`, and "folders" holds
  the letters of "old". Reproduced on Linux with a `/tmp/var/folders/zz/T`-shaped TMPDIR. The
  needle is now the TOML string `"old"`. A class sweep over short negative `contains` needles
  against the deterministic macOS path words (var/folders/T/private) found one member.
- `351dad5` — the coverage job flagged the unix HOME-fallback config lines: they were covered
  only when the ambient host env cooperated. The join now lives in a pure
  `unix_config_dir(xdg, home)` with a hermetic three-branch owner (XDG, HOME fallback, None);
  no test mutates process env. Same-fragility siblings flagged for later: the cli.rs twin at
  `:10626` and both `platform_state_dir` shapes (one ambient hit today).
- `ad289e2` — `javascript_gate.rs` `name()`'s Deno arm was covered only on hosts with deno
  installed; the runtime-name unit now asserts all four arms hermetically.
- `7b18108` / `994387f` — the two Windows `environment_report_contract` fixtures hardcoded
  `/usr/bin/env` and a POSIX path fragment. Both are platform-aware now; the Windows slow
  command names `%SystemRoot%\System32\PING.EXE` directly (no PATH lookup, no shell), unix
  keeps exact-equality assertions. Cross-checked by a temporary cfg-swap
  `cargo check -p skit-benchmarks --tests` (restored byte-exact); Windows runtime proof is
  the next CI run.
- `aa1da10` — the mutation baseline at `43c1875` died on an ETXTBSY fork-window race: a gate
  fixture writes its fake runner and the product execs it while a sibling test's
  fork-to-exec window still holds the write fd. The two identically shaped gate tests now
  retry only `Spawn` results containing "Text file busy" (9 tries, 20 ms) with the `Timeout`
  assertion unchanged. A suite-wide survey of ~32 write-then-exec fixtures retro-fitted none:
  everywhere else the exec sits inside a stateful CLI invocation where a retry would rerun a
  partial commit. Watch-flags: the multi-megabyte `fs::copy` uv probes (probe-only, never
  exec'd) and `run_identity_races.rs:35` (deliberately blocks mid-run).

After the third wave the full workspace suite is 4063 / 0 / 525 (one new test name, the
`unix_config_dir` owner). fmt, workspace Clippy `-D warnings`, tooling contracts (with and
without rg), Actionlint, Zizmor, and the English gate pass. A fresh committed-state workspace
LCOV run at the third-wave tree passed 4063 / 0 / 525 and `scripts/check_coverage.sh`
returned `complete executable-source line coverage`.

The third wave was pushed as head `e329419ec8a0327e017693ffa5d364ee99cb7292` (runs: CI
32565964925, mutation 32565964953). The coverage job and the lint job went green for the
first time; ubuntu stayed green; only macOS and Windows remained, still progressively
unmasking. The fourth wave (2026-08-22, `9abfe71`..`e3a6d7f`) closed them:

- `9abfe71` — three Windows lib-test failures: Debug-formatted output escapes backslashes,
  so `contains(raw_path)` can never match (compare against the `{:?}` spelling); a
  slash-joined skills path never matches the store's backslash join (join per component);
  and the "invalid arguments" fixture used an unpaired single quote, which Windows argument
  rules read as an ordinary character — an unpaired double quote is invalid on every host.
- `9a79b0c` — the macOS PTY stall: the live terminal sends cursor-position queries and waits
  for the reply, but the harness answered them only while delivering effect keys, so a query
  that arrived between keystrokes stalled the child forever on a timing-dependent host. The
  harness now answers every cursor question whenever it reads output, file-wide, and the
  timeout panic reports the child's status so the next failure names its class outright.
- `910a736` — the class sweep (367 files, three needle classes) found one member the first
  grep missed: a `format!`-wrapped mixed-separator needle in `v040_compatibility.rs`. Final
  counts: Debug-escape class 1, exec-bit class 0, separator class 2, all fixed. Watch-flag:
  `Message::quoted()` Debug-escapes its value; all 20 call sites pass names today.
- `e3a6d7f` — the wave-3 ETXTBSY class resurfaced in a skit-benchmarks pipeline test whose
  shims log every invocation, so the plain warm-up retry would break the counting owner.
  All three shim makers now inject a `__skit_probe__` guard directly under the shebang
  (above every side effect) and warm each new shim with a bounded busy-retry. The counting
  owner still reads exactly 3; five consecutive under-load suite runs stay green. All shim
  modules are unix-gated, and no test pins shim script bytes.

- `1ee58ac` — the LCOV gate flagged the new ETXTBSY retry arm as uncovered (it runs only when
  the race fires). A deterministic owner now forces the busy answer: the test holds a write
  handle on a probe-guarded shim (exec of a file the process holds open for write is ETXTBSY
  by definition), releases it after ~4 retry beats, and asserts the wait returned with at
  least one beat elapsed. The arm went from 0 to 4 hits; `suites/mod.rs` has no uncovered
  line.

The fourth wave was pushed as head `dff74a1c76df18307c042e393279602e5c30c5c6`. The macOS
test job went green for the first time; only Windows remained, down to one lib-test failure —
and that one was a REAL Windows data-loss defect, fixed in the fifth wave:

- `30d0326` — deleting a kept draft checks identity, modified time, and permissions. The
  Windows identity is volume number, file number, and creation time: an in-place write moves
  none of them, the modified time can repeat inside one clock tick, and permissions carry one
  bit — so an edited draft was judged unchanged and REMOVED. Windows draft rows now carry a
  content witness (a `content_hash` read at listing), the delete check verifies it wherever
  it reads the other fields, and a mismatched or unreadable witness answers "changed", so the
  draft is kept and the row refreshed. Unix records no witness and reads no extra bytes: its
  identity carries the change time, so behavior and work are byte-identical. The snapshot
  lane already compares exact bytes. Three mutant probes go RED against the plain suite.

The fifth wave was pushed as head `22471d667b05bf9f5d0ad3723ece7544843a242a`. Windows lib
tests PASSED (the content-witness fix is validated on real Windows); Windows advanced into
the integration targets and fell at the `~` home-expansion test in `add_lanes.rs`. The sixth
wave, stage 1 (2026-08-22, `fbaac32`..`50c4645`):

- `d725b05` -- shared shim infrastructure: `tests/support/shim.rs` names shim BEHAVIORS
  (Exit / MakeDirectory / TouchFromEnvironment) and emits `#!/bin/sh` on unix and `.cmd` on
  Windows, returning the written path so no caller re-spells the name. Verified mechanics:
  `program_names` (launch.rs:1364) appends PATHEXT entries to bare names, so `foo.cmd` is
  found for bare `foo`; std::process delegates `.cmd` to cmd.exe with strict escaping and
  none of the converted shims carries a character it refuses. The real conversion surface
  was 4 shims (healthcheck fake uv; edge_workflows node/npm/custom-js) -- the wave-5
  file-level forecast overcounted: surface_edges and v040_run_parity were already
  per-helper unix-gated, and the edge_workflows editor shim's content is never executed.
  One shim cannot be `.cmd`: the surface_edges private-uv stub pinned to the hardcoded
  `bin/uv.exe` path (stage-2 item: a real argv-echoing exe, or it stays unix-gated).
- `50c4645` -- product fix, found by overturning the supervisor's own premise: the fixture
  already seeded USERPROFILE and Windows still failed, because `dirs::home_dir()` reads NO
  environment variable on Windows (SHGetKnownFolderPath), while v0.4's `ntpath.expanduser`
  reads USERPROFILE then HOMEDRIVE+HOMEPATH. `expand_leading_tilde` now reads what v0.4
  reads on Windows, keeping the shell answer only as the last resort (a strict-superset
  corner: v0.4 leaves the path literal there). Unix is byte-identical. A class sweep found
  every other seeded-home fixture already correct.
- `fbaac32` -- the temp_root doc comment no longer claims equal spellings on every platform.

Receipts: unix byte-identical (stage-1 files plus add_lanes, agent_install, skit-store green
under plain and symlinked TMPDIR); cfg-swap `cargo check` for both cfg(windows) bodies; full
workspace 4065 / 0 / 525; fresh workspace LCOV `complete executable-source line coverage`.
Windows runtime proof is the next CI run.

Stage 1 was pushed as head `a2365e5`. Its run: ubuntu/coverage/lint green; macOS red on the
SAME preset test as wave 4 (the new diagnostics finally named the class: child alive, ZERO
cursor questions, the only post-checkpoint bytes were the ECHO of the key the harness had
just sent); Windows HUNG 2+ hours and the run was cancelled without a flushed log — ci.yml
had no job time bounds. Wave 6 stage 2 (2026-08-22, `9265344` / `2ec0011`):

- `9265344` — every job in every workflow now carries `timeout-minutes` (15 jobs were
  unbounded across ci/codspeed/docs/release; mutation and the benchmark workflows already
  had bounds). A hang now fails its job with a flushed log naming the site. A fail-closed
  tooling contract keeps the bound from disappearing (removing one produces
  "workflow job has no timeout-minutes", exit 1).
- `2ec0011` — the macOS preset stall mechanism: the harness wrote a key the instant prompt
  TEXT appeared, but the OPTIONAL_SECRET prompt changes terminal mode before reading and a
  TCSAFLUSH-style change discards pending input; the echoed "
" was the swallowed key.
  Every plain-flow write now settles first (30 ms of child silence, bounded 5 s); the
  timeout diagnostics also report the last keys written. Linux cannot reproduce (15 loaded
  runs on pre-fix code: 0 failures); the next macOS run is the discriminator.

Deferred by evidence, awaiting the first BOUNDED Windows log: the S2c audit reduced the shim
wall to ONE real file (`port_test_declared_params` — 53 argv-echo parsers plus PATHEXT kind
inference; `port_test_config_cmd` and `port_test_js_deps` shims are already unix-gated, the
wave-5 file-level counts were inflated), and the S2d survey found `terminal_pty.rs:998-1016`
carries DELIBERATE Windows runner support (`.cmd` + `cmd.exe /C`), so blind-gating the
real-PTY targets would discard real work; the oracle itself has no real-PTY tests (Textual
in-process pilot only). Both proceed once the bounded run names actual failures.

Stage 2 was pushed as head `36c3fb6`. Its bounded run: macOS GREEN (the settle mechanism
confirmed on the real host), the four converted .cmd shims and the tilde fix validated on
Windows, and the Windows hang finally NAMED by the flushed log: `edge_workflows` passed 12
of 14 tests and hung on its two EDITOR tests. The seventh wave (2026-08-22,
`b01cabd`..`62eb418`) closed the editor-hang class and the one coverage line:

- The mechanism is the vi-hang trap, Windows edition: the v0.4 editor fallback is `notepad`,
  and CreateProcess resolves bare `notepad` from System32 REGARDLESS of PATH, so the unix
  protections (pin EDITOR, empty the PATH) are void there — the real GUI editor launches and
  never exits. `8f7847a` gates the two edge_workflows fallback tests; the exhaustive sweep
  (reachability-mapped: `skit edit` has no TTY gate and is the dangerous door; `add --edit`
  and the plain menu cannot reach a launch from a non-PTY test) found exactly one more
  ungated hang risk (`surface_edges.rs:507`, gated) and one launching-but-passing lib test
  left alone because two real Windows runs already falsified the hang prediction. `d28737c`
  gates the terminal_pty shell-editor authoring test whose inner `#[cfg(unix)]` guarded only
  a permissions block, not the test.
- `b01cabd`/`62eb418` — the blocking-read arm of the skit-tui terminal loop is owned
  deterministically: the poll/read calls arrive as parameters, and shared counting stubs
  prove non-invocation by a counter standing still (never-called panic closures were
  themselves uncovered lines; the LCOV gate caught that first draft).

After the seventh wave: full workspace 4066 / 0 / 525 (one new gate-compiled owner), fresh
workspace LCOV `complete executable-source line coverage`, fmt/Clippy/tooling green.

MUTATION STRUCTURAL FINDING: the first surviving-baseline mutation run (at `e329419`) found
9,667 mutants and was cancelled by its own 360-minute bound — the GitHub hosted-runner 6-hour
ceiling cannot fit a single-job run, so the workflow must shard (cargo-mutants `--shard k/n`
matrix with zero-missed aggregation). Its partial log already names EIGHT survivors, all in
skit-application: library_detail.rs:152 (`&&`->`||` in entry_detail), :223 and :227
(`<`->`<=` in LibraryRunAge::from_elapsed), path_completion.rs:221 (`&&`->`||` in
trailing_piece), path_insertion.rs:95 (current_argument_dialect -> Default), 
payload_policy.rs:190 (delete match arm "exe" in add_workdir), preferences.rs:28 and :29
(`||`->`&&` in MirrorConfiguration::has_urls). Each needs a killing owner; expect more
survivors from the unscored remainder — plan a full local mutants run once the tree freezes.

The seventh wave was pushed as head `fb91d5b`. Its run: SIX jobs green (macOS, ubuntu,
coverage, lint, both audits); Windows hit its bound with a flushed log naming four hangs in
`port_test_add_no_source.rs` (three directory-consent Confirms and the plain-menu path
Input). Waves 8 and 9 (2026-08-22, `fddcf44`..`4c512d9`):

- `fddcf44` / `1060094` -- the mutation gate is restructured: 9,700 measured mutants cannot
  fit one hosted job (both rate estimates give 8-14 h at n=16), so mutation.yml is a 48-way
  cargo-mutants `--shard k/48` matrix (flag semantics verified against the pinned 27.1.0:
  indices 0..47), each shard bounded at 300 minutes, with a fail-closed aggregate: a missing
  or empty shard record is a FAILURE, and missed/timeout lists must be empty. On pull
  requests the whole set is opt-in via the `mutation-requested` label (unlabeled pushes skip
  instantly) because 48 shards per push would starve the account's concurrency and a push
  invalidates the previous result anyway; push-to-main, the nightly cron, and dispatch are
  unchanged. The tooling contract cross-checks the shard count in three places, pins the
  label gate on both jobs, and pins `.cargo/mutants.toml` to EXACTLY one excluded function.
- `b27e119` -- seven of the eight partial-run survivors are killed with per-mutant hand-probe
  RED receipts; the age-bucket boundaries are asserted at the oracle's strict `<` thresholds
  (tui.py:112-118: 90 s IS Minutes(1), 129,600 s IS Days(1)). The eighth is a false survivor
  inside `#[cfg(windows)]` (the mutant edits code Linux never compiles), excluded in
  `.cargo/mutants.toml` with the reason recorded; `ArgumentDialect` deliberately did NOT gain
  a Default derive for a test's convenience.
- `8016c43` / `4c512d9` -- the four Windows hangs were an input-dialect defect in the PTY
  harnesses, not ConPTY breakage (12 of 14 edge_workflows PTY tests already passed on real
  Windows): the `console` crate reads Enter as CARRIAGE RETURN ONLY on Windows
  (console/src/windows_term/mod.rs:449) while unix accepts either (unix_term.rs:323), so a
  harness that typed b"n\n" appended a printable character and both sides waited. Every
  harness that types into a live terminal now translates the line feed at the WRITER (one
  cited convention line per harness, 12 sites across 8 harnesses; source literals keep \n so
  no assertion needle drifts). Untranslated writes are cursor replies, interrupt bytes, and
  pipe/file writes, each with the reason recorded.

Receipts: full workspace 4070 / 0 / 525 (arithmetic: 4066 + 4 survivor owners), fresh
workspace LCOV `complete executable-source line coverage`, fmt / Clippy `-D warnings` /
actionlint / zizmor / tooling contracts (six fail-closed probes) green, plain and
symlinked-TMPDIR suites green for every touched harness. Windows runtime proof for the
Enter dialect is the next bounded run.

The 444bbd9 run refuted sufficiency of the Enter translation: the SAME four
port_test_add_no_source tests hung with the translation verified present. Wave 10
(`3124103`) found the real discriminator structurally: the hanging harness NEVER READS
output (blind clock-paced writes; the drain thread joins only at the end), while every
dialoguer harness that passes on Windows waits for the prompt text before typing. The
console crate's Windows intake (`ReadConsoleInputW` one record at a time, discarding
non-key and key-up records, console/src/windows_term/mod.rs:531-560) does not give an early
answer the tty line-buffer guarantee, so a key typed into the gap is not there when the
prompt reads — the macOS TCSAFLUSH shape on another platform. The flush theory was checked
and is NEGATIVE (no FlushConsoleInputBuffer anywhere in console; dialoguer's init flushes
are output-only). All three clock-typing harnesses (pty_in_locale, pty_after_output,
terminal_pty::run_pty_configured, plus the unix-gated port_test_editor::run_pty for
consistency) now settle before every write; unix suites are green and FASTER (fixed pauses
became silence-detection). Agreed fallback: if the next bounded run still hangs these four,
they get gated with the recorded reason (Rust-additive interactive PTY choreography, no
oracle counterpart, outcomes owned by non-PTY owners, Windows interactivity falls to the
hands-on gate) — the fix commit touches only harness internals so the gate commit stays
clean. Also: stale queued single-job mutation runs at old heads were cancelled; GitHub's
concurrency keeps only the newest pending run, and the label-gated workflow skips instantly
on unlabeled pushes.

Round 2 (head `4c2609c`) hung the SAME four tests with the settle conversion present, so
both mechanism theories are host-refuted and the agreed fallback executed (`fa8464b`): all
30 live PTY-driven tests in port_test_add_no_source are `#[cfg(unix)]` (33 with the three
already gated; 32 ignored untouched), the three no-input/plain lanes stay live as the
file's Windows coverage, a scoped `cfg_attr(not(unix), allow(dead_code))` keeps the
Windows compile warning-free, and the commit message declares the escalation rule: the next
file whose terminal choreography hangs gets the class gated suite-wide in one sweep. A
NEW observation for any future cure attempt: every harness that PASSES dialoguer prompts on
Windows waits for the PROMPT TEXT before typing (plain_add_pty, agent_install), while both
failed cures waited on a clock or on SILENCE — silence can arrive before the prompt is
drawn, so the key still lands in the pre-prompt gap. Escalation candidates mapped:
terminal_pty.rs carries 27 ungated PTY tests whose 30 run_plain_in_pty sites drive the same
dialoguer prompts (highest risk; its LiveTui/crossterm half is a different read path);
port_test_agent_install waits for prompt text and may survive; editor/prompt_cli files are
already file-gated. Windows has still never executed anything after port_test_add_no_source
alphabetically.

The gate round (head `b77cb57`) advanced Windows to 612 passing tests and ONE hang in
port_test_agent_install: `test_cli_bare_interactive_no_candidates_exits_1`. Wave 12
(`330ff09`) fixed rather than gated it: the harness held `pair.master` alive for the whole
call, and ConPTY never delivers the reader's EOF while the pseudo-console is open — the six
harnesses that pass on Windows all drop the master before joining their drain. Both earlier
theories were excluded from code (zero candidates never build a Select; the test types
nothing). The same missing release was fixed pre-emptively in terminal_pty.rs's three
teardowns — the next binary in Windows execution order.

Round three for the lone agent_install straggler survived the master-drop fix, so wave 13
(`d8737f1` / `1eda1ab`) closed the ConPTY chapter structurally: the straggler is gated with
the recorded reason and its zero-candidates contract retained by extending the existing
skit-application owner (exit class + three-locale voice); all four Windows-reachable PTY
teardowns are now child-exit-keyed with bounded drains (never blocking on a ConPTY EOF that
may not arrive), audited across all 14 native_pty_system files; and the declared escalation
was applied pre-emptively to terminal_pty.rs's dialoguer plain lane — 12 tests gated on the
READ-PATH boundary (two reached through inlined run_pty_configured(.., false, ..) — the
wrapper name was not the class), while the 14 LiveTui/crossterm tests stay LIVE on Windows
for empirical adjudication and 9 are neither. Aggregate unchanged at 4070 / 0 / 525.

The fc93b5c run marked the regime change: Windows now FAILS FAST with full diagnostics
instead of hanging (10s failure, no bound hit). Its three agent_install failures showed the
child emitting a CSI 6n cursor query and waiting: unix console answers position through an
ioctl and never sends the escape, so only Windows needs the reply. Wave 14 (`30d9ec7`)
taught run_agent_install_pty the repo's canonical data-driven answerer (count queries in
the stream, answer each once, inside the needle-wait loop), with a LOCAL RED/GREEN: a
scripted child emitting CSI 6n reproduced the CI failure byte-identically and passed in
1.02s after the fix. An 11-harness exposure scan found no other exposed harness; the one
residual (skit-tui's run_child_in_pty answers only in its first loop; its later loops wait
on file metadata and child exit, were green on Windows, and a blind extension could add a
unix write-after-exit flake) is flagged in the code review record, not patched.

The 30d9ec7 run validated the cursor-answer fix (2 of 3 tests green on real Windows). The
last, the EOF choreography, is a platform absence, gated in wave 15 (`ef86cba`): the phase
sends the VEOF character (\x04), which the unix line discipline converts to end-of-input
at the child's read — ConPTY runs no line discipline, the byte arrives as an ordinary
Ctrl-D key event, and Windows' own console EOF spelling (Ctrl-Z + Enter, cooked by the real
console host) is equally undeliverable through a pseudo-console. The gate records honestly
that the test's reprompt, bare-Enter default hint, and both CJK locale passes are unique to
it and fall to the hands-on gate on Windows (the two passing siblings cover only a valid
English numbered choice); the abort SENTENCE stays owned on every host by the catalog. A
workspace scan of every \x04 site found one live-on-Windows candidate that is empirically
green (left ungated, over-gating refused) and zero \x1a dependence. Deferred commission
once Windows is fully green: split the phases so default+reprompt return to Windows.

The ef86cba run cleared agent_install entirely and advanced into port_test_config_cmd
(60/1, 2.12s). Wave 16 (`85c21fc`) closed the JSON-escaping class: the failure's root was a
hand-rolled parse_flat_json that never unescapes (its "no serde_json dev-dependency"
justification was obsolete), replaced with serde_json; the sweep found ONE more member (the
doctor private-uv owner searched raw JSON for a path — now an exact parsed-field equality,
strictly stronger) and classified every non-member (token-valued reads, URL values,
plain-text stdout, the two {:?}-escaped TOML writers, three non-JSON hand-rolled readers).
RED was reproduced ON LINUX by naming a probe file `ba\sh` — a legal unix filename whose
serde escaping shows the exact doubled-separator signature.

The 85c21fc run cleared config_cmd and reached the S2c stronghold, port_test_declared_params
(49/3/1, whole binary ran). Wave 17 (`260b1c4` / `6adce1a`) converted all three rather than
gating: the two launched sh fixtures became host-dialect shims (new EchoArguments /
EchoEnvironment behaviors; one explicit CRLF normalization at the single line-boundary
assertion), and the secret-env command text is cfg-selected (`echo $TOKEN` / `echo %TOKEN%`)
so the contract is asserted identically on both hosts. A 23-binary forward sweep found ZERO
remaining members of the launched-fixture or POSIX-command shapes — and two members of a
THIRD shape (runner rows naming POSIX-only programs: `printf` in v040_compatibility:1504,
`python3` in port_test_prompt_kind:247), both pre-fixed. Trap recorded: several files carry
`#[cfg(unix)]` on a chmod-only STATEMENT — those tests DO run on Windows and are safe only
because nothing launches. Honest limit: the Windows shim halves (the `set /p` no-newline
idiom in particular) are by construction until the next run. Supervisor directive now in
force for later waves: fix subagents run on Fable (forks), and every brief demands an
explicit root-cause-versus-manifestation analysis; the PTY-harness consolidation (the
structural root of the Windows saga) is scheduled for the post-green review rounds.

The wave-17 run validated every converted fixture on real Windows (889 tests passed;
declared_params, edit, editor all cleared) and hung on ONE test: the bare `skit` entry lane
(port_test_entrypoint). Wave 18 (`27f33cc`, the first Fable-fork wave under the root-cause
directive) found a REAL cross-platform product bug: the TUI terminal claim had NO explicit
precondition — unix raw-mode failing on pipes was an accidental guard, and Windows crossterm
attaches to the process console even with redirected stdio, so `skit > file` hangs forever
for a real Windows user. The fix is the missing precondition at the single seam:
`terminal_claim_refusal(stdin_is_tty, stdout_is_tty)` + `claim_terminal()` now guard both
session entries (net -7 duplicated lines), matching the CLI's own both-streams policy at
every prompt door. The refusal renders through the localized "terminal I/O failed: {}"
wrapper per the repo's io-cause convention. RED tightened the entrypoint owner to the exact
refusal sentence (a fallback spelling means the guard is gone — kills guard-deletion
mutants); a four-combination parameterized owner covers the boolean mutants. Aggregate
4071 / 0 / 525 (+1 owner, arithmetic exact); fresh workspace LCOV returned
`complete executable-source line coverage`.

The wave-18 run validated the terminal-claim guard on real Windows and advanced deep into
the p-range with no hangs, failing fast on one js-inject batch-c test. Wave 19 (`0c80bde`,
Fable fork) root-caused it as a CLASS: the only cmd dialect this repository has ever
validated on a real Windows host is the flat one-statement-per-line form; the failing twins
were authored in the parenthesized compound dialect — including one certain defect
(`exit /b %errorlevel%` inside parens expands at parse time, returning the pre-node status).
The product gate chain was verified sound end to end (candidate-name kind detection, nonzero
refusal, no cfg divergence, std's bat encoding read at source). Both batch-c twins are
rewritten flat (goto/labels, line-level exits); the family audit table confirms every other
twin was already flat. Asserts reordered into diagnosis order, nothing weakened. Aggregate
4071 / 0 / 525.

Wave 20 (`6ee08ab` / `0b7deb3`): the instrumented coverage flake was the worker-answer
budget class (10 s busy-yield poll under an oversubscribed runner; now one 30 s
WORKER_ANSWER_BUDGET convention over both worker waits, 1 ms sleeps so the waiter stops
competing, checkpoint-driven success paths unchanged); the Windows langs failure was a
POSIX-literal fixture hitting the host's real `Path::is_absolute` (launch.rs:1246) — a
`virtual_workdir` helper now builds the fiction host-absolutely, pre-fixing the test's
second half too. Forecast recorded: skit-runtime's test targets have NEVER run on Windows
and hold ~45 POSIX-literal sites with confirmed members (`port_test_launcher.rs:556`
WorkdirMissing-vs-InvalidWorkdir, launch.rs private_tests `/custom`//`/missing`/`/bin/sh`,
no cfg gates) — the pre-fix wave for that crate is dispatched alongside this push.

Waves 21 and 22 (`6270c35` / `a949dc1`) closed the remaining known Windows map in one
push. Wave 21 inventoried the 162 never-ran-on-Windows targets and treated skit-runtime
completely: host-absolute virtual workdirs where the contract is neutral, `#[cfg(unix)]`
with the named compile-time branch where the contract IS the POSIX arm (the `/bin/sh`
lowering, POSIX render, real signal conventions; template-quoting file-gated), inert sites
verified as honest negatives; the cross-crate sweep found zero further members of any
established class. Flag on record: `render_windows_command_template` has no active owner
anywhere — a native-Windows additive owner is owed. Wave 22 fixed a REAL product
divergence at its policy seam: the Windows command-entry shell was resolved by the same
PATH probe as user programs, but v0.4 (CPython `shell=True`, launcher.py:293-298) reads
COMSPEC and lets CreateProcess find bare cmd.exe in the system directory before PATH. The
new `windows_command_shell(comspec, system_root, probe)` (COMSPEC verbatim -> System32 via
probe -> PATH -> typed refusal) compiles on every host with a four-arm parameterized owner.
Flagged asymmetry for a later ledger judgment: the unix arm PATH-probes `sh` while CPython
hardcodes `/bin/sh` — pre-existing and test-pinned, deliberately not changed. Aggregate
4072 / 0 / 525 (+1 owner); fresh workspace LCOV `complete executable-source line coverage`.

The 2792fa6 run reached the LiveTui lane for the first time; wave 23 (`8948c0a`)
adjudicated the 6-vs-15 split with a definitive evidence chain: portable-pty 0.9.0 attaches
the child via PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE (procthreadattr.rs:49-52) with no pipe
override — the exact mechanism Windows Terminal uses — so children hold REAL console
handles, `is_terminal` answers yes, and the wave-18 claim guard does NOT refuse under
ConPTY (real Windows Terminal users are unaffected; the run itself proves full Ratatui
sessions start, interact, and submit on Windows via the six passing run_in_pty owners).
The 15 failures are the harness's unix-only sync protocol: LiveTui::spawn gates every
session on a `\x1b[6n` handshake that Windows crossterm never sends (it reads
GetConsoleScreenBufferInfo; unix.rs:35 vs windows.rs:38-45). The 15 are gated with
per-class reasons; the needle-sync conversion is folded into the planned PTY consolidation;
wave-15's mistaken "green on Windows" note for the VEOF test is corrected. Aggregate
4072 / 0 / 525.

The 8675393 run FULLY CLEARED skit-cli on Windows and moved into skit-runtime's lib
(51/2/1). Wave 24 (`12df0f9`): the uv bootstrap failure was the uv-vs-uv.exe class inside
the SHARED test fixture (`test_asset`/`tar_archive` hardcoded unix spellings while
production installs `uv.exe`; the durability fixture's local host-aware patch was the
smoking gun of the shared defect and is removed with the root fix); the system-probe
failure spawned a literal `/bin/sh` where the contract is real OS status passthrough — it
now spawns the host interpreter per the wave-22 resolution semantics. The in-src sweep of
every remaining unproven crate's `#[cfg(test)]` modules found ZERO further members, with
reasoned clearances (component-wise Path equality for the `/`-literal suspects, cfg-gated
PermissionsExt, parser-input shebangs). Aggregate 4072 / 0 / 525.

Wave 25 (`18266e3`): the two coverage lines and the last known Windows member. The
multi-line runtime-`cfg!` shell tuple in launch.rs private_tests became compile-time arms
(the checker counts every compiled src line; single-line runtime forms stay, per-site
judgment recorded); session.rs:303 had been covered only by scheduling luck that wave-20's
calmer waits removed — it now has a deterministic drop-the-receiver owner (0 -> 1 hit);
the js-deps real-spawn site wave-21's blanket "inert" clearance missed now uses the
host-interpreter pattern with Windows-validated flat cmd idioms and host-correct CRLF
expectations. Every other real-spawn clearance in skit-runtime tests/ was re-verified
individually. Aggregate 4073 / 0 / 525 (+1 worker owner); fresh workspace LCOV
`complete executable-source line coverage`.

Wave 26 (`4bf3114`): the last Windows failure was a NEW class — policy divergence in an
owner's EXPECTED ERROR VARIANT (no path ever touches the host, so no earlier needle matched
it): the shell lane's missing-runtime refusal is `ProgramNotFound` on unix but the
adjudicated `WindowsShellMissing` under the Windows bash policy (launch.rs:1216-1231). The
owner now cfg-matches the host's variant with both policies named; the test's other
assertions were rechecked host-neutral and the rest of launch_plan.rs is empirically
Windows-proven. Aggregate 4073 / 0 / 525.

Wave 26b (`00a1e8a`) swept the policy-divergence class suite-wide: four members total
(launch_plan, two interpreters asserts including configured zsh, one module doc), all
cfg-matched; every cfg-divergent policy point (COMSPEC lane, prompt argv limits, template
renderer, uv spelling, PATHEXT/dialect/editor/tilde) is ticked against its owners with
member/non-member evidence. Three vacuous unix-spelling negative asserts in Windows-proven
files are flagged for the cleanup rounds, not churned mid-convergence. Aggregate
4073 / 0 / 525.

Wave 27 (`53774b2`): the uvman manifest collector joined repo-relative paths with the
host separator against forward-slash literals — one collection-point normalization fixes
every comparison in the file; the sweep of the whole manifest/census family found no second
member (component-wise PathBuf equality, name-keyed censuses, literal-sourced paths, or
Windows-proven targets throughout). Aggregate 4073 / 0 / 525.

Wave 26c (`9f0ee8a`): the fifth policy-divergence member hid in FIELD guards, not variant
names — `limit == 100_000` is the POSIX prompt-argv constant (Windows budgets 60,000
UTF-16 bytes; `unit` is "bytes" on both arms). Fixed with cfg-matched limits and an honest
rename (the fixture fires the refusal on both hosts by construction, so gating would
discard real Windows coverage; the file is not a port module, so the rename is
ledger-safe). The field-level re-audit of every cleared file found no sixth member.
Aggregate 4073 / 0 / 525.

Wave 28 (`d4914fa`): the skit-store lib batch — six failures, all platform-semantic
test expectations aligned to documented product policy: the atomic-write op sequence
cfg-matches the Windows directory-sync omission; the mandatory-lock owner observes the held
phase through metadata (Windows locks are mandatory); a blind-authored readonly-replace
expectation was OVERTURNED with CPython parity evidence (os.replace raises PermissionError
on a readonly target, so v0.4 refuses too — the owner now asserts the refusal and the
preserved original); the pre-epoch timestamp probe respects FILETIME's 100 ns tick. One
latent tests/-target member (keep-mode on a readonly target) was pre-fixed with the
oracle's own comment as evidence (test_atomic.py:445-460). Supervisor decision recorded:
the readonly temp file a failed Windows replace leaves behind is CPython-parity litter and
stays (a product hardening would add an unix-unobservable mutation arm); revisit only with
a native-Windows mutation story. Aggregate 4073 / 0 / 525.

Wave 28b (`27f98bf`): both residuals of 28a's own fixes. The "single-line runtime cfg!"
coverage rule is defeated by rustfmt splitting multi-line expressions — the robust form is
`#[cfg]` CONSTS feeding one assert (const items emit no coverage lines and cannot be split
into counted code); the rule is amended accordingly. The timestamp test advanced to its
next sub-tick probe (7 ns truncates to 0 FILETIME ticks) — the whole test is now audited
probe-by-probe (700 ns = 7 exact ticks keeps the nanosecond-unit proof) with the tick rule
stated once. Product verdict, cited: sub-tick fidelity is NOT load-bearing — Windows
freshness verification carries the authoritative content hash exactly because stable Rust
exposes no change counter (registry.rs:318-323, :371-373, :458-480), so a same-tick
in-place edit is still rejected by hash. Aggregate 4073 / 0 / 525; fresh workspace LCOV
`complete executable-source line coverage`.

Wave 29 (`a645a75`): the frontier reached skit-tui's integration tests; the picker
fixture created `beta file*.txt` and `*` is a Windows-forbidden filename character, so the
file itself failed to create. Host-neutral fix: `beta file[1].txt` — brackets are legal on
every host and are exactly the metacharacter the picked-path glob contract (`0fd6d85`)
centers on. The invalid-filename sweep over skit-tui and skit-ui tests found no second
filesystem member (the skit-ui star strings are reducer data proving the Replace-literal
contract, never touching the filesystem). Aggregate 4073 / 0 / 525.

Wave 26d (`e18b5cd`): the policy-divergence class surfaced in skit-ui's in-src dialect
asserts (single-quote stripping and the unpaired-single-quote rule are POSIX-only
outcomes). Both fixed with host-neutral respellings — an unpaired DOUBLE quote is
UnbalancedQuotes in both dialects and double quotes strip in both, with the Windows arm
empirically proven on this host by the parameterized runner_management owners — so no cfg
was needed and both rules stay owned everywhere. The extended in-src dialect sweep over
skit-ui/tui/form/application found no further member. Aggregate 4073 / 0 / 525.

Cleanup P1 stage 1 (`34cb4f3` / `ff417b8`): the shared PTY module
`tests/support/pty.rs` now carries the five platform invariants with their wave evidence
(CR-is-Enter; prompt-visible-before-write; counted cursor answers; child-exit-keyed
teardown with the master dropped at spawn; never-joined detached drain). agent_install and
terminal_pty adopted it — net -286 lines, both suites faster than before (2.3 s vs 3.1 s),
test names unchanged. `AnswerQueries::Off` preserves the run-family lanes that script their
own reply. Stage-2 ruling recorded: NO `skit-test-pty` workspace crate — the sdist's
34-file census and Cargo lock are release-guarded artifacts not worth churning for ~60
lines; the remaining skit-cli files adopt by `#[path]`, and skit-tui's already-compliant
local harness keeps a cross-reference comment as the one documented exception.

Cleanup P1 stage 2 (`692b35f`) adopted the shared module in add_no_source, editor,
prompt_cli, and plain_add_pty (net -344 lines; the third-generation one-shot cursor
answerer and the `answer_cursor` parameter are retired across 34 call sites). Two
deliberate non-adopters keep cross-reference comments with their reasons: the `cli/tests.rs`
mirror (a `#[path]` include would compile a 450-line harness into the library build where
the coverage checker counts every line) and skit-tui's harness (the sdist census ruling).
Duplicate census: keystrokes 6 -> 2 documented survivors, settle 2 -> 0.

Stage 1 also caused — and stage 2's follow-up `ec93c84` fixed — a REAL Windows regression
that proves the consolidation's value: the shared spawn dropped `pair.master` immediately,
but portable-pty's cloned reader/writer hold NO reference to the pty `Inner`
(win/conpty.rs:130-146), so dropping master ran `ClosePseudoConsole` while the child was
still starting; the wait then panicked on the resulting `Disconnected` instead of asking
the child. Every pre-existing harness drops the master AFTER the child exits — the module
doc's invariant 4 had it backwards and is corrected with source citations. The same
over-strict treatment in `wait_cursor_query_after` and a busy-spin in `wait_exit_within`
are fixed with it; a deterministic regression owner pins the wait's semantics (RED
reproduced the CI message byte-identically). Aggregate 4074 / 0 / 525.

Cleanup P2 (`cce8f92`) unified the duplicated platform-directory suites into
`skit-store/src/paths.rs` (the filesystem-adapter layer AGENTS.md names; skit-application
cannot depend on a concrete filesystem). The audit confirmed the two copies had ZERO
behavioral divergence — pure copy-paste — so unification changed nothing, and one
parameterized rule that compiles on every host now replaces the ambient-luck coverage with
hermetic per-branch owners. The audit's micro item #6 was REJECTED with evidence:
`has_owned_draft_shape` and `existing_owned_drafts_dir` answer different questions and
produce different refusals (three tests pin the "not an owned directory" message), so
collapsing them to save one canonicalize would couple two independent guards on a
destructive path.

P2 then surfaced a REAL v0.4 compatibility defect, fixed in `c298f1b`: v0.4 resolves all
three roots through platformdirs, whose Windows arm aliases config and state to data AND
appends the appname twice (appauthor defaults to None), so the oracle's roots are
`%LOCALAPPDATA%\skit\skit`. Our Rust used `%LOCALAPPDATA%\skit` for data and state and
the ROAMING `%APPDATA%\skit` for config — a Windows user upgrading from v0.4 would have
found none of their library, state, or configuration, and no test pinned it. The oracle
spellings are now implemented and owned with citations (verified by EXECUTING the vendored
platformdirs, not by reading it). No read-time fallback was added: v0.5 has not shipped, so
no installed base exists at the roaming path. The same seam also fixed a third fidelity
divergence — v0.4 falls through to the platform default for a blank `SKIT_*_DIR` override
and trims XDG values, while `env::var_os` accepted `Some("")` and returned a RELATIVE root.
Aggregate 4077 / 0 / 525 with complete executable-source line coverage.

INDEPENDENT FINAL REVIEW (2026-08-23, read-only forks at `815305f`, per the original
mandate's "彻底完成后走几轮独立代码审查"):

Round 1 found ONE real compat break, fixed in `cbd1a08`: v0.4 stamps with
`datetime.now(UTC).replace(microsecond=0).isoformat()` -> `2026-08-23T09:48:54+00:00`
(whole seconds, `+00:00`), while the Rust wrote RFC3339 with nanoseconds and `Z` — the
same root as the platformdirs defect (a library's default rendering instead of a
translation of the oracle's formatter), in the seam next to it, on the `added_at` key the
`show --json` contract exposes. One shared `skit-store/src/stamp.rs` now serves all three
writers (store `added_at`, the completed-run `at`, and the benchmark dataset's real state
writes); the parse side stays permissive; owners pin the exact shape at the store, in
`show --json`, and in the run-state file, plus a v0.4-spelled value surviving a read
byte-for-byte. RED probes killed the offset and fraction mutants. The fix also removed an
impossible error path and a `1970-01-01T00:00:00Z` fallback v0.4 would never write. Note
for the record: the divergence survived because the existing owner only asserted that JSON
echoes whatever the meta held — true under both spellings.

Round 1 also cleared (with evidence): lock discipline (RAII, no unlock/forget anywhere in
skit-store), the create path's staging+rename being STRONGER than the oracle's build-in-place,
copy-entry originals, the library activity sort, picker case folding, glob's hidden-dot
translation, and composition-root reachability for all six newest features (real dispatch
chains from main(), nothing dead in the binary).

Round 2 found NO new compat break across TOML meta round-trip, locale negotiation,
update/rename/remove rollback safety, subprocess env/exit mapping, and Rich markup
escaping — each cleared by EXECUTING both implementations, with the oracle's own reader
run against our output as the decisive test (it read every field, both unknown-field
shapes, and both parameter rows correctly). Locale agreed 24/24 including script-subtag
precedence. Two places where v0.5 is strictly better and stays that way: our writes
preserve hand-written `meta.toml` comments (v4's tomli_w destroys them) and churn fewer
parameter keys. Every rollback hazard has an owner (they live in in-src modules, which is
why a tests/-only grep misses them).

Round 3 covered registry/state/config round-trips and the `.bak` recovery, PEP 723 byte
fidelity, the CLI help/error surface, Rich table layout, and the `{today}`/`{now}` token
surface. It found ONE real capability regression and two small items, all fixed
(`6165062`, `50e19c8`, `017af33`, `ce1e927`):

- TABLE REFLOW (v0.4 had it, we did not — a "less, not more" break): `write_table`
  computed columns from content and never consulted a terminal, while Rich fits the
  console and folds the flexible column. Fixed at the one seam every table passes
  through: width is read only when stdout is a tty, columns give back from the widest
  until the table fits, cells fold at spaces and cut unbreakable words with an ellipsis,
  wide glyphs count two. `list`, `params`, `runner list`, and the runner-rows table all
  inherit it; output is byte-identical to the oracle at widths 100/60/40/25 including CJK.
  Rich's tie-break (rightmost column gives back first) was characterized by EXECUTING
  Rich — and caught the implementation's first attempt having it backwards. Zero golden
  owners moved, structurally: tests run piped, where width detection returns None. New
  owners include a real-PTY narrow-width owner through the shared support/pty.rs.
- The corrupt-config `.bak` now carries the original's times (v0.4 uses `shutil.copy2`,
  which preserves mode AND times; we set mode only). The other two copy2 sites are
  non-members with evidence: nothing reads the stored payload's or uv staging file's times.
- The `entry not found:` wording (all three locales) is now a RECORDED deviation in
  `docs/design/rust-contract-matrix.md`. The sample answered whether it is systematic:
  of 92 oracle msgids naming a script, 48 keep the word verbatim — those about a real
  script file — and only the library-item sense became "entry", which in v0.5 can be a
  prompt or a command.

Round 3 also cleared: registry/state/config round-trips (the oracle's own readers parse
every file we write, including empty-section pruning), PEP 723 (formatting differs, parsed
values are equal in every case, and reference entries never touch the user's file), all
sampled exit codes, and the token surface byte-for-byte (including local-time correctness).

Round 4 found the ROOT that round 3's table fix was a manifestation of, plus a
discoverability regression; both are fixed (`34d4563`, `52053da`, `456e90f`):

- CONSOLE PROSE WRAPPING: the oracle's `Console()` folds EVERY printed string to the
  terminal width (and builds a SECOND console for stderr, so each stream answers for
  itself). We had fixed only tables. Measured at 60 columns before: params 131, doctor
  132, show 118, runner list's notice 102. The fold now lives in the two macros all 113
  prose sites funnel through. Rich's algorithm was characterized by reading `_wrap.py`
  and then EXECUTING it: a long word folds at the exact cell with no ellipsis (unlike
  table cells), trailing spaces are trimmed only when the line's CELL width exceeds the
  console, and the space after a folded word opens the next line. Two self-caught errors
  later, a 400-sentence x 8-width differential test matches Rich exactly, and `params` at
  width 40 is byte-identical including its trailing space. Piped output stays unwrapped,
  which is why one golden owner moved and only honestly: a 120-column PTY owner whose
  132-character refusal now folds (its terminal was widened to 200; folding is owned
  separately).
- INVENTED HELP TEXT: 81 strings (67 option helps + 14 command descriptions) were skit's
  own inventions rather than translations — their Chinese too. All are now the oracle's
  wording, with zh-CN and zh-TW copied verbatim from the .po (63 checked, 0 mismatches).
  The applying fork re-derived the package independently and found what it missed:
  `preset save --from-last` (an 18th differing flag) and an 8th `--json` site. Two rows
  were deliberately KEPT rather than replaced — `Confirm removal` serves four skit-tui
  screens, and `Refuse to prompt` serves `preset delete --no-input`, an option v0.4 does
  not have. One table-driven owner now pins all 81 strings with three Chinese canaries;
  the review's diagnosis was exactly that nothing had pinned them.

Round 4 also cleared, by driving the oracle's Textual pilot in-process: the TUI Library
screen's columns/rows/sort, the detail panel's field set, order, labels and secret
masking, and the footer command sets (ours adds only an `h` alias and `Backspace`, both
additive). The Windows-coverage statement is on record: all five earlier fixes have named
Windows-passing owners; the oracle-interop clearances, Windows locale negotiation, and the
gated interactive-PTY choreography remain Linux-only or hands-on by design.

Round 5's remaining scope: the TUI form screens (Add/Run/Settings) against the oracle's
compose(), Windows-side locale negotiation, and confirmation that the prose fold keeps
piped output byte-identical (verified locally; CI confirms).

Aggregate 4091 / 0 / 525 with complete executable-source line coverage.

The round-4 fold owners then failed on Windows for a MEASUREMENT flaw, fixed in `12783c1`:
they measured raw PTY bytes, but ConPTY prepends a cursor question, mode switches, and an
OSC window title carrying the full binary path, so a correctly-folded 59-cell line measured
past 60. The control stripper moved into the shared harness as INVARIANT 6 ("read the
visible text, never the control stream") with the Windows evidence recorded. The round-3
table owner was passing by luck in a worse way than suspected: its border-row filter
`starts_with(['┏','│',...])` never matched the chrome-prefixed row, so the row was silently
DROPPED instead of measured. Both owners now strip first; the RED probe (width detection
forced off) fails both, and the failure text is the product's own line with no chrome —
itself proof the stripping works. Aggregate 4091 / 0 / 525.

Round 5 found the SIXTH instance of the ecosystem-default class and the most visible one:
the CLI emitted no colour at all, while v0.4 colours nearly every line (105 explicit
markup sites across 253 prints, plus Rich's auto-highlighting). Fixed in `a9abb4a` /
`2266833` at the same two-macro seam as the fold, gated on `is_terminal`: 31 sites now
carry the oracle's sense (green receipts, dim hints, yellow warnings, red refusals, and
doctor's OK/ERROR/WARN rows mapping 1:1 onto its checkmark rows). The mapping was DERIVED
by extracting all 173 oracle print-with-markup pairs and matching our literals, not
guessed. `NO_COLOR` and `TERM=dumb` suppress exactly as Rich does, and the piped invariant
still holds (six commands, zero ESC bytes). Rich's auto-highlighting is RECORDED AS NOT
IMPLEMENTED rather than approximated — matching it means reproducing its whole regex
battery and precedence rules, and a subset would look right while diverging untested. The
work uncovered two defects of our own: the catalog scanner broke on the new style argument,
and a test's ANSI stripper dropped the escape byte but kept its parameters.

Two supervisor premises were DISPROVEN by the fixing fork, and it was right to refuse them:
`failed (code None)` is byte-identical to the oracle (which renders that Python-ism itself,
and neither implementation reaches it from a launch failure — the `couldn't launch` wording
lives on the status-line surface instead, now recorded), and the "48 absent TUI strings"
count is really 42 with false positives from escaped quotes defeating a substring search,
so blind translation was refused. The densest cluster (the parameter editor) was
adjudicated by reading both implementations: two hints were bare and now carry the v0.4
sentence with .po rows, and `Type` is deliberately left to our choice picker. The remaining
40 are packaged for per-string adjudication.

Two deviations were recorded in the matrix: the Windows system language (v0.4's
`locale.getlocale()` name normalises to no supported tag, so a Chinese Windows desktop
gets English there and Chinese here — more, not less, and overridable) and the post-run
status line. The Linux-untestable locale gap is closed honestly by making the negotiation
a seam that takes the host's preferences as a parameter.

Aggregate 4093 / 0 / 525 with complete executable-source line coverage.

Round 6 adjudicated the 40 packaged TUI strings ONE BY ONE and proved the package's high
false-positive rate: only 6 were genuine absences (17 present in other words at the same
surface, 17 recorded — 10 of them status-line calls already covered by that deviation).
Two of the six were REAL DEFECTS, not just missing text (`7b942b0`, `5f48312`): the
draft-delete confirmation said "Remove this entry:" — a kept draft is a file, not a
library entry, and the sentence never said the copy is the only one — and the PEP 723
block rendered only `dependencies.join(", ")`, so a fence pinning `requires-python` with
no installs showed an EMPTY LIST and dropped the Python requirement entirely. The
responsive tiers match exactly (breakpoints and footer caps 1:1); an apparent portrait
divergence was settled by the authoritative source — the oracle's CSS names both tiers
where its docstring named one.

The colour owner then failed on Windows for the measurement class again, fixed in
`3674c5a` by reading each line's ACTIVE SGR state rather than the byte stream
(`visible_with_styles`/`styles_over` now live beside the stripper under invariant 6);
ConPTY repaints and moves a style code across the line break, yet the style over each
line's characters is identical on both hosts. That fork also found a VACUOUS assertion in
its sibling's owner: the quiet-mode runs sat after the entry removal, so the yellow half
had no line to check.

The coverage gate then flagged the now-unreachable `"Remove this entry:"` fallback, and
the honest answer was neither "delete dead code" nor "add an owner" (`e29db41`): the arm
WAS reachable, and its reachability was the defect. `ConfirmDraftDelete(true)` took the
candidate without changing the stage, so `stage == ConfirmDraftDelete && candidate ==
None` was a real reducer state that today's synchronous CLI host merely hides — a Tauri
host drawing while the delete runs would land there. The candidate now lives exactly as
long as the stage that names it, and the fallback has no state left to serve. Three
mechanisms had to agree to surface this: oracle wording comparison, the 100% coverage
gate, and the frontend-neutral seam.

Aggregate 4097 / 0 / 525 with complete executable-source line coverage.

Round 6 (`7b942b0` / `5f48312`) adjudicated the 40 packaged TUI strings one by one and
DISPROVED most of the package: only 6 were genuine, fixable absences. 17 are present in
other words at the same surface (the runner CAS notices, the unknown-type/no-choices/bad-
default family, the command-template refusal, the purge receipt, the Flag label, the
draft buttons), and 17 are recorded — 10 of them `_refresh_status` calls already covered
by the post-run status-line deviation. The package also mis-claimed that the draft-delete
string had no .po row; its key carried a stray `)`.

Two of the six were REAL defects, not missing text: the kept-draft confirmation said
"Remove this entry:" (a draft is a file, not a library entry, and nothing said the copy is
the only one), and the PEP 723 block printed only `dependencies.join(", ")`, so a fence
that pins `requires-python` and installs nothing rendered as an EMPTY LIST with the Python
requirement dropped entirely. Both now follow the oracle: the confirmation names the draft
and warns, and the fence lists its Python requirement and each install, with `(none
declared)` when it declares neither. The editable dependency and Python fields regained
their hint lines. Three owners with RED probes pin all of it.

The responsive tiers match the oracle EXACTLY (80/16/10/28 breakpoints, footer caps 1:1,
detail pinning). The reviewing fork first read the oracle's docstring as "narrow AND tall
stacks" and flagged a divergence, then went to the authoritative CSS
(`Screen.-w-narrow.-h-normal #main, Screen.-w-narrow.-h-tall #main { layout: vertical; }`)
and corrected itself: that is precisely our `narrow && !is_short`. No capability is lost at
any tier.

Two process notes worth keeping: the catalog gate caught the fork mid-flight replacing the
CLI's `…(PEP 723): {}` row when the oracle keeps TWO spellings (inline for the command
line at cli.py:192, a heading for the TUI at tui_add.py:930) — both now exist with a
comment; and the new hint rows cost two rows, pushing a PTY owner's last assertions below
the fold, so that harness was raised from 24 to 32 rows rather than the assertion weakened.

Aggregate 4097 / 0 / 525 with complete executable-source line coverage.

MUTATION SCHEDULING NOTE: the 48-shard matrix re-queues on every push and has repeatedly
been superseded before getting runner capacity. The tree must FREEZE for it to finish; the
label stays attached, so the run on the frozen head is the one to read.
