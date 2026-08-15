use std::collections::BTreeMap;

use skit_application::{SourcePermissions, prompt_selection::{PromptSelectionService, PromptSelectionStore as _}};
use skit_domain::parameters::ParamDecl;
use skit_store::{FileConfigStore, FilePromptSelectionStore, PromptRunner};
use skit_ui::{
    Action, AddAction, AddEffect, AddWorkflowState, Effect, FieldValue, FormPurpose, KnownEntryKind,
    LibraryState, ModalState, ReviewDefaults, ReviewState, RunnerEditorAction, RunnerEditorError,
    RunnerEditorOwner, RunnerSaveOwner, RunnerSaveTarget, Screen, SettingsAction, SettingsInputs,
    SettingsView, SourceSnapshot, TypedValue, RunFormView,
};
use tempfile::TempDir;

fn prompt_settings(runners: Vec<String>) -> SettingsView {
    SettingsView::from_inputs(&SettingsInputs {
        selector: "p".to_owned(),
        kind: "prompt".to_owned(),
        name: "p".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_stored_name: true,
        declared_schema: true,
        interpolate: true,
        configured_runners: runners,
        ..SettingsInputs::default()
    })
}

fn run_form(runners: &[String]) -> RunFormView {
    RunFormView::from_declarations(
        "p",
        "p",
        &[ParamDecl::new("a")],
        &BTreeMap::from([("a".to_owned(), "x".to_owned())]),
        runners,
        runners.first().map_or("", String::as_str),
        &BTreeMap::new(),
        "",
    )
}

fn prompt_review(runners: Vec<String>) -> ReviewState {
    ReviewState::from_source(
        SourceSnapshot {
            path: "n.prompt.md".into(),
            source_record: "n.prompt.md".to_owned(),
            bytes: b"{{a}}\n".to_vec(),
            permissions: SourcePermissions::default(),
            is_regular: true,
            is_directory: false,
            is_draft: false,
        },
        KnownEntryKind::Prompt,
        ReviewDefaults {
            runner_names: runners,
            ..ReviewDefaults::default()
        },
    )
}

fn save_new_runner(config: &FileConfigStore, request: &skit_ui::RunnerSaveRequest) {
    assert!(matches!(request.target, RunnerSaveTarget::New));
    config
        .set_runner(
            PromptRunner {
                name: request.name.clone(),
                argv: request.argv.clone(),
            },
            false,
        )
        .unwrap();
}

fn new_runner_effect(state: &mut LibraryState, owner: RunnerEditorOwner, name: &str, command: &str) -> skit_ui::RunnerSaveRequest {
    assert!(matches!(state.modal(), Some(ModalState::RunnerEditor { owner: current, .. }) if current == &owner));
    assert_eq!(
        state.update(Action::RunnerEditor(RunnerEditorAction::SetName(name.to_owned()))),
        Effect::None
    );
    assert_eq!(
        state.update(Action::RunnerEditor(RunnerEditorAction::SetCommand(command.to_owned()))),
        Effect::None
    );
    let Effect::SaveRunner { request, owner: actual_owner } =
        state.update(Action::RunnerEditor(RunnerEditorAction::Submit))
    else {
        panic!("valid runner editor did not request a host save")
    };
    assert_eq!(actual_owner, RunnerSaveOwner::Editor(owner));
    request
}

#[test]
fn test_settings_ctrl_n_adds_a_custom_agent_ready_to_pin() {
    let config_root = TempDir::new().unwrap();
    let state_root = TempDir::new().unwrap();
    let config = FileConfigStore::new(config_root.path());
    let selection_store = FilePromptSelectionStore::new(state_root.path());
    PromptSelectionService::new(selection_store.clone())
        .remember_runner("amp")
        .unwrap();
    let runners = config.runners().unwrap().into_iter().map(|runner| runner.name).collect::<Vec<_>>();

    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(prompt_settings(runners)))));
    assert_eq!(state.update(Action::Settings(SettingsAction::NewRunner)), Effect::None);
    let owner = RunnerEditorOwner::Settings { selector: "p".to_owned() };
    let request = new_runner_effect(&mut state, owner.clone(), "mycli", "mycli go {{prompt}}");
    assert_eq!(request.name, "mycli");
    assert_eq!(request.argv, ["mycli", "go", "{{prompt}}"]);
    save_new_runner(&config, &request);
    assert!(config.runners().unwrap().iter().any(|runner| runner.name == "mycli"));
    let raw = std::fs::read_to_string(config_root.path().join("config.toml")).unwrap();
    assert!(raw.contains("runners_seeded = true"), "custom-agent save failed to materialize the seeded runner config: {raw}");

    assert_eq!(
        state.update(Action::RunnerEditorSaved {
            owner,
            name: "mycli".to_owned(),
            message: "saved".to_owned(),
        }),
        Effect::None
    );
    let view = state.settings_view().unwrap();
    assert_eq!(view.field(skit_ui::RUNNER_KEY).unwrap().value().as_text(), "mycli");
    assert_eq!(
        view.submitted_values().get(skit_ui::RUNNER_KEY),
        Some(&FieldValue::Explicit(TypedValue::Choice("mycli".to_owned())))
    );
    assert_eq!(PromptSelectionService::new(selection_store).last_runner(), "amp", "defining a settings pin was incorrectly remembered as a run pick");
}

#[test]
fn test_settings_runner_select_change_arms_the_discard_ask() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(prompt_settings(vec!["claude".to_owned()])))));
    assert_eq!(
        state.update(Action::Settings(SettingsAction::SetField {
            key: skit_ui::RUNNER_KEY.to_owned(),
            value: FieldValue::Explicit(TypedValue::Choice("claude".to_owned())),
        })),
        Effect::None
    );
    assert!(state.settings_view().unwrap().is_dirty());
    assert_eq!(state.update(Action::Settings(SettingsAction::Close)), Effect::None);
    assert!(matches!(state.modal(), Some(ModalState::ConfirmDiscardChanges)));
    assert_eq!(state.update(Action::KeepEditing), Effect::None);
    assert!(state.modal().is_none());
    assert!(matches!(state.screen(), Screen::Settings(_)));
    assert_eq!(state.settings_view().unwrap().field(skit_ui::RUNNER_KEY).unwrap().value().as_text(), "claude");
}

#[test]
fn test_settings_modal_cancel_leaves_the_picker_alone() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(prompt_settings(vec!["claude".to_owned(), "codex".to_owned()])))));
    let before = state.settings_view().unwrap().field(skit_ui::RUNNER_KEY).unwrap().clone();
    assert_eq!(state.update(Action::Settings(SettingsAction::NewRunner)), Effect::None);
    assert!(matches!(state.modal(), Some(ModalState::RunnerEditor { .. })));
    assert_eq!(state.update(Action::RunnerEditor(RunnerEditorAction::Cancel)), Effect::None);
    assert!(state.modal().is_none());
    assert_eq!(state.settings_view().unwrap().field(skit_ui::RUNNER_KEY).unwrap(), &before);
    assert!(state.settings_view().unwrap().submitted_values().is_empty());
}

#[test]
fn test_form_modal_cancel_leaves_the_picker_alone() {
    let runners = vec!["claude".to_owned(), "codex".to_owned()];
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(run_form(&runners)))));
    let before = state.run_form().unwrap().fields()[0].clone();
    assert_eq!(state.update(Action::OpenRunRunnerEditor), Effect::None);
    assert!(matches!(state.modal(), Some(ModalState::RunnerEditor { .. })));
    assert_eq!(state.update(Action::RunnerEditor(RunnerEditorAction::Cancel)), Effect::None);
    assert!(state.modal().is_none());
    assert_eq!(state.run_form().unwrap().fields()[0], before);
}

#[test]
fn test_form_ctrl_n_defines_a_custom_agent_and_runs_with_it() {
    let config_root = TempDir::new().unwrap();
    let config = FileConfigStore::new(config_root.path());
    let runners = config.runners().unwrap().into_iter().map(|runner| runner.name).collect::<Vec<_>>();
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(run_form(&runners)))));
    assert_eq!(state.update(Action::OpenRunRunnerEditor), Effect::None);
    let owner = RunnerEditorOwner::Run { selector: "p".to_owned() };
    let request = new_runner_effect(&mut state, owner.clone(), "aider", "aider --message {{prompt}}");
    save_new_runner(&config, &request);
    assert!(config.runners().unwrap().iter().any(|runner| runner.name == "aider"));
    let raw = std::fs::read_to_string(config_root.path().join("config.toml")).unwrap();
    assert!(raw.contains("runners_seeded = true"), "new runner did not materialize built-in seeds: {raw}");
    assert_eq!(
        state.update(Action::RunnerEditorSaved {
            owner,
            name: "aider".to_owned(),
            message: "saved".to_owned(),
        }),
        Effect::None
    );
    let runner_field = &state.run_form().unwrap().fields()[0];
    assert_eq!(runner_field.control.value(), "aider");
    let skit_ui::FormControl::Choice(choice) = &runner_field.control else { panic!("runner stopped being a picker") };
    assert!(choice.options.iter().any(|name| name == "aider"));

    let Effect::Submit { purpose: FormPurpose::Run, values, .. } = state.update(Action::Submit) else {
        panic!("run form did not submit after adding a runner")
    };
    assert_eq!(
        values.get("_skit_runner"),
        Some(&FieldValue::Explicit(TypedValue::Choice("aider".to_owned())))
    );
}

#[test]
fn test_review_ctrl_n_defines_a_custom_agent_and_selects_it() {
    let config_root = TempDir::new().unwrap();
    let config = FileConfigStore::new(config_root.path());
    let runners = config.runners().unwrap().into_iter().map(|runner| runner.name).collect::<Vec<_>>();
    let review = prompt_review(runners);
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(AddWorkflowState::from_review(review)))));
    assert_eq!(state.update(Action::OpenAddRunnerEditor), Effect::None);
    let owner = RunnerEditorOwner::Add;
    let request = new_runner_effect(&mut state, owner.clone(), "aider", "aider --message {{prompt}}");
    save_new_runner(&config, &request);
    assert!(config.runners().unwrap().iter().any(|runner| runner.name == "aider"));

    assert_eq!(
        state.update(Action::RunnerEditorSaved {
            owner,
            name: "aider".to_owned(),
            message: "saved".to_owned(),
        }),
        Effect::None
    );
    let review = state.add_workflow().unwrap().review().unwrap();
    assert_eq!(review.runner(), "aider");
    assert!(review.runner_was_picked(), "a runner defined from review was not marked as an active pick");
    assert!(review.runner_names().contains(&"aider".to_owned()));

    let Effect::Add(effects) = state.update(Action::Add(AddAction::Save)) else {
        panic!("review did not emit its atomic add request")
    };
    let [AddEffect::Commit { entry, .. }] = effects.as_slice() else {
        panic!("review save emitted something other than one commit: {effects:?}")
    };
    assert_eq!(entry.settings.runner, "aider");
    assert_eq!(entry.settings.params, ["a"]);
}

#[test]
fn test_review_modal_cancel_leaves_the_picker_alone() {
    let initial = vec!["claude".to_owned(), "codex".to_owned()];
    let review = prompt_review(initial.clone());
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(AddWorkflowState::from_review(review)))));
    let before_runner = state.add_workflow().unwrap().review().unwrap().runner().to_owned();
    let before_names = state.add_workflow().unwrap().review().unwrap().runner_names().to_vec();

    assert_eq!(state.update(Action::OpenAddRunnerEditor), Effect::None);
    assert!(matches!(state.modal(), Some(ModalState::RunnerEditor { owner: RunnerEditorOwner::Add, .. })));
    assert_eq!(state.update(Action::RunnerEditor(RunnerEditorAction::Cancel)), Effect::None);
    assert!(state.modal().is_none());
    let review = state.add_workflow().unwrap().review().unwrap();
    assert_eq!(review.runner(), before_runner);
    assert_eq!(review.runner_names(), before_names);
    assert!(!review.runner_was_picked());
}

#[test]
fn test_runner_modal_validation_covers_every_refusal() {
    let config_root = TempDir::new().unwrap();
    let config = FileConfigStore::new(config_root.path());
    config.ensure_runners_seeded().unwrap();
    let before = std::fs::read(config_root.path().join("config.toml")).unwrap();

    let mut editor = skit_ui::RunnerEditorView::new();
    assert_eq!(editor.reduce(RunnerEditorAction::Submit), skit_ui::RunnerEditorEffect::None);
    assert_eq!(editor.error(), Some(&RunnerEditorError::NameRequired));

    editor.reduce(RunnerEditorAction::SetName("mycli".to_owned()));
    for (command, expected) in [
        ("", RunnerEditorError::EmptyCommand),
        ("mycli run", RunnerEditorError::PromptSlotCount),
        ("{{prompt}}", RunnerEditorError::PromptInProgram),
        ("mycli {{prompt}} {{extra}}", RunnerEditorError::UnsupportedHole),
        ("mycli \"run {{prompt}}", RunnerEditorError::UnbalancedQuotes),
    ] {
        editor.reduce(RunnerEditorAction::SetCommand(command.to_owned()));
        assert_eq!(editor.reduce(RunnerEditorAction::Submit), skit_ui::RunnerEditorEffect::None, "command unexpectedly passed validation: {command}");
        assert_eq!(editor.error(), Some(&expected), "wrong refusal for {command}");
    }

    // Name collision is a host/store refusal rather than an editor-grammar refusal. Exercise the
    // same non-replacing mutation a New-agent modal requests and prove it neither overwrites nor
    // creates rows.
    let collision = config.set_runner(
        PromptRunner { name: "claude".to_owned(), argv: vec!["claude".to_owned(), "{{prompt}}".to_owned()] },
        false,
    ).expect_err("a seeded runner name was silently replaced");
    let shown = collision.to_string();
    assert!(shown.contains("already") || shown.contains("another row"), "duplicate-name refusal lost its actionable reason: {shown}");
    assert_eq!(std::fs::read(config_root.path().join("config.toml")).unwrap(), before, "duplicate-name refusal mutated runner config");
    assert!(config.runners().unwrap().iter().all(|runner| runner.name != "mycli"));
}
