use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_ui::{Action, Effect, InputMode, LibraryState};

fn entry(slug: &str, name: &str, description: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Copy,
        description: description.to_owned(),
        target: None,
    }
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
fn search_is_an_explicit_mode_and_filters_across_visible_fields() {
    let mut state = state();
    assert_eq!(state.input_mode(), InputMode::Browse);

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
fn effects_are_frontend_neutral() {
    let mut state = state();
    assert_eq!(state.update(Action::Reload), Effect::Reload);
    assert_eq!(state.update(Action::Quit), Effect::Quit);
}

#[test]
fn direct_row_selection_uses_a_visible_index() {
    let mut state = state();
    state.update(Action::SelectVisible(2));
    assert_eq!(state.selected().unwrap().slug.as_str(), "gamma");
    state.update(Action::SelectVisible(999));
    assert_eq!(state.selected().unwrap().slug.as_str(), "gamma");
}
