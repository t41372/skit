//! Mechanical port of the Python oracle module `tests/test_argv_text.py`
//! (`origin/main@206f9ef`): "Platform-aware editable argv text behavior." The
//! oracle module round-trips argv through the TUI's one-line, no-shell command
//! fields and has exactly one `def test_*`.
//!
//! Concept mapping:
//! - Python `argv_text.split(command)` with `sys.platform == "win32"`
//!   (monkeypatched in the test) -> `split_editable_argv(command,
//!   EditableArgvDialect::Windows)` in skit-application, the frontend-neutral
//!   home of the editable-argv logic. The Python module reads the live
//!   `sys.platform`; the Rust API takes the convention as an explicit `dialect`
//!   argument, so the oracle's `monkeypatch.setattr(argv_text, "sys", ...)`
//!   becomes the `EditableArgvDialect::Windows` value. That parameter is a
//!   runtime match arm, not a `#[cfg]`, so this Windows-branch test runs on a
//!   Linux host too (unlike skit-cli's private `split_windows_arguments`
//!   mirror, which is `#[cfg(any(test, target_os = "windows"))]`).
//! - Python `_split_windows` CRT state machine -> the
//!   `EditableArgvDialect::Windows` arm of `split_editable_argv`, backed by the
//!   `windows-args` crate.
//!
//! Buckets: 1 test, API EXISTS -> real asserting `#[test]`. No cross-crate, no
//! absent gap.

use skit_application::runner_management::{EditableArgvDialect, split_editable_argv};

#[test]
fn test_windows_split_ignores_separator_only_tail() {
    // The editable command field is only an argv representation; no shell runs it.
    // Under the Windows convention, a separator-only line yields no tokens, and
    // trailing spaces/tabs after a token add no empty argument.
    assert_eq!(
        split_editable_argv(" \t ", EditableArgvDialect::Windows).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        split_editable_argv("agent.exe \t ", EditableArgvDialect::Windows).unwrap(),
        ["agent.exe"]
    );
}
