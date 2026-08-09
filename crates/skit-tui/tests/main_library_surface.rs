use std::collections::BTreeMap;

use ratatui_core::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    terminal::Terminal,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_localized, render_with_session};
use skit_ui::{
    Action, LibraryEntryDetail, LibraryLastRun, LibraryParameterDetail, LibraryPromptRunner,
    LibraryRunAge, LibraryState,
};
use unicode_width::UnicodeWidthStr as _;

const ACCENT: Color = Color::Rgb(0xD9, 0x77, 0x57);
const SELECT_BG: Color = Color::Rgb(0x5A, 0x2D, 0x1E);
const SELECT_FG: Color = Color::Rgb(0xEE, 0xEE, 0xEE);
const BOX_GREEN: Color = Color::Rgb(0x3D, 0x7B, 0x46);
const BOX_INDIGO: Color = Color::Rgb(0x4B, 0x44, 0xB0);

fn entry(
    slug: &str,
    name: &str,
    kind: &str,
    mode: StorageMode,
    description: &str,
    target: Option<&str>,
) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode,
        description: description.to_owned(),
        target: target.map(str::to_owned),
    }
}

fn state(entries: Vec<EntrySummary>) -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries,
        diagnostics: Vec::new(),
    })
}

fn draw(view: &LibraryState, width: u16, height: u16, locale: Locale) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_localized(frame, view, locale);
        })
        .unwrap();
    terminal
}

fn draw_with_session(
    view: &LibraryState,
    session: &mut TuiSession,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, view, Locale::En, session);
        })
        .unwrap();
    (terminal, geometry)
}

fn lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn line_with<'a>(lines: &'a [String], needle: &str) -> (usize, &'a str) {
    lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains(needle))
        .map(|(row, line)| (row, line.as_str()))
        .unwrap_or_else(|| panic!("missing {needle:?} in {lines:#?}"))
}

#[test]
fn library_uses_main_kind_badges_localized_columns_and_reference_marker() {
    let view = state(vec![
        entry(
            "selected",
            "Selected",
            "python",
            StorageMode::Copy,
            "",
            None,
        ),
        entry(
            "linked-shell",
            "Linked shell",
            "shell",
            StorageMode::Reference,
            "Shell entry",
            Some("/tmp/linked.sh"),
        ),
        entry(
            "command",
            "Command entry",
            "command",
            StorageMode::Reference,
            "Command entry",
            None,
        ),
        entry(
            "program",
            "Program entry",
            "exe",
            StorageMode::Copy,
            "Program entry",
            None,
        ),
        entry(
            "future",
            "Future entry",
            "future-kind",
            StorageMode::Reference,
            "Future entry",
            Some("/tmp/future"),
        ),
    ]);

    let terminal = draw(&view, 120, 30, Locale::En);
    let text = lines(terminal.backend().buffer()).join("\n");
    assert!(text.contains("Name"), "{text}");
    assert!(text.contains("Kind"), "{text}");
    assert!(text.contains("⬡ Python"), "{text}");
    assert!(text.contains("# Shell ↗"), "{text}");
    assert!(text.contains("$ Command"), "{text}");
    assert!(!text.contains("$ Command ↗"), "{text}");
    assert!(text.contains("▶ Program"), "{text}");
    assert!(text.contains("? future-kind"), "{text}");
    assert!(!text.contains("? future-kind ↗"), "{text}");

    let terminal = draw(&view, 120, 30, Locale::ZhTw);
    let text = lines(terminal.backend().buffer()).join("\n");
    assert!(text.contains("名 稱"), "{text}");
    assert!(text.contains("類 型"), "{text}");
    assert!(text.contains("▶ 程 式"), "{text}");
}

#[test]
fn library_uses_main_panel_palette_rounded_borders_and_selection_style() {
    let view = state(vec![entry(
        "selected",
        "Selected",
        "python",
        StorageMode::Copy,
        "Description",
        None,
    )]);
    let terminal = draw(&view, 120, 30, Locale::En);
    let cells = terminal.backend().buffer().content();

    assert!(
        cells
            .iter()
            .any(|cell| cell.symbol() == "╭" && cell.fg == BOX_GREEN),
        "the Library panel must have the main green rounded border"
    );
    assert!(
        cells
            .iter()
            .any(|cell| cell.symbol() == "╭" && cell.fg == BOX_INDIGO),
        "the Detail pane must have the main indigo rounded border"
    );
    assert!(
        cells.iter().any(|cell| {
            cell.symbol() == "S"
                && cell.fg == SELECT_FG
                && cell.bg == SELECT_BG
                && cell.modifier.contains(Modifier::BOLD)
        }),
        "the selected table row must use the dark terracotta selection bar"
    );
    assert!(
        cells
            .iter()
            .any(|cell| cell.symbol() == "S" && cell.fg == ACCENT),
        "the detail name must use the main accent"
    );
    assert!(
        cells.iter().any(|cell| {
            cell.symbol() == "N"
                && cell.fg == Color::White
                && cell.modifier.contains(Modifier::BOLD)
        }),
        "the table header must use bold bright white"
    );
}

#[test]
fn library_layout_uses_main_breakpoints_and_three_to_two_ratio() {
    let mut view = state(vec![entry(
        "selected",
        "Selected",
        "python",
        StorageMode::Copy,
        "Description",
        None,
    )]);

    let wide = draw(&view, 120, 30, Locale::En);
    let wide_lines = lines(wide.backend().buffer());
    let (detail_row, detail_line) = line_with(&wide_lines, "╭ Detail pane");
    let detail_column = detail_line[..detail_line.find("Detail pane").unwrap()].width();
    assert!(detail_row < 8, "wide detail must be beside the list");
    assert!(
        (70..=76).contains(&detail_column),
        "the detail must start after the 3/5 Library pane: {detail_column}"
    );

    let portrait = draw(&view, 60, 24, Locale::En);
    let portrait_lines = lines(portrait.backend().buffer());
    let (detail_row, _) = line_with(&portrait_lines, "╭ Detail pane");
    assert!(
        detail_row > 7,
        "a narrow normal-height view must stack detail"
    );

    let short = draw(&view, 60, 15, Locale::En);
    let short_text = lines(short.backend().buffer()).join("\n");
    assert!(
        !short_text.contains("╭ Detail pane"),
        "a narrow short view must auto-hide detail"
    );

    view.update(Action::ToggleDetail);
    let pinned_closed = draw(&view, 120, 30, Locale::En);
    assert!(
        !lines(pinned_closed.backend().buffer())
            .join("\n")
            .contains("╭ Detail pane"),
        "pinning detail closed must override a wide layout"
    );

    view.update(Action::ToggleDetail);
    let pinned_open = draw(&view, 60, 15, Locale::En);
    assert!(
        lines(pinned_open.backend().buffer())
            .join("\n")
            .contains("╭ Detail pane"),
        "pinning detail open must override the short-height auto-hide"
    );
}

#[test]
fn detail_summary_matches_main_copy_reference_and_description_contract() {
    let copy = state(vec![entry(
        "copy",
        "Copy entry",
        "python",
        StorageMode::Copy,
        "",
        None,
    )]);
    let copy_terminal = draw(&copy, 200, 30, Locale::En);
    let copy_text = lines(copy_terminal.backend().buffer()).join("\n");
    assert!(
        copy_text.contains("✓ The copy is kept by skit; your original file is never modified."),
        "{copy_text}"
    );
    assert!(
        copy_text.contains("(no description — add one in Entry settings)"),
        "{copy_text}"
    );
    assert!(!copy_text.contains("Storage mode:"), "{copy_text}");
    assert!(!copy_text.contains("Slug:"), "{copy_text}");

    let reference = state(vec![entry(
        "reference",
        "Reference entry",
        "shell",
        StorageMode::Reference,
        "A linked script",
        Some("/work/original.sh"),
    )]);
    let reference_terminal = draw(&reference, 200, 30, Locale::En);
    let reference_text = lines(reference_terminal.backend().buffer()).join("\n");
    assert!(
        reference_text.contains("↗ Linked to the original: /work/original.sh"),
        "{reference_text}"
    );
    assert!(
        reference_text.contains("A linked script"),
        "{reference_text}"
    );

    let command = state(vec![entry(
        "command",
        "Command entry",
        "command",
        StorageMode::Reference,
        "A template",
        None,
    )]);
    let command_terminal = draw(&command, 200, 30, Locale::En);
    let command_text = lines(command_terminal.backend().buffer()).join("\n");
    assert!(
        !command_text.contains("Linked to the original"),
        "{command_text}"
    );
    assert!(!command_text.contains("The copy is kept"), "{command_text}");

    let translated = draw(&copy, 200, 30, Locale::ZhTw);
    let translated_text = lines(translated.backend().buffer()).join("\n");
    assert!(translated_text.contains("副 本 由"), "{translated_text}");
    assert!(translated_text.contains("skit"), "{translated_text}");
    assert!(translated_text.contains("原 始 檔"), "{translated_text}");
    assert!(translated_text.contains("沒 有 說 明"), "{translated_text}");
}

#[test]
fn library_activity_health_and_complete_detail_match_latest_main() {
    let recently_added = entry(
        "newer",
        "Recently added",
        "python",
        StorageMode::Copy,
        "",
        None,
    );
    let recently_run = entry(
        "active",
        "Recently run",
        "prompt",
        StorageMode::Reference,
        "A prompt",
        Some("/work/prompt.md"),
    );
    let details = BTreeMap::from([
        (
            recently_added.slug.clone(),
            LibraryEntryDetail {
                added_at: "2026-08-08T12:00:00+00:00".to_owned(),
                ..LibraryEntryDetail::default()
            },
        ),
        (
            recently_run.slug.clone(),
            LibraryEntryDetail {
                added_at: "2025-01-01T00:00:00+00:00".to_owned(),
                template: None,
                prompt_runner: Some(LibraryPromptRunner::Missing("old-agent".to_owned())),
                parameters: vec![
                    LibraryParameterDetail {
                        key: "topic".to_owned(),
                        value: "Rust".to_owned(),
                        secret: false,
                    },
                    LibraryParameterDetail {
                        key: "token".to_owned(),
                        value: "must-not-render".to_owned(),
                        secret: true,
                    },
                ],
                presets: vec!["weekly".to_owned(), "daily".to_owned()],
                dependencies: vec!["httpx>=0.28".to_owned()],
                last_run: Some(LibraryLastRun {
                    at: "2026-08-09T12:00:00+00:00".to_owned(),
                    age: LibraryRunAge::Minutes(12),
                    exit: Some(7),
                }),
                missing_target: Some("/work/prompt.md".to_owned()),
                drifted: true,
                original_file_preserved: true,
            },
        ),
    ]);
    let view = LibraryState::from_surface(
        LibraryScan {
            entries: vec![recently_added, recently_run],
            diagnostics: Vec::new(),
        },
        details,
    );

    assert_eq!(
        view.visible_entries()
            .map(|entry| entry.slug.as_str())
            .collect::<Vec<_>>(),
        ["active", "newer"],
        "last run or added time, whichever is newer, owns Library order"
    );
    let terminal = draw(&view, 220, 40, Locale::En);
    let rendered = lines(terminal.backend().buffer()).join("\n");
    assert!(rendered.contains("⚠"), "{rendered}");
    assert!(
        rendered.contains("🤖 old-agent (no longer configured)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Parameters  topic=Rust  token=•••🔒"),
        "{rendered}"
    );
    assert!(rendered.contains("Presets  daily · weekly"), "{rendered}");
    assert!(rendered.contains("Depends on  httpx>=0.28"), "{rendered}");
    assert!(
        rendered.contains("Last run  12 min ago · ✗ failed (code 7)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("⚠ missing: /work/prompt.md"),
        "{rendered}"
    );
    assert!(!rendered.contains("must-not-render"), "{rendered}");
    assert!(
        !rendered.contains("The script changed —"),
        "a missing target takes precedence over drift: {rendered}"
    );
}

#[test]
fn library_detail_keeps_template_runner_states_drift_and_empty_onboarding_visible() {
    let command = entry(
        "command",
        "Deploy",
        "command",
        StorageMode::Reference,
        "",
        None,
    );
    let details = BTreeMap::from([(
        command.slug.clone(),
        LibraryEntryDetail {
            template: Some("deploy --env {{environment}}".to_owned()),
            last_run: None,
            drifted: true,
            ..LibraryEntryDetail::default()
        },
    )]);
    let view = LibraryState::from_surface(
        LibraryScan {
            entries: vec![command],
            diagnostics: Vec::new(),
        },
        details,
    );
    let terminal = draw(&view, 240, 34, Locale::En);
    let rendered = lines(terminal.backend().buffer()).join("\n");
    assert!(
        rendered.contains("deploy --env {{environment}}"),
        "{rendered}"
    );
    assert!(rendered.contains("Not run yet"), "{rendered}");
    assert!(
        rendered
            .contains("⚠ The script changed — skit checks the form against it before every run."),
        "{rendered}"
    );

    let empty = state(Vec::new());
    let terminal = draw(&empty, 160, 24, Locale::En);
    let rendered = lines(terminal.backend().buffer()).join("\n");
    assert!(
        rendered.contains("Your entries will appear here."),
        "{rendered}"
    );
    assert!(
        rendered.contains("Press a to add the first one,"),
        "{rendered}"
    );
    assert!(
        rendered.contains("or run: skit add <path> in a terminal."),
        "{rendered}"
    );
}

#[test]
fn library_detail_uses_mature_keyboard_and_mouse_scrolling_after_pointer_focus() {
    let item = entry(
        "long",
        "Long detail",
        "python",
        StorageMode::Copy,
        &format!("TOP {} BOTTOM", "wrapped words ".repeat(80)),
        None,
    );
    let view = LibraryState::from_surface(
        LibraryScan {
            entries: vec![item.clone()],
            diagnostics: Vec::new(),
        },
        BTreeMap::from([(
            item.slug,
            LibraryEntryDetail {
                added_at: "2026-08-09T00:00:00+00:00".to_owned(),
                ..LibraryEntryDetail::default()
            },
        )]),
    );
    let mut session = TuiSession::default();
    let (initial, geometry) = draw_with_session(&view, &mut session, 100, 18);
    let initial_text = lines(initial.backend().buffer()).join("\n");
    assert!(initial_text.contains("TOP"), "{initial_text}");
    assert!(!initial_text.contains("Not run yet"), "{initial_text}");

    let detail_click = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 75,
        row: 7,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(
        session.handle_event(detail_click, &view, &geometry),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (at_bottom, geometry) = draw_with_session(&view, &mut session, 100, 18);
    let at_bottom_text = lines(at_bottom.backend().buffer()).join("\n");
    assert!(at_bottom_text.contains("Not run yet"), "{at_bottom_text}");

    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 75,
                row: 7,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
            &geometry,
        ),
        EventHandling::Consumed,
        "the detail wheel must route through ScrollableContentState"
    );
    let (after_wheel, _) = draw_with_session(&view, &mut session, 100, 18);
    assert_ne!(
        lines(after_wheel.backend().buffer()),
        lines(at_bottom.backend().buffer()),
        "mouse scrolling must change the visible wrapped detail viewport"
    );
}
