//! TUI-owned half of the mechanical port of `tests/test_path_type.py`
//! (`main@206f9ef`). The type-label assertion belongs to the Ratatui adapter, not the shared
//! form model.

use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use skit_domain::parameters::{ParamDecl, ParameterType};
use skit_i18n::Locale;
use skit_tui::{TuiSession, render_with_session};
use skit_ui::{Action, LibraryState, RunFormView, Screen};

fn row_text(buffer: &Buffer, row: u16) -> String {
    (0..buffer.area.width)
        .map(|column| buffer[(column, row)].symbol())
        .collect()
}

#[test]
fn test_type_label_path() {
    let declaration = ParamDecl {
        parameter_type: ParameterType::Path,
        prompt: "Source file".to_owned(),
        ..ParamDecl::new("src")
    };
    let form = RunFormView::from_declarations(
        "path-label",
        "Type label",
        &[declaration],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let row = (0..buffer.area.height)
        .find(|row| row_text(buffer, *row).contains("Source file"))
        .expect("the path field label must be visible");
    assert!(
        row_text(buffer, row).contains("path"),
        "the path field must render the exact Python type label"
    );
}
