//! Searchable library browser and responsive detail pane.

use ratatui_core::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui_interact::{
    components::{
        ScrollableContentState, handle_scrollable_content_key, handle_scrollable_content_mouse,
    },
    state::FocusManager,
};
use ratatui_widgets::{
    paragraph::{Paragraph, Wrap},
    table::{Cell, Row, Table, TableState},
};
use skit_domain::{EntrySummary, StorageMode};
use skit_i18n::{Locale, format_text, kind_label, text};
use skit_ui::{
    Action, DetailPaneMode, LibraryEntryDetail, LibraryLastRun, LibraryPromptRunner, LibraryRunAge,
    LibraryState,
};

use crate::{
    ViewGeometry,
    pointer::contains,
    theme::{ACCENT, BOX_GREEN, BOX_INDIGO, SELECT_BG, SELECT_FG, padded_panel, panel_block},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum LibraryPane {
    List,
    Detail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LibraryClickTarget {
    Row(usize),
    Detail,
}

/// Result of one pointer event owned by the Library body.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LibraryPointerHandling {
    Action(Action),
    Consumed,
    Ignored,
}

/// Persistent focus and scroll state for the Library panes.
#[derive(Debug)]
pub(crate) struct LibraryScreenSession {
    detail_scroll: ScrollableContentState,
    detail_area: Rect,
    detail_hit_area: Rect,
    detail_height: usize,
    focus: FocusManager<LibraryPane>,
    detail_signature: Option<(Option<skit_domain::Slug>, u16)>,
}

impl Default for LibraryScreenSession {
    fn default() -> Self {
        let mut focus = FocusManager::new();
        focus.register_all([LibraryPane::List, LibraryPane::Detail]);
        Self {
            detail_scroll: ScrollableContentState::empty(),
            detail_area: Rect::default(),
            detail_hit_area: Rect::default(),
            detail_height: 0,
            focus,
            detail_signature: None,
        }
    }
}

impl LibraryScreenSession {
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        state: &LibraryState,
        locale: Locale,
    ) -> ViewGeometry {
        let narrow = crate::layout::is_narrow(area.width);
        let short = crate::layout::is_short(frame.area().height);
        let detail_requested = match state.detail_pane_mode() {
            DetailPaneMode::PinnedOpen => true,
            DetailPaneMode::PinnedClosed => false,
            DetailPaneMode::Automatic => !narrow || !short,
        };
        let panes = if !detail_requested {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(100), Constraint::Length(0)])
                .split(area)
        } else if !narrow || short {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Ratio(3, 5), Constraint::Ratio(2, 5)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Ratio(3, 5), Constraint::Ratio(2, 5)])
                .split(area)
        };
        let show_detail = detail_requested && !panes[1].is_empty();
        if !show_detail {
            self.focus.set(LibraryPane::List);
        }

        let list_block = panel_block(text(locale, "Library").into_owned(), BOX_GREEN);
        let table_inner = list_block.inner(panes[0]);
        let rows = Rect::new(
            table_inner.x,
            table_inner.y.saturating_add(1),
            table_inner.width,
            table_inner.height.saturating_sub(1),
        );
        let table_rows = state
            .visible_entries()
            .map(|entry| {
                let mut label = format!(
                    "{} {}",
                    kind_glyph(entry.kind.as_str()),
                    kind_label(locale, entry.kind.as_str())
                );
                if supports_modes(entry.kind.as_str()) && entry.mode == StorageMode::Reference {
                    label.push_str(" ↗");
                }
                let health = if state
                    .entry_detail(&entry.slug)
                    .is_some_and(|detail| detail.missing_target.is_some())
                {
                    "⚠"
                } else {
                    ""
                };
                Row::new(vec![
                    Cell::from(entry.name.as_str()),
                    Cell::from(label),
                    Cell::from(health),
                ])
            })
            .collect::<Vec<_>>();
        let header = Row::new(vec![
            Cell::from(text(locale, "Name")),
            Cell::from(text(locale, "Kind")),
            Cell::from(" "),
        ])
        .style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
        let mut table_state = TableState::default();
        table_state.select(state.selected_visible_index());
        let table = Table::new(
            table_rows,
            [
                Constraint::Percentage(57),
                Constraint::Percentage(38),
                Constraint::Length(2),
            ],
        )
        .block(list_block)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .fg(SELECT_FG)
                .bg(SELECT_BG)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_stateful_widget(table, panes[0], &mut table_state);

        let detail = detail_lines(state, locale);
        self.render_detail(frame, panes[1], detail, state, locale);

        ViewGeometry {
            rows,
            first_visible: table_state.offset(),
            hits: Vec::new(),
            detail_pane_visible: show_detail,
        }
    }

    /// Route one wheel event to the Library pane under the pointer.
    pub(crate) fn handle_wheel(
        &mut self,
        mouse: &MouseEvent,
        geometry: &ViewGeometry,
    ) -> LibraryPointerHandling {
        if contains(self.detail_area, mouse.column, mouse.row) {
            self.focus.set(LibraryPane::Detail);
            let _ = handle_scrollable_content_mouse(
                &mut self.detail_scroll,
                mouse,
                self.detail_area,
                self.detail_height,
            );
            return LibraryPointerHandling::Consumed;
        }
        if contains(geometry.rows, mouse.column, mouse.row) {
            self.focus.set(LibraryPane::List);
            return LibraryPointerHandling::Action(if mouse.kind == MouseEventKind::ScrollUp {
                Action::Previous
            } else {
                Action::Next
            });
        }
        LibraryPointerHandling::Ignored
    }

    /// Return the semantic Library target under one pointer coordinate.
    pub(crate) fn click_target(
        &self,
        mouse: &MouseEvent,
        geometry: &ViewGeometry,
    ) -> Option<LibraryClickTarget> {
        if contains(self.detail_hit_area, mouse.column, mouse.row) {
            Some(LibraryClickTarget::Detail)
        } else if contains(geometry.rows, mouse.column, mouse.row) {
            let index = geometry
                .first_visible
                .saturating_add(usize::from(mouse.row.saturating_sub(geometry.rows.y)));
            Some(LibraryClickTarget::Row(index))
        } else {
            None
        }
    }

    /// Activate a semantic Library target after the shared tracker matches its release.
    pub(crate) fn activate_click(
        &mut self,
        target: LibraryClickTarget,
        state: &LibraryState,
    ) -> Option<Action> {
        match target {
            LibraryClickTarget::Detail => {
                self.focus.set(LibraryPane::Detail);
                None
            }
            LibraryClickTarget::Row(index) => {
                self.focus.set(LibraryPane::List);
                Some(if state.selected_visible_index() == Some(index) {
                    Action::OpenRun
                } else {
                    Action::SelectVisible(index)
                })
            }
        }
    }

    fn render_detail(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        lines: Vec<Line<'static>>,
        state: &LibraryState,
        locale: Locale,
    ) {
        if area.is_empty() {
            self.detail_area = Rect::default();
            self.detail_hit_area = Rect::default();
            self.detail_height = 0;
            return;
        }
        let signature = (state.selected().map(|entry| entry.slug.clone()), area.width);
        if self.detail_signature.as_ref() != Some(&signature) {
            self.detail_scroll.scroll_to_top();
            self.detail_signature = Some(signature);
        }
        let base_block = padded_panel(text(locale, "Detail pane").into_owned(), BOX_INDIGO);
        let inner = base_block.inner(area);
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let line_count = paragraph.line_count(inner.width);
        self.detail_scroll
            .set_lines(vec![String::new(); line_count]);
        self.detail_height = usize::from(inner.height);
        self.detail_area = inner;
        self.detail_hit_area = area;
        let maximum = line_count.saturating_sub(self.detail_height);
        self.detail_scroll
            .set_scroll_offset(self.detail_scroll.scroll_offset().min(maximum));
        let indicator = match (
            self.detail_scroll.is_at_top(),
            self.detail_scroll.is_at_bottom(self.detail_height),
        ) {
            (true, true) => "",
            (true, false) => " ↓",
            (false, true) => " ↑",
            (false, false) => " ↑↓",
        };
        let block = padded_panel(
            format!("{}{}", text(locale, "Detail pane"), indicator),
            BOX_INDIGO,
        );
        frame.render_widget(block, area);
        frame.render_widget(
            paragraph.scroll((
                u16::try_from(self.detail_scroll.scroll_offset()).unwrap_or(u16::MAX),
                0,
            )),
            inner,
        );
    }

    /// Dispatch detail-pane keyboard scrolling through the mature scroll component.
    pub(crate) fn handle_event(&mut self, event: &Event) -> bool {
        match event {
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && self.focus.is_focused(&LibraryPane::Detail)
                    && key.modifiers == KeyModifiers::NONE
                    && is_scroll_key(key.code) =>
            {
                handle_scrollable_content_key(&mut self.detail_scroll, key, self.detail_height)
                    .is_some()
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Key(_)
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Resize(_, _) => false,
        }
    }
}

fn is_scroll_key(code: KeyCode) -> bool {
    [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
    ]
    .contains(&code)
}

pub(crate) fn detail_lines(state: &LibraryState, locale: Locale) -> Vec<Line<'static>> {
    let Some(entry) = state.selected() else {
        return if state.entry_count() == 0 {
            vec![
                Line::from(Span::styled(
                    text(locale, "Your entries will appear here."),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::default(),
                Line::from(text(locale, "Press a to add the first one,")),
                Line::from(text(locale, "or run: skit add <path> in a terminal.")),
            ]
        } else {
            Vec::new()
        };
    };
    let facts = state.entry_detail(&entry.slug);
    let mut lines = vec![
        Line::from(Span::styled(
            entry.name.clone(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "{} {}",
            kind_glyph(entry.kind.as_str()),
            kind_label(locale, entry.kind.as_str())
        )),
    ];
    append_storage_mode(&mut lines, entry, locale);
    if let Some(template) = facts.and_then(|facts| facts.template.as_deref()) {
        lines.push(Line::from(Span::styled(
            template.to_owned(),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    if let Some(runner) = facts.and_then(|facts| facts.prompt_runner.as_ref()) {
        let runner = match runner {
            LibraryPromptRunner::PickOnRunForm => {
                text(locale, "Runner picked on the run form").into_owned()
            }
            LibraryPromptRunner::Configured(name) => format_text(locale, "Runs with {}", &[name]),
            LibraryPromptRunner::Missing(name) => {
                format_text(locale, "{} (no longer configured)", &[name])
            }
        };
        lines.push(Line::from(Span::styled(
            format!("🤖{runner}"),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    lines.push(Line::default());
    lines.push(if entry.description.is_empty() {
        Line::from(Span::styled(
            text(locale, "(no description — add one in Entry settings)"),
            Style::default().add_modifier(Modifier::DIM),
        ))
    } else {
        Line::from(entry.description.clone())
    });
    lines.push(Line::default());
    if let Some(facts) = facts {
        append_state_lines(&mut lines, facts, locale);
    }
    lines
}

fn append_storage_mode(lines: &mut Vec<Line<'static>>, entry: &EntrySummary, locale: Locale) {
    if !supports_modes(entry.kind.as_str()) {
        return;
    }
    let mode = match entry.mode {
        StorageMode::Copy => format!(
            "✓ {}",
            text(
                locale,
                "The copy is kept by skit; your original file is never modified."
            )
        ),
        StorageMode::Reference => format!(
            "↗ {}",
            format_text(
                locale,
                "Linked to the original: {}",
                &[&entry.target.as_deref().unwrap_or_default()],
            )
        ),
    };
    lines.push(Line::from(Span::styled(
        mode,
        Style::default().add_modifier(Modifier::DIM),
    )));
}

fn append_state_lines(lines: &mut Vec<Line<'static>>, facts: &LibraryEntryDetail, locale: Locale) {
    if !facts.parameters.is_empty() {
        let mut shown = facts
            .parameters
            .iter()
            .take(6)
            .map(|field| {
                if field.secret {
                    format!("{}=•••🔒", field.key)
                } else if field.value.is_empty() {
                    field.key.clone()
                } else {
                    format!("{}={}", field.key, field.value)
                }
            })
            .collect::<Vec<_>>()
            .join("  ");
        if facts.parameters.len() > 6 {
            shown.push_str(" …");
        }
        lines.push(Line::from(format_text(locale, "Parameters  {}", &[&shown])));
    }
    if !facts.presets.is_empty() {
        let mut names = facts.presets.clone();
        names.sort();
        lines.push(Line::from(format_text(
            locale,
            "Presets  {}",
            &[&names.join(" · ")],
        )));
    }
    if !facts.dependencies.is_empty() {
        lines.push(Line::from(format_text(
            locale,
            "Depends on  {}",
            &[&facts.dependencies.join(", ")],
        )));
    }
    if let Some(last_run) = &facts.last_run {
        lines.push(last_run_line(last_run, locale));
    } else {
        lines.push(Line::from(Span::styled(
            text(locale, "Not run yet"),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    if let Some(path) = &facts.missing_target {
        lines.push(Line::from(Span::styled(
            format_text(locale, "⚠ missing: {}", &[path]),
            Style::default().fg(Color::Yellow),
        )));
    } else if facts.drifted {
        lines.push(Line::from(vec![
            Span::styled("⚠ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                text(
                    locale,
                    "The script changed — skit checks the form against it before every run.",
                ),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }
}

fn last_run_line(last_run: &LibraryLastRun, locale: Locale) -> Line<'static> {
    let when = match &last_run.age {
        LibraryRunAge::JustNow => text(locale, "just now").into_owned(),
        LibraryRunAge::Minutes(minutes) => format_text(locale, "{} min ago", &[minutes]),
        LibraryRunAge::Hours(hours) => format_text(locale, "{} h ago", &[hours]),
        LibraryRunAge::Days(days) => format_text(locale, "{} d ago", &[days]),
        LibraryRunAge::Raw(raw) => raw.clone(),
    };
    let (glyph, outcome, color) = match last_run.exit {
        Some(0) => ('✓', text(locale, "finished").into_owned(), Color::Green),
        Some(code) => (
            '✗',
            format_text(locale, "failed (code {})", &[&code]),
            Color::Yellow,
        ),
        None => (
            '✗',
            format_text(locale, "failed (code {})", &[&"None"]),
            Color::Yellow,
        ),
    };
    let styled_outcome = format!("{glyph} {outcome}");
    let rendered = format_text(locale, "Last run  {} · {}", &[&when, &styled_outcome]);
    let outcome_at = rendered
        .rfind(&styled_outcome)
        .expect("the formatted last-run line must retain its outcome argument");
    let outcome_end = outcome_at.saturating_add(styled_outcome.len());
    Line::from(vec![
        Span::raw(rendered[..outcome_at].to_owned()),
        Span::styled(styled_outcome, Style::default().fg(color)),
        Span::raw(rendered[outcome_end..].to_owned()),
    ])
}

fn kind_glyph(kind: &str) -> &'static str {
    match kind {
        "python" => "⬡",
        "shell" => "#",
        "fish" => "∿",
        "js" => "✦",
        "ts" => "✧",
        "powershell" => "»",
        "ruby" => "◆",
        "perl" => "◈",
        "lua" => "○",
        "r" => "◇",
        "exe" => "▶",
        "command" => "$",
        "prompt" => "✎",
        _ => "?",
    }
}

fn supports_modes(kind: &str) -> bool {
    matches!(
        kind,
        "python"
            | "shell"
            | "fish"
            | "js"
            | "ts"
            | "powershell"
            | "ruby"
            | "perl"
            | "lua"
            | "r"
            | "prompt"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_core::{backend::TestBackend, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::KeyEvent;
    use skit_application::LibraryScan;
    use skit_domain::{EntryKind, Slug};

    #[test]
    fn a_zero_area_detail_is_not_visible_and_releases_keyboard_focus() {
        let state = LibraryState::from_scan(LibraryScan {
            entries: vec![EntrySummary {
                slug: Slug::parse("entry").unwrap(),
                name: "Entry".to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                description: "Detail".to_owned(),
                target: None,
            }],
            diagnostics: Vec::new(),
        });
        let mut session = LibraryScreenSession::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &state, Locale::En);
            })
            .unwrap();
        assert_eq!(
            session.activate_click(LibraryClickTarget::Detail, &state),
            None
        );

        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = session.render(frame, Rect::new(0, 0, 100, 0), &state, Locale::En);
            })
            .unwrap();
        assert!(
            !session.handle_event(&Event::Key(KeyEvent::new(
                KeyCode::Down,
                KeyModifiers::NONE,
            ))),
            "a zero-area detail consumed Library-list navigation"
        );
        assert!(
            !geometry.detail_pane_visible,
            "a zero-area detail was advertised as visible"
        );
    }
}
