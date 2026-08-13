//! Shebang and kind-inference ports from Python `tests/test_interpreters.py` at `main@206f9ef`.
//!
//! Rust separates source I/O from shebang parsing, so the one Python path-read/OSError helper
//! contract is classified in the companion manifest. Every parser/kind contract here uses the
//! public `shebang_program` / `infer_kind` seams directly.

use std::path::Path;

use skit_language::{infer_kind, shebang_program};

fn shebang_kind(line: &str) -> Option<&'static str> {
    infer_kind(Path::new("extensionless"), Some(line), false)
}

#[test]
fn test_shebang_plain() {
    assert_eq!(shebang_program("#!/bin/bash"), Some("bash"));
}

#[test]
fn test_shebang_env_form() {
    assert_eq!(shebang_program("#!/usr/bin/env python3"), Some("python3"));
}

#[test]
fn test_shebang_env_dash_s_with_flags() {
    assert_eq!(
        shebang_program("#!/usr/bin/env -S deno run --allow-net"),
        Some("deno")
    );
}

#[test]
fn test_shebang_none_when_no_shebang() {
    assert_eq!(shebang_program("echo hi"), None);
}

#[test]
fn test_shebang_none_when_empty_hashbang_line() {
    assert_eq!(shebang_program("#!"), None);
}

#[test]
fn test_shebang_env_with_only_flags_is_none() {
    assert_eq!(shebang_program("#!/usr/bin/env -S"), None);
}

#[test]
fn test_kind_for_shebang_maps_the_program_or_none() {
    assert_eq!(shebang_kind("#!/usr/bin/env bash"), Some("shell"));
    assert_eq!(shebang_kind("#!/usr/bin/env node"), Some("js"));
    assert_eq!(shebang_kind("#!/usr/bin/env python3"), Some("python"));
    assert_eq!(shebang_kind("#!/usr/bin/env cobol"), None);
    assert_eq!(infer_kind(Path::new("extensionless"), None, false), None);
}

#[test]
fn rust_additive_kind_for_shebang_bash() {
    assert_eq!(shebang_kind("#!/usr/bin/env bash"), Some("shell"));
}

#[test]
fn rust_additive_kind_for_shebang_node() {
    assert_eq!(shebang_kind("#!/usr/bin/env node"), Some("js"));
}

#[test]
fn rust_additive_kind_for_shebang_python3() {
    assert_eq!(shebang_kind("#!/usr/bin/env python3"), Some("python"));
}

#[test]
fn rust_additive_kind_for_shebang_unmapped() {
    assert_eq!(shebang_kind("#!/usr/bin/env cobol"), None);
}

#[test]
fn rust_additive_kind_for_shebang_absent() {
    assert_eq!(infer_kind(Path::new("extensionless"), None, false), None);
}

#[test]
fn test_kind_for_shebang_versioned_python_is_python() {
    assert_eq!(shebang_kind("#!/usr/bin/env python3.12"), Some("python"));
    assert_eq!(shebang_kind("#!/usr/bin/env pythonw"), None);
}

#[test]
fn rust_additive_versioned_python_shebang_is_python() {
    assert_eq!(shebang_kind("#!/usr/bin/env python3.12"), Some("python"));
}

#[test]
fn rust_additive_pythonw_shebang_is_unmapped() {
    assert_eq!(shebang_kind("#!/usr/bin/env pythonw"), None);
}

#[test]
fn test_kind_for_shebang_text_versioned_python_and_non_matches() {
    assert_eq!(shebang_kind("#!/usr/bin/env python3.12"), Some("python"));
    assert_eq!(shebang_kind("#!/usr/bin/env python3"), Some("python"));
    assert_eq!(shebang_kind("#!/usr/bin/env python"), Some("python"));
    assert_eq!(shebang_kind("#!/usr/bin/env pythonw"), None);
    assert_eq!(shebang_kind("#!/usr/bin/awk -f"), None);
}

#[test]
fn rust_additive_shebang_text_python312() {
    assert_eq!(shebang_kind("#!/usr/bin/env python3.12"), Some("python"));
}

#[test]
fn rust_additive_shebang_text_python3() {
    assert_eq!(shebang_kind("#!/usr/bin/env python3"), Some("python"));
}

#[test]
fn rust_additive_shebang_text_python() {
    assert_eq!(shebang_kind("#!/usr/bin/env python"), Some("python"));
}

#[test]
fn rust_additive_shebang_text_pythonw_is_unmapped() {
    assert_eq!(shebang_kind("#!/usr/bin/env pythonw"), None);
}

#[test]
fn rust_additive_shebang_text_awk_is_unmapped() {
    assert_eq!(shebang_kind("#!/usr/bin/awk -f"), None);
}

#[test]
fn test_infer_kind_versioned_python_shebang() {
    assert_eq!(
        infer_kind(
            Path::new("runme"),
            Some("#!/usr/bin/env python3.12"),
            !cfg!(windows),
        ),
        Some("python")
    );
}

#[test]
fn test_infer_extension_beats_shebang() {
    assert_eq!(
        infer_kind(Path::new("j.py"), Some("#!/bin/bash"), true),
        Some("python")
    );
}

#[test]
fn test_infer_shebang_beats_exec_bit() {
    assert_eq!(
        infer_kind(Path::new("deploy"), Some("#!/usr/bin/env bash"), true),
        Some("shell")
    );
}

#[test]
fn test_infer_unknown_shebang_program_falls_to_exec_bit() {
    assert_eq!(
        infer_kind(
            Path::new("prog"),
            Some("#!/usr/bin/env frobnicator"),
            !cfg!(windows),
        ),
        if cfg!(windows) { None } else { Some("exe") }
    );
}

#[test]
fn test_infer_exec_bit_only_is_exe() {
    if cfg!(windows) {
        // Python skips this exact contract on Windows because Windows has no POSIX execute bit.
        return;
    }
    assert_eq!(infer_kind(Path::new("prog"), None, true), Some("exe"));
}

#[test]
fn test_infer_plain_file_is_unknown() {
    assert_eq!(infer_kind(Path::new("notes"), None, false), None);
}

#[test]
fn test_infer_zsh_extension_is_shell() {
    assert_eq!(infer_kind(Path::new("x.zsh"), None, false), Some("shell"));
}

#[test]
fn test_infer_r_extension_is_case_insensitive() {
    assert_eq!(infer_kind(Path::new("x.R"), None, false), Some("r"));
}
