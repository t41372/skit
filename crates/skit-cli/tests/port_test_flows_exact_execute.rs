//! Exact process-boundary ports from Python v0.4 `tests/test_flows.py`.
#![cfg(unix)]

#[path = "support/shell_inject.rs"]
mod support;

use std::{fs, path::Path};

use support::{Sandbox, output_text, tagged};

fn install_reporting_bash(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt as _;
    let path = sandbox.bin_path().join("bash");
    fs::write(
        &path,
        r#"#!/bin/sh
if [ "$1" = "-n" ]; then exit 0; fi
if [ -n "${SKIT_TEST_MARKER:-}" ]; then : > "$SKIT_TEST_MARKER"; fi
printf 'FLOW_PATH=%s\n' "$1"
printf 'FLOW_MODE=%s\n' "${MODE-}"
printf 'FLOW_CWD=%s\n' "$PWD"
shift
printf 'FLOW_ARGS='
printf '<%s>' "$@"
printf '\n'
exit 0
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn install_syntax_rejecting_bash(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt as _;
    let path = sandbox.bin_path().join("bash");
    fs::write(
        &path,
        r#"#!/bin/sh
if [ "$1" = "-n" ]; then printf '%s\n' 'syntax error near unexpected token' >&2; exit 2; fi
if [ -n "${SKIT_TEST_MARKER:-}" ]; then : > "$SKIT_TEST_MARKER"; fi
exit 0
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn entry_dir(payload: &Path) -> &Path {
    payload.parent().expect("stored payload lives under its entry directory")
}

#[test]
fn test_execute_env_only_does_not_materialize_temp_copy() {
    let sandbox = Sandbox::new();
    install_reporting_bash(&sandbox);
    sandbox.create_managed_entry("envonly", "#!/usr/bin/env bash\necho \"${MODE:-auto}\"\n");
    let original = sandbox.payload_path("envonly");

    let output = sandbox.run_sets("envonly", &[("MODE", "manual")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(Path::new(tagged(&text, "FLOW_PATH=")), original.as_path(), "{text}");
    assert_eq!(tagged(&text, "FLOW_MODE="), "manual", "{text}");
    assert!(sandbox.staged_files("envonly").is_empty(), "{text}");
}

#[test]
fn test_no_values_managed_copy_runs_original_payload() {
    let sandbox = Sandbox::new();
    install_reporting_bash(&sandbox);
    sandbox.create_managed_entry("novalues", "#!/usr/bin/env bash\nWIDTH=800\necho \"$WIDTH\"\n");
    let original = sandbox.payload_path("novalues");

    let output = sandbox.command().args(["run", "novalues", "--no-input"]).output().unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(Path::new(tagged(&text, "FLOW_PATH=")), original.as_path(), "{text}");
    assert!(sandbox.staged_files("novalues").is_empty(), "{text}");
}

#[test]
fn test_execute_injected_script_is_not_written_under_the_entry_dir() {
    let sandbox = Sandbox::new();
    install_reporting_bash(&sandbox);
    sandbox.create_managed_entry("outside", "#!/usr/bin/env bash\nWIDTH=800\necho \"$WIDTH\"\n");
    let original = sandbox.payload_path("outside");

    let output = sandbox.run_sets("outside", &[("WIDTH", "1200")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let launched = Path::new(tagged(&text, "FLOW_PATH="));
    assert_ne!(launched, original.as_path(), "injection incorrectly used the original payload: {text}");
    assert_ne!(
        launched.parent(),
        Some(entry_dir(&original)),
        "secret-bearing injected source must live in OS temp, not the persistent entry directory: {text}"
    );
    assert!(!launched.exists(), "the injected source survived the run: {}", launched.display());
}

#[test]
fn test_execute_injected_script_falls_back_to_entry_dir_if_os_temp_unavailable() {
    let sandbox = Sandbox::new();
    install_reporting_bash(&sandbox);
    sandbox.create_managed_entry("fallback", "#!/usr/bin/env bash\nWIDTH=800\necho \"$WIDTH\"\n");
    let original = sandbox.payload_path("fallback");
    let bad_tmp = sandbox.home_path().join("missing-temp-parent/nope");

    let output = sandbox
        .command()
        .env("TMPDIR", &bad_tmp)
        .args(["run", "fallback", "--set", "WIDTH=1200", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let launched = Path::new(tagged(&text, "FLOW_PATH="));
    assert_eq!(launched.parent(), Some(entry_dir(&original)), "{text}");
    assert!(
        launched.file_name().is_some_and(|name| name.to_string_lossy().starts_with(".injected-")),
        "fallback copy must retain the frozen .injected-* naming contract: {text}"
    );
    assert!(!launched.exists(), "fallback injected source survived cleanup: {text}");
}

#[test]
fn test_execute_cleans_injected_file_after_launcher() {
    let sandbox = Sandbox::new();
    install_reporting_bash(&sandbox);
    sandbox.create_managed_entry("cleanup", "#!/usr/bin/env bash\nWIDTH=800\necho \"$WIDTH\"\n");
    let output = sandbox.run_sets("cleanup", &[("WIDTH", "1200")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let launched = Path::new(tagged(&text, "FLOW_PATH="));
    assert!(!launched.exists(), "injected source survived successful launch: {text}");
    assert!(sandbox.staged_files("cleanup").is_empty(), "{text}");
}

#[test]
fn test_execute_cleans_injected_file_after_launcher_raises() {
    use std::os::unix::fs::PermissionsExt as _;
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry("raise", "#!/usr/bin/env bash\nWIDTH=800\necho \"$WIDTH\"\n");
    let broken = sandbox.bin_path().join("bash");
    fs::write(&broken, "#!/skit/no/such/interpreter\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&broken).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&broken, permissions).unwrap();

    let output = sandbox.run_sets("raise", &[("WIDTH", "1200")]);
    let text = output_text(&output);
    assert_ne!(output.status.code(), Some(0), "broken launcher unexpectedly ran: {text}");
    assert!(sandbox.staged_files("raise").is_empty(), "injected source leaked after spawn failure: {text}");
    let payload = sandbox.payload_path("raise");
    let entry = entry_dir(&payload).to_path_buf();
    let leaked = fs::read_dir(entry)
        .unwrap()
        .filter_map(Result::ok)
        .map(|item| item.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".injected-") || name.starts_with(".run-"))
        .collect::<Vec<_>>();
    assert!(leaked.is_empty(), "injected source leaked after launcher failure: {leaked:?}\n{text}");
}

#[test]
fn test_execute_syntax_failure_does_not_blame_the_user() {
    let sandbox = Sandbox::new();
    install_syntax_rejecting_bash(&sandbox);
    sandbox.create_managed_entry("syntax", "#!/usr/bin/env bash\nTITLE=hello\necho \"$TITLE\"\n");
    let marker = sandbox.home_path().join("launched.marker");
    let output = sandbox
        .command()
        .env("SKIT_TEST_MARKER", &marker)
        .args(["run", "syntax", "--set", "TITLE=x", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(!text.contains("--resync"), "a generated-syntax failure must not blame source drift: {text}");
    assert!(!marker.exists(), "syntax gate failure reached the child: {text}");
    assert!(sandbox.staged_files("syntax").is_empty(), "{text}");
}

#[test]
fn test_execute_dry_run_skips_launcher() {
    let sandbox = Sandbox::new();
    install_reporting_bash(&sandbox);
    sandbox.create_managed_entry("dry", "#!/usr/bin/env bash\nWIDTH=800\necho \"$WIDTH\"\n");
    let marker = sandbox.home_path().join("launched.marker");
    let output = sandbox
        .command()
        .env("SKIT_TEST_MARKER", &marker)
        .args(["run", "dry", "--set", "WIDTH=1200", "--dry-run", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(!marker.exists(), "dry-run launched the child: {text}");
    assert!(sandbox.staged_files("dry").is_empty(), "dry-run materialized an injected copy: {text}");
}
