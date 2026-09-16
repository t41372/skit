use std::collections::BTreeMap;

use ratatui_core::{
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
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
        entry("fish", "Fish entry", "fish", StorageMode::Copy, "", None),
        entry("js", "JS entry", "js", StorageMode::Copy, "", None),
        entry("ts", "TS entry", "ts", StorageMode::Copy, "", None),
        entry(
            "powershell",
            "PowerShell entry",
            "powershell",
            StorageMode::Copy,
            "",
            None,
        ),
        entry("ruby", "Ruby entry", "ruby", StorageMode::Copy, "", None),
        entry("perl", "Perl entry", "perl", StorageMode::Copy, "", None),
        entry("lua", "Lua entry", "lua", StorageMode::Copy, "", None),
        entry("r", "R entry", "r", StorageMode::Copy, "", None),
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
            "prompt",
            "Prompt entry",
            "prompt",
            StorageMode::Copy,
            "Prompt entry",
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
    assert!(text.contains("∿ fish"), "{text}");
    assert!(text.contains("✦ JavaScript"), "{text}");
    assert!(text.contains("✧ TypeScript"), "{text}");
    assert!(text.contains("» PowerShell"), "{text}");
    assert!(text.contains("◆ Ruby"), "{text}");
    assert!(text.contains("◈ Perl"), "{text}");
    assert!(text.contains("○ Lua"), "{text}");
    assert!(text.contains("◇ R"), "{text}");
    assert!(text.contains("$ Command"), "{text}");
    assert!(!text.contains("$ Command ↗"), "{text}");
    assert!(text.contains("▶ Program"), "{text}");
    assert!(text.contains("✎ Prompt"), "{text}");
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

    view.update(Action::ToggleDetail {
        currently_visible: true,
    });
    let pinned_closed = draw(&view, 120, 30, Locale::En);
    assert!(
        !lines(pinned_closed.backend().buffer())
            .join("\n")
            .contains("╭ Detail pane"),
        "pinning detail closed must override a wide layout"
    );

    view.update(Action::ToggleDetail {
        currently_visible: false,
    });
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
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 75,
                row: 7,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
            &geometry,
        ),
        EventHandling::Consumed,
        "the detail pane must take focus only after a matching release"
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (at_bottom, _geometry) = draw_with_session(&view, &mut session, 100, 18);
    let at_bottom_text = lines(at_bottom.backend().buffer()).join("\n");
    assert!(at_bottom_text.contains("Not run yet"), "{at_bottom_text}");

    let (_, hidden) = draw_with_session(&view, &mut session, 1, 12);
    assert!(!hidden.detail_pane_visible);
    let (restored, geometry) = draw_with_session(&view, &mut session, 100, 18);
    let restored_text = lines(restored.backend().buffer()).join("\n");
    assert!(
        restored_text.contains("Not run yet"),
        "a zero-width detail pane reset its preserved scroll position: {restored_text}"
    );

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

    let (grown, _) = draw_with_session(&view, &mut session, 100, 50);
    assert!(
        lines(grown.backend().buffer()).join("\n").contains("TOP"),
        "a grown detail viewport must clamp its old scroll offset"
    );
    for event in [
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 75,
            row: 7,
            modifiers: KeyModifiers::NONE,
        }),
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 75,
            row: 7,
            modifiers: KeyModifiers::NONE,
        }),
    ] {
        assert_eq!(
            session.handle_event(event, &view, &geometry),
            EventHandling::Ignored
        );
    }
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 7,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert!(matches!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 2,
                row: 7,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
            &geometry,
        ),
        EventHandling::Action(Action::SelectVisible(_))
    ));
    let selected = view.selected_visible_index();
    for kind in [MouseEventKind::ScrollDown, MouseEventKind::ScrollUp] {
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind,
                    column: 2,
                    row: 7,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
                &geometry,
            ),
            EventHandling::Consumed,
            "{kind:?} over the Library rows must scroll the list viewport"
        );
        assert_eq!(
            view.selected_visible_index(),
            selected,
            "{kind:?} over the Library rows must keep the selection"
        );
    }
}

#[test]
fn selecting_another_entry_resets_the_detail_viewport_to_its_top() {
    let alpha = entry(
        "alpha",
        "Alpha",
        "python",
        StorageMode::Copy,
        &format!("ALPHA_TOP {}", "alpha words ".repeat(80)),
        None,
    );
    let beta = entry(
        "beta",
        "Beta",
        "python",
        StorageMode::Copy,
        &format!("BETA_TOP {}", "beta words ".repeat(80)),
        None,
    );
    let mut view = LibraryState::from_surface(
        LibraryScan {
            entries: vec![alpha.clone(), beta.clone()],
            diagnostics: Vec::new(),
        },
        BTreeMap::from([
            (alpha.slug, LibraryEntryDetail::default()),
            (beta.slug, LibraryEntryDetail::default()),
        ]),
    );
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 100, 18);
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind,
                    column: 75,
                    row: 7,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
                &geometry,
            ),
            EventHandling::Consumed
        );
    }
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (bottom, _) = draw_with_session(&view, &mut session, 100, 18);
    assert!(
        lines(bottom.backend().buffer())
            .join("\n")
            .contains("Not run yet")
    );

    view.update(Action::Next);
    assert_eq!(
        view.selected().map(|entry| entry.name.as_str()),
        Some("Beta")
    );
    let (selected, _) = draw_with_session(&view, &mut session, 100, 18);
    let selected_text = lines(selected.backend().buffer()).join("\n");
    assert!(
        selected_text.contains("BETA_TOP"),
        "a new detail kept the previous entry's bottom offset: {selected_text}"
    );
    assert!(
        !selected_text.contains("Not run yet"),
        "a new detail did not return to its first viewport: {selected_text}"
    );
}

#[test]
fn library_detail_focus_requires_a_matching_primary_release() {
    let view = state(vec![
        entry(
            "alpha",
            "Alpha",
            "python",
            StorageMode::Copy,
            "Alpha detail",
            None,
        ),
        entry(
            "beta",
            "Beta",
            "python",
            StorageMode::Copy,
            "Beta detail",
            None,
        ),
    ]);
    let detail = (75, 7);

    for (release, label) in [
        ((2, 7), "the Library list"),
        ((0, 0), "outside the Library body"),
    ] {
        let mut session = TuiSession::default();
        let (_, geometry) = draw_with_session(&view, &mut session, 100, 18);
        assert!(geometry.detail_pane_visible);
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: detail.0,
                    row: detail.1,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
                &geometry,
            ),
            EventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column: release.0,
                    row: release.1,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
                &geometry,
            ),
            EventHandling::Ignored,
            "release over {label} activated detail focus"
        );
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                &view,
                &geometry,
            ),
            EventHandling::Action(Action::Next),
            "a cancelled detail click stole Library-list keyboard focus"
        );
    }

    for button in [MouseButton::Right, MouseButton::Middle] {
        let mut session = TuiSession::default();
        let (_, geometry) = draw_with_session(&view, &mut session, 100, 18);
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: detail.0,
                    row: detail.1,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
                &geometry,
            ),
            EventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(button),
                    column: detail.0,
                    row: detail.1,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
                &geometry,
            ),
            EventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column: detail.0,
                    row: detail.1,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
                &geometry,
            ),
            EventHandling::Ignored,
            "{button:?} did not cancel the armed detail focus"
        );
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                &view,
                &geometry,
            ),
            EventHandling::Action(Action::Next)
        );
    }

    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 100, 18);
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: detail.0,
                row: detail.1,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: detail.0,
                row: detail.1,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &view,
            &geometry,
        ),
        EventHandling::Action(Action::OpenRun),
        "the focused detail viewport must leave Enter for Library activation"
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)),
            &view,
            &geometry,
        ),
        EventHandling::Ignored,
        "an unrelated key must pass through the focused detail viewport"
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            &view,
            &geometry,
        ),
        EventHandling::Consumed,
        "a same-target detail click did not transfer keyboard focus"
    );
}

#[test]
fn hidden_library_detail_releases_keyboard_focus_after_resize() {
    let mut view = state(vec![
        entry("alpha", "Alpha", "python", StorageMode::Copy, "Alpha", None),
        entry("beta", "Beta", "python", StorageMode::Copy, "Beta", None),
    ]);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 100, 18);
    assert!(geometry.detail_pane_visible);
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 75,
                row: 7,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 75,
                row: 7,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
            &geometry,
        ),
        EventHandling::Consumed,
        "the fixture must focus the visible detail before it hides"
    );

    let (_, hidden) = draw_with_session(&view, &mut session, 46, 12);
    assert!(!hidden.detail_pane_visible);
    let handling = session.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        &view,
        &hidden,
    );
    assert_eq!(
        handling,
        EventHandling::Action(Action::Next),
        "a hidden detail pane must not consume Library navigation"
    );
    if let EventHandling::Action(action) = handling {
        view.update(action);
    }
    assert_eq!(
        view.selected().map(|entry| entry.name.as_str()),
        Some("Beta")
    );
}

#[test]
fn library_detail_renders_overflow_parameters_and_every_last_run_age_and_exit_shape() {
    let item = entry(
        "history",
        "History",
        "python",
        StorageMode::Copy,
        "History detail",
        None,
    );
    let parameters = (0..7)
        .map(|index| LibraryParameterDetail {
            key: format!("p{index}"),
            value: if index == 0 {
                String::new()
            } else {
                index.to_string()
            },
            secret: false,
        })
        .collect();
    let hours = LibraryState::from_surface(
        LibraryScan {
            entries: vec![item.clone()],
            diagnostics: Vec::new(),
        },
        BTreeMap::from([(
            item.slug.clone(),
            LibraryEntryDetail {
                parameters,
                last_run: Some(LibraryLastRun {
                    at: "2026-08-20T00:00:00Z".to_owned(),
                    age: LibraryRunAge::Hours(3),
                    exit: None,
                }),
                ..LibraryEntryDetail::default()
            },
        )]),
    );
    let rendered = lines(draw(&hours, 220, 40, Locale::En).backend().buffer()).join("\n");
    assert!(rendered.contains("Parameters  p0  p1=1"), "{rendered}");
    assert!(rendered.contains('…'), "{rendered}");
    assert!(
        rendered.contains("Last run  3 h ago · ✗ failed (code None)"),
        "{rendered}"
    );

    let exact_parameters = (0..6)
        .map(|index| LibraryParameterDetail {
            key: format!("p{index}"),
            value: index.to_string(),
            secret: false,
        })
        .collect();
    let exact = LibraryState::from_surface(
        LibraryScan {
            entries: vec![item.clone()],
            diagnostics: Vec::new(),
        },
        BTreeMap::from([(
            item.slug.clone(),
            LibraryEntryDetail {
                parameters: exact_parameters,
                ..LibraryEntryDetail::default()
            },
        )]),
    );
    let exact_lines = lines(draw(&exact, 220, 40, Locale::En).backend().buffer());
    let (_, parameter_line) = line_with(&exact_lines, "Parameters");
    assert!(parameter_line.contains("p5=5"), "{parameter_line}");
    assert!(
        !parameter_line.contains('…'),
        "exactly six parameters advertised hidden content: {parameter_line}"
    );

    for (age, expected) in [
        (LibraryRunAge::Raw("earlier".to_owned()), "earlier"),
        (LibraryRunAge::JustNow, "just now"),
        (LibraryRunAge::Days(2), "2 d ago"),
    ] {
        let raw = LibraryState::from_surface(
            LibraryScan {
                entries: vec![item.clone()],
                diagnostics: Vec::new(),
            },
            BTreeMap::from([(
                item.slug.clone(),
                LibraryEntryDetail {
                    last_run: Some(LibraryLastRun {
                        at: "2026-08-20T00:00:00Z".to_owned(),
                        age,
                        exit: Some(0),
                    }),
                    ..LibraryEntryDetail::default()
                },
            )]),
        );
        let rendered = lines(draw(&raw, 180, 30, Locale::En).backend().buffer()).join("\n");
        assert!(rendered.contains(expected), "{rendered}");
        assert!(rendered.contains('✓'), "{rendered}");
    }
}

// ---------------------------------------------------------------------------
// Library list viewport: the wheel scrolls it, the keyboard realigns it.
// ---------------------------------------------------------------------------

/// Rows per wheel notch. `ratatui-interact` owns this constant for every scroll surface.
const WHEEL_ROWS: usize = 3;

/// A library big enough that the 46x12 list viewport must scroll.
fn numbered_library(count: usize) -> LibraryState {
    state(
        (0..count)
            .map(|index| {
                entry(
                    &format!("entry-{index}"),
                    &format!("Entry {index}"),
                    "python",
                    StorageMode::Copy,
                    "",
                    None,
                )
            })
            .collect(),
    )
}

fn wheel_at(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// Report every rendered row index that carries the selected-row background.
fn selected_rows(terminal: &Terminal<TestBackend>, rows: Rect) -> Vec<u16> {
    let buffer = terminal.backend().buffer();
    (rows.y..rows.y.saturating_add(rows.height))
        .filter(|row| buffer[(rows.x, *row)].bg == SELECT_BG)
        .collect()
}

/// Send one Library key through the session and apply the action it produces.
fn press(
    session: &mut TuiSession,
    view: &mut LibraryState,
    geometry: &ViewGeometry,
    code: KeyCode,
) {
    let handling = session.handle_event(
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        view,
        geometry,
    );
    let EventHandling::Action(action) = handling else {
        panic!("{code:?} produced no Library action: {handling:?}");
    };
    view.update(action);
}

/// Scroll the Library rows by whole notches and check that every notch is consumed.
fn wheel_rows(
    session: &mut TuiSession,
    view: &LibraryState,
    geometry: &ViewGeometry,
    kind: MouseEventKind,
    notches: usize,
) {
    for _ in 0..notches {
        assert_eq!(
            session.handle_event(
                wheel_at(kind, geometry.rows.x, geometry.rows.y),
                view,
                geometry,
            ),
            EventHandling::Consumed
        );
    }
}

#[test]
fn a_boundary_key_at_the_first_entry_reveals_the_scrolled_library_selection() {
    for code in [KeyCode::Up, KeyCode::Home, KeyCode::PageUp] {
        let mut view = numbered_library(12);
        let mut session = TuiSession::default();
        let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
        wheel_rows(
            &mut session,
            &view,
            &geometry,
            MouseEventKind::ScrollDown,
            2,
        );
        let (_, scrolled) = draw_with_session(&view, &mut session, 46, 12);
        assert_eq!(
            scrolled.first_visible, 6,
            "the fixture must scroll the selection out of view"
        );
        assert!(
            selected_rows(
                &draw_with_session(&view, &mut session, 46, 12).0,
                scrolled.rows
            )
            .is_empty()
        );

        press(&mut session, &mut view, &scrolled, code);
        assert_eq!(
            view.selected_visible_index(),
            Some(0),
            "{code:?}: the reducer clamps at the first entry"
        );

        let (terminal, revealed) = draw_with_session(&view, &mut session, 46, 12);
        assert_eq!(
            revealed.first_visible, 0,
            "{code:?} must reveal the selection the wheel scrolled away"
        );
        assert_eq!(
            selected_rows(&terminal, revealed.rows),
            vec![revealed.rows.y],
            "{code:?} must paint the selected row"
        );

        let (_, settled) = draw_with_session(&view, &mut session, 46, 12);
        assert_eq!(
            settled.first_visible, 0,
            "{code:?}: a render with no input must keep the offset"
        );
    }
}

#[test]
fn a_library_action_that_is_not_navigation_keeps_the_wheel_offset() {
    let view = numbered_library(12);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
    wheel_rows(
        &mut session,
        &view,
        &geometry,
        MouseEventKind::ScrollDown,
        2,
    );
    let (_, scrolled) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(scrolled.first_visible, 6);

    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
            &view,
            &scrolled,
        ),
        EventHandling::Action(Action::Rerun),
        "the fixture must produce a Library action that does not navigate"
    );
    let (_, after) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        after.first_visible, 6,
        "only a navigation action may pull the viewport back to the selection"
    );
}

#[test]
fn a_boundary_key_at_the_last_entry_reveals_the_scrolled_library_selection() {
    for code in [KeyCode::Down, KeyCode::End, KeyCode::PageDown] {
        let mut view = numbered_library(12);
        let mut session = TuiSession::default();
        let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
        let visible = usize::from(geometry.rows.height);
        press(&mut session, &mut view, &geometry, KeyCode::End);
        assert_eq!(view.selected_visible_index(), Some(11));
        let (_, followed) = draw_with_session(&view, &mut session, 46, 12);
        assert_eq!(followed.first_visible, 12 - visible);

        wheel_rows(&mut session, &view, &followed, MouseEventKind::ScrollUp, 3);
        let (terminal, away) = draw_with_session(&view, &mut session, 46, 12);
        assert_eq!(
            away.first_visible, 0,
            "the fixture must scroll the selection out of view"
        );
        assert!(selected_rows(&terminal, away.rows).is_empty());

        press(&mut session, &mut view, &away, code);
        assert_eq!(
            view.selected_visible_index(),
            Some(11),
            "{code:?}: the reducer clamps at the last entry"
        );

        let (terminal, revealed) = draw_with_session(&view, &mut session, 46, 12);
        assert_eq!(
            revealed.first_visible,
            12 - visible,
            "{code:?} must reveal the selection the wheel scrolled away"
        );
        assert_eq!(
            selected_rows(&terminal, revealed.rows),
            vec![
                revealed
                    .rows
                    .y
                    .saturating_add(revealed.rows.height)
                    .saturating_sub(1)
            ],
            "{code:?} must paint the selected row"
        );

        let (_, settled) = draw_with_session(&view, &mut session, 46, 12);
        assert_eq!(
            settled.first_visible,
            12 - visible,
            "{code:?}: a render with no input must keep the offset"
        );
    }
}

#[test]
fn a_library_wheel_notch_scrolls_the_viewport_and_keeps_the_selection() {
    let view = numbered_library(12);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(geometry.rows.height, 5, "the 46x12 list shows five rows");
    assert_eq!(geometry.first_visible, 0);

    assert_eq!(
        session.handle_event(
            wheel_at(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
            &view,
            &geometry,
        ),
        EventHandling::Consumed,
        "the Library rows viewport must own the wheel"
    );
    assert_eq!(
        view.selected_visible_index(),
        Some(0),
        "a wheel scroll must not move the selection"
    );

    let (terminal, scrolled) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        scrolled.first_visible, WHEEL_ROWS,
        "one notch must scroll exactly three rows"
    );
    let rendered = lines(terminal.backend().buffer()).join("\n");
    assert!(rendered.contains("Entry 3"), "{rendered}");
    assert!(rendered.contains("Entry 7"), "{rendered}");
    assert!(!rendered.contains("Entry 2"), "{rendered}");
    assert!(!rendered.contains("Entry 8"), "{rendered}");
    assert!(
        selected_rows(&terminal, scrolled.rows).is_empty(),
        "a selection scrolled out of view must not paint a row: {rendered}"
    );
}

#[test]
fn a_library_render_without_input_keeps_the_wheel_offset() {
    let view = numbered_library(12);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        session.handle_event(
            wheel_at(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (_, scrolled) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(scrolled.first_visible, WHEEL_ROWS);

    let (_, settled) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        settled.first_visible, WHEEL_ROWS,
        "a render with no new input must keep the reader's offset"
    );
}

#[test]
fn the_library_wheel_clamps_at_both_ends_of_the_entry_list() {
    let view = numbered_library(12);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
    let maximum = 12 - usize::from(geometry.rows.height);

    for _ in 0..10 {
        assert_eq!(
            session.handle_event(
                wheel_at(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
                &view,
                &geometry,
            ),
            EventHandling::Consumed
        );
    }
    let (terminal, bottom) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        bottom.first_visible, maximum,
        "the wheel must stop at the last full screen of entries"
    );
    let rendered = lines(terminal.backend().buffer()).join("\n");
    assert!(rendered.contains("Entry 11"), "{rendered}");

    for _ in 0..10 {
        assert_eq!(
            session.handle_event(
                wheel_at(MouseEventKind::ScrollUp, bottom.rows.x, bottom.rows.y),
                &view,
                &bottom,
            ),
            EventHandling::Consumed
        );
    }
    let (_, top) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(top.first_visible, 0, "the wheel must stop at the first row");
}

#[test]
fn a_click_on_a_scrolled_library_row_selects_the_entry_that_row_shows() {
    let view = numbered_library(12);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        session.handle_event(
            wheel_at(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (terminal, scrolled) = draw_with_session(&view, &mut session, 46, 12);

    let offset = 2_u16;
    let row = scrolled.rows.y.saturating_add(offset);
    assert!(
        lines(terminal.backend().buffer())[usize::from(row)].contains("Entry 5"),
        "the third visible row must show the sixth entry"
    );
    assert_eq!(
        session.handle_event(
            wheel_at(
                MouseEventKind::Down(MouseButton::Left),
                scrolled.rows.x,
                row
            ),
            &view,
            &scrolled,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            wheel_at(MouseEventKind::Up(MouseButton::Left), scrolled.rows.x, row),
            &view,
            &scrolled,
        ),
        EventHandling::Action(Action::SelectVisible(
            scrolled.first_visible + usize::from(offset)
        )),
        "a click must select the entry the scrolled row shows"
    );
}

#[test]
fn the_library_realigns_only_when_the_selection_leaves_the_viewport() {
    let mut view = numbered_library(12);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
    let visible = usize::from(geometry.rows.height);
    assert_eq!(
        session.handle_event(
            wheel_at(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );

    // The last row inside the viewport keeps the offset the reader chose.
    for _ in 0..WHEEL_ROWS + visible - 1 {
        view.update(Action::Next);
    }
    assert_eq!(
        view.selected_visible_index(),
        Some(WHEEL_ROWS + visible - 1)
    );
    let (_, inside) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        inside.first_visible, WHEEL_ROWS,
        "a selection on the last visible row must not scroll the viewport"
    );

    // The first row outside it scrolls by exactly one.
    view.update(Action::Next);
    let (_, outside) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        outside.first_visible,
        WHEEL_ROWS + 1,
        "the first row below the viewport must scroll it by one row"
    );
}

#[test]
fn keyboard_selection_realigns_the_scrolled_library_viewport_from_both_sides() {
    let mut view = numbered_library(12);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
    let visible = usize::from(geometry.rows.height);

    // A selection below the viewport lands on the last visible row.
    for _ in 0..8 {
        view.update(Action::Next);
    }
    assert_eq!(view.selected_visible_index(), Some(8));
    let (_, followed) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(followed.first_visible, 9 - visible);

    for _ in 0..2 {
        assert_eq!(
            session.handle_event(
                wheel_at(MouseEventKind::ScrollUp, followed.rows.x, followed.rows.y),
                &view,
                &followed,
            ),
            EventHandling::Consumed
        );
    }
    let (_, away) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(away.first_visible, 0, "the wheel must leave the selection");

    view.update(Action::Next);
    let (terminal, realigned) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        realigned.first_visible,
        10 - visible,
        "keyboard selection must realign with the smallest offset"
    );
    assert_eq!(
        selected_rows(&terminal, realigned.rows),
        vec![
            realigned
                .rows
                .y
                .saturating_add(realigned.rows.height)
                .saturating_sub(1)
        ],
        "the realigned selection must paint the last visible row"
    );

    // A selection above the viewport lands on the first visible row.
    for _ in 0..3 {
        assert_eq!(
            session.handle_event(
                wheel_at(
                    MouseEventKind::ScrollDown,
                    realigned.rows.x,
                    realigned.rows.y
                ),
                &view,
                &realigned,
            ),
            EventHandling::Consumed
        );
    }
    let (_, below) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(below.first_visible, 12 - visible);
    for _ in 0..7 {
        view.update(Action::Previous);
    }
    assert_eq!(view.selected_visible_index(), Some(2));
    let (terminal, above) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        above.first_visible, 2,
        "a selection above the viewport must become its first row"
    );
    assert_eq!(
        selected_rows(&terminal, above.rows),
        vec![above.rows.y],
        "the realigned selection must paint the first visible row"
    );
}

#[test]
fn filtering_the_library_clamps_the_viewport_to_the_shorter_list() {
    let mut view = numbered_library(20);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 46, 12);
    view.update(Action::End);
    let (_, followed) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(view.selected_visible_index(), Some(19));
    assert_eq!(
        followed.first_visible, 15,
        "the last entry pins the viewport to the tail of twenty entries"
    );
    let _ = geometry;

    // A query drops the list from twenty entries to eleven. The offset the long list earned is
    // past the tail of the short one, so the render must clamp it before it draws.
    view.update(Action::BeginSearch);
    view.update(Action::SetSearchQuery("Entry 1".to_owned()));
    let (terminal, filtered) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(
        view.visible_entry_count(),
        11,
        "the query must keep the eleven entries whose name holds a 1"
    );
    assert_eq!(
        filtered.first_visible, 5,
        "a shorter list must clamp the offset to its last full screen"
    );

    let rendered = lines(terminal.backend().buffer());
    let drawn = (filtered.rows.y..filtered.rows.y.saturating_add(filtered.rows.height))
        .map(|row| {
            rendered[usize::from(row)]
                .trim_start_matches('│')
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        drawn,
        vec![
            "Entry 14", "Entry 15", "Entry 16", "Entry 17", "Entry 18", "Entry 19"
        ],
        "an unclamped offset draws the last entry over blank rows"
    );
}

#[test]
fn a_shorter_terminal_keeps_the_selected_library_row_visible() {
    let mut view = numbered_library(12);
    let mut session = TuiSession::default();
    let _ = draw_with_session(&view, &mut session, 46, 12);
    for _ in 0..11 {
        view.update(Action::Next);
    }
    let (_, tall) = draw_with_session(&view, &mut session, 46, 12);
    assert_eq!(tall.first_visible, 12 - usize::from(tall.rows.height));

    let (_, short) = draw_with_session(&view, &mut session, 46, 10);
    assert!(
        short.rows.height < tall.rows.height,
        "the fixture must shrink the list viewport"
    );
    assert_eq!(
        short.first_visible,
        12 - usize::from(short.rows.height),
        "a resize must realign the selected row into the shorter viewport"
    );
}

#[test]
fn a_wheel_over_the_library_detail_pane_leaves_the_list_offset() {
    let view = numbered_library(40);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 100, 18);
    assert!(geometry.detail_pane_visible);
    assert_eq!(
        session.handle_event(
            wheel_at(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (_, scrolled) = draw_with_session(&view, &mut session, 100, 18);
    assert_eq!(scrolled.first_visible, WHEEL_ROWS);

    assert_eq!(
        session.handle_event(
            wheel_at(MouseEventKind::ScrollDown, 75, 7),
            &view,
            &scrolled,
        ),
        EventHandling::Consumed,
        "the detail pane keeps its own wheel"
    );
    let (_, after) = draw_with_session(&view, &mut session, 100, 18);
    assert_eq!(
        after.first_visible, WHEEL_ROWS,
        "a detail-pane wheel must not move the list viewport"
    );
}

#[test]
fn a_library_wheel_returns_keyboard_focus_to_the_list_pane() {
    let view = numbered_library(40);
    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&view, &mut session, 100, 18);
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        assert_eq!(
            session.handle_event(wheel_at(kind, 75, 7), &view, &geometry),
            EventHandling::Consumed
        );
    }
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            &view,
            &geometry,
        ),
        EventHandling::Consumed,
        "the fixture must give the detail pane keyboard focus"
    );

    assert_eq!(
        session.handle_event(
            wheel_at(MouseEventKind::ScrollDown, geometry.rows.x, geometry.rows.y),
            &view,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            &view,
            &geometry,
        ),
        EventHandling::Action(Action::Next),
        "a wheel over the rows must return keyboard focus to the list"
    );
}
