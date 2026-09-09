# Source review against the Rust branch

Date: 2026-09-08.

The user requested a source review against `rewrite/rust-ratatui-complete-20260808-codex`.
The resolved base was `5af68f8f7a796e4483d3c9900db90d3939b4cb56`.
Seven read-only review agents examined the complete change in separate scopes. The parent
checked each finding against callers, contracts, and the local application trust model.
CodeRabbit was not used.

## Reviewed source and history

The agents used immutable tip `19f3bbb085a188524f853e75468dcc45b50ed215`.
Its tree was `1afb67b5c2736db48a471e225932cca24dd0d65d`.
The first split produced `ef756e7c3a110454edb36d19cba30df49b609404` with that exact tree.
It changed six commits into thirteen commits. Seven commits now describe the product fixes
that had been grouped with mouse and host changes. The review target contained 133 changed files,
including the mouse work and all changes after `fix/tui-mouse`.

The repair set below is folded into the related commits. The branch still targets Rust directly.
Local backup refs retain both earlier histories. No Rust or mouse branch was changed.
The ignored `target/history-rewrite-20260908/` directory holds the commit and tree maps.

## Findings and decisions

| Priority | Verified defect | Repair and evidence |
| --- | --- | --- |
| P1 | A Windows launch snapshot kept write-data handles while a child read it. Readers that permit only shared reads can fail. | Finish the write, retain the existing attribute-only cleanup handle, compare its identity, then release both write-data handles. Add a native Windows shared-reader contract. |
| P2 | Mouse selection changed a Run picker value but kept its dropdown open. | Close local picker state when the option click completes. The public session test failed before repair for the already-selected option. It now checks both options and keyboard/mouse paths. |
| P2 | A second Language anchor click did not close its picker. A panel border could reach a covered control. | Toggle the anchor and let the open panel own dismissal. Place text cursors only after target resolution. Keep clicks on visible controls outside the panel available. The anchor, panel, and blank-area tests failed before repair. |
| P2 | The corpus generator sampled its first source fingerprint after main recording. | Capture the source before recording. Compare it before and after each install, including replay. A Linux regression reproduced the wrong successful publication, then passed for changes during both recording phases. |
| P3 | The recording editor reported success before an actual queued write failed. | Execute the operation first. Record and return the same result. A filesystem-error regression failed before repair and passed after it. |
| P3 | Library ages used a timestamp captured before a blocking script run. | Sample the existing injected run clock for the completion surface. The test failed before repair. Submit and rerun now each advance the clock and check another entry's displayed age. |

The Windows finding follows the production path from copy preparation to `pwsh -File`.
[PowerShell reads the script with a three-argument FileStream constructor](https://raw.githubusercontent.com/PowerShell/PowerShell/master/src/System.Management.Automation/engine/ExternalScriptInfo.cs).
[That constructor uses FileShare.Read](https://raw.githubusercontent.com/dotnet/runtime/main/src/libraries/System.Private.CoreLib/src/System/IO/FileStream.cs).
The new test uses the same sharing contract. Native execution is checked by CI, not by the Mac
cross-compilation result. Unix direct executable entries use their recorded source and do not
create this snapshot, so the proposed Unix executable failure was rejected.

## Review coverage and limits

The seven scopes were TUI core/session/input; screen adapters/UI reducers/i18n; production
CLI/runtime/store; walker support; real host/filesystem; corpus recording/publication; and
model/random walk/CI/tooling. Each scope traced related callers and tests. The parent also
checked history, design contracts, and the combined repair set. The model/CI reviewer initially
omitted two benchmark source files, then examined both and corrected the coverage statement.

No additional defect was established in the support or model scopes. Generic timeline checks
are intentionally completed by typed product validation. They do not need another validation
layer. Manually sampled liveness still requires the same `SKIT_WALKER_LIVENESS_EVERY` on replay;
no actual failure was established, and this was not promoted into a repair requirement.

The old Terra/Luna corpus reports remain historical UI evidence. They are not whole-branch
source reviews. Their source hashes and saved artifacts have not been changed or relabeled.
This source review also does not replace native tests or the mutation gate.

## Validation

The new history boundaries passed workspace compilation. The foundation also passed its CLI,
TUI, and UI library tests. Formatting changes at intermediate boundaries preserve the final tree.
The repair set passed the focused TUI, CLI, and store tests. The new source-change test failed,
then passed on the local Linux VM. Windows store tests passed cross-compilation.

The final Mac workspace run passed 5,175 tests across 265 test targets. It failed no tests
and ignored 534 tests under the suite configuration. Mac Clippy passed with warnings denied.
The [first native CI run](https://github.com/t41372/skit/actions/runs/34227719971) passed
Linux, macOS, and Windows tests, packaging, and the dependency/workflow audit. It found a Clippy
style error and two uncovered failure branches in the new tests. Those tests now use
direct assertions and the existing control-area query. Focused LLVM coverage reports
no zero-count lines in any of the three corrected test functions. Full Linux Clippy,
Rustdoc with warnings denied, and the English contract check passed. The native Windows
log confirms both the new shared-reader test and the existing readonly cleanup test
passed. The [final complete CI run](https://github.com/t41372/skit/actions/runs/34230692345)
passed all seven jobs for source `a47d874b5da3e94a22d9092b5e50f7f289e54cf5`. This includes
Linux, macOS, Windows, 100% executable-source line coverage, format/Clippy/Rustdoc/i18n,
dependency/workflow audit, and PyPI/uv compatibility. The final follow-up commit updates
only this report and the handoff; it does not change the tested source.
The complete mutation gate remains unrun and is not waived. Its existing budget and timeout
question remains separate from this source review. No merge has been performed.
