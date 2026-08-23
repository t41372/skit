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

The corrected Windows forecast for the next runs (read-only scan, no fixes yet): nothing
fails to compile on Windows; the `\?\` verbatim class is neutralized because
canonicalization applies to both sides of every assertion; the real wall is PATHEXT —
extensionless `#!/bin/sh` shims are "not found" on Windows, concentrated in
`edge_workflows.rs` (first alphabetically), then `port_test_config_cmd`,
`port_test_declared_params`, and `port_test_js_deps` (143 tests). The highest-leverage
remedy is `#[cfg(windows)]` `.cmd` counterparts for the ~6 shared shim helpers, extending
the repo's existing `uv.exe`/PATHEXT conventions. `terminal_pty` under ConPTY risks
target-level timeouts (VT chatter, EOF semantics), which must not be misread as infra
flakes. The `support/temp_root.rs` doc comment overclaims "same spelling on every platform"
(false on Windows; harmless today because both sides canonicalize) and needs one honest
sentence.

After the fourth wave the full workspace suite is 4064 / 0 / 525 (one new test name, the
fork-window owner). fmt, workspace Clippy `-D warnings`, and the terminal_pty suite under
plain and symlinked TMPDIR pass. A fresh committed-state workspace LCOV run at the fourth-wave tree passed
4064 / 0 / 525 and `scripts/check_coverage.sh` returned
`complete executable-source line coverage`. macOS and Windows proof is by construction plus
the next CI run.

Receipts at `f274fea`: `cargo fmt --all --check`, workspace Clippy `-D warnings`, Rustdoc
`-D warnings`, English gate, tooling contracts, Actionlint, and Zizmor pass. The full workspace
suite is **4062 passed / 0 failed / 525 ignored**, identical in three configurations: plain;
uv-hidden PATH; and combined symlinked-`TMPDIR` plus uv-hidden PATH. A fresh committed-state
workspace `cargo llvm-cov --locked --workspace --all-targets --all-features` run at `f274fea`
also passed 4062 / 0 / 525 and `scripts/check_coverage.sh` returned
`complete executable-source line coverage`; no checker rule or exclusion changed. (Two earlier
local coverage attempts died on a full disk and on a mid-run `cargo clean`; a third hit the
6-second mirror-PTY finish deadline in two lib owners under cold-cache instrumented load — the
CI coverage job passed those same owners under instrumentation at `8306c16`, and the warm rerun
passed them, so that was scheduling, not a regression.) Arithmetic from the
`8306c16` baseline: `e665966` added 3 owners and repaired 1 flake (4060), `195587e` added the
symlink owner (4061), `2af1e3d` added the catalog owner (4062); the ignored count 525 is
unchanged throughout. The prompt-audit item "runner = 123 typed corruption" was verified CLOSED
at `8306c16` before this wave: `test_meta_rejects_wrong_typed_runner_at_the_corruption_boundary`
is an active green owner, `RawMeta` types `runner`/`dependencies`/`needs`/`params`/`parameters`,
and the three-locale corruption copy exists in the catalog.

The first current-head CodeQL run exposed four high cache-poisoning alerts in the benchmark
comparison workflow. Disabling the uv cache was not enough because `workflow_dispatch` still ran
caller-selected refs in the default-branch cache scope. The follow-up changes the workflow to the
low-privilege `pull_request` event and checks out only the event's fixed base and head SHAs. The
tooling and benchmark front-door contracts reject a return to arbitrary input refs, and Actionlint
plus Zizmor pass. Confirmed 2026-08-21: the CodeQL aggregate check and all three language analyses
are green at `8306c16`, so alerts 8 through 11 are closed.

The first complete PR #45 run at `f09e488` then produced eight red checks. They were not eight
product regressions. The final local follow-up fixes seven fixture/workflow roots: the Fish owner
now retains the normal launch announcement; doctor tests install their own private uv; the PTY
harness drains final output after child exit; portable benchmark tests no longer hide common imports
behind `cfg(unix)`; macOS expectations use the physical repository cwd; setup-uv activates and
verifies CPython 3.13 in all benchmark workflows; and CodSpeed build/run both select `--workspace`.
CodSpeed's upstream pins are already the latest compatible releases (`CodSpeedHQ/action` 5.0.3 and
`cargo-codspeed` 5.0.1). A follow-up check found setup-uv 10.0.1, so every workflow now shares its
full commit pin; v10 disables automatic caches on sensitive events and v10.0.1 adds resilient
manifest downloads. The old mutation job
failed during its unmodified baseline on the host-dependent doctor owner and did not score any
mutants. Focused owners, CLI and benchmark full suites, workspace Clippy/Rustdoc, tooling, English,
Actionlint, and Zizmor pass locally. The Linux-to-MSVC check reaches dependency C compilation and
then stops because this host has no `lib.exe`; the native Windows job remains authoritative.

The final increment has 1,110 unique `test_*` names: 969 already exist on this branch, 471 of those
are ignored/ledger owners, and six names are duplicated inside the PR increment itself. The raw
fixed tree fails `cargo fmt --all --check` and fails workspace `--no-run` at multiple independent
compile blockers (`TryLockError::kind`, an invalid raw-string delimiter, chained
`portable_pty::CommandBuilder` setters, and an undeclared `ratatui_core` test dependency). Its
support files also emit enough dead-code/unreachable-public warnings to fail Clippy `-D warnings`.
Scratch-only compile probes were discarded.

The final manifests cannot prove completeness: several collect into sets before checking
multiplicity, most scan only hand-picked split files, and they do not reject ignored/cfg-disabled
owners or prove that guard tests pass. The `3018` frozen denominator is correct, but the master
guard only proves that a guard file contains some `#[test]`. Concrete body review also found both
stronger owners and dishonest green rewrites: for example, PR moved Python's public
`flows.assemble` retyping contract to lower-level `delivery::assemble` with a hand-built
`PreparedValue`, bypassing the gate under test. Keep folding only a stronger unique body after a
three-way PR/main/Python comparison. Current accepted final-head folds are `ef5c5a2`, `a219125`,
`f5423d4`, `0e9f082`, `24e9510`, `f8091da`, `0f76635`, `c9d78af`, `949a51a`, `ccec2c6`, `8eb9b03`, `afeae66`, `9718a1a`, `4ba75ec`, `fa1e9f3`, `22103c2`, and `7fd57de`. Runner confirmation exposed and fixed a real
exit-code divergence, and the PR's seeded-runner fixture itself needed correction before its CAS
assertion was valid. Prompt editor review likewise found that the PR's untouched test used an empty
fixture and did not prove the localized starter; the corrected owner now does.

Earlier reviewed green waves on this branch
include parser mutation
contracts (`184726d`), argstate filesystem contracts (`40b6087`), atomic state contracts
(`817f14c`), two non-duplicate boolean parameter-edit guards (`7fcc177`), and packaging distribution
contracts (`606c716`). A single non-duplicate Fish managed-env delivery owner was extracted from the
latest red manifest wave (`c592560`). Two shell-analyzer stubs now have strong public contracts for
read enumeration and attached flag values (`952394e`), without importing the duplicate split files
or manifests. The existing unknown-flag owner also proves that `read -er` preserves the known raw
flag in the injected command (`cdf89ac`). The i18n replacement is green after production fixes
(`f1dc3c7`..`5d3c303`).

Two policy items keep their oracle-matching defaults (user did not object): the store self-heal
reversal (`c04395c`) and shim secret crash-safety. The latter is complete: ordinary injected Python
copies use the OS temporary directory, entry-directory staging is only the fallback or an explicit
runtime-adjacency requirement, and the final shim manifest owns both paths plus cleanup. Reversible.

## 4. Last full verified baseline (refresh on the pushed SHA)

```
git status --short          # clean at the 029f9dd release-evidence checkpoint
cargo test --locked --workspace --all-targets --all-features | <awk aggregate, §8>
# historical checkpoint => 4054 passed / 0 failed / 526 ignored
rg '^\s*#\[ignore = "FAILING CONTRACT' crates --glob='*.rs' | wc -l   # => 0
```

Do not publish 4,054/0/526 as the pushed-head count. The final authority audit added one active
prompt-kind owner and one benchmark manifest owner, removed one prompt-kind ignore, and changed the
metadata reader. Focused suites are green; the full workspace and LCOV aggregate remain the first
handoff task.

Keep the regex anchored to the start of an attribute line. The previous unanchored `grep` also
counted three module comments in `port_test_path_tui.rs`, `port_test_editor.rs`, and
`port_test_add_validation_contracts.rs` that merely showed the ignore spelling.

The full-workspace benchmark target previously had one intermittent timing failure in
`process_timeout_terminates_the_complete_descendant_tree`; 126 exact/parallel/full-binary reruns
all passed. `08046bd` hardened the test with separate 3-second pipe-holder and 250ms marker-writer
descendants, a 20ms product timeout, and a diagnostic 1-second return budget. Production did not
change; exact stress, full benchmark crate, format, and Clippy passed.
The product workspace excluding
`skit-benchmarks` most recently passed 2878 / 0 / 1134 before the six JS-deps contracts were
un-ignored. The language/runtime suites and `port_test_js_deps` are green at `81c99e7`.

## 5. Implementation fix pass — COMPLETE (0 FAILING CONTRACTs)

Counts are exact as of `86f54a4`. Every former divergence is active at its strongest executable
owner or carries an explicit architecture/private/version/semantic-duplicate closure. The final
owned-draft root has 12 active canonical owners and 12 closures across its 24 frozen names, with
typed identity, classifier, boundary, CLI/TUI host, quarantine, race, and cleanup coverage.

- **0 port_test_prompt_cli.rs + 0 port_test_prompt_kind.rs + 0 port_test_prompt_utf8.rs.** All
  implementation markers in the prompt CLI cluster are closed. Remaining ignores are explicit
  cross-crate/private/architecture classifications, including the frozen private empty-runner row.
- **0 port_test_js_deps.rs implementation divergences — JS dependency materialization.** First-seen scanner order and empty
  requirements are fixed in `54bf6c6`; mirror empty-value precedence is fixed in `c3e02b8`;
  reference implicit scanning, installer argv/fallback, and refusal voices are fixed in
  `6aedb9b`, `05e8b9d`, and `dc770fc`. Transactional explicit cleanup and the aged legacy injected
  sweep are fixed in `cc0ead1`; minimal dependency-free module manifests are fixed in `d7575c5`.
  Add-lane refusal voices are fixed in `a9e1452`. The full manifest/stamp/install transaction is
  fixed in `219a136`; the 88 remaining ignores in that file are classified architecture,
  cross-crate, private-helper, or absent-public-seam ports rather than `FAILING CONTRACT` markers.
  Oracle: langs/javascript/deps.py.
- **0 port_test_cli.rs implementation divergences.** Params drift completeness is fixed in
  `7d94289`; bare unknown directories get the frozen noninteractive `--exe` recovery before any
  read or write (`0d1a294`). Missing add-source paths use the oracle's
  localized `File not found` preflight (`22016c2`), and typed directory sources use the exact
  `Not a file` diagnostic (`4fc1659`), and unreadable files use the localized read failure
  (`de754cf`). Explicit passthrough arguments now satisfy only blank required flags (`39f9bd0`);
  the remaining items overlap the add-lane and general CLI clusters.
- **0 owned-draft implementation divergences.** Shared prompt/shebang-first classification,
  boundary recovery, post-commit consume, CLI/TUI identity claims, atomic quarantine/no-clobber
  restore, symlink/outside/replacement races, and refreshed manual-delete claims are green in
  `99a4a7c`..`86f54a4`.
- **0 port_test_declared_params.rs / port_test_params_edit.rs implementation divergences.** The
  shared pure domain engine, 41 domain owners, typed warnings, partial success/rollback, raw row
  extension merge, CLI JSON/write/purge/receipt order, and path type are green in
  `7ce033e`..`aad6f42`.
- **0 port_test_run_set.rs implementation divergences.** Its 23 executable contracts are green;
  the 4 remaining ignores are interactive/cross-crate seam classifications.
- **0 port_test_config.rs divergences.** Low-level bash-path persistence stays literal while direct
  CLI and Preferences write doors own file validation (`929e547`). The two remaining ignores are
  architecture closures; axis display runs against its correct private CLI owner (`a5710d9`).
- **Data-safety (in js_deps + elsewhere):** injected copies now use an OS-private temporary file
  unless npm adjacency requires the entry directory, Settings and `deps --clear` share one
  identity-gated cleanup transaction, and dependency metadata replacement is atomic on Windows
  (`b777d3c`). Final cleanup now reports real unlink/rmtree failures, treats a concurrent NotFound
  as success, preserves a committed replacement environment after a partial cleanup, and leaves a
  typed quarantine that the next operation repairs (`13c04fe`).
- **JS injection ownership is complete.** The 37-row frozen module has 33 unique executable owners
  and 4 structured stronger-owner closures. The CLI resolves one exact runtime, stages and checks
  the injected source before dependency mutation, and drops the temporary guard on every refusal.
  Node rejects before package.json, dependency locks, markers, node_modules, or launch. Successful
  mjs runs create the module manifest only after the gate. Dependency-backed copies prefer entry
  adjacency and fall back to the OS private temp directory on a real allocation failure. Drift and
  injected-copy failures use exact three-locale exit-125 wrappers; only drift includes `--resync`.
  Real runtime and E2E owners are active with an availability condition. Native Windows full-suite
  verification remains the platform gate.
- **Private uv bootstrap ownership is complete.** All 36 `test_uvman.py` contracts are accounted as
  31 active exact owners, 3 honest gates, and 2 structured stronger-owner closures. The final
  runtime manifest rejects duplicate occurrences before set conversion, missing or extra category
  names, ignored executable owners, undocumented gates, and residual closure stubs. Consent and
  mirror selection run through real CLI/FileConfigStore/PTY paths; musl detection uses a private
  path seam while the host still reads only `/lib`; install durability injects the actual staged
  write/sync/replace/directory-sync operations and exercises the complete locked retry path.
  Orphan-pin completeness is active in `76aaab1`. Only upstream network liveness and native Windows
  directory-sync omission remain explicit run-condition gates; no uvman parity work remains.
- **TUI path completion is complete.** `port_test_path_tui.rs` now has 62 executable contracts and
  4 honest cross-crate closures. The restored ghost completion uses a typed 2,000-entry bounded
  scanner, no cache, two bounded workers, complete-request latest-wins, dim suffix rendering, and
  Right-arrow acceptance. Secret fields dispatch no request. The terminal polls only while work is
  pending, and a real PTY proves ghost-to-launch composition. The adapter filters prefix and hidden
  misses before metadata probes while they still count toward the cap; `{cwd}` has one token
  authority, and async owners are serialized without weakening result checkpoints (`b02a3d9`).
  Missing reference origins remain available to the form projection while launch still refuses
  them (`ef76e01`..`0f7703a`).
- **Small:** no implementation divergences. The frozen 21-key show JSON contract is now a version-contract closure; the
  active v0.5 owner pins the exact 25-key strict superset (`8253219`).

The explicit JS-dependency and editor ABSENT-marker backlogs are now closed. The following JS
classification is the historical audit input, not current unfinished work. The two
source-default guards were rehomed to real
`skit-language` semantic units, and the language capability-stripping monkeypatch was classified as
a framework-injection closure (`694a717`). The JS three-way audit classified its 35 rows as 5 with
stronger owners, 13 stale owners, and 17 product gaps found at that time across cleanup,
freshness/preflight, launch sweep, diagnostics, and announce behavior. The source-edit audit found
a parser-backed resync data-loss risk on syntax errors in addition to the seven stale stubs; the
shared typed editor now closes that whole root, including partial warnings, rebind, operation order,
final-secret scrubbing, state purge, and TUI receipts (`95c8465`..`4c1689a`). The editor helper
root is now closed by public process owners and the external-edit transaction (`4832197` /
`cd15b26` / `40fa612`). JavaScript freshness, launch sweep, captured diagnostics, announce
discipline, and helper surfaces close the other audited root (`ab4d93a`..`5265eeb`).
- **Interpreter detection ownership is complete.** Seventeen frozen shebang/inference owners now
  run at the existing `skit-language` parser and classifier seams (`98efbb4`). The unreadable-path
  helper remains one honest architecture closure because Rust separates path I/O from line
  parsing. The complete 74-row accounting is now 37 baseline executable/rehomed + 1 architecture
  closure + 7 Batch A + 14 Batch C + 6 Batch B + 9 Batch D. The add-kind gaps closed in
  `dc1bac7`/`b99f36a`; the final config, POSIX E2E, and Windows interpreter owners closed in
  `2791482`/`1285121`. No interpreter ABSENT/FAILING or cross-crate runtime stub remains. The typed
  resolver is platform-neutral and Linux-tested, but a real Windows-host run remains a CI gate.
- **Store oracle ownership is complete.** `test_store.py` now accounts for all 78 exact names as
  73 executable REAL owners and 5 retained semantic/version closures. The CLI manifest rejects a
  duplicate occurrence, a missing name, a changed set, or an undocumented ignore. The final work
  includes typed Windows PATHEXT inference (`0e4a698`/`8a15675`), private CLI size owners
  (`e326985`), 15 real-binary/FileStore add owners (`cebb661`), and registry repair/race owners
  through `fae2d20`. A real Windows-host CLI run remains the native gate.
- **`test_store_fix.py` ownership is complete.** Its 38 frozen names have 32 unique executable
  owners and 6 structured stronger-owner or architecture closures. Metadata Batch 1 owns 12 rows;
  filesystem/recovery Batch 2 owns four; add/deps Batch 3 owns 13; lock/concurrency Batch 4 owns
  three. Batch 3 covers workdir defaults, non-UTF-8 byte fidelity with metadata fallback, UTF-8
  PEP 723 edits, authoritative-block refusals, untouched axes, and CRLF/LF preservation. Its one
  fault-only owner injects OSError at the private resolved-copy read inside the complete deps
  handler. No public test API or permission trick was added. The final manifest rejects duplicate,
  missing, extra, overlapping, or unstructured accounting. No rows remain.
- **`test_atomic.py` ownership is complete.** The stale 13-passed / 19-ignored baseline is retired.
  Its 32 frozen names now have 22 exact owners (15 common and 7 target-gated) and 10 structured
  architecture closures. Shared-writer owners cover file/parent sync order, temp cleanup,
  replacement retry, permission application, and no-clobber outcomes. A real child hard-exit owns
  POSIX kernel-lock release, and real `FileFormStateStore` / `FileStore` replacements own portable
  readonly preservation. Native Windows additive owners cover lock blocking/release with a
  persistent sentinel and all three permission outcomes. The final manifest rejects duplicate,
  missing, ignored-exact, empty-stub, overlap, or unstructured accounting. Windows execution is a
  real-host gate; Linux results do not substitute for it.

The ledger has the authoritative per-module adjudication log.

## 6. After the fix pass (Phase 3–5 — START HERE)

- Phase 3 gates: 100% executable-source line coverage (`cargo llvm-cov` + `scripts/check_coverage.sh`
  — do NOT relax it), i18n completeness (3 locales), ASD-STE100 English, cargo deny/audit, zizmor,
  docs build, benchmark budget, Maturin wheel + `uv tool install` smoke.
- **Final-candidate coverage COMPLETE (2026-08-21):** the first full LCOV run executed the workspace tests but the
  checker correctly failed on 2,514 uncovered executable lines. Do not add exclusions or weaken
  `scripts/check_coverage.sh`. Fresh crate reports now show `skit-domain`, `skit-application`,
  `skit-form`, `skit-runtime`, `skit-language`, `skit-store`, `skit-ui`, and `skit-benchmarks` at
  0 gaps. Ratatui per-screen coverage is also 0: Add and Management closed 430 lines in `379d395`,
  the remaining eight screen/render files closed 364 lines in `a8a1af7`, and central session plus
  terminal closed 344 lines in `559ee2c`. Benchmark coverage moved from 210 to 0; the final sequence
  covers real front doors, typed filesystem failures and races, Hyperfine/merge invariants,
  deterministic Rust tool discovery, suite adapters, footprint retries, and a real single-suite
  execute/publication path. `skit-cli` moved from 542 gaps to 0 through public CLI, real PTY,
  composition-root, and typed structural owners. The final committed-state workspace
  `cargo llvm-cov` command used `--all-targets --all-features` and wrote
  `lcov-phase3-final-green.info`. The checker read that file. All workspace tests passed, and the
  checker returned `complete executable-source line coverage`. No executable-source gap remains.
  A later fresh workspace LCOV run on the integrated candidate again executed all targets and
  features. The checker returned `complete executable-source line coverage`; the default-parallel
  workspace suite reported 4,054 passed, 0 failed, and 526 classified ignores. Workspace Clippy,
  Rustdoc, formatting, and diff checks pass on that candidate.
- **Local release evidence refreshed (2026-08-21):** `cargo deny --locked check`,
  `cargo audit --deny warnings`, Actionlint for all workflows, and Zizmor for workflows plus the
  composite action pass. English, tooling, i18n, and catalog gates pass. The docs install,
  type-check, static build, and link check pass: 90 static pages built and all links and anchors in
  34 documentation pages resolve. The local Node 24/npm 11 host is older than the documented
  toolchain; the docs workflow now pins Node 26.7.0 and npm 12.0.2 in the required order. The test
  matrix pins Node 26.7.0 before the workspace suite and makes its real-runtime owners mandatory,
  runs the ignored uv directory-sync owner on
  native Windows, and installs zsh before the real bash/sh/zsh/dash child gate on Linux. The tooling
  contract pins their presence, platform conditions, uniqueness, and order. The Windows, POSIX
  shell, and CPython 3.13 steps fail if Cargo reports zero tests. CI now has a manual dispatch so the
  current three-platform candidate can run without updating the mutation-coupled draft PR. The
  release workflow also supports
  a safe build-only manual dispatch. It installs and smokes every native-compatible Linux, Windows,
  and macOS wheel, verifies all 8 wheel archives plus the sdist and its Cargo lock, Agent Skill, and
  corpus, and permits PyPI publication only on a version-tag push. All 12 localized screenshots are
  visible through locale-matched 2x2 grids in the three repository READMEs; the old attachment
  videos and stale GIF remain absent.
  Here, declared-runtime evidence means the parser- or injection-sensitive CPython 3.13, Node,
  Fish, and POSIX bash/sh/zsh/dash paths. PowerShell, Ruby, Perl, Lua, and R share the generic typed
  spawn path and remain covered by native platform launch-plan owners instead of tool-specific
  parser behavior.
  Maturin 1.14.1 built a 6,229,905-byte wheel and a 1,742,189-byte sdist on the exact candidate.
  An isolated uv tool install passed version/help, a real add/manage/run smoke, a 34-file sdist
  corpus census, and an embedded-Agent-Skill byte comparison. The packaging and manifest owners
  also pass. The benchmark budget first caught a real release-profile defect:
  plain `cargo build --release` did not strip the binary even though the budget explicitly measures
  a stripped release binary. `[profile.release] strip = true` is now the single Cargo policy. The
  rebuilt binary is 18,858,736 bytes. A fresh 112-metric PR run passed all 8 evaluated enforced
  budgets and all 6 evaluated target budgets; the three Python-import rows are honestly not
  applicable on this Python 3.14 host. `cargo bench --locked --workspace --all-features` also passed
  and produced 23 Criterion estimate sets.
- **Phase 4 review reopened the fix pass (2026-08-21):** independent read-only reviews found real
  defects after the 100% line-coverage checkpoint. Fixed on the current branch: one shared atomic
  writer now cleans failed temp files and retries Windows replacement; injected sources use the OS
  private temp directory unless npm resolution needs entry-directory adjacency; generic copy mode
  preserves complete Unix mode bits while prompt snapshots keep the v0.4 `0o777` mask; CLI and TUI
  Settings use one identity-gated JavaScript cleanup-before-update transaction; editor command
  parsing now follows the host POSIX or Windows dialect. JavaScript installer failures now retain
  the actionable child stderr line and keep rollback atomic. Source candidate management now warns
  per invalid item while valid siblings commit once (`85ecf69`). Interactive Python dependency and
  version questions are now owned by real three-language PTY contracts (`cf33d82`). Library
  projection orchestration moved out of `skit-store` (`f5bba41`..`75fd38e`); the independent review
  blocker is fixed in `d22d390`, which derives summaries and details from one registry membership
  snapshot and one metadata read per member. The Prompt Settings placeholder picker activated its
  eight owners (`c4b8659`); `6b29372` then added option-level keyboard navigation, restored managed
  row/preview/overflow/chooser order, and removed the duplicate footer and dead reducer route.
  The reopened 25-owner PathSuggester group is now complete (`ef76e01`..`0f7703a`). The first JS
  follow-up made six real cleanup-failure/race owners executable and stopped swallowing cleanup
  removal failures (`13c04fe`); freshness/preflight is complete (`ab4d93a` / `e276058`). The
  later follow-ups closed launch sweeping, captured diagnostics, announcements, and the
  split/manifest/installer helper owners (`e62e36e`..`5265eeb`). Source resync is now one
  parser-backed CLI/TUI operation with all 14 frozen edit owners active (`95c8465`..`4c1689a`).
  Source secrecy commits and state purge now share one transaction with rollback, and completed
  runs re-read the current schema under the state lock before they persist values (`33aedcb` /
  `eac38c9`). Public editor owners and the locked external-edit snapshot close the editor root
  (`a63ecb4`..`f1ee263`). The explicit `ABSENT`/`FAILING CONTRACT` marker count is zero. The final
  coverage, supply-chain, docs, package, benchmark, and hands-on gates are not valid until these
  changes stop. The six real interpreter-cluster gaps found while adjudicating older
  `CROSS-CRATE` rows are now closed (`dc1bac7`..`1285121`). The local Windows MSVC cross-check
  reached `ring` but could not continue because this devbox has no `lib.exe`; Windows compilation
  and real-host execution therefore remain CI gates.
  The benchmark-tooling audit is also complete: 17 frozen exact owners are active, CPython 3.13 is
  one explicit three-platform host-tool gate, 116 oracle occurrences have stronger consolidated
  owners, and 22 are structured architecture closures. A fail-closed manifest preserves the
  oracle's one duplicate bare name and rejects missing, duplicate, ignored, or empty ownership.
  The denominator is 156 occurrences with no stale owner or known product gap. The Rust benchmark
  crate now has 129 test bodies, and its integrated all-target/all-feature suite is green.
  Metadata reads now also reject scalar `runner`, `needs`, and `parameters` containers at the same
  typed corruption boundary as `dependencies` and `params`; the public prompt-kind owner proves
  scan/list/doctor/resolve degradation, valid-sibling visibility, and read purity.
  The final shell pass now owns resolved-interpreter `-n` gating, self-location warnings and params
  hints, and multi-name normalization with typed refusal batching and one CAS commit. The final TUI
  pass proves each shared registry action and each local Add/management/picker action by keyboard
  and mouse at `120x30`, `46x12`, and `24x6`; compact layouts scroll instead of dropping commands.
  The final contract matrix is 20 Complete / 1 In progress.
  A review also questioned `FileStore::scan()` repairing stale registry rows. This is not a new
  blocker: `registry.toml` is a derived index, not authoritative user data, and v0.4 explicitly
  runs `_repair_rows` under a non-blocking lock. `rust-contract-matrix.md` records this narrow
  exception to pure reads. Preserve it while moving Library projection; metadata, source, state,
  and config reads remain byte-pure.
- **User's hands-on test** fits AFTER the fix pass restores behavior, BEFORE mutation. Use a real
  terminal emulator, not `TestBackend`: resize through wide/narrow/tiny layouts, wheel-scroll the
  footer, click a first-page action and a post-scroll action, and verify stale hit regions do not
  fire. In each locale, inspect CJK width, emoji, focus/cursor placement, clipping, and contrast.
- Mutation (`cargo mutants`) is **BLOCKED on explicit user approval** — it invalidates on any
  change, so it runs only after behavior is frozen; ~4.5h local, likely on another box/modal.com.
  (Local mutants runs show ~9-15% survivors on ANY branch including pristine main; the CI nightly
  on 3.13/hosted runner is the authoritative zero-survivor gate.)
- Phase 4: multi-round read-only independent review. Phase 5: the unchanged
  `docs/assets/demo/{demo,shots}.tape` product spec now produces 12 tracked localized PNGs and three
  deliberately untracked MP4s. Visual QA is complete. After hands-on, native-platform, and approved
  mutation evidence, delete `CLAUDE_HANDOFF.md`, this handoff, and mutation
  artifacts, then open the non-draft PR.

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
- **`resolve_editor_argv` / `launch_editor` / `report_saved_edit`** (cli.rs) are the editor process
  seams. `open_editor_in` deliberately accepts an ordinary nonzero editor status, as v0.4 does.
  Copy edit passes the real stored source path and uses `prepare_external_copy_edit` /
  `finalize_external_copy_edit`: an opaque repository claim prevents a frontend from substituting
  another path. The post-editor transaction verifies the exact stored path, identity, and complete
  held metadata, then reads and hashes the current bytes under the same lock. Validation and the
  success report use that returned snapshot. The transaction updates only source hash and registry
  projection. It never rewrites or rolls back the editor's bytes.
- **Message hole order:** positional `{}` formatting remains for translations whose values keep
  English order. `Message::named` and one-pass named formatting now own translations that reorder
  values (for example normalize `{name}` / `{names}`); inserted user values are never reparsed as
  format text. Do not add locale branches or multi-pass replacement.

## 9. Pointers

- **Ledger (authoritative):** `docs/design/python-test-port-ledger.md` — every module, its
  crate/file, status (`**X FIXED <sha>**` convention), and the adjudication log with per-divergence
  oracle refs.
- **Contract matrix:** `docs/design/rust-contract-matrix.md` (21 rows: 20 Complete / 1 In progress;
  the final row needs hands-on, native/declared-runtime CI, and approved mutation evidence).
- **AGENTS.md / CLAUDE.md:** product rules, trust model (skit is a launcher, NOT a sandbox — do not
  add sanitization/threat-mitigation for trusted local content), architecture, gates. Commands to
  run gates are in AGENTS.md "## Commands".
- **Machine-local `.claude` memory is NOT a dependency** — everything essential is inlined here.
