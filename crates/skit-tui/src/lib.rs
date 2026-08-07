//! Ratatui frontend adapter for skit.

#![forbid(unsafe_code)]

mod terminal;

use ratatui::{
    Frame,
    crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use skit_ui::{Action, InputMode, LibraryState};

pub use terminal::{TuiError, run};

/// A clickable footer command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitAction {
    /// Exit the frontend.
    Quit,
    /// Reload library data.
    Reload,
    /// Enter search mode.
    Search,
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
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(3),
        ])
        .split(frame.area());

    render_header(frame, areas[0], state);
    let mut geometry = render_body(frame, areas[1], state);
    geometry.hits = render_footer(frame, areas[2], state);
    geometry
}

fn render_header(frame: &mut Frame, area: Rect, state: &LibraryState) {
    let mode = match state.input_mode() {
        InputMode::Browse => "Library",
        InputMode::Search => "Search",
    };
    let cursor = (state.input_mode() == InputMode::Search)
        .then_some("▌")
        .unwrap_or_default();
    let text = if state.query().is_empty() {
        format!("{mode}: all entries{cursor}")
    } else {
        format!("{mode}: {}{cursor}", state.query())
    };
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" skit ")),
        area,
    );
}

fn render_body(frame: &mut Frame, area: Rect, state: &LibraryState) -> ViewGeometry {
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

    let list_block = Block::default().borders(Borders::ALL).title(" Entries ");
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
            Line::from(format!("slug: {}", entry.slug)),
            Line::from(format!("kind: {}", entry.kind)),
            Line::from(format!("mode: {:?}", entry.mode).to_lowercase()),
            Line::from(""),
            Line::from(entry.description.as_str()),
        ],
        None => vec![Line::from("No matching entries")],
    };
    frame.render_widget(
        Paragraph::new(detail)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(" Details ")),
        panes[1],
    );

    ViewGeometry {
        rows,
        first_visible: list_state.offset(),
        hits: Vec::new(),
    }
}

fn render_footer(frame: &mut Frame, area: Rect, state: &LibraryState) -> Vec<HitRegion> {
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let labels = [
        ("[q] Quit", HitAction::Quit),
        ("[r] Reload", HitAction::Reload),
        ("[/] Search", HitAction::Search),
    ];
    let mut spans = Vec::new();
    let mut hits = Vec::new();
    let mut x = inner.x;
    for (index, (label, action)) in labels.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
            x = x.saturating_add(2);
        }
        let width = u16::try_from(label.chars().count()).unwrap_or(u16::MAX);
        spans.push(Span::styled(
            label,
            Style::default().add_modifier(Modifier::BOLD),
        ));
        hits.push(HitRegion {
            rect: Rect::new(x, inner.y, width.min(rect_right(inner).saturating_sub(x)), 1),
            action,
        });
        x = x.saturating_add(width);
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);

    if let Some(status) = state.status() {
        frame.render_widget(Paragraph::new(status), inner);
    } else if !state.diagnostics().is_empty() && inner.width > 50 {
        let note = format!("{} damaged entries hidden", state.diagnostics().len());
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

/// Translate Crossterm input into frontend-neutral actions.
#[must_use]
pub fn map_event(
    event: Event,
    state: &LibraryState,
    geometry: &ViewGeometry,
) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            map_key(key, state.input_mode())
        }
        Event::Mouse(mouse) => map_mouse(mouse, geometry),
        Event::FocusGained
        | Event::FocusLost
        | Event::Key(_)
        | Event::Paste(_)
        | Event::Resize(_, _) => None,
    }
}

fn map_key(key: KeyEvent, mode: InputMode) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    match mode {
        InputMode::Browse => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
            KeyCode::Char('r') => Some(Action::Reload),
            KeyCode::Char('/') => Some(Action::BeginSearch),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::Previous),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::Next),
            KeyCode::PageUp => Some(Action::PagePrevious),
            KeyCode::PageDown => Some(Action::PageNext),
            KeyCode::Home => Some(Action::Home),
            KeyCode::End => Some(Action::End),
            _ => None,
        },
        InputMode::Search => match key.code {
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
    }
}

fn map_mouse(mouse: MouseEvent, geometry: &ViewGeometry) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::ScrollUp => Some(Action::Previous),
        MouseEventKind::ScrollDown => Some(Action::Next),
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
                });
            }
            contains(geometry.rows, mouse.column, mouse.row).then(|| {
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
        | MouseEventKind::ScrollRight => None,
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
