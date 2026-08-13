//! Observable Ratatui equivalents for Python `tests/test_tui_responsive.py` at `main@206f9ef`.
//!
//! Python's Textual CSS class names are not copied into Rust. These tests pin the same terminal
//! outcomes through the real `TestBackend + TuiSession + LibraryState` path: breakpoint edges,
//! responsive detail placement, indivisible/clickable footer buttons, scroll reachability, modal
//! fit, and short-screen focus scrolling. Two genuinely Textual-only widget-shape contracts are
//! classified by the companion manifest instead of being replaced by weaker stand-ins.

use std::collections::{BTreeMap, BTreeSet};

use ratatui_core::{backend::TestBackend, buffer::Buffer, layout::Rect, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
use skit_application::{
    LibraryScan,
    preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesSnapshot,
    },
    tokens::TokenContext,
};
use skit_domain::{
    EntryKind, EntrySummary, Slug, StorageMode,
    parameters::ParamDecl,
};
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddScreenEvent, AddScreenSession, AddTextField, EventHandling, HitTarget,
    TuiSession, ViewGeometry, render_add, render_with_session,
};
use skit_ui::{
    Action, AddWorkflowState, FormField, FormPurpose, FormView, LibraryState, PreferencesAction,
    PreferencesControlId, PreferencesView, RunFormContext, RunFormView, Screen, UiCommand,
};

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn click(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(ratatui_crossterm::crossterm::event::MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn scroll_down(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn entry(slug: &str, name: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        description: String::new(),
        target: None,
    }
}

fn library(entries: Vec<EntrySummary>) -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries,
        diagnostics: Vec::new(),
    })
}

fn draw(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    (terminal, geometry)
}

fn drive(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    event: Event,
) -> EventHandling {
    let handling = session.handle_event(event, state, geometry);
    if let EventHandling::Action(action) = &handling {
        state.update(action.clone());
    }
    handling
}

fn lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|row| {
            (0..buffer.area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn rendered(buffer: &Buffer) -> String {
    lines(buffer).join("\n")
}

fn position(buffer: &Buffer, needle: &str) -> Option<(usize, usize)> {
    lines(buffer)
        .into_iter()
        .enumerate()
        .find_map(|(row, line)| line.find(needle).map(|column| (row, column)))
}

fn command_hit(geometry: &ViewGeometry, command: UiCommand) -> Option<Rect> {
    geometry.hits.iter().find_map(|hit| {
        (hit.action == HitTarget::Command(command)).then_some(hit.rect)
    })
}

fn command_rows(geometry: &ViewGeometry) -> BTreeSet<u16> {
    geometry
        .hits
        .iter()
        .filter_map(|hit| matches!(hit.action, HitTarget::Command(_)).then_some(hit.rect.y))
        .collect()
}

fn hit_text(buffer: &Buffer, area: Rect) -> String {
    (0..area.width)
        .map(|offset| buffer[(area.x.saturating_add(offset), area.y)].symbol())
        .collect()
}

fn bordered_control_height(buffer: &Buffer, title: &str) -> usize {
    let rows = lines(buffer);
    let Some((top, _)) = rows
        .iter()
        .enumerate()
        .find(|(_, row)| row.contains(title))
    else {
        panic!("missing control title {title:?}:\n{}", rows.join("\n"));
    };
    let has_top_border = rows[top].contains('┌') || rows[top].contains('╭');
    if !has_top_border {
        return 1;
    }
    rows.iter()
        .enumerate()
        .skip(top + 1)
        .find(|(_, row)| row.contains('└') || row.contains('╰'))
        .map_or(1, |(bottom, _)| bottom.saturating_sub(top).saturating_add(1))
}

fn preferences() -> PreferencesView {
    PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
        effective_language: "en".to_owned(),
        editor: String::new(),
        editor_fallback: Some("vim".to_owned()),
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Exit,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runner_names: Vec::new(),
        mirror: MirrorConfiguration::default(),
    }))
}

fn form_state() -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Rename,
        title: "Responsive form".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: Some("demo".to_owned()),
        fields: vec![
            FormField::text_raw("first", "First", ""),
            FormField::text_raw("second", "Second", ""),
        ],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    state
}

#[test]
fn test_breakpoint_tiers_are_the_documented_contract() {
    let state = library(vec![entry("alpha", "Alpha")]);

    let mut session = TuiSession::default();
    let (wide, _) = draw(&mut session, &state, 80, 24);
    let library_wide = position(wide.backend().buffer(), "Library").unwrap();
    let detail_wide = position(wide.backend().buffer(), "Detail pane").unwrap();
    assert_eq!(detail_wide.0, library_wide.0, "80 columns must be the normal-width tier");
    assert!(detail_wide.1 > library_wide.1);

    let mut session = TuiSession::default();
    let (narrow, _) = draw(&mut session, &state, 79, 24);
    let library_narrow = position(narrow.backend().buffer(), "Library").unwrap();
    let detail_narrow = position(narrow.backend().buffer(), "Detail pane").unwrap();
    assert!(detail_narrow.0 > library_narrow.0, "79 columns must be narrow");
    assert_eq!(detail_narrow.1, library_narrow.1);

    let footer_rows = |height| {
        let mut session = TuiSession::default();
        let (_, geometry) = draw(&mut session, &state, 26, height);
        command_rows(&geometry).len()
    };
    assert!(footer_rows(28) > 6, "28 rows must remove the footer-row cap");
    assert_eq!(footer_rows(27), 6, "27 rows is still the normal-height tier");
    assert_eq!(footer_rows(16), 6, "16 rows starts the normal-height tier");
    assert_eq!(footer_rows(15), 2, "15 rows is the short tier");
    assert_eq!(footer_rows(10), 2, "10 rows starts the short tier");
    assert_eq!(footer_rows(9), 1, "9 rows is the tiny tier");
}

#[test]
fn test_chip_glues_every_blank_so_the_pill_is_one_word() {
    let state = library(vec![entry("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 44, 24);

    for command in [UiCommand::Search, UiCommand::ToggleDetail, UiCommand::Help] {
        let area = command_hit(&geometry, command)
            .unwrap_or_else(|| panic!("{command:?} footer chip is not visible"));
        assert_eq!(area.height, 1, "one footer pill was split vertically");
        assert!(area.width > 0);
    }
    let search = command_hit(&geometry, UiCommand::Search).unwrap();
    let search_text = hit_text(terminal.backend().buffer(), search);
    assert!(search_text.contains('/'), "{search_text:?}");
    assert!(search_text.contains("Search"), "{search_text:?}");
    let detail = command_hit(&geometry, UiCommand::ToggleDetail).unwrap();
    let detail_text = hit_text(terminal.backend().buffer(), detail);
    assert!(detail_text.contains("Tab"), "{detail_text:?}");
    assert!(detail_text.contains("Detail"), "{detail_text:?}");
}

#[test]
fn test_nav_chip_is_exactly_the_two_key_only_pills() {
    let state = form_state();
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 100, 22);
    let text = rendered(terminal.backend().buffer());

    assert!(text.contains("Tab/↓"), "{text}");
    assert!(text.contains("Shift+Tab/↑"), "{text}");
    assert!(!text.contains("Next field"), "navigation pill gained a label: {text}");
    assert!(!text.contains("Previous field"), "navigation pill gained a label: {text}");
    assert_eq!(
        geometry
            .hits
            .iter()
            .filter(|hit| hit.action == HitTarget::Command(UiCommand::FocusNext))
            .count(),
        1
    );
    assert_eq!(
        geometry
            .hits
            .iter()
            .filter(|hit| hit.action == HitTarget::Command(UiCommand::FocusPrevious))
            .count(),
        1
    );
}

#[test]
fn test_width_tier_boundary_flips_side_by_side_to_stacked() {
    let state = library(vec![entry("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (normal, _) = draw(&mut session, &state, 80, 24);
    let list = position(normal.backend().buffer(), "Library").unwrap();
    let detail = position(normal.backend().buffer(), "Detail pane").unwrap();
    assert_eq!(detail.0, list.0);
    assert!(detail.1 > list.1);

    let mut session = TuiSession::default();
    let (narrow, _) = draw(&mut session, &state, 79, 24);
    let list = position(narrow.backend().buffer(), "Library").unwrap();
    let detail = position(narrow.backend().buffer(), "Detail pane").unwrap();
    assert!(detail.0 > list.0);
    assert_eq!(detail.1, list.1);
}

#[test]
fn test_narrow_short_hides_detail_and_tab_pin_survives_resizes() {
    let mut state = library(vec![entry("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 70, 12);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());

    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Tab, KeyModifiers::NONE),
        ),
        EventHandling::Action(Action::ToggleDetail)
    );
    let (terminal, _) = draw(&mut session, &state, 70, 12);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
    let (terminal, _) = draw(&mut session, &state, 120, 12);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
    let (terminal, geometry) = draw(&mut session, &state, 70, 12);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());

    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Tab, KeyModifiers::NONE),
    );
    let (terminal, geometry) = draw(&mut session, &state, 70, 12);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Tab, KeyModifiers::NONE),
    );
    let (terminal, _) = draw(&mut session, &state, 70, 12);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
}

#[test]
fn test_tiny_narrow_tab_still_brings_the_pane_back() {
    let mut state = library(vec![entry("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 46, 9);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());

    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Tab, KeyModifiers::NONE),
    );
    let (terminal, _) = draw(&mut session, &state, 46, 9);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
}

#[test]
fn test_tab_walks_the_pin_states_on_a_wide_terminal_too() {
    let mut state = library(vec![entry("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    for expected in [true, false, true, false] {
        let (terminal, geometry) = draw(&mut session, &state, 120, 24);
        assert_eq!(
            position(terminal.backend().buffer(), "Detail pane").is_some(),
            expected
        );
        if expected != false || state.detail_pane_mode() != skit_ui::DetailPaneMode::PinnedClosed {
            let _ = drive(
                &mut session,
                &mut state,
                &geometry,
                key(KeyCode::Tab, KeyModifiers::NONE),
            );
        }
    }
}

#[test]
fn test_height_tier_boundaries_flatten_search_then_drop_the_global_row() {
    let mut search_state = library(vec![entry("alpha", "Alpha")]);
    search_state.update(Action::BeginSearch);
    let mut session = TuiSession::default();
    let (normal, _) = draw(&mut session, &search_state, 100, 16);
    assert_eq!(bordered_control_height(normal.backend().buffer(), "Search"), 3);
    let mut session = TuiSession::default();
    let (short, _) = draw(&mut session, &search_state, 100, 15);
    assert_eq!(
        bordered_control_height(short.backend().buffer(), "Search"),
        1,
        "short-tier Search must flatten instead of spending three rows"
    );

    let browse = library(vec![entry("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (ten, ten_geometry) = draw(&mut session, &browse, 100, 10);
    assert_eq!(command_rows(&ten_geometry).len(), 2);
    assert!(rendered(ten.backend().buffer()).contains("1/1 entry"));
    let mut session = TuiSession::default();
    let (nine, nine_geometry) = draw(&mut session, &browse, 100, 9);
    assert_eq!(command_rows(&nine_geometry).len(), 1);
    assert!(rendered(nine.backend().buffer()).contains("1/1 entry"));
}

#[test]
fn test_flattened_search_still_filters() {
    let mut state = library(vec![entry("alpha", "alpha"), entry("beta", "beta")]);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 12);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('/'), KeyModifiers::NONE),
    );
    for character in ['b', 'e'] {
        let (_, geometry) = draw(&mut session, &state, 100, 12);
        let _ = drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char(character), KeyModifiers::NONE),
        );
    }
    assert_eq!(
        state
            .visible_entries()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["beta"]
    );
}

#[test]
fn test_footer_wraps_between_pills_and_wrapped_chips_stay_clickable() {
    let mut state = library(vec![entry("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 44, 24);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
    let search = command_hit(&geometry, UiCommand::Search).expect("Search footer chip");
    let tab = command_hit(&geometry, UiCommand::ToggleDetail).expect("Tab footer chip");
    assert!(tab.y > search.y, "the wrapped Tab pill did not move to a later row");

    assert_eq!(
        drive(&mut session, &mut state, &geometry, click(tab.x, tab.y)),
        EventHandling::Action(Action::ToggleDetail)
    );
    let (terminal, geometry) = draw(&mut session, &state, 44, 24);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());
    let tab = command_hit(&geometry, UiCommand::ToggleDetail).unwrap();
    let _ = drive(&mut session, &mut state, &geometry, click(tab.x, tab.y));
    let (terminal, _) = draw(&mut session, &state, 44, 24);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
}

#[test]
fn test_portrait_stacks_the_detail_pane_and_uncaps_the_footer() {
    let mut state = library(vec![entry("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 26, 44);
    let list = position(terminal.backend().buffer(), "Library").unwrap();
    let detail = position(terminal.backend().buffer(), "Detail pane").unwrap();
    assert!(detail.0 > list.0);
    assert_eq!(detail.1, list.1);
    assert!(command_rows(&geometry).len() > 3, "tall footer remained capped");

    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Tab, KeyModifiers::NONE),
    );
    let (terminal, geometry) = draw(&mut session, &state, 26, 44);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Tab, KeyModifiers::NONE),
    );
    let (terminal, _) = draw(&mut session, &state, 26, 44);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
}

#[test]
fn test_short_tier_caps_visible_lines_but_keeps_chips_scroll_reachable() {
    let state = library(vec![entry("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (_, mut geometry) = draw(&mut session, &state, 46, 12);
    assert_eq!(command_rows(&geometry).len(), 2);
    assert!(command_hit(&geometry, UiCommand::Help).is_none());

    for _ in 0..20 {
        if command_hit(&geometry, UiCommand::Help).is_some() {
            break;
        }
        let anchor = geometry
            .hits
            .iter()
            .find(|hit| matches!(hit.action, HitTarget::Command(_)))
            .expect("a visible footer chip anchors wheel scrolling")
            .rect;
        assert_eq!(
            session.handle_event(scroll_down(anchor.x, anchor.y), &state, &geometry),
            EventHandling::Consumed
        );
        let (_, next) = draw(&mut session, &state, 46, 12);
        geometry = next;
    }
    assert!(
        command_hit(&geometry, UiCommand::Help).is_some(),
        "the last global footer chip never became wheel-reachable"
    );
}

#[test]
fn test_prefs_mirror_rows_are_horizontal_until_narrow_and_sentences_always_stack() {
    let pairs = [
        (PreferencesControlId::PypiChoice, "tsinghua", "aliyun"),
        (PreferencesControlId::GithubChoice, "nju", "custom"),
        (PreferencesControlId::NpmChoice, "npmmirror", "custom"),
    ];
    for (control, first, second) in pairs {
        let mut wide_state = LibraryState::default();
        let mut wide = preferences();
        let _ = wide.update(PreferencesAction::Focus(control));
        wide_state.update(Action::Present(Screen::Preferences(Box::new(wide))));
        let mut session = TuiSession::default();
        let (terminal, _) = draw(&mut session, &wide_state, 120, 40);
        assert_eq!(
            position(terminal.backend().buffer(), first).unwrap().0,
            position(terminal.backend().buffer(), second).unwrap().0,
            "{control:?} did not stay horizontal when wide"
        );

        let mut narrow_state = LibraryState::default();
        let mut narrow = preferences();
        let _ = narrow.update(PreferencesAction::Focus(control));
        narrow_state.update(Action::Present(Screen::Preferences(Box::new(narrow))));
        let mut session = TuiSession::default();
        let (terminal, _) = draw(&mut session, &narrow_state, 60, 40);
        assert!(
            position(terminal.backend().buffer(), first).unwrap().0
                < position(terminal.backend().buffer(), second).unwrap().0,
            "{control:?} must stack in the narrow tier"
        );
    }

    let mut state = LibraryState::default();
    let mut view = preferences();
    let _ = view.update(PreferencesAction::Focus(PreferencesControlId::InteractiveForm));
    state.update(Action::Present(Screen::Preferences(Box::new(view))));
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 120, 40);
    assert!(
        position(terminal.backend().buffer(), "Mini form").unwrap().0
            < position(terminal.backend().buffer(), "Line-by-line prompts").unwrap().0,
        "sentence-length form choices must stack even on a wide terminal"
    );
}

#[test]
fn test_help_overlay_caps_to_a_tiny_screen_and_scrolls_by_key() {
    let mut state = library(vec![entry("alpha", "Alpha")]);
    state.update(Action::OpenHelp);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 40, 8);
    assert_eq!(geometry.first_visible, 0);
    assert!(geometry.rows.right() <= 40 && geometry.rows.bottom() <= 8);
    assert!(rendered(terminal.backend().buffer()).contains("Help"));

    assert_eq!(
        session.handle_event(
            key(KeyCode::Down, KeyModifiers::NONE),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (_, geometry) = draw(&mut session, &state, 40, 8);
    assert!(geometry.first_visible > 0, "Down did not reveal clipped help rows");
}

#[test]
fn test_confirm_remove_shrinks_for_a_long_name_on_a_narrow_screen() {
    let long_name = "a-script-with-a-name-far-wider-than-the-terminal-itself";
    let mut state = library(vec![entry("long", long_name)]);
    state.update(Action::AskRemove);
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 40, 20);
    let rows = lines(terminal.backend().buffer());
    let (top, _) = rows
        .iter()
        .enumerate()
        .find(|(_, row)| row.contains("Confirm removal"))
        .expect("confirmation title must be visible");
    let top_line = &rows[top];
    assert!(
        top_line.contains('┐') || top_line.contains('╮'),
        "narrow modal lost its right border: {top_line:?}"
    );
    let bottom = rows
        .iter()
        .enumerate()
        .skip(top + 1)
        .find(|(_, row)| row.contains('└') || row.contains('╰'))
        .map(|(row, _)| row)
        .expect("narrow modal lost its bottom border");
    assert!(bottom < 20);
    let text = rows.join("\n");
    assert!(text.contains("a-script-with-a-name"), "{text}");
    assert!(text.contains("terminal-itself"), "{text}");
}

#[test]
fn test_env_picker_fits_input_and_esc_chip_across_the_tiers() {
    let declaration = ParamDecl::new("value");
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[declaration],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "command".to_owned(),
        path: None,
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/demo".to_owned()),
            env: BTreeMap::from([
                ("HOME".to_owned(), "/home/demo".to_owned()),
                ("PATH".to_owned(), "/bin".to_owned()),
            ]),
            today: "2026-08-12".to_owned(),
            now: "17-00-00".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state.update(Action::OpenRunTokenMenuFor(0));
    state.update(Action::OpenRunEnvironmentPicker(0));

    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 70, 20);
    assert!(rendered(terminal.backend().buffer()).contains("Environment variable"));
    assert!(command_hit(&geometry, UiCommand::CloseModal).is_some());

    let (terminal, geometry) = draw(&mut session, &state, 70, 10);
    assert_eq!(
        bordered_control_height(terminal.backend().buffer(), "type to filter…"),
        3,
        "the short-tier environment input was clipped"
    );
    let cancel = command_hit(&geometry, UiCommand::CloseModal)
        .expect("Esc/Cancel must remain on screen at the short-tier floor");
    assert!(cancel.bottom() <= 10);
}

#[test]
fn test_add_source_fields_stay_reachable_on_short_terminals() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &workflow, &mut session, Locale::En);
        })
        .unwrap();
    assert_eq!(
        session.focused(),
        Some(&AddControlId::Text(AddTextField::SourcePath))
    );

    for _ in 0..2 {
        if let Some(AddScreenEvent::Action(action)) = session.handle_event(
            key(KeyCode::Down, KeyModifiers::NONE),
            &workflow,
            &geometry,
        ) {
            let _ = workflow.reduce(action);
        }
        terminal
            .draw(|frame| {
                geometry = render_add(frame, frame.area(), &workflow, &mut session, Locale::En);
            })
            .unwrap();
    }

    let name = AddControlId::Text(AddTextField::CommandName);
    assert_eq!(session.focused(), Some(&name));
    let area = geometry
        .hits
        .iter()
        .find_map(|hit| (hit.target == name).then_some(hit.area))
        .expect("focused command-name field must be rendered after auto-scroll");
    assert_eq!(area.height, 3, "focused name input is clipped by the short viewport");
    assert!(area.bottom() <= geometry.body.bottom());
}
