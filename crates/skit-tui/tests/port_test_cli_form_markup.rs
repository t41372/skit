use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, terminal::Terminal};
use skit_i18n::Locale;
use skit_tui::{TuiSession, render_with_session};
use skit_ui::{Action, LibraryState, RunFormView, Screen};
use skit_domain::parameters::ParamDecl;

fn render(form: RunFormView) -> String {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("terminal");
    terminal
        .draw(|frame| {
            let _ = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .expect("draw run form");
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

#[test]
fn test_collect_command_values_prompt_escapes_markup_in_placeholder_name() {
    let hostile = ParamDecl::new("[red]msg[/red]");
    let form = RunFormView::from_declarations(
        "e",
        "e",
        &[hostile],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let shown = render(form);
    assert!(
        shown.contains("[red]msg[/red]"),
        "the user-controlled placeholder label must render literally: {shown}"
    );
}

#[test]
fn test_collect_param_form_prompt_escapes_markup_in_param_prompt_text() {
    let mut city = ParamDecl::new("CITY");
    city.prompt = "[red]Where[/red]?".to_owned();
    let form = RunFormView::from_declarations(
        "a",
        "a",
        &[city],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let shown = render(form);
    assert!(
        shown.contains("[red]Where[/red]?"),
        "the user-controlled prompt text must render literally: {shown}"
    );
}
