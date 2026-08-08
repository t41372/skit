use skit_application::{Diagnostic, DiagnosticCode, LibraryScan};
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_ui::{
    Action, Effect, FormField, FormPurpose, FormView, HostRequest, InputMode, LibraryState,
    ReportItem, ReportView, Screen,
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

    state.update(Action::Replace(LibraryScan {
        entries: vec![
            entry("beta", "Beta renamed", "still here"),
            entry("delta", "Delta", "new"),
        ],
        diagnostics: Vec::new(),
    }));

    assert_eq!(state.selected().unwrap().slug.as_str(), "beta");
}

#[test]
fn replacing_without_the_old_selection_falls_back_to_the_first_visible_entry() {
    let mut state = state();
    state.update(Action::End);
    state.update(Action::Replace(LibraryScan {
        entries: vec![entry("delta", "Delta", "new")],
        diagnostics: Vec::new(),
    }));
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
fn diagnostics_remain_available_to_every_frontend() {
    let diagnostic = Diagnostic {
        code: DiagnosticCode::CorruptMetadata,
        slug: Some("bad".to_owned()),
        message: "bad TOML".to_owned(),
    };
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
    assert!(matches!(state.screen(), Screen::ConfirmRemove { .. }));
    assert_eq!(
        state.update(Action::Submit),
        Effect::Remove {
            selector: "alpha".to_owned(),
        }
    );
    state.update(Action::Back);
    assert_eq!(state.screen(), &Screen::Library);
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
