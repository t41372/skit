//! Health and prompt-runner management widgets.

use ratatui_core::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::Line,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui_interact::{
    components::{
        ListPicker, ListPickerState, ListPickerStyle, ScrollableContentState,
        handle_scrollable_content_key, handle_scrollable_content_mouse,
    },
    state::FocusManager,
    traits::ClickRegionRegistry,
};
use ratatui_widgets::{clear::Clear, paragraph::Paragraph, paragraph::Wrap};
use skit_application::runner_management::{EditableArgvDialect, join_editable_argv};
use skit_i18n::{Locale, format_text, text};
use skit_ui::{
    HealthAction, HealthIssue, HealthIssueKind, HealthView, MirrorHealth, RunnerEditorAction,
    RunnerEditorError, RunnerEditorField, RunnerEditorMode, RunnerEditorView, RunnerManagerAction,
    RunnerManagerView, RunnerRemovalView, RunnerRow, UvHealth,
};
use tui_input::{Input as LineInput, InputRequest, backend::crossterm::EventHandler as _};

use crate::{
    footer::{
        ActionFooterItem, ActionFooterMouse, ActionFooterSession, ActionFooterStyle,
        action_footer_required_height,
    },
    session::render_line_input,
    theme::{ACCENT, BOX_DIM, BOX_GREEN, BOX_MAROON, padded_panel},
};

const BOX_OLIVE: Color = Color::Rgb(0x76, 0x75, 0x32);

/// Result of one Health terminal event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HealthEventHandling {
    /// Dispatch through the pure Health reducer.
    Action(HealthAction),
    /// Ephemeral scroll state changed.
    Consumed,
    /// The screen did not accept the event.
    Ignored,
}

#[derive(Clone, Debug)]
enum HealthHit {
    Issue(usize),
}

/// Mature list and scroll state for the typed Health screen.
#[derive(Debug, Default)]
pub(crate) struct HealthScreenSession {
    issues: ListPickerState,
    summary_scroll: ScrollableContentState,
    summary_area: Rect,
    summary_height: usize,
    issue_height: usize,
    clicks: ClickRegionRegistry<HealthHit>,
    issue_areas: Vec<(usize, Rect)>,
    footer: ActionFooterSession<HealthAction>,
}

impl HealthScreenSession {
    /// Render the complete actionable Health report.
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &HealthView,
        locale: Locale,
    ) {
        self.clicks.clear();
        self.issue_areas.clear();
        let block = padded_panel(text(locale, "Health check").into_owned(), BOX_GREEN);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let has_issues = !view.snapshot().issues.is_empty();
        let footer_items = health_footer_items(locale);
        let footer_height = action_footer_required_height(inner.width, &footer_items)
            .min(3)
            .min(inner.height);
        let [summary, issue_heading, issue_list, footer] = Layout::vertical([
            Constraint::Length(if has_issues {
                7.min(inner.height)
            } else {
                inner.height.saturating_sub(footer_height)
            }),
            Constraint::Length(u16::from(has_issues)),
            Constraint::Min(u16::from(has_issues)),
            Constraint::Length(footer_height),
        ])
        .areas(inner);
        self.render_summary(frame, summary, view, locale);
        if has_issues {
            frame.render_widget(
                Paragraph::new(text(locale, "Issues (Enter jumps to the entry):")).style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                issue_heading,
            );
            self.render_issues(frame, issue_list, view, locale);
        } else {
            self.issue_height = 0;
        }
        self.footer.render(
            frame,
            footer,
            &footer_items,
            ActionFooterStyle::new(Color::White, BOX_DIM),
        );
    }

    fn render_summary(&mut self, frame: &mut Frame, area: Rect, view: &HealthView, locale: Locale) {
        self.summary_area = area;
        self.summary_height = usize::from(area.height);
        let snapshot = view.snapshot();
        let mut lines = Vec::new();
        match &snapshot.uv {
            UvHealth::Found(path) => lines.push(Line::styled(
                format!("✓ {}", format_text(locale, "uv: {}", &[path])),
                Style::default().fg(Color::Green),
            )),
            UvHealth::NotRequired => lines.push(Line::styled(
                format!("✓ {}", text(locale, "uv: not required")),
                Style::default().fg(Color::Green),
            )),
            UvHealth::Missing => lines.push(Line::styled(
                format!(
                    "✗ {}",
                    text(locale, "uv: not found. Install it from https://docs.astral.sh/uv/getting-started/installation/")
                ),
                Style::default().fg(Color::Red),
            )),
        }
        let count_message = if snapshot.entry_count == 1 {
            "{} entry registered"
        } else {
            "{} entries registered"
        };
        lines.push(Line::styled(
            format!(
                "✓ {}",
                format_text(locale, count_message, &[&snapshot.entry_count])
            ),
            Style::default().fg(Color::Green),
        ));
        if !snapshot.invalid_runner_rows.is_empty() {
            lines.push(Line::styled(
                format!(
                    "⚠ {}",
                    format_text(
                        locale,
                        "Malformed agent (runner) rows in config: {} — fix them in Preferences → Manage agents",
                        &[&snapshot.invalid_runner_rows.join(", ")],
                    )
                ),
                Style::default().fg(Color::Yellow),
            ));
        }
        let mirror = match &snapshot.mirror {
            MirrorHealth::Off => text(locale, "Mirrors: off").into_owned(),
            MirrorHealth::On { axes } => format_text(locale, "Mirrors: {}", &[axes]),
            MirrorHealth::Paused { axes } => {
                format_text(locale, "Mirrors: off (saved: {})", &[axes])
            }
        };
        lines.push(Line::styled(
            format!("✓ {mirror}"),
            Style::default().fg(Color::Green),
        ));
        lines.push(Line::styled(
            format_text(
                locale,
                "Library: {} ({} · {})",
                &[
                    &snapshot.library_path,
                    &snapshot.entry_count,
                    &snapshot.library_size,
                ],
            ),
            Style::default().fg(Color::DarkGray),
        ));
        if let Some(outcome) = view.rebuilt() {
            let template = if outcome.entry_count == 1 {
                "Index rebuilt: {} entry"
            } else {
                "Index rebuilt: {} entries"
            };
            lines.push(Line::styled(
                format_text(locale, template, &[&outcome.entry_count]),
                Style::default().fg(Color::Green),
            ));
            lines.extend(
                outcome
                    .problems
                    .iter()
                    .cloned()
                    .map(|problem| Line::styled(problem, Style::default().fg(Color::Yellow))),
            );
        }
        lines.extend(
            snapshot
                .diagnostics
                .iter()
                .cloned()
                .map(|problem| Line::styled(problem, Style::default().fg(Color::Yellow))),
        );
        self.summary_scroll
            .set_lines(vec![String::new(); lines.len()]);
        let maximum = lines.len().saturating_sub(self.summary_height);
        if self.summary_scroll.scroll_offset() > maximum {
            self.summary_scroll.set_scroll_offset(maximum);
        }
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((
                u16::try_from(self.summary_scroll.scroll_offset()).unwrap_or(u16::MAX),
                0,
            )),
            area,
        );
    }

    fn render_issues(&mut self, frame: &mut Frame, area: Rect, view: &HealthView, locale: Locale) {
        let issues = &view.snapshot().issues;
        self.issues.set_total(issues.len());
        if let Some(selected) = view.selected_issue() {
            self.issues.select(selected);
        }
        self.issue_height = usize::from(area.height);
        self.issues.ensure_visible(self.issue_height.max(1));
        let labels = issues
            .iter()
            .map(|issue| health_issue_label(issue, locale))
            .collect::<Vec<_>>();
        frame.render_widget(
            ListPicker::new(&labels, &self.issues).style(list_style(ACCENT)),
            area,
        );
        for visible in 0..self.issue_height {
            let index = usize::from(self.issues.scroll).saturating_add(visible);
            if index >= issues.len() {
                break;
            }
            let row = Rect::new(
                area.x,
                area.y
                    .saturating_add(u16::try_from(visible).unwrap_or(u16::MAX)),
                area.width,
                1,
            );
            self.issue_areas.push((index, row));
            self.clicks.register(row, HealthHit::Issue(index));
        }
    }

    /// Dispatch keyboard and mouse through mature list/scroll state.
    pub(crate) fn handle_event(&mut self, event: Event, view: &HealthView) -> HealthEventHandling {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return HealthEventHandling::Action(HealthAction::Rebuild);
                }
                let has_issues = !view.snapshot().issues.is_empty();
                let action = match key.code {
                    KeyCode::Esc => Some(HealthAction::Back),
                    KeyCode::Enter => Some(HealthAction::Jump),
                    KeyCode::Up if has_issues => Some(HealthAction::Previous),
                    KeyCode::Down if has_issues => Some(HealthAction::Next),
                    KeyCode::PageUp if has_issues => {
                        Some(HealthAction::PagePrevious(self.issue_height.max(1)))
                    }
                    KeyCode::PageDown if has_issues => {
                        Some(HealthAction::PageNext(self.issue_height.max(1)))
                    }
                    KeyCode::Home if has_issues => Some(HealthAction::Home),
                    KeyCode::End if has_issues => Some(HealthAction::End),
                    _ => None,
                };
                if let Some(action) = action {
                    return HealthEventHandling::Action(action);
                }
                if handle_scrollable_content_key(
                    &mut self.summary_scroll,
                    &key,
                    self.summary_height,
                )
                .is_some()
                {
                    HealthEventHandling::Consumed
                } else {
                    HealthEventHandling::Ignored
                }
            }
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                if let ActionFooterMouse::Action(action) = self.footer.handle_mouse(&mouse) {
                    return HealthEventHandling::Action(action);
                }
                match self.clicks.handle_click(mouse.column, mouse.row).cloned() {
                    Some(HealthHit::Issue(index)) => {
                        HealthEventHandling::Action(HealthAction::ActivateIssue(index))
                    }
                    None => HealthEventHandling::Ignored,
                }
            }
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) =>
            {
                if matches!(
                    self.footer.handle_mouse(&mouse),
                    ActionFooterMouse::Scrolled
                ) {
                    HealthEventHandling::Consumed
                } else if self
                    .issue_areas
                    .iter()
                    .any(|(_, area)| area.contains((mouse.column, mouse.row).into()))
                {
                    let action = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                        HealthAction::Previous
                    } else {
                        HealthAction::Next
                    };
                    HealthEventHandling::Action(action)
                } else if handle_scrollable_content_mouse(
                    &mut self.summary_scroll,
                    &mouse,
                    self.summary_area,
                    self.summary_height,
                )
                .is_some()
                {
                    HealthEventHandling::Consumed
                } else {
                    HealthEventHandling::Ignored
                }
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Key(_)
            | Event::Resize(_, _) => {
                let _ = view;
                HealthEventHandling::Ignored
            }
        }
    }
}

/// Result of one reusable runner-editor event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RunnerEditorEventHandling {
    /// Dispatch through the pure editor reducer.
    Action(RunnerEditorAction),
    /// Ephemeral cursor state changed.
    Consumed,
    /// The editor did not accept the event.
    Ignored,
}

#[derive(Clone, Debug)]
enum RunnerEditorHit {
    Field(RunnerEditorField),
}

/// Reusable mature input session for new, edit, and raw-repair runner flows.
#[derive(Debug, Default)]
pub(crate) struct RunnerEditorSession {
    name: LineInput,
    command: LineInput,
    focus: FocusManager<RunnerEditorField>,
    signature: Option<(String, String, bool)>,
    clicks: ClickRegionRegistry<RunnerEditorHit>,
    footer: ActionFooterSession<RunnerEditorAction>,
}

impl RunnerEditorSession {
    /// Render the shared editor as a modal overlay.
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &RunnerEditorView,
        locale: Locale,
    ) {
        self.sync(view);
        self.clicks.clear();
        let footer_items = runner_editor_footer_items(locale);
        let expected_inner_width = area.width.min(72).saturating_sub(4);
        let footer_height = action_footer_required_height(expected_inner_width, &footer_items)
            .clamp(2, 4)
            .min(area.height);
        let panel = centered(area, 72, 12_u16.saturating_add(footer_height));
        frame.render_widget(Clear, panel);
        let title = match view.mode() {
            RunnerEditorMode::New => text(locale, "New agent (runner)"),
            RunnerEditorMode::Edit | RunnerEditorMode::Repair => {
                text(locale, "Edit agent (runner)")
            }
        };
        let block = padded_panel(title.into_owned(), ACCENT);
        let inner = block.inner(panel);
        frame.render_widget(block, panel);
        let [name, command, hint, error, actions] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(footer_height.min(inner.height)),
        ])
        .areas(inner);
        let name_label = if view.name_is_locked() {
            format!("🔒 {}", text(locale, "Name"))
        } else {
            text(locale, "Name, e.g. aider").into_owned()
        };
        render_line_input(
            frame,
            name,
            &self.name,
            false,
            view.focused() == RunnerEditorField::Name,
            &name_label,
        );
        render_line_input(
            frame,
            command,
            &self.command,
            false,
            view.focused() == RunnerEditorField::Command,
            &text(locale, "Command, e.g. aider --message {{prompt}}"),
        );
        if !view.name_is_locked() {
            self.clicks
                .register(name, RunnerEditorHit::Field(RunnerEditorField::Name));
        }
        self.clicks
            .register(command, RunnerEditorHit::Field(RunnerEditorField::Command));
        frame.render_widget(
            Paragraph::new(text(locale, "{{prompt}} marks where the prompt text goes. Each word becomes one argument — quotes group words, and no shell is involved."))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::DarkGray)),
            hint,
        );
        if let Some(message) = view
            .host_error()
            .map(str::to_owned)
            .or_else(|| view.error().map(|error| runner_editor_error(error, locale)))
        {
            frame.render_widget(
                Paragraph::new(message)
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(Color::Red)),
                error,
            );
        }
        self.footer.render(
            frame,
            actions,
            &footer_items,
            ActionFooterStyle::new(Color::White, BOX_DIM),
        );
    }

    /// Dispatch one event through the focused mature input or visible buttons.
    pub(crate) fn handle_event(
        &mut self,
        event: Event,
        view: &RunnerEditorView,
    ) -> RunnerEditorEventHandling {
        self.sync(view);
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                match key.code {
                    KeyCode::Esc => {
                        return RunnerEditorEventHandling::Action(RunnerEditorAction::Cancel);
                    }
                    KeyCode::Enter => {
                        return RunnerEditorEventHandling::Action(RunnerEditorAction::Submit);
                    }
                    KeyCode::Tab | KeyCode::Down => {
                        return RunnerEditorEventHandling::Action(RunnerEditorAction::FocusNext);
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        return RunnerEditorEventHandling::Action(
                            RunnerEditorAction::FocusPrevious,
                        );
                    }
                    _ => {}
                }
                let input = match view.focused() {
                    RunnerEditorField::Name => &mut self.name,
                    RunnerEditorField::Command => &mut self.command,
                };
                let before = input.value().to_owned();
                if input.handle_event(&Event::Key(key)).is_none() {
                    return RunnerEditorEventHandling::Ignored;
                }
                if before == input.value() {
                    RunnerEditorEventHandling::Consumed
                } else {
                    RunnerEditorEventHandling::Action(match view.focused() {
                        RunnerEditorField::Name => {
                            RunnerEditorAction::SetName(input.value().to_owned())
                        }
                        RunnerEditorField::Command => {
                            RunnerEditorAction::SetCommand(input.value().to_owned())
                        }
                    })
                }
            }
            Event::Paste(value) => {
                let input = match view.focused() {
                    RunnerEditorField::Name => &mut self.name,
                    RunnerEditorField::Command => &mut self.command,
                };
                for character in value.chars() {
                    let _ = input.handle(InputRequest::InsertChar(character));
                }
                RunnerEditorEventHandling::Action(match view.focused() {
                    RunnerEditorField::Name => {
                        RunnerEditorAction::SetName(input.value().to_owned())
                    }
                    RunnerEditorField::Command => {
                        RunnerEditorAction::SetCommand(input.value().to_owned())
                    }
                })
            }
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                if let ActionFooterMouse::Action(action) = self.footer.handle_mouse(&mouse) {
                    return RunnerEditorEventHandling::Action(action);
                }
                match self.clicks.handle_click(mouse.column, mouse.row).cloned() {
                    Some(RunnerEditorHit::Field(field)) => {
                        RunnerEditorEventHandling::Action(RunnerEditorAction::Focus(field))
                    }
                    None => RunnerEditorEventHandling::Ignored,
                }
            }
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) =>
            {
                if matches!(
                    self.footer.handle_mouse(&mouse),
                    ActionFooterMouse::Scrolled
                ) {
                    RunnerEditorEventHandling::Consumed
                } else {
                    RunnerEditorEventHandling::Ignored
                }
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Key(_)
            | Event::Resize(_, _) => RunnerEditorEventHandling::Ignored,
        }
    }

    fn sync(&mut self, view: &RunnerEditorView) {
        let signature = (
            view.name().to_owned(),
            view.command().to_owned(),
            view.name_is_locked(),
        );
        if self.signature.as_ref() != Some(&signature) {
            self.name = LineInput::new(view.name().to_owned());
            self.command = LineInput::new(view.command().to_owned());
            self.signature = Some(signature);
        }
        self.focus.clear();
        if !view.name_is_locked() {
            self.focus.register(RunnerEditorField::Name);
        }
        self.focus.register(RunnerEditorField::Command);
        self.focus.set(view.focused());
    }
}

/// Result of one runner-management terminal event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RunnerManagerEventHandling {
    /// Dispatch through the pure runner manager reducer.
    Action(RunnerManagerAction),
    /// Ephemeral widget state changed.
    Consumed,
    /// The manager did not accept the event.
    Ignored,
}

#[derive(Clone, Debug)]
enum RunnerHit {
    Row(usize),
}

/// Mature list, buttons, and shared editor for complete runner management.
#[derive(Debug, Default)]
pub(crate) struct RunnerManagerSession {
    rows: ListPickerState,
    row_height: usize,
    clicks: ClickRegionRegistry<RunnerHit>,
    row_areas: Vec<(usize, Rect)>,
    editor: RunnerEditorSession,
    footer: ActionFooterSession<RunnerManagerAction>,
}

impl RunnerManagerSession {
    /// Render the registry and its active typed overlay.
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &RunnerManagerView,
        locale: Locale,
    ) {
        self.clicks.clear();
        self.row_areas.clear();
        let block = padded_panel(
            text(locale, "Agents (prompt runners)").into_owned(),
            BOX_OLIVE,
        );
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let footer_items = runner_manager_footer_items(locale);
        let footer_height = action_footer_required_height(inner.width, &footer_items)
            .clamp(1, 2)
            .min(inner.height);
        let [hint, rows, status, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(u16::from(view.status().is_some())),
            Constraint::Length(footer_height),
        ])
        .areas(inner);
        frame.render_widget(
            Paragraph::new(text(
                locale,
                "The agents prompt entries run with. Pick one to edit or remove it.",
            ))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::DarkGray)),
            hint,
        );
        self.render_rows(frame, rows, view, locale);
        if let Some(message) = view.status() {
            frame.render_widget(
                Paragraph::new(message).style(Style::default().fg(Color::Yellow)),
                status,
            );
        }
        self.footer.render(
            frame,
            footer,
            &footer_items,
            ActionFooterStyle::new(Color::White, BOX_DIM),
        );

        if let Some(editor) = view.editor() {
            self.editor.render(frame, area, editor, locale);
        } else if let Some(removal) = view.removal() {
            self.render_removal(frame, area, removal, locale);
        } else if let Some(index) = view.action_row()
            && let Some(row) = view.rows().get(index)
        {
            self.render_actions(frame, area, row, locale);
        }
    }

    fn render_rows(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &RunnerManagerView,
        locale: Locale,
    ) {
        self.rows.set_total(view.rows().len());
        if let Some(selected) = view.selected() {
            self.rows.select(selected);
        }
        self.row_height = usize::from(area.height);
        self.rows.ensure_visible(self.row_height.max(1));
        if view.rows().is_empty() {
            frame.render_widget(
                Paragraph::new(text(locale, "No agents configured yet."))
                    .style(Style::default().fg(Color::DarkGray)),
                area,
            );
            return;
        }
        let labels = view
            .rows()
            .iter()
            .map(|row| runner_row_label(row, locale))
            .collect::<Vec<_>>();
        frame.render_widget(
            ListPicker::new(&labels, &self.rows).style(list_style(ACCENT)),
            area,
        );
        for visible in 0..self.row_height {
            let index = usize::from(self.rows.scroll).saturating_add(visible);
            if index >= view.rows().len() {
                break;
            }
            let rect = Rect::new(
                area.x,
                area.y
                    .saturating_add(u16::try_from(visible).unwrap_or(u16::MAX)),
                area.width,
                1,
            );
            self.row_areas.push((index, rect));
            self.clicks.register(rect, RunnerHit::Row(index));
        }
    }

    fn render_actions(&mut self, frame: &mut Frame, area: Rect, row: &RunnerRow, locale: Locale) {
        let footer_items = runner_action_footer_items(locale, row.is_editable());
        let expected_inner_width = area.width.min(68).saturating_sub(4);
        let footer_height = action_footer_required_height(expected_inner_width, &footer_items)
            .clamp(1, 3)
            .min(area.height);
        let panel = centered(area, 68, 8_u16.saturating_add(footer_height));
        frame.render_widget(Clear, panel);
        let block = padded_panel(row.label().to_owned(), ACCENT);
        let inner = block.inner(panel);
        frame.render_widget(block, panel);
        let [command, reason, actions] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(footer_height.min(inner.height)),
        ])
        .areas(inner);
        frame.render_widget(
            Paragraph::new(row.argv.as_ref().map_or_else(String::new, |argv| {
                join_editable_argv(argv, EditableArgvDialect::host())
            }))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::DarkGray)),
            command,
        );
        if let Some(code) = &row.reason {
            frame.render_widget(
                Paragraph::new(format!("⚠ {}", runner_reason(code, locale)))
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(Color::Red)),
                reason,
            );
        }
        self.footer.render(
            frame,
            actions,
            &footer_items,
            ActionFooterStyle::new(Color::White, BOX_DIM),
        );
    }

    fn render_removal(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        removal: &RunnerRemovalView,
        locale: Locale,
    ) {
        let footer_items = runner_removal_footer_items(locale);
        let expected_inner_width = area.width.min(72).saturating_sub(4);
        let footer_height = action_footer_required_height(expected_inner_width, &footer_items)
            .clamp(1, 2)
            .min(area.height);
        let panel = centered(area, 72, 8_u16.saturating_add(footer_height));
        frame.render_widget(Clear, panel);
        let block = padded_panel(text(locale, "Confirm removal").into_owned(), BOX_MAROON);
        let inner = block.inner(panel);
        frame.render_widget(block, panel);
        let [question, warning, actions] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(footer_height.min(inner.height)),
        ])
        .areas(inner);
        let prompt = if removal.container {
            text(locale, "Remove the malformed prompt runner container?").into_owned()
        } else if removal.invalid_row {
            format_text(
                locale,
                "Remove malformed runner row \"{}\"?",
                &[&removal.name],
            )
        } else {
            format_text(locale, "Remove the agent \"{}\"?", &[&removal.name])
        };
        frame.render_widget(Paragraph::new(prompt).wrap(Wrap { trim: false }), question);
        if removal.pinned_count > 0 {
            let template = if removal.pinned_count == 1 {
                "{} prompt pins this runner and will need another runner before it can run again."
            } else {
                "{} prompts pin this runner and will need another runner before they can run again."
            };
            frame.render_widget(
                Paragraph::new(format_text(locale, template, &[&removal.pinned_count]))
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(Color::Yellow)),
                warning,
            );
        }
        self.footer.render(
            frame,
            actions,
            &footer_items,
            ActionFooterStyle::new(Color::White, BOX_DIM),
        );
    }

    /// Dispatch one manager, overlay, or editor event.
    pub(crate) fn handle_event(
        &mut self,
        event: Event,
        view: &RunnerManagerView,
    ) -> RunnerManagerEventHandling {
        if let Some(editor) = view.editor() {
            return match self.editor.handle_event(event, editor) {
                RunnerEditorEventHandling::Action(action) => RunnerManagerEventHandling::Action(
                    if matches!(action, RunnerEditorAction::Cancel) {
                        RunnerManagerAction::CancelEditor
                    } else {
                        RunnerManagerAction::Editor(action)
                    },
                ),
                RunnerEditorEventHandling::Consumed => RunnerManagerEventHandling::Consumed,
                RunnerEditorEventHandling::Ignored => RunnerManagerEventHandling::Ignored,
            };
        }
        if let Event::Mouse(mouse) = &event {
            match self.footer.handle_mouse(mouse) {
                ActionFooterMouse::Action(action) => {
                    return RunnerManagerEventHandling::Action(action);
                }
                ActionFooterMouse::Scrolled => return RunnerManagerEventHandling::Consumed,
                ActionFooterMouse::Ignored => {}
            }
        }
        if view.removal().is_some() {
            return self.handle_removal_event(event);
        }
        if view.action_row().is_some() {
            return self.handle_action_event(event, view);
        }
        self.handle_list_event(event)
    }

    fn handle_list_event(&self, event: Event) -> RunnerManagerEventHandling {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Char('n') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return RunnerManagerEventHandling::Action(RunnerManagerAction::New);
                }
                let action = match key.code {
                    KeyCode::Esc => Some(RunnerManagerAction::Back),
                    KeyCode::Enter => Some(RunnerManagerAction::ActivateSelected),
                    KeyCode::Up => Some(RunnerManagerAction::Previous),
                    KeyCode::Down => Some(RunnerManagerAction::Next),
                    KeyCode::PageUp => {
                        Some(RunnerManagerAction::PagePrevious(self.row_height.max(1)))
                    }
                    KeyCode::PageDown => {
                        Some(RunnerManagerAction::PageNext(self.row_height.max(1)))
                    }
                    KeyCode::Home => Some(RunnerManagerAction::Home),
                    KeyCode::End => Some(RunnerManagerAction::End),
                    _ => None,
                };
                action.map_or(
                    RunnerManagerEventHandling::Ignored,
                    RunnerManagerEventHandling::Action,
                )
            }
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                match self.clicks.handle_click(mouse.column, mouse.row).cloned() {
                    Some(RunnerHit::Row(index)) => {
                        RunnerManagerEventHandling::Action(RunnerManagerAction::ActivateRow(index))
                    }
                    None => RunnerManagerEventHandling::Ignored,
                }
            }
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) =>
            {
                if self
                    .row_areas
                    .iter()
                    .any(|(_, area)| area.contains((mouse.column, mouse.row).into()))
                {
                    RunnerManagerEventHandling::Action(
                        if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                            RunnerManagerAction::Previous
                        } else {
                            RunnerManagerAction::Next
                        },
                    )
                } else {
                    RunnerManagerEventHandling::Ignored
                }
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Key(_)
            | Event::Resize(_, _) => RunnerManagerEventHandling::Ignored,
        }
    }

    fn handle_action_event(
        &self,
        event: Event,
        view: &RunnerManagerView,
    ) -> RunnerManagerEventHandling {
        let editable = view
            .action_row()
            .and_then(|index| view.rows().get(index))
            .is_some_and(RunnerRow::is_editable);
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let action = match key.code {
                    KeyCode::Char('e') if editable => Some(RunnerManagerAction::EditSelected),
                    KeyCode::Char('d') => Some(RunnerManagerAction::RemoveSelected),
                    KeyCode::Esc => Some(RunnerManagerAction::CloseActions),
                    _ => None,
                };
                action.map_or(
                    RunnerManagerEventHandling::Ignored,
                    RunnerManagerEventHandling::Action,
                )
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Key(_)
            | Event::Resize(_, _) => RunnerManagerEventHandling::Ignored,
        }
    }

    fn handle_removal_event(&self, event: Event) -> RunnerManagerEventHandling {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Char('y') => {
                    RunnerManagerEventHandling::Action(RunnerManagerAction::ConfirmRemove)
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    RunnerManagerEventHandling::Action(RunnerManagerAction::CancelRemove)
                }
                _ => RunnerManagerEventHandling::Ignored,
            },
            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Key(_)
            | Event::Resize(_, _) => RunnerManagerEventHandling::Ignored,
        }
    }
}

fn health_issue_label(issue: &HealthIssue, locale: Locale) -> String {
    let detail = match &issue.kind {
        HealthIssueKind::MissingTarget => {
            text(locale, "the launch target is gone from disk").into_owned()
        }
        HealthIssueKind::DriftedForm => text(
            locale,
            "form definitions are out of sync (open Entry settings → Resync)",
        )
        .into_owned(),
        HealthIssueKind::MissingNeeds { tools } => format_text(
            locale,
            "missing external command(s): {}",
            &[&tools.join(", ")],
        ),
        HealthIssueKind::LaunchBlocked { reason } => {
            format_text(locale, "a run would refuse to start — {}", &[reason])
        }
    };
    format!("⚠ {} — {detail}", issue.name)
}

fn runner_row_label(row: &RunnerRow, locale: Locale) -> String {
    let command = row.argv.as_ref().map_or_else(String::new, |argv| {
        join_editable_argv(argv, EditableArgvDialect::host())
    });
    if let Some(reason) = &row.reason {
        format!(
            "⚠ {}  {}  {command}",
            row.label(),
            runner_reason(reason, locale)
        )
    } else {
        format!("{}  {command}", row.label())
    }
}

fn runner_reason(code: &str, locale: Locale) -> String {
    let message = match code {
        "prompt-section-not-table" => {
            "the prompt value is not a table; repair it before runner management"
        }
        "runners-not-list" => {
            "the prompt.runners value is not a list; repair it before runner management"
        }
        "empty" => "Type the agent's command, e.g. mycli run {{prompt}}",
        "prompt-slot-count" => {
            "The command needs the {{prompt}} slot exactly once — that's where the rendered prompt lands."
        }
        "prompt-in-binary" => {
            "{{prompt}} can't be the command itself — the first word must be the program to run."
        }
        "stray-hole" => {
            "Runner commands take only the {{prompt}} slot — single-brace text is literal, and other {{holes}} aren't supported."
        }
        "name" => "A name is required.",
        "argv-type" => "The command must be a list of text arguments.",
        "row-not-table" => "This runner row isn't a table.",
        "duplicate" => "Another row already uses this runner name.",
        _ => "This runner row is malformed.",
    };
    text(locale, message).into_owned()
}

fn runner_editor_error(error: &RunnerEditorError, locale: Locale) -> String {
    let message = match error {
        RunnerEditorError::NameRequired => "A name is required.",
        RunnerEditorError::UnbalancedQuotes => "Unbalanced quotes in the command.",
        RunnerEditorError::EmptyCommand => "Type the agent's command, e.g. mycli run {{prompt}}",
        RunnerEditorError::PromptSlotCount => {
            "The command needs the {{prompt}} slot exactly once — that's where the rendered prompt lands."
        }
        RunnerEditorError::PromptInProgram => {
            "{{prompt}} can't be the command itself — the first word must be the program to run."
        }
        RunnerEditorError::UnsupportedHole => {
            "Runner commands take only the {{prompt}} slot — single-brace text is literal, and other {{holes}} aren't supported."
        }
    };
    text(locale, message).into_owned()
}

fn centered(area: Rect, maximum_width: u16, desired_height: u16) -> Rect {
    let [vertical] = Layout::vertical([Constraint::Length(desired_height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [horizontal] = Layout::horizontal([Constraint::Length(maximum_width.min(area.width))])
        .flex(Flex::Center)
        .areas(vertical);
    horizontal
}

fn list_style(accent: Color) -> ListPickerStyle {
    ListPickerStyle {
        selected_style: Style::default()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD),
        normal_style: Style::default().fg(Color::White),
        indicator_style: Style::default().fg(accent),
        border_style: Style::default(),
        indicator: "▶ ",
        indicator_empty: "  ",
        bordered: false,
    }
}

pub(crate) fn health_footer_items(locale: Locale) -> Vec<ActionFooterItem<HealthAction>> {
    vec![
        ActionFooterItem::new("Enter", text(locale, "Jump to entry"), HealthAction::Jump),
        ActionFooterItem::new(
            "Ctrl+R",
            text(locale, "Rebuild index"),
            HealthAction::Rebuild,
        ),
        ActionFooterItem::new("Esc", text(locale, "Back"), HealthAction::Back),
    ]
}

pub(crate) fn runner_editor_footer_items(
    locale: Locale,
) -> Vec<ActionFooterItem<RunnerEditorAction>> {
    vec![
        ActionFooterItem::new(
            "Tab/↓",
            text(locale, "Next field"),
            RunnerEditorAction::FocusNext,
        ),
        ActionFooterItem::new(
            "Shift+Tab/↑",
            text(locale, "Previous field"),
            RunnerEditorAction::FocusPrevious,
        ),
        ActionFooterItem::new_group("Enter", text(locale, "Save"), RunnerEditorAction::Submit),
        ActionFooterItem::new("Esc", text(locale, "Cancel"), RunnerEditorAction::Cancel),
    ]
}

pub(crate) fn runner_manager_footer_items(
    locale: Locale,
) -> Vec<ActionFooterItem<RunnerManagerAction>> {
    vec![
        ActionFooterItem::new(
            "Ctrl+N",
            text(locale, "New agent…"),
            RunnerManagerAction::New,
        ),
        ActionFooterItem::new("Esc", text(locale, "Back"), RunnerManagerAction::Back),
    ]
}

pub(crate) fn runner_action_footer_items(
    locale: Locale,
    editable: bool,
) -> Vec<ActionFooterItem<RunnerManagerAction>> {
    let mut items = Vec::new();
    if editable {
        items.push(ActionFooterItem::new(
            "e",
            text(locale, "Edit"),
            RunnerManagerAction::EditSelected,
        ));
    }
    items.push(ActionFooterItem::new(
        "d",
        text(locale, "Remove"),
        RunnerManagerAction::RemoveSelected,
    ));
    items.push(ActionFooterItem::new(
        "Esc",
        text(locale, "Back"),
        RunnerManagerAction::CloseActions,
    ));
    items
}

pub(crate) fn runner_removal_footer_items(
    locale: Locale,
) -> Vec<ActionFooterItem<RunnerManagerAction>> {
    vec![
        ActionFooterItem::new(
            "y",
            text(locale, "Remove"),
            RunnerManagerAction::ConfirmRemove,
        ),
        ActionFooterItem::new(
            "Esc",
            text(locale, "Keep"),
            RunnerManagerAction::CancelRemove,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_core::{backend::TestBackend, style::Color, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent};
    use skit_ui::{
        HealthIssue, HealthIssueKind, HealthRebuildOutcome, HealthSnapshot, HealthView,
        MirrorHealth, RunnerRow, RunnerRowIdentity, UvHealth,
    };

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn advertised_key(hint: &str) -> Event {
        let (code, modifiers) = match hint {
            "Enter" => (KeyCode::Enter, KeyModifiers::NONE),
            "Esc" => (KeyCode::Esc, KeyModifiers::NONE),
            "Tab/↓" => (KeyCode::Tab, KeyModifiers::NONE),
            "Shift+Tab/↑" => (KeyCode::BackTab, KeyModifiers::SHIFT),
            "Ctrl+N" => (KeyCode::Char('n'), KeyModifiers::CONTROL),
            "Ctrl+R" => (KeyCode::Char('r'), KeyModifiers::CONTROL),
            "e" => (KeyCode::Char('e'), KeyModifiers::NONE),
            "d" => (KeyCode::Char('d'), KeyModifiers::NONE),
            "y" => (KeyCode::Char('y'), KeyModifiers::NONE),
            _ => panic!("unsupported advertised management key: {hint}"),
        };
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn mouse(column: u16, row: u16) -> Event {
        mouse_event(MouseEventKind::Down(MouseButton::Left), column, row)
    }

    fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn text_position(buffer: &ratatui_core::buffer::Buffer, needle: &str) -> (u16, u16) {
        (0..buffer.area.height)
            .find_map(|y| {
                (0..buffer.area.width).find_map(|x| {
                    let tail = (x..buffer.area.width)
                        .map(|tail_x| buffer[(tail_x, y)].symbol())
                        .collect::<String>();
                    tail.starts_with(needle).then_some((x, y))
                })
            })
            .unwrap()
    }

    fn lines(buffer: &ratatui_core::buffer::Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn health() -> HealthView {
        HealthView::new(HealthSnapshot {
            uv: UvHealth::Missing,
            entry_count: 2,
            issues: vec![
                HealthIssue {
                    slug: "gone".to_owned(),
                    name: "Gone".to_owned(),
                    kind: HealthIssueKind::MissingTarget,
                },
                HealthIssue {
                    slug: "needs".to_owned(),
                    name: "Needs".to_owned(),
                    kind: HealthIssueKind::MissingNeeds {
                        tools: vec!["ffmpeg".to_owned()],
                    },
                },
            ],
            invalid_runner_rows: vec!["bad".to_owned()],
            mirror: MirrorHealth::Paused {
                axes: "pypi=tsinghua · github=nju · npm=npmmirror".to_owned(),
            },
            library_path: "/tmp/skit/scripts".to_owned(),
            library_size: "3 KiB".to_owned(),
            diagnostics: vec!["orphan metadata".to_owned()],
        })
    }

    fn row(index: usize, reason: Option<&str>, pinned_count: usize) -> RunnerRow {
        RunnerRow {
            identity: RunnerRowIdentity {
                index: Some(index),
                snapshot_token: format!("row-{index}"),
            },
            name: Some(if index == 0 { "good" } else { "broken" }.to_owned()),
            argv: Some(vec!["agent".to_owned(), "{{prompt}}".to_owned()]),
            reason: reason.map(str::to_owned),
            descriptor: "row".to_owned(),
            key_identities: vec![RunnerRowIdentity {
                index: Some(index),
                snapshot_token: format!("row-{index}"),
            }],
            pinned_count,
        }
    }

    #[test]
    fn every_management_footer_key_emits_its_typed_action_at_every_size_tier() {
        for (width, height) in [(120, 30), (46, 12), (24, 6)] {
            let health_view = health();
            let mut health_session = HealthScreenSession::default();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    health_session.render(frame, frame.area(), &health_view, Locale::En);
                })
                .unwrap();
            for item in health_footer_items(Locale::En) {
                assert_eq!(
                    health_session
                        .handle_event(advertised_key(item.advertised_key()), &health_view),
                    HealthEventHandling::Action(item.typed_action().clone()),
                    "Health key {} at {width}x{height}",
                    item.advertised_key()
                );
            }

            let editor_view = RunnerEditorView::new();
            let mut editor_session = RunnerEditorSession::default();
            terminal
                .draw(|frame| {
                    editor_session.render(frame, frame.area(), &editor_view, Locale::En);
                })
                .unwrap();
            for item in runner_editor_footer_items(Locale::En) {
                assert_eq!(
                    editor_session
                        .handle_event(advertised_key(item.advertised_key()), &editor_view),
                    RunnerEditorEventHandling::Action(item.typed_action().clone()),
                    "runner editor key {} at {width}x{height}",
                    item.advertised_key()
                );
            }

            let manager_view = RunnerManagerView::new(vec![row(0, None, 3)]);
            let mut manager_session = RunnerManagerSession::default();
            terminal
                .draw(|frame| {
                    manager_session.render(frame, frame.area(), &manager_view, Locale::En);
                })
                .unwrap();
            for item in runner_manager_footer_items(Locale::En) {
                assert_eq!(
                    manager_session
                        .handle_event(advertised_key(item.advertised_key()), &manager_view,),
                    RunnerManagerEventHandling::Action(item.typed_action().clone()),
                    "runner manager key {} at {width}x{height}",
                    item.advertised_key()
                );
            }

            let mut action_view = RunnerManagerView::new(vec![row(0, None, 3)]);
            action_view.reduce(RunnerManagerAction::ActivateSelected);
            terminal
                .draw(|frame| {
                    manager_session.render(frame, frame.area(), &action_view, Locale::En);
                })
                .unwrap();
            for item in runner_action_footer_items(Locale::En, true) {
                assert_eq!(
                    manager_session
                        .handle_event(advertised_key(item.advertised_key()), &action_view,),
                    RunnerManagerEventHandling::Action(item.typed_action().clone()),
                    "runner action key {} at {width}x{height}",
                    item.advertised_key()
                );
            }

            action_view.reduce(RunnerManagerAction::RemoveSelected);
            terminal
                .draw(|frame| {
                    manager_session.render(frame, frame.area(), &action_view, Locale::En);
                })
                .unwrap();
            for item in runner_removal_footer_items(Locale::En) {
                assert_eq!(
                    manager_session
                        .handle_event(advertised_key(item.advertised_key()), &action_view,),
                    RunnerManagerEventHandling::Action(item.typed_action().clone()),
                    "runner removal key {} at {width}x{height}",
                    item.advertised_key()
                );
            }

            let mut locked_view = RunnerManagerView::new(vec![row(0, Some("broken"), 0)]);
            locked_view.reduce(RunnerManagerAction::ActivateSelected);
            terminal
                .draw(|frame| {
                    manager_session.render(frame, frame.area(), &locked_view, Locale::En);
                })
                .unwrap();
            for item in runner_action_footer_items(Locale::En, false) {
                assert_eq!(
                    manager_session
                        .handle_event(advertised_key(item.advertised_key()), &locked_view,),
                    RunnerManagerEventHandling::Action(item.typed_action().clone()),
                    "locked runner action key {} at {width}x{height}",
                    item.advertised_key()
                );
            }
        }
    }

    #[test]
    fn health_renders_actionable_complete_report_with_color() {
        let view = health();
        let mut session = HealthScreenSession::default();
        let backend = TestBackend::new(100, 25);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = lines(buffer);
        assert!(text.contains("Health check"), "{text}");
        assert!(text.contains("uv: not found"), "{text}");
        assert!(text.contains("docs.astral.sh/uv"), "{text}");
        assert!(text.contains("missing external command"), "{text}");
        assert!(text.contains("ffmpeg"), "{text}");
        assert!(text.contains("Mirrors: off (saved:"), "{text}");
        assert!(text.contains("Ctrl+R"), "{text}");
        assert!(buffer.content.iter().any(|cell| cell.fg != Color::Reset));
    }

    #[test]
    fn health_keyboard_and_mouse_emit_the_advertised_actions() {
        let view = health();
        let mut session = HealthScreenSession::default();
        let backend = TestBackend::new(100, 25);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        assert_eq!(
            session.handle_event(key(KeyCode::Down), &view),
            HealthEventHandling::Action(HealthAction::Next)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &view),
            HealthEventHandling::Action(HealthAction::Jump)
        );
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL,)),
                &view
            ),
            HealthEventHandling::Action(HealthAction::Rebuild)
        );
        let (index, area) = session.issue_areas[1];
        assert_eq!(index, 1);
        assert_eq!(
            session.handle_event(mouse(area.x, area.y), &view),
            HealthEventHandling::Action(HealthAction::ActivateIssue(1))
        );
        for (needle, expected) in [
            ("Enter Jump to entry", HealthAction::Jump),
            ("Ctrl+R Rebuild index", HealthAction::Rebuild),
            ("Esc Back", HealthAction::Back),
        ] {
            let (x, y) = text_position(terminal.backend().buffer(), needle);
            assert_eq!(
                session.handle_event(mouse(x, y), &view),
                HealthEventHandling::Action(expected),
                "visible Health chip must be clickable: {needle}"
            );
        }
    }

    #[test]
    fn healthy_report_keeps_jump_visible_and_scrolls_the_summary() {
        let mut snapshot = health().snapshot().clone();
        snapshot.issues.clear();
        snapshot.diagnostics = (0..12).map(|index| format!("diagnostic {index}")).collect();
        let view = HealthView::new(snapshot);
        let mut session = HealthScreenSession::default();
        let backend = TestBackend::new(70, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();

        let (x, y) = text_position(terminal.backend().buffer(), "Enter Jump to entry");
        assert_eq!(
            session.handle_event(mouse(x, y), &view),
            HealthEventHandling::Action(HealthAction::Jump)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Down), &view),
            HealthEventHandling::Consumed
        );
        assert!(session.summary_scroll.scroll_offset() > 0);
    }

    #[test]
    fn runner_manager_renders_raw_reasons_empty_state_and_list_actions() {
        let mut view =
            RunnerManagerView::new(vec![row(0, None, 3), row(1, Some("prompt-slot-count"), 0)]);
        let mut session = RunnerManagerSession::default();
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        let text = lines(terminal.backend().buffer());
        assert!(text.contains("Agents (prompt runners)"), "{text}");
        assert!(text.contains("good"), "{text}");
        assert!(text.contains("exactly once"), "{text}");
        assert!(text.contains("Ctrl+N"), "{text}");
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
                &view,
            ),
            RunnerManagerEventHandling::Action(RunnerManagerAction::New)
        );
        let (index, area) = session.row_areas[1];
        assert_eq!(
            session.handle_event(mouse(area.x, area.y), &view),
            RunnerManagerEventHandling::Action(RunnerManagerAction::ActivateRow(index))
        );

        view = RunnerManagerView::new(Vec::new());
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        assert!(lines(terminal.backend().buffer()).contains("No agents configured yet."));
    }

    #[test]
    fn reusable_editor_uses_mature_inputs_and_every_visible_chip_is_clickable() {
        let mut view = RunnerEditorView::new();
        let mut session = RunnerEditorSession::default();
        let backend = TestBackend::new(90, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        assert_eq!(
            session.handle_event(key(KeyCode::Char('x')), &view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::SetName("x".to_owned()))
        );
        view.reduce(RunnerEditorAction::SetName("x".to_owned()));
        view.reduce(RunnerEditorAction::Focus(RunnerEditorField::Command));
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        assert_eq!(
            session.handle_event(key(KeyCode::Char('a')), &view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::SetCommand("a".to_owned()))
        );
        view.reduce(RunnerEditorAction::SetCommand(
            "agent --message {{prompt}}".to_owned(),
        ));
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        let screen = lines(terminal.backend().buffer());
        assert!(screen.contains("New agent (runner)"), "{screen}");
        assert!(!screen.contains("Edit agent (runner)"), "{screen}");
        for (needle, expected) in [
            ("Tab/↓ Next field", RunnerEditorAction::FocusNext),
            (
                "Shift+Tab/↑ Previous field",
                RunnerEditorAction::FocusPrevious,
            ),
        ] {
            let (x, y) = text_position(terminal.backend().buffer(), needle);
            assert_eq!(
                session.handle_event(mouse(x, y), &view),
                RunnerEditorEventHandling::Action(expected),
                "visible editor chip must be clickable: {needle}"
            );
        }
        let save = text_position(terminal.backend().buffer(), "Enter Save");
        assert_eq!(
            session.handle_event(mouse(save.0, save.1), &view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::Submit)
        );
        let cancel = text_position(terminal.backend().buffer(), "Esc Cancel");
        assert_eq!(
            session.handle_event(mouse(cancel.0, cancel.1), &view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::Cancel)
        );
        for (code, expected) in [
            (KeyCode::Tab, RunnerEditorAction::FocusNext),
            (KeyCode::Down, RunnerEditorAction::FocusNext),
            (KeyCode::BackTab, RunnerEditorAction::FocusPrevious),
            (KeyCode::Up, RunnerEditorAction::FocusPrevious),
        ] {
            assert_eq!(
                session.handle_event(key(code), &view),
                RunnerEditorEventHandling::Action(expected)
            );
        }
    }

    #[test]
    fn existing_runner_name_is_visibly_locked_and_cannot_take_focus() {
        let view = RunnerEditorView::edit(&row(0, None, 2));
        let mut session = RunnerEditorSession::default();
        let backend = TestBackend::new(90, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        let screen = lines(terminal.backend().buffer());
        assert!(screen.contains('🔒') && screen.contains("Name"), "{screen}");
        let (x, y) = text_position(terminal.backend().buffer(), "good");
        assert_eq!(
            session.handle_event(mouse(x, y), &view),
            RunnerEditorEventHandling::Ignored
        );
        assert_eq!(view.focused(), RunnerEditorField::Command);
    }

    #[test]
    fn pinned_removal_warning_and_action_overlay_are_keyboard_and_mouse_operable() {
        let mut view = RunnerManagerView::new(vec![row(0, None, 3)]);
        view.reduce(RunnerManagerAction::ActivateSelected);
        let mut session = RunnerManagerSession::default();
        let backend = TestBackend::new(90, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        assert!(lines(terminal.backend().buffer()).contains("Edit"));
        for (needle, expected) in [
            ("e Edit", RunnerManagerAction::EditSelected),
            ("d Remove", RunnerManagerAction::RemoveSelected),
            ("Esc Back", RunnerManagerAction::CloseActions),
        ] {
            let (x, y) = text_position(terminal.backend().buffer(), needle);
            assert_eq!(
                session.handle_event(mouse(x, y), &view),
                RunnerManagerEventHandling::Action(expected),
                "visible row-action chip must be clickable: {needle}"
            );
        }
        assert_eq!(
            session.handle_event(key(KeyCode::Char('d')), &view),
            RunnerManagerEventHandling::Action(RunnerManagerAction::RemoveSelected)
        );
        view.reduce(RunnerManagerAction::RemoveSelected);
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        let text = lines(terminal.backend().buffer());
        assert!(text.contains("3 prompts pin this runner"), "{text}");
        assert_eq!(
            session.handle_event(key(KeyCode::Char('y')), &view),
            RunnerManagerEventHandling::Action(RunnerManagerAction::ConfirmRemove)
        );
        for (needle, expected) in [
            ("y Remove", RunnerManagerAction::ConfirmRemove),
            ("Esc Keep", RunnerManagerAction::CancelRemove),
        ] {
            let (x, y) = text_position(terminal.backend().buffer(), needle);
            assert_eq!(
                session.handle_event(mouse(x, y), &view),
                RunnerManagerEventHandling::Action(expected),
                "visible confirmation chip must be clickable: {needle}"
            );
        }
    }

    #[test]
    fn runner_list_footer_chips_match_keyboard_and_mouse() {
        let view = RunnerManagerView::new(vec![row(0, None, 0)]);
        let mut session = RunnerManagerSession::default();
        let backend = TestBackend::new(90, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();

        for (needle, expected) in [
            ("Ctrl+N New agent…", RunnerManagerAction::New),
            ("Esc Back", RunnerManagerAction::Back),
        ] {
            let (x, y) = text_position(terminal.backend().buffer(), needle);
            assert_eq!(
                session.handle_event(mouse(x, y), &view),
                RunnerManagerEventHandling::Action(expected),
                "visible manager chip must be clickable: {needle}"
            );
        }
    }

    #[test]
    fn management_widgets_render_safely_in_a_tiny_terminal() {
        let health = health();
        let mut health_session = HealthScreenSession::default();
        let backend = TestBackend::new(24, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| health_session.render(frame, frame.area(), &health, Locale::En))
            .unwrap();

        let mut manager = RunnerManagerView::new(vec![row(0, None, 2)]);
        manager.reduce(RunnerManagerAction::ActivateSelected);
        manager.reduce(RunnerManagerAction::EditSelected);
        let mut manager_session = RunnerManagerSession::default();
        terminal
            .draw(|frame| manager_session.render(frame, frame.area(), &manager, Locale::En))
            .unwrap();

        manager.reduce(RunnerManagerAction::CancelEditor);
        manager.reduce(RunnerManagerAction::ActivateSelected);
        manager.reduce(RunnerManagerAction::RemoveSelected);
        terminal
            .draw(|frame| manager_session.render(frame, frame.area(), &manager, Locale::En))
            .unwrap();
    }

    #[test]
    fn health_variants_and_full_event_surface_are_rendered_and_dispatched() {
        for (uv, mirror) in [
            (UvHealth::Found("/usr/bin/uv".to_owned()), MirrorHealth::Off),
            (
                UvHealth::NotRequired,
                MirrorHealth::On {
                    axes: "pypi=custom".to_owned(),
                },
            ),
        ] {
            let mut snapshot = health().snapshot().clone();
            snapshot.uv = uv;
            snapshot.mirror = mirror;
            snapshot.entry_count = 1;
            snapshot.issues = vec![
                HealthIssue {
                    slug: "drift".to_owned(),
                    name: "Drift".to_owned(),
                    kind: HealthIssueKind::DriftedForm,
                },
                HealthIssue {
                    slug: "blocked".to_owned(),
                    name: "Blocked".to_owned(),
                    kind: HealthIssueKind::LaunchBlocked {
                        reason: "runner missing".to_owned(),
                    },
                },
            ];
            let mut view = HealthView::new(snapshot.clone());
            view.reduce(HealthAction::Rebuilt {
                snapshot: Box::new(snapshot),
                outcome: HealthRebuildOutcome {
                    entry_count: 1,
                    problems: vec!["one rebuild warning".to_owned()],
                },
            });
            let mut session = HealthScreenSession::default();
            let mut terminal = Terminal::new(TestBackend::new(66, 14)).unwrap();
            terminal
                .draw(|frame| session.render(frame, frame.area(), &view, Locale::ZhCn))
                .unwrap();
            let screen = lines(terminal.backend().buffer());
            assert!(
                screen.contains("索 引") || screen.contains("Index"),
                "{screen}"
            );

            for (code, expected) in [
                (KeyCode::Esc, HealthAction::Back),
                (KeyCode::Up, HealthAction::Previous),
                (
                    KeyCode::PageUp,
                    HealthAction::PagePrevious(session.issue_height.max(1)),
                ),
                (
                    KeyCode::PageDown,
                    HealthAction::PageNext(session.issue_height.max(1)),
                ),
                (KeyCode::Home, HealthAction::Home),
                (KeyCode::End, HealthAction::End),
            ] {
                assert_eq!(
                    session.handle_event(key(code), &view),
                    HealthEventHandling::Action(expected)
                );
            }
            let area = session.issue_areas[0].1;
            assert_eq!(
                session.handle_event(mouse_event(MouseEventKind::ScrollUp, area.x, area.y), &view),
                HealthEventHandling::Action(HealthAction::Previous)
            );
            assert_eq!(
                session.handle_event(
                    mouse_event(MouseEventKind::ScrollDown, area.x, area.y),
                    &view,
                ),
                HealthEventHandling::Action(HealthAction::Next)
            );
            for event in [
                mouse_event(MouseEventKind::Moved, 0, 0),
                mouse_event(MouseEventKind::Up(MouseButton::Left), 0, 0),
                Event::FocusGained,
                Event::FocusLost,
                Event::Paste("ignored".to_owned()),
                Event::Resize(10, 10),
            ] {
                assert_eq!(
                    session.handle_event(event, &view),
                    HealthEventHandling::Ignored
                );
            }
        }
    }

    #[test]
    fn runner_editor_renders_every_mode_error_and_event_class() {
        let mut views = vec![
            RunnerEditorView::new(),
            RunnerEditorView::repair(&row(1, Some("empty"), 0)),
        ];
        let mut host_error = RunnerEditorView::new();
        host_error.reduce(RunnerEditorAction::MutationFailed(
            "host refused".to_owned(),
        ));
        views.push(host_error);
        for view in &mut views {
            if view.host_error().is_none() {
                view.reduce(RunnerEditorAction::Submit);
            }
            let mut session = RunnerEditorSession::default();
            let mut terminal = Terminal::new(TestBackend::new(50, 16)).unwrap();
            terminal
                .draw(|frame| session.render(frame, frame.area(), view, Locale::ZhTw))
                .unwrap();
            let screen = lines(terminal.backend().buffer());
            assert!(screen.contains("{{prompt}}"), "{screen}");

            assert!(matches!(
                session.handle_event(Event::Paste("xy".to_owned()), view),
                RunnerEditorEventHandling::Action(_)
            ));
            assert_eq!(
                session.handle_event(mouse_event(MouseEventKind::Moved, 0, 0), view),
                RunnerEditorEventHandling::Ignored
            );
            assert_eq!(
                session.handle_event(
                    mouse_event(MouseEventKind::Up(MouseButton::Left), 0, 0),
                    view
                ),
                RunnerEditorEventHandling::Ignored
            );
            assert_eq!(
                session.handle_event(Event::Resize(1, 1), view),
                RunnerEditorEventHandling::Ignored
            );
            assert_eq!(
                session.handle_event(Event::FocusGained, view),
                RunnerEditorEventHandling::Ignored
            );
            assert_eq!(
                session.handle_event(mouse_event(MouseEventKind::ScrollDown, 0, 0), view),
                RunnerEditorEventHandling::Ignored
            );
        }
    }

    #[test]
    fn runner_manager_dispatches_list_action_removal_and_pointer_reverse_matrix() {
        let mut malformed = row(1, Some("row-not-table"), 0);
        malformed.argv = None;
        let mut view = RunnerManagerView::new(vec![row(0, None, 0), malformed]);
        let mut session = RunnerManagerSession::default();
        let mut terminal = Terminal::new(TestBackend::new(52, 12)).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();

        for (code, expected) in [
            (KeyCode::Esc, RunnerManagerAction::Back),
            (KeyCode::Enter, RunnerManagerAction::ActivateSelected),
            (KeyCode::Up, RunnerManagerAction::Previous),
            (KeyCode::Down, RunnerManagerAction::Next),
            (
                KeyCode::PageUp,
                RunnerManagerAction::PagePrevious(session.row_height.max(1)),
            ),
            (
                KeyCode::PageDown,
                RunnerManagerAction::PageNext(session.row_height.max(1)),
            ),
            (KeyCode::Home, RunnerManagerAction::Home),
            (KeyCode::End, RunnerManagerAction::End),
        ] {
            assert_eq!(
                session.handle_event(key(code), &view),
                RunnerManagerEventHandling::Action(expected)
            );
        }
        let row_area = session.row_areas[0].1;
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::ScrollUp, row_area.x, row_area.y),
                &view,
            ),
            RunnerManagerEventHandling::Action(RunnerManagerAction::Previous)
        );
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::ScrollDown, row_area.x, row_area.y),
                &view,
            ),
            RunnerManagerEventHandling::Action(RunnerManagerAction::Next)
        );
        for event in [
            mouse_event(MouseEventKind::Moved, 0, 0),
            mouse_event(MouseEventKind::Up(MouseButton::Left), 0, 0),
            Event::FocusLost,
            Event::Paste("ignored".to_owned()),
            Event::Resize(1, 1),
        ] {
            assert_eq!(
                session.handle_event(event, &view),
                RunnerManagerEventHandling::Ignored
            );
        }

        view.reduce(RunnerManagerAction::ActivateRow(1));
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        assert_eq!(
            session.handle_event(key(KeyCode::Char('e')), &view),
            RunnerManagerEventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('d')), &view),
            RunnerManagerEventHandling::Action(RunnerManagerAction::RemoveSelected)
        );
        view.reduce(RunnerManagerAction::RemoveSelected);
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        for (code, expected) in [
            (KeyCode::Char('n'), RunnerManagerAction::CancelRemove),
            (KeyCode::Esc, RunnerManagerAction::CancelRemove),
            (KeyCode::Char('y'), RunnerManagerAction::ConfirmRemove),
        ] {
            assert_eq!(
                session.handle_event(key(code), &view),
                RunnerManagerEventHandling::Action(expected)
            );
        }
        assert_eq!(
            session.handle_event(key(KeyCode::Char('x')), &view),
            RunnerManagerEventHandling::Ignored
        );
    }

    #[test]
    fn remaining_management_variants_keep_real_render_and_event_ownership() {
        let mut snapshot = health().snapshot().clone();
        snapshot.entry_count = 2;
        snapshot.issues.clear();
        snapshot.invalid_runner_rows.clear();
        snapshot.diagnostics.clear();
        let mut health_view = HealthView::new(snapshot.clone());
        health_view.reduce(HealthAction::Rebuilt {
            snapshot: Box::new(snapshot),
            outcome: HealthRebuildOutcome {
                entry_count: 2,
                problems: vec!["first".to_owned(), "second".to_owned()],
            },
        });
        let mut health_session = HealthScreenSession::default();
        health_session.summary_scroll.set_scroll_offset(usize::MAX);
        let mut health_terminal = Terminal::new(TestBackend::new(26, 7)).unwrap();
        health_terminal
            .draw(|frame| {
                health_session.render(frame, frame.area(), &health_view, Locale::En);
            })
            .unwrap();
        assert_eq!(health_session.summary_scroll.scroll_offset(), 0);
        assert_eq!(
            health_session.handle_event(
                mouse_event(MouseEventKind::ScrollDown, u16::MAX, u16::MAX),
                &health_view,
            ),
            HealthEventHandling::Ignored
        );
        assert_eq!(
            health_session.handle_event(key(KeyCode::Char('x')), &health_view),
            HealthEventHandling::Ignored
        );
        assert_eq!(
            health_session.handle_event(mouse(0, 0), &health_view),
            HealthEventHandling::Ignored
        );
        let _ = health_session
            .handle_event(mouse_event(MouseEventKind::ScrollDown, 2, 5), &health_view);

        let error_inputs = [
            ("name", "\"", RunnerEditorError::UnbalancedQuotes),
            ("name", "", RunnerEditorError::EmptyCommand),
            ("name", "agent", RunnerEditorError::PromptSlotCount),
            ("name", "{{prompt}}", RunnerEditorError::PromptInProgram),
            (
                "name",
                "agent {{other}} {{prompt}}",
                RunnerEditorError::UnsupportedHole,
            ),
        ];
        for (name, command, expected) in error_inputs {
            let mut view = RunnerEditorView::new();
            view.reduce(RunnerEditorAction::SetName(name.to_owned()));
            view.reduce(RunnerEditorAction::SetCommand(command.to_owned()));
            view.reduce(RunnerEditorAction::Submit);
            assert_eq!(view.error(), Some(&expected));
            let mut session = RunnerEditorSession::default();
            let mut terminal = Terminal::new(TestBackend::new(38, 14)).unwrap();
            terminal
                .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
                .unwrap();
            assert_eq!(
                session.handle_event(key(KeyCode::Esc), &view),
                RunnerEditorEventHandling::Action(RunnerEditorAction::Cancel)
            );
            assert_eq!(
                session.handle_event(key(KeyCode::Enter), &view),
                RunnerEditorEventHandling::Action(RunnerEditorAction::Submit)
            );
            assert_eq!(
                session.handle_event(key(KeyCode::Null), &view),
                RunnerEditorEventHandling::Ignored
            );
            let _ = session.handle_event(mouse_event(MouseEventKind::ScrollDown, 2, 12), &view);
        }

        let reason_codes = [
            "prompt-section-not-table",
            "runners-not-list",
            "prompt-in-binary",
            "stray-hole",
            "name",
            "argv-type",
            "duplicate",
            "future-code",
        ];
        let rows = reason_codes
            .iter()
            .enumerate()
            .map(|(index, reason)| row(index, Some(reason), 0))
            .collect::<Vec<_>>();
        let mut manager_view = RunnerManagerView::new(rows);
        manager_view.reduce(RunnerManagerAction::MutationSucceeded {
            rows: manager_view.rows().to_vec(),
            selected_name: None,
            message: "saved".to_owned(),
        });
        let mut manager_session = RunnerManagerSession::default();
        let mut manager_terminal = Terminal::new(TestBackend::new(84, 22)).unwrap();
        manager_terminal
            .draw(|frame| {
                manager_session.render(frame, frame.area(), &manager_view, Locale::ZhCn);
            })
            .unwrap();
        assert!(lines(manager_terminal.backend().buffer()).contains("saved"));
        assert_eq!(
            manager_session.handle_event(key(KeyCode::Char('x')), &manager_view),
            RunnerManagerEventHandling::Ignored
        );
        assert_eq!(
            manager_session.handle_event(mouse(0, 0), &manager_view),
            RunnerManagerEventHandling::Ignored
        );
        assert_eq!(
            manager_session
                .handle_event(mouse_event(MouseEventKind::ScrollDown, 0, 0), &manager_view,),
            RunnerManagerEventHandling::Ignored
        );

        manager_view.reduce(RunnerManagerAction::ActivateRow(0));
        assert_eq!(
            manager_session.handle_event(key(KeyCode::Char('e')), &manager_view),
            RunnerManagerEventHandling::Action(RunnerManagerAction::EditSelected)
        );
        assert_eq!(
            manager_session.handle_event(key(KeyCode::Esc), &manager_view),
            RunnerManagerEventHandling::Action(RunnerManagerAction::CloseActions)
        );
        assert_eq!(
            manager_session.handle_event(Event::FocusLost, &manager_view),
            RunnerManagerEventHandling::Ignored
        );

        let narrow_view = RunnerManagerView::new(vec![row(0, None, 0)]);
        let mut narrow_terminal = Terminal::new(TestBackend::new(18, 7)).unwrap();
        narrow_terminal
            .draw(|frame| {
                manager_session.render(frame, frame.area(), &narrow_view, Locale::En);
            })
            .unwrap();
        let footer_scroll = manager_session
            .handle_event(mouse_event(MouseEventKind::ScrollDown, 2, 5), &narrow_view);
        assert!(matches!(
            footer_scroll,
            RunnerManagerEventHandling::Consumed | RunnerManagerEventHandling::Ignored
        ));

        manager_view.reduce(RunnerManagerAction::New);
        manager_terminal
            .draw(|frame| {
                manager_session.render(frame, frame.area(), &manager_view, Locale::En);
            })
            .unwrap();
        assert_eq!(
            manager_session.handle_event(key(KeyCode::Esc), &manager_view),
            RunnerManagerEventHandling::Action(RunnerManagerAction::CancelEditor)
        );
        assert!(matches!(
            manager_session.handle_event(key(KeyCode::Char('a')), &manager_view),
            RunnerManagerEventHandling::Action(RunnerManagerAction::Editor(_))
        ));
    }

    #[test]
    fn management_scroll_clamp_editor_fields_and_removal_shapes_are_positive() {
        let mut crowded = health().snapshot().clone();
        crowded.issues.clear();
        crowded.diagnostics = (0..20).map(|index| format!("line {index}")).collect();
        let crowded_view = HealthView::new(crowded);
        let mut health_session = HealthScreenSession::default();
        let mut terminal = Terminal::new(TestBackend::new(42, 8)).unwrap();
        terminal
            .draw(|frame| health_session.render(frame, frame.area(), &crowded_view, Locale::En))
            .unwrap();
        assert_eq!(
            health_session.handle_event(key(KeyCode::End), &crowded_view),
            HealthEventHandling::Consumed
        );
        assert!(health_session.summary_scroll.scroll_offset() > 0);
        let summary = health_session.summary_area;
        assert_eq!(
            health_session.handle_event(
                mouse_event(MouseEventKind::ScrollUp, summary.x, summary.y),
                &crowded_view,
            ),
            HealthEventHandling::Consumed
        );
        let sparse_view = HealthView::new(HealthSnapshot {
            uv: UvHealth::NotRequired,
            entry_count: 0,
            issues: Vec::new(),
            invalid_runner_rows: Vec::new(),
            mirror: MirrorHealth::Off,
            library_path: "library".to_owned(),
            library_size: "0 B".to_owned(),
            diagnostics: Vec::new(),
        });
        terminal
            .draw(|frame| health_session.render(frame, frame.area(), &sparse_view, Locale::En))
            .unwrap();
        assert_eq!(health_session.summary_scroll.scroll_offset(), 0);

        let mut editor_view = RunnerEditorView::new();
        editor_view.reduce(RunnerEditorAction::SetName("a".to_owned()));
        let mut editor_session = RunnerEditorSession::default();
        let mut editor_terminal = Terminal::new(TestBackend::new(32, 14)).unwrap();
        editor_terminal
            .draw(|frame| editor_session.render(frame, frame.area(), &editor_view, Locale::En))
            .unwrap();
        assert_eq!(
            editor_session.handle_event(key(KeyCode::Left), &editor_view),
            RunnerEditorEventHandling::Consumed
        );
        let name = text_position(editor_terminal.backend().buffer(), "Name, e.g. aider");
        assert_eq!(
            editor_session.handle_event(mouse(name.0, name.1), &editor_view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::Focus(RunnerEditorField::Name))
        );
        editor_view.reduce(RunnerEditorAction::Focus(RunnerEditorField::Command));
        assert!(matches!(
            editor_session.handle_event(Event::Paste("cmd".to_owned()), &editor_view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::SetCommand(_))
        ));

        let mut container = row(0, Some("runners-not-list"), 0);
        container.identity.index = None;
        container.name = None;
        container.argv = None;
        let mut manager_view = RunnerManagerView::new(vec![container]);
        manager_view.reduce(RunnerManagerAction::ActivateSelected);
        manager_view.reduce(RunnerManagerAction::RemoveSelected);
        let mut manager_session = RunnerManagerSession::default();
        let mut manager_terminal = Terminal::new(TestBackend::new(50, 12)).unwrap();
        manager_terminal
            .draw(|frame| manager_session.render(frame, frame.area(), &manager_view, Locale::En))
            .unwrap();
        assert!(
            lines(manager_terminal.backend().buffer())
                .contains("malformed prompt runner container")
        );
        assert_eq!(
            manager_session.handle_event(Event::FocusLost, &manager_view),
            RunnerManagerEventHandling::Ignored
        );

        let mut pinned = RunnerManagerView::new(vec![row(0, None, 1)]);
        pinned.reduce(RunnerManagerAction::ActivateSelected);
        pinned.reduce(RunnerManagerAction::RemoveSelected);
        manager_terminal
            .draw(|frame| manager_session.render(frame, frame.area(), &pinned, Locale::En))
            .unwrap();
        assert!(lines(manager_terminal.backend().buffer()).contains("1 prompt pins"));

        let mut editor_manager = RunnerManagerView::new(Vec::new());
        editor_manager.reduce(RunnerManagerAction::New);
        editor_manager.reduce(RunnerManagerAction::Editor(RunnerEditorAction::SetName(
            "a".to_owned(),
        )));
        manager_terminal
            .draw(|frame| {
                manager_session.render(frame, frame.area(), &editor_manager, Locale::En);
            })
            .unwrap();
        assert_eq!(
            manager_session.handle_event(key(KeyCode::Left), &editor_manager),
            RunnerManagerEventHandling::Consumed
        );
        assert_eq!(
            manager_session
                .handle_event(mouse_event(MouseEventKind::Moved, 0, 0), &editor_manager,),
            RunnerManagerEventHandling::Ignored
        );
    }
}
