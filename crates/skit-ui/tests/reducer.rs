use std::collections::BTreeMap;

use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration, PreferencesDraft,
    PreferencesSnapshot,
};
use skit_application::{Diagnostic, DiagnosticCode, LibraryScan, SourcePermissions};
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_ui::{
    Action, AddAction, AddEffect, AddWorkflowState, CommandContext, DetailPaneMode, DraftSummary,
    Effect, FormControl, FormField, FormPurpose, FormView, HealthAction, HealthIssue,
    HealthIssueKind, HealthSnapshot, HealthView, HostRequest, InputMode, KnownEntryKind,
    LibraryState, MirrorHealth, ModalState, PreferencesAction, PreferencesView, ReportItem,
    ReportView, ReviewDefaults, ReviewState, RunFormView, RunnerEditorAction, RunnerEditorOwner,
    RunnerManagerAction, RunnerManagerView, RunnerSaveOwner, Screen, SourceSnapshot, UiCommand,
    UvHealth, command_specs,
};

fn entry_with_kind(slug: &str, name: &str, kind: &str, description: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode: StorageMode::Copy,
        description: description.to_owned(),
        target: None,
    }
}

fn entry(slug: &str, name: &str, description: &str) -> EntrySummary {
    entry_with_kind(slug, name, "command", description)
}

fn state() -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries: vec![
            entry("alpha", "Alpha", "first"),
            entry("beta", "Beta", "second"),
            entry("gamma", "Gamma", "third"),
        ],
        diagnostics: Vec::new(),
    })
}

fn preferences_view() -> PreferencesView {
    preferences_view_with_runners(Vec::new())
}

fn preferences_view_with_runners(runner_names: Vec<String>) -> PreferencesView {
    PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
        effective_language: "en".to_owned(),
        editor: String::new(),
        editor_fallback: Some("vim".to_owned()),
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Exit,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runner_names,
        mirror: MirrorConfiguration::default(),
    }))
}

#[test]
fn preferences_close_is_local_and_dirty_edits_use_a_typed_discard_guard() {
    let mut state = state();
    state.update(Action::Present(Screen::Preferences(Box::new(
        preferences_view(),
    ))));

    assert_eq!(
        state.update(Action::Preferences(PreferencesAction::Close)),
        Effect::None
    );
    assert_eq!(state.screen(), &Screen::Library);

    state.update(Action::Present(Screen::Preferences(Box::new(
        preferences_view(),
    ))));
    state.update(Action::Preferences(PreferencesAction::SetEditor(
        "micro".to_owned(),
    )));
    assert_eq!(
        state.update(Action::Preferences(PreferencesAction::Close)),
        Effect::None
    );
    assert_eq!(state.modal(), Some(&ModalState::ConfirmDiscardChanges));
    assert!(matches!(state.screen(), Screen::Preferences(_)));

    state.update(Action::KeepEditing);
    assert_eq!(state.modal(), None);
    assert!(matches!(state.screen(), Screen::Preferences(_)));

    state.update(Action::Preferences(PreferencesAction::Close));
    state.update(Action::DiscardChanges);
    assert_eq!(state.modal(), None);
    assert_eq!(state.screen(), &Screen::Library);
}

#[test]
fn a_saved_preferences_transaction_returns_to_library_with_a_separate_locale_tag() {
    let mut state = state();
    state.update(Action::Present(Screen::Preferences(Box::new(
        preferences_view(),
    ))));

    assert_eq!(
        state.update(Action::PreferencesSaved {
            locale: "zh-TW".to_owned(),
            message: "偏好設定已儲存".to_owned(),
        }),
        Effect::None
    );
    assert_eq!(state.screen(), &Screen::Library);
    assert_eq!(state.modal(), None);
    assert_eq!(state.input_mode(), InputMode::Browse);
    assert_eq!(state.status(), Some("偏好設定已儲存"));
}

#[test]
fn an_added_slug_is_selected_after_the_authoritative_reload() {
    let mut state = state();
    state.update(Action::BeginSearch);
    state.update(Action::SetSearchQuery("new".to_owned()));
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    let added = Slug::parse("new-tool").unwrap();

    assert_eq!(
        state.update(Action::AddCompleted {
            scan: LibraryScan {
                entries: vec![
                    entry("alpha", "Alpha", "old"),
                    entry("new-tool", "New Tool", "new"),
                ],
                diagnostics: Vec::new(),
            },
            rerunnable: vec![added.clone()],
            slug: added,
            message: "Added".to_owned(),
        }),
        Effect::None
    );
    assert_eq!(state.screen(), &Screen::Library);
    assert_eq!(state.input_mode(), InputMode::Browse);
    assert_eq!(state.query(), "new");
    assert_eq!(state.selected().unwrap().slug.as_str(), "new-tool");
    assert_eq!(state.status(), Some("Added"));
    assert!(state.command_enabled(UiCommand::Rerun));
}

#[test]
fn a_typed_add_cancel_returns_to_the_library_without_guessing_from_status() {
    let mut state = state();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    state.update(Action::SetStatus("unchanged".to_owned()));

    assert_eq!(state.update(Action::AddCancelled), Effect::None);
    assert_eq!(state.screen(), &Screen::Library);
    assert_eq!(state.input_mode(), InputMode::Browse);
    assert_eq!(state.status(), Some("unchanged"));
}

#[test]
fn navigation_is_clamped_and_never_points_outside_the_filtered_list() {
    let mut state = state();
    assert_eq!(state.selected().unwrap().slug.as_str(), "alpha");

    assert_eq!(state.update(Action::Previous), Effect::None);
    assert_eq!(state.selected().unwrap().slug.as_str(), "alpha");

    state.update(Action::End);
    state.update(Action::Next);
    assert_eq!(state.selected().unwrap().slug.as_str(), "gamma");

    state.update(Action::Home);
    assert_eq!(state.selected().unwrap().slug.as_str(), "alpha");
}

#[test]
fn page_navigation_is_saturating() {
    let mut state = state();
    state.update(Action::PageNext);
    assert_eq!(state.selected().unwrap().slug.as_str(), "gamma");
    state.update(Action::PagePrevious);
    assert_eq!(state.selected().unwrap().slug.as_str(), "alpha");
}

#[test]
fn search_is_an_explicit_mode_and_filters_across_visible_fields() {
    let mut state = LibraryState::from_scan(LibraryScan {
        entries: vec![
            entry_with_kind("alpha-tool", "Alpha", "command", "first"),
            entry_with_kind("beta", "Beta", "python", "second"),
            entry_with_kind("gamma", "Gamma", "shell", "third needle"),
        ],
        diagnostics: Vec::new(),
    });
    assert_eq!(state.input_mode(), InputMode::Browse);

    state.update(Action::Input('x'));
    assert!(state.query().is_empty());

    state.update(Action::BeginSearch);
    state.update(Action::Input('s'));
    state.update(Action::Input('e'));
    state.update(Action::Input('c'));

    assert_eq!(state.input_mode(), InputMode::Search);
    assert_eq!(state.query(), "sec");
    assert_eq!(state.visible_entries().len(), 1);
    assert_eq!(state.selected().unwrap().slug.as_str(), "beta");

    state.update(Action::Backspace);
    state.update(Action::FinishSearch);
    assert_eq!(state.input_mode(), InputMode::Browse);
    assert_eq!(state.query(), "se");

    for query in ["alpha-tool", "python", "needle", "gamma"] {
        state.update(Action::BeginSearch);
        state.update(Action::ClearSearch);
        for character in query.chars() {
            state.update(Action::Input(character));
        }
        assert_eq!(state.visible_entries().len(), 1, "query {query:?}");
        state.update(Action::FinishSearch);
    }
}

#[test]
fn search_keeps_the_v040_case_insensitive_subsequence_contract() {
    let mut state = LibraryState::from_scan(LibraryScan {
        entries: vec![
            entry("create-task", "Create Task", "work item"),
            entry("archive", "Archive", "completed work"),
        ],
        diagnostics: Vec::new(),
    });

    state.update(Action::BeginSearch);
    for character in "CTa".chars() {
        state.update(Action::Input(character));
    }

    assert_eq!(state.visible_entries().len(), 1);
    assert_eq!(state.selected().unwrap().slug.as_str(), "create-task");

    state.update(Action::ClearSearch);
    for character in "tac".chars() {
        state.update(Action::Input(character));
    }
    assert!(state.visible_entries().next().is_none());
}

#[test]
fn clearing_and_empty_backspace_keep_selection_valid() {
    let mut state = state();
    state.update(Action::BeginSearch);
    state.update(Action::Backspace);
    state.update(Action::Input('z'));
    assert!(state.selected().is_none());
    assert_eq!(state.selected_visible_index(), None);

    state.update(Action::Previous);
    state.update(Action::Next);
    state.update(Action::Home);
    state.update(Action::End);
    assert!(state.selected().is_none());

    state.update(Action::ClearSearch);
    assert_eq!(state.selected().unwrap().slug.as_str(), "alpha");
}

#[test]
fn replacing_the_library_preserves_selection_when_possible() {
    let mut state = state();
    state.update(Action::Next);
    assert_eq!(state.selected().unwrap().slug.as_str(), "beta");

    state.update(Action::Replace {
        scan: LibraryScan {
            entries: vec![
                entry("beta", "Beta renamed", "still here"),
                entry("delta", "Delta", "new"),
            ],
            diagnostics: Vec::new(),
        },
        rerunnable: vec![Slug::parse("beta").unwrap()],
    });

    assert_eq!(state.selected().unwrap().slug.as_str(), "beta");
}

#[test]
fn replacing_without_the_old_selection_falls_back_to_the_first_visible_entry() {
    let mut state = state();
    state.update(Action::End);
    state.update(Action::Replace {
        scan: LibraryScan {
            entries: vec![entry("delta", "Delta", "new")],
            diagnostics: Vec::new(),
        },
        rerunnable: vec![],
    });
    assert_eq!(state.selected().unwrap().slug.as_str(), "delta");
}

#[test]
fn effects_and_status_are_frontend_neutral() {
    let mut state = state();
    assert_eq!(state.update(Action::Reload), Effect::Reload);
    assert_eq!(state.update(Action::Quit), Effect::Quit);

    state.update(Action::SetStatus("reloaded".to_owned()));
    assert_eq!(state.status(), Some("reloaded"));
    state.update(Action::ClearStatus);
    assert_eq!(state.status(), None);
}

#[test]
fn rerun_is_a_distinct_frontend_neutral_effect() {
    let mut state = state();
    assert_eq!(
        state.update(Action::Rerun),
        Effect::Rerun {
            selector: "alpha".to_owned(),
        }
    );

    let mut empty = LibraryState::default();
    assert_eq!(empty.update(Action::Rerun), Effect::None);
}

#[test]
fn search_navigation_paste_and_run_preserve_the_v040_focus_contract() {
    let mut state = state();
    state.update(Action::BeginSearch);
    state.update(Action::Paste("a".to_owned()));
    assert_eq!(state.query(), "a");
    assert_eq!(state.selected().unwrap().slug.as_str(), "alpha");

    state.update(Action::Next);
    assert_eq!(state.selected().unwrap().slug.as_str(), "beta");
    state.update(Action::Previous);
    assert_eq!(state.selected().unwrap().slug.as_str(), "alpha");

    assert_eq!(
        state.update(Action::OpenRun),
        Effect::Open {
            request: HostRequest::Run,
            selector: Some("alpha".to_owned()),
        }
    );
    assert_eq!(state.input_mode(), InputMode::Browse);
}

#[test]
fn workflow_modal_and_detail_state_are_explicit_and_serializable() {
    let mut state = state();
    assert_eq!(state.modal(), None);
    assert_eq!(state.detail_pane_mode(), DetailPaneMode::Automatic);

    state.update(Action::OpenHelp);
    assert_eq!(state.modal(), Some(&ModalState::Help));
    assert_eq!(state.screen(), &Screen::Library);

    state.update(Action::ToggleDetail);
    assert_eq!(state.detail_pane_mode(), DetailPaneMode::PinnedClosed);
    state.update(Action::ToggleDetail);
    assert_eq!(state.detail_pane_mode(), DetailPaneMode::PinnedOpen);

    let encoded = serde_json::to_string(&state).unwrap();
    assert_eq!(
        serde_json::from_str::<LibraryState>(&encoded).unwrap(),
        state
    );

    state.update(Action::Back);
    assert_eq!(state.modal(), None);
}

#[test]
fn one_command_registry_defines_library_search_and_modal_surfaces() {
    let browse = command_specs(CommandContext::LibraryBrowse).collect::<Vec<_>>();
    let search = command_specs(CommandContext::LibrarySearch).collect::<Vec<_>>();
    let help = command_specs(CommandContext::Help).collect::<Vec<_>>();

    for command in [
        UiCommand::Run,
        UiCommand::Rerun,
        UiCommand::Settings,
        UiCommand::Presets,
        UiCommand::Edit,
        UiCommand::Remove,
        UiCommand::Add,
        UiCommand::Health,
        UiCommand::Help,
        UiCommand::ToggleDetail,
    ] {
        assert!(
            browse.iter().any(|spec| spec.command == command),
            "missing browse command {command:?}"
        );
    }
    assert_eq!(
        search
            .iter()
            .filter(|spec| spec.footer)
            .map(|spec| spec.command)
            .collect::<Vec<_>>(),
        [UiCommand::Run, UiCommand::LeaveSearch]
    );
    assert_eq!(
        help.iter().map(|spec| spec.command).collect::<Vec<_>>(),
        [UiCommand::CloseModal]
    );
}

#[test]
fn diagnostics_remain_available_to_every_frontend() {
    let diagnostic = Diagnostic::plain(
        DiagnosticCode::CorruptMetadata,
        Some("bad".to_owned()),
        "bad TOML".to_owned(),
    );
    let state = LibraryState::from_scan(LibraryScan {
        entries: Vec::new(),
        diagnostics: vec![diagnostic.clone()],
    });
    assert_eq!(state.diagnostics(), [diagnostic]);
}

#[test]
fn direct_row_selection_uses_a_visible_index() {
    let mut state = state();
    state.update(Action::SelectVisible(2));
    assert_eq!(state.selected().unwrap().slug.as_str(), "gamma");
    state.update(Action::SelectVisible(999));
    assert_eq!(state.selected().unwrap().slug.as_str(), "gamma");
}

#[test]
fn library_commands_request_host_data_without_embedding_an_adapter() {
    let mut state = state();
    let selected = Some("alpha".to_owned());

    let cases = [
        (Action::OpenRun, HostRequest::Run),
        (Action::OpenSettings, HostRequest::Settings),
        (Action::OpenPresets, HostRequest::Presets),
        (Action::OpenRename, HostRequest::Rename),
    ];
    for (action, request) in cases {
        assert_eq!(
            state.update(action),
            Effect::Open {
                request,
                selector: selected.clone(),
            }
        );
    }

    assert_eq!(
        state.update(Action::OpenAdd),
        Effect::Open {
            request: HostRequest::Add,
            selector: None,
        }
    );
    assert_eq!(
        state.update(Action::OpenPreferences),
        Effect::Open {
            request: HostRequest::Preferences,
            selector: None,
        }
    );
    assert_eq!(
        state.update(Action::OpenHealth),
        Effect::Open {
            request: HostRequest::Health,
            selector: None,
        }
    );
    assert_eq!(
        state.update(Action::OpenRunners),
        Effect::Open {
            request: HostRequest::Runners,
            selector: None,
        }
    );
    assert_eq!(
        state.update(Action::Edit),
        Effect::Edit {
            selector: "alpha".to_owned(),
        }
    );
}

#[test]
fn entry_commands_are_inert_when_the_library_has_no_selection() {
    let mut state = LibraryState::default();

    for action in [
        Action::OpenRun,
        Action::OpenSettings,
        Action::OpenPresets,
        Action::OpenRename,
        Action::Rerun,
        Action::Edit,
        Action::AskRemove,
    ] {
        assert_eq!(state.update(action), Effect::None);
    }
    assert_eq!(state.screen(), &Screen::Library);
}

#[test]
fn forms_are_frontend_neutral_editable_models_and_submit_typed_requests() {
    let mut state = state();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Run,
        title: "Run Alpha".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: Some("alpha".to_owned()),
        fields: vec![
            FormField::text("name", "Name", "old"),
            FormField::secret("token", "Token", ""),
        ],
        focused: 0,
        submit_label: "Run".to_owned(),
    })));

    assert_eq!(state.input_mode(), InputMode::Form);
    state.update(Action::Backspace);
    state.update(Action::Input('!'));
    state.update(Action::FocusNext);
    state.update(Action::Input('x'));
    state.update(Action::FocusField(99));
    assert_eq!(state.form().unwrap().focused, 1);
    state.update(Action::FocusField(0));

    let Effect::Submit {
        purpose,
        selector,
        values,
    } = state.update(Action::Submit)
    else {
        panic!("the form must request a submission");
    };
    assert_eq!(purpose, FormPurpose::Run);
    assert_eq!(selector.as_deref(), Some("alpha"));
    assert_eq!(values["name"], "ol!");
    assert_eq!(values["token"], "x");

    state.update(Action::FocusPrevious);
    assert_eq!(state.form().unwrap().focused, 0);
    state.update(Action::Back);
    assert_eq!(state.screen(), &Screen::Library);
    assert_eq!(state.input_mode(), InputMode::Browse);
}

#[test]
fn reports_and_remove_confirmation_have_keyboard_and_mouse_ready_actions() {
    let mut state = state();
    state.update(Action::Present(Screen::Report(ReportView {
        title: "Health".to_owned(),
        items: vec![ReportItem {
            status: "ok".to_owned(),
            label: "Library".to_owned(),
            translate_label: true,
            detail: "Ready".to_owned(),
            translate_detail: true,
        }],
    })));
    assert!(matches!(state.screen(), Screen::Report(_)));
    state.update(Action::Back);
    assert_eq!(state.screen(), &Screen::Library);

    state.update(Action::AskRemove);
    assert!(matches!(
        state.modal(),
        Some(ModalState::ConfirmRemove { .. })
    ));
    assert_eq!(state.screen(), &Screen::Library);
    assert_eq!(
        state.update(Action::Submit),
        Effect::Remove {
            selector: "alpha".to_owned(),
        }
    );
    state.update(Action::Back);
    assert_eq!(state.screen(), &Screen::Library);
    assert_eq!(state.modal(), None);
}

#[test]
fn host_completion_returns_to_the_library_and_can_replace_the_scan() {
    let mut state = state();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Rename,
        title: "Rename".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: Some("alpha".to_owned()),
        fields: vec![],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    state.update(Action::Complete {
        scan: Some(LibraryScan {
            entries: vec![entry("renamed", "Renamed", "done")],
            diagnostics: vec![],
        }),
        rerunnable: Some(vec![Slug::parse("renamed").unwrap()]),
        message: "Saved".to_owned(),
    });

    assert_eq!(state.screen(), &Screen::Library);
    assert_eq!(state.status(), Some("Saved"));
    assert_eq!(state.selected().unwrap().slug.as_str(), "renamed");
}

#[test]
fn public_ui_contract_round_trips_through_json_for_a_future_tauri_adapter() {
    let action = Action::Complete {
        scan: Some(LibraryScan {
            entries: vec![entry("delta", "Delta", "new")],
            diagnostics: vec![],
        }),
        rerunnable: Some(vec![Slug::parse("delta").unwrap()]),
        message: "Saved".to_owned(),
    };
    let encoded = serde_json::to_string(&action).unwrap();
    assert_eq!(serde_json::from_str::<Action>(&encoded).unwrap(), action);

    let effect = Effect::Open {
        request: HostRequest::Settings,
        selector: Some("alpha".to_owned()),
    };
    let encoded = serde_json::to_string(&effect).unwrap();
    assert_eq!(serde_json::from_str::<Effect>(&encoded).unwrap(), effect);

    let screen = Screen::Form(FormView {
        purpose: FormPurpose::Preferences,
        title: "Preferences".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: None,
        fields: vec![FormField::text("lang", "Language", "auto")],
        focused: 0,
        submit_label: "Save".to_owned(),
    });
    let encoded = serde_json::to_string(&screen).unwrap();
    assert_eq!(serde_json::from_str::<Screen>(&encoded).unwrap(), screen);

    let state = state();
    let encoded = serde_json::to_string(&state).unwrap();
    assert_eq!(
        serde_json::from_str::<LibraryState>(&encoded).unwrap(),
        state
    );

    let secret = FormField::secret_raw("token", "Raw token", "value");
    assert!(secret.secret);
    assert!(!secret.translate_label);
    let multiline =
        FormField::multiline_with_arguments("body", "{} body", vec!["Prompt".to_owned()], "text");
    assert!(multiline.multiline);
    assert_eq!(multiline.label_arguments, ["Prompt"]);
}

#[test]
fn typed_add_workflow_round_trips_and_preserves_ordered_host_effects() {
    let add = AddWorkflowState::new(vec![DraftSummary {
        path: "/tmp/draft.py".into(),
        modified: 42,
    }]);
    let screen = Screen::Add(Box::new(add));
    let encoded = serde_json::to_string(&screen).unwrap();
    let decoded = serde_json::from_str::<Screen>(&encoded).unwrap();
    assert_eq!(decoded, screen);

    let review_screen = Screen::Add(Box::new(AddWorkflowState::from_review(
        ReviewState::from_source(
            SourceSnapshot {
                path: "typed.py".into(),
                source_record: "typed.py".to_owned(),
                bytes: b"COUNT = 3\n".to_vec(),
                permissions: SourcePermissions::default(),
                is_regular: true,
                is_directory: false,
                is_draft: false,
            },
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        ),
    )));
    let encoded = serde_json::to_string(&review_screen).unwrap();
    assert_eq!(
        serde_json::from_str::<Screen>(&encoded).unwrap(),
        review_screen
    );

    let mut state = state();
    state.update(Action::Present(screen));
    assert_eq!(
        state.update(Action::Add(AddAction::SetSourcePath(
            "/tmp/tool.py".to_owned(),
        ))),
        Effect::None
    );
    let effect = state.update(Action::Add(AddAction::Continue));
    assert!(matches!(
        effect,
        Effect::Add(effects)
            if matches!(effects.as_slice(), [AddEffect::InspectSource { path, .. }] if path == &std::path::PathBuf::from("/tmp/tool.py"))
    ));
    assert!(state.add_workflow().is_some());
}

#[test]
fn inert_editing_and_submission_actions_are_total_on_every_non_form_screen() {
    let mut state = state();
    assert!(state.form().is_none());
    for action in [Action::Backspace, Action::FocusNext, Action::Submit] {
        assert_eq!(state.update(action), Effect::None);
    }

    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Add,
        title: "Empty".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: None,
        fields: Vec::new(),
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    state.update(Action::FocusNext);
    assert_eq!(state.form().unwrap().focused, 0);

    state.update(Action::Present(Screen::Report(ReportView {
        title: "Report".to_owned(),
        items: Vec::new(),
    })));
    assert!(state.form().is_none());
    assert_eq!(state.update(Action::Backspace), Effect::None);
    assert_eq!(state.update(Action::Submit), Effect::None);
}

#[test]
fn typed_management_screens_reduce_host_effects_and_restore_their_owner() {
    let mut state = state();
    let preferences = preferences_view();
    state.update(Action::Present(Screen::Preferences(Box::new(
        preferences.clone(),
    ))));
    state.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(Vec::new()),
    ))));

    assert_eq!(
        state.update(Action::Runners(RunnerManagerAction::New)),
        Effect::None
    );
    state.update(Action::Runners(RunnerManagerAction::Editor(
        RunnerEditorAction::SetName("local".to_owned()),
    )));
    state.update(Action::Runners(RunnerManagerAction::Editor(
        RunnerEditorAction::SetCommand("agent {{prompt}}".to_owned()),
    )));
    assert!(matches!(
        state.update(Action::Runners(RunnerManagerAction::Editor(
            RunnerEditorAction::Submit,
        ))),
        Effect::SaveRunner {
            owner: RunnerSaveOwner::Manager,
            request,
        } if request.name == "local" && request.argv == ["agent", "{{prompt}}"]
    ));

    assert_eq!(
        state.update(Action::Runners(RunnerManagerAction::Back)),
        Effect::None
    );
    assert_eq!(
        state.update(Action::Runners(RunnerManagerAction::Back)),
        Effect::RefreshPreferencesAfterRunners
    );
    assert!(matches!(state.screen(), Screen::Runners(_)));

    let refreshed = preferences_view_with_runners(vec!["local".to_owned()]);
    assert_eq!(
        state.update(Action::RunnerManagerClosed {
            preferences: Box::new(refreshed.clone()),
        }),
        Effect::None
    );
    assert_eq!(state.screen(), &Screen::Preferences(Box::new(refreshed)));
    assert_ne!(state.screen(), &Screen::Preferences(Box::new(preferences)));
}

#[test]
fn health_rebuild_and_jump_are_typed_and_keep_library_selection_rules() {
    let mut state = state();
    state.update(Action::Present(Screen::Health(Box::new(HealthView::new(
        HealthSnapshot {
            uv: UvHealth::NotRequired,
            entry_count: 3,
            issues: vec![HealthIssue {
                slug: "beta".to_owned(),
                name: "Beta".to_owned(),
                kind: HealthIssueKind::MissingTarget,
            }],
            invalid_runner_rows: Vec::new(),
            mirror: MirrorHealth::Off,
            library_path: "/data/scripts".to_owned(),
            library_size: "3 KiB".to_owned(),
            diagnostics: Vec::new(),
        },
    )))));

    assert_eq!(
        state.update(Action::Health(HealthAction::Rebuild)),
        Effect::HealthRebuild
    );
    assert_eq!(
        state.update(Action::Health(HealthAction::Jump)),
        Effect::None
    );
    assert_eq!(state.screen(), &Screen::Library);
    assert_eq!(state.selected().unwrap().slug.as_str(), "beta");
}

#[test]
fn shared_runner_editor_returns_to_run_and_add_owners_without_losing_typed_state() {
    let mut state = state();
    let run = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    );
    state.update(Action::Present(Screen::Run(Box::new(run))));
    assert_eq!(state.update(Action::OpenRunRunnerEditor), Effect::None);
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunnerEditor {
            owner: RunnerEditorOwner::Run { selector },
            ..
        }) if selector == "demo"
    ));
    state.update(Action::RunnerEditor(RunnerEditorAction::SetName(
        "local".to_owned(),
    )));
    state.update(Action::RunnerEditor(RunnerEditorAction::SetCommand(
        "agent {{prompt}}".to_owned(),
    )));
    assert!(matches!(
        state.update(Action::RunnerEditor(RunnerEditorAction::Submit)),
        Effect::SaveRunner {
            owner: RunnerSaveOwner::Editor(RunnerEditorOwner::Run { ref selector }),
            ref request,
        } if selector == "demo" && request.name == "local"
    ));
    state.update(Action::RunnerEditorSaved {
        owner: RunnerEditorOwner::Run {
            selector: "demo".to_owned(),
        },
        name: "local".to_owned(),
        message: "Agent saved".to_owned(),
    });
    assert_eq!(state.modal(), None);
    assert!(matches!(
        &state.run_form().unwrap().fields()[0].control,
        FormControl::Choice(choice)
            if choice.selected == "local" && choice.options == ["codex", "local"]
    ));

    let review = ReviewState::from_source(
        SourceSnapshot {
            path: "task.prompt.md".into(),
            source_record: "task.prompt.md".to_owned(),
            bytes: b"Summarize {{topic}}".to_vec(),
            permissions: SourcePermissions::default(),
            is_regular: true,
            is_directory: false,
            is_draft: false,
        },
        KnownEntryKind::Prompt,
        ReviewDefaults {
            runner_names: vec!["codex".to_owned()],
            ..ReviewDefaults::default()
        },
    );
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::from_review(review),
    ))));
    assert_eq!(state.update(Action::OpenAddRunnerEditor), Effect::None);
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunnerEditor {
            owner: RunnerEditorOwner::Add,
            ..
        })
    ));
    state.update(Action::RunnerEditorSaved {
        owner: RunnerEditorOwner::Add,
        name: "local".to_owned(),
        message: "Agent saved".to_owned(),
    });
    let review = state.add_workflow().unwrap().review().unwrap();
    assert_eq!(review.runner(), "local");
    assert_eq!(review.runner_names(), ["codex", "local"]);
}
