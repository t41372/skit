//! Ratatui frontend adapter for skit.

#![forbid(unsafe_code)]

mod terminal;

use ratatui_core::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui_widgets::{
    block::Block,
    borders::Borders,
    list::{List, ListItem, ListState},
    paragraph::{Paragraph, Wrap},
};
use skit_i18n::{Locale, render as localize, text};
use skit_ui::{Action, FormView, InputMode, LibraryState, ReportView, Screen};
use unicode_width::UnicodeWidthStr as _;

pub use terminal::{TuiError, collect_form, run};

/// A clickable footer command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitAction {
    /// Exit the frontend.
    Quit,
    /// Reload library data.
    Reload,
    /// Enter search mode.
    Search,
    /// Launch the selected entry.
    Run,
    /// Open the add-entry screen.
    Add,
    /// Open the selected source in an editor.
    Edit,
    /// Open settings for the selected entry.
    Settings,
    /// Open presets for the selected entry.
    Presets,
    /// Rename the selected entry.
    Rename,
    /// Ask before removal of the selected entry.
    Remove,
    /// Open application preferences.
    Preferences,
    /// Open the health report.
    Health,
    /// Open the prompt runner manager.
    Runners,
    /// Submit the active form or confirmation.
    Submit,
    /// Return to the library.
    Back,
    /// Focus one form row.
    Focus(usize),
}

/// One clickable rectangular target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HitRegion {
    /// Terminal cells occupied by the target.
    pub rect: Rect,
    /// Frontend-neutral intent represented by the target.
    pub action: HitAction,
}

/// Geometry emitted by rendering and consumed by mouse-event mapping.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ViewGeometry {
    /// Visible list-row cells, excluding borders.
    pub rows: Rect,
    /// Filtered-entry index represented by the first rendered row.
    pub first_visible: usize,
    /// Clickable footer chips.
    pub hits: Vec<HitRegion>,
}

/// Draw the library browser and return its mouse hit map.
#[must_use]
pub fn render(frame: &mut Frame, state: &LibraryState) -> ViewGeometry {
    render_localized(frame, state, Locale::En)
}

/// Draw the library browser with one explicit presentation locale.
#[must_use]
pub fn render_localized(frame: &mut Frame, state: &LibraryState, locale: Locale) -> ViewGeometry {
    let footer_height = if frame.area().width < 60 { 7 } else { 5 };
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(2),
            Constraint::Length(footer_height),
        ])
        .split(frame.area());

    render_header(frame, areas[0], state, locale);
    let mut geometry = match state.screen() {
        Screen::Library => render_library(frame, areas[1], state, locale),
        Screen::Form(form) => render_form(frame, areas[1], form, locale),
        Screen::Report(report) => render_report(frame, areas[1], report, locale),
        Screen::ConfirmRemove { name, .. } => render_confirmation(frame, areas[1], name, locale),
    };
    geometry
        .hits
        .extend(render_footer(frame, areas[2], state, locale));
    geometry
}

fn render_header(frame: &mut Frame, area: Rect, state: &LibraryState, locale: Locale) {
    let title = match state.screen() {
        Screen::Library => {
            let mode = if state.input_mode() == InputMode::Search {
                text(locale, "Search")
            } else {
                text(locale, "Library")
            };
            let cursor = if state.input_mode() == InputMode::Search {
                "▌"
            } else {
                ""
            };
            if state.query().is_empty() {
                format!("{mode}: {}{cursor}", text(locale, "all entries"))
            } else {
                format!("{mode}: {}{cursor}", state.query())
            }
        }
        Screen::Form(form) => localize(locale, &form.title),
        Screen::Report(report) => localize(locale, &report.title),
        Screen::ConfirmRemove { .. } => text(locale, "Confirm removal").to_owned(),
    };
    frame.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL).title(" skit ")),
        area,
    );
}

fn render_library(
    frame: &mut Frame,
    area: Rect,
    state: &LibraryState,
    locale: Locale,
) -> ViewGeometry {
    let panes = if area.width >= 80 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area)
    };

    let list_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", text(locale, "Entries")));
    let rows = list_block.inner(panes[0]);
    let items = state
        .visible_entries()
        .map(|entry| {
            ListItem::new(Line::from(vec![
                Span::raw(entry.name.as_str()),
                Span::raw("  "),
                Span::styled(
                    entry.kind.as_str(),
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    let mut list_state = ListState::default();
    list_state.select(state.selected_visible_index());
    let list = List::new(items)
        .block(list_block)
        .highlight_symbol("› ")
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(list, panes[0], &mut list_state);

    let detail = match state.selected() {
        Some(entry) => vec![
            Line::from(vec![Span::styled(
                entry.name.as_str(),
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from(format!("{}: {}", text(locale, "Slug"), entry.slug)),
            Line::from(format!("{}: {}", text(locale, "Kind"), entry.kind)),
            Line::from(
                format!("{}: {:?}", text(locale, "Storage mode"), entry.mode).to_lowercase(),
            ),
            Line::from(""),
            Line::from(entry.description.as_str()),
        ],
        None => vec![Line::from(text(locale, "No matching entries"))],
    };
    frame.render_widget(
        Paragraph::new(detail).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", text(locale, "Details"))),
        ),
        panes[1],
    );

    ViewGeometry {
        rows,
        first_visible: list_state.offset(),
        hits: Vec::new(),
    }
}

fn render_form(frame: &mut Frame, area: Rect, form: &FormView, locale: Locale) -> ViewGeometry {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", localize(locale, &form.title)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let row_height = 2_u16;
    let capacity = usize::from(inner.height / row_height).max(1);
    let first = form.focused.saturating_sub(capacity.saturating_sub(1));
    let mut hits = Vec::new();

    for (visible_index, (index, field)) in form
        .fields
        .iter()
        .enumerate()
        .skip(first)
        .take(capacity)
        .enumerate()
    {
        let y = inner.y.saturating_add(
            u16::try_from(visible_index)
                .unwrap_or(u16::MAX)
                .saturating_mul(row_height),
        );
        let area = Rect::new(
            inner.x,
            y,
            inner.width,
            row_height.min(inner.bottom().saturating_sub(y)),
        );
        let marker = if index == form.focused { "›" } else { " " };
        let value = if field.secret && !field.value.is_empty() {
            "•".repeat(field.value.chars().count())
        } else {
            field.value.replace('\n', " ↵ ")
        };
        let line = format!("{marker} {}: {value}", localize(locale, &field.label));
        frame.render_widget(
            Paragraph::new(line).style(if index == form.focused {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            }),
            area,
        );
        hits.push(HitRegion {
            rect: area,
            action: HitAction::Focus(index),
        });
    }
    ViewGeometry {
        rows: Rect::default(),
        first_visible: first,
        hits,
    }
}

fn render_report(
    frame: &mut Frame,
    area: Rect,
    report: &ReportView,
    locale: Locale,
) -> ViewGeometry {
    let lines = report
        .items
        .iter()
        .map(|item| {
            Line::from(format!(
                "[{}] {}: {}",
                localize(locale, &item.status),
                localize(locale, &item.label),
                localize(locale, &item.detail)
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL)),
        area,
    );
    ViewGeometry::default()
}

fn render_confirmation(frame: &mut Frame, area: Rect, name: &str, locale: Locale) -> ViewGeometry {
    frame.render_widget(
        Paragraph::new(format!("{} {name}?", text(locale, "Remove this entry:")))
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL)),
        area,
    );
    ViewGeometry::default()
}

fn render_footer(
    frame: &mut Frame,
    area: Rect,
    state: &LibraryState,
    locale: Locale,
) -> Vec<HitRegion> {
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let compact = inner.width < 58;
    let labels = footer_labels(state, locale, compact);
    let mut hits = Vec::new();
    let mut x = inner.x;
    let mut y = inner.y;
    for (label, action) in labels {
        let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
        if x > inner.x && x.saturating_add(width) > inner.right() {
            x = inner.x;
            y = y.saturating_add(1);
        }
        if y >= inner.bottom() {
            break;
        }
        let chip_width = width.min(inner.right().saturating_sub(x));
        let chip_area = Rect::new(x, y, chip_width, 1);
        frame.render_widget(
            Paragraph::new(label).style(Style::default().add_modifier(Modifier::BOLD)),
            chip_area,
        );
        hits.push(HitRegion {
            rect: chip_area,
            action,
        });
        x = x.saturating_add(width).saturating_add(2);
    }

    if let Some(status) = state.status() {
        let status_area = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
        frame.render_widget(Paragraph::new(localize(locale, status)), status_area);
    } else if matches!(state.screen(), Screen::Library)
        && !state.diagnostics().is_empty()
        && inner.width > 50
    {
        let note = format!(
            "{} {}",
            state.diagnostics().len(),
            text(locale, "damaged entries hidden")
        );
        let width = u16::try_from(note.chars().count()).unwrap_or(u16::MAX);
        let note_area = Rect::new(
            rect_right(inner).saturating_sub(width.min(inner.width)),
            inner.y,
            width.min(inner.width),
            1,
        );
        frame.render_widget(Paragraph::new(note), note_area);
    }
    hits
}

fn footer_labels(state: &LibraryState, locale: Locale, compact: bool) -> Vec<(String, HitAction)> {
    let chip = |wide: &str, short: &str, label: &str, action| {
        let key = if compact { short } else { wide };
        (format!("[{key}] {}", localize(locale, label)), action)
    };
    match state.screen() {
        Screen::Library => vec![
            chip("Enter", "↵", "Run", HitAction::Run),
            chip("Ctrl+N", "^N", "Add", HitAction::Add),
            chip("Ctrl+E", "^E", "Edit", HitAction::Edit),
            chip("s", "s", "Settings", HitAction::Settings),
            chip("p", "p", "Presets", HitAction::Presets),
            chip("F2", "F2", "Rename", HitAction::Rename),
            chip("Del", "⌫", "Remove", HitAction::Remove),
            chip(",", ",", "Preferences", HitAction::Preferences),
            chip("h", "h", "Health", HitAction::Health),
            chip("a", "a", "Runners", HitAction::Runners),
            chip("/", "/", "Search", HitAction::Search),
            chip("Ctrl+R", "^R", "Reload", HitAction::Reload),
            chip("q", "q", "Quit", HitAction::Quit),
        ],
        Screen::Form(form) => vec![
            chip("Esc", "Esc", "Back", HitAction::Back),
            chip(
                "Tab",
                "Tab",
                "Next field",
                HitAction::Focus(form.focused.saturating_add(1)),
            ),
            chip("Ctrl+S", "^S", &form.submit_label, HitAction::Submit),
        ],
        Screen::Report(_) => vec![
            chip("Esc", "Esc", "Back", HitAction::Back),
            chip("Ctrl+R", "^R", "Reload", HitAction::Reload),
        ],
        Screen::ConfirmRemove { .. } => vec![
            chip("Esc", "Esc", "Cancel", HitAction::Back),
            chip("Enter", "↵", "Remove", HitAction::Submit),
        ],
    }
}

/// Translate Crossterm input into frontend-neutral actions.
#[must_use]
pub fn map_event(event: Event, state: &LibraryState, geometry: &ViewGeometry) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => map_key(key, state),
        Event::Mouse(mouse) => map_mouse(mouse, state, geometry),
        Event::FocusGained
        | Event::FocusLost
        | Event::Key(_)
        | Event::Paste(_)
        | Event::Resize(_, _) => None,
    }
}

fn map_key(key: KeyEvent, state: &LibraryState) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    match state.screen() {
        Screen::Library if state.input_mode() == InputMode::Search => match key.code {
            KeyCode::Esc | KeyCode::Enter => Some(Action::FinishSearch),
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::ClearSearch)
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                Some(Action::Input(character))
            }
            _ => None,
        },
        Screen::Library => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::Reload)
            }
            KeyCode::Char('r') => Some(Action::Reload),
            KeyCode::Char('/') => Some(Action::BeginSearch),
            KeyCode::Enter => Some(Action::OpenRun),
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::OpenAdd)
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::Edit)
            }
            KeyCode::Char('s') => Some(Action::OpenSettings),
            KeyCode::Char('p') => Some(Action::OpenPresets),
            KeyCode::F(2) => Some(Action::OpenRename),
            KeyCode::Delete => Some(Action::AskRemove),
            KeyCode::Char(',') => Some(Action::OpenPreferences),
            KeyCode::Char('h') => Some(Action::OpenHealth),
            KeyCode::Char('a') => Some(Action::OpenRunners),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::Previous),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::Next),
            KeyCode::PageUp => Some(Action::PagePrevious),
            KeyCode::PageDown => Some(Action::PageNext),
            KeyCode::Home => Some(Action::Home),
            KeyCode::End => Some(Action::End),
            _ => None,
        },
        Screen::Form(form) => match key.code {
            KeyCode::Esc => Some(Action::Back),
            KeyCode::Tab => Some(Action::FocusNext),
            KeyCode::BackTab => Some(Action::FocusPrevious),
            KeyCode::Up => Some(Action::FocusPrevious),
            KeyCode::Down => Some(Action::FocusNext),
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::Submit)
            }
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Enter
                if form
                    .fields
                    .get(form.focused)
                    .is_some_and(|field| field.multiline) =>
            {
                Some(Action::Input('\n'))
            }
            KeyCode::Enter => Some(Action::FocusNext),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                Some(Action::Input(character))
            }
            _ => None,
        },
        Screen::Report(_) => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::Back),
            KeyCode::Char('r') => Some(Action::Reload),
            _ => None,
        },
        Screen::ConfirmRemove { .. } => match key.code {
            KeyCode::Enter | KeyCode::Char('y') => Some(Action::Submit),
            KeyCode::Esc | KeyCode::Char('n') => Some(Action::Back),
            _ => None,
        },
    }
}

fn map_mouse(mouse: MouseEvent, state: &LibraryState, geometry: &ViewGeometry) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::ScrollUp if matches!(state.screen(), Screen::Library) => {
            Some(Action::Previous)
        }
        MouseEventKind::ScrollDown if matches!(state.screen(), Screen::Library) => {
            Some(Action::Next)
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(hit) = geometry
                .hits
                .iter()
                .find(|hit| contains(hit.rect, mouse.column, mouse.row))
            {
                return Some(match hit.action {
                    HitAction::Quit => Action::Quit,
                    HitAction::Reload => Action::Reload,
                    HitAction::Search => Action::BeginSearch,
                    HitAction::Run => Action::OpenRun,
                    HitAction::Add => Action::OpenAdd,
                    HitAction::Edit => Action::Edit,
                    HitAction::Settings => Action::OpenSettings,
                    HitAction::Presets => Action::OpenPresets,
                    HitAction::Rename => Action::OpenRename,
                    HitAction::Remove => Action::AskRemove,
                    HitAction::Preferences => Action::OpenPreferences,
                    HitAction::Health => Action::OpenHealth,
                    HitAction::Runners => Action::OpenRunners,
                    HitAction::Submit => Action::Submit,
                    HitAction::Back => Action::Back,
                    HitAction::Focus(index) => Action::FocusField(index),
                });
            }
            matches!(state.screen(), Screen::Library)
                .then_some(())
                .filter(|()| contains(geometry.rows, mouse.column, mouse.row))
                .map(|()| {
                    Action::SelectVisible(
                        geometry.first_visible + usize::from(mouse.row - geometry.rows.y),
                    )
                })
        }
        MouseEventKind::Down(MouseButton::Right)
        | MouseEventKind::Down(MouseButton::Middle)
        | MouseEventKind::Up(_)
        | MouseEventKind::Drag(_)
        | MouseEventKind::Moved
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight
        | MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown => None,
    }
}

fn contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect_right(rect)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

const fn rect_right(rect: Rect) -> u16 {
    rect.x.saturating_add(rect.width)
}
