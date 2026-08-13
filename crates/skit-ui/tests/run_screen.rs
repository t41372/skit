use std::collections::{BTreeMap, BTreeSet};

use skit_application::{LibraryScan, form_feedback::GlobCountRequest, tokens::TokenContext};
use skit_domain::{
    EntryKind, EntrySummary, Slug, StorageMode,
    parameters::{ParamDecl, ParameterType, ParameterValue},
};
use skit_ui::{
    Action, CommandContext, Effect, FormControl, FormInputKind, FormPurpose, LibraryState,
    ModalState, RunFieldRole, RunFormContext, RunFormOptions, RunFormView, RunPathContext,
    RunPathInsertMode, RunTokenOption, RunValidationError, RunnerEditorOwner, Screen, UiCommand,
    UiKey, command_specs,
};
use skit_ui::{FieldValue, TypedValue};

fn entry(slug: &str, name: &str, description: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        description: description.to_owned(),
        target: None,
    }
}

#[test]
fn run_screen_preserves_parameter_widget_semantics() {
    let mut enabled = ParamDecl::new("enabled");
    enabled.parameter_type = ParameterType::Bool;
    enabled.default = Some(ParameterValue::Bool(true));
    enabled.prompt = "Enable upload?".to_owned();

    let mut format = ParamDecl::new("format");
    format.parameter_type = ParameterType::Choice;
    format.choices = vec!["json".to_owned(), "yaml".to_owned()];
    format.default = Some(ParameterValue::String("json".to_owned()));

    let mut token = ParamDecl::new("token");
    token.secret = true;
    token.env_source = "API_TOKEN".to_owned();

    let mut output = ParamDecl::new("output");
    output.parameter_type = ParameterType::Path;
    output.help = "Destination file".to_owned();

    let values = BTreeMap::from([
        ("enabled".to_owned(), "false".to_owned()),
        ("format".to_owned(), "yaml".to_owned()),
        ("output".to_owned(), "build/result.json".to_owned()),
    ]);
    let view = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[enabled, format, token, output],
        &values,
        &["claude".to_owned(), "codex".to_owned()],
        "codex",
        &BTreeMap::from([(
            "daily".to_owned(),
            BTreeMap::from([("format".to_owned(), "json".to_owned())]),
        )]),
        "--verbose",
    );

    assert_eq!(view.purpose(), FormPurpose::Run);
    assert_eq!(view.fields().len(), 7);
    assert!(matches!(view.fields()[0].role, RunFieldRole::Runner));
    assert!(matches!(
        &view.fields()[0].control,
        FormControl::Choice(choice)
            if choice.options == ["claude", "codex"] && choice.selected == "codex"
    ));
    assert!(matches!(view.fields()[1].role, RunFieldRole::Preset));
    assert!(matches!(
        &view.fields()[2].control,
        FormControl::Checkbox { checked: false }
    ));
    assert!(matches!(
        &view.fields()[3].control,
        FormControl::Choice(choice)
            if choice.options == ["json", "yaml"] && choice.selected == "yaml"
    ));
    assert!(matches!(
        &view.fields()[4].control,
        FormControl::Text(text) if text.secret && text.value.is_empty()
    ));
    assert_eq!(view.fields()[4].environment_source(), Some("API_TOKEN"));
    assert!(matches!(
        &view.fields()[5].control,
        FormControl::Text(text)
            if text.kind == FormInputKind::Path && text.value == "build/result.json"
    ));
    assert!(matches!(
        view.fields()[6].role,
        RunFieldRole::ExtraArguments
    ));
}

#[test]
fn reducer_edits_and_submits_typed_run_controls() {
    let mut enabled = ParamDecl::new("enabled");
    enabled.parameter_type = ParameterType::Bool;
    let mut format = ParamDecl::new("format");
    format.parameter_type = ParameterType::Choice;
    format.choices = vec!["json".to_owned(), "yaml".to_owned()];
    let output = ParamDecl::new("output");

    let view = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[enabled, format, output],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(view))));

    assert_eq!(state.focused_form_field(), Some(0));
    state.update(Action::ToggleField(0));
    state.update(Action::SelectFieldOption {
        field: 1,
        value: "yaml".to_owned(),
    });
    state.update(Action::SetFieldValue {
        field: 2,
        value: "report.json".to_owned(),
    });
    state.update(Action::SetFieldValue {
        field: 3,
        value: "--force".to_owned(),
    });

    assert_eq!(
        state.update(Action::Submit),
        Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("demo".to_owned()),
            values: BTreeMap::from([
                ("value:enabled".to_owned(), FieldValue::boolean(true)),
                (
                    "value:format".to_owned(),
                    FieldValue::Explicit(TypedValue::Choice("yaml".to_owned())),
                ),
                ("value:output".to_owned(), FieldValue::text("report.json")),
                ("_skit_args".to_owned(), FieldValue::text("--force")),
                ("_skit_save_preset".to_owned(), FieldValue::text("")),
                ("_skit_dry_run".to_owned(), FieldValue::text("false")),
            ]),
        }
    );
}

#[test]
fn run_focus_uses_the_complete_control_order() {
    let view = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[ParamDecl::new("name")],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::from([("daily".to_owned(), BTreeMap::new())]),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(view))));

    assert_eq!(state.command_context(), CommandContext::RunForm);
    let submit = command_specs(CommandContext::RunForm)
        .find(|spec| spec.command == UiCommand::Submit)
        .unwrap();
    assert_eq!(submit.bindings[0].key, UiKey::Enter);

    // The form must be typeable the moment it opens, so the runner and preset pickers are
    // skipped at boot. Version 0.4 focuses `"Input, Checkbox, RadioSet"`, which no picker
    // matches (`src/skit/tui_form.py:566`).
    assert_eq!(state.focused_form_field(), Some(2));
    for expected in [3, 3] {
        state.update(Action::FocusNext);
        assert_eq!(state.focused_form_field(), Some(expected));
    }
    for expected in [2, 1, 0, 0] {
        state.update(Action::FocusPrevious);
        assert_eq!(state.focused_form_field(), Some(expected));
    }
}

#[test]
fn runner_picker_marks_selection_events_without_marking_its_default() {
    let view = RunFormView::from_declarations(
        "prompt",
        "Prompt",
        &[ParamDecl::new("topic")],
        &BTreeMap::new(),
        &["codex".to_owned(), "claude".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(view))));

    let Effect::Submit { values, .. } = state.update(Action::Submit) else {
        panic!("run form did not submit");
    };
    assert!(!values.contains_key("_skit_runner_picked"));

    state.update(Action::SelectFieldOption {
        field: 0,
        value: "claude".to_owned(),
    });
    state.update(Action::SelectFieldOption {
        field: 0,
        value: "codex".to_owned(),
    });
    let Effect::Submit { values, .. } = state.update(Action::Submit) else {
        panic!("run form did not submit");
    };
    assert_eq!(
        values.get("_skit_runner_picked"),
        Some(&FieldValue::boolean(true))
    );
    assert_eq!(
        values.get("_skit_runner").map(FieldValue::as_text),
        Some("codex".to_owned())
    );
}

#[test]
fn run_focus_starts_on_the_first_typeable_control_in_every_shape() {
    let with_pickers = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[ParamDecl::new("name")],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::from([("daily".to_owned(), BTreeMap::new())]),
        "",
    );
    assert_eq!(with_pickers.focused(), 2);

    // No parameters means no preset row at all, so the extra-arguments input is first.
    let pickers_only = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::from([("daily".to_owned(), BTreeMap::new())]),
        "",
    );
    assert_eq!(pickers_only.focused(), 1);

    // Fixing every parameter removes its control, and focus must stay inside the form.
    let reduced = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[ParamDecl::new("name")],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::from([("daily".to_owned(), BTreeMap::new())]),
        "",
    )
    .with_options(RunFormOptions {
        fixed_values: BTreeMap::from([("name".to_owned(), "Ada".to_owned())]),
        ..RunFormOptions::default()
    });
    assert_eq!(reduced.focused(), 2);
}

#[test]
fn run_commands_match_the_latest_main_contract_and_target_typed_fields() {
    let mut output = ParamDecl::new("output");
    output.default = Some(ParameterValue::String("report.txt".to_owned()));
    let view = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[output],
        &BTreeMap::from([("output".to_owned(), "changed.txt".to_owned())]),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "prompt".to_owned(),
        path: Some(RunPathContext {
            workdir: "/work".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/alice".to_owned()),
            env: BTreeMap::new(),
            today: "2026-08-08".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(view))));

    let footer = command_specs(CommandContext::RunForm)
        .filter(|spec| spec.footer && state.command_enabled(spec.command))
        .map(|spec| spec.command)
        .collect::<Vec<_>>();
    assert_eq!(
        footer,
        [
            UiCommand::Submit,
            UiCommand::InsertValue,
            UiCommand::ResetDefault,
            UiCommand::SavePreset,
            UiCommand::Back,
            UiCommand::FocusNext,
            UiCommand::FocusPrevious,
        ]
    );

    // The runner picker is control 0, so the boot focus is already the first typed field.
    assert_eq!(state.focused_form_field(), Some(1));
    state.update(Action::ResetFocusedRunField);
    assert_eq!(
        state.run_form().unwrap().fields()[1].control.value(),
        "report.txt"
    );

    state.update(Action::OpenRunTokenMenu);
    let Some(ModalState::RunTokenMenu { field, options }) = state.modal() else {
        panic!("the insert command did not open its typed menu");
    };
    assert_eq!(*field, 1);
    assert!(matches!(
        options.first(),
        Some(RunTokenOption::RuntimeDirectory)
    ));
    assert!(
        matches!(options.get(1), Some(RunTokenOption::FixedDirectory { path }) if path == "/invoke")
    );
    assert!(options.contains(&RunTokenOption::FileOrFolder));

    state.update(Action::Back);
    state.update(Action::FocusPrevious);
    assert_eq!(state.update(Action::OpenRunRunnerEditor), Effect::None);
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunnerEditor {
            owner: RunnerEditorOwner::Run { selector },
            ..
        }) if selector == "demo"
    ));
}

#[test]
fn selecting_a_typed_preset_overlays_fields_and_last_values_restores_them() {
    let mut format = ParamDecl::new("format");
    format.parameter_type = ParameterType::Choice;
    format.choices = vec!["json".to_owned(), "yaml".to_owned()];
    let output = ParamDecl::new("output");
    let presets = BTreeMap::from([(
        "daily".to_owned(),
        BTreeMap::from([
            ("format".to_owned(), "json".to_owned()),
            ("output".to_owned(), "daily.json".to_owned()),
        ]),
    )]);
    let view = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[format, output],
        &BTreeMap::from([
            ("format".to_owned(), "yaml".to_owned()),
            ("output".to_owned(), "last.json".to_owned()),
        ]),
        &[],
        "",
        &presets,
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(view))));

    state.update(Action::SelectFieldOption {
        field: 0,
        value: "daily".to_owned(),
    });
    assert_eq!(
        state.run_form().unwrap().fields()[1].control.value(),
        "json"
    );
    assert_eq!(
        state.run_form().unwrap().fields()[2].control.value(),
        "daily.json"
    );

    state.update(Action::SelectFieldOption {
        field: 0,
        value: String::new(),
    });
    assert_eq!(
        state.run_form().unwrap().fields()[1].control.value(),
        "yaml"
    );
    assert_eq!(
        state.run_form().unwrap().fields()[2].control.value(),
        "last.json"
    );
}

#[test]
fn inline_run_options_are_submission_metadata_not_visible_string_fields() {
    let view = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[ParamDecl::new("name")],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "--remembered",
    )
    .with_options(RunFormOptions {
        selected_preset: String::new(),
        save_preset: "snapshot".to_owned(),
        dry_run: true,
        include_extra: false,
        fixed_values: BTreeMap::new(),
    });
    assert!(
        view.fields()
            .iter()
            .all(|field| !field.key.starts_with("_skit_"))
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(view))));

    let Effect::Submit { values, .. } = state.update(Action::Submit) else {
        panic!("run form did not submit");
    };
    assert_eq!(
        values
            .get("_skit_save_preset")
            .map(FieldValue::as_text)
            .as_deref(),
        Some("snapshot")
    );
    assert_eq!(
        values
            .get("_skit_dry_run")
            .map(FieldValue::as_text)
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        values.get("_skit_args").map(FieldValue::as_text).as_deref(),
        Some("--remembered")
    );
}

#[test]
fn inline_fixed_values_win_over_presets_and_are_not_editable() {
    let fields = [ParamDecl::new("name"), ParamDecl::new("output")];
    let view = RunFormView::from_declarations(
        "demo",
        "Demo",
        &fields,
        &BTreeMap::from([
            ("name".to_owned(), "remembered".to_owned()),
            ("output".to_owned(), "last.txt".to_owned()),
        ]),
        &[],
        "",
        &BTreeMap::from([(
            "daily".to_owned(),
            BTreeMap::from([
                ("name".to_owned(), "preset".to_owned()),
                ("output".to_owned(), "daily.txt".to_owned()),
            ]),
        )]),
        "",
    )
    .with_options(RunFormOptions {
        selected_preset: "daily".to_owned(),
        fixed_values: BTreeMap::from([("name".to_owned(), "explicit".to_owned())]),
        ..RunFormOptions::default()
    });
    assert!(view.fields().iter().all(|field| field.key != "value:name"));
    assert_eq!(view.fields()[1].control.value(), "daily.txt");
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(view))));

    let Effect::Submit { values, .. } = state.update(Action::Submit) else {
        panic!("run form did not submit");
    };
    assert_eq!(
        values.get("value:name").map(FieldValue::as_text).as_deref(),
        Some("explicit")
    );
    assert_eq!(
        values
            .get("value:output")
            .map(FieldValue::as_text)
            .as_deref(),
        Some("daily.txt")
    );
}

#[test]
fn fuzzy_search_uses_unicode_normalization() {
    let mut state = LibraryState::from_scan(LibraryScan {
        entries: vec![
            entry("resume", "Résumé Builder", "Generate a CV"),
            entry("archive", "Archive", "Old files"),
        ],
        diagnostics: Vec::new(),
    });
    state.update(Action::BeginSearch);
    for character in "resume".chars() {
        state.update(Action::Input(character));
    }

    assert_eq!(state.visible_entries().len(), 1);
    assert_eq!(state.selected().unwrap().slug.as_str(), "resume");
}

#[test]
fn run_submit_keeps_typed_validation_errors_on_the_frontend_neutral_fields() {
    let mut count = ParamDecl::new("count");
    count.parameter_type = ParameterType::Int;
    count.required = true;
    count.prompt = "Count".to_owned();
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[count],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));

    assert_eq!(state.update(Action::Submit), Effect::None);
    assert_eq!(
        state.run_form().unwrap().fields()[0].validation_error,
        Some(RunValidationError::Required)
    );

    state.update(Action::SetFieldValue {
        field: 0,
        value: "one".to_owned(),
    });
    assert_eq!(state.update(Action::Submit), Effect::None);
    assert_eq!(
        state.run_form().unwrap().fields()[0].validation_error,
        Some(RunValidationError::InvalidType)
    );

    state.update(Action::SetFieldValue {
        field: 0,
        value: "1".to_owned(),
    });
    assert!(matches!(
        state.update(Action::Submit),
        Effect::Submit { .. }
    ));
}

#[test]
fn reset_restores_only_defaults_that_the_main_form_can_represent() {
    let mut count = ParamDecl::new("count");
    count.parameter_type = ParameterType::Int;
    count.default = Some(ParameterValue::Integer(3));
    let mut secret = ParamDecl::new("token");
    secret.secret = true;
    secret.default = Some(ParameterValue::String("do-not-echo".to_owned()));
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[count, secret],
        &BTreeMap::from([("count".to_owned(), "8".to_owned())]),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state.update(Action::ResetRunField(0));
    state.update(Action::ResetRunField(1));

    let fields = state.run_form().unwrap().fields();
    assert_eq!(fields[0].control.value(), "3");
    assert!(fields[0].resettable());
    assert_eq!(fields[1].control.value(), "");
    assert!(!fields[1].resettable());
}

#[test]
fn live_feedback_expands_tokens_and_requests_glob_counts_through_a_typed_port() {
    let mut paths = ParamDecl::new("paths");
    paths.multiple = true;
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[paths],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: "/child".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/demo".to_owned()),
            env: BTreeMap::new(),
            today: "2026-08-08".to_owned(),
            now: "10-11-12".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));

    let effect = state.update(Action::SetFieldValue {
        field: 0,
        value: "{cwd}/*.rs".to_owned(),
    });
    assert_eq!(
        effect,
        Effect::CountRunGlob {
            selector: "demo".to_owned(),
            field: 0,
            value: "{cwd}/*.rs".to_owned(),
            request: GlobCountRequest {
                cwd: "/invoke".to_owned(),
                pieces: vec!["{cwd}/*.rs".to_owned()],
            },
        }
    );
    assert_eq!(
        state.run_form().unwrap().fields()[0]
            .feedback
            .expanded
            .as_deref(),
        Some("/invoke/*.rs")
    );

    state.update(Action::SetRunGlobCount {
        field: 0,
        value: "stale".to_owned(),
        count: 9,
    });
    assert_eq!(
        state.run_form().unwrap().fields()[0].feedback.glob_count,
        None
    );
    state.update(Action::SetRunGlobCount {
        field: 0,
        value: "{cwd}/*.rs".to_owned(),
        count: 2,
    });
    assert_eq!(
        state.run_form().unwrap().fields()[0].feedback.glob_count,
        Some(2)
    );
}

#[test]
fn preset_name_modal_saves_a_nonsecret_snapshot_and_refreshes_the_picker_in_place() {
    let mut name = ParamDecl::new("name");
    name.default = Some(ParameterValue::String("Ada".to_owned()));
    let mut token = ParamDecl::new("token");
    token.secret = true;
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[name, token],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));

    assert_eq!(state.update(Action::OpenRunPresetSave), Effect::None);
    assert_eq!(
        state.modal(),
        Some(&ModalState::RunPresetName {
            value: String::new(),
            existing: BTreeSet::new(),
        })
    );
    state.update(Action::SetModalInput("daily".to_owned()));
    assert_eq!(
        state.update(Action::Submit),
        Effect::SaveRunPreset {
            selector: "demo".to_owned(),
            name: "daily".to_owned(),
            values: BTreeMap::from([("name".to_owned(), "Ada".to_owned())]),
            secret_names: BTreeSet::from(["token".to_owned()]),
        }
    );

    state.update(Action::RunPresetSaved {
        name: "daily".to_owned(),
        presets: BTreeMap::from([(
            "daily".to_owned(),
            BTreeMap::from([("name".to_owned(), "Ada".to_owned())]),
        )]),
        message: "saved".to_owned(),
    });
    assert_eq!(state.modal(), None);
    let form = state.run_form().unwrap();
    assert_eq!(form.fields()[0].control.value(), "daily");
    assert_eq!(form.fields()[1].control.value(), "Ada");
    assert_eq!(state.status(), Some("saved"));
}

#[test]
fn picked_paths_use_the_field_shape_and_never_share_one_quoting_shortcut() {
    let mut scalar = ParamDecl::new("output");
    scalar.parameter_type = ParameterType::Path;
    let mut multiple = ParamDecl::new("inputs");
    multiple.multiple = true;
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[scalar, multiple],
        &BTreeMap::from([("inputs".to_owned(), "--before".to_owned())]),
        &[],
        "",
        &BTreeMap::new(),
        "--verbose",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: "/work/project".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: None,
            env: BTreeMap::new(),
            today: "2026-08-08".to_owned(),
            now: "10-11-12".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));

    state.update(Action::OpenRunTokenMenu);
    state.update(Action::OpenRunFilePicker(0));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 0,
            mode: RunPathInsertMode::Replace,
            ..
        })
    ));
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: 0,
        path: "reports/final file*.txt".to_owned(),
    });
    assert_eq!(
        state.run_form().unwrap().fields()[0].control.value(),
        "reports/final file*.txt"
    );

    state.update(Action::FocusField(1));
    state.update(Action::OpenRunTokenMenu);
    state.update(Action::OpenRunFilePicker(1));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 1,
            mode: RunPathInsertMode::Shlex,
            ..
        })
    ));
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: 1,
        path: "data sets/data?.csv".to_owned(),
    });
    #[cfg(not(windows))]
    assert_eq!(
        state.run_form().unwrap().fields()[1].control.value(),
        "--before 'data sets/data[?].csv'"
    );

    state.update(Action::FocusField(2));
    state.update(Action::OpenRunTokenMenu);
    state.update(Action::OpenRunFilePicker(2));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 2,
            mode: RunPathInsertMode::Arguments,
            ..
        })
    ));
}
