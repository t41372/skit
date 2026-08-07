use skit_application::{Diagnostic, DiagnosticCode, LibraryScan};
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_ui::{Action, Effect, InputMode, LibraryState};

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
