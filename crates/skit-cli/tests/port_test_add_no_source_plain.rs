//! Exact public plain-CLI ports from Python v0.4 `tests/test_add_no_source.py`.
//!
//! Interactive cases use a real PTY; no test monkeypatches Rust's private lane router or prompt
//! helpers. A parity mismatch is allowed to stay red.

#[path = "support/add_no_source.rs"]
mod support;

use std::fs;

use skit_application::EntryRepository as _;
use support::{Sandbox, combined, flat};

fn assert_empty(s: &Sandbox) {
    assert!(s.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_bare_add_no_input_lists_the_lanes() {
    let s = Sandbox::new();
    let output = s.run(&["add", "--no-input"]);
    let shown = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("Provide a source path"), "{shown}");
    assert!(!shown.contains("--edit"), "non-interactive advice offered an editor lane: {shown}");
    assert!(shown.contains("--prompt"), "{shown}");
    assert!(shown.contains("--cmd"), "{shown}");
    assert!(shown.contains("skit add -"), "{shown}");
    assert!(shown.contains("-n NAME"), "{shown}");
    assert_empty(&s);
}

#[test]
fn test_bare_add_piped_lists_the_lanes() {
    let s = Sandbox::new();
    let output = s.run(&["add"]);
    let shown = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("Provide a source path"), "{shown}");
    assert_empty(&s);
}

#[test]
fn test_bare_add_interactive_refuses_each_orphan_flag() {
    for (index, args, needle) in [
        (0, vec!["add", "--name", "x"], "--name"),
        (1, vec!["add", "--description", "d"], "--description"),
        (2, vec!["add", "--ref"], "--ref"),
        (3, vec!["add", "--exe"], "--exe"),
        (4, vec!["add", "--kind", "shell"], "--kind"),
        (5, vec!["add", "--runner", "claude"], "--runner"),
        (6, vec!["add", "--dep", "rich"], "--dep"),
        (7, vec!["add", "--python", ">=3.11"], "--python"),
        (8, vec!["add", "--no-interpolate"], "--no-interpolate"),
    ] {
        let s = Sandbox::new();
        let (code, output) = s.run_pty(&args, "1\n\n\n");
        let shown = flat(&output);
        assert_eq!(code, 2, "case={index}, argv={args:?}: {shown}");
        assert!(shown.contains("need a source"), "case={index}: {shown}");
        assert!(shown.contains(needle), "case={index}: {shown}");
        assert_empty(&s);
    }
}

#[test]
fn test_plain_menu_choice4_command_template_happy_path() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "4\nffmpeg -i {input}\nencode\n\n");
    assert_eq!(code, 0, "{output}");
    let entry = s.store().resolve("encode").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "command");
    assert_eq!(skit_domain::EntrySettings::from_meta(&entry.meta).params, ["input"]);
    assert!(output.contains("Detected parameters: input"), "{output}");
}

#[test]
fn test_plain_menu_choice4_empty_template_cancels() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "4\n   \n");
    assert_eq!(code, 130, "{output}");
    assert!(output.to_ascii_lowercase().contains("nothing was added"), "{output}");
    assert_empty(&s);
}

#[test]
fn test_plain_menu_choice4_empty_name_cancels() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "4\necho {x}\n   \n");
    assert_eq!(code, 130, "{output}");
    assert!(output.to_ascii_lowercase().contains("nothing was added"), "{output}");
    assert_empty(&s);
}

#[test]
fn test_plain_menu_choice4_stores_the_description() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "4\necho {x}\nshout\nsay it loud\n");
    assert_eq!(code, 0, "{output}");
    let entry = s.store().resolve("shout").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "command");
    assert_eq!(entry.meta.description, "say it loud");
}

#[test]
fn test_plain_menu_choice1_path_continues_into_a_real_add() {
    let s = Sandbox::new();
    let name = if cfg!(windows) { "tool.exe" } else { "tool" };
    let source = s.source(name, b"opaque bytes\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(&source).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&source, permissions).unwrap();
    }
    let input = format!("1\n{}\n\n\n", source.display());
    let (code, output) = s.run_pty(&["add"], &input);
    assert_eq!(code, 0, "{output}");
    assert_eq!(s.store().resolve("tool").unwrap().meta.kind.as_str(), "exe");
}

#[test]
fn test_plain_menu_choice1_empty_path_cancels() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "1\n\n");
    assert_eq!(code, 130, "{output}");
    assert!(output.to_ascii_lowercase().contains("nothing was added"), "{output}");
    assert_empty(&s);
}

#[test]
fn test_unknown_plain_pick_language_adds_it() {
    let s = Sandbox::new();
    let source = s.source("mystery.xyz", b"echo hi\n");
    // Frozen Python order: fish, js, lua, perl, powershell, python, r, ruby, shell, ts.
    let (code, output) = s.run_pty(&["add", source.to_str().unwrap()], "9\n\n\n");
    assert_eq!(code, 0, "{output}");
    assert_eq!(s.store().resolve("mystery").unwrap().meta.kind.as_str(), "shell");
}

#[test]
fn test_unknown_plain_pick_exe_adds_it() {
    let s = Sandbox::new();
    let source = s.source("mystery.xyz", b"some opaque text\n");
    // Ten interpreted kinds, then program, then prompt.
    let (code, output) = s.run_pty(&["add", source.to_str().unwrap()], "11\n\n\n");
    assert_eq!(code, 0, "{output}");
    assert_eq!(s.store().resolve("mystery").unwrap().meta.kind.as_str(), "exe");
}

#[test]
fn test_unknown_plain_cancel_exits_130() {
    let s = Sandbox::new();
    let source = s.source("mystery.xyz", b"some opaque text\n");
    let (code, output) = s.run_pty(&["add", source.to_str().unwrap()], "-\n");
    assert_eq!(code, 130, "{output}");
    assert!(output.to_ascii_lowercase().contains("nothing was added"), "{output}");
    assert_empty(&s);
}

#[test]
fn test_cli_ask_kind_plain_full_layout() {
    let s = Sandbox::new();
    let source = s.source("mystery.xyz", b"opaque text\n");
    let (code, output) = s.run_pty(&["add", source.to_str().unwrap()], "-\n");
    assert_eq!(code, 130, "{output}");
    assert!(output.contains("What is mystery.xyz? skit can't tell from the name."), "{output}");
    for (index, label) in [
        "Fish", "JavaScript", "Lua", "Perl", "PowerShell", "Python", "R", "Ruby", "Shell", "TypeScript",
        "A program (run it directly)", "A prompt for an AI agent",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(output.contains(&format!("  {}. {label}", index + 1)), "missing option {label:?}:\n{output}");
    }
    assert!(output.contains("- = cancel"), "{output}");
    assert!(flat(&output).contains("[1/2/3/4/5/6/7/8/9/10/11/12/-]"), "{output}");
}

#[test]
fn test_cli_ask_kind_plain_shebang_question() {
    let s = Sandbox::new();
    let source = s.source("mystery.xyz", b"#!/usr/bin/env florblang\ncode\n");
    let (code, output) = s.run_pty(&["add", source.to_str().unwrap()], "-\n");
    assert_eq!(code, 130, "{output}");
    assert!(
        output.contains("The #! in mystery.xyz names no interpreter skit knows. What is it?"),
        "{output}"
    );
}
