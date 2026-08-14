//! Exact defensive execute contract from Python v0.4 `tests/test_shell_inject.py`.
#![cfg(unix)]

#[path = "support/shell_inject.rs"]
mod support;

use std::{fs, path::Path};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions, payload_stored_name,
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery},
};
use skit_language::write_managed_params;
use support::{Sandbox, output_text};

fn install_marker_fish(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt as _;

    let path = sandbox.bin_path().join("fish");
    fs::write(
        &path,
        r#"#!/bin/sh
if [ -n "${SKIT_TEST_MARKER:-}" ]; then : > "$SKIT_TEST_MARKER"; fi
exit 0
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn create_fish_with_inject_decl(sandbox: &Sandbox) {
    let source = "set -x NAME x\n";
    let mut declaration = ParamDecl::new("NAME");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    let managed = write_managed_params("fish", source, &[declaration]).unwrap();
    let kind = EntryKind::parse("fish").unwrap();
    sandbox
        .store()
        .create(CreateEntry {
            name: "noinj".to_owned(),
            kind: kind.clone(),
            mode: StorageMode::Copy,
            source: "noinj.fish".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: managed.into_bytes(),
                stored_name: Some(payload_stored_name(&kind, Path::new("noinj.fish"))),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
}

#[test]
fn test_execute_without_an_injector_does_not_crash() {
    let sandbox = Sandbox::new();
    install_marker_fish(&sandbox);
    create_fish_with_inject_decl(&sandbox);
    let marker = sandbox.home_path().join("fish-launched.marker");

    let output = sandbox
        .command()
        .env("SKIT_TEST_MARKER", &marker)
        .args(["run", "noinj", "--set", "NAME=y", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);

    assert_eq!(
        output.status.code(),
        Some(0),
        "an analyzer-only kind with an impossible inject plan must degrade to launch, not crash in injection:\n{text}"
    );
    assert!(marker.exists(), "the available fish runtime was never launched:\n{text}");
    assert!(
        !text.contains("source operation is not supported"),
        "missing injector surfaced as an internal source-operation failure:\n{text}"
    );
}
