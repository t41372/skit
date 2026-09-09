# ChatGPT review adjudication

Reviewed source: `501be70a204aafb8b60253acd36bb633139fa75f`.
Base: `5af68f8f7a796e4483d3c9900db90d3939b4cb56`.
Input: `skit-ui-walker-deep-review-redone.md`, SHA-256
`6aee92ef96bf23c03181575e06c0f6c01be0f6c24c592f168f67b48ef5227563`.
The input report remains unchanged. Its proposed severity and fixes are not requirements.

## Decisions

| Finding | Independent decision | Action |
| --- | --- | --- |
| F1: normal copy launch identity | Valid compatibility defect in the Rust base. Normal and raw launches change the filename and self path. Adjacent resource lookup still works because the snapshot is in the same directory. | Restore the stored path. Keep the existing launch removal lease, short claim check, source hash, and post-run state transactions. Do not hold a content lock for the child's lifetime. |
| F2: raw mouse becomes a click | The main claim is false. `resolve_corpus_operation` already returns exactly one raw event. Only the Quit probe synthesizes a click. That probe can skip an inert press or an unarmed release. | Probe the same single event on the session fork. Keep complete clicks for semantic targets. |
| F3: inventory errors disappear | Valid test-oracle defect. Legal screens already return an empty inventory. A stale render or invalid picker is an error. | Return `Result` from `live_inventory` and propagate it from the random walk. |
| F4: a click survives other input | Valid interaction defect. A Help press, Tab, then release still opens Help. | Cancel existing click trackers at the common non-pointer input boundary. Keep the current drag cancellation rule. No gesture generation or extra owner abstraction is needed. |
| F5: configured steps are only a maximum | Valid exploration-budget defect. With 100 configured steps, eight initial cases drew 70, 62, 58, 63, 66, 13, 34, and 87 operations. | Generate exactly the configured number. Keep the existing bounded value shrinking on the failed profile. |
| F6: successful replay accepts drift | Valid replay defect. Success accepts extra resolutions and changed skip records. | Compare complete success records. For a recorded failure, compare resolutions and skip decisions through the old failure boundary. Keep the existing error comparison. |
| F7: infrastructure cost and threat model | Partly valid maintenance concern; the report does not justify its proposed rewrite. The sandbox is test-only. Stable paths preserve exact renderer cells. Leases and markers prevent test collisions and wrong-root cleanup. | Stop adding hostile-user scenarios. Remove two proven duplicate directory syncs. Keep the current owned-root cleanup and distinct per-profile determinism checks. |
| F8: focus events are not enabled | Valid for ANSI terminals. The native Windows backend already reports focus. | Enable focus reporting when the terminal is claimed and disable it when restored. Focus changes use the same gesture cancellation policy. |
| F9: forwarding wrappers | Three `Borrowed*` wrappers have no behavior. `PreviewProbe` does have behavior and must remain. | Let runtime helpers accept trait objects with `?Sized`; remove the three wrappers. |

## Contract choices

The launcher runs trusted user code. Ordinary launch must retain the stored script path, as version
0.4 did. An injected source can still use a temporary path because it has different bytes. The
existing warning and injection behavior remain. A snapshot is not a write transaction. Removing
it does not remove the source hash check, atomic source writes, identity checks, or completed-run
state coordination. A long content lock would block source edits and could deadlock a child that
calls skit to update its own entry.

The raw mouse report traced the Quit probe but missed the actual dispatch vector. Raw Down, Drag,
Key, and Up sequences were already possible away from the over-filtered Quit chip. The repair
aligns the probe with dispatch; it does not introduce a new event model or schema.

Replay files already contain `resolved`, `skipped`, and `error`. Exact success comparison needs no
schema change. Existing failure descriptions remain useful after a repair. Each resolution or skip
consumes one drawn operation, so their recorded counts define the comparison boundary. Value
shrinking remains bounded by the existing eight-step budget. Fixed-length generation does not
promise length minimization.

The mutation workflow's 64 shards, six-hour ceiling, and 136-second relink comment are unchanged
from the Rust base. They are not a cost introduced by this walker. Corpus generation times measure
the complete host/render/record/validate path; the report provides no profile that attributes them
to the filesystem or path scanner. The four path checks carry different profile facts and are not
four identical scans. Reviewer-authored text is not sanitized by this change.

The snapshot allocator's `record_attempt` callback had a real purpose while the store owned file
creation: an allocator that supplies only a name cannot observe the store's allocation result.
Removing just the callback would move ownership or add another observer. Its removal is justified
only with removal of the normal snapshot producer. This is a consequence of F1, not an independent
reason to redesign the host interfaces.

## Validation

The focused checks first reproduced the Quit probe, stale inventory, interrupted gesture, step
count, success replay, and missing terminal focus reporting defects. The repaired walker and
random-walk suites pass. The terminal test checks native PTY enable/restore bytes. Full TUI and
model suites pass. Runtime trait-object contracts first failed to compile with `Sized`; the full
runtime suite then passed with the relaxed bounds.

The launch repair passes the six identity and concurrency tests, 391 store tests, 40 TuiHost tests,
147 RealHost tests, and 11 run-command tests. Store and CLI Clippy and Rustdoc pass with warnings
denied. The Windows store test cross-check passes. Shell and JavaScript had valid pre-fix failures;
the first Python failure was an invalid test adapter, so it is not counted as product red evidence.
The corrected Python contract runs a real interpreter through a local uv adapter and passes.

The complete Mac workspace ran 264 test targets and found two old assertions that required a
snapshot filename. Both were corrected. Six affected fixture targets then passed all 199 tests.
Injection cleanup tests now check the actual `.injected-*` prefix and exercise an injected value.
The product source did not change during this fixture correction.

The four-profile corpus passed recording, fresh replay, installation, and validation in 1115.03
seconds. Its directory is `corpus-60a1a885045ccd40758242fb3cfe6cd9bb958a23696bc781e1ae27980155634e`.
It contains the same 100-operation vector. Its source is
`501be70a204aafb8b60253acd36bb633139fa75f+worktree:d179b54acb7ac046cb52af9428f69cd4ce0b6843e23eb069034955b04744b7f0`.
All 177 crate source and Cargo files matched the first CI head, `3db88637`. The archive and source
patch are kept in the ignored `target/uiwalker-evidence/` directory. This corpus is not retagged.

The first native CI found a lost `cfg(unix)` on an adjacent permissions test. It is restored, and
the complete Windows workspace cross-check passes. Linux tests passed; coverage found a duplicate
guard in a path-registration helper whose only caller already makes the check. The helper is now
inlined into that caller. The complete RealHost suite and CLI Clippy pass after that simplification.

A Linux PTY test also sent a manual cursor reply before the second terminal session was ready.
Its CI output shows the reply echoed before the next cursor query. Two tests with this sequence
now use the existing live driver and wait for the review screen before Save. All 41 native Mac PTY
tests pass. No timeout or product behavior changed for this fixture repair.

These follow-ups do not change the corpus operation vector or projection result. They restore a
test platform condition, remove a redundant call, and synchronize test input. Complete Linux
executable-source coverage now passes at 100%. The full Windows workspace cross-check passes
with the final PTY fixtures. Final native CI passed all seven jobs for `7cbac127`:
https://github.com/t41372/skit/actions/runs/34293517714 . This includes Windows, Linux, macOS,
100% executable-source coverage, quality checks, packaging, and the dependency/workflow audit.
The final amendment updates only this report and `HANDOFF.md`; the tested code is unchanged.
Full workspace
mutation testing remains unrun and is not represented as zero survivors. The old corpus and its
review reports remain evidence for their original source identities.
