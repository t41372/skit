//! Exact remaining public plain-lane edges from Python v0.4 `tests/test_add_no_source.py`.

#[path = "support/add_no_source.rs"]
mod support;

use skit_application::EntryRepository as _;
use support::{Sandbox, flat};

#[test]
fn test_unknown_plain_pick_language_with_runner_hits_prompt_only_refusal() {
    let s = Sandbox::new();
    let source = s.source("mystery.xyz", b"echo hi\n");
    let (code, output) = s.run_pty(
        &["add", source.to_str().unwrap(), "--runner", "claude"],
        "9\n",
    );
    assert_eq!(code, 2, "{output}");
    assert!(
        flat(&output).contains("--runner only applies to prompt entries"),
        "the picked language did not rejoin the ordinary prompt-only refusal:\n{output}"
    );
    assert!(s.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_unknown_plain_kept_draft_offers_no_program_option() {
    let s = Sandbox::new();
    let draft = s.draft("skit-new-mystery", b"some opaque text\n");
    let (code, output) = s.run_pty(&["add", draft.to_str().unwrap()], "-\n");
    assert_eq!(code, 130, "{output}");
    assert!(
        !output.contains("A program (run it directly)"),
        "a kept draft was incorrectly offered the destructive exe lane:\n{output}"
    );
    assert!(output.contains("A prompt for an AI agent"), "{output}");
}

#[test]
fn test_ans_no_stray_markup_tokens_in_output() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "1\n\n");
    assert_eq!(code, 130, "{output}");
    assert!(!output.contains("XX"), "string-mutation marker leaked:\n{output}");
}
