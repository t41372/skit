//! Frozen helper-shaped contracts from Python `tests/test_add_no_source.py`, proven through the
//! real public CLI rather than by recreating Python's private `_add_no_source_ask` helper in Rust.

#[path = "support/add_no_source.rs"]
mod support;

use std::fs;
use skit_application::EntryRepository as _;
use support::{Sandbox, flat};

fn assert_cancelled(code: u32, output: &str) {
    assert_eq!(code, 130, "{output}");
    assert!(
        output.lines().any(|line| line.trim() == "Cancelled — nothing was added."),
        "the frozen exact cancellation line disappeared:\n{output}"
    );
}

#[test]
fn test_ans_term_dumb_forces_the_plain_menu_even_with_form_tui() {
    let s = Sandbox::new();
    s.set_form("tui");
    let (code, output) = s.run_pty_with_term(&["add"], "1\n\n", "dumb");
    assert_cancelled(code, &output);
    assert!(output.lines().any(|line| line.trim() == "What would you like to add?"), "{output}");
}

#[test]
fn test_ans_plain_menu_lines_are_exact() {
    let s = Sandbox::new();
    s.set_form("plain");
    let (code, output) = s.run_pty_with_term(&["add"], "1\n\n", "xterm");
    assert_cancelled(code, &output);
    for expected in [
        "What would you like to add?",
        "1. A file you already have — a script, program, or prompt",
        "2. A new script, written in your editor",
        "3. A new AI-agent prompt, written in your editor",
        "4. A command template (e.g. ffmpeg -i {input})",
    ] {
        assert!(
            output.lines().any(|line| line.trim() == expected),
            "missing exact plain-menu line {expected:?}:\n{output}"
        );
    }
}

#[test]
fn test_ans_choice4_reports_params_and_stores_description() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "4\ntpl {a} {b}\ncmd4\na fine command\n");
    assert_eq!(code, 0, "{output}");
    let entry = s.store().resolve("cmd4").unwrap();
    let settings = skit_domain::EntrySettings::from_meta(&entry.meta);
    assert_eq!(entry.meta.kind.as_str(), "command");
    assert_eq!(settings.params, ["a", "b"]);
    assert_eq!(entry.meta.description, "a fine command");
    assert!(
        output.lines().any(|line| line.trim() == "Detected parameters: a, b (the run form asks for them; your last values are remembered)"),
        "the frozen teaching line disappeared:\n{output}"
    );
}

#[test]
fn test_ans_choice4_empty_template_cancels_with_exact_message() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "4\n   \n");
    assert_cancelled(code, &output);
    assert!(s.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_ans_choice4_empty_name_cancels_with_exact_message() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "4\necho {x}\n  \n");
    assert_cancelled(code, &output);
    assert!(s.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_ans_choice1_empty_path_cancels_with_exact_message() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "1\n  \n");
    assert_cancelled(code, &output);
    assert!(s.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_ans_choice1_returns_the_typed_path() {
    let s = Sandbox::new();
    let source = s.home().join("tool.py");
    fs::write(&source, "print(1)\n").unwrap();
    let (code, output) = s.run_pty(&["add"], "1\n  ~/tool.py  \n\n\n");
    assert_eq!(code, 0, "the typed path was not trimmed and rejoined to the real path lane:\n{output}");
    assert_eq!(s.store().resolve("tool").unwrap().meta.kind.as_str(), "python");
}

#[test]
fn test_cli_plain_choice4_prompt_labels_and_choices() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "4\ntpl {a} {b}\nenc\n\n");
    assert_eq!(code, 0, "{output}");
    let shown = flat(&output);
    for expected in [
        "Which one? [1/2/3/4] (1):",
        "Command template:",
        "Name for the command:",
        "Description (optional)",
    ] {
        assert!(shown.contains(expected), "missing prompt contract {expected:?}: {shown}");
    }
}

#[test]
fn test_cli_plain_choice1_path_label() {
    let s = Sandbox::new();
    let name = if cfg!(windows) { "tool.exe" } else { "tool" };
    let source = s.source(name, b"bytes\n");
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
    assert!(flat(&output).contains("Path to the file:"), "{output}");
    assert_eq!(s.store().resolve("tool").unwrap().meta.kind.as_str(), "exe");
}

#[test]
fn test_cancelled_add_exact_line_and_exit_code() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "1\n\n");
    assert_cancelled(code, &output);
    assert!(!output.contains("XX"), "string-mutation marker leaked:\n{output}");
}
