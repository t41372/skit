//! Exact port of Python v0.4 `tests/test_argv_text.py`.

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
