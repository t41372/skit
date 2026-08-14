#[path = "support/shell_inject.rs"]
mod support;

use std::{fs, path::{Path, PathBuf}};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_runtime::{ProgramProbe as _, SystemProbe};
use support::{Sandbox, output_text};

fn create_copy(sandbox: &Sandbox, name: &str, kind_name: &str, source: &str) -> PathBuf {
    let kind = EntryKind::parse(kind_name).unwrap();
    sandbox
        .store()
        .create(CreateEntry {
            name: name.to_owned(),
            kind: kind.clone(),
            mode: StorageMode::Copy,
            source: format!("{name}.{}", if kind_name == "python" { "py" } else { "sh" }),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: source.as_bytes().to_vec(),
                stored_name: Some(payload_stored_name(
                    &kind,
                    Path::new(&format!("{name}.{}", if kind_name == "python" { "py" } else { "sh" })),
                )),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    sandbox.payload_path(name)
}

fn create_reference_shell(sandbox: &Sandbox, name: &str, source: &str) -> PathBuf {
    let path = sandbox.home_path().join(format!("{name}.sh"));
    fs::write(&path, source).unwrap();
    let kind = EntryKind::parse("shell").unwrap();
    sandbox
        .store()
        .create(CreateEntry {
            name: name.to_owned(),
            kind,
            mode: StorageMode::Reference,
            source: path.display().to_string(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    path
}

#[cfg(unix)]
#[test]
fn test_cli_dry_run_shows_the_command() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "cln1",
        "#!/usr/bin/env bash\nWIDTH=800\necho \"$WIDTH\"\n",
    );
    let payload = sandbox.payload_path("cln1");
    let output = sandbox
        .command()
        .args(["run", "cln1", "--set", "WIDTH=1200", "--dry-run", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("WIDTH = 1200"), "{text}");
    assert!(text.contains(&payload.display().to_string()), "dry run must show the original stored script path:\n{text}");
    assert!(!text.contains(".run-"), "dry run must not materialize or display a staged injected copy:\n{text}");
    assert!(sandbox.staged_files("cln1").is_empty(), "dry run wrote a staged copy");
}

#[test]
fn test_cli_normalize_turns_a_const_into_an_env_param() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "cln2",
        "#!/usr/bin/env bash\nWIDTH=800\nDEPTH=3\necho \"$WIDTH $DEPTH\"\n",
    );
    let payload = sandbox.payload_path("cln2");
    let output = sandbox
        .command()
        .args(["params", "cln2", "--normalize", "WIDTH"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");

    let stored = fs::read_to_string(&payload).unwrap();
    assert!(stored.contains("WIDTH=\"${WIDTH:-800}\""), "{stored}");
    assert!(stored.contains("DEPTH=3"), "{stored}");
    assert!(stored.contains("kind = \"envdefault\""), "managed definition did not follow the normalized source:\n{stored}");

    let shown = sandbox.command().args(["show", "cln2", "--json"]).output().unwrap();
    assert_eq!(shown.status.code(), Some(0), "{}", output_text(&shown));
    let value: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    let fields = value["fields"].as_array().expect("show --json fields array");
    let width = fields.iter().find(|field| field["key"] == "WIDTH").expect("WIDTH field");
    let depth = fields.iter().find(|field| field["key"] == "DEPTH").expect("DEPTH field");
    assert_eq!(width["source"], "env");
    assert_eq!(depth["source"], "inject");
}

#[cfg(unix)]
#[test]
fn test_cli_normalized_param_runs_through_the_environment() {
    if SystemProbe.find_program("bash").is_none() { return; }
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "cln3",
        concat!(
            "#!/usr/bin/env bash\n",
            "WIDTH=800\n",
            "printf 'w=%s\\n' \"$WIDTH\" > \"$PWD/normalized-result.txt\"\n",
        ),
    );
    let normalized = sandbox
        .command()
        .args(["params", "cln3", "--normalize", "WIDTH"])
        .output()
        .unwrap();
    assert_eq!(normalized.status.code(), Some(0), "{}", output_text(&normalized));

    let output = sandbox.run_sets("cln3", &[("WIDTH", "1200")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(
        fs::read_to_string(sandbox.home_path().join("normalized-result.txt")).unwrap(),
        "w=1200\n"
    );
    assert!(text.contains("WIDTH=1200"), "transparency must show the env overlay:\n{text}");
    assert!(text.contains(&sandbox.payload_path("cln3").display().to_string()), "normalized env delivery must launch the original stored copy:\n{text}");
    assert!(!text.contains(".run-"), "normalized env delivery must not use an injected temp copy:\n{text}");
    assert!(sandbox.staged_files("cln3").is_empty(), "{text}");
}

#[test]
fn test_cli_normalize_reports_refusals() {
    let sandbox = Sandbox::new();
    let payload = create_copy(
        &sandbox,
        "cln4",
        "shell",
        "#!/usr/bin/env bash\nreadonly MAX=100\n",
    );
    let before = fs::read(&payload).unwrap();
    let output = sandbox
        .command()
        .args(["params", "cln4", "--normalize", "MAX"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("readonly"), "{text}");
    assert_eq!(fs::read(&payload).unwrap(), before, "a refusal must leave the source byte-identical");
}

#[test]
fn test_cli_normalize_refuses_a_non_shell_kind() {
    let sandbox = Sandbox::new();
    create_copy(&sandbox, "cln5", "python", "WIDTH = 800\n");
    let output = sandbox
        .command()
        .args(["params", "cln5", "--normalize", "WIDTH"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.to_ascii_lowercase().contains("normalize"), "{text}");
}

#[test]
fn test_cli_normalize_refuses_reference_mode() {
    let sandbox = Sandbox::new();
    let source = "#!/usr/bin/env bash\nWIDTH=800\n";
    let path = create_reference_shell(&sandbox, "cln6", source);
    let output = sandbox
        .command()
        .args(["params", "cln6", "--normalize", "WIDTH"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("reference mode"), "{text}");
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn test_cli_normalize_without_a_stored_copy() {
    let sandbox = Sandbox::new();
    let payload = create_copy(
        &sandbox,
        "cln7",
        "shell",
        "#!/usr/bin/env bash\nWIDTH=800\n",
    );
    fs::remove_file(payload).unwrap();
    let output = sandbox
        .command()
        .args(["params", "cln7", "--normalize", "WIDTH"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("no stored copy"), "{text}");
}

#[test]
fn test_params_warns_when_a_self_locating_script_has_injectable_consts() {
    let sandbox = Sandbox::new();
    create_copy(
        &sandbox,
        "selfloc",
        "shell",
        "#!/usr/bin/env bash\nHERE=$(dirname \"$0\")\nREGION=us-east-1\necho \"$HERE $REGION\"\n",
    );
    let output = sandbox.command().args(["params", "selfloc"]).output().unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flattened.contains("locates itself"), "{flattened}");
    assert!(flattened.contains("--normalize NAME` does the rewrite for you on the stored copy"), "{flattened}");
    assert!(flattened.contains("NAME=\"${NAME:-value}\""), "{flattened}");
    assert!(!flattened.contains("leaves the file untouched"), "old misleading wording returned:\n{flattened}");
}

#[test]
fn test_params_does_not_warn_when_the_script_never_self_locates() {
    let sandbox = Sandbox::new();
    create_copy(
        &sandbox,
        "noloc",
        "shell",
        "#!/usr/bin/env bash\nREGION=us-east-1\necho $REGION\n",
    );
    let output = sandbox.command().args(["params", "noloc"]).output().unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(!text.contains("locates itself"), "{text}");
}
