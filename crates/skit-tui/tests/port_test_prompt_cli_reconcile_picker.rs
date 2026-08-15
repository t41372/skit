use std::path::PathBuf;

use ratatui_core::layout::Rect;
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use skit_application::SourcePermissions;
use skit_tui::screens::picker::{ChoicePickerGeometry, PromptCandidatePickerEvent, PromptCandidatePickerSession};
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn review(body: &str) -> ReviewState {
    ReviewState::from_source(
        SourceSnapshot {
            path: PathBuf::from("/work/greet.prompt.md"),
            source_record: "/work/greet.prompt.md".to_owned(),
            bytes: body.as_bytes().to_vec(),
            permissions: SourcePermissions::default(),
            is_regular: true,
            is_directory: false,
            is_draft: false,
        },
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    )
}

fn geometry() -> ChoicePickerGeometry {
    ChoicePickerGeometry {
        search: Rect::new(0, 0, 40, 1),
        rows: Rect::new(0, 2, 40, 20),
        hits: Vec::new(),
    }
}

#[test]
fn test_edit_prompt_tui_reconcile_manages_the_pickers_selection() {
    let mut review = review("{{a}} {{b}} {{c}}\n");
    assert_eq!(review.selected_prompt_names(), ["a", "b", "c"], "non-flood review must preselect every new name");
    let mut picker = PromptCandidatePickerSession::new(review.prompt_picker());
    let geometry = geometry();

    // Cursor starts on a; move to b and turn only b off, then accept the actual picker.
    assert_eq!(picker.handle_event(key(KeyCode::Down, KeyModifiers::NONE), &geometry), Some(PromptCandidatePickerEvent::Changed));
    assert_eq!(picker.handle_event(key(KeyCode::Char(' '), KeyModifiers::NONE), &geometry), Some(PromptCandidatePickerEvent::Changed));
    let accepted = picker.handle_event(key(KeyCode::Char('s'), KeyModifiers::CONTROL), &geometry);
    let Some(PromptCandidatePickerEvent::Accepted(selected)) = accepted else { panic!("picker did not accept") };
    assert_eq!(selected, ["a", "c"]);
    review.set_prompt_selection(&selected);
    let create = review.create_entry().expect("accepted picker selection");
    assert_eq!(create.settings.params, ["a", "c"]);
}

#[test]
fn test_edit_prompt_tui_reconcile_none_manages_nothing() {
    let mut review = review("{{username}}\n");
    let mut picker = PromptCandidatePickerSession::new(review.prompt_picker());
    let geometry = geometry();
    assert_eq!(
        picker.handle_event(key(KeyCode::Esc, KeyModifiers::NONE), &geometry),
        Some(PromptCandidatePickerEvent::Cancelled)
    );
    // The frozen edit contract defines cancelling this optional management picker as managing none.
    review.set_prompt_selection(&[]);
    let create = review.create_entry().expect("cancelled optional picker");
    assert!(create.settings.params.is_empty());
    assert!(create.settings.parameters.is_empty());
}

#[test]
fn test_edit_prompt_tui_reconcile_flood_preselects_nothing() {
    let body = (0..31)
        .map(|index| format!("{{{{h{index}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut review = review(&body);
    assert!(review.prompt_is_flooded(), "fixture must cross the frozen 30-name auto-management cap");
    assert!(review.selected_prompt_names().is_empty(), "flooded picker opened with a nonempty preselection");

    let picker = PromptCandidatePickerSession::new(review.prompt_picker());
    assert_eq!(picker.visible_names().len(), 31, "searchable picker must contain all names despite empty preselection");
    let create = review.create_entry().expect("flooded no-selection review");
    assert!(create.settings.params.is_empty());
}
