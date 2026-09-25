//! Searchable library browser and responsive detail pane.

use ratatui_core::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEvent};
use ratatui_interact::{
    components::{
        ScrollableContentState, handle_scrollable_content_key, handle_scrollable_content_mouse,
    },
    state::FocusManager,
};
use ratatui_widgets::{
    paragraph::{Paragraph, Wrap},
    table::{Cell, Row, Table},
};
use skit_domain::{EntrySummary, StorageMode};
use skit_i18n::{Locale, format_text, kind_label, text};
use skit_ui::{
    Action, DetailPaneMode, LibraryEntryDetail, LibraryLastRun, LibraryPromptRunner, LibraryRunAge,
    LibraryState,
};

use crate::{
    ViewGeometry,
    agent_review::{
        AgentReviewNode, AgentReviewSnapshotError, focus as snapshot_focus, node as snapshot_node,
        rect as snapshot_rect, scroll as snapshot_scroll, value as snapshot_value,
    },
    pointer::contains,
    session::alignment_snapshot,
    theme::{self, Panel, Status, padded_panel, panel_block},
    viewport::{AlignmentSignature, Viewport},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
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
    Consumed,
    Ignored,
}

/// Persistent focus and scroll state for the Library panes.
#[derive(Clone, Debug)]
pub(crate) struct LibraryScreenSession {
    detail_scroll: ScrollableContentState,
    detail_area: Rect,
    detail_hit_area: Rect,
    detail_height: usize,
    focus: FocusManager<LibraryPane>,
    detail_signature: Option<(Option<skit_domain::Slug>, u16)>,
    list_scroll: ScrollableContentState,
    list_alignment: Option<AlignmentSignature<Option<usize>, usize>>,
    pending_follow: bool,
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
            list_scroll: ScrollableContentState::empty(),
            list_alignment: None,
            pending_follow: false,
        }
    }
}

impl LibraryScreenSession {
    #[cfg(test)]
    pub(crate) fn perturb_agent_review_state(&mut self) {
        self.detail_height = self.detail_height.saturating_add(1);
    }

    pub(crate) fn agent_review_snapshot(
        &self,
    ) -> Result<AgentReviewNode, AgentReviewSnapshotError> {
        let Self {
            detail_scroll,
            detail_area,
            detail_hit_area,
            detail_height,
            focus,
            detail_signature,
            list_scroll,
            list_alignment,
            pending_follow,
        } = self;
        let list_alignment = list_alignment
            .as_ref()
            .map(|alignment| alignment_snapshot("library.list_alignment", alignment))
            .transpose()?;
        Ok(snapshot_node(
            "library",
            [
                ("detail_scroll", snapshot_scroll(detail_scroll)),
                ("detail_area", snapshot_rect(*detail_area)),
                ("detail_hit_area", snapshot_rect(*detail_hit_area)),
                ("detail_height", serde_json::json!(detail_height)),
                ("focus", snapshot_focus("library.focus", focus)?),
                (
                    "detail_signature",
                    snapshot_value("library.detail_signature", detail_signature)?,
                ),
                ("list_scroll", snapshot_scroll(list_scroll)),
                ("list_alignment", serde_json::json!(list_alignment)),
                ("pending_follow", serde_json::json!(pending_follow)),
            ],
        ))
    }

    /// Ask the next render to put the selected entry back in the viewport.
    ///
    /// The reducer clamps the selection at both ends of the list. `Previous` at the first entry
    /// and `Next` at the last one keep the same index, so the index alone cannot report that the
    /// reader asked to see the selection. This flag records the request itself.
    pub(crate) const fn follow_selection(&mut self) {
        self.pending_follow = true;
    }

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

        let list_block = panel_block(text(locale, "Library").into_owned(), Panel::Library);
        let table_inner = list_block.inner(panes[0]);
        let rows = Rect::new(
            table_inner.x,
            table_inner.y.saturating_add(1),
            table_inner.width,
            table_inner.height.saturating_sub(1),
        );
        let offset = self.align_list(state, rows);
        let selected = state.selected_visible_index();
        let table_rows = state
            .visible_entries()
            .enumerate()
            .skip(offset)
            .take(usize::from(rows.height))
            .map(|(index, entry)| {
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
                let row = Row::new(vec![
                    Cell::from(entry.name.as_str()),
                    Cell::from(label),
                    Cell::from(health),
                ]);
                if selected == Some(index) {
                    row.style(theme::selection().add_modifier(Modifier::BOLD))
                } else {
                    row
                }
            })
            .collect::<Vec<_>>();
        let header = Row::new(vec![
            Cell::from(text(locale, "Name")),
            Cell::from(text(locale, "Kind")),
            Cell::from(" "),
        ])
        .style(theme::table_header());
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
        .column_spacing(1);
        frame.render_widget(table, panes[0]);

        let detail = detail_lines(state, locale);
        self.render_detail(frame, panes[1], detail, state, locale);

        ViewGeometry {
            rows,
            first_visible: offset,
            hits: Vec::new(),
            detail_pane_visible: show_detail,
        }
    }

    /// Clamp the list offset and follow the selection only on new keyboard or layout intent.
    ///
    /// The Library list owns its offset because a wheel notch must move the viewport and leave the
    /// selection alone. A stateful `Table` cannot do that: `Table::visible_rows` pulls the selected
    /// row back into view on every draw.
    ///
    /// Two inputs decide when to realign. The signature reports a change when the selection moved,
    /// when the viewport was resized, and when the entry count changed under a selection that did
    /// not move. `pending_follow` reports a navigation key that the reducer clamped, which leaves
    /// the index alone. Neither is true on the render right after a wheel notch, so the reader
    /// keeps the viewport where the wheel put it. The test is deliberately not "did the offset
    /// change": that answer is true on the render right after a scroll and false on the one after
    /// that, so the viewport would spring back one frame later.
    ///
    /// A realignment moves the offset as little as it can. The selected row is visible for every
    /// offset from `selected + 1 - visible` through `selected`, so the realignment clamps the
    /// offset into that band. Under the band the selection is below the viewport. Over it the
    /// selection is above. A viewport with no room for a row has no band, so it keeps the offset.
    fn align_list(&mut self, state: &LibraryState, rows: Rect) -> usize {
        let total = state.visible_entry_count();
        let visible = usize::from(rows.height);
        self.list_scroll.set_lines(vec![String::new(); total]);
        Viewport::new(rows, total).clamp_scroll(&mut self.list_scroll);
        let selected = state.selected_visible_index();
        let moved = AlignmentSignature::update(&mut self.list_alignment, selected, rows, total);
        let follow = std::mem::take(&mut self.pending_follow);
        if let Some(selected) = selected
            && (moved || follow)
            && let Some(lowest) = visible
                .checked_sub(1)
                .map(|last| selected.saturating_sub(last))
        {
            let offset = self.list_scroll.scroll_offset();
            self.list_scroll
                .set_scroll_offset(offset.clamp(lowest, selected));
        }
        self.list_scroll.scroll_offset()
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
            let _ = handle_scrollable_content_mouse(
                &mut self.list_scroll,
                mouse,
                geometry.rows,
                usize::from(geometry.rows.height),
            );
            return LibraryPointerHandling::Consumed;
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
        let base_block = padded_panel(text(locale, "Detail pane").into_owned(), Panel::Detail);
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
            Panel::Detail,
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
        Line::from(Span::styled(entry.name.clone(), theme::emphasis())),
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
            theme::muted(),
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
            theme::muted(),
        )));
    }
    lines.push(Line::default());
    lines.push(if entry.description.is_empty() {
        Line::from(Span::styled(
            text(locale, "(no description — add one in Entry settings)"),
            theme::muted(),
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
    lines.push(Line::from(Span::styled(mode, theme::muted())));
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
            theme::muted(),
        )));
    }
    if let Some(path) = &facts.missing_target {
        lines.push(Line::from(theme::status_spans(
            format_text(locale, "⚠ missing: {}", &[path]),
            Status::Warning,
        )));
    } else if facts.drifted {
        lines.push(Line::from(vec![
            Span::styled("⚠ ", theme::status_glyph(Status::Warning)),
            Span::styled(
                text(
                    locale,
                    "The script changed — skit checks the form against it before every run.",
                ),
                theme::status(Status::Warning),
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
    let (glyph, outcome, status) = match last_run.exit {
        Some(0) => ('✓', text(locale, "finished").into_owned(), Status::Success),
        Some(code) => (
            '✗',
            format_text(locale, "failed (code {})", &[&code]),
            Status::Warning,
        ),
        None => (
            '✗',
            format_text(locale, "failed (code {})", &[&"None"]),
            Status::Warning,
        ),
    };
    let styled_outcome = format!("{glyph} {outcome}");
    let rendered = format_text(locale, "Last run  {} · {}", &[&when, &styled_outcome]);
    let outcome_at = rendered
        .rfind(&styled_outcome)
        .expect("the formatted last-run line must retain its outcome argument");
    let outcome_end = outcome_at.saturating_add(styled_outcome.len());
    let mut spans = vec![Span::raw(rendered[..outcome_at].to_owned())];
    spans.extend(theme::status_spans(styled_outcome, status));
    spans.push(Span::raw(rendered[outcome_end..].to_owned()));
    Line::from(spans)
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

    #[test]
    fn library_snapshot_keeps_focus_scroll_area_and_signature() {
        let mut session = LibraryScreenSession::default();
        session
            .detail_scroll
            .set_lines(vec![String::new(), String::new()]);
        session.detail_scroll.set_scroll_offset(1);
        session.detail_area = Rect::new(1, 2, 3, 4);
        session.detail_hit_area = Rect::new(2, 3, 1, 1);
        session.detail_height = 4;
        session.focus.set(LibraryPane::Detail);
        session.detail_signature = Some((None, 80));
        session
            .list_scroll
            .set_lines(vec![String::new(), String::new(), String::new()]);
        session.list_scroll.set_scroll_offset(2);
        let json = serde_json::to_string(&session.agent_review_snapshot().unwrap()).unwrap();
        assert!(json.contains("detail_signature"));
        assert!(json.contains("detail_hit_area"));
        assert!(json.contains("Detail"));
        assert!(json.contains("80"));
        assert!(json.contains("list_scroll"), "{json}");
        assert!(
            json.contains("\"list_alignment\":null"),
            "an unrendered Library has no list alignment yet: {json}"
        );
        assert!(
            json.contains("\"pending_follow\":false"),
            "an unrendered Library has no follow request yet: {json}"
        );

        session.follow_selection();
        let json = serde_json::to_string(&session.agent_review_snapshot().unwrap()).unwrap();
        assert!(json.contains("\"pending_follow\":true"), "{json}");

        assert!(AlignmentSignature::update(
            &mut session.list_alignment,
            Some(7),
            Rect::new(0, 0, 44, 5),
            13,
        ));
        let json = serde_json::to_string(&session.agent_review_snapshot().unwrap()).unwrap();
        assert!(json.contains("\"focus\":7"), "{json}");
        assert!(json.contains("\"viewport_height\":5"), "{json}");
        assert!(json.contains("\"reflow\":13"), "{json}");
    }
}
