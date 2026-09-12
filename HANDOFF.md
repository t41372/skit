# UI Walker Real-Host Migration Handoff

Updated: 2026-09-11 (Claude: third ChatGPT report adjudication and the focus-reporting fix).

Read this file completely before changing source. This handoff and both phase plans are tracked.
`PHASE3D-PLAN.md` is the approved Phase3d specification, and **`PHASE4-PLAN.md` revision 30 is the
authoritative Phase4 specification**. Read
`PHASE4-PLAN.md` before touching Phase4. It holds the commit ladder A through H, the measured
`Action` inventory, the corrected JSON pointer table, and the reasoning from the advisor rounds.
It is updated through completed C3 and splits the former commit C into C0a, C0b, and C1 through C3.
D through G3 and H are committed and closed. Use the current checkpoint below for G4 and the
remaining gates. The current checkpoint supersedes the retired normal-launch snapshot sections.
This handoff summarizes state; that file
holds the design.

## Current checkpoint: 2026-09-11 third review adjudication and the focus-reporting fix (Claude)

The user supplied a second report dated 2026-09-11 (`skit-ui-walker-deep-review-2026-09-11 (1).md`,
target `49e94911`). The input stays unchanged and local. See
[the adjudication](docs/reviews/ui-walker-20260911/chatgpt-review-2-adjudication.md) for the four
findings, the decisions, the independent review, and the validation.

Commits on top of `6e27ce30`:

- `3edd8058` — F1: the suspend before a host effect and the resume after it now toggle the same
  screen modes as the claim and the restore (alternate screen, mouse capture, focus reporting).
  The 2026-09-08 F8 repair had enabled focus reporting at the claim only, so a script or editor
  launched from the TUI received focus reports as input on a window switch. One shared command set
  (`enter_screen_modes`, `leave_screen_modes` in `crates/skit-tui/src/terminal.rs`) serves the four
  transitions; the leave order is the reverse of the enter order. A PTY test in
  `crates/skit-tui/tests/terminal_pty.rs` asserts the toggles around the host effect on real
  output bytes, and a unit test pins the command set.
- `b57cdcef` — the adjudication, the first form of this checkpoint, and the guarantee boundary of
  the parity probes in `MIGRATION-DESIGN.md`.
- `3e6042ae` — the coverage gate counts every line of a unit test inside a crate; the assertion
  messages of the new terminal unit test built their text on lines that ran only on failure. The
  test converts the bytes once and uses inline captures.
- `201ba830` — a race that the CI coverage job exposed in
  `different_profiles_can_initialize_and_run_in_parallel`: the Linux ticket scan in
  `crates/skit-cli/src/cli/tui_real_sandbox_fs.rs` lists a directory and then opens every name, and
  one profile's remove verification opened a sibling's transient cleanup directory at the moment
  that sibling removed it. An entry that is absent at its open now gets no ticket; every other open
  failure stays an error. A pure `present_tickets` takes the open as a parameter, so one test drives
  every answer, and a second test pins the real `NotFound` mapping of `ticket_for_name`. Not
  related to the focus fix; the walker's cleanup design is unchanged.
- The final commit adds this checkpoint and the CI record.

Decisions: F2 (a `String` status carries both catalog keys and rendered text) is a base-branch
trade-off with no reachable wrong translation; deferred and recorded. A1 (walker introspection in
the public surface of `skit-tui`) needs one owner decision: never publish to crates.io, or gate the
seam behind a non-default feature before a first publish. No workflow publishes a crate today, and
`publish = false` on `skit-tui` alone would also block `skit-cli-rs`. A2 is documented.

Rules for later PTY tests in `skit-tui`: end a wait at drawn text, not at a control sequence. The
resume calls `Terminal::clear`, and ratatui asks for the cursor position there; the harness answers
that question only inside a wait, so a wait that ends at `?1049h` can leave the question unanswered
and the child fails after two seconds. The exit phase does not answer queries.

Rule for unit tests inside a crate: the coverage gate counts every line of the test. Build an
assertion message from values that exist before the assertion, and use inline captures; a value
computed on a separate line inside `assert!` runs only on failure and the gate reports it.

Opus reviewers verified both fixes in scratch worktrees. The focus fix: one blocker (that flake)
and five nits, all repaired before the commit was rebuilt; the reviewer confirmed by hand-applied
mutants that the tests kill an emptied helper and the two helpers swapped. The scan fix: zero
blockers and four nits (a real-open test variant, this checkpoint, wording, and one line of
rationale), all repaired. `cargo mutants` skips the `#[cfg(test)]` walker modules, so the mutant
analysis of the scan fix is by hand and is in the adjudication.

Gates on `3edd8058`: Linux fmt, workspace Clippy and Rustdoc with warnings denied, every
`skit-tui` target, thirty consecutive PTY runs, the `skit-cli-rs` PTY tests (41), and the English
check; macOS Clippy, Rustdoc, terminal unit tests (23), five PTY runs; `cargo xwin check` for
`skit-tui` with zero errors and no new warning. Gates on `201ba830`: Linux Clippy for
`skit-cli-rs`, the sandbox filesystem module tests (32), ten runs of the parallel-profiles test,
and the English check; macOS Clippy; `cargo xwin check` for `skit-cli-rs` with the same warning
set as before.

Still open, for the owner: the 64-shard mutation gate; a pull request; the `skit-tui` publish
decision. The dedicated `ui-walker.yml` workflow had never run on this branch; see the CI record
in the adjudication.

## Previous checkpoint: 2026-09-11 review adjudication and module split (Claude)

The user supplied `skit-ui-walker-deep-review-2026-09-11.md` for independent verification
against `42f9767f`. The input stays unchanged and local. See
[the adjudication](docs/reviews/ui-walker-20260911/chatgpt-review-adjudication.md) for the three
findings, the decisions, the split method with its move-only proof, and the validation.

Commits on top of `42f9767f`:

- `75ee5e6d` — split `tui_real_host.rs` (23,610 lines) into a 29-line parent, nine
  implementation files, `tests.rs` with the shared fixtures, and eighteen topic test files under
  `crates/skit-cli/src/cli/tui_real_host/`. Code moved only. Items and struct fields that tests or
  sibling files read are `pub(super)`; the old `pub(super)` items are `pub(crate)` and the parent
  re-exports the names that sibling modules import.
- `8cf34611` — split `tui_real_walker.rs` (8,578 lines) the same way under `tui_real_walker/`.
- `79a493fa` — F1: `SKIT_WALKER_CASES` and `SKIT_WALKER_STEPS` refuse a present value that is not
  a positive integer; `SKIT_WALKER_LIVENESS_EVERY` keeps `0` as "never" and refuses a
  non-integer; the reader tests are hermetic through the child-process helper.
- The final commit adds this checkpoint and the adjudication.

An Opus reviewer read the frozen change in a scratch worktree and verified every finding before
reporting: zero blockers, one major, four nits, all repaired before the commits above were
rebuilt. The major was two generated imports whose `#[cfg]` was broader than their use sites
(dead on Windows); the nits were a comment that rustfmt had sorted away from its import, two
references to the old walker file name, a refusal message that was wrong for a negative
liveness interval, and the lost assertion on a zero default in `positive`. The reviewer also
confirmed the move-only claim at token level (host 741 items before, 742 after with the one new
helper; walker 336 both times) and that the enabled test sets are identical per platform
(macOS 150 and 41, Linux 210 and 59, Windows 154 and 59).

The split script, its specs, and the proof reports are kept outside every repository at
`/Users/tim/LocalData/coding/2026/Projects/6-skit/skit-uiwalker-split-tooling/` (the repository
bans Python tooling files). Reuse it for a later split of `tui_walker_bundle.rs`.

Rules that the split introduced. Any new child-process test must build its harness name with
`harness_test_name(module_path!(), "<test>")` from `tui_real_host/tests.rs`; a literal module path
goes stale on a move and libtest then runs zero tests in the child. A test file whose every item is
platform-gated needs the same `#[cfg]` on its `mod` line in `tests.rs`. `tui_walker_bundle.rs`
(6,898 lines, tests interleaved with code) and `tui_real_sandbox_fs.rs` (4,093 lines) stay whole;
both are smaller than the base's `cli/tests.rs`, `cli.rs`, and `session.rs`.

Gates run for this checkpoint (rerun after the review repairs). Linux, in the OrbStack worktree `~/coding/skit-split`: workspace
Clippy with warnings denied, Rustdoc with warnings denied, `cargo fmt --check`, the model crate
(149 passed), and the complete `skit-cli-rs` library suite (662 passed, 0 failed, 6 ignored,
108.25 s); each of the four child-process tests spawned a child that ran exactly one test. macOS:
`cargo check`, workspace Clippy with warnings denied, and the macOS-compilable walker and host
tests (188 passed, 3 ignored). Windows: `cargo xwin check --locked --target x86_64-pc-windows-msvc
--workspace --all-targets`, zero errors, pre-existing warning set. F1 in its own worktree: model
crate coverage complete, `cargo mutants --no-config` over `artifacts.rs` zero missed. Full Linux
coverage and the complete workspace suite on all three platforms ran in CI, dispatched for
`49e94911` as the earlier runs were: all seven jobs passed at
https://github.com/t41372/skit/actions/runs/34642242849 (format, lint, and documentation;
Linux, macOS, and Windows tests; the 100% executable-source coverage gate; the dependency and
workflow audit; PyPI and uv tool compatibility). The full mutation gate remains unrun; the 64-shard budget
decision stays with the user.

The existing corpus identities bind the old tree. A moved-file tree has a new source identity.
Nothing was re-recorded and nothing is retagged; a move-only change cannot change recorded bytes.

## Previous checkpoint: ChatGPT report adjudication (2026-09-08)

The user supplied `skit-ui-walker-deep-review-redone.md` for independent verification against
`501be70a`. The input stays unchanged and local. See
[the adjudication](docs/reviews/ui-walker-20260908/chatgpt-review-adjudication.md) for all nine
findings, accepted repairs, rejected premises, and validation limits.

Ordinary and raw copy launches now retain the stored script path. The existing removal lease,
source checks, atomic writes, and completed-run state transactions remain. Normal launch snapshot
machinery is removed. Injection still uses a temporary source when its bytes must change.
Raw mouse dispatch was already one event; its Quit probe now tests that same event. Inventory
errors propagate. Non-pointer input cancels an armed click. Terminal focus reporting is enabled
and restored, and it is also disabled before a host effect and enabled again after it. Random
cases use the configured length. Successful replay compares all resolutions and skips; failure
replay compares the recorded boundary. Runtime helpers accept trait objects without three
forwarding wrappers. Two duplicate sandbox directory syncs are removed.

Repairs are folded into the related commits. The stored-path change is a separate product
commit. The complete Mac workspace reached only two obsolete snapshot-path assertions; both are
corrected, and all six affected fixture targets pass (199 tests). Component suites and Clippy pass.
The first native CI is `34291632236` at `3db88637`. Its follow-ups restore a Unix-only test
condition, inline a duplicate path-map guard, and make two PTY tests wait for the review screen.
The complete Windows workspace cross-check and all 41 Mac PTY tests pass. Complete Linux
executable-source coverage passes at 100%. Final CI passed all seven jobs at `7cbac127`:
https://github.com/t41372/skit/actions/runs/34293517714 . The final amendment updates only this
handoff and the adjudication report. The new four-profile corpus passed in 1115.03 seconds as
`corpus-60a1a885045ccd40758242fb3cfe6cd9bb958a23696bc781e1ae27980155634e`. Keep the old corpus tied to its
original source. The new Linux recording uses the frozen source patch at
`target/uiwalker-evidence/source-followup-501be70a.patch`, based on `501be70a`. All 177 crate source
and Cargo files matched `3db88637`. Later repairs fix test synchronization and platform gating,
and inline an equivalent path-registration helper. Do not retag that recording with a later commit.
The corpus archive is `target/uiwalker-evidence/corpus-followup-60a1a885.tar.gz`; its checksum and
the patch checksum are in `target/uiwalker-evidence/SHA256SUMS-followup`.

The user already authorized rewriting and pushing this integration branch and running CI. No
Rust or mouse branch is changed. No merge is authorized by this review request. The full mutation
gate remains unrun and unwaived; do not start the large matrix without its pending budget choice.

## Previous checkpoint: history split and source review

The user requested a direct Rust merge path. Do not preserve `fix/tui-mouse` as a required
parent or merge target. The complete change from Rust is now organized into thirteen commits,
with seven product repair commits separate from the mouse/host foundation. Local backups are
`feat/uiwalker-history-backup-20260908` and `feat/uiwalker-before-ui-fix-split-20260908`.
The first split kept tree `1afb67b5c2736db48a471e225932cca24dd0d65d` byte-for-byte.

The requested source review used seven read-only agents against Rust base
`5af68f8f7a796e4483d3c9900db90d3939b4cb56`. See
[the source review](docs/reviews/ui-walker-20260908/source-review.md) for scope, findings,
reproductions, and decisions. Six defects were accepted after independent checks. Repairs cover
Windows snapshot readers, Run and Language picker behavior, corpus source identity, editor
result recording, and completion-time Library ages. Repairs are folded into the related commits.
The older corpus reviews and source hashes below remain historical evidence.

The user authorized rewriting and pushing this integration branch and running CI. No Rust or
mouse branch has been changed. No PR was created or retargeted, and no merge has occurred.
Native CI for `e8dcb545` passed all three platform test jobs, packaging, and the audit.
Its quality failures were confined to new test assertion style and two uncovered test
failure branches. Those fixtures were corrected and checked with Linux Clippy and focused
LLVM coverage. The final complete CI passed all seven jobs for `a47d874b`:
https://github.com/t41372/skit/actions/runs/34230692345 .
This includes Linux, macOS, Windows, 100% executable-source line coverage, quality checks,
audit, and packaging. The final follow-up commit updates only the handoff and source-review
report. It does not change the tested code.
The full mutation gate remains unrun;
do not treat the earlier Rust PR deferral as an automatic waiver for this branch.

## Start here: the state on 2026-09-08 (Codex)

This section supersedes the older checkpoints below. Do not turn old findings into new tasks
without checking the current code and evidence. The user again required simple solutions for a
trusted local application. Preserve data and operating-system semantics; do not add a hostile-user
model. The user approved Terra high and Luna max for the final comparison.

### Current source and commits

Mac worktree: `/Users/tim/LocalData/coding/2026/Projects/6-skit/skit-uiwalker`.
Branch: `integration/ui-walker-on-mouse`. The user authorized a push and fresh CI; commits through
`193d7377` are pushed. No PR retarget has occurred.

- `a0ae7ea9` — align the documented mutation command with `--timeout 300`.
- `db104289` — invert the frame oracle, remove observed-form/clip grants, share text boundaries,
  retain raw facts for non-frame checks, and scan stable checkpoints before recording them.
- `473a9016` — use explicit English receipts and canonical source paths in portable host tests.
- `c011f095` — preserve metadata flag/environment riders on source-owned settings saves.
- `c42ffcb7` — remove the invalid provisional Windows directory metadata query. Creation identity
  and promoted-handle metadata validation remain. The next native run passed that contract.
- `4306c86b` — remove path, JSON-escape, display-quote, executable-suffix, clock-precision, and
  locale-fixture assumptions from host tests. Random locale smoke uses a copied entry because a
  reference detail can paint a random path after the save refresh.
- `949c7942` — close the remaining keyboard and geometry coverage contracts. The picker test uses
  a fixed Unicode memory source. Dropdown tests check the actual painted border and recorded
  geometry instead of copying the placement implementation.
- `03536e05` — preserve the independent reports and the root comparison under
  `docs/reviews/ui-walker-20260908/`.

The first updated-head CI (`34185925301`) then found two more shared causes and native fixture
assumptions. These repairs are committed:

- `13160ba0` — translate only complete catalog keys in the footer. Host receipts are already
  localized. Translating the complete receipt again changed user path letters in pseudo locale.
  The four-locale regression first failed on `/tmp/on/off/agent/SKILL.md`. The full TUI suite and
  the exact corpus regression with `TMPDIR=/tmp` passed (86.55 s).
- `c016763d` — use writable handles for fixture timestamps, the held lease handle for its own
  byte checks, and marker byte corruption for cleanup failures. Windows tests assert its actual
  no-delete handle semantics; Linux still exercises removal and replacement. Two more launch
  display checks compare parsed arguments, and one user-text check handles JSON escaping.
- `28df410e` — capture the profile root's resolved spelling once. Keep it and the declared spelling
  in existing path facts. Use one mapping for JSON paths, source comparisons, and host text.
  Stable frames can contain either known spelling. This fixes the native 8.3/canonical mismatch
  without changing the declared namespace or adding renderer-derived grants.

Mac and Linux library suites reached one obsolete empty-facts assertion after these repairs.
It now asserts the exact initial profile fact. Its focused rerun passed. Workspace Clippy and
Windows cross-check passed. Full Linux coverage passed again with no exclusions changed.
The full corpus passed twice against a frozen Linux tree in **1017.28 s**. Source evidence is
`target/inversion-evidence/native-repairs-{source.patch,base.txt}`;
logs are `~/coding/.tmp/uiwalker-native-repairs-{coverage,coverage-gate,corpus}.log`.
The latest corpus is
`target/ui-walker-review-corpus/corpus-b77347af5f56e9b77bcafc601a071daeaae03f1a62f9da93b35eb8591f2aabcb`.
Its source identity is
`aee95d086eaada51ffd585890b5eb1b4962c4b47+worktree:ced3df783d9eb6818704d2de8937976392fe8bffec8fad057fb254e46ce91f41`.
All 429 tracked Rust and Cargo files matched commit `28df410e`. Each profile has 181 rows.
Every later Run retains `value:pattern`; pseudo sequence 55 now preserves the exact path and
has one set of pseudo markers. The latest full macOS workspace suite passed as well.
The new coverage artifact is `target/uiwalker-native-repairs.lcov`, SHA-256
`ef43155f2b1eb1d0d9025942d64bc035910ce59c6c2c43a438fd7c88ffee11b2`.
The follow-up push completed. Native CI for `28df410e` ran at
https://github.com/t41372/skit/actions/runs/34187935746 .
All jobs except Windows passed. The four remaining Windows failures are described below.

Native CI `34187935746` at `28df410e` passed every job except Windows. Windows reached
556 passed / 4 failed / 3 ignored in the CLI library. The remaining repairs are:

- `5772cd86` — compare JSON-encoded path text in two assertions and parse the final JavaScript
  fallback launch display. The query assertion was redundant with the next typed-field assertion.
- `9517bced` — compare replay's produced `Effect` with the decoded typed `Effect`, so Windows
  `PathBuf` separator semantics apply. A JSON round trip still refuses discarded extra fields.
  A real reducer contract first failed for two spellings of the same skill path, then passed.
  It still refuses a changed target, an extra field, and a non-Effect value.

The last change only affects the read-back comparison and test assertions. The existing full
corpus remains bound to `28df410e`; do not retag it. The new reader validated the completed Terra
copy of the original 724-row corpus in 100.47 s. Mac CLI library: 541 passed / 10 ignored.
Workspace Clippy and Windows cross-check passed; the Windows warning set is unchanged.
Full Linux coverage passed again with no exclusions changed. The log is
`~/coding/.tmp/uiwalker-replay-semantics-coverage.log`; the gate log ends in
`complete executable-source line coverage`. Artifact: `target/uiwalker-replay-semantics.lcov`,
SHA-256 `3a06b53f47527230b82d3e3dd50e7cc2215baa61d4538e40169cda770ddb3a9e`.
The source patch and base are in `target/inversion-evidence/replay-semantics-{source.patch,base.txt}`.
The push through `9517bced` completed. Its native CI ran at
https://github.com/t41372/skit/actions/runs/34189833000 .

CI `34189833000` at `9517bced` passed every walker and CLI library test on native Windows:
**561 passed, zero failed, 3 ignored**. The full workspace then reached one pre-existing
`edge_workflows` fixture that set HOME but inherited the real Windows USERPROFILE.

- `129dbb24` — pin USERPROFILE beside HOME in completion and agent-install fixtures. Clear
  HOMEDRIVE/HOMEPATH for the missing-home case. Product home resolution stays unchanged.
- `a090e95d` — bind root aliases by their declared physical root. The canonical seed profile
  (`canonical-corpus`) and the review profile (`en-80x24`, etc.) are different IDs. A corrected
  fixture first failed under the old ID lookup. The physical-root match passes it.

The four affected integration targets passed on macOS. Clippy and Windows cross-check passed.
The updated full Linux coverage gate passed with no exclusions changed. Artifact:
`target/uiwalker-home-root.lcov`, SHA-256
`d2f66e0a890527e39b23c0a921c2a9cc1d03a5ab3328c71c44afd010186e5587`.
Logs: `~/coding/.tmp/uiwalker-home-root-coverage{,-gate}.log`; source patch and base:
`target/inversion-evidence/home-root-{source.patch,base.txt}`.
The push through `a090e95d` completed. Its native CI ran at
https://github.com/t41372/skit/actions/runs/34191888817 .

CI `34191888817` at `a090e95d` passed every non-Windows job. Windows passed the complete CLI
suite, then reached one store integration assertion: its expected `skit/SKILL.md` string used
mixed separators while the installer joined each native path component.

- `ba718eef` — compare that fault target with `Path` equality. The store target passed 18 tests
  on macOS; no store implementation changed.
- `780d95d0` — add `--no-fail-fast` to the CI matrix's full workspace test command. Cargo still
  fails the command if any target fails, but later test binaries now run and report in the same
  job. This prevents one failed target from hiding the remaining native failures.

The Linux tooling contracts and zizmor passed for the CI edit. The push completed, and the new
CI ran at https://github.com/t41372/skit/actions/runs/34194014805 .
Do not rerun unchanged corpus generation for these test and CI edits.

The complete Windows run `34194014805` at `780d95d0` executed every test target. All targets
passed except `skit-tui-walker-model --lib`: one fixture assumed a directory could not replace
an existing file, which Windows permitted. This was the complete failure set, not a stopped run.

- `193d7377` — give the publication-failure fixture a regular file in the destination's parent
  position. The rename now has an invalid path on every platform, so the test can check staged
  cleanup without changing the operating system's rename semantics. No implementation changed.

The full model crate passed: 143 library tests and 11 integration tests, with two ignored owners.
Package Clippy passed. The push completed. The current complete CI passed at
https://github.com/t41372/skit/actions/runs/34196664736 .
All seven jobs passed at `193d7377`: Linux, macOS, Windows, complete executable-source coverage,
format/lint/docs/i18n/tooling, dependency/workflow audit, and PyPI/uv compatibility. The native
Windows uv durability gate ran exactly one test and passed. The Windows CPython 3.13 corpus
compile gate also passed. Full CI log: `/private/tmp/skit-uiwalker-final-ci.log`.

The approved push, corpus generation, independent review comparison, confirmed repairs, and
native CI work are complete. The full mutation gate remains unrun and is not waived. Its
execution budget needs a decision, as recorded below. No history rewrite, PR retarget, or merge
has occurred.

The exact frame policy and its rationale are in `G4-DESIGN.md` D5. Stable rendered paths and names
are data. Ambient paths are checked in every channel. A known namespace prefix may end at the
physical content edge, including widget padding; this uses declared metadata, not observed grants.
JSON keeps its typed projections. Random diagnostic walks do not acquire the stable corpus gate.

### The first complete real corpus and model comparison

The four-profile, 100-operation owner passed twice with identical immutable bytes in **1490.69 s**.
There are 181 rows per profile, 32 chunks, and 1,576 unique objects. The installed size is 218 MiB.
The preserved pre-repair corpus is:

`/home/tim/coding/skit-codex/target/ui-walker-review-corpus/corpus-91b4e24d55fd694106b8ae3d1c8802875e0dcb5d3406e14f3205890a2072e38d`

Its source identity is
`aee95d086eaada51ffd585890b5eb1b4962c4b47+worktree:1373509cdf9972dbb8d88ef43d106b18d809fa7cf3465bf521184a98e2b6002f`.
Its source patch and base are preserved in `target/inversion-evidence/` in that checkout.

Terra high and Luna max received the same prompt and independently reviewed byte-identical copies.
Both read all 32 chunks, 724 rows, and 1,576 objects. Both completed reports passed the real Linux
validator (77.19 s and 77.11 s). Copies are under
`target/ui-walker-review-comparison/91b4e24d/{terra,luna}/` in the same checkout.

Root assessment:

- Luna found real data loss. An untouched settings save removed metadata riders on source-owned
  kinds. The seed is valid: v0.4 and current form planning support these riders, and the CLI already
  preserves them. The repair is `c011f095`. The regression covers Python, shell, JS, TS, and fish,
  copy and reference modes, unchanged saves, unrelated edits, source bytes, and later form fields.
- Terra reported the word `agent` as an untranslated title. Chinese catalog entries exist, and
  both Chinese READMEs and many UI rows use `AI agent` as established terminology. This is not a
  missing-translation defect. No product change was made for that claim.

The reports describe the preserved pre-repair corpus. Do not retag it with a newer source revision.
The root comparison records scope, completeness, misses, false positives, and measurement limits.
Later native CI confirmed a footer defect already visible in the reviewed pseudo frame at
sequence 55. Both reviewers missed it. The comparison now counts two missed confirmed defects
for Terra and one for Luna. Windows-only failures are not charged to Linux corpus reviewers.

### Linux validation checkout and source freeze

Use `orb -m skit-linux bash -c '. ~/.cargo/env; export TMPDIR=~/coding/.tmp; cd ~/coding/skit-codex; ...'`.
The Mac path is `/Users/tim/OrbStack/skit-linux/home/tim/coding/skit-codex`.
This is an assistant-created worktree. The older `skit-rev`, `skit-u6b`, and `skit-uiwalker` clones
contain prior uncommitted work and were preserved. The old claim that `skit-u6b` was clean was false.

`skit-codex/target` is a real ignored directory. Its `debug` child links to the existing
`skit-u6b/target/debug` cache. Its coverage target and corpus files are local to `skit-codex/target`.
Do not replace `target` with a symlink: the directory-only ignore rule then exposes it to Git.

An automatic approval review rejected a proposed `git reset --hard FETCH_HEAD` because the Linux
tree was dirty. No reset ran. The safe alternative preserved every file. A SHA-256 comparison
verified all **429 tracked Rust source and manifest files** against the committed Mac source with
zero differences. The generator can bind that frozen worktree with its existing source identity.
Do not discard its changes just to obtain a clean status.

A post-repair double generation passed against this preserved source in **992.00 s**. Its log is
`~/coding/.tmp/uiwalker-final-corpus.log`; its source patch and base are
`target/inversion-evidence/final-corpus-source.patch` and `final-corpus-base.txt`.
The final corpus is
`target/ui-walker-review-corpus/corpus-03efa4de5166e16029303ab3fca3d8eed158e426b8431b1a3f0e520d4d277158`.
Its source identity is
`aee95d086eaada51ffd585890b5eb1b4962c4b47+worktree:b41d6f0f67b71ebec05e92080af4faa581452f0808e058735c5131e4b39de9a3`.
All four profiles retain `value:pattern` at the later Run responses, sequences 168 and 172.

### Measured gates

- Full macOS and Linux workspace tests passed. The Linux warm test targets total about 141 s.
- Full Linux executable-source coverage passed with no exclusions changed. Artifact:
  `target/uiwalker-final.lcov`, SHA-256
  `04b28e2e0ba627aac2b34c1f7e48cee27873f517656d28a509e89485434fabcd`.
  The gate log says `complete executable-source line coverage`.
- Workspace Clippy with warnings denied passed. Windows `cargo xwin check`, workspace/all
  targets/all features, passed; unrelated Windows integration-test warnings remain.
- Workspace Rustdoc, docs install/type/build, cargo deny, cargo audit, and zizmor passed.
- The updated macOS wheel and source archive built with Maturin 1.14.1. Both installed through
  task-local uv tool directories and ran `skit --version`. The sdist build needed the already
  installed `RUSTUP_TOOLCHAIN=1.97.1`; the machine's default 1.96 is below the project MSRV.
- Linux tooling, English, and coverage-script contracts passed. Do not trust the Mac tooling
  script's exit code alone: BSD sed did not execute one GNU-sed negative fixture.
- Scoped mutation evidence for `leak_oracle.rs`: **35 tested, 28 caught, 4 unviable, 3 timeouts,
  zero missed**. This is package-scoped development evidence with cargo-mutants 27.0.0, not the
  workspace gate. Evidence is in `/private/tmp/skit-support-mutants.xYJu1g/evidence/mutants.out`.
- The earlier workspace inventory had **11,992 mutation candidates** on Linux (27.1.0). The full mutation gate
  has not run. Do not claim it passed. The user has a pending choice about starting this long run.
  The final GitHub Linux job at `9517bced` measured **384.69 s for the CLI library alone**
  (658 passed / 6 ignored). That exceeds the current mutation `--timeout 300` setting.
  The local VM's 141 s workspace measurement does not predict the CI runner. This is duration
  evidence, not a mutation result; assess the execution budget before dispatching all 64 shards.
  Native Linux log: `/private/tmp/skit-uiwalker-final-linux-ci.log`.

The Mac worktree also has local archives under `target/uiwalker-evidence/`: the final
`b77347af` corpus, the original reviewed `91b4e24d` corpus, and Linux source evidence.
Each archive passed `gzip -t`; `SHA256SUMS` binds all three. The README states their source
provenance. These build artifacts are not in Git.

### Native CI and remaining work

The existing workflow was dispatched against the already-pushed **old** head `aee95d08`:
https://github.com/t41372/skit/actions/runs/34174102595

Its PyPI/uv compatibility and dependency/workflow audit jobs passed. Other jobs failed:

- macOS exposed the canonical-path test assumption repaired by `473a9016`.
- Linux tests, quality, and coverage exposed the random reference path in locale smoke. It was
  reproduced with `TMPDIR=/tmp`, then repaired by the copy fixture in `4306c86b`.
- Windows exposed the `fs_at::mkdir_at` handle: it has no `FILE_READ_ATTRIBUTES`, while the caller
  immediately requested ordinary metadata. The native red is real. `c42ffcb7` retains its file ID
  and validates metadata on the promoted readable handle. Other verified Windows fixture errors
  were quotes, JSON escapes, `uv.exe`, and 100 ns clock precision; those are in `4306c86b`.

The updated-head native run passed the directory-creation contract, then reported 42 failed
library tests. Most shared the read-only timestamp helper. The remaining groups were lease/read
and no-delete fixture assumptions, two display checks, one escaped user-text check, and real
8.3/canonical source projection failures. The three follow-up commits above address these causes.
The next native run must verify them; cross-compilation alone does not prove them.

The user authorized pushing the tested commits and reports to `integration/ui-walker-on-mouse` and
running fresh CI. That push completed. The first updated-head CI failed at
https://github.com/t41372/skit/actions/runs/34185925301
and the existing x86_64 full benchmark workflow passed at
https://github.com/t41372/skit/actions/runs/34186079806 . No PR base has changed.

The Linux VM is **aarch64** with Node 22.22.1 and Hyperfine; `uv` and `strace` are not on its PATH.
It is not the `linux-x86_64` benchmark reference host. The real reference-host benchmark at
`03536e05` passed: 13 enforced rows, 11 evaluated, 11 passed, zero failed. The log is
`/private/tmp/skit-uiwalker-benchmark-native.log`; CI retains `benchmark-results-full`.
The first updated-head CI macOS tests, PyPI/uv compatibility, and dependency/workflow audit passed.
Linux tests, coverage, and quality shared the footer double-translation failure described above.

Next: honor the user's mutation choice and plan a run that fits the measured CI test duration.
Then complete the full mutation gate and obtain explicit authorization for the history and PR
changes. Keep every claim tied to its actual source.

## Previous checkpoint: the state on 2026-09-07 (Claude, Fable 5.1, orchestrating Opus 5 implementers)

This checkpoint is history. Use the newer checkpoint above for the current state and decisions.

### Where everything is

- **The branch.** `integration/ui-walker-on-mouse`, tip `35ba1c27`, pushed to `origin`. Twenty-two
  commits sit on `128fe52a`, the durability blob that froze the rescued work. The Mac worktree is
  `/Users/tim/LocalData/coding/2026/Projects/6-skit/skit-uiwalker` and it is clean.
- **Linux.** The stable walker sandbox refuses macOS by design, so every corpus test is gated on
  linux and windows and compiles out on a Mac. Use the OrbStack machine:
  `orb -m skit-linux bash -c '. ~/.cargo/env; export TMPDIR=~/coding/.tmp; cd <clone> && <cmd>'`.
  Clones: `~/coding/skit-rev` (holds the unfinished `u5f/commit-payload`) and `~/coding/skit-u6b`
  (free; sync it with `git fetch` plus `git checkout FETCH_HEAD -- .`). Their Mac-side path is
  `~/OrbStack/skit-linux/home/tim/coding/<clone>`, which is how you read and edit their files and
  how the Mac repository fetches their branches.
- **Windows.** No Windows machine, but a real cross-check works on the Mac and has caught a live
  defect: `cargo xwin check --locked --target x86_64-pc-windows-msvc --workspace --all-targets
  --all-features`. It type-checks every test body. It cannot run anything.
- **Side branches** held in the Mac repository, none pushed: `u5/g4-repairs`, `u5c/quarantine-leak`,
  `u5d/corpus-leaks`, `u5e/frame-root`, `u6/migd2-mige2`, `u6b/replay-repairs` (all merged into the
  branch already) and `u5f/commit-payload` (unfinished, see below).

### How to work

Opus 5 subagents implement one file-scoped unit each from a written brief. A fresh-context Opus 5
subagent reviews the frozen scope by invoking the repository's `code-review` skill and then
reproducing or disproving every finding before reporting; it reports only what survives, with a
`VERDICT b/m/n` line. Repairs go back to the same implementer. The orchestrator reruns the gates
and commits at 0 blocking / 0 major. The Codex roles named in the older sections are void.

Two hard rules learned the expensive way:

- A reviewer must run the `code-review` skill in **its own scratch worktree**, never in the shared
  one. The skill runs `git stash` over the whole working tree; in a shared worktree that makes other
  agents' files vanish, and one `stash pop` aborted.
- Never pipe a long cargo run into `head`. `head` exits, cargo dies on SIGPIPE, and a partial
  profile set produced a 29,523-line phantom coverage report that cost a 24 GB target and half an
  hour. Write to a log file and grep the file.

Watch the disk on both sides. One `cargo mutants` run grew a 31 GB temporary copy, and an
instrumented workspace coverage build filled the VM. Give mutation runs an isolated `TMPDIR` and
delete it afterwards.

### What the twenty-two commits did

**They made the branch buildable off Linux for the first time.** The CLI library test target had not
compiled on macOS since C0b (`465ff075`), thirteen host tests failed there (`e7888c08`), and the
workspace Clippy gate could not pass because `skit-benchmarks` lints on a non-Linux host
(`47cbcdf5`). Running the suite on a Mac found three real defects that Linux-only work had hidden,
and a Windows cross-check then found a fourth that the macOS fix itself created (`35ba1c27`).

**They finished the migration.** The model crate `skit-tui-walker-model` owns the parity probes, the
random operation model and the artifact writers (`cd2380a8`); the fifteen legacy host rules moved to
real stores (`af195e85`); the random walk was rebuilt on the real host with its CI retarget
(`c022b825`, repaired by `155b53e0`); and the FakeHost walker was deleted: 16,088 lines and 171
tests (`2bd7834c`). Every ledger row names a live owner.

**They fixed three product defects.** A settings save made while a prompt's insertion was off wiped
the stored parameter schema, root-caused against version 0.4 (`7ccdd283`); the launch menu built its
run summary without the catalog while the rerun path localized the same sentence (same commit); and
a picker test asserted a host-dependent path length, then a display-width one (`7590a098`,
`0b36cd5a`).

**They advanced the corpus.** See the next section, which is the critical path.

### The critical path: the four-profile review corpus

`cli::tui_walker_corpus_tests::generates_and_reinstalls_the_stable_review_corpus` is the ten-minute
owner that proves the mission. Run it on Linux, on a **clean tree**, because `source_identity` hashes
HEAD plus the working diff and a mid-run commit poisons it:

```bash
nohup cargo test --locked -p skit-cli-rs --lib -j 6 \
  cli::tui_walker_corpus_tests::generates_and_reinstalls_the_stable_review_corpus \
  -- --exact --ignored --nocapture > ~/coding/.tmp/corpus.log 2>&1 &
```

It has surfaced six leaks. Five are fixed and it now runs 434 seconds before refusing:

1. The draft-quarantine allocator boundary was never projected; three of the four C0a boundaries
   were. Fixed in `86712ba1`.
2. Any nonempty right-edge text was classified as a clip, so a single letter refused a session
   object. Fixed in `86712ba1`.
3. A `Ctrl+U` moved the review name into the input's cut buffer, which no projection owned. Fixed
   in `4865eba7`.
4. A sentence period was read as part of an allocator name, so a status sentence failed the grammar
   in English and pseudo but not in Chinese. Fixed in `4865eba7`.
5. A rendered frame row showed a path rooted in the sandbox. Granted at its exact painted row in
   `cc181c43`.
6. **Open.** `profiles/en-80x24/chunks/en-80x24-0006.json: leak oracle found a disallowed draft
   token (Clipped(Left)) at /3/cause/emitted/add/0/commit/entry/payload/stored_name`.

### The decision the user made, and what it means for defect 6

A read-only design audit asked whether this chain converges. Its answer, and the user's decision on
2026-09-07, change the next unit's shape. **Read the "Design audit of the leak oracle" section below
before writing any more corpus code.**

The audit found the frame rule inverted: it forbids exactly the values that are already
deterministic (in stable mode the profile root is a compile-time literal and the allocator counter
is `skit-new-000000`) and permits the only values that are machine-specific (`checkout_root()`,
`$HOME`, the real process cwd, and a `temp_dir()` that is not the declared namespace root are in no
forbidden set at all). It also found the grant cannot converge: there is no display-side path
shortening anywhere in the TUI, about a dozen screens paint an absolute path, and `footer.rs:634`
renders any host status message on every screen.

**The user chose to invert the rule.** So the work is:

- **Change 1, the inversion.** Forbid ambient text in every channel including frames. Permit
  sandbox-internal deterministic text in frames. This deletes the renderer observed-form and clip
  vocabulary: `RendererObservedForm`, `RendererObservedFragment`, `renderer_root_lines`,
  `rendered_facts_in_lines`, `rendered_root_lines`, `retain_live_renderer_facts`,
  `validate_renderer_observation`, `validate_renderer_fragment`, `renderer_fragment_spans`,
  `exact_or_edge_clipped_token_spans`, `ClipEdge`, `DraftScanOutcome::Clipped`,
  `left_clipped_fragment`, `right_clipped_observed_at` — most of `leak_oracle.rs` and a block of
  `tui_walker_bundle.rs`, including what `cc181c43` just added. It also closes the ambient hole,
  which is a real gap and not only a relaxation. Write the reasoning into `G4-DESIGN.md` D5: this
  narrows the mission's "no sandbox root survives into any recorded artifact" to non-frame
  artifacts, and that must be recorded, not applied silently.
- **Change 2, three improvements the user also approved, no decision needed.** Scan at record time
  inside `SchemaThreeSink::record_checkpoint` (`tui_real_walker.rs:2637`), which is sound because
  registration precedes projection, so a leak names its operation index and JSON pointer in seconds
  instead of after ten minutes. Move the producer's boundary predicate `accept_host_path_token`
  (`tui_real_host.rs:7923`) into walker-support beside the consumer's `is_boundary_after`, so the
  two grammars cannot diverge again, which is exactly what defect 4 was. Add two proptests: the
  round trip (if the projection replaced every raw token, the scan is empty) and producer/consumer
  completeness (the second becomes unnecessary once Change 1 lands; keep the first).

Defect 6 sits inside that decision. `u5f/commit-payload` (below) read it as the oracle mis-reading
an ordinary stored name as a truncated allocator token, which is the same family as defects 2 and 4
and which Change 1 largely dissolves. Do Change 1 first and re-run the corpus before writing a
targeted fix for defect 6.

### Unfinished work you can pick up

- **`u5f/commit-payload`**, tip `297844c6`, in the Mac repository and in `~/coding/skit-rev`. It does
  **not compile**: `no field 'run' on type RecordedRealCorpus`. It adds `left_fragment_minimum` to
  `leak_oracle.rs` (the mirror of the right-edge minimum `86712ba1` added),
  `committed_prompt_corpus_operations` for a short vector that reaches the Add commit, and three
  contracts. Judge it against Change 1 before finishing it; the inversion may delete the code it
  extends.
- **The Windows random walk gate is undone.** `real_random_walk`
  (`tui_real_random_walk.rs:696`) is deliberately not ignored so the coverage gate sees its lines,
  and `ci.yml`'s `test (windows-latest)` leg therefore spawns four real hosts and drives 24 random
  operations on a platform where the real host has never run. `35ba1c27` fixed the path-spelling
  defect underneath it but not this exposure. Decide the gate and say what has to happen before
  Windows is re-enabled.

### Never measured, do not claim it

A session limit killed ten agents mid-run. None of the following was measured after the rescue, and
the pre-rescue numbers in the older sections are stale:

- **The coverage gate.** Two attempts failed: one to a SIGPIPE from `head`, one when the VM filled.
  The pre-rescue baseline was exactly 80 uncovered lines in nine files; parts of it may already be
  closed by the recovered work. Measure it on **Linux**, which is the gate's platform, in a clone
  with room for an instrumented target, and never pipe it.
- **The workspace `cargo mutants` gate.** `.cargo/mutants.toml` sets `test_workspace = true`, so
  every mutant runs the whole suite, and `real_random_walk` is a fixed real-host tax inside it.
  Time the workspace suite against `mutation.yml`'s `--timeout 300` before claiming this gate.
- **i18n completeness**, **benchmarks**, **packaging** and the **`skills/skit/SKILL.md`
  synchronization** that AGENTS.md requires. This branch changed CLI behavior, so SKILL.md may
  document something that is now wrong.
- **Two of the four commits that landed without a review**: `86712ba1` and `4865eba7`. `2bd7834c`
  and `7590a098` were reviewed (the picker unit came back 0/0/3 and its findings are applied).

### Gates that did pass

Measured on 2026-09-06 and 2026-09-07 at the commits named:

| Gate | Result |
| --- | --- |
| `cargo deny --locked check` | advisories, bans, licenses, sources ok |
| `cargo audit --deny warnings` | clean over 451 crates |
| `zizmor` on every workflow | no findings, online and offline, version-matched with CI |
| `cargo clippy -p skit-benchmarks --all-targets --all-features -- -D warnings` | clean on macOS and Linux |
| `cargo xwin check` for `x86_64-pc-windows-msvc`, workspace, all targets | 0 errors |
| `scripts/test_tooling_contracts.sh`, `scripts/check_english.sh` | exit 0 on Linux |
| `cargo test -p skit-tui --lib` | 348 passed on macOS and Linux |
| `cargo test -p skit-tui-walker-model` | 143 lib plus 11 integration |
| `cargo test -p skit-cli-rs --lib cli::tui_real_host` | 145 on macOS, 208 on Linux |

The workspace Clippy gate still fails on macOS, but only on about 48 dead-code and unused-import
diagnostics in `tui_real_walker.rs` and `tui_walker_bundle.rs` whose only users are linux and
windows tests. `e7888c08` cleared the same class in the two sandbox files; the same treatment
applies here, and it is blocked on nothing.

### The order to finish the pull request

1. Change 1, the frame-rule inversion, with its `G4-DESIGN.md` entry. Then re-run the corpus.
2. Change 2, the three improvements. Record-time scanning makes every later leak cheap to find.
3. Work the remaining leaks with the ranked list in the design-audit section until the corpus
   completes twice with identical bytes.
4. The macOS dead-code sweep in the two walker files, so the workspace Clippy gate passes on every
   host.
5. Decide the Windows exposure, then get one real Windows run. The cheapest way is a throwaway
   branch or a temporary `workflow_dispatch` leg that runs only `cargo test -p skit-cli-rs --lib`
   on `windows-latest`, before this branch reaches a pull request where about 310 never-run tests
   would arrive together and a 60-minute timeout would cut them short.
6. The unmeasured gates above, coverage and mutants first.
7. Review `86712ba1` and `4865eba7`.
8. The corpus review comparison. The user replaced the Codex reviewers: the Terra role is Opus 5 and
   the Luna role is Sonnet 5, same prompt, byte-identical corpus copies, independent, compared on
   confirmed findings, misses, false positives, completeness and working style. Do not add it to CI.
9. The history rewrite the user requires, then the PR base change. Nothing is pushed or retargeted
   without the user's explicit instruction. The intended base is `fix/tui-mouse`; verify the remote
   state first.

### Detail behind the summary above

Open review findings that no unit has repaired yet:

- U6's replay path, `crates/skit-cli/src/cli/tui_real_random_walk.rs:526-542`. `SKIT_WALKER_REPRO`
  returns before the drift comparison when the recorded bug still reproduces, so it validates
  nothing; and when the bug is fixed it compares a full resolved vector against the bundle's
  truncated prefix, so it always reports drift. The truncation comes from `walk_case` (`:322`),
  which records the operation value before `run_operation` aborts.
- U6's failure bundle, `:617-652`. `finish_failure` drops the profile the walk latched and rescans
  the whole profile matrix, taking the first profile whose replay of the minimal vector fails.
  The bundle can then name a different profile and error than the panic message, and the rescan
  adds up to fifteen real-host spawns after every failure, which the nightly job's 65-minute
  timeout can cut short before the artifact is written.
- Seven minors on the same unit, listed in the review: a diagnostic-frame failure reported as a
  host-cleanup failure; `CorpusRefusalSink.boundaries` written and never read; `CorpusEvent::primary`
  constructing non-primary kinds; five unforced `pub(super)` widenings; an environment-dependent
  settings test; `repro_profile` accepting any locale tag; and a drift guard that a missing
  `resolved` array skips.

Windows, audited 2026-09-06 and never run:

The CI test matrix includes `windows-latest`, and no commit on this branch has ever run there.
A cross-check with `cargo xwin check --locked --target x86_64-pc-windows-msvc --workspace
--all-targets --all-features` (the target and its SDK cache are present on the development Mac)
reports 0 errors, so the macOS-class compile break does not repeat and every cfg union covers
Windows. Everything below compiles and has never executed.

- The fix that repaired macOS created a Windows defect. `resolved_random_root`
  (`tui_real_host.rs:2770-2781`) resolves the fixture root on Unix and leaves it alone on Windows,
  but the product canonicalizes on every platform (`cli.rs:11602` `resolve_add_source`, reached
  from the effect the walker host serves), so `source_record` holds a `\?\` verbatim spelling the
  fixture root never matches. `project_commit_review_name` (`:6711`) then refuses every Add commit,
  and `is_registered_path` (`:7394`) never normalizes the path, so the raw root reaches the
  artifact. The `windows-latest` `TEMP` is also an 8.3 short name that canonicalization expands.
- `real_random_walk` (`tui_real_random_walk.rs:696`) is deliberately not ignored, so the Windows CI
  leg spawns four real hosts and drives 24 random operations each, on a platform where the real
  host has never run.
- About 310 walker tests are ungated and would execute on Windows for the first time, including the
  Windows-only sandbox arms: `rename_native_noreplace` through the alpha dependency
  `fence-windows`, the handle-relative open and delete paths, junction and reparse handling, and
  file identity by volume, file index and creation time.
- A first Windows failure triggers the shrink and rescan storm recorded above, inside a 60-minute
  leg, on a filesystem several times slower than Linux. The leg would time out instead of
  reporting the real error.

The cheapest way to convert all of that into facts: push a throwaway branch, or add a temporary
`workflow_dispatch` leg, that runs only `cargo test --locked -p skit-cli-rs --lib` on
`windows-latest`. Do that before this branch reaches a pull request, where the failures would
arrive together and be cut short by the timeout.

Design audit of the leak oracle, 2026-09-06, read-only:

The audit answers whether the corpus leak chain converges. Its verdict: the JSON channels
converge and their remaining gaps are enumerable, but the frame oracle has one inversion that
cannot converge.

- **The inversion.** The frame rule forbids exactly the values that are already deterministic and
  permits the only values that are machine-specific. In stable mode the profile root is the
  compile-time literal `/tmp/skit-ui-walker-v1/sandboxes/<profile>` and the allocator counter is
  `skit-new-000000`, so forbidding them in a frame buys no determinism. Meanwhile no ambient path
  is forbidden anywhere: `checkout_root()`, the process `$HOME`, the real process cwd, and a
  `temp_dir()` that is not the declared namespace root all pass every channel in every mode today.
  That is a live hole, not only a relaxation.
- **The grant cannot converge.** There is no display-side path shortening anywhere in the TUI, so
  every path that reaches a frame reaches it verbatim. About a dozen screens paint one, and
  `footer.rs:634` renders any host status message on every screen, including `"Error: {}"` around
  any host error and templates that carry a path. The painted-row grant added in `cc181c43` is the
  first three of an open set, and the audit shows that grant is self-certifying: it is derived from
  the same rows it authorizes, so it can never fail on a real recording.
- **The recommended change, which needs the user's decision** because it narrows a stated mission
  requirement: forbid ambient text in every channel including frames, and permit sandbox-internal
  deterministic text in frames. It deletes the renderer observed-form and clip vocabulary, most of
  `leak_oracle.rs` and a block of `tui_walker_bundle.rs`. Its cost is reviewer tidiness: a frame
  would read the literal path the user sees instead of a sentinel. The JSON channels keep their
  sentinels. Two alternatives are rejected with reasons: renaming the sandbox root does not help
  because draft names are painted too, and making the product shorten paths for display cannot
  reach the footer's host-built strings and collides with product rule 3.
- **Independent of that decision**, two changes need no sign-off. Scan at record time inside
  `SchemaThreeSink::record_checkpoint`, which is sound because registration precedes projection, so
  a leak names its operation index and JSON pointer in seconds instead of after a ten-minute run.
  And move the producer's boundary predicate (`accept_host_path_token`) into walker-support beside
  the consumer's, so the two grammars cannot diverge again, which is what the sentence-period
  defect was.
- **The ranked list of JSON channels still unprojected**, in the order the canonical vector reaches
  them: the Add commit payload; `runner_editor` and `run` `LineInput.yank`; `PromptRunnerRow`
  descriptor in the unprojected `HostObservation.config`; runner and health status strings, whose
  session owners are not walked at all; `library.detail_signature`; `confirm_remove`; the settings
  owners; and the `form_state` and `prompt_runner` pass-throughs. Path-typed session fields are
  already covered: all ten `snapshot_path` emitters land in owners the projector walks.

Environment facts worth keeping:

- macOS runs everything except the stable sandbox, which refuses macOS by design. Every G4 test
  and the corpus owners are gated on linux and windows. Use the OrbStack machine `skit-linux`
  (`orb -m skit-linux bash -c '...'`, `TMPDIR=$HOME/coding/.tmp`) for them. Clones there:
  `~/coding/skit-rev` and `~/coding/skit-u6b`.
- `skit-benchmarks` used to stop the workspace Clippy gate on any host that is not Linux. That is
  fixed in `47cbcdf5`. What still stops the gate on macOS is about 48 dead-code and unused-import
  diagnostics in `tui_real_walker.rs` and `tui_walker_bundle.rs`, whose only users are linux and
  windows tests.
- `scripts/check_english.sh` needs bash 4 or newer; run it on Linux.
- One `cargo mutants` run filled 31 GB of scratch. Give it an isolated `TMPDIR`, watch the disk,
  and delete the copy afterwards.

## Successor checkpoint, 2026-09-06 (Claude, Fable 5.1, after the SSD rescue)

This records the rescue itself. "Start here" above supersedes its locations and decisions; keep
this for the account of what the failed disk cost and what survived.

What happened:

- The Proxmox host SSD failed silently under load. Freeze time 2026-09-05 02:39 America/Phoenix.
  Everything after that instant is lost; everything before it was rescued intact
  (`/Users/tim/LocalData/coding/2026/vm/skit-recovery/`, archive only).
- The 59 branch commits and the uncommitted work are committed as one durability blob,
  `128fe52a wip(recovery): freeze the uiwalker worktree as it stood before the SSD failure`,
  and the branch is pushed to `origin/integration/ui-walker-on-mouse` for the first time.
  `128fe52a` is a checkpoint, not a reviewed unit. Split it by workstream when committing
  reviewed units. No PR base was changed.
- The 82 `refs/codex-review/*` refs and the two stashes survive in the Mac repository
  (`refs/codex-review/*`, `refs/recovered-stash/*`) and in the Linux clone. They are not pushed.

Where the work lives now:

- Mac worktree `Projects/6-skit/skit-uiwalker` (this file). Mac-neutral work runs here.
- Linux: OrbStack machine `skit-linux`, native btrfs clone at `~/coding/skit-uiwalker`
  (from the Mac: `~/OrbStack/skit-linux/home/tim/coding/skit-uiwalker`, fetchable). Run commands
  with `orb -m skit-linux bash -c '...'` and set `TMPDIR=$HOME/coding/.tmp` there.
- The stable walker sandbox refuses macOS by design (`tui_real_host.rs`,
  `SandboxError::UnsupportedPlatform { platform: "macos" }`), and every G4 fast test and corpus
  owner is gated on linux/windows. G4 is Linux-only. A green Mac run proves nothing about G4.

Inventory at `128fe52a` (measured 2026-09-06):

| Workstream | Files | Mac | Linux |
| --- | --- | --- | --- |
| G4a/G4b/G4c corpus owner | `tui_real_walker.rs`, `tui_walker_bundle.rs`, `tui_walker_corpus_tests.rs` | compiled out | `tui_walker_bundle` 88/88 in 99 s, `tui_real_walker` 50/50, `tui_real_sandbox_fs` 30/30 |
| MIG-B host rules + `SettingsInputs` pin | `cli/tests/legacy_walker_rules.rs`, `skit-ui/src/settings.rs` | pin 1/1; rules blocked by the macOS compile break | not yet run |
| Package 1, prompt-parameter data-loss fix | `cli.rs`, `tests.rs`, `port_test_prompt_cli.rs`, `port_test_prompt_tui.rs` | CLI regression 1/1, TUI port 1/1; TUI oracle blocked by the compile break | not yet run |
| Model crate MIG-D part 1 + MIG-E part 1 | `crates/skit-tui-walker-model/src/*` | 135/135 | not yet run |
| Coverage debt (nine files) | see "Measured environment facts" | `agent_skill_store` 18/18; others blocked | not yet measured |
| Workspace clippy | | red in `skit-benchmarks` (pre-existing on the rewrite branch, untouched here) | green |
| Legacy `model_walker` | | 167 passed / 4 ignored | not yet run |

Two defects the Linux-only history hid:

1. `cargo test -p skit-cli-rs --lib` does not compile on macOS. Blame: committed C0b `256287d`.
   `tui_real_sandbox_fs.rs` writes `#[cfg(not(any(linux, windows)))] return Err(...)` and then
   uses a `file` binding that only the linux and windows arms define (about lines 1416, 1556,
   1557, 1599); two `tui_real_host.rs` tests call the linux/windows-gated
   `initialized_sandbox_paths` (about lines 11905, 11923). The CI test matrix includes
   `macos-latest`, so this branch has been red on macOS since 2026-09-01. Fix with cfg'd
   function pairs, which is the design's own rule. Linux counts must stay 88/50/30.
2. `cargo clippy --workspace --all-targets -- -D warnings` is red on macOS in `skit-benchmarks`.
   The branch does not touch that crate. It belongs to the rewrite branch, not here.

Decisions (orchestrator, 2026-09-06):

- Working mode replaces the Codex roles: Opus 5 subagents implement one file-scoped unit each
  from a written brief; fresh-context Fable subagents review a frozen scope read-only and report
  blocking/major/minor; repairs go to the same implementer; commit at 0/0/0. The orchestrator
  reruns the gates before accepting any report. Codex re-review and the Terra-versus-Luna corpus
  comparison wait for the user's decision.
- Unit order. U0 `fix(walker): compile the sandbox owner on macOS` (defect 1, first, small).
  U1 `fix(cli): keep prompt parameters while interpolation is off` (package 1; Mac; does not
  include `legacy_walker_rules.rs`). U2 `feat(walker): add parity probes and the random
  operation model` (MIG-D part 1 + MIG-E part 1; Mac; close the two recorded open points:
  `AdvertisedKey.binding` is unconsumed, and the env readers are proven only unset).
  U3 `test(cli): own host rules from the legacy walker` (MIG-B; Mac after U0; the lost minor list
  is replaced by a fresh review). U4 coverage debt, measured on Linux, the gate's platform.
  U5 G4b/G4c on Linux: apply the five recorded review findings and the hotspot, review, commit
  as `feat(walker): install the four-profile review corpus`, then run the canonical corpus owner
  once and record the wall time. U1 and U2 may run in parallel; U3 follows U1 (shared
  `tests.rs`); U5 runs in parallel with all of them.
- G4 hotspot. After the no-replace rename, prove that the installed tree equals the staged tree
  byte for byte (`read_tree` plus map equality) and do not run `read_review_corpus` and the leak
  scan a second time. Both are pure functions of bytes that were already validated. Add a
  contract that a post-rename byte change is caught by that equality.
- G4 suite time. The 890 s figure was the whole CLI lib suite under an eight-agent load. Measure
  `cargo test -p skit-cli-rs --lib` on `skit-linux` first. Adopt a memoized small corpus only if
  the suite exceeds five minutes, and then memoize only the in-memory `RecordedRealCorpus` with
  its sandbox `TempDir` dropped inside the initializer, so no static holds a directory.
- Coverage runs on Linux only, serialized, in a clean target. Do not run two `cargo llvm-cov`
  invocations at once; the shared instrumented target reports phantom misses.
- `scripts/check_english.sh` needs bash 4 or newer. On the Mac run it with
  `/opt/homebrew/bin/bash`, or run it on Linux.

## Successor checkpoint, 2026-09-04 (Claude, Fable 5.1)

Read this section before the older sections. It records what changed after the previous agent
(Codex) stopped, and it corrects the "Read this first: current checkpoint" claim that the tree was
clean.

How the previous session ended:

- Codex stopped at 2026-09-03T05:41Z with a `usage_limit_exceeded` error. Its last user message
  (a request to tidy the related branches and state the merge order) was never answered.
- The Codex usage quota is exhausted until **2026-09-06 20:50 (local)**. Every Codex-backed role
  (sol/terra/luna reviewers, the advisor, and the final Terra-versus-Luna corpus comparison) is
  unavailable until then. Claude-based reviewers (fresh-context Opus and a full-context fork) cover
  design and code review until the quota returns; the Codex re-review of every commit made in the
  meantime is mandatory when it returns, using the immutable `refs/codex-review/g4*` refs.
- The commit signing agent is unreachable (`SSH_AUTH_SOCK` points at a missing socket) and the
  repo-local config sets `commit.gpgsign=false`; 19 of the 55 branch commits are unsigned. History
  will be rewritten before main, so this is recorded, not repaired.

Working tree state (verified with `git status` and `git diff`):

- HEAD `eb7adcd`, 55 commits over `origin/fix/tui-mouse`, never pushed.
- `crates/skit-cli/src/cli/tui_real_walker.rs` (+597) is Codex's G4a: `RecordedRealCorpus`,
  `StableCorpusTracePair`, `record_corpus_profile`, `prepare_corpus_host_with`,
  `combine_corpus_result_after_close`, `canonical_corpus_artifact_values`, and four fast contracts.
  It is frozen at `refs/codex-review/g4a-trace-precommit-20260902` (`a69e164`). Terra reviewed it
  at 0 blocking / 0 major / 1 minor; the minor (independent leak facts on the pair) was fixed
  afterwards and verified green, but not re-reviewed. Luna's review never returned.
- `crates/skit-cli/src/cli/tui_walker_bundle.rs` (+348) is Codex's unreviewed G4b start:
  `ReviewCorpusDocuments`, the `BundleWriter` split (`create_common`, `prepare_profile`,
  `write_profile_trace`, `write_root_documents`, `write_review_corpus`), one root-document test.
  `write_review_corpus` has no passing test yet. The tree compiles; the 50 real-walker tests list.
- Codex's two G4 read-only audits on 2026-09-03 decided: keep the four per-profile leak fact sets
  separate; write `operations.json`/`final-liveness.json` once from consistency-validated traces;
  derive aggregate sandbox metadata from the four host singletons; keep manifest order separate from
  the run's sorted order; publish with the existing no-replace rename primitive; keep fast contracts
  in the main files and put the single long 100x4x2 owner in a separate `#[ignore]`d test file.

Where the G4 work stands now:

- The G4 implementation design is `G4-DESIGN.md` beside this file (untracked, keep it). It moves the
  required-vocabulary and canonical-vector rules out of measurement and into a final-corpus policy so
  the aggregate transaction is testable with a small real four-profile corpus. Its advisor review
  is in progress; PHASE4-PLAN.md revision 31 will record the accepted version.
- Measured on this host under load: one stable recording about 10 s; the four-profile
  100-operation preflight (no artifact capture) 89 s; Codex measured the full capture at 495 s.
- Parallel workstreams started 2026-09-04: the pre-existing 80-line workspace coverage debt is
  being driven to zero in the nine listed files (owner-scoped, no walker files); a read-only
  FakeHost migration ledger is being written to prepare the legacy-owner deletion.

G4 implementation dispatch (2026-09-04, after two advisor reviews accepted `G4-DESIGN.md`
revision 31):

- walker-support D2 contract: owner-scoped to `crates/skit-tui-walker-support/src/{bundle.rs,
  bundle_tests.rs, aggregate_contract_tests.rs, lib.rs}`.
- D6a `PinnedDirectory::rename_child_directory_noreplace`: owner-scoped to
  `crates/skit-cli/src/cli/tui_real_sandbox_fs.rs`.
- G4b CLI owner (D1, D3–D7): owner-scoped to `tui_real_walker.rs`, `tui_walker_bundle.rs`, the new
  `tui_walker_corpus_tests.rs`, and its one-line module declaration in `cli.rs`.
- Coverage debt: owner-scoped to the nine debt files listed in "Measured environment facts".
- FakeHost migration ledger: read-only; output lands in the scratchpad first, then `docs/design/`.

Commit plan: G4a `feat(walker): define review corpus regeneration contract` (support only), then
G4b `feat(walker): install the four-profile review corpus` (D6a plus the CLI owner). Each unit is
frozen as `refs/codex-review/g4*` and reviewed by a fresh-context reviewer at 0/0/0 before commit;
a Codex review of both follows when the quota returns.

G4a status (2026-09-04): the walker-support D2 contract is implemented (169 lib tests, `bundle.rs`
zero uncovered lines, package-scoped mutants 153 caught / 0 missed / 5 unviable, clippy/doc/fmt
green) and frozen at `refs/codex-review/g4a-support-precommit-20260904` (commit `897aea7`, tree
`9d5fd1a`, exactly the three support files over `eb7adcd`). A fresh-context review is in progress;
commit it as `feat(walker): define review corpus regeneration contract` when it reaches 0/0/0.

G4a is committed as `ff67bd8 feat(walker): define review corpus regeneration contract` (56 commits
over `origin/fix/tui-mouse`), byte-identical to the reviewed final ref below; the fresh-context
re-review returned 0/0/1 (the design's D2 block now lists `validate_review_report`).

G4a final ref after the review's four minors: `refs/codex-review/g4a-support-final-20260904`
(commit `1082117`, tree `edf4ef4`; four support files, now including `lib.rs` where
`validate_completed_review_claim` calls the new `bundle::validate_review_report`). 171 lib tests,
mutants 154 caught / 0 missed / 5 unviable in `bundle.rs`, claim function 3/3 caught. The
`compare_regenerated_corpus` message is "review corpus is missing a file: {path}".

D6a is committed as `3c65646 feat(walker): rename a private child directory without replacement`
(57 commits over `origin/fix/tui-mouse`): `PinnedDirectory::rename_child_directory_noreplace`,
`SandboxFsError::destination_already_exists` (kind-blind by design), and
`PinnedDirectory::open_directory_shared` (a child directory of any mode, no-follow, kind check) for
the installer's compare path; 30 fs tests; reviewed at 0/1/4 then 0/0/0 on
`refs/codex-review/g4-d6a-final-20260904`. The installer (G4b) must use
`destination_already_exists` then `open_directory_shared` for its three-way branch.

G4b (the CLI corpus owner: D1 split, `RecordedRealCorpus`, `record_real_review_corpus_in`,
`ProfileReadback`/`validate_profile_tree`, byte-only `read_review_corpus`, the shared leak scanner
and `validate_review_corpus_leaks`, `install_review_corpus`, `generate_and_install_review_corpus`,
`validate_completed_review_corpus`, `validate_final_review_corpus`, six real fast tests, and the two
ignored owners in `tui_walker_corpus_tests.rs`) is implemented: CLI lib 619 passed / 6 ignored,
clippy/fmt/English clean, `tui_walker_bundle.rs` zero misses; frozen at
`refs/codex-review/g4b-precommit-20260904` (commit `0deef7f`) with only its two-line `cli.rs` hunk;
reviewed at 0 blocking / 2 major / 5 minor (reader refusal tests must pin messages and isolate one
rule each; the cross-profile leak test must inject a per-profile draft name; the non-collision
publish error needs a filesystem-boundary seam and a contract; the replay pair must be compared
before the replay installs; residue assertions in the FIFO/symlink test); the repairs are in
progress with the same implementer. The reviewer also named the hotspot: the success arm re-reads
and re-validates the whole tree after the rename although byte equality with the staged tree already
holds; dropping that re-read roughly halves an install. Known costs to resolve before delivery: the CLI lib suite grew from
about 360 s to about 890 s (each fast test records a four-profile corpus, 55–70 s, and installs
about 20 times at 40–50 s each); a measurement of the hotspots is in progress, and a
`OnceLock<RecordedRealCorpus>` whose initializer drops its own TempDir is the first candidate.
`tui_real_walker.rs` has 14 pre-existing uncovered lines (`record_real_smoke_stable_pair` reachable
only through the system namespace, an `EngineCause::Host` helper arm, and `panic!`/`unreachable!`
guards) that the delivery gate must drive to zero.

MIG-B (15 host rules in `crates/skit-cli/src/cli/tests/legacy_walker_rules.rs` plus the 27-field
`SettingsInputs` pin in skit-ui) is frozen at `refs/codex-review/mig-b-precommit-20260904`; review
in progress. It surfaced a candidate data-loss defect: a settings save made while prompt
interpolation is off drops the stored declarations (`cli.rs` ~11095 saves the effective list); a
root-cause investigation against version 0.4 is in progress. Its item-28 finding: production maps
`Effect::None`/`Quit`/`PreferencesEffect::None` to `ClearStatus`; the ledger's "refuse at the host"
note was FakeHost strictness, not a product rule.

Confirmed product defect (2026-09-04, root-caused against version 0.4; not yet fixed): a settings
save made while a prompt entry's interpolation is off wipes the stored parameter schema
(`params` and `[[parameters]]`, including help text, defaults, choices, secret flags). Two write paths
seed their declaration list from the interpolation-gated effective plan (`skit-form/src/lib.rs:415`
returns an empty plan): TUI `tui_submit_settings_at` (`cli.rs:11099`, writes at `:11150` and
`:11168`) and `skit params` (`cli.rs:5951`, writes at `:6064` and `:6086`, guarded only when the
command carries an interpolation flag). Reproduction: `skit add p.prompt.md --prompt -n p`,
`skit params p --help-text TOPIC=x`, `skit params p --no-interpolate`, `skit params p --workdir store`
loses both; `--interpolate` does not restore them. Version 0.4 seeded from stored metadata
(`src/skit/tui_settings.py:344`) and skipped the parameter write while off (`:952-954`, `:1090-1095`);
its CLI ops returned before touching parameters (`src/skit/cli.py:4229-4246`). The ported oracle
`port_test_prompt_tui.rs:1032` dropped the two store assertions of
`tests/test_prompt_tui.py:926`. Fix (v0.4-faithful): seed from stored metadata by planning against a
clone of the stored settings with `interpolate: true`, and skip both parameter writes when the
submitted `interpolate` is false for a prompt; then flip `tests.rs:11321`, rewrite the tail of
`legacy_walker_rules.rs:597`, restore the oracle's store assertions through
`tui_submit_settings_at`, and add a `skit params --workdir/--runner` regression test. Waits for the
coverage-debt agent to release `cli.rs`. Update: the MIG-B implementer now owns the fix as its
"package 1" (`fix(cli): keep prompt parameters while interpolation is off`, TDD through
`tui_submit_settings_at` and the `skit params` binary, edits confined to the two save functions),
followed by "package 2", the MIG-B review repairs (1 blocking: the item-21 test pinned the defect;
3 major: item 27 asserts the parsed argv, item 29 asserts whole-tree rollback, item 24 asserts the
preset value; 6 minor). Each package is frozen and reviewed separately before commit.

Measured 2026-09-04: the coverage run records no `SF` entry for `*_tests.rs`, `tests.rs`, or
`tests/` files; `cli/tests.rs`, `bundle_tests.rs`, and the model crate's `*_tests.rs` are outside
the gate by the repository's own measurement. G4's D7 therefore keeps two thin `#[ignore]` owners in
`tui_walker_corpus_tests.rs` over fully covered functions.

Disk (2026-09-04 19:00): the disk filled to 100% while eight agents built concurrently. Freed about
49 GB by removing only rebuildable artifacts: orphan test temp dirs older than three hours under
`/home/ubuntu/coding/skit/.tmp`, the Codex-era scratch target dirs `probe_target`/`ptarget`/
`probe-target`/`cov_target` inside `.tmp/cr3d1r2_*`, `cr5`, `impl3d1fix*` (their logs, probes,
lcov files, and `mutants` evidence were kept), incremental caches untouched for 12 hours, and
12-hour-old `deps` artifacts of skit's own crates and test binaries. Third-party artifacts, fresh
artifacts, `target/*.lcov` evidence, the `uiwalker-mouse-review-*` directories, and
`target/llvm-cov-target` were not touched. 51 GB free afterwards.

Operational rule learned today: concurrent `cargo llvm-cov` runs in the shared
`target/llvm-cov-target` delete each other's profile files and report phantom misses. Every
coverage run must hold `flock /home/ubuntu/coding/skit/.tmp/llvm-cov.lock`, and per-package
measurements must use an isolated `CARGO_TARGET_DIR=/home/ubuntu/coding/skit/.tmp/cov-<slice>`
removed afterwards, because the shared instrumented target accumulates stale objects from other
agents' builds and reports phantom misses even when serialized. The final workspace gate runs once
in the shared target after `cargo llvm-cov clean --workspace`. Disk is at 86% with 42 GB free.

Legacy-walker migration (started 2026-09-04 in parallel with G4):

- `docs/design/fakehost-migration-ledger.md` classifies all 171 legacy tests (124 with product
  meaning, 34 must-migrate, no production code exists only for the legacy owner).
- `MIGRATION-DESIGN.md` revision 2 is the design, accepted by two advisor reviews: a new dev-only
  mutation-visible crate `crates/skit-tui-walker-model` owns the state invariants, the parity
  probes, the random operation model and its tables, and the artifact/env readers; skit-cli keeps
  only the real-host wiring; the random walk is a non-ignored env-scaled test with deterministic
  defaults; CI retargets in MIG-E; deliberate losses are listed at the end of the design.
- MIG-A (new files under `crates/skit-tui/tests/`: `local_action_parity.rs`, `run_pointer_rules.rs`,
  `preferences_focus_rules.rs`) and MIG-B (`crates/skit-cli/src/cli/tests/legacy_walker_rules.rs`
  appended to `cli/tests.rs`, plus a `SettingsInputs` pin in skit-ui) are being implemented now.
- MIG-C is committed as `b769f8e feat(walker): add the model crate with state invariants`
  (58 commits over `origin/fix/tui-mouse`): `crates/skit-tui-walker-model` with `invariants`
  (56 tests, 167 mutants with 0 missed, complete coverage, cargo-deny ok); reviewed at 0/0/4 on
  `refs/codex-review/mig-c-precommit-20260904` with the two test-precision minors applied in the
  commit. The reviewer verified all six guard rewrites as equivalent. The skit-cli dev-dependency
  edge is in place but unused until MIG-D part 2 wires `check_state` into the real frontends.
- MIG-A is committed as `4dfe990 test(tui): own screen-local parity and pointer rules` (three files
  under `crates/skit-tui/tests/`, 11 tests, plus `docs/design/fakehost-migration-ledger.md`);
  reviewed at 0/3/6 then 0/0/2 on `refs/codex-review/mig-a-final-20260904` with the two minors
  applied in the commit. Its two whole-frame parity sweeps (legacy `driver.rs:3220`/`:3411`) return
  when the files move into the model crate's `tests/` and call the `parity` API (after MIG-D part 1).
- MIG-E part 1 is implemented in the model crate (`model.rs`, `model_tests.rs`, `artifacts.rs`,
  `artifacts_tests.rs`; 32 tests; 71 mutants with 0 missed; complete coverage; cargo-deny ok).
  Public API: `RESIZE_CASES`, `PASTE_CASES`, `random_walk_profiles()` (15), `RandomOperation` with
  nine families and exact serde (`"operation"` tag), `LiveInventory::new`, `resolve(operation, live)
  -> ResolvedInput` (including `ScreenFocus`/`ScreenHit` reached through a concatenated
  local-plus-screen ordinal space, `RawMouse`, and typed `NotApplicable` refusals with Quit refused),
  `operation_strategy()`, bundle writers, and typed env readers (`walk_cases`, `walk_steps`,
  `walk_profiles`, `record_success`, `liveness_every`, `repro_source`). Two points for its review:
  `AdvertisedKey.binding` is generated but not consumed (legacy dispatched the chosen binding; the
  per-checkpoint parity probe covers alternative bindings instead, or part 2 maps ordinal > 0 to the
  binding's key event), and the readers are proven only in their unset state because `set_var` is
  unsafe in edition 2024 (a child-process test with the variables set can close that in part 2).
- MIG-D part 1 (`parity` module) is still being implemented; it shares `lib.rs` and `Cargo.toml`
  with MIG-E part 1, so both freeze and review together as one unit.
- MIG-D part 2 (wiring `check_state` and `check_public_hit_parity` into the real frontends) and
  MIG-E part 2 (`tui_real_random_walk.rs`, `RawMouse`, CI retarget) edit `tui_real_walker.rs` and
  wait for G4's commit.

Branch topology, verified 2026-09-04 with `git rev-list --count`:

- `origin/main` `206f9ef` (2026-07-29).
- `origin/rewrite/rust-ratatui-complete-20260808-codex` `5af68f8`: 1024 commits over main. PR #45.
- `origin/fix/tui-mouse` `0edf38c`: 1 commit over rewrite. PR #49 (base rewrite).
- `integration/ui-walker-on-mouse` `eb7adcd`: 55 commits over `fix/tui-mouse`, local only.
- `origin/feat/issue-46-ui-walker` `9fe8512`: 3 commits over rewrite; PR #48 (base rewrite). It is
  the pre-integration walker head and is superseded by the integration branch.
- Other open PRs into rewrite: #44 (Python test port). Into main: #34, #35 chain, dependabot #40 #41.

Merge order to propose to the user (not executed): finish this branch; retarget PR #48 to head
`integration/ui-walker-on-mouse`, base `fix/tui-mouse`; merge #48 into `fix/tui-mouse`; merge #49
into rewrite; settle #44; then rewrite into main through PR #45 after the history rewrite the user
requires. Nothing is pushed or retargeted without the user's explicit instruction.

## Mission

Finish PR #48's deterministic TUI walker on top of `fix/tui-mouse`. The walker must drive the real reducer, real production `TuiHost`, and real temporary stores; produce a deterministic, content-addressed, agent-readable UI corpus; remove the high-level `FakeHost` second source of truth; then use the same real-host corpus to compare Terra high and Luna max as TUI reviewers.

The job is not complete when a smoke test passes. It is complete only when the real disk corpus and 100-operation/four-profile owner exist, FakeHost is deleted after contract migration, both model reviews are reproduced and compared, confirmed bugs are fixed, and all AGENTS.md gates pass.

## Read this first: current checkpoint

The latest committed units are the G3 review follow-ups and Phase4 H:

```text
5a81602 fix(tui): ignore empty Add paste
e24e458 fix(tui): keep compact runner navigation visible
eb7adcd fix(walker): forecast resized cast canvas
```

The canonical corpus is now exactly 100 operations and all four required profiles complete it with
no NotApplicable operation, no Quit, and truthful final liveness. Measured coverage remains derived
only from typed `TransitionCause::Host.request` rows; no cause counter or FakeHost path participates.
The full four-profile diagnostic reaches exactly 23 nonstatic outer and 14 nonstatic nested effect
names in every profile. H now forecasts a grow-resize cast from the same operation vector while
retaining completed timeline rows as the final truth. Do not split or redo G1 through G3 or H.
Current Git state:

- branch `integration/ui-walker-on-mouse`, 55 commits over `origin/fix/tui-mouse` at the 2026-09-02
  checkpoint (56 after `ff67bd8` on 2026-09-04);
- no tracked or staged changes after the commit;
- only `.playwright-mcp/`, `HANDOFF.md`, `PHASE3D-PLAN.md`, and `PHASE4-PLAN.md` are untracked;
- `.playwright-mcp/` contains Kiro-era console/page logs. It is not part of C1 and was not reviewed
  or committed. Preserve it unless the user explicitly authorizes deletion;
- no push and no PR-base change;
- final HEAD `eb7adcd5eaa003d3fdc1e86acc31a4d2dd90ee11`, tree
  `83b49ec33c2fdb796e858e6538b6e2dbeda604a0`;
- G3 prerequisite projection commit `716d4c8`, tree
  `0ef8de7e1769096e1d9c233929fe00aa7269c9c6`;
- G2c chain: `8c25636`, `770bcff`, `29c1f2d`, `aa99dcc`, `095ef1c`; G3a remains
  `151fa7d`; G1 `81d32e7`, G2a `b975985`, G2b `a34d0ff`, picker tree `0b4dbf4`.

Final root evidence through G3:

- G1 final support run: `154 unit + 21 integration`; LCOV
  `target/g1-terra-coverage-final.lcov`, SHA-256
  `b714cd9ab2c86995513da59e2ad818416aa24ada37e9028ae5285499e082a04c`, has zero G1
  executable misses. Sol and Terra final reviews are `0/0/0`.
- G2a final host run: `8/8` focused and `188/188` complete real-host tests; LCOV
  `target/g2a-review-closure-final2.lcov`, SHA-256
  `823926b8554fcf5ad2a846a0f7568f877d69cf033b1773331b73704fca801b3c`, has zero G2a
  misses. Both independent final reviews are `0/0/0`.
- G2b final walker run: `7/7` focused and `33/33` complete real-walker tests; LCOV
  `target/g2b-owned-green.lcov`, SHA-256
  `bd3e41790673b0644c093253288b7e20934ef5e02583ca31fe215fd73cd54c7f`, has zero misses on
  added walker lines. Both independent final reviews are `0/0/0`.
- Root reran the combined G2 focused tests (`8 + 7`), complete real walker (`33`), CLI Clippy,
  rustfmt, and diff checks before committing.
- G3a bundle contracts ran `79/79`; `target/g3a-timeline-coverage-final.lcov`, SHA-256
  `123be1f0b5640922c970cf7671fda97783f7d7833156033152fec79a9f3ea01f`, has zero G3a misses.
- G2c host picker ran `8/8`; `target/g2c-host.lcov` has zero owned misses. G2c inventory ran
  `336/1` in root's final lib LCOV; `target/g2c-screen-root-final.lcov`, SHA-256
  `77de60c1cb56882522ea9909bd5834a051187ea7cb3742323dabaf5aeed145e5`, has zero misses in
  the repaired inventory ranges. All three slices closed independent final review at `0/0/0`.

- G3's canonical operation JSON is 7,839 bytes with SHA-256
  `824a42732e08119f3c57286f636d3f12936e3a6b0aa99947c8da17db92d0e922`.
- `refs/codex-review/g3-final-full-diagnostic-20260902` is commit
  `3d042c2f9787dc4ea161f5418423eb8f77a2f971`, tree
  `cd64820cc9d8edd332c6a5e1ba4b3b195cc45d12`. Its four fresh stable profiles each completed
  100/100 operations, exact 23/14 measured coverage, required-order merge, and final liveness in
  494.76 seconds. The temporary full diagnostic exists in that ref; G4 will make this read-back
  permanent.
- Final G3 source review ref `refs/codex-review/g3-final-review3-20260902` is commit
  `e34ace30461b530fb7d4512e3cc12c1809d7391f`, tree
  `9d058bd16612ed29bf6b308cdcf028f32ceaff4c`, exactly equal to final HEAD's tree.
- Final tests: walker 43/43; C2 host projection 31/31; TUI lib 344 passed/1 ignored; CLI lib
  562 passed/4 ignored. Workspace all-target/all-feature Clippy, rustfmt, and diff-check are green.
- Final LCOV artifacts: walker `target/g3-walker-final3.lcov`, SHA-256
  `f80c367a1519277da39f764a69eeb61d508f656e8c486891b35b396990a76ba1`; host
  `target/g3-host-final.lcov`, SHA-256
  `a178b311dd60f0e0d4574fd503f3d908de416ad9fd9845d2c5f8d7f8a2c7b2ea`; TUI
  `target/g3-tui-final6.lcov`, SHA-256
  `67c6a415aa1a4586d2435610209b5072fa08872f129dc796d421b5b17cddc876`. Diff-aware new
  executable misses are zero in all six changed files.
- Terra xhigh's final G3 review is 0/0/0. Luna max found one real missed boundary: empty Add Paste
  emitted an unchanged Set action and cleared projection provenance. Commit `5a81602` fixes it, and
  both models cleared the follow-up at 0/0/0. Both models withdrew the named-runner keyboard concern
  after checking the Tab-only ScreenFocus contract and malformed unnamed row counterexample.
- The full four-profile rerun caught a later local footer-grouping regression that the focused UI
  test missed: pseudo-120x12 could not reach Next field. Commit `e24e458` restores the
  Save/Cancel-first single responsive group. Canonical preflight, TUI lib, and both model reviews are
  green.
- H final review ref `refs/codex-review/h-final-review2-20260902` is commit
  `81b496be1d94e0fafccdd49dc856f4cf7a20f185`, tree
  `83b49ec33c2fdb796e858e6538b6e2dbeda604a0`, exactly equal to `eb7adcd`'s tree. Support is
  160/160, walker 46/46, legacy model 167 passed/4 ignored, workspace Clippy and support Rustdoc are
  green. H LCOV: `target/h-walker2.lcov` SHA-256
  `2dccc46e236d59538bd3b30bae3ae4aaa154eac2440bba348515eceeb803f3e4` and
  `target/h-support3.lcov` SHA-256
  `2ad3ef1aafa04854ef0b08a80c4e98cd874861cc58cd9b87ad95e8be33172cde`, both zero owned
  misses. Terra and Luna final H reviews are 0/0/0.

- support full LLVM coverage ran `141 passed`, plus `21` leak-oracle integration tests;
  `target/f-support3.lcov`, SHA-256
  `56e0d97b57ea7ed7a3803f93984ffc502b22a9748ea7e5ac34b35553ce245bac`, has zero misses in
  F's new `bundle.rs` lines;
- CLI-lib LLVM coverage ran `518 passed / 4 ignored`; `target/f-cli.lcov`, SHA-256
  `7e1892aa0b75348864c6c1a7d59d0c2382004c22556b8c144dc44cf971364018`, and the F classifier
  range has no `DA:0` lines. A final equivalent fixture cleanup occurred afterward and was rerun by
  focused tests and workspace Clippy;
- support contracts pin exact canonical bytes, every missing/unknown/invalid shape, count-state and
  exclusion invariants, and derived Clone/Debug paths. CLI contracts cover all 16 outer Effects,
  8 HostRequests, 6 FormPurposes, 10 AddEffects, and 7 PreferencesEffects;
- workspace Clippy with warnings denied, rustfmt, and diff checks pass;
- the first immutable reviews found Sol/max `1 blocking / 1 major / 0 minor` and Terra/high
  `0/1/0`: one redundant fixture binding and missing decoder/derived-code coverage. The final ref
  adds present-but-non-object Effects, noninteger schema, Clone/Debug execution, and the direct
  fixture return. Sol/max and Terra/high both cleared it at `0/0/0`.

C0b remains closed in parent commit `256287d`; its filesystem evidence and immutable refs are kept
below for provenance.

### Immutable review baseline

The user asked the current diff to remain independently reviewable after a successor changes the
worktree. Two local Git plumbing refs record it without changing HEAD, the branch, the real index, or
the worktree:

```text
refs/codex-review/c0b-before-successor-20260831
refs/codex-review/c0b-before-successor-20260831-code
refs/codex-review/c0b-final-pre-review-20260901
refs/codex-review/c0b-final-delta-review-20260901
refs/codex-review/c0b-final-second-review-20260901
refs/codex-review/c1-pre-review-20260902
refs/codex-review/c1-post-clippy-review-20260902
refs/codex-review/c2-pre-review-20260902
refs/codex-review/c2-post-review-20260902
refs/codex-review/c3-pre-review-20260902
refs/codex-review/c3-post-review-20260902
refs/codex-review/c3-post-review2-20260902
refs/codex-review/c3-post-review3-20260902
refs/codex-review/d-pre-review-20260902
refs/codex-review/d-post-review-20260902
refs/codex-review/d-post-review2-20260902
refs/codex-review/e-final-review2-20260902
refs/codex-review/f-pre-review-20260902
refs/codex-review/f-post-implementation-review1-20260902
refs/codex-review/f-final-review1-20260902
refs/codex-review/g-pre-review-20260902
refs/codex-review/g1-post-implementation-review1-20260902
refs/codex-review/g1-final-review1-20260902
refs/codex-review/g1-final-review2-20260902
refs/codex-review/g2a-post-implementation-review1-20260902
refs/codex-review/g2a-final-review1-20260902
refs/codex-review/g2b-post-implementation-review1-20260902
refs/codex-review/g2b-final-review1-20260902
refs/codex-review/g2c-host-final-review2-20260902
refs/codex-review/g2c-screen-final-review4-20260902
refs/codex-review/g2cc-walker-final-review1-20260902
refs/codex-review/picked-source-action-post-review1-20260902
refs/codex-review/picked-source-host-final-review1-20260902
refs/codex-review/event-chain-post-review1-20260902
refs/codex-review/utf8-picker-target-final-review1-20260902
refs/codex-review/g3-pre-review-20260902
refs/codex-review/g3-full-diagnostic-20260902
refs/codex-review/g3-final-full-diagnostic-20260902
refs/codex-review/g3-final-precommit-20260902
refs/codex-review/g3-final-review2-20260902
refs/codex-review/g3-final-review3-20260902
refs/codex-review/h-final-precommit-20260902
refs/codex-review/h-final-review2-20260902
```

The C1 pre-review ref is commit `1d9625d66b8ec890c50fa898388c3bf7d44296c9`, tree
`92f6b74ae1abe720b7d7ce9724e6f8b8c486ca71`. The final C1 ref is commit
`ba95e917c39c546d7b05570b78363dccc639ffbc`, tree
`2716627756894e3a7f2b86ef476d3fa82ebdfd20`. Their complete delta is two redundant borrow removals
in `tui_real_host.rs`; both independent reviewers cleared that delta. Commit `978aff1` has the final
tree exactly.

The C2 pre-review ref is commit `3893efde108fe8a2143a0935356dff44a04b7f28`, tree
`a9f21a8642f364b80d9a133d090d24ba1a9b6756`. The final C2 ref is commit
`a2208fcc2b659e1d8dbf023d0c0c5153372506ce`, tree
`68a8a929bfe7c27378aab4fc10007c1b73680bb6`. Their delta contains every reviewed repair. Commit
`bd8eb08` has the final tree exactly.

The C3 pre-review ref is commit `feebc6e96d80f71f7e395c6b8c18a586e343c9d2`, tree
`d41954232a1b05a3051e69d468aa6ec9e3857941`. The three post-review refs retain the review repair
sequence rather than moving one ref. The final ref is commit
`291fd5b6eed0e66d629f44e3e7c0e74ccce93f08`, tree
`0bc22f5cef23d7ff627c45d251663badd585f541`; commit `4c62b73` has that final tree exactly.
The intermediate refs are evidence: the first exposed registration-wide renderer grants,
raw/projected overlap, and hidden cell symbols; the second exposed clipped fragments granting a
complete form; the third repair iteration exposed separator-containing left clips. Do not move or
delete them.

The D pre-review ref is commit `39214b67224bfe658f38ef635a20ce5987df5e1b`, tree
`8c1a7aaceb9cca4f514a12860255c7b8137f89cf`. The first post-review ref is commit
`e6cf320a960309f7bed0b3e38023e91d7fad8d51`, tree
`688831a7cc86c2c777eb3f4c479454526dadf5d3`; it retains the producer-kind and parent-alias
repairs. The final D ref is commit `7d1d5879b7264e8d3d0e934ce1ece4b95cace6dd`, tree
`5c16f5be26502b009d23021da8b271c0c2e5625b`; it adds the last external-reference Copy seed lane.
Commit `1d9a521` has the final tree exactly. Do not move or delete these refs.

The `-code` ref points to commit `154284a100bad9832dbaed9c00cf999b287ff91c`, the exact complete
handoff worktree before this baseline paragraph was added. Its tree is
`9e070166d6b8e6508f15f523f2f2b35b05db1ab3`; its binary diff from HEAD hashes to
`c8cadb3b5d9e837f5451c5469edc3f78d5d0428e0435084aff71f10dee111ae4`.

The main ref is updated once, immediately after this paragraph, to capture the complete current
worktree including these instructions. Do not move or delete either ref. Ordinary `git commit` and
`git push` do not include custom `refs/codex-review/*` refs.

Review a successor against the recorded baseline with:

```bash
baseline=refs/codex-review/c0b-before-successor-20260831
git diff --stat "$baseline" --
git diff --binary "$baseline" -- \
  Cargo.toml Cargo.lock crates/skit-cli crates/skit-tui-walker-support crates/skit-tui
```

To inspect the original C0b delta rather than successor changes:

```bash
git diff --stat e82bae8 refs/codex-review/c0b-before-successor-20260831
git diff --binary e82bae8 refs/codex-review/c0b-before-successor-20260831 -- \
  Cargo.toml Cargo.lock crates/skit-cli crates/skit-tui-walker-support crates/skit-tui
```

Always resolve and record the ref's commit ID before review. If the ref unexpectedly differs from
the handoff-era objects, use the immutable `-code` commit ID above and treat the ref move as a
finding. Do not compare only `git status`; that misses deleted or rewritten evidence.

The final three refs preserve the adversarial review sequence. `c0b-final-pre-review` is the first
complete implementation, `c0b-final-delta-review` contains the lease, Linux promotion, Windows
flush, namespace grammar, and acquisition-abort repairs, and `c0b-final-second-review` adds the last
retained-handle empty-directory proof. The last ref's tree is exactly commit `256287d`'s tree.

## Measured environment facts. Do not re-derive these.

- **`#[cfg(test)]` modules inside `crates/*/src/` ARE coverage-instrumented and gated.** Measured:
  `cargo llvm-cov -p skit-cli-rs --lib` emits `SF:` records for `cli/tui_real_host.rs` (3819 `DA:`
  lines) and `cli/tui_real_walker.rs`. Source under `crates/*/tests/**` gets no `SF:` record at all.
  Any pattern copied from the legacy integration-test owner that has mutually exclusive branches
  will fail the gate once it moves into `src/`.
- **`cargo mutants` skips `#[cfg(test)]` items**, so private skit-cli test modules are
  mutation-blind while walker-support is mutation-visible. Put every rule that can honestly live in
  walker-support there.
- **The global workspace coverage gate still has debt from commits before Phase3d.**
  `cargo llvm-cov --locked --workspace --all-targets --all-features` then
  `scripts/check_coverage.sh lcov.info` reports exactly **80** uncovered executable lines:
  `cli/tui_host.rs` 34, `store/mutations/agent_skill.rs` 20, `tui/screens/add.rs` 7,
  `runtime/javascript_deps.rs` 5, `cli.rs` 5, `run/command.rs` 4, `runtime/launch.rs` 3,
  `screens/modal.rs` 1, `runtime/uv.rs` 1. I measured the identical 80 with the same per-file
  distribution at `33d3863` with all Phase3d work stashed. **This must be driven to zero before
  final delivery.** It is not caused by Phase3d, and Phase3d adds none.
- **`cargo mutants` is also already red on this branch.** A package-scoped run
  (`--no-config --package skit-tui-walker-support --file .../lib.rs`) reports 46 missed after
  Phase3d versus **52 missed at the stashed baseline**, so Phase3d kills six previously surviving
  mutants and adds none. The survivors sit in `validate_styled_frame`, `validate_timeline`,
  `validate_event_chains`, `StyledFrameSnapshot::to_buffer`, `manifest_digest`, and
  `validate_progress`. The repository `.cargo/mutants.toml` sets `test_workspace = true`, so the
  real gate runs the whole workspace suite per mutant and may kill more; that run needs roughly
  20 GB of scratch and did not fit when tried. **Run the real gate before final delivery.**
- **The mouse/keyboard parity blocker is resolved in `e0a515d`.** It was a product regression, not
  an over-strict property. The bounded property stays intact with no exemption. The persisted
  counterexample and the full model walker now pass: **166 passed / 4 ignored**. The full locked
  workspace all-target/all-feature suite also passed at this checkpoint. Read the next section for
  provenance and the exact product fix.
- Disk was the binding constraint and is now workable. A read-only audit identified rebuildable
  Cargo artifacts, and safe Cargo cleanup reclaimed about **75 GiB net**. The checkpoint after the
  subsequent test and coverage runs had **122 GiB free**. The current integration worktree's warm
  `target`, the persisted parity seed/reproductions, and the nine `uiwalker-mouse-review-*` review
  directories were preserved. Cargo refused several old scratch/target directories with invalid
  `CACHEDIR.TAG`; they remain. Do not imply that they were removed, and do not raw-delete them.
- The user declared the disk side task complete. Do not monitor or clean again unless the user asks.
- **Do NOT set `CARGO_TARGET_DIR`.** The worktree-local
  `/home/ubuntu/coding/skit-uiwalker-mouse-integration/target` is warm and large. Pointing Cargo at
  `/home/ubuntu/coding/skit/target` rebuilds the world and costs an hour. Set only
  `export TMPDIR=/home/ubuntu/coding/skit/.tmp`. This corrects the "Build and disk safety" section
  further down, which was written before the warm worktree target existed.
- Several `gpt-5.6-sol` agents ended a turn with a transient "Selected model is at capacity" error.
  Their edits remained in the shared worktree and a follow-up turn resumed them successfully. Retry
  from the existing diff; do not restart work or turn this into a global serialization rule.

## Resolved parity regression (`e0a515d`)

The persisted `driver::bounded_smoke_walk_checks_every_transition_boundary` counterexample exposed a
real product regression. Its provenance is `0edf38c`/`432fc48`: pointer transactions made a matching
mouse Up activate the run picker, but the open-picker anchor still called `open()`. A second click on
an already-open anchor therefore kept it open, while the keyboard endpoint closed it.

Commit `e0a515d` changes the matching-Up path to call `toggle()`. Pointer Down still only arms the
anchor; a release away cancels the press; a stale matching release after cancellation is ignored.
The endpoint contract now proves that the second picker click closes the same dropdown as Escape.
No property exemption was added or widened.

Two independent final reviews both reported **0 blocking / 0 major / 0 minor**. The persisted bounded
property passes, the complete model-walker target passes at **166 passed / 4 ignored**, and the full
locked workspace all-target/all-feature test suite passes. The seed, reproduction artifacts, and
review directories remain preserved under the warm integration target/review storage as historical
evidence; they are no longer blockers.

## Durable workspace

- Persistent integration worktree: `/home/ubuntu/coding/skit-uiwalker-mouse-integration`
- User's main worktree: `/home/ubuntu/coding/skit` — dirty and unrelated; do not edit/reset/clean its source.
- Branch: `integration/ui-walker-on-mouse`
- Upstream/base: `origin/fix/tui-mouse`
- HEAD: `978aff1` `feat(walker): capture typed checkpoints atomically`
- Git reports 34 commits in `origin/fix/tui-mouse..HEAD`.
- The previous `/tmp/skit-uiwalker-mouse-integration` worktree was erased by VM reboot. Do not use `/tmp` for durable source again.
- `/home/ubuntu/coding/skit/ui-walker-handoff.md` is an older 228-line pre-reboot copy with stale HEAD/Phase3c state. Do not use or edit it; this `HANDOFF.md` is authoritative.

PR #48 metadata, reverified with `gh pr view` on 2026-09-01:

- URL: https://github.com/t41372/skit/pull/48
- State: OPEN, not draft
- Title: `feat(tui): add deterministic UI model walker`
- Remote head: `feat/issue-46-ui-walker`
- Remote head OID: `9fe8512e06e51de9bc4046fce70c8dfedf91bfce`
- Current base: `rewrite/rust-ratatui-complete-20260808-codex`
- Mergeability: `MERGEABLE`
- The displayed remote checks are for the older pushed remote head, not the local 33-commit branch;
  they do not validate the local C0a/C0b commits.

Do not push, force-push, or change the PR base until the full migration, model comparison, and final gates are complete. The intended final base is `fix/tui-mouse`, but verify remote state before changing it.

Read and obey `/home/ubuntu/coding/skit/AGENTS.md`. It is the hard product and quality contract.

## User decisions

The user strongly rejects `FakeHost`: a high-level test implementation that maps `Effect -> Action` creates a second product truth and permanent maintenance burden. The final design must use:

1. private production `skit-cli::TuiHost`;
2. real per-profile TempDir data/state/config/home/cwd/external trees;
3. production stores and application services;
4. low-level injected ports only at actual external boundaries; and
5. fresh seed plus operation/effect-history replay, never cloning a live TempDir or inode/source claim.

Do not merely stop calling FakeHost. Migrate its useful contracts to production TuiHost tests, store/application contracts, low-level fault tests, or real reachable walker flows, then delete it and its high-level fixtures. No test adapter may decide product behavior.

The user later declared the storage side quest complete. Do not monitor or clean storage unless the
user asks again. The earlier blanket `cargo clean` authorization is historical and must not trigger
proactive cleanup now.

The final model comparison is explicitly required:

- Terra: `gpt-5.6-terra`, reasoning `high` (the user allowed high or xhigh).
- Luna: `gpt-5.6-luna`, reasoning `max`.
- Give them separate byte-identical copies of the same final real-host corpus and the exact same prompt.
- Keep reviews independent; neither model sees the other's output.
- Compare reproducible findings, confirmed defects, misses, false positives, completeness, and work style.
- Do not add any LLM/agent invocation to GitHub Actions.
- Old FakeHost corpora/reviews are obsolete and must not be used for this comparison.

## Working mode

The user explicitly asked the current primary agent to finish the remaining mission. Continue after
periodic handoff updates; do not stop at a handoff checkpoint merely to wait for another agent.
The primary agent owns implementation and verification. Use subagents for bounded independent
audits or reviews when they improve confidence or parallelize a real dependency. There is no global
writer rule and no requirement to serialize implementation through one named agent.

Use parallel, dependency-aware workstreams. Do not serialize independent work behind a single activity:

1. Build a dependency graph for the next objective and parallelize independent investigation, test migration, artifact validation, coverage, and review work when useful.
2. Give a writing agent explicit file/module/test ownership. Multiple agents may write concurrently in disjoint scopes or persistent worktrees. Coordinate only where scopes overlap.
3. Write failing contracts before each behavior change. Add positive and refusal/corruption/race/rollback cases as appropriate.
4. Implement complete, architecture-coherent, maintainable solutions through production logic and low-level ports. Never add a behavior fake.
5. Integrate compatible workstreams at explicit checkpoints. Run focused gates per workstream, then shared gates once for the combined source instead of needlessly repeating long gates serially.
6. Freeze the scope being reviewed and record its hashes. An independent reviewer stays read-only for that scope, while other agents may continue useful work in non-overlapping files/scopes.
7. Fix each substantive finding with a mutation-killing contract. Re-freeze only the affected scope and re-review until 0B/0M/0m.
8. Commit coherent reviewed units. A commit may combine several parallel workstreams when they form one architectural result.

Avoid concurrent edits to the same file and avoid workspace-wide formatters while another agent owns shared files. This is a conflict-control rule, not a global writer limit. Tie long gates to pre/post fingerprints and keep the user updated during long runs.

Use `apply_patch` for edits. Verify subagent reports yourself; an earlier implementer reported a full
workspace suite green while the model-walker test was failing in its own working tree. Run the gates
again before accepting a report.

## Architecture

- Private production host and real TempDir composition live in skit-cli private/test modules.
- Generic engine, artifact DTOs/validators, and product-independent asciicast live in `skit-tui-walker-support`.
- Concrete frontend owns real `LibraryState`, `TuiSession`, Ratatui `TestBackend`, and `ViewGeometry`.
- Concrete host wraps `SeededTuiHost`; its adapter only calls production `dispatch` (preflight plus serve).
- `HostAdapter::observe` receives the exact checkpoint frontend observation. Do not add `Rc`, shadow state, or duplicate reducer state.
- Counterfactual replay spawns a fresh seed and replays successful operations. Do not clone/copy live TempDirs.
- Private CLI artifact code consumes generic engine checkpoints and public support schemas.

Do not make walker-support depend on skit-cli, skit-tui, skit-ui, or proptest. `skit-tui` already dev-depends on support; reversing that creates a cycle. A CLI dev-dependency on support is valid.

The real host owns one persistent structural `PathMap`. Normalize only typed/schema-known product fields. Preserve user values whose names/text merely resemble IDs, paths, UUIDs, roots, `.run-*`, or `.injected-*`. Entry/source replacement generations must remain observable.

## Branch commits over `fix/tui-mouse`

```text
c49f84d feat(tui): add deterministic UI model walker
61b4bc1 fix(tui): harden walker parity probes
36122c4 test(tui): broaden walker viewport profiles
4c6832f feat(tui): add local agent review corpus
d81d72d fix(tui): serialize add review row spans
b5066f8 fix(tui): report tagged walker screens
7eb7ea5 fix(tui): repair agent-reviewed responsive overlays
432fc48 fix(tui): align walker with pointer transactions
5db5179 fix(tui): record pointer dispatch chains
5a80027 fix(tui): keep compact confirmation operable
225a23f test(tui): register compact confirmation coverage
45c7cef fix(i18n): restore parameter editor labels
61579e0 feat(tui): distinguish presented review frames
d31e707 refactor(cli): centralize the production TUI host
05abd69 refactor(cli): inject TUI platform services
2845aa1 refactor(runtime): inject launch and uv transport
7d70c91 refactor(cli): inject TUI run services
752d2f0 refactor(cli): inject interpreter platform policy
c4c045d refactor(cli): inject preference filesystem access
a919ac8 refactor(store): inject agent skill install faults
f41eb64 fix(ui): resolve contextual runner commands
509cf1f feat(walker): add replayable generic engine
dd163fe test(walker): seed the production TUI host
b8e8ce9 test(walker): connect the production host engine
0ce16b8 fix(cli): keep completion install inside explicit home
881a0fc feat(walker): record real schema three traces and final liveness
33d3863 fix(tooling): own the frozen runner and add review copy oracles
0945263 feat(walker): install and validate real-host disk bundles
8d24dd7 feat(walker): replace host values with type-compatible sentinels
e0a515d fix(tui): toggle open run picker on click
f748a1b feat(walker): add deterministic projection helpers
e82bae8 feat(walker): make real file allocation deterministic
256287d feat(walker): retain stable real sandbox ownership
978aff1 feat(walker): capture typed checkpoints atomically
bd8eb08 feat(walker): project complete recorded checkpoints
4c62b73 feat(walker): validate complete projected bundles
1d9a521 feat(walker): project live filesystem modes
745826c feat(walker): track live frontend locale
22ff2ca feat(walker): classify host-served effect coverage
81d32e7 feat(walker): define aggregate review corpus contract
b975985 feat(walker): seed replayable corpus inputs
a34d0ff feat(walker): drive semantic corpus operations
151fa7d feat(walker): derive effect coverage from timelines
0b4dbf4 feat(walker): seed deterministic picker trees
7899893 feat(tui): expose rendered walker targets
8c25636 feat(ui): preserve picked Add source provenance
770bcff fix(walker): project picked source provenance
29c1f2d feat(walker): validate ordered event chains
aa99dcc fix(tui): expose only exact picker targets
095ef1c feat(walker): resolve rendered screen targets
716d4c8 fix(walker): project derived Add commit names
14edd22 feat(walker): define exact real-host corpus
5a81602 fix(tui): ignore empty Add paste
e24e458 fix(tui): keep compact runner navigation visible
eb7adcd fix(walker): forecast resized cast canvas
```

`0945263` is Phase3d. `8d24dd7` is Phase4 commit A. `e0a515d` resolves the parity
regression. `f748a1b` is Phase4 commit B. `e82bae8` is Phase4 commit C0a. `256287d` is Phase4
commit C0b. `978aff1` through `eb7adcd` complete C1 through G3 and H. All are reviewed and committed.

## Phase3d completed (`0945263`)

`PHASE3D-PLAN.md` holds the approved specification and both advisor reviews that shaped it. What
landed:

- New public `crates/skit-tui-walker-support/src/bundle.rs` (679 lines) plus `bundle_tests.rs`
  (751 lines). It is pure: relative path layout, `object_view_bytes`, `frame_view_bytes`,
  `timeline_ndjson_bytes` and its decoder, `chunk_json_bytes`, `decode_canonical_bytes`,
  `ChunkViewCursor`, `cast_canvas`, `rebuild_presented_cast`, `RunMetadata` and its validator,
  `ProfileExpectation`, `validate_profile_manifest`, `bundle_digest`, `source_revision`, and
  `BundleLayout`. It contains no `std::fs` and no product dependency.
- `validate_profile_manifest` is **extracted** out of `validate_manifest`, not duplicated.
  Characterization tests pinning every current manifest and chunk-range message and its precedence
  were written first. `validate_object` now also requires the typed envelope to match its stored
  bytes.
- New private `#[cfg(test)] crates/skit-cli/src/cli/tui_walker_bundle.rs` (1633 lines) owns every
  effect: `BundleWriter`, the `symlink_metadata` tree walk, the transaction, `real_git` plus
  `source_identity`, and the product-coupled `validate_reducer_replay`, `validate_live_locales`, and
  the store-quiet host invariant.
- On-disk layout, all validated on read-back of the **installed** bundle:
  `run.json` as the root index, `operations.json`, `final-liveness.json`,
  `objects/{reducer,host,session,styled-frame,geometry}/<sha256>.json`,
  `views/{reducer,host,session,geometry}/<sha256>.json`, `views/frames/<sha256>.txt`, and
  `profiles/<id>/{profile.json,timeline.ndjson,trace.cast,chunks/<id>.{json,md}}`.
  There is deliberately **no `manifest.json`**, so that name keeps exactly one meaning for the
  four-profile `CorpusManifest` the next phase writes. `profiles/<id>/profile.json` holds the
  existing `ProfileManifest`; no new manifest DTO was invented.
- The transaction stages into `.walker-XXXXXX`, validates, resamples the injected fingerprint,
  refuses on change with nothing left behind, then renames with **no destination pre-check** so
  every branch is reachable from a test. An identical installed tree returns its path; any differing
  byte refuses and leaves the installed tree untouched.
- `SchemaThreeSink::finish` now returns a `RealTrace` carrying the operations vector and the
  final-liveness request, which removes two of the smoke-specific hardcodings.
- The seed profile and the review profile are separate identities. The smoke keeps seed
  `engine-smoke` and uses review profile `engine-smoke-60x24`.
- The sink takes an explicit cast canvas and asserts `cast_canvas(rows)` against it, removing two of
  the four disagreeing canvas sources.
- The legacy walker now delegates `kind_directory`, `object_view_bytes`, `frame_view_bytes`,
  `chunk_view_bytes`, and `rebuild_presented_cast` to walker-support and its private copies are
  deleted. Its suite stays at 166 passed and 4 ignored, so those tests now characterize the shared
  API. The legacy private four-field `RunMetadata` is deliberately kept: adopting the seven-field
  walker-support struct would change legacy on-disk bytes, and the legacy layout has no
  `final-liveness.json` to digest.

Counts after Phase3d: walker-support lib **78**, `skit-cli --lib cli::` **268 passed / 4 ignored**,
`skit-tui --test model_walker` **166 passed / 4 ignored**. Full workspace suite was green at that
checkpoint and is green again after the later parity fix. `cargo fmt`,
workspace Clippy `-D warnings`, workspace Rustdoc `-D warnings`, `check_english.sh`,
`test_coverage.sh`, `test_english.sh`, `test_tooling_contracts.sh`, and `git diff --check` pass.
`cargo mutants` scoped to `bundle.rs`: 90 mutants, 89 caught, 1 unviable, **0 survivors**.

Two independent reviewers reached **0 blocking / 0 major / 0 minor**. Their substantive findings, all
fixed before the commit, were: the read-back never bound `profile.operations_sha256` to anything; the
read-back had no typed live-locale proof, so a nested `"locale"` member could authorize a false
transition because `skit_ui` types have no `deny_unknown_fields`; two refusal branches were
unreachable; and styled-frame plus content-object envelopes were not typed-round-trip checked.

The last two commits were made before the reboot, so Phase3c source survived even though its `/tmp` worktree did not.

## Phase4 commit A completed (`8d24dd7`)

`PHASE4-PLAN.md` section "Commit A" is the specification. Two advisor rounds shaped it and two
independent reviewers cleared it at **0 blockers**. What landed, and why:

**It repaired a live round-trip defect.** `normalize_artifact_object` wrote
`Value::String("<source-identity>")` over `identity: Option<SourceIdentity>` and
`Value::String("<modified>")` over `modified: u64`. Neither deserializes. Yet three committed sites
require a typed round trip that also re-serializes byte for byte: `parse_library_state` and
`validate_reducer_action` in `tui_real_walker.rs`, and `validate_live_locales` in
`tui_walker_bundle.rs`. The state normalizer reaches those tokens through
`normalize_library_json` to `normalize_workflow_artifacts` to `normalize_add_screen`, so a reducer
object holding `Screen::Add` with one draft could not be parsed back. It was latent only because the
smoke walk never presents `Screen::Add` with a draft. A 100-operation walk does.

**`modified` is now an ascending rank, allocated from the sorted scan, never from the observation.**
This detail is load-bearing and cost two advisor rounds to get right, so do not "simplify" it:

- `AddSourceState` sorts drafts with `sort_by_key(|d| Reverse(d.modified))` (`skit-ui/src/add.rs:391`
  and `:426`), and the normalizer walks them in stored order. An observation-order counter therefore
  gives the newest draft the smallest number and **inverts** the ordering, which breaks
  `validate_reducer_action` as soon as `DraftDeleteOutcome::Changed` lands with two survivors.
- A gap or midpoint allocator was also proposed and rejected: its output depends on allocation order,
  and the first allocation ran over `HostObservation.drafts`, which came from `fs::read_dir`. Two runs
  would produce different bytes for identical inputs.
- The allocator lives in `crates/skit-tui-walker-support/src/sentinels.rs` as
  `AscendingRankAllocator`. `PathMap::refresh` feeds it the ascending scan;
  `normalize_artifact_object` is a pure lookup and **refuses** a miss. An unseen value that does not
  exceed the current maximum refuses, which also enforces mtime distinctness across drafts. The rank
  table never shrinks and never renumbers, so deleting a draft cannot renumber a survivor. That
  property is what protects the store-quiet host rule in `tui_walker_bundle.rs`.

**`identity` is now a typed sentinel keyed on the `same_file` projection.** It keeps the observed
`platform` tag and refuses a missing or unknown tag. The file rank keys on device plus inode for unix
and volume plus file index plus creation time for windows, matching
`SourceIdentity::same_file` (`skit-application/src/lib.rs:129`) exactly. Keying on the raw identity
JSON was the obvious choice and is **wrong**: a re-stat after an in-place write yields a new change
time, so the artifact would say "different file" where the product says "same file", which is exactly
the case `DraftChanged` exists to report. A unix-only change sub-generation keeps full inequality
visible. Windows `file_index` is a decimal **string**, because `u128_decimal`'s `Encoded` enum is
`untagged` and a number would deserialize but then fail the canonical re-serialize check.

**It fixed a second live defect: nondeterministic draft order.** `HostObservation.drafts` came from
`fs::read_dir` over `tempfile`-random `skit-new-*` names, so its array order varied between runs and
would have broken `main_and_replay_bundles_are_byte_identical` as soon as two drafts existed. It now
sorts by content bytes, then extension, then path, reusing the comparator `refresh` already used for
`<draft:N>`. Recorded hazard for the vector author: that comparator breaks ties on the random path, so
the 100-operation vector must not author two byte-identical drafts with the same suffix.

**It fixed a third live leak.** `normalize_settings_screen` allowed the template
`"Stored copy (original: {})"`, which does not exist in `skit-i18n`. The real copy-mode template is
`"Keep a copy — your original file is never modified. Source: {}"` (`skit-ui/src/settings.rs:926`), so
every copy-mode entry leaked an absolute source path.

**`permissions/unix_mode` is seeded, not normalized.** The revision-1 plan folded it into the sentinel
rewrite; the advisor rejected that as the wrong instrument. Draft permissions are already
deterministic because `tempfile` creates `0o600` regardless of umask. The umask-dependent case was
`WalkerExternalSeed::File` with `unix_mode: None`. That field is now a required `u32`, so "every seed
carries an explicit mode" is a compile-time guarantee. `TreeRecord.mode` and `portable_mode` remain
commit D's problem.

The artifact normalizer chain is now fallible up to `observe` and `canonical_action`, and the
fabricated identity fixtures were rewritten to real `platform`-tagged shapes.

Counts after commit A: walker-support lib **84**, `skit-cli --lib cli::` **275 passed / 4 ignored**.
Green: `cargo fmt --all --check`, `git diff --check`, workspace Clippy `-D warnings`, workspace
Rustdoc `-D warnings`, `check_english.sh`, `test_english.sh`, `test_tooling_contracts.sh`,
`test_coverage.sh`. `cargo mutants` scoped to `sentinels.rs`: **0 survivors**. A full
`cargo llvm-cov clean --workspace` then fresh lcov then `scripts/check_coverage.sh` reports exactly
the known **80** baseline lines in the nine unrelated files and **zero** lines in `sentinels.rs`; a
reviewer's contrary report was traced to stale objects in the warm target directory. At the commit-A
checkpoint, the only red in `cargo test --workspace` was the later-diagnosed
model-walker regression. Commit `e0a515d` resolves it; do not treat that historical result as current.

## Parity regression completed (`e0a515d`)

The persisted counterexample was diagnosed as a product defect with provenance in
`0edf38c`/`432fc48`, not as an over-strict walker property. The open run-picker anchor now calls
`toggle()` on its matching mouse Up. The contracts keep Down-only arming, release-away cancellation,
and stale-release refusal. No property exemption was added. Two independent final reviews reported
**0 blocking / 0 major / 0 minor**. The persisted bounded property and the full model walker pass at
**166 passed / 4 ignored**, and the full workspace all-target/all-feature suite passes.

## Phase4 commit B completed (`f748a1b`)

New mutation-visible `crates/skit-tui-walker-support/src/projection.rs` owns the two
product-independent helpers that commit C needs:

- `rewrite_json_pointer_matches` applies a fallible callback to every match. Patterns use RFC 6901
  `~0` and `~1` escapes. A raw `*` visits both array elements in ascending index order and object
  values in lexically sorted UTF-8 key order. Missing paths and container mismatches yield no match;
  invalid patterns refuse before mutation; callback failure stops deterministically after retaining
  earlier mutations.
- `replace_longest_text_tokens` orders candidates by descending UTF-8 byte length, then lexical raw
  token order for ties. It exposes the complete prefix and suffix to a caller-supplied acceptance
  policy, ignores empty raw tokens, and preserves the existing sequential replacement semantics.

The CLI's existing English boundary policy now delegates to the shared text helper byte-faithfully.
Commit C2 must add the bounded four-locale text policy and its contracts; commit B does not claim
that product policy is complete.

Both independent final reviews reported **0 blocking / 0 major / 0 minor**. Fresh focused evidence:
walker-support **103 passed / 103 total**; `target/phase4b-projection.lcov` passes
`scripts/check_coverage.sh`; and
`/home/ubuntu/coding/skit/.tmp/phase4b-mutants-evidence/mutants.out` records **27 total, 23 caught,
2 unviable, 2 timeout, 0 missed**, with `end_time` present. The combined workspace Clippy, Rustdoc,
fmt, English, and tooling checks pass.

## Phase4 C0a completed (`e82bae8`)

C0a is committed and must not be folded back into C0b. It removed four proven filename-entropy
sources while preserving production randomness and lifecycle ownership:

- authored drafts `skit-new-<6hex>.<kind>`;
- draft quarantine `.skit-quarantine-<6hex>/draft`;
- injected source `.injected-<6hex>.<suffix>`; and
- store-owned launch snapshot `.run-<32hex>.<suffix>`.

The walker adapter supplies closed typed allocation purposes and deterministic per-host counters.
It never supplies an Action, Effect, product result, launch path, or open launch file. `FileStore`
alone owns the launch prefix/path, direct-child rule, `create_new`, identity validation, copied bytes,
source mode, sync, rollback, and `PreparedLaunch` cleanup. It accepts only a typed
`LaunchSnapshotStem` and records accepted/rejected attempt evidence. Production keeps one random
attempt; the walker can retry a real collision.

The store retains an opened file identity across cleanup, preserves replacements, rejects unverified
paths, protects symlink/direct-child boundaries, and handles Windows readonly/hardlink behavior with
target-scoped dependencies. Stale `.run-*` sweeping runs even without injection and skips the live
current payload. The accepted Unix boundary is best-effort retention of unreadable stale `000/0200`
files; active cleanup still works through the retained handle.

System and recording allocators share exact Unix private modes (`0600` files, `0700` directories).
Walker system temp is a real per-host `system-temp` directory. Observation registers pending
artifacts, refuses residual temp entries, completes all fallible tree work, and drains transcripts
only on success. Allocation transcripts include purpose, attempt, stem/counter, location, path, and
stable accepted/rejected outcome; only accepted artifacts enter the `PathMap`.

C0a also hardened the mutation gate itself. `.cargo/mutants.toml` has a narrowly documented
Linux-only exclusion for the one non-Unix identity helper that Linux parses but cannot compile.
`scripts/test_tooling_contracts.sh` fail-closes the complete top-level mutants config, exact ordered
exclusions, valid TOML commas, and the two Ubuntu mutation jobs. Negative fixtures reject hidden
`exclude_globs`, `examine_globs`, `examine_re`, `skip_calls`, extra exclusions, and an injected
Windows job.

Final C0a evidence: store `93/93`, launch safety `6/6`, CLI lib `311/4 ignored` at its freeze,
workspace Clippy/Rustdoc/fmt/tooling green, added executable lines covered, shared identity shard
`7 caught / 0 missed`, and both final independent reviews `0/0/0`.

## Phase4 C0 completed (`e82bae8`, `256287d`)

Three read-only design audits found that the former JSON-only commit C was incomplete. Random
sandbox roots and random draft/launch names can reach the serialized session, a rendered styled
frame, its text view, and `trace.cast`. A typed JSON `PathMap` cannot repair bytes that the renderer
already produced. A blanket cell rewrite or detached re-render would lose provenance or change
layout, styles, geometry, and session history.

Revision 6 splits C into five dependency-ordered commits:

- **C0a** makes low-level real file allocation deterministic at the four proven entropy boundaries.
- **C0b** gives final corpus runs a stable marked and leased real sandbox.
- **C1** adds one host-owned atomic checkpoint capture and exhaustive typed state, Action, and Effect
  projection. Raw values still drive the reducer and production host.
- **C2** projects session nodes, bounded four-locale host text, and draft-derived fields with explicit
  provenance while preserving user lookalikes.
- **C3** scans every installed bundle file and compares fresh main/replay trees byte for byte.

C0a's four allocator boundaries are authored draft `skit-new-*`, draft quarantine
`.skit-quarantine-*`, injected source `.injected-*`, and launch snapshot `.run-*`. It landed as
`e82bae8`. Production remains random. The walker uses independent typed counters. `FileStore` retains
exclusive ownership of launch snapshot path construction, file identity, bytes, mode, sync,
rollback, and cleanup. The final focused mutation shard is `7 caught / 0 missed`; final independent
reviews are `0 blocking / 0 major / 0 minor`.

C0b uses a marked stable ephemeral OS-temp namespace only for corpus generation.
Ordinary unit tests keep random `TempDir`. A per-profile lease permits different profiles in
parallel. Main and replay for the same profile run sequentially, drop and fresh-seed, and never copy
a live directory. The walker maps its low-level `TempLocation::System` boundary to a real
profile-local `system-temp/` child. Production still uses `Builder::tempfile()`. This prevents two
profile counters or an ambient process from racing for the same deterministic injected-source name.
The host refuses a checkpoint if this transient root contains any residual entry.

Two C0b advisor reviews exposed important gaps in revision 4. Revision 5 required a
persistent parent-level namespace-init lock; a `SafeProfileId` newtype keyed on the review profile;
explicit random versus stable APIs; consuming fallible close with structured primary-plus-cleanup
errors; directory identity across rename; a deterministic cleanup quarantine with all nine
original/quarantine crash states; Windows reparse refusal; schema-2 typed sandbox metadata; and a
structural-only C3 exception for the declared root fields. Read the complete C0b section before
implementation.

Schema-2 metadata applies to ordinary random smoke bundles as well as final stable bundles. Random
tests must record their actual `TempDir` root, `random` mode, review-profile map, and truthful
`linux`/`macos`/`windows` platform. Only the dedicated final pair uses `stable`; stable acquisition
may remain unsupported on macOS even though random macOS metadata is required by CI.

Commit `256287d` contains all of that architecture. Its reviewed scopes are:

- Support/engine, eight files, aggregate
  `da8c8d11de5fc0d86001d8d5e9b44d131e39e79b73d7d4d10685f3a6f731d063`:
  `SafeProfileId`; derived Linux/macOS/Windows literal layouts; complete Win32 alias/device-name
  refusal; typed canonical init/namespace/lease/sandbox markers; random/stable metadata; the
  nine-state quarantine decision; schema-2 `RunMetadata`; sandbox-bound digest; typed profile DTOs;
  and `WalkerEngine::into_parts`. Support is now `132/132`, complete line coverage, all final targeted
  mutation shards have zero missed, and two independent final reviews are `0/0/0`.
- Wiring/bundle, two files, aggregate
  `3b1cef190b8aafce0c61f667dc2b987a90cc371b73a4fac0bf23449e06902dbb`:
  truthful random `RecordedRealTrace`, a dedicated stable main/close/fresh-replay pair, typed
  primary-plus-cleanup failures, schema-2 writer/readback, stored-metadata digest recomputation, and
  stable installed-tree byte equality. Focused walker and bundle contracts are included in the full
  CLI-lib gate. Two final whole-C0b reviews are `0/0/0`.
- Legacy typed-profile migration:
  `cast_projection.rs` `e65e709fc7e028bfae3d5e2dd1ca09ef868986dff3f173187721bc670f21398a`,
  `corpus.rs` `157d0fb665a7cca516d027bdf1e65c88c3cb66bc49e4c85a0ec6732bbb34c4a2`,
  and `driver.rs` `662a29e8a239865507702616451d84d84194f5aa9d79dbd741201e4c9ef5bfa3`.
  Model walker `166/4 ignored`; two final whole-C0b reviews are `0/0/0`.

The private filesystem owner is accepted and committed. Its final hashes are:

- `tui_real_sandbox_fs.rs`:
  `122a6855d2a123c63edf3d5a0c2d2cb1bea544c81190524ea8836ce643ee340b`;
- `tui_real_host.rs`:
  `8b330769009e1f1710b8837be5cff87b968fc73330b6dc3acff1213742b8a1d7`;
- `Cargo.lock`:
  `9fe0266613abfde07693648b4c7e1ed41271942c45b96cb7da6740d67f9fcbd9`.

The review sequence found and closed Windows replace-on-rename, ancestor and cleanup TOCTOU,
acquisition identity loss, init-lock release, special-bit and path-chmod defects, unacquired lease
unlock, Linux directory promotion mutation, Windows sync no-op, namespace panic, acquisition Drop
error loss, and nonempty exact-mode directory promotion. The final two independent delta reviews
reported `0/0/0`.

Current C0b code map:

- `skit-tui-walker-support/src/sandbox.rs`: `SafeProfileId`, derived markers/layouts,
  `SandboxMetadata`, and `decide_quarantine`.
- `skit-tui-walker-support/src/bundle.rs`: typed schema-2 `RunMetadata`, canonical codec, profile
  paths, layout, and sandbox-bound `bundle_digest`.
- `skit-tui-walker-support/src/engine.rs`: consuming `WalkerEngine::into_parts` for cleanup after
  healthy or poisoned runs.
- `tui_real_sandbox_fs.rs`: retained handles, typed identities/tickets, relative create/open/remove,
  no-replace rename, real directory sync, exact Linux modes, Windows capability pins, and low-level
  fault/race seams.
- `tui_real_host.rs`: typed namespace roots, initialization/profile leases, typed Original/Cleanup
  evidence, the nine-state lifecycle, marker-last cleanup, explicit acquisition abort, stable/random
  owners, `spawn_stable_in`, metadata, and consuming close.
- `tui_real_walker.rs`: `RecordedRealTrace`, `StableTracePair`, typed pair errors, and
  `record_real_smoke_stable_pair[_in]`.
- `tui_walker_bundle.rs`: schema-2 writer/readback/install and stable installed-tree equality.

Ordinary `record_real_smoke_main/replay` remain random and parallel-safe. Only the dedicated pair is
stable. Tests take an explicit `StableSandboxNamespace` under a caller-owned `TempDir`; they do not
mutate the literal `/tmp` namespace. The default system API is referenced but not executed by unit
tests.

The C1 ownership decision is also settled. Delete `TraceAction`; do not add `TraceEffect`. The generic
engine owns detached checkpoint-cause clones and lends them to one fallible host
`capture_checkpoint` hook. The host refreshes its single `PathMap` once, registers pending event
artifacts, projects all checkpoint channels, and drains the transcript only after complete success.
Projection failure poisons the engine and emits no partial checkpoint.

## Phase4 C1 completed (`978aff1`)

C1 implements that settled design. The generic engine moves raw Actions into the reducer and raw
Effects into the production host. It clones only checkpoint evidence, lends the clone fields through
`CheckpointCauseProjection`, and receives one `CheckpointCapture`. Capture failure never calls the
sink and poisons the engine. The sink contract now requires an error to leave no externally visible
artifact; `SchemaThreeSink` stages its five objects and publishes objects, cast, and row only after
complete success.

`SeededTuiHost.path_map` is now a direct persistent `PathMap`, not `Rc`, `RefCell`, a cloned map, or a
frontend shadow. Capture order is raw host facts, pending-event snapshot, one map refresh, event
artifact registration, cause projection in field order, detached state, the explicit C1
byte-preserving session seam, host/tree/transcript projection, and one success-only snapshot-prefix
drain. Failure keeps the event prefix. Map registrations made before failure remain invisible and
idempotent because the engine is terminal.

The authoritative typed projector contains no wildcard enum fallthrough and has executable fixtures
for Action 75, Effect 16, Screen 9, AddAction 30, AddEffect 10, HealthAction 12, PreferencesAction
23, PreferencesEffect 7, RunnerManagerAction 20, RunnerEditorAction 8, and SettingsAction 9. A strict
serialized Screen dispatcher is paired with the typed Screen guard because workflow history is
private. Helpers reject empty, multi-tag, unknown, scalar, wrong unit/payload, wrong internal tag,
and noncanonical shapes. Deserialize and exact reserialize checks prevent accepted unknown fields
from disappearing.

C1 deliberately projects only `AddEffect::DeleteDraft.draft` beyond the previous projection surface.
The other Add effect paths, session nodes, bounded localized text, and provenance remain C2. One
narrow state addition projects the typed Add `notice.draft_deleted` path because the explicit real
Add deletion replay cannot otherwise make the projected host response reproduce the projected next
state. Adjacent user lookalikes remain byte-exact.

The real Add contract opens and highlights a real draft, requests and confirms deletion, captures
the reducer row, dispatches the raw effect through production, captures the host row, and validates
both transitions with `validate_reducer_action`. Reducer-emitted equals the next host request, and
reducer-emitted, host-request, plus a direct host-emitted fixture all remove raw draft path, modified
time, and identity. Separate contracts prove pending event registration precedes cause projection,
failure retains the transcript, success drains once, Noop replay still captures, and raw values are
not changed by evidence projection.

Final commit tree: `2716627756894e3a7f2b86ef476d3fa82ebdfd20`. Final host source SHA-256:
`c4f7ecf59b09593ee29051d6e85192dbd65e5950afdc31e1b238e3f9898fc867`. The final coverage and
review evidence is in the current-checkpoint section above. No push or PR-base change followed the
commit.

## Phase4 C2 completed (`bd8eb08`)

C2 projects the remaining typed Library, screen, Action, Effect, and session path owners through
the single persistent host `PathMap`. It uses exact top-level `AgentReviewNode` schemas, canonical
current-platform native-path objects, bounded host-text tokens, exact four-locale Agent Skill
templates, and cause-proven persistent Add/status/runner ownership. User values and lookalikes stay
byte-exact.

The first immutable review found five live defects. The final code gates Add input mirrors by the
serialized Add stage; preserves `.prompt.md` and `.prompt` so projected reducer replay keeps prompt
semantics; records separate kind-picker, file-picker, and kind-sensitive review-name spellings;
keeps a stable derived value stable until that value changes; and encodes non-Unicode Windows path
components by exact UTF-16 units instead of lossy text. Real sequential contracts cover Review and
Complete session stages, `HighlightDraft`, same and changed `SourceEdited`, Unicode and non-UTF
file-picker snapshots, Unicode kind/review names, `skit-x.prompt.py`, both prompt suffixes, and
forged unregistered provenance.

The typed `LibraryState` and `Action` serde boundary still refuses a non-UTF `PathBuf` before C2
projection. The review proved that this older limitation is not part of C2's new session-native
wire; do not claim C2 added a custom state/action serializer. The reachable session native-path
channel is covered.

Final commit tree: `68a8a929bfe7c27378aab4fc10007c1b73680bb6`. Final host source SHA-256:
`53b3919ed9e794ce8e642f0b827ec535bd33afa4cb8cfc2c219c70bba9de7e0d`. The coverage, gate,
review, and immutable-ref evidence is in the current-checkpoint section above. No push or PR-base
change followed the commit.

## Phase4 C3 completed (`4c62b73`)

C3 adds a mutation-visible closed allocator grammar and exact JSON/text traversal helpers, captures
raw-to-projected artifact facts from the live `PathMap`, and snapshots those facts after final trace
construction but before explicit host close. Facts remain memory-only and run-local. They never
enter JSON, object digests, bundle digests, or main/replay equality.

The installed-bundle reader now runs the oracle only after its core structural, canonicality,
digest, view, cast, replay, effect-chain, locale, layout, and directory-name checks. It accounts for
every declared file. Non-renderer channels have zero draft-name tolerance. `run.json` masks only its
typed, already-validated root fields. Identity and modified values use per-projected-path exact facts;
raw and projected vocabularies must be disjoint, and raw membership is rejected before projected
membership is accepted.

Renderer evidence is narrow. Only referenced, validated styled frames and their byte-exact derived
views/cast can consume an exception. Exact FullPath, Basename, and Stem observations are independent.
Edge-clipped evidence stores its exact source form, bytes, and physical edge and never grants the
complete source or a sibling form. Producer and consumer use the same edge-span helper. This keeps
separator-containing left clips valid while rejecting malformed and wrong-context spans. Stored
symbols omitted by `readable_lines`—explicit skip, legacy skip, continuation, and over-wide forced
width—are scanned with zero tolerance.

Fresh stable main and replay runs use independent facts and compare their complete installed trees
byte for byte. The review sequence found and closed: registration-wide and sibling renderer grants;
raw/projected identity and mtime overlap; hidden cell-symbol leaks; clipped evidence promoted to a
complete form; wrong-context fragment overrides; harmless partial-text false positives; and
separator-containing left clips that could not validate their own bundle. Both final reviewers
reported `0 blocking / 0 major / 0 minor` on `refs/codex-review/c3-post-review3-20260902`.

Final tree: `0bc22f5cef23d7ff627c45d251663badd585f541`. Final source SHA-256 values:

- `tui_real_host.rs`: `52f0f179821bfc7ac23dc081beda591f0e4758b4c1901e621ba134ac7746bfb5`;
- `tui_real_walker.rs`: `d1ed645f8ff91888414a450b1ccbe3b95891d374600832160fdac8bf2941721a`;
- `tui_walker_bundle.rs`: `23bfd74f6300ea07d9a2855d92c3a0d7c76022cb074d143ced7d96a0afe585f8`.

## Phase4 D completed (`1d9a521`)

D makes `TreeRecord.mode` independent of ambient umask without changing the filesystem. The generic
`portable_mode` remains raw for seed reopen checks, physical snapshots, source permissions, and
product behavior. Only `snapshot_path` uses the private observation policy. Proven ordinary-create
files and directories serialize `mode: null`; physical `readonly` remains unchanged.

Exact raw mode is retained for current production payloads and drafts, declared external seeds,
accepted private allocator nodes, stable sandbox child roots, managed executables, symlinks, and
special-bit nodes. A memory-only provenance sidecar records absolute physical nodes and expected
kinds. It uses canonical parent plus unchanged final component as its key, so macOS ancestor aliases
match without following the leaf symlink. Seed and runtime Copy creation record producer-expected
File facts before callers can replace the path; this includes custom stored names and Copy requests
in the `external_references` seed lane. Reference and executable sources retain file-or-symlink
semantics.

The red owner ran the same real seed in child processes under umask `0022` and `0077`. Raw ordinary
modes differed (`0755/0644` versus `0700/0600`), pre/post-observe disk witnesses were equal, and
canonical observations became byte-identical. Explicit `04751`, `0640`, readonly `0440`, symlink,
private allocation, stable-root, replacement, stale-fact, alias, and deferred-error contracts are
covered. Review found and closed current-kind trust before first observation, physical/literal parent
alias mismatch, and an unrecorded external-reference Copy seed lane. Final Sol/max and Terra/high
reviews reported `0 blocking / 0 major / 0 minor`.

Final tree: `5c16f5be26502b009d23021da8b271c0c2e5625b`. Final host source SHA-256:
`9b45a8c1e6602030f08aa4e2960b28382f664e3394f87c28b9842d6f48372f03`.

## Phase4 G3 completed (`716d4c8`, `14edd22`)

The exact vector now uses the same 100 semantic operations in the required manifest order:
`en-80x24`, `zh-cn-120x30`, `zh-tw-40x40`, and `pseudo-120x12`. Factories derive directly from
`required_review_profiles()`. Each profile uses a real stable sandbox, production `TuiSession`,
production reducer, production `TuiHost`, deterministic memory picker tree, Home/Cwd agent-skill
directories, and a replayable four-write editor queue. The vector contains no raw Enter, no Quit,
and no private Add/Preferences/Agent/File/Runner ordinal choreography.

G3 exposed and repaired four live issues:

- derived Add review names were projected in state but not the emitted Commit entry, so stored
  reducer replay diverged; the host now rewrites only exact causal draft provenance and leaves all
  manual lookalikes raw;
- a local action target can advertise multiple different chords; resolution now selects the exact
  target/chord pair and still refuses duplicate identical chords;
- Add swallowed mature input control edits and ignored Paste; text-focused Ctrl+U and Paste now use
  the production `tui-input` state, while nontext controls still reject them;
- Runner rows lacked a stable public mouse identity and the compact pseudo-locale RunnerEditor
  clipped Save. Named live row hits now come from renderer geometry, overlays hide them, duplicates
  refuse as ambiguous, and Save/Cancel occupy the first footer group with field navigation in the
  second group.

The exact bytes, full diagnostic, final source review, test counts, coverage artifacts, and hashes
are in the current-checkpoint section. Do not restore the rejected cause-stream coverage sink: the
only measured source is the completed timeline's typed Host request.

## Phase4 H completed (`eb7adcd`)

The plan's old claim that legacy and stored-timeline canvas algorithms differed was false; both were
already component-wise maxima. H instead fixes the real latent bug: `SchemaThreeSink` used only the
initial viewport for its live cast, so a successful grow Resize disagreed with its completed
timeline at finish. `bundle::maximum_canvas` is now the mutation-visible pairwise rule used by
stored timeline, corpus operation forecast, and legacy forecast. Corpus forecast and resolution use
the same helper that refuses either zero dimension. Legacy keeps its distinct `max(1)` normalization
and has direct tests for both single-zero forms.

The sink starts with the operation forecast but still compares it against `cast_canvas(rows)` during
finish. That equality remains the refusal boundary and stored rows remain the artifact truth. H did
not touch `BundleWriter`, installation, or publication. Full evidence and the final immutable ref are
in the current-checkpoint section.

## Completed work


### Deterministic walker/artifact and UI fixes

- Schema3 timeline with content-addressed reducer/host/session/frame/geometry evidence.
- Exact event chains, including pointer Down and matching Up as distinct terminal dispatches.
- Presented versus NotPresented checkpoints; diagnostic state remains, invisible frames are excluded from cast.
- Lossless stored frame -> Ratatui buffer reconstruction and cast byte validation.
- Chunk/manifest/progress/report integrity, exact allowlists, no symlink/extras, and source pre/post fingerprinting in the old corpus owner.
- Corpus-discovered UI fixes: Add dropdown z-order/hit ownership, RunnerEditor compositing, compact ConfirmRemove keyboard+mouse at 24x5/24x6, 1x1 hit safety, logical CompletedFrame CJK ghost removal, CJK/emoji hit width, shared-footer focus parity, contextual NewRunner mapping, missing Chinese parameter labels, and responsive overlays.

### Production TuiHost and low-level boundaries

- The TUI effect match moved into a persistent private production `TuiHost`.
- Explicit roots, locale synchronization, clocks, environment, token context, editor, output, terminal capability, program probe, run services, raw launch/uv transport, Windows interpreter policy, preference filesystem, and raw agent-skill install checkpoints exist.
- Low-level ports return raw transport/process/filesystem outcomes; production logic maps errors and Actions.
- Real store paths retain atomic writes, rollback, symlink/identity/TOCTOU rules, modes, unknown TOML fields, and v0.4 behavior.

### Real host foundation (`dd163fe`)

`crates/skit-cli/src/cli/tui_real_host.rs` provides typed `WalkerSeedSpec`, real roots, production seed/reopen, deterministic create clock, profile-local external trees, full canonical `HostObservation`, persistent identity generations, raw document/tree/byte/mode/symlink evidence, and exact low-level transcripts. It has adversarial contracts for paths, actions, modal/preferences state, missing targets, program resolution, uv, launch, editor, dependencies/gates, and fault/rollback cases.

### Generic and real engine (`509cf1f`, `b8e8ce9`)

The generic engine provides ordered operation dispatch, reducer/host draining, error poisoning, exact phase/dispatch/round identities, effect limits, pure parity snapshots, and fresh prefix replay without a Clone host. The private real frontend/host smoke uses true state/session/backend/geometry and different TempDirs for main/replay. Typed handoff contracts require initial Library state and settled Run/reference state.

### Completion containment (`0ce16b8`)

Absent HOME/USERPROFILE now refuses completion installation rather than using passwd/expanduser fallback and writing outside the test sandbox. Do not remove `/home/ubuntu/.local/share/bash-completion/completions/skit`; an earlier faulty test may have created/overwritten it, but prior contents are unknown.

### Phase3c committed (`881a0fc`, tooling follow-up `33d3863`)

This is no longer uncommitted. Verify it at current HEAD rather than relying on pre-reboot hashes.

- Public product-independent `AsciicastRecorder` lives in walker-support.
- The old skit-tui model walker uses the same recorder; no recorder implementation is duplicated.
- Engine FinalLiveness uses a separate phase/dispatch sequence and does not append to successful operations.
- The real smoke drives production Open Run, then real Escape -> Back -> Effect::None back to Library.
- It creates four in-memory schema3 rows: Initial, operation Reducer, operation Host, final-liveness Reducer.
- Reducer, host, session, styled-frame, and geometry objects are canonical/content-addressed.
- Exact event/modifiers, operation index, phase, dispatch sequence, host round, predecessor hash, and Presentation are recorded.
- Only Presented frames enter cast. NotPresented intervals accumulate via `skip`.
- Stored frames rebuild the cast byte-exactly.
- A recorded clone of the raw Action is canonicalized with the persistent PathMap; the raw Action
  still drives the reducer and is not reconstructed from post-reducer state.
- Canonical `LibraryState`, `Action`, and `Effect` are deserialized and replayed through production `update` to prove each emitted effect and next state.
- Complete key/modifier sensitivity, NotApplicable refusal, distinct NotPresented timing, and corruption cases are pinned.

Before commit, two independent reviewers reported 0 BLOCKING / 0 MAJOR / 0 MINOR. Their pre-commit reports listed CLI lib 258 passed/4 ignored, walker-support 49, old model walker 166 passed/4 ignored, private real walker 7, Clippy, Rustdoc, fmt/diff-check, and changed-file hard coverage with no executable misses.

After reboot, the recovered persistent worktree was verified again at `33d3863`. Current focused counts are walker-support 61/61, private real walker 8/8, and old model walker 166 passed/4 ignored. CLI/support all-target/all-feature Clippy, CLI/support Rustdoc `-D warnings`, English contraction checks, rustfmt, and diff-check also pass. The higher support/real-walker counts reflect the final committed contracts that were added after the earlier interim gate report.

`33d3863` adds six entries to `scripts/english_contractions.allow` for the frozen runner/add review copy oracles. Review that commit if copy conventions change.

### Post-handoff successor-agent audit

The successor agent did more than merely commit the pre-handoff diff. Direct inspection of `0ce16b8..33d3863` confirms:

- `881a0fc` changes ten files, with 2,053 insertions and 682 deletions.
- It adds `SeededTuiHost::canonical_action`, which refreshes and uses the host's existing persistent PathMap before serializing a raw Action.
- Current action canonicalization explicitly handles `Action::Present` screens (Run, Settings, Preferences, Add). This is sufficient for the OpenRun smoke, but it is **not yet exhaustive for a 100-operation corpus**. Before cutover, extend typed action projection for every reachable Action that can carry profile paths or volatile identities; add leak/lookalike tests instead of global text replacement.
- It promotes the old private asciicast implementation to walker-support and replaces the old file with a one-line re-export. `ratatui-widgets` is a dev-dependency only, used by recorder tests; production dependency direction stays clean.
- It adds `EnginePhase`, optional operation indices, independent operation/final-liveness dispatch counters, one-shot final liveness, post-liveness operation refusal, and poison contracts.
- It hardens shared timeline semantics, not only the smoke sink. Added contracts reject multi-checkpoint session-only chains, host-first chains, changed event plans, multiple chains for a single-event plan, incomplete click plans at operation boundaries/tail, unsupported input kinds, and invalid liveness dispatches.
- `SchemaThreeSink` remains deliberately in-memory and smoke-specific. It hardcodes the OpenRun operation payload and synthetic final-liveness payload. It does not write manifests/chunks/views/run metadata or support the 100-operation strategy yet.
- The sink verifies canonical object bytes/digests, every referenced object, canonical LibraryState decoding, production reducer transitions, final Library/no-modal state, timeline grammar/semantics, host-effect termination, styled-frame viewport, asciicast validity, and live-versus-rebuilt cast equality.
- Its seven focused tests cover real main/replay equality, full object/timeline/cast projection, corrupt/inconsistent artifacts, distinct NotPresented timing, cause/refusal/bounds projection, and dangling effect shapes.
- `33d3863` is a tooling follow-up, not a new walker feature. It adds exact English-contraction allowlist owners for four runner-management source oracles and two Add workflow assertions so the frozen user-visible copy passes repository tooling without weakening the production strings.

Phase3d already provides the single-profile real-host disk bundle, and G3 now provides the exact
four-profile/100-operation real strategy in memory. G4 must join those owners into one aggregate
transaction. FakeHost migration/deletion and the final installed-corpus model comparison remain.

## Actual current progress

Completed:

- deterministic walker and original agent-review artifact foundation;
- mouse/UI integration and all currently known corpus-discovered fixes;
- production TuiHost and low-level seam migration;
- generic replayable engine;
- real TempDir seed/host observation;
- real engine/frontend/host smoke;
- in-memory real schema3 artifact projection, asciicast, and truthful final liveness;
- Phase3c review and commit;
- Phase3d real-host disk bundle, reviewed and committed as `0945263`;
- Phase4 commit A type-compatible sentinels, reviewed and committed as `8d24dd7`;
- mouse/keyboard run-picker parity regression, reviewed and fixed as `e0a515d`;
- Phase4 commit B deterministic projection helpers, reviewed and committed as `f748a1b`;
- Phase4 commit C0a deterministic real file allocation, reviewed and committed as `e82bae8`.
- Phase4 commit C0b stable retained-handle real sandbox ownership, reviewed and committed as
  `256287d`.
- Phase4 commit C1 atomic host capture, raw production ownership, exhaustive typed projection, and
  Settings wire repair, reviewed and committed as `978aff1`.
- Phase4 commit C2 session/native-path projection, bounded localized text, and persistent
  provenance, reviewed and committed as `bd8eb08`.
- Phase4 commit C3 whole-installed-bundle leak proof, exact renderer evidence, hidden-cell scanning,
  and fresh main/replay byte equality, reviewed and committed as `4c62b73`.
- Phase4 commit D observation-only umask normalization with live source/private provenance, reviewed
  and committed as `1d9a521`.
- Phase4 commit E live frontend locale, truthful locale-save terminal trace, localized saved status,
  and strict dynamic-locale bundle read-back, reviewed and committed as `745826c`.
- Phase4 commit F typed canonical effect-coverage schema, explicit honest-unmeasured state,
  exhaustive typed classifier, and separate served/nested tally, reviewed and committed as
  `22ff2ca`.
- Phase4 G1 strict aggregate manifest/progress codecs, exact profile order, atomic aggregate layout,
  and pure completed-review claim, reviewed and committed as `81d32e7`.
- Phase4 G2a replayable Home/Cwd directory seeds and real editor-write queue, reviewed and committed
  as `b975985`.
- Phase4 G2b live semantic keyboard/mouse/local/raw/resize operation adapter, shared real trace sink,
  and stable main/replay contracts, reviewed and committed as `a34d0ff`.
- Phase4 G3a timeline-derived typed coverage, exact 23/14/8 vocabulary, required-profile merge, and
  stored-summary comparison, reviewed and committed as `151fa7d`.
- Phase4 G2c-b deterministic external file-picker tree, strict seed overlap/payload rules, and stable
  replay, reviewed and committed as `0b4dbf4`.
- Phase4 G2c-a live rendered target inventory with overlay occlusion, exact render freshness,
  filtered picker identity, portable paths, and real resize evidence, reviewed and committed as
  `7899893`.
- Phase4's remaining G2c causal chain: typed `PickedSourcePath`, exact seed-owned host provenance,
  ordered event-chain validation, strict UTF-8 picker targets, and semantic screen resolution,
  committed as `8c25636`, `770bcff`, `29c1f2d`, `aa99dcc`, and `095ef1c`.
- Phase4 G3 derived Add Commit-name projection and the exact 100-operation/four-profile real-host
  corpus, reviewed and committed as `716d4c8` and `14edd22`. It reaches every required 23/14 effect
  name in every profile from completed timeline host requests only.
- G3 review follow-ups: Luna's empty-Paste provenance defect is fixed in `5a81602`; the
  four-profile preflight's compact RunnerEditor navigation regression is fixed in `e24e458`.
- Phase4 H shared canvas maximum, real operation-aware sink forecast, grow/shrink/zero-dimension
  contracts, and retained completed-timeline equality, reviewed and committed as `eb7adcd`.

Still incomplete:

- Phase4 G4 aggregate filesystem transaction, installed byte-only core validator, deterministic
  double generation, review template preservation, and completed-review validator;
- migration of inventory/invariants/driver contracts away from FakeHost;
- deletion of FakeHost and high-level fixtures;
- Terra high versus Luna max review comparison;
- reproduction/fixing of those final findings;
- complete final AGENTS.md gates and PR branch/base update. **The global workspace coverage debt and
  global workspace cargo-mutants debt from earlier commits remain unresolved; see the measured facts
  section. Both must reach zero before delivery.**

Current source evidence confirms FakeHost remains heavily used: about 131 `FakeHost`/`fake_host` references remain under `crates/skit-tui/tests/model_walker*`. The old 100-operation corpus owner remains in `crates/skit-tui/tests/model_walker/driver.rs`. Do not claim migration completion.

## Exact next task: Phase4 G4

**Read `PHASE4-PLAN.md` revision 30 first.** G1 through G3 and H are complete at `eb7adcd`. Do not
reopen them unless a reproducible defect appears.

Implement G4 as one aggregate filesystem transaction. It owns physical `coverage.json`, the
root corpus manifest/run/operations/final-liveness documents, review progress/template files,
BundleLayout changes, and installed read-back. Do not call the single-profile installer four times.
Core read-back must depend only on stored bytes; generation/install leak validation separately
receives the four in-memory `LeakOracleFacts`. Regeneration treats only `review.md` and
`review-progress.json` as mutable and preserves valid existing reviewer content. Profile order is
exact. Measured counts are reconstructed from typed completed timeline host requests and then
reconstructed again from installed bytes; never maintain a counter beside the timeline and never
borrow FakeHost's `trace_coverage`.

## Closed C0b repair history — do not execute

The following C0a/C0b description and old findings are retained only as review provenance. Commit
`256287d` closes them.

C0a added low-level raw allocation for the four proven entropy boundaries: authored drafts, draft
quarantine, injected sources, and launch snapshots. Production stays random. Each fresh real walker
host uses independent deterministic purpose counters. The reviewed launch seam does not accept a
`(File, PathBuf)` pair from an adapter. `FileStore` owns the `.run-` path, `create_new`, file/path
identity, bytes, mode, sync, rollback, lease, and cleanup. The adapter supplies only a typed 32-hex
stem and records the store's attempt outcome. This is committed as `e82bae8`; do not redesign it.

C0b already gives random and stable traces typed metadata, a dedicated sequential stable pair,
schema-2 bundle readback, and the nine-state owner. The remaining work is entirely in the private
filesystem owner and its tests unless a correct handle-relative design requires a small port/type
change. Start with a design review; do not patch around the findings with more path prechecks.

Required filesystem repairs, from two independent reviews:

1. **Replace-disabled Windows rename.** `rename_no_replace` currently uses `std::fs::rename` on
   Windows. That can replace an existing empty directory. Use a safe Windows API that guarantees
   `replace=false`. Direct unsafe code is forbidden by this crate; select a reviewed safe wrapper or
   isolate the platform operation behind an approved safe dependency. Contracts must preserve an
   existing empty directory, file, and non-symlink reparse destination; absent destination must move
   the original and retain its opened identity.
2. **Handle-relative namespace containment.** `validate_namespace` drops the opened `leases/` and
   `sandboxes/` handles, then later creates descendants by absolute path. An ancestor swap can route
   writes outside the namespace. Retain and revalidate namespace, leases, and sandboxes identities
   across descendant create/open/rename/remove, or use handle-relative APIs that cannot follow a
   replaced ancestor. Final-component `NOFOLLOW` is insufficient.
3. **TOCTOU-safe cleanup.** The custom `symlink_metadata` then `read_dir` recursion can descend into
   an external symlink/junction swapped between those calls. Use retained directory handles and
   dirfd/handle-relative operations, or a proven protected remover such as the platform-backed
   `remove_dir_all` algorithm while preserving marker-last semantics. A race hook must swap a checked
   child to an external link/junction; refuse and preserve target, parked original, and replacement.
4. **Bind acquisition identity through destructive use.** Candidate validation currently opens and
   drops another handle. Compare that exact candidate handle with the acquisition identity, perform
   a final/reverse identity check, and pass the retained identity into quarantine/cleanup. A
   byte-valid replacement inserted between identity and candidate checks must fail metadata and
   close without deleting either tree.
5. **Init-lock release check.** Revalidate the retained parent init-lock identity before release. A
   barrier test must replace the lock path with exact valid bytes while the first initializer holds
   the old inode and prove a second initializer cannot enter.
6. **Exact Unix modes and handle chmod.** Compare `mode & 0o7777` against `0600/0700`; reject setuid,
   setgid, and sticky bits. Apply permissions through the opened file/directory handle, not the path.
   A chmod-boundary path replacement must remain unchanged.
7. **Real Windows contracts.** The current reparse test creates a symlink and does not exercise a
   non-symlink junction/reparse point. Add that contract. Add a Windows child-process `process::exit`
   lease-release contract; the current one is Linux-only.
8. **Coverage.** The independent full CLI-lib run found new uncovered host lines 1032, 1068, 1090,
   1285, 1420, 1429, and 1491 at host hash `6ffa8457...`. They cover child refusal, directory sync or
   marker publication, recovery cleanup propagation, and final revalidation. Recompute after the
   redesign and reach zero with honest real fault/race contracts or removal of redundant edges.

The trusted local-content model does not authorize skit to delete or chmod outside its own validated
namespace. These findings are data-integrity and ownership bugs, not a request for command/content
sanitization. Do not weaken stable Windows support or fall back to random mode.

After repairs: rerun host focused tests, full CLI lib, full CLI-lib llvm-cov plus
`scripts/check_coverage.sh`, workspace Clippy/Rustdoc/fmt, Windows CI contracts, and two independent
final FS reviews. Then obtain the missing second wiring review and second legacy-migration review,
freeze the whole source aggregate, commit C0b as one coherent unit, update this handoff, and only
then begin C1.

After C0, C1 adds the settled **host-owned checkpoint capture hook**. The old proposal,
`TraceAction::frontend(&mut PathMap)`, is impossible: the sole persistent `PathMap` lives inside
`SeededTuiHost`, not the frontend action constructor. Preserve these invariants:

- the raw `Action` and raw `Effect` continue to drive the production reducer and host;
- only recorded clones are projected at the checkpoint boundary;
- one persistent, host-owned `PathMap` supplies every state/action/Effect generation;
- do not add `Rc`, a shadow map, or a copied generation table;
- C2 adds the four-locale bounded text policy, session projection, and draft provenance contracts on
  top of commit B's generic full-prefix and full-suffix callback;
- C3 scans and compares the whole installed bundle.

After C, D, E, F, G1 through G3, and H, only G4 remains in Phase4:

- **G4** — four-profile artifacts: `manifest.json` with `CorpusManifest`, `coverage.json`,
  `review-progress.json`, `README.md`, `review.md`, and a completed-review validator.

**The former raw-Effect gap is closed.** C1 projects recorded Action and Effect clones through the
same persistent host-owned `PathMap`, and G3's full Add cleanup tail replays from stored timeline
bytes. Do not reintroduce raw path-bearing Effects or weaken `validate_reducer_action`.

**One unresolved risk to measure before G generates the corpus.** Commit A makes the artifact depend
on drafts having distinct mtimes and turns a collision into a hard refusal. I measured 3000 sequential
create-and-write cycles on this host's ext4 `TMPDIR` and 2000 on tmpfs with zero adjacent mtime
collisions and a 36 microsecond minimum delta, so the assumption holds comfortably here. The walker
CI job runs on `ubuntu-24.04` (`ui-walker.yml:26`), which was not measured. Measure it there before
generating, and treat the result as a precondition for the Add tail rather than discovering it as CI
flake. If granularity is coarse there, enforce separation between draft-creating operations; do not
weaken the refusal.

Then, in order:

1. Complete H and G4 as described above.
2. Migrate `local_inventory`, invariants, parity/liveness probes, and useful fake tests to real or
   lower-level owners.
3. Delete `fake_host.rs`, high-level fixtures, old fake driver owner paths, and live-state clone/fork
   assumptions after contract transfer. The legacy private `RunMetadata` and the legacy
   `validate_live_locales`, `validate_reducer_transitions`, `validate_host_transition_boundaries`,
   `GeometrySnapshot`, and coverage helpers die with it.
4. Generate the final real corpus twice and compare immutable bytes.
5. Run the Terra/Luna comparison, validate their review artifacts, reproduce findings, and fix
   confirmed defects.
6. Drive the pre-existing coverage and mutants debt to zero, run all final gates, then update and
   push PR history and base.

## Closed preconditions for the 100-operation vector

Do not rediscover these. They are in `PHASE4-PLAN.md` with evidence.

- **Agent skill discovery is now reachable.** G2a added replayable Home/Cwd directory seeds, and G3
  uses `.codex/skills` plus `.claude/skills` to reach discover and install from the real host.
- **Do not serve `Action::Quit` in the corpus.** A host response that emits `Effect::Quit` makes the
  engine terminal and prevents the required final-liveness operation. Keep `after_run=stay`, filter
  user Quit, and retain `quit` as F's static-unserved served-effect name.
- **`CountRunGlob` needs a glob character in a run field.** G3 pastes `*` into the real run field.
- **Draft retention needs real editor writes.** G2a added the replayable editor-write queue, and G3
  uses four deterministic writes to reach edit, retained draft, prompt authoring, and prompt edit.
- **The session object is stored unnormalized** (`tui_real_walker.rs:612`), and `source_snapshot` and
  `draft_snapshot` (`skit-tui/src/screens/add.rs:1172`, `:1199`) do emit raw path, mtime, identity and
  `source_record`. I settled that those four arms are **unreachable**: the only producer is
  `add_local_outcome` (`skit-tui/src/session.rs:3610`), which maps `AddScreenEvent`s, and
  `AddScreenEvent::Action(AddAction::SourceInspected | DraftEdited | DraftDeleted | SourceEdited)` has
  **zero** occurrences in the workspace. The advertised variants are genuinely path-free. C3 and G3
  now scan and replay the complete session/timeline surfaces.

## Superseded Phase3d task description

The original Phase3d instructions are kept below for provenance. They are complete.

Start a new bounded Phase3d: write/read a real-host disk bundle from the already-reviewed in-memory schema3 trace.

Requirements:

- Reuse walker-support DTOs, canonical JSON, object validators, timeline validators, and asciicast.
- Do not copy the old roughly 77KB `corpus.rs` into skit-cli.
- Write exact object files, timeline, chunks, profile manifest, readable views, cast, operations, final-liveness request, and run metadata.
- Enforce exact regular-file/directory allowlists, no symlinks, immutable collision comparison, full content digests, and source pre/post fingerprint.
- Read the installed bundle back from disk, validate every reference/digest, rebuild cast from stored Presented frames, and require byte equality.
- Start with the current real OpenRun/final-liveness smoke only. Do not mix the 100-operation cutover into this batch.
- Add refusal/corruption/collision/extra-file/symlink tests and complete executable-line coverage.
- Freeze, independently adversarially review to 0B/0M/0m, then commit Phase3d separately.

After Phase3d:

1. Move the actual local review and 100-operation/four-profile owner into private skit-cli real-host composition.
2. Add targeted preludes for old-vector gaps: Run Submit, CountGlob, SavePreset, HealthRebuild, runner mutations, Preferences Save/Manage/Install, and full Add cleanup tail.
3. Migrate `local_inventory`, invariants, parity/liveness probes, and useful fake tests to real or lower-level owners.
4. Delete `fake_host.rs`, high-level fixtures, old fake driver owner paths, and live-state clone/fork assumptions after contract transfer.
5. Generate the final real corpus twice and compare immutable bytes.
6. Run the Terra/Luna comparison, validate their review artifacts, reproduce findings, and fix confirmed defects.
7. Run all final gates, then update/push PR history and base.

## Four-profile corpus and model review contract

Final profiles:

- `en-80x24`
- `zh-cn-120x30`
- `zh-tw-40x40`
- `pseudo-120x12`

Use one explicit 100-operation semantic vector across all profiles, plus truthful final liveness. Keep every checkpoint and first-appearance object reviewable. `not_presented` rows are diagnostic evidence, not visible frames; keep their objects but exclude their frame from cast and accumulate timing.

When the real bundle is ready, create two byte-identical copies. Spawn exactly:

- Terra `gpt-5.6-terra`, reasoning `high`;
- Luna `gpt-5.6-luna`, reasoning `max`.

Give each the same standalone prompt: inspect every manifest chunk in order, every checkpoint, and every first-appearance reducer/host/session/frame/geometry object; distinguish Presented from NotPresented; record exact profile/sequence/object evidence; write `review.md` and canonical `review-progress.json`; validate the completion locally. Do not leak findings between reviewers.

Compare:

- confirmed unique findings;
- findings both models agree on;
- misses against reproduced defects;
- false positives (especially diagnostic NotPresented frames mistaken for user-visible UI);
- chunk/object completeness;
- localization and keyboard/mouse reasoning;
- ability to use canonical state to explain rendered output;
- time/cost/working style.

Do not add this workflow to CI or GitHub Actions.

## Build and disk safety

Set only `TMPDIR`:

```bash
mkdir -p /home/ubuntu/coding/skit/.tmp
export TMPDIR=/home/ubuntu/coding/skit/.tmp
```

**Do not set `CARGO_TARGET_DIR`.** An earlier revision of this file told you to point it at
`/home/ubuntu/coding/skit/target`. That is now wrong and expensive: the worktree-local
`/home/ubuntu/coding/skit-uiwalker-mouse-integration/target` is warm, and redirecting Cargo rebuilds
the whole workspace.

Do not create a durable worktree or primary source under `/tmp`. TempDirs created by tests are fine.

The user declared the storage side quest complete and said the available space is enough. Do not
monitor, audit, or clean storage unless the user asks again. If cleanup is explicitly resumed, touch
only rebuildable Cargo artifacts after confirming no Cargo or rustc process is running. Preserve the
warm integration target, parity evidence, source, corpus, and review data.

Useful focused checks:

```bash
cargo test --locked -p skit-cli-rs --lib cli::tui_real_walker
cargo test --locked -p skit-cli-rs --lib cli::tui_walker_bundle
cargo test --locked -p skit-tui-walker-support --lib
cargo test --locked -p skit-tui --tests
cargo test --locked -p skit-tui-walker-model --tests
cargo clippy --locked -p skit-cli-rs -p skit-tui-walker-support -p skit-tui --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked -p skit-cli-rs -p skit-tui-walker-support --all-features --no-deps
cargo fmt --all --check
bash scripts/check_english.sh
git diff --check
```

Do not run `cargo fmt --all` while another agent owns a file in a different crate; use
`cargo fmt -p <package>`.

For coverage, use cargo-llvm-cov and `scripts/check_coverage.sh`. Hard executable-line results are authoritative; a high percentage alone is not sufficient.

Before final delivery, run the complete commands in AGENTS.md, including workspace tests, Clippy, Rustdoc, coverage, cargo-mutants, deny/audit, workflow checks, docs, packaging, and benchmarks as applicable.

## Reboot recovery facts

- The erased `/tmp` worktree was only a checkout; its committed objects and branch ref lived in the persistent main `.git` database.
- Git preserves all named implementation commits through current HEAD `e82bae8`.
- The pre-reboot uncommitted repo-root handoff was lost, so this file was reconstructed from Git history, source inspection, PR metadata, and the surviving older handoff.
- Do not assume undocumented conversational details. If a future decision cannot be proven from this file, Git, tests, or source, mark it as unknown and ask the user instead of guessing.

## Final reminder

The goal is not merely to make a walker test pass. The goal is a trustworthy observation of the real product, no duplicate behavior model, a deterministic inspectable corpus, and an empirical Terra-versus-Luna review comparison on identical evidence.
