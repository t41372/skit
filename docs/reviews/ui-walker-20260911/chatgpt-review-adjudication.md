# ChatGPT review adjudication (2026-09-11)

Reviewed source: `42f9767f71640d3554ab59cc92590fa2dcf3ecbf`.
Base: `5af68f8f7a796e4483d3c9900db90d3939b4cb56`.
Input: `skit-ui-walker-deep-review-2026-09-11.md`, SHA-256
`9a3420055fa9fedb278d0923ed8ac9149cda75787d00a44a05ee55d50fb3094a`.
The input report remains unchanged and local. Its severities and proposed fixes are not requirements.

## Decisions

| Finding | Independent decision | Action |
| --- | --- | --- |
| F1: invalid walker numbers fail open | Valid, low severity. The fail-open reader was a documented design choice (`MIGRATION-DESIGN.md`: branch-free readers, shell guards refuse zero). Two facts still justify the repair. `selected` already fails closed, so the module was inconsistent. The migration ledger records that the legacy driver refused zero, so the migration weakened that contract without a decision. The workflow shell guards never covered a local run. | A present value of `SKIT_WALKER_CASES` or `SKIT_WALKER_STEPS` that is not a positive integer stops the walk with a message that names the variable and the value. `SKIT_WALKER_LIVENESS_EVERY` keeps `0` as "never" and refuses a non-integer. The default-value tests run in a child process with every walker variable removed. Child-process negative tests cover each refusal. The design record is updated. The shell guards remain as an early check. |
| Q1: the full mutation gate is unrun | Fact, not a code defect. | No action in this change. The 64-shard run is the user's budget decision; see `HANDOFF.md`. |
| M1: test modules are too large | Valid for `tui_real_host.rs` (23,610 lines). The other three walker files (8,578, 6,898, and 4,093 lines) are inside the range the Rust base already carries: `cli/tests.rs` has 12,531 lines, `cli.rs` 11,843, and `skit-tui/src/session.rs` 7,722. | Split `tui_real_host.rs` and `tui_real_walker.rs` into topic files. Move code only. Keep `tui_walker_bundle.rs` (its tests are interleaved with the code they test, and it is smaller than the base's largest files) and `tui_real_sandbox_fs.rs`. |
| Section 7: oracle boundary | Agreed. The limit is already documented in `G4-DESIGN.md` and the contract matrix. | None. |
| Sections 8 to 12 | No new finding. | None. |

## The split

A script cut each source file into items (one function, type, impl block, or use statement with
its comments and attributes), assigned every item to a topic file, and wrote the files. Before
rustfmt ran, the script proved that every non-blank line of the source appears exactly once in the
output. The proof ignores four documented differences: visibility qualifiers, `use` and `mod`
statements (regenerated per file), the path rewrites that the new module tree needs, and one
`include_bytes!` path that moved two directories deeper. rustfmt then reflowed about one hundred
lines at the shallower indentation, so the committed files differ from the source at line level but
not at token level; an independent reviewer confirmed the token-level equality.

Rules:

- An item that was `pub(super)` in the old module (visible in `cli`) is `pub(crate)` now, because
  `pub(super)` inside a submodule would mean the parent only. The parent module re-exports the
  names that sibling modules import, with a `#[cfg]` on re-exports of gated items.
- A private item or struct field that tests or sibling files read is `pub(super)` now. Every such
  widening is the honest record of a test that reads internals; the counts are below.
- Test files whose every item is platform-gated carry the same `#[cfg]` on their `mod` line, so a
  macOS build does not warn about an unused `use super::*`.
- Inside tests, `super::X` became `X` with an explicit import in `tests.rs`, and `super::super::`
  became `crate::cli::`. Inside implementation files, `super::` became `crate::cli::`.
- The four child-process tests used to name themselves with a `concat!` literal that would have
  gone stale on a move. A stale name makes libtest run zero tests in the child, and one of the four
  parents checked only the exit status. They now build the name from `module_path!()` through
  `harness_test_name` in `tui_real_host/tests.rs`.

Result:

| Module | Before | After |
| --- | ---: | --- |
| `tui_real_host` | 23,610 lines in one file | a 29-line parent, nine implementation files (largest `stable_namespace.rs`, 1,293 lines), `tests.rs` with shared fixtures (797 lines), and eighteen topic test files (largest `tests/stable_sandbox.rs`, 1,601 lines) |
| `tui_real_walker` | 8,578 lines in one file | a 14-line parent, five implementation files (largest `corpus.rs`, 1,526 lines), `tests.rs` (246 lines), and four topic test files (largest `tests/corpus.rs`, 1,159 lines) |

Visibility and path changes that the proof excludes:

| Module | Items widened to `pub(super)` | Struct fields widened | Former `pub(super)` items now `pub(crate)` without a re-export | Path rewrites | Include paths re-based |
| --- | ---: | ---: | ---: | ---: | ---: |
| `tui_real_host` | 255 | 88 | 4 | 705 | 1 |
| `tui_real_walker` | 97 | 58 | 16 | 7 | 0 |

The largest files in the workspace are now `cli/tests.rs` (12,531), `cli.rs` (11,843),
`session.rs` (7,722), `tui_walker_bundle.rs` (6,898), and `screens/add.rs` (5,131). Only
`tui_walker_bundle.rs` comes from this branch.

Known loss: one comment that sat next to a `use` statement in the old module header
(`// Only the stable-marker contract reads the sandbox roots.`) was not carried, because import
blocks are regenerated per file; the platform condition it explained is now the `#[cfg]` attribute
on the import itself. The other two import comments moved with their imports.

## Independent review of this change

An Opus reviewer read the frozen change in a scratch worktree and verified every finding before
reporting. Verdict: zero blockers, one major, four nits. All five were repaired and the commits
were rebuilt:

- Major: two generated imports in `tui_real_host/tests.rs` carried a `#[cfg]` broader than their
  use sites, so a Windows build warned about unused imports. The imports now carry the exact
  predicates (`unix`, and `target_os = "linux"`). The eight warnings that the Windows cross-check
  still reports inside these modules are the pre-existing ones of the old file.
- A comment that rustfmt had sorted away from its import is grouped with it again.
- Two references to `tui_real_walker.rs` now name `tui_real_walker/corpus.rs`.
- The liveness refusal now says "zero or a positive integer", which is true for a negative value.
- `positive` keeps the assertion that a zero default is a caller defect, with its own test.

The reviewer also confirmed the token-level move-only claim (host 741 items before and 742 after,
the one addition being `harness_test_name`; walker 336 both times) and that the enabled test sets
are identical on every platform (macOS 150 and 41, Linux 210 and 59, Windows 154 and 59).

## Validation

- macOS: `cargo check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  pass. The macOS-compilable walker and host tests pass (see the checkpoint in `HANDOFF.md`).
- Linux: workspace Clippy with warnings denied, Rustdoc with warnings denied, `cargo fmt --check`,
  the model crate suite, and the complete `skit-cli-rs` library suite pass; see `HANDOFF.md` for
  the counts. Each of the four child-process tests spawned a child that ran exactly one test.
- Windows: `cargo xwin check --locked --target x86_64-pc-windows-msvc --workspace --all-targets`
  reports zero errors. The warning set is the pre-existing one.
- F1 in the model crate: 149 tests pass, `scripts/check_coverage.sh` reports complete
  executable-source line coverage for the crate, and `cargo mutants --no-config` over `artifacts.rs`
  reports 16 caught, 6 unviable, and zero missed.
- The existing corpus identities bind the old tree. A moved-file tree has a new source identity.
  No corpus was re-recorded and none is retagged; a move-only change cannot change the recorded
  bytes.
