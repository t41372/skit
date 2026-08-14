#[path = "support/shell_inject.rs"]
mod support;

use std::{fs, path::Path};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_language::{ParseOutcome, parse_document, write_managed_params};
use support::{Sandbox, output_text};

#[cfg(unix)]
fn install_secret_inspector_bash(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt as _;
    let path = sandbox.bin_path().join("bash");
    fs::write(
        &path,
        r#"#!/bin/sh
if [ "$1" = "-n" ]; then exit 0; fi
printf 'STAGED_MODE=%s\n' "$(LC_ALL=C ls -ld "$1" | cut -c1-10)"
if grep -Fq "s3cr3t" "$1"; then printf '%s\n' 'STAGED_SECRET_PRESENT=yes'; else exit 9; fi
printf '%s\n' 'done'
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn create_secret_const_entry(sandbox: &Sandbox, name: &str) {
    let source = "#!/usr/bin/env bash\nAPI_KEY=changeme\necho \"done\"\n";
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("fixture must parse");
    };
    let mut declaration = document
        .analysis()
        .candidates
        .into_iter()
        .find(|candidate| candidate.declaration.name == "API_KEY")
        .expect("API_KEY candidate")
        .declaration;
    declaration.secret = true;
    let managed = write_managed_params("shell", source, &[declaration]).unwrap();
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
fn test_execute_reports_a_whitespace_split_as_a_bad_value() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_bash();
    sandbox.create_managed_entry(
        "split",
        "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\necho \"$FIRST $LAST\"\n",
    );
    let output = sandbox.run_sets(
        "split",
        &[("input-1", "John Paul"), ("input-2", "Doe")],
    );
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(text.contains("input-1"), "split refusal must identify the unsafe non-last field:\n{text}");
    assert!(!text.contains("SHELL_PATH="), "split refusal must happen before launch:\n{text}");
    assert!(sandbox.staged_files("split").is_empty(), "{text}");
}

#[cfg(unix)]
#[test]
fn test_secret_value_never_reaches_stdout() {
    let sandbox = Sandbox::new();
    install_secret_inspector_bash(&sandbox);
    create_secret_const_entry(&sandbox, "secret");
    let output = sandbox.run_sets("secret", &[("API_KEY", "s3cr3t")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("STAGED_SECRET_PRESENT=yes"), "the child must receive a staged source containing the real secret:\n{text}");
    assert!(text.contains("STAGED_MODE=-rw-------"), "secret-bearing staged source must be mode 0600:\n{text}");
    assert!(text.lines().any(|line| line == "done"), "child output must remain the script's public output:\n{text}");
    assert!(!text.contains("s3cr3t"), "secret leaked to visible stdout/stderr/transparency:\n{text}");
    assert!(sandbox.staged_files("secret").is_empty(), "secret-bearing staged source must be short-lived:\n{text}");
}
