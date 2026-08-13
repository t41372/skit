//! Language-owned ports from Python v0.4 `tests/test_store.py`.

use std::path::Path;

use skit_language::{infer_kind, suggest_description};

#[test]
fn test_extract_comment_description_first_comment_line_wins() {
    let text = b"#!/bin/bash\n# Ship the current build\n# more\necho hi\n";
    assert_eq!(suggest_description("shell", text), "Ship the current build");
}

#[test]
fn test_extract_comment_description_skips_shebang_and_blank_lines() {
    let text = b"#!/bin/sh\n\n# real desc\necho hi\n";
    assert_eq!(suggest_description("shell", text), "real desc");
}

#[test]
fn test_extract_comment_description_skips_metadata_fence() {
    let text = b"#!/bin/bash\n# /// script\n# actual desc\ncode\n";
    assert_eq!(suggest_description("shell", text), "actual desc");
}

#[test]
fn test_extract_comment_description_empty_comment_line_continues() {
    let text = b"#!/bin/sh\n#\n# after empty\necho\n";
    assert_eq!(suggest_description("shell", text), "after empty");
}

#[test]
fn test_extract_comment_description_code_first_is_empty() {
    let text = b"NAME=1\n# a comment below code\n";
    assert_eq!(suggest_description("shell", text), "");
}

#[test]
fn test_extract_comment_description_only_shebang_is_empty() {
    assert_eq!(suggest_description("shell", b"#!/bin/sh\n\n"), "");
}

#[test]
fn test_extract_comment_description_lua_double_dash_prefix() {
    assert_eq!(suggest_description("lua", b"-- Lua tool\nprint('x')\n"), "Lua tool");
}

#[test]
fn test_infer_kind_python_and_forced_exe() {
    assert_eq!(infer_kind(Path::new("a.py"), None, false), Some("python"));
    assert_eq!(infer_kind(Path::new("B.PY"), None, false), Some("python"));
    // Python's `force_exe=True` is now a caller-side override. The underlying inference must still
    // report Python even when the file is executable; otherwise callers cannot distinguish infer
    // from an explicit --exe override.
    assert_eq!(infer_kind(Path::new("a.py"), None, true), Some("python"));
}

#[cfg(not(windows))]
#[test]
fn test_infer_kind_posix_uses_execute_bit() {
    assert_eq!(infer_kind(Path::new("prog"), None, false), None);
    assert_eq!(infer_kind(Path::new("prog"), None, true), Some("exe"));
    assert_eq!(
        infer_kind(Path::new("deploy"), Some("#!/usr/bin/env bash"), false),
        Some("shell")
    );
    assert_eq!(
        infer_kind(Path::new("deploy"), Some("#!/usr/bin/env bash"), true),
        Some("shell")
    );
}
