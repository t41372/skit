#[path = "support/shell_inject.rs"]
mod support;

use std::fs;

use skit_language::{ParseOutcome, inject_values, parse_document};
use support::{Sandbox, output_text};

#[cfg(unix)]
fn install_gate_bash(sandbox: &Sandbox, stderr: Option<&str>) {
    use std::os::unix::fs::PermissionsExt as _;

    let path = sandbox.bin_path().join("bash");
    let diagnostic = stderr
        .map(|message| format!("printf '%s\\n' '{}' >&2\n", message.replace('\\'', "'\\''")))
        .unwrap_or_default();
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"-n\" ]; then\n{diagnostic}  exit 1\nfi\nif [ -n \"${{SKIT_TEST_MARKER:-}}\" ]; then : > \"$SKIT_TEST_MARKER\"; fi\nexit 0\n"
    );
    fs::write(&path, script).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
#[test]
fn test_interpreter_gate_refuses_what_the_offline_gate_missed() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry("gate2", "#!/usr/bin/env bash\nTITLE=hello\n");

    // Prove the parser-backed/offline layer accepts the exact injected text first.  Gate 2 is a
    // separate authority: the real configured shell may still reject syntax the offline parser
    // considers acceptable.
    let source = "#!/usr/bin/env bash\nTITLE=hello\n";
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("fixture must parse before injection");
    };
    let declarations = document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();
    let values = std::collections::BTreeMap::from([("TITLE".to_owned(), "x".to_owned())]);
    let rewritten = inject_values("shell", source, &declarations, &values).unwrap();
    assert!(matches!(parse_document("shell", &rewritten), ParseOutcome::Parsed(_)));

    install_gate_bash(&sandbox, Some("bash: staged source rejected"));
    let marker = sandbox.home_path().join("launched.marker");
    let output = sandbox
        .command()
        .env("SKIT_TEST_MARKER", &marker)
        .args(["run", "gate2", "--set", "TITLE=x", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);

    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(text.contains("bash"), "interpreter-gate refusal must identify the shell boundary:\n{text}");
    assert!(!marker.exists(), "a gate-2 syntax refusal must stop the actual child launch");
    assert!(sandbox.staged_files("gate2").is_empty(), "gate-2 refusal must delete the staged source:\n{text}");
}

#[cfg(unix)]
#[test]
fn test_interpreter_gate_reports_an_empty_stderr_without_crashing() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry("gate_empty", "#!/usr/bin/env bash\nTITLE=hello\n");
    install_gate_bash(&sandbox, None);
    let marker = sandbox.home_path().join("launched.marker");
    let output = sandbox
        .command()
        .env("SKIT_TEST_MARKER", &marker)
        .args(["run", "gate_empty", "--set", "TITLE=x", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);

    assert_eq!(output.status.code(), Some(125), "empty shell stderr must still become a stable injection refusal, not a panic:\n{text}");
    assert!(!marker.exists(), "empty diagnostic text must not accidentally bypass gate 2");
    assert!(sandbox.staged_files("gate_empty").is_empty(), "empty-stderr refusal must clean the staged source:\n{text}");
}
