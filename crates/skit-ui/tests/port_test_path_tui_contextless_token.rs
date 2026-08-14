use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterType};
use skit_ui::{Action, LibraryState, ModalState, RunFormView, RunTokenOption, Screen};

#[test]
fn test_token_menu_without_context_has_no_file_row() {
    let mut value = ParamDecl::new("value");
    value.parameter_type = ParameterType::Path;
    let form = RunFormView::from_declarations(
        "job",
        "job",
        &[value],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state.update(Action::OpenRunTokenMenuFor(0));

    let Some(ModalState::RunTokenMenu { field, options }) = state.modal() else {
        panic!("the contextless token menu must still open; only its filesystem row is absent");
    };
    assert_eq!(*field, 0);
    assert!(
        !options.contains(&RunTokenOption::FileOrFolder),
        "without a path context the token menu must not advertise a filesystem action"
    );
    assert!(
        options.contains(&RunTokenOption::Today) && options.contains(&RunTokenOption::Now),
        "lack of a path context removes the file row, not the rest of the token menu"
    );
}
