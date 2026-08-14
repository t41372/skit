//! Final public command-door contracts from Python v0.4 `tests/test_add_no_source.py`.
//!
//! The frozen Python tests reached these projections through private summary helpers. Rust exposes
//! the TUI command door as a frontend-neutral reducer, so we compare its complete create request to
//! a real `skit add --cmd` entry and derive the summary inputs from persisted public state.

#[path = "support/add_no_source.rs"]
mod support;

use skit_application::{EntryMutationRepository as _, EntryRepository as _};
use skit_domain::{EntrySettings, parameters::is_secret_name};
use skit_ui::{AddAction, AddEffect, AddWorkflowState};
use support::{Sandbox, combined};

fn tui_command_request(template: &str, name: &str) -> skit_application::CreateEntry {
    let mut workflow = AddWorkflowState::new(Vec::new());
    assert!(workflow
        .reduce(AddAction::SetCommandTemplate(template.to_owned()))
        .is_empty());
    assert!(workflow
        .reduce(AddAction::SetCommandName(name.to_owned()))
        .is_empty());
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::Commit { entry, source, .. }] = effects.as_slice() else {
        panic!("TUI command door must produce exactly one create request: {effects:?}");
    };
    assert!(source.is_none(), "command entries are metadata-only");
    (**entry).clone()
}

#[test]
fn test_command_secret_names_picks_the_secret_holes() {
    let s = Sandbox::new();
    let output = s.run(&[
        "add",
        "--cmd",
        "curl -H {API_KEY} {url}",
        "--name",
        "curler-secret-projection",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let entry = s.store().resolve("curler-secret-projection").unwrap();
    let settings = EntrySettings::from_meta(&entry.meta);
    assert_eq!(settings.params, ["API_KEY", "url"]);
    assert_eq!(
        settings
            .params
            .iter()
            .filter(|name| is_secret_name(name))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["API_KEY"]
    );
}

#[test]
fn test_bare_add_tui_command_door_matches_the_cmd_door() {
    let s = Sandbox::new();
    let template = "echo {API_KEY} {msg}";
    let tui = tui_command_request(template, "viatui3");

    let output = s.run(&[
        "add",
        "--cmd",
        template,
        "--name",
        "viaflag3",
    ]);
    let shown = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{shown}");
    let direct = s.store().resolve("viaflag3").unwrap();
    let direct_settings = EntrySettings::from_meta(&direct.meta);

    assert_eq!(tui.kind, direct.meta.kind);
    assert_eq!(tui.source, direct.meta.source);
    assert_eq!(tui.mode, direct.meta.mode);
    assert_eq!(tui.workdir, direct.meta.workdir);
    assert_eq!(tui.description, direct.meta.description);
    assert_eq!(tui.settings.params, direct_settings.params);
    assert_eq!(tui.settings.params, ["API_KEY", "msg"]);
    assert_eq!(tui.settings.dependencies, direct_settings.dependencies);
    assert!(tui.payload.is_none());

    assert!(shown.contains("Detected parameters"), "{shown}");
    assert!(!shown.contains("Managed parameters"), "{shown}");
    assert!(shown.contains("Secret parameter values are never saved"), "{shown}");
}

#[test]
fn test_bare_add_tui_command_door_summary_call_contract() {
    let s = Sandbox::new();
    let request = tui_command_request("echo {API_KEY} {msg}", "viatui4");
    assert!(request.settings.dependencies.is_empty());
    assert_eq!(request.settings.params, ["API_KEY", "msg"]);
    assert!(request.payload.is_none(), "command door must have no source body from which managed declarations could be read");

    let persisted = s.store().create(request).unwrap();
    let settings = EntrySettings::from_meta(&persisted.meta);
    assert!(settings.dependencies.is_empty(), "summary deps input must be empty");
    assert_eq!(settings.params, ["API_KEY", "msg"]);
    let secrets = settings
        .params
        .iter()
        .filter(|name| is_secret_name(name))
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(secrets, ["API_KEY"]);
    assert!(
        s.store().payload_path(&persisted).is_none(),
        "summary managed-input projection must be empty for a metadata-only command entry"
    );
}
