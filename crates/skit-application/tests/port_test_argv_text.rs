//! Exact port of Python v0.4 `tests/test_argv_text.py` at
//! `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//!
//! Keep the separator-only tail cases exact: accepting a phantom empty argument would change the
//! executable argv even though ordinary round-trip tests can still look correct.

use skit_application::runner_management::{EditableArgvDialect, split_editable_argv};

#[test]
fn test_windows_split_ignores_separator_only_tail() {
    assert_eq!(
        split_editable_argv(" \t ", EditableArgvDialect::Windows).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        split_editable_argv("agent.exe \t ", EditableArgvDialect::Windows).unwrap(),
        ["agent.exe"]
    );
}
