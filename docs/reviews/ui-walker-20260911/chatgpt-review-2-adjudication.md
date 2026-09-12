# Second ChatGPT review adjudication (2026-09-11)

Reviewed source: `49e9491158eefb74c8b3c33d0ade543e6d79d914` (the report's fixed target).
Base: `5af68f8f7a796e4483d3c9900db90d3939b4cb56`.
Input: `skit-ui-walker-deep-review-2026-09-11 (1).md`, SHA-256
`e7c0ec0f20d7cdc23476e1098deecef2e2c3bf052665e32152d578f373a7f6aa`.
The input report remains unchanged and local. Its severities and proposed fixes are not requirements.

## Decisions

| Finding | Independent decision | Action |
| --- | --- | --- |
| F1: focus reporting stays on while a host effect owns the terminal | Valid. The defect is real, reachable on the primary product path, and this branch introduced it. The 2026-09-08 adjudication (F8) enabled focus reporting at the terminal claim and disabled it at the final restore. The suspend before a host effect and the resume after it kept the older two-mode set (alternate screen, mouse capture). Focus reporting is not part of the alternate screen, so a script rerun or an editor launched from the TUI received `CSI I` and `CSI O` as input on every window switch. The base branch had no focus reporting. | One shared command set for the four transitions (`enter_screen_modes`, `leave_screen_modes` in `crates/skit-tui/src/terminal.rs`). A PTY test in `crates/skit-tui/tests/terminal_pty.rs` fixes the order: the alternate screen, mouse capture, and focus reporting are all off before the host effect writes its first byte and all on again after it returns. A unit test pins the command set. See the commit below. |
| F2: the status line guesses between a catalog key and rendered text | Valid as a type observation, not a defect of this branch. `LibraryState.status` is a `String` in the base as well, and the base localized it with `render`, a fragment replacement over the whole string. This branch narrowed that to "translate a complete catalog key once" (`2d090bbb`), which repaired receipts that carried user values and pseudo-locale text. A wrong translation needs a non-English locale and a host receipt that equals an English catalog key; host receipts in those locales are already translated, so only a receipt made of one user value could collide. Every host producer was enumerated: six pass a complete catalog key (`Entry added`, `Source saved`, `Entry removed`, `Preferences saved`, `Prompt runners saved`, `Entry renamed`), and the others pass a localized template with its values (`format_text`, `text`, `localize`) or a composed receipt that starts with a localized key. None passes a bare user value. The residual risk is a new reducer status literal that is missing from the catalog: it renders in English. The catalog tests cover named lists, not a scan of reducer literals. | Deferred. A typed status changes the serialized `LibraryState` that the walker invariants round-trip and every producer in three crates. That is not proportionate to a finding with no reachable instance. Recorded here for the next i18n change. |
| A1: walker introspection is in the normal public surface of `skit-tui` | Fact, and a documented trade-off. `MIGRATION-DESIGN.md` moved the oracle into a dev-only crate so that mutation testing compiles it; the public seam is the price. Only `skit-benchmarks`, `skit-tui-walker-model`, and `skit-tui-walker-support` carry `publish = false`. No workflow or script runs `cargo publish`. The product ships as a PyPI wheel. `publish = false` on `skit-tui` alone would also make `skit-cli-rs` unpublishable, because a published crate needs published dependencies, so the honest form of that decision covers every product crate. | No change. The decision (never publish to crates.io, or gate the seam behind a non-default feature before a first publish) is the owner's and is recorded as open in `HANDOFF.md`. |
| A2: the parity inventory cannot prove that every visible control is registered | Correct statement of the guarantee boundary. The probes prove parity for the targets that the live inventory holds. A control drawn without a hit region is absent from the geometry, the inventory, and every parity loop. The state-specific inventory tests hold that side today. | The boundary is now written into `MIGRATION-DESIGN.md` next to the `parity` family. The report's "mutation probe" (remove one hit registration, expect a failing test) is what the unrun workspace mutation gate provides in part. |
| Section 9.2: no dedicated walker workflow run on the current head | Fact. `ui-walker.yml` ran on the old feature branch in August and never on this integration branch. | Dispatched on the fixed head; see the CI record below. |
| Sections 6, 7, 8, 10 | No finding. The report's rejections match the earlier adjudications. | None. |

## F1 in detail

The four transitions were:

| Transition | Before | After |
| --- | --- | --- |
| claim | alternate screen, mouse, focus | `enter_screen_modes` |
| suspend before a host effect | alternate screen, mouse | `leave_screen_modes` |
| resume after a host effect | alternate screen, mouse | `enter_screen_modes` |
| final restore | alternate screen, mouse, focus | `leave_screen_modes` |

The helpers share the command set only. The claim keeps its rollback, the restore keeps its
"attempt both, keep the first error" rule, and the suspend and resume keep their sequential rule.
The command order inside each set is unchanged, so the existing shutdown assertions in
`crates/skit-cli/tests/terminal_pty.rs` and the recorded demos see the same bytes.

The report asked for a child that reads stdin while the test injects a focus report. That tests the
terminal emulator's side of the protocol. Our side is the order of the mode toggles around the host
effect, and the PTY test asserts that order on the real output bytes.

## Independent review of this change

An Opus reviewer read the frozen commit in a scratch worktree and verified every finding before
reporting: one blocker and five nits, all repaired before the commit was rebuilt.

- Blocker: the new PTY test was flaky (two failures in twelve Linux runs, four in fifty-five on
  macOS). Its second wait ended at the `?1049h` of the resume, but the resume then calls
  `Terminal::clear`, and ratatui asks the terminal for the cursor position there. When that
  question arrived after the wait had ended, nothing answered it, crossterm gave up after two
  seconds, and the child exited with a panic. The wait now ends at the resumed library frame, which
  the child can draw only after the answer. Thirty consecutive Linux runs and five macOS runs pass.
- Nit: both leave paths turned raw mode off before the mode toggles went out, so a cooked terminal
  could echo a mouse or focus report in that window. The leave order is now the reverse of the
  enter order: screen modes off, then raw mode. The restore keeps its "attempt both, keep the
  first error" rule.
- Nit: a doc comment still said the transition callback leaves and re-enters the alternate screen.
- Three wording nits in comments (one term for the read position, a lint named as a compiler
  refusal, two figurative phrases).

The reviewer also confirmed by hand-applied mutants that the unit test and the PTY test each kill
an emptied `enter_screen_modes` or `leave_screen_modes`, and that only the PTY test catches the two
helpers swapped at their call sites.

## Validation

The fix is commit `3edd8058` on top of `6e27ce30`.

- The new PTY test failed against the unfixed code with the message that `?1004l` was missing
  before the host effect, and passes after the fix.
- Linux: `cargo fmt --check`, workspace Clippy with warnings denied, workspace Rustdoc with
  warnings denied, every `skit-tui` target, thirty consecutive runs of the `skit-tui` PTY tests,
  the `skit-cli-rs` PTY tests (41), and `scripts/check_english.sh` pass.
- macOS: Clippy for `skit-tui` and `skit-cli-rs`, Rustdoc for `skit-tui`, the terminal unit tests
  (23), and five runs of the PTY tests pass.
- Windows: `cargo xwin check --locked --target x86_64-pc-windows-msvc -p skit-tui --all-targets`
  reports zero errors and no warning in `skit-tui`; the one warning is the pre-existing
  `skit-store` one.
- The existing shutdown assertions in `crates/skit-cli/tests/terminal_pty.rs` see the same bytes.
  The walker corpus does not record terminal control bytes, so no corpus changes.

## CI record

Head `b57cdcef` (the fix plus the first docs commit):

- `ui-walker.yml`, run 34660689495: success. This is the first run of the dedicated walker
  workflow on this branch (the earlier runs were on the August feature branch).
- `ci.yml`, run 34660687050: six jobs passed; the coverage job failed twice for two different
  reasons, both repaired below.
  - First attempt: `different_profiles_can_initialize_and_run_in_parallel` failed with "could not
    open directory entry ... sandboxes/.cleanup-parallel-b: No such file or directory". The Linux
    ticket scan lists a directory and then opens every entry by name; the shared `sandboxes/`
    directory holds every profile's transient cleanup directory, so one profile's verification
    scan opened a sibling's cleanup directory at the moment the sibling removed it. That is a race
    in the walker's Linux sandbox scan, not in the focus fix. Repaired in `201ba830`: an entry that
    is absent when it is opened gets no ticket; every other open failure stays an error. The
    Windows scan reads identities from the listing and has no such window. An Opus reviewer
    verified that commit (zero blockers, four nits, all repaired): no absence or presence
    postcondition of the scan's callers is weakened, the `NotFound` of the open cannot come from a
    dangling link because the open uses `O_PATH` with `O_NOFOLLOW`, and every hand-applied mutant
    of `present_tickets` and `is_not_found` is killed by the injected-open test. A second test pins
    the real `NotFound` mapping of `ticket_for_name`. `cargo mutants` skips this `#[cfg(test)]`
    module, so the mutant analysis is by hand.
  - Rerun of the failed job: the race did not fire, and the gate reported four uncovered lines in
    the new unit test of `terminal.rs`. The assertion messages built their text on separate lines
    that run only on failure. Repaired in `3e6042ae`: the test converts the bytes once and uses
    inline captures.

Head `201ba830` (the focus fix, the two repairs above, and the docs before them):

- `ci.yml`, run 34663434541: success, all seven jobs (format, lint, and documentation; Ubuntu,
  macOS, and Windows tests; the complete line coverage gate; dependency and workflow audit; PyPI
  and uv tool compatibility). <https://github.com/t41372/skit/actions/runs/34663434541>
- `ui-walker.yml`, run 34663435759: success.
  <https://github.com/t41372/skit/actions/runs/34663435759>

The docs commit on top of `201ba830` adds this record and the checkpoint in `HANDOFF.md`.
