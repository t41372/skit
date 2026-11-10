//! The footer shows each status with the glyph and the colors of the outcome it reports.
//!
//! Ways these tests can fail: a kind draws no glyph, or the wrong one; the `skit` theme paints
//! the line in a color that does not match the outcome; the terminal theme gives an error a hue on
//! its letters (the census forbids that) or leaves the glyph without its color; information grows
//! a glyph; the entry count does not come back when the status clears.

use ratatui_core::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    terminal::Terminal,
};
use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::{Appearance, ThemeName, TuiSession, render_with_session};
use skit_ui::{Action, LibraryState, StatusKind};

fn library() -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries: ["alpha", "beta"]
            .into_iter()
            .map(|slug| EntrySummary {
                slug: Slug::parse(slug).unwrap(),
                name: slug.to_owned(),
                kind: EntryKind::parse("command").unwrap(),
                mode: StorageMode::Copy,
                description: String::new(),
                target: None,
            })
            .collect(),
        diagnostics: Vec::new(),
    })
}

fn draw(state: &LibraryState, theme: ThemeName) -> Buffer {
    draw_sized(state, theme, 100)
}

fn draw_sized(state: &LibraryState, theme: ThemeName, width: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
    let mut session = TuiSession::default();
    session.set_appearance(Appearance::default().with_theme(theme));
    terminal
        .draw(|frame| {
            let _ = render_with_session(frame, state, Locale::En, &mut session);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

/// The text of one row and the cells of the first match of `needle` in it.
fn find(buffer: &Buffer, needle: &str) -> Option<(String, Vec<ratatui_core::buffer::Cell>)> {
    (0..buffer.area.height).find_map(|row| {
        let symbols = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol().to_owned())
            .collect::<Vec<_>>();
        let line = symbols.concat();
        let byte = line.find(needle)?;
        let start = line[..byte].chars().count();
        let mut column = 0_usize;
        let mut chars = 0_usize;
        while chars < start {
            chars += symbols[column].chars().count();
            column += 1;
        }
        let cells = (column..column + needle.chars().count())
            .map(|column| buffer[(u16::try_from(column).unwrap(), row)].clone())
            .collect();
        Some((line, cells))
    })
}

fn with_status(kind: StatusKind, message: &str) -> LibraryState {
    let mut state = library();
    state.update(Action::SetStatus {
        kind,
        message: message.to_owned(),
    });
    state
}

#[test]
fn each_outcome_draws_its_glyph_in_its_color_in_the_skit_theme() {
    for (kind, glyph, color) in [
        (StatusKind::Success, "✓", Color::Green),
        (StatusKind::Warning, "⚠", Color::Yellow),
        (StatusKind::Error, "✗", Color::Red),
    ] {
        let buffer = draw(&with_status(kind, "Settings saved"), ThemeName::Skit);
        let needle = format!("{glyph} Settings saved");
        let (line, cells) = find(&buffer, &needle).unwrap_or_else(|| panic!("{needle} missing"));
        assert!(line.starts_with(&format!("│{needle}")), "{line}");
        assert!(
            cells.iter().all(|cell| cell.fg == color),
            "{kind:?}: {cells:?}"
        );
        // The status replaces the entry count while it shows.
        assert!(find(&buffer, "2/2 entries").is_none());
    }
}

#[test]
fn the_terminal_theme_colors_the_glyph_and_keeps_an_error_text_plain_and_bold() {
    for (kind, glyph, glyph_color, text_color, text_bold) in [
        (StatusKind::Success, "✓", Color::Green, Color::Green, false),
        (
            StatusKind::Warning,
            "⚠",
            Color::Yellow,
            Color::Yellow,
            false,
        ),
        (StatusKind::Error, "✗", Color::Red, Color::Reset, true),
    ] {
        let buffer = draw(&with_status(kind, "Done"), ThemeName::Terminal);
        let (_, cells) = find(&buffer, &format!("{glyph} Done")).unwrap();
        assert_eq!(cells[0].fg, glyph_color, "{kind:?} glyph");
        for cell in &cells[2..] {
            assert_eq!(cell.fg, text_color, "{kind:?} text");
            assert_eq!(
                cell.modifier.contains(Modifier::BOLD),
                text_bold,
                "{kind:?} text"
            );
        }
    }
}

#[test]
fn information_has_no_glyph_and_the_default_colors() {
    for theme in [ThemeName::Skit, ThemeName::Terminal] {
        let buffer = draw(&with_status(StatusKind::Info, "Note"), theme);
        let (line, cells) = find(&buffer, "Note").unwrap();
        assert!(line.starts_with("│Note"), "{line}");
        assert!(cells.iter().all(|cell| cell.fg == Color::Reset));
    }
}

#[test]
fn the_entry_count_comes_back_when_the_status_clears() {
    let mut state = with_status(StatusKind::Success, "Saved");
    state.update(Action::ClearStatus);
    let buffer = draw(&state, ThemeName::Terminal);
    let (line, _) = find(&buffer, "2/2 entries").unwrap();
    assert!(line.starts_with("│2/2 entries"), "{line}");
}

#[test]
fn a_status_wider_than_the_footer_wraps_onto_a_second_row() {
    let message = "skit-test-missing-shell is not on PATH. Install it, or set the interpreter in Entry settings.";
    let state = with_status(StatusKind::Error, message);
    let buffer = draw_sized(&state, ThemeName::Terminal, 60);
    let (first, _) = find(&buffer, "✗ skit-test-missing-shell").expect("the first row");
    let (second, _) = find(&buffer, "Entry settings.").expect("the end of the status");
    assert!(first.starts_with("│✗ skit-test-missing-shell"), "{first}");
    assert_ne!(first, second, "the status did not wrap");
    // The entry count gives way to the status, and the row under the chips stays one row when
    // the status fits.
    assert!(find(&buffer, "2/2 entries").is_none());
    let short = draw_sized(
        &with_status(StatusKind::Error, "Failed"),
        ThemeName::Terminal,
        60,
    );
    let (row, _) = find(&short, "✗ Failed").unwrap();
    assert!(row.starts_with("│✗ Failed"), "{row}");
}
