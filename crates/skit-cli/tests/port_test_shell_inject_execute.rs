#[path = "support/shell_inject.rs"]
mod support;

use std::{fs, path::Path, process::Command};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions, payload_stored_name,
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_language::write_managed_params;
use skit_runtime::{ProgramProbe as _, SystemProbe};
use support::{Sandbox, output_text};

#[cfg(unix)]
fn real_bash_available() -> bool {
    SystemProbe.find_program("bash").is_some()
}

#[cfg(unix)]
fn install_rejecting_gate_bash(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt as _;
    let path = sandbox.bin_path().join("bash");
    fs::write(
        &path,
        r#"#!/bin/sh
if [ "$1" = "-n" ]; then
  printf '%s\n' 'synthetic shell syntax refusal' >&2
  exit 1
fi
if [ -n "${SKIT_TEST_MARKER:-}" ]; then : > "$SKIT_TEST_MARKER"; fi
exit 0
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn create_drifted_width_entry(sandbox: &Sandbox, name: &str) {
    let mut width = ParamDecl::new("WIDTH");
    width.binding = ParameterBinding::Const;
    width.delivery = ParameterDelivery::Inject;
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));
    let managed = write_managed_params(
        "shell",
        "#!/usr/bin/env bash\nTALL=800\n",
        &[width],
    )
    .unwrap();
    let kind = EntryKind::parse("shell").unwrap();
    sandbox
        .store()
        .create(CreateEntry {
            name: name.to_owned(),
            kind: kind.clone(),
            mode: StorageMode::Copy,
            source: format!("{name}.sh"),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: managed.into_bytes(),
                stored_name: Some(payload_stored_name(&kind, Path::new(&format!("{name}.sh")))),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
}

#[cfg(unix)]
#[test]
fn test_execute_runs_a_shell_entry_with_injected_values() {
    if !real_bash_available() { return; }
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "exsh1",
        concat!(
            "#!/usr/bin/env bash\n",
            "WIDTH=800\n",
            "printf 'w=%s\\n' \"$WIDTH\" > \"$PWD/injected-result.txt\"\n",
        ),
    );
    let output = sandbox.run_sets("exsh1", &[("WIDTH", "1200")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(
        fs::read_to_string(sandbox.home_path().join("injected-result.txt")).unwrap(),
        "w=1200\n"
    );
    assert!(sandbox.staged_files("exsh1").is_empty(), "staged copy must be cleaned after the run:\n{text}");
}

#[cfg(unix)]
#[test]
fn test_execute_runs_a_managed_read_with_the_block_in_place() {
    if !real_bash_available() { return; }
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "exsh1b",
        concat!(
            "#!/usr/bin/env bash\n",
            "read -s -p \"Password: \" PW\n",
            "printf 'len=%s\\n' \"${#PW}\" > \"$PWD/read-result.txt\"\n",
        ),
    );
    let stored = fs::read_to_string(sandbox.payload_path("exsh1b")).unwrap();
    assert!(stored.contains("# /// script"), "managed block must be physically present:\n{stored}");

    let output = sandbox.run_sets("exsh1b", &[("input-1", "hunter2")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("Password: ***"), "{text}");
    assert!(!text.contains("hunter2"), "secret leaked to visible output:\n{text}");
    assert_eq!(
        fs::read_to_string(sandbox.home_path().join("read-result.txt")).unwrap(),
        "len=7\n"
    );
    assert!(sandbox.staged_files("exsh1b").is_empty(), "{text}");
}

#[cfg(unix)]
#[test]
fn test_run_refuses_a_bad_value_before_it_ever_launches() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_bash();
    sandbox.create_managed_entry("exsh3", "#!/usr/bin/env bash\nWIDTH=800\n");
    let output = sandbox.run_sets("exsh3", &[("WIDTH", "abc")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(!text.contains("SHELL_PATH="), "bad values must be refused before interpreter launch:\n{text}");
    assert!(sandbox.staged_files("exsh3").is_empty(), "{text}");
}

#[cfg(unix)]
#[test]
fn test_execute_maps_a_drifted_shell_definition_to_drift() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_bash();
    create_drifted_width_entry(&sandbox, "exsh3b");
    let output = sandbox.run_sets("exsh3b", &[("WIDTH", "1200")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(text.contains("--resync"), "drift refusal must carry the resync hint:\n{text}");
    assert!(!text.contains("SHELL_PATH="), "drift must be refused before launch:\n{text}");
    assert!(sandbox.staged_files("exsh3b").is_empty(), "{text}");
}

#[cfg(unix)]
#[test]
fn test_execute_reports_a_positional_gap_as_a_bad_value() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_bash();
    sandbox.create_managed_entry(
        "exsh4",
        "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\necho \"$FIRST $LAST\"\n",
    );
    let output = sandbox.run_sets("exsh4", &[("input-2", "Lovelace")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(text.contains("input-1"), "gap diagnostic must identify the empty earlier field:\n{text}");
    assert!(!text.contains("SHELL_PATH="), "gap refusal must happen before launch:\n{text}");
    assert!(sandbox.staged_files("exsh4").is_empty(), "{text}");
}

#[cfg(unix)]
#[test]
fn test_execute_surfaces_the_self_location_warning() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_bash();
    sandbox.create_managed_entry(
        "exsh5",
        "#!/usr/bin/env bash\nHERE=$(dirname \"$0\")\nWIDTH=800\necho \"$WIDTH\"\n",
    );
    let output = sandbox.run_sets("exsh5", &[("WIDTH", "1200")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("$0"), "self-location warning must reach the user:\n{text}");
}

#[cfg(unix)]
#[test]
fn test_execute_syntax_gate_failure_never_launches() {
    let sandbox = Sandbox::new();
    install_rejecting_gate_bash(&sandbox);
    sandbox.create_managed_entry("exsh6", "#!/usr/bin/env bash\nTITLE=hello\n");
    let marker = sandbox.home_path().join("launched.marker");
    let output = sandbox
        .command()
        .env("SKIT_TEST_MARKER", &marker)
        .args(["run", "exsh6", "--set", "TITLE=x", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(!marker.exists(), "syntax-gate failure must never reach the real launch path");
    assert!(!text.contains("--resync"), "resync cannot repair skit's own post-injection syntax failure:\n{text}");
    assert!(sandbox.staged_files("exsh6").is_empty(), "gate failure must clean the staged source:\n{text}");
}
