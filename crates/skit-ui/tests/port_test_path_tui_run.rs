use std::collections::BTreeMap;

use skit_application::{path_insertion::RunPathInsertMode, tokens::TokenContext};
use skit_domain::parameters::{ParamDecl, ParameterType};
use skit_ui::{
    Action, LibraryState, ModalState, RunFormContext, RunFormView, RunPathContext, RunTokenOption,
    Screen,
};

fn context() -> RunFormContext {
    RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: "/work/project".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/alice".to_owned()),
            env: BTreeMap::new(),
            today: "2026-08-14".to_owned(),
            now: "15-57-00".to_owned(),
        },
    }
}

fn path_form() -> RunFormView {
    let mut src = ParamDecl::new("src");
    src.parameter_type = ParameterType::Path;
    let mut files = ParamDecl::new("files");
    files.parameter_type = ParameterType::Path;
    files.multiple = true;
    RunFormView::from_declarations(
        "job",
        "job",
        &[src, files],
        &BTreeMap::from([
            ("src".to_owned(), "old-prefill.csv".to_owned()),
            ("files".to_owned(), "first.txt".to_owned()),
        ]),
        &[],
        "",
        &BTreeMap::new(),
        "--verbose",
    )
    .with_context(context())
}

fn state_with(form: RunFormView) -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

#[test]
fn test_token_menu_puts_file_row_first_on_path_fields_and_picker_replaces() {
    let mut state = state_with(path_form());
    assert_eq!(state.focused_form_field(), Some(0));

    assert_eq!(state.update(Action::OpenRunTokenMenu), skit_ui::Effect::None);
    let Some(ModalState::RunTokenMenu { field, options }) = state.modal() else {
        panic!("path field did not open the run token menu");
    };
    assert_eq!(*field, 0);
    assert_eq!(options.first(), Some(&RunTokenOption::FileOrFolder));

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
        path: "data.csv".to_owned(),
    });
    assert_eq!(state.modal(), None);
    assert_eq!(
        state.run_form().expect("run form").fields()[0]
            .control
            .value(),
        "data.csv"
    );
}

#[test]
fn test_picker_appends_quoted_to_the_extra_args_row() {
    let mut state = state_with(path_form());
    state.update(Action::FocusField(2));
    state.update(Action::OpenFocusedRunFilePicker);
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 2,
            mode: RunPathInsertMode::Arguments,
            ..
        })
    ));
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: 2,
        path: "a b.txt".to_owned(),
    });
    let value = state.run_form().expect("run form").fields()[2]
        .control
        .value();
    #[cfg(windows)]
    assert_eq!(value, "--verbose \"a b.txt\"");
    #[cfg(not(windows))]
    assert_eq!(value, "--verbose 'a b.txt'");
}

#[test]
fn test_picker_appends_quoted_to_a_multiple_field() {
    let mut state = state_with(path_form());
    state.update(Action::FocusField(1));
    state.update(Action::OpenFocusedRunFilePicker);
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
        path: "a b.txt".to_owned(),
    });
    assert_eq!(
        state.run_form().expect("run form").fields()[1]
            .control
            .value(),
        "first.txt 'a b.txt'"
    );
}

#[test]
fn test_browse_link_opens_the_picker_directly_and_replaces() {
    let mut state = state_with(path_form());
    state.update(Action::OpenFocusedRunFilePicker);
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
        path: "data.csv".to_owned(),
    });
    assert_eq!(
        state.run_form().expect("run form").fields()[0]
            .control
            .value(),
        "data.csv"
    );
    assert_eq!(state.focused_form_field(), Some(0));
}

#[test]
fn test_browse_without_a_key_uses_the_focused_field_and_its_dialect() {
    let mut state = state_with(path_form());
    state.update(Action::FocusField(2));
    assert_eq!(state.focused_form_field(), Some(2));
    state.update(Action::OpenFocusedRunFilePicker);
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 2,
            mode: RunPathInsertMode::Arguments,
            ..
        })
    ));
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: 2,
        path: "a b.txt".to_owned(),
    });
    let value = state.run_form().expect("run form").fields()[2]
        .control
        .value();
    #[cfg(windows)]
    assert_eq!(value, "--verbose \"a b.txt\"");
    #[cfg(not(windows))]
    assert_eq!(value, "--verbose 'a b.txt'");
}

#[test]
fn test_browse_refuses_numeric_secret_and_unknown_rows() {
    let mut count = ParamDecl::new("count");
    count.parameter_type = ParameterType::Int;
    let mut loud = ParamDecl::new("loud");
    loud.parameter_type = ParameterType::Bool;
    let mut token = ParamDecl::new("token");
    token.secret = true;
    let note = ParamDecl::new("note");
    let form = RunFormView::from_declarations(
        "mixed",
        "mixed",
        &[count, loud, token, note],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(context());
    assert!(!form.can_browse_field(0));
    assert!(!form.can_browse_field(1));
    assert!(!form.can_browse_field(2));
    assert!(form.can_browse_field(3));
    assert!(!form.can_browse_field(99));

    let mut state = state_with(form);
    for field in [0, 1, 2, 99] {
        state.update(Action::OpenRunFilePicker(field));
        assert_eq!(state.modal(), None, "field {field} must not open a picker");
    }
}

#[test]
fn test_fieldrow_browsable_needs_a_context() {
    let field = ParamDecl::new("x");
    let without = RunFormView::from_declarations(
        "job",
        "job",
        &[field.clone()],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    assert!(!without.can_browse_field(0));
    assert!(without.with_context(context()).can_browse_field(0));
}

#[test]
fn test_fieldrow_shlexy_and_insert_mode_all_branches() {
    let mut state = state_with(path_form());

    state.update(Action::OpenRunFilePicker(0));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 0,
            mode: RunPathInsertMode::Replace,
            ..
        })
    ));
    state.update(Action::Back);

    state.update(Action::OpenRunFilePicker(1));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 1,
            mode: RunPathInsertMode::Shlex,
            ..
        })
    ));
    state.update(Action::Back);

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
