//! Pointer rules for the Library list viewport.
//!
//! The Library list is a scroll owner, not a selector that a wheel drives. A notch over the rows
//! moves the viewport and leaves the selected entry where the reader put it, which is what every
//! graphical list does. The row-to-entry mapping must survive that offset, so the pointer endpoint
//! of a scrolled row is checked against the entry the row shows.
//!
//! Every state is built from `LibraryState` values, so no host is involved.

use ratatui_core::layout::Size;
use ratatui_crossterm::crossterm::event::{Event, KeyModifiers, MouseEvent, MouseEventKind};
use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::EventHandling;
use skit_tui_walker_model::parity::{
    apply_probe_handling, check_public_hit_parity, render_probe_endpoint, render_probe_session,
    session_primary_click,
};
use skit_ui::{Action, LibraryState};

/// The rule that `session_primary_click` keeps for a published target.
const PRESS_ARMS: &str = "a primary press over a published target must arm without activating";

/// Rows per wheel notch. `ratatui-interact` owns this constant for every scroll surface.
const WHEEL_ROWS: usize = 3;

/// The profile whose Library list is smaller than the entry count.
const SCROLLING_PROFILE: Size = Size {
    width: 46,
    height: 12,
};

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// A library that overflows the `SCROLLING_PROFILE` list viewport.
fn scrolling_library() -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries: (0..12)
            .map(|index| EntrySummary {
                slug: Slug::parse(format!("entry-{index}")).unwrap(),
                name: format!("Entry {index}"),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                description: String::new(),
                target: None,
            })
            .collect(),
        diagnostics: Vec::new(),
    })
}

#[test]
fn the_library_rows_viewport_owns_the_wheel_and_keeps_the_selection() {
    let state = scrolling_library();
    let (mut session, geometry) =
        render_probe_session(&state, Locale::En, SCROLLING_PROFILE).unwrap();
    assert_eq!(
        geometry.first_visible, 0,
        "an unscrolled Library starts at its first entry"
    );
    assert!(
        usize::from(geometry.rows.height) < state.visible_entry_count(),
        "the fixture must overflow the Library list viewport"
    );
    let selected = state.selected_visible_index();
    assert_eq!(selected, Some(0));

    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
        "the Library rows viewport must own the wheel"
    );
    let scrolled = render_probe_endpoint(&mut session, &state, Locale::En, SCROLLING_PROFILE)
        .unwrap()
        .geometry;
    assert_eq!(
        state.selected_visible_index(),
        selected,
        "a wheel scroll must not move the selection"
    );
    assert_eq!(
        scrolled.first_visible, WHEEL_ROWS,
        "one notch must scroll exactly three rows"
    );
    check_public_hit_parity(&state, &scrolled, SCROLLING_PROFILE, &session, Locale::En).unwrap();
}

#[test]
fn a_scrolled_library_row_reaches_the_entry_that_row_shows() {
    let mut state = scrolling_library();
    let (mut session, geometry) =
        render_probe_session(&state, Locale::En, SCROLLING_PROFILE).unwrap();
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let scrolled = render_probe_endpoint(&mut session, &state, Locale::En, SCROLLING_PROFILE)
        .unwrap()
        .geometry;

    let row = scrolled.rows.y.saturating_add(scrolled.rows.height - 1);
    let expected = scrolled.first_visible + usize::from(scrolled.rows.height) - 1;
    let handling = session_primary_click(&mut session, &state, &scrolled, scrolled.rows.x, row)
        .expect(PRESS_ARMS);
    assert_eq!(
        handling,
        EventHandling::Action(Action::SelectVisible(expected)),
        "the last visible row must reach the entry the offset put there"
    );
    apply_probe_handling(&mut state, handling, "scrolled Library row").unwrap();
    assert_eq!(
        state.selected_visible_index(),
        Some(expected),
        "a click on a scrolled row must select the entry it shows"
    );
}
