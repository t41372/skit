//! Final exact CLI-boundary ports from Python v0.4 `tests/test_flows.py`.
//!
//! These cases intentionally use the real CLI/store boundary. A current Rust wording or
//! classification mismatch stays red; the test must not be weakened to fit the implementation.

#[path = "support/shell_inject.rs"]
mod support;

use std::{fs, path::{Path, PathBuf}};

use serde_json::Value as JsonValue;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use support::{Sandbox, output_text};

const MANAGED_PYTHON: &str = concat!(
    "# /// script\n",
    "# dependencies = []\n",
    "#\n",
    "# [tool.skit]\n",
    "# schema = 1\n",
    "#\n",
    "# [[tool.skit.params]]\n",
    "# name = \"OUTPUT\"\n",
    "# kind = \"const\"\n",
    "# type = \"str\"\n",
    "# default = \"out.jpg\"\n",
    "#\n",
    "# [[tool.skit.params]]\n",
    "# name = \"WIDTH\"\n",
    "# kind = \"const\"\n",
    "# type = \"int\"\n",
    "# default = 800\n",
    "#\n",
    "# [[tool.skit.params]]\n",
    "# name = \"API_KEY\"\n",
    "# kind = \"const\"\n",
    "# type = \"str\"\n",
    "# default = \"xxx\"\n",
    "# secret = true\n",
    "# env_source = \"MY_API_KEY\"\n",
    "# ///\n",
    "OUTPUT = 'out.jpg'\n",
    "WIDTH = 800\n",
    "API_KEY = 'xxx'\n",
    "print(OUTPUT, WIDTH, API_KEY)\n",
);

const RETRIES_PYTHON: &str = concat!(
    "# /// script\n",
    "# [tool.skit]\n",
    "# schema = 1\n",
    "# [[tool.skit.params]]\n",
    "# name = \"RETRIES\"\n",
    "# kind = \"const\"\n",
    "# type = \"int\"\n",
    "# default = 3\n",
    "# ///\n",
    "RETRIES = 3\n",
    "print(RETRIES)\n",
);

const DRIFT_PYTHON: &str = concat!(
    "# /// script\n",
    "# [tool.skit]\n",
    "# schema = 1\n",
    "# [[tool.skit.params]]\n",
    "# name = \"CITY\"\n",
    "# kind = \"const\"\n",
    "# type = \"str\"\n",
    "# default = \"Taipei\"\n",
    "# [[tool.skit.params]]\n",
    "# name = \"GONE\"\n",
    "# kind = \"const\"\n",
    "# type = \"str\"\n",
    "# default = \"old\"\n",
    "# ///\n",
    "CITY = \"Taipei\"\n",
    "GONE = \"old\"\n",
    "print(CITY)\n",
);

fn create_copy(
    sandbox: &Sandbox,
    name: &str,
    kind_name: &str,
    source_name: &str,
    body: &[u8],
    settings: EntrySettings,
) -> PathBuf {
    let kind = EntryKind::parse(kind_name).unwrap();
    sandbox
        .store()
        .create(CreateEntry {
            name: name.to_owned(),
            kind: kind.clone(),
            mode: StorageMode::Copy,
            source: source_name.to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: body.to_vec(),
                stored_name: Some(payload_stored_name(&kind, Path::new(source_name))),
                permissions: SourcePermissions::default(),
            }),
            settings,
        })
        .unwrap();
    sandbox.payload_path(name)
}

fn create_prompt(sandbox: &Sandbox, name: &str, runner: &str) -> PathBuf {
    create_copy(
        sandbox,
        name,
        "prompt",
        &format!("{name}.prompt.md"),
        b"Do it\n",
        EntrySettings {
            runner: runner.to_owned(),
            ..EntrySettings::default()
        },
    )
}

fn show_json(sandbox: &Sandbox, name: &str) -> JsonValue {
    let output = sandbox
        .command()
        .args(["show", name, "--json"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[cfg(unix)]
fn install_amp(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt as _;
    let path = sandbox.bin_path().join("amp");
    fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn test_plan_missing_script_is_none() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry("missing-plan", "#!/usr/bin/env bash\nCITY=Taipei\necho \"$CITY\"\n");
    fs::remove_file(sandbox.payload_path("missing-plan")).unwrap();

    let payload = show_json(&sandbox, "missing-plan");
    assert_eq!(payload["missing"], true);
    assert_eq!(payload["param_source"], "none");
    assert_eq!(payload["fields"], serde_json::json!([]));
}

#[test]
fn test_assemble_defaults_env_to_os_environ() {
    let sandbox = Sandbox::new();
    create_copy(
        &sandbox,
        "ambient-env",
        "python",
        "ambient.py",
        MANAGED_PYTHON.as_bytes(),
        EntrySettings::default(),
    );

    let output = sandbox
        .command()
        .env_remove("MY_API_KEY")
        .args([
            "run",
            "ambient-env",
            "--set",
            "OUTPUT=o",
            "--set",
            "WIDTH=1",
            "--set",
            "API_KEY=",
            "--no-input",
        ])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_ne!(output.status.code(), Some(0), "an unset ambient secret source unexpectedly ran:\n{text}");
    assert!(text.contains("MY_API_KEY"), "the default process environment was not used for the named secret source:\n{text}");
}

#[test]
fn test_plan_drift_names_entry_and_keeps_usable_specs() {
    let sandbox = Sandbox::new();
    let script = create_copy(
        &sandbox,
        "poster",
        "python",
        "poster.py",
        DRIFT_PYTHON.as_bytes(),
        EntrySettings::default(),
    );
    let drifted = DRIFT_PYTHON.replace("GONE = \"old\"\n", "");
    fs::write(&script, &drifted).unwrap();

    let payload = show_json(&sandbox, "poster");
    assert_eq!(payload["param_source"], "inject");
    assert_eq!(payload["drift"], true);
    assert_eq!(
        payload["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["CITY"]
    );

    let output = sandbox.command().args(["show", "poster"]).output().unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("poster"), "the drift explanation lost the entry name:\n{text}");
    assert!(text.contains("skit params poster --resync"), "the drift explanation lost the recovery command:\n{text}");
    assert_eq!(fs::read_to_string(&script).unwrap(), drifted, "planning/show mutated the source instead of carrying its usable text forward");
}

#[cfg(unix)]
#[test]
fn test_pinned_amp_prompt_warns_on_runner_none_shared_execution_path() {
    let sandbox = Sandbox::new();
    sandbox.write_config(concat!(
        "[mirror]\n",
        "enabled = false\n",
        "[prompt]\n",
        "runners = [{ name = \"amp\", argv = [\"amp\", \"-x\", \"{{prompt}}\"] }]\n",
    ));
    install_amp(&sandbox);
    create_prompt(&sandbox, "amp-task", "amp");

    let output = sandbox
        .command()
        .args(["run", "amp-task", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(
        text.contains("amp -x runs this prompt once"),
        "the pinned runner path lost the frozen amp one-shot warning:\n{text}"
    );
}

#[test]
fn test_prompt_validation_classifies_missing_body_before_transparency() {
    let sandbox = Sandbox::new();
    sandbox.write_config(concat!(
        "[mirror]\n",
        "enabled = false\n",
        "[prompt]\n",
        "runners = [{ name = \"amp\", argv = [\"amp\", \"-x\", \"{{prompt}}\"] }]\n",
    ));
    let body = create_prompt(&sandbox, "gone", "amp");
    fs::remove_file(body).unwrap();

    let output = sandbox
        .command()
        .args(["run", "gone", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_ne!(output.status.code(), Some(0), "a missing prompt body unexpectedly ran:\n{text}");
    assert!(text.contains("doesn't exist"), "the missing-body classification lost the frozen message:\n{text}");
    assert!(
        !text.lines().any(|line| line.trim_start().starts_with('→')),
        "transparency was emitted before prompt-body validation:\n{text}"
    );
}

#[test]
fn test_prompt_validation_classifies_empty_runner_config_before_transparency() {
    let sandbox = Sandbox::new();
    sandbox.write_config(concat!(
        "[mirror]\n",
        "enabled = false\n",
        "[prompt]\n",
        "runners = []\n",
    ));
    create_prompt(&sandbox, "unrunnable", "");

    let output = sandbox
        .command()
        .args(["run", "unrunnable", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_ne!(output.status.code(), Some(0), "a prompt with no configured runners unexpectedly ran:\n{text}");
    assert!(text.contains("No agents are configured"), "the empty-runner classification lost its frozen explanation:\n{text}");
    assert!(
        text.contains("skit runner add mycli -- mycli run {{prompt}}"),
        "the empty-runner recovery command disappeared:\n{text}"
    );
    assert!(
        !text.lines().any(|line| line.trim_start().starts_with('→')),
        "transparency was emitted before runner validation:\n{text}"
    );
}

#[test]
fn test_execute_bad_value_reports_value_not_drift() {
    let sandbox = Sandbox::new();
    create_copy(
        &sandbox,
        "bad-value",
        "python",
        "bad.py",
        RETRIES_PYTHON.as_bytes(),
        EntrySettings::default(),
    );

    let output = sandbox
        .command()
        .args([
            "run",
            "bad-value",
            "--set",
            "RETRIES=not-a-number",
            "--no-input",
        ])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_ne!(output.status.code(), Some(0), "invalid typed input unexpectedly launched:\n{text}");
    assert!(
        text.contains("'not-a-number' isn't a valid int for RETRIES."),
        "bad-value handling no longer preserves the frozen classification/message:\n{text}"
    );
    assert!(!text.contains("resync"), "a value error was misclassified as source drift:\n{text}");
}
