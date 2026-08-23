//! Observable Ratatui equivalents for Python `tests/test_tui_responsive.py` at `main@206f9ef`.
//!
//! Textual CSS class names are implementation detail. These ports assert the same visible terminal
//! contract through real `TestBackend + TuiSession + LibraryState` geometry and interaction paths.

use std::collections::{BTreeMap, BTreeSet};

use ratatui_core::{backend::TestBackend, buffer::Buffer, layout::Rect, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::{
    LibraryScan,
    preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesSnapshot,
    },
    tokens::TokenContext,
};
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode, parameters::ParamDecl};
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddScreenEvent, AddScreenSession, AddTextField, EventHandling, HitTarget,
    TuiSession, ViewGeometry, render_add, render_with_session,
};
use skit_ui::{
    Action, AddWorkflowState, FormField, FormPurpose, FormView, LibraryState, PreferencesAction,
    PreferencesControlId, PreferencesView, RunFormContext, RunFormView, Screen, UiCommand,
};

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn click(area: Rect) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    })
}

fn scroll_down(area: Rect) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    })
}

fn summary(slug: &str, name: &str) -> EntrySummary {
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
        .draw(|frame| geometry = render_with_session(frame, state, Locale::En, session))
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
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn text(buffer: &Buffer) -> String {
    lines(buffer).join("\n")
}

fn position(buffer: &Buffer, needle: &str) -> Option<(usize, usize)> {
    let exact = match needle {
        "Library" => "╭ Library",
        "Detail pane" => "╭ Detail pane",
        other => other,
    };
    lines(buffer)
        .into_iter()
        .enumerate()
        .find_map(|(y, line)| line.find(exact).map(|x| (y, x)))
}

fn position_after(buffer: &Buffer, anchor: &str, needle: &str) -> Option<(usize, usize)> {
    let rows = lines(buffer);
    let anchor_y = rows.iter().position(|row| row.contains(anchor))?;
    rows.into_iter()
        .enumerate()
        .skip(anchor_y)
        .find_map(|(y, line)| line.find(needle).map(|x| (y, x)))
}

fn rows_with(buffer: &Buffer, needle: &str) -> Vec<usize> {
    lines(buffer)
        .into_iter()
        .enumerate()
        .filter_map(|(y, line)| line.contains(needle).then_some(y))
        .collect()
}

fn command_hit(geometry: &ViewGeometry, command: UiCommand) -> Option<Rect> {
    geometry
        .hits
        .iter()
        .find_map(|hit| (hit.action == HitTarget::Command(command)).then_some(hit.rect))
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
        .map(|dx| buffer[(area.x.saturating_add(dx), area.y)].symbol())
        .collect()
}

fn bordered_control_height(buffer: &Buffer, title: &str) -> usize {
    let rows = lines(buffer);
    let top = rows
        .iter()
        .position(|row| row.contains(title))
        .unwrap_or_else(|| panic!("missing {title:?}:\n{}", rows.join("\n")));
    if !(rows[top].contains('┌') || rows[top].contains('╭')) {
        return 1;
    }
    rows.iter()
        .enumerate()
        .skip(top + 1)
        .find(|(_, row)| row.contains('└') || row.contains('╰'))
        .map_or(1, |(bottom, _)| bottom - top + 1)
}

fn preferences() -> PreferencesView {
    PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".into(), "zh-CN".into(), "zh-TW".into()],
        effective_language: "en".into(),
        editor: String::new(),
        editor_fallback: Some("vim".into()),
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Exit,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runner_names: Vec::new(),
        mirror: MirrorConfiguration::default(),
    }))
}

fn generic_form_state() -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Rename,
        title: "Responsive form".into(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: Some("demo".into()),
        fields: vec![
            FormField::text_raw("first", "First", ""),
            FormField::text_raw("second", "Second", ""),
        ],
        focused: 0,
        submit_label: "Save".into(),
    })));
    state
}

#[test]
fn test_breakpoint_tiers_are_the_documented_contract() {
    let state = library(vec![summary("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (wide, _) = draw(&mut session, &state, 80, 24);
    let list = position(wide.backend().buffer(), "Library").unwrap();
    let detail = position(wide.backend().buffer(), "Detail pane").unwrap();
    assert_eq!(detail.0, list.0);
    assert!(detail.1 > list.1);

    let mut session = TuiSession::default();
    let (narrow, _) = draw(&mut session, &state, 79, 24);
    let list = position(narrow.backend().buffer(), "Library").unwrap();
    let detail = position(narrow.backend().buffer(), "Detail pane").unwrap();
    assert!(detail.0 > list.0);
    assert_eq!(detail.1, list.1);

    let footer_rows = |height| {
        let mut session = TuiSession::default();
        let (_, geometry) = draw(&mut session, &state, 26, height);
        command_rows(&geometry).len()
    };
    assert!(footer_rows(28) > 6);
    assert_eq!(footer_rows(27), 6);
    assert_eq!(footer_rows(16), 6);
    assert_eq!(footer_rows(15), 2);
    assert_eq!(footer_rows(10), 2);
    assert_eq!(footer_rows(9), 1);
}

#[test]
fn test_chip_glues_every_blank_so_the_pill_is_one_word() {
    let state = library(vec![summary("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 44, 24);
    for command in [UiCommand::Search, UiCommand::ToggleDetail, UiCommand::Help] {
        let area = command_hit(&geometry, command)
            .unwrap_or_else(|| panic!("{command:?} has no visible button"));
        assert_eq!(area.height, 1, "a footer button split across terminal rows");
        assert!(area.width > 0);
    }
    let search = hit_text(
        terminal.backend().buffer(),
        command_hit(&geometry, UiCommand::Search).unwrap(),
    );
    assert!(
        search.contains('/') && search.contains("Search"),
        "{search:?}"
    );
    let detail = hit_text(
        terminal.backend().buffer(),
        command_hit(&geometry, UiCommand::ToggleDetail).unwrap(),
    );
    assert!(
        detail.contains("Tab") && detail.contains("Detail"),
        "{detail:?}"
    );
}

#[test]
fn test_nav_chip_is_exactly_the_two_key_only_pills() {
    let state = generic_form_state();
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 100, 22);
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("Tab/↓"), "{rendered}");
    assert!(rendered.contains("Shift+Tab/↑"), "{rendered}");
    assert!(!rendered.contains("Next field"));
    assert!(!rendered.contains("Previous field"));
    for command in [UiCommand::FocusNext, UiCommand::FocusPrevious] {
        assert_eq!(
            geometry
                .hits
                .iter()
                .filter(|hit| hit.action == HitTarget::Command(command))
                .count(),
            1
        );
    }
}

#[test]
fn test_width_tier_boundary_flips_side_by_side_to_stacked() {
    let state = library(vec![summary("alpha", "Alpha")]);
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
    let mut state = library(vec![summary("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 70, 12);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Tab)),
        EventHandling::Action(Action::ToggleDetail {
            currently_visible: false,
        })
    );
    for (width, height) in [(70, 12), (120, 12), (70, 12)] {
        let (terminal, _) = draw(&mut session, &state, width, height);
        assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
    }
    let (_, geometry) = draw(&mut session, &state, 70, 12);
    let _ = drive(&mut session, &mut state, &geometry, key(KeyCode::Tab));
    let (terminal, geometry) = draw(&mut session, &state, 70, 12);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());
    let _ = drive(&mut session, &mut state, &geometry, key(KeyCode::Tab));
    let (terminal, _) = draw(&mut session, &state, 70, 12);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
}

#[test]
fn test_tiny_narrow_tab_still_brings_the_pane_back() {
    let mut state = library(vec![summary("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 46, 9);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());
    let _ = drive(&mut session, &mut state, &geometry, key(KeyCode::Tab));
    let (terminal, _) = draw(&mut session, &state, 46, 9);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
}

#[test]
fn test_tab_walks_the_pin_states_on_a_wide_terminal_too() {
    let mut state = library(vec![summary("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    for expected in [true, false, true, false] {
        let (terminal, geometry) = draw(&mut session, &state, 120, 24);
        assert_eq!(
            position(terminal.backend().buffer(), "Detail pane").is_some(),
            expected
        );
        let _ = drive(&mut session, &mut state, &geometry, key(KeyCode::Tab));
    }
}

#[test]
fn test_height_tier_boundaries_flatten_search_then_drop_the_global_row() {
    let mut search = library(vec![summary("alpha", "Alpha")]);
    search.update(Action::BeginSearch);
    let mut session = TuiSession::default();
    let (normal, _) = draw(&mut session, &search, 100, 16);
    assert_eq!(
        bordered_control_height(normal.backend().buffer(), "Search"),
        3
    );
    let mut session = TuiSession::default();
    let (short, _) = draw(&mut session, &search, 100, 15);
    assert_eq!(
        bordered_control_height(short.backend().buffer(), "Search"),
        1,
        "short-tier search must flatten"
    );

    let browse = library(vec![summary("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (ten, geometry) = draw(&mut session, &browse, 100, 10);
    assert_eq!(command_rows(&geometry).len(), 2);
    assert!(text(ten.backend().buffer()).contains("1/1 entry"));
    let mut session = TuiSession::default();
    let (nine, geometry) = draw(&mut session, &browse, 100, 9);
    assert_eq!(command_rows(&geometry).len(), 1);
    assert!(text(nine.backend().buffer()).contains("1/1 entry"));
}

#[test]
fn test_flattened_search_still_filters() {
    let mut state = library(vec![summary("alpha", "alpha"), summary("beta", "beta")]);
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 100, 12);
    let _ = drive(&mut session, &mut state, &geometry, key(KeyCode::Char('/')));
    for character in ['b', 'e'] {
        let (_, geometry) = draw(&mut session, &state, 100, 12);
        let _ = drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Char(character)),
        );
    }
    assert_eq!(
        state
            .visible_entries()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["beta"]
    );
}

#[test]
fn test_footer_wraps_between_pills_and_wrapped_chips_stay_clickable() {
    let mut state = library(vec![summary("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 44, 24);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
    let search = command_hit(&geometry, UiCommand::Search).unwrap();
    let detail = command_hit(&geometry, UiCommand::ToggleDetail).unwrap();
    assert!(detail.y > search.y, "Tab pill did not wrap after Search");
    assert_eq!(
        drive(&mut session, &mut state, &geometry, click(detail)),
        EventHandling::Action(Action::ToggleDetail {
            currently_visible: true,
        })
    );
    let (terminal, geometry) = draw(&mut session, &state, 44, 24);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());
    let detail = command_hit(&geometry, UiCommand::ToggleDetail).unwrap();
    let _ = drive(&mut session, &mut state, &geometry, click(detail));
    let (terminal, _) = draw(&mut session, &state, 44, 24);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
}

#[test]
fn test_portrait_stacks_the_detail_pane_and_uncaps_the_footer() {
    let mut state = library(vec![summary("alpha", "Alpha")]);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 26, 44);
    let list = position(terminal.backend().buffer(), "Library").unwrap();
    let detail = position(terminal.backend().buffer(), "Detail pane").unwrap();
    assert!(detail.0 > list.0);
    assert_eq!(detail.1, list.1);
    assert!(command_rows(&geometry).len() > 3);
    let _ = drive(&mut session, &mut state, &geometry, key(KeyCode::Tab));
    let (terminal, geometry) = draw(&mut session, &state, 26, 44);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_none());
    let _ = drive(&mut session, &mut state, &geometry, key(KeyCode::Tab));
    let (terminal, _) = draw(&mut session, &state, 26, 44);
    assert!(position(terminal.backend().buffer(), "Detail pane").is_some());
}

#[test]
fn test_short_tier_caps_visible_lines_but_keeps_chips_scroll_reachable() {
    let state = library(vec![summary("alpha", "Alpha")]);
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
            .expect("footer has a visible chip")
            .rect;
        assert_eq!(
            session.handle_event(scroll_down(anchor), &state, &geometry),
            EventHandling::Consumed
        );
        let (_, next) = draw(&mut session, &state, 46, 12);
        geometry = next;
    }
    assert!(command_hit(&geometry, UiCommand::Help).is_some());
}

#[test]
fn test_prefs_mirror_rows_are_horizontal_until_narrow_and_sentences_always_stack() {
    let rows = [
        (
            PreferencesControlId::PypiChoice,
            "PyPI index (Python packages)",
            "tsinghua",
            "aliyun",
        ),
        (
            PreferencesControlId::GithubChoice,
            "GitHub releases (Python builds, the uv binary)",
            "nju",
            "custom",
        ),
        (
            PreferencesControlId::NpmChoice,
            "npm registry (JS/TS packages)",
            "npmmirror",
            "custom",
        ),
    ];
    for (control, anchor, first, second) in rows {
        for (width, should_stack) in [(120, false), (60, true)] {
            let mut state = LibraryState::default();
            let mut view = preferences();
            let _ = view.update(PreferencesAction::Focus(control));
            state.update(Action::Present(Screen::Preferences(Box::new(view))));
            let mut session = TuiSession::default();
            let (terminal, _) = draw(&mut session, &state, width, 40);
            let first_y = position_after(terminal.backend().buffer(), anchor, first)
                .unwrap()
                .0;
            let second_y = position_after(terminal.backend().buffer(), anchor, second)
                .unwrap()
                .0;
            assert_eq!(
                first_y < second_y,
                should_stack,
                "{control:?} width={width}"
            );
        }
    }
    let mut state = LibraryState::default();
    let mut view = preferences();
    let _ = view.update(PreferencesAction::Focus(
        PreferencesControlId::InteractiveForm,
    ));
    state.update(Action::Present(Screen::Preferences(Box::new(view))));
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 120, 40);
    assert!(
        position(terminal.backend().buffer(), "Mini form")
            .unwrap()
            .0
            < position(terminal.backend().buffer(), "Line-by-line prompts")
                .unwrap()
                .0
    );
}

#[test]
fn test_help_overlay_caps_to_a_tiny_screen_and_scrolls_by_key() {
    let mut state = library(vec![summary("alpha", "Alpha")]);
    state.update(Action::OpenHelp);
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 40, 8);
    assert_eq!(geometry.first_visible, 0);
    assert!(geometry.rows.right() <= 40 && geometry.rows.bottom() <= 8);
    assert!(text(terminal.backend().buffer()).contains("Help"));
    assert_eq!(
        session.handle_event(key(KeyCode::Down), &state, &geometry),
        EventHandling::Consumed
    );
    let (_, geometry) = draw(&mut session, &state, 40, 8);
    assert!(geometry.first_visible > 0);
}

#[test]
fn test_confirm_remove_shrinks_for_a_long_name_on_a_narrow_screen() {
    let mut state = library(vec![summary(
        "long",
        "a-script-with-a-name-far-wider-than-the-terminal-itself",
    )]);
    state.update(Action::AskRemove);
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 40, 20);
    let rows = lines(terminal.backend().buffer());
    let titles = rows_with(terminal.backend().buffer(), "Confirm removal");
    assert!(
        titles.len() >= 2,
        "header rendered but popup title is missing: {rows:#?}"
    );
    let top = *titles.last().unwrap();
    assert!(rows[top].contains('┐') || rows[top].contains('╮'));
    let bottom = rows
        .iter()
        .enumerate()
        .skip(top + 1)
        .find(|(_, row)| row.contains('└') || row.contains('╰'))
        .map(|(y, _)| y)
        .expect("confirmation bottom border is visible");
    assert!(bottom < 20);
}

#[test]
fn test_env_picker_fits_input_and_esc_chip_across_the_tiers() {
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[ParamDecl::new("value")],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "command".into(),
        path: None,
        tokens: TokenContext {
            cwd: "/invoke".into(),
            home: Some("/home/demo".into()),
            env: BTreeMap::from([
                ("HOME".into(), "/home/demo".into()),
                ("PATH".into(), "/bin".into()),
            ]),
            today: "2026-08-12".into(),
            now: "17-00-00".into(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state.update(Action::OpenRunTokenMenuFor(0));
    state.update(Action::OpenRunEnvironmentPicker(0));

    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state, 70, 20);
    assert!(
        rows_with(terminal.backend().buffer(), "Environment variable").len() >= 2,
        "environment header rendered but picker popup is missing"
    );
    assert!(command_hit(&geometry, UiCommand::CloseModal).is_some());

    let (terminal, geometry) = draw(&mut session, &state, 70, 10);
    assert_eq!(
        bordered_control_height(terminal.backend().buffer(), "type to filter…"),
        3
    );
    assert!(
        command_hit(&geometry, UiCommand::CloseModal)
            .expect("Esc/Cancel remains visible")
            .bottom()
            <= 10
    );
}

#[test]
fn test_add_source_fields_stay_reachable_on_short_terminals() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &workflow, &mut session, Locale::En)
        })
        .unwrap();
    assert_eq!(
        session.focused(),
        Some(&AddControlId::Text(AddTextField::SourcePath))
    );
    let rendered = text(terminal.backend().buffer());
    assert!(
        rendered.contains("[Ctrl+O] Select"),
        "the short layout must keep Browse's independent keyboard path visible: {rendered}"
    );
    let browse = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::BrowseSource)
        .expect("the short layout keeps the Browse mouse target visible");
    assert!(browse.area.bottom() <= geometry.body.bottom());

    // Browse has its own key, so two field-navigation steps reach Name: path -> template -> name.
    for _ in 0..2 {
        if let Some(AddScreenEvent::Action(action)) =
            session.handle_event(key(KeyCode::Tab), &workflow, &geometry)
        {
            let _ = workflow.reduce(action);
        }
        terminal
            .draw(|frame| {
                geometry = render_add(frame, frame.area(), &workflow, &mut session, Locale::En)
            })
            .unwrap();
    }
    let name = AddControlId::Text(AddTextField::CommandName);
    assert_eq!(session.focused(), Some(&name));
    let area = geometry
        .hits
        .iter()
        .find_map(|hit| (hit.target == name).then_some(hit.area))
        .expect("focused name input is visible after focus scrolling");
    assert_eq!(area.height, 3);
    assert!(area.bottom() <= geometry.body.bottom());
}
