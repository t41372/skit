//! Health and prompt-runner management widgets.

use ratatui_core::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::Line,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui_interact::{
    components::{
        ListPicker, ListPickerState, ListPickerStyle, ScrollableContentState,
        handle_scrollable_content_key, handle_scrollable_content_mouse,
    },
    traits::ClickRegionRegistry,
};
use ratatui_widgets::{clear::Clear, paragraph::Paragraph, paragraph::Wrap};
use skit_i18n::{Locale, format_text, text};
use skit_ui::{
    HealthAction, HealthIssue, HealthIssueKind, HealthView, MirrorHealth, RunnerEditorAction,
    RunnerEditorError, RunnerEditorField, RunnerEditorMode, RunnerEditorView, UvHealth,
};
use tui_input::{Input as LineInput, InputRequest, backend::crossterm::EventHandler as _};

use crate::{
    agent_review::{
        AgentReviewNode, AgentReviewSnapshotError, list_picker as snapshot_list_picker,
        node as snapshot_node, rect as snapshot_rect, scroll as snapshot_scroll,
        value as snapshot_value,
    },
    footer::{
        ActionFooterItem, ActionFooterMouse, ActionFooterSession, ActionFooterStyle,
        action_footer_required_height,
    },
    local_action::LocalKey,
    pointer::{ClickOutcome, ClickTracker, EditableGeometry},
    session::render_line_input,
    theme::{ACCENT, BOX_DIM, BOX_GREEN, padded_panel},
};

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

fn cancels_pointer_press(event: &Event) -> bool {
    match event {
        Event::Resize(_, _) => true,
        Event::Mouse(mouse) => matches!(
            mouse.kind,
            MouseEventKind::Drag(_)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
        Event::FocusGained | Event::FocusLost | Event::Key(_) | Event::Paste(_) => false,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
enum HealthHit {
    Issue(usize),
}

/// Mature list and scroll state for the typed Health screen.
#[derive(Clone, Debug, Default)]
pub(crate) struct HealthScreenSession {
    issues: ListPickerState,
    summary_scroll: ScrollableContentState,
    summary_area: Rect,
    summary_height: usize,
    issue_height: usize,
    clicks: ClickRegionRegistry<HealthHit>,
    click: ClickTracker<HealthHit>,
    issue_areas: Vec<(usize, Rect)>,
    footer: ActionFooterSession<HealthAction>,
}

impl HealthScreenSession {
    /// Cancel armed issue and footer targets before a pointer discontinuity.
    pub(crate) fn cancel_click(&mut self) {
        self.click.cancel();
        self.footer.cancel_click();
    }

    #[cfg(test)]
    pub(crate) fn perturb_agent_review_state(&mut self) {
        self.summary_height = self.summary_height.saturating_add(1);
    }

    pub(crate) fn agent_review_snapshot(
        &self,
    ) -> Result<AgentReviewNode, AgentReviewSnapshotError> {
        let Self {
            issues,
            summary_scroll,
            summary_area,
            summary_height,
            issue_height,
            clicks,
            click,
            issue_areas,
            footer,
        } = self;
        let issue_areas = issue_areas
            .iter()
            .map(|(index, area)| serde_json::json!({"index": index, "area": snapshot_rect(*area)}))
            .collect::<Vec<_>>();
        Ok(snapshot_node(
            "health",
            [
                ("issues", snapshot_list_picker(issues)),
                ("summary_scroll", snapshot_scroll(summary_scroll)),
                ("summary_area", snapshot_rect(*summary_area)),
                ("summary_height", serde_json::json!(summary_height)),
                ("issue_height", serde_json::json!(issue_height)),
                ("clicks", health_clicks_snapshot(clicks)),
                (
                    "click",
                    click
                        .pressed()
                        .map(health_hit_snapshot)
                        .unwrap_or(serde_json::Value::Null),
                ),
                ("issue_areas", serde_json::json!(issue_areas)),
                (
                    "footer",
                    snapshot_value("health.footer", &footer.agent_review_snapshot()?)?,
                ),
            ],
        ))
    }

    pub(crate) fn advertised(&self) -> &[(Rect, LocalKey, HealthAction)] {
        self.footer.advertised()
    }

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
                        "Malformed agent (runner) rows in config: {} — fix them in Preferences",
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
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let content_height = paragraph.line_count(area.width.max(1));
        self.summary_scroll
            .set_lines(vec![String::new(); content_height]);
        crate::viewport::Viewport::new(area, content_height).clamp_scroll(&mut self.summary_scroll);
        frame.render_widget(
            paragraph.scroll((
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
        if cancels_pointer_press(&event) {
            self.cancel_click();
        }
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if let Some(action) = self.footer.handle_key(&key) {
                    return HealthEventHandling::Action(action);
                }
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
            Event::Mouse(mouse)
                if matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_)) =>
            {
                match self.footer.handle_mouse(&mouse) {
                    ActionFooterMouse::Action(action) => {
                        self.click.cancel();
                        return HealthEventHandling::Action(action);
                    }
                    ActionFooterMouse::Armed => {
                        self.click.cancel();
                        return HealthEventHandling::Consumed;
                    }
                    ActionFooterMouse::Scrolled | ActionFooterMouse::Ignored => {}
                }
                let target = self.clicks.handle_click(mouse.column, mouse.row);
                match self.click.update(&mouse, target) {
                    ClickOutcome::Armed => HealthEventHandling::Consumed,
                    ClickOutcome::Activated(HealthHit::Issue(index)) => {
                        HealthEventHandling::Action(HealthAction::ActivateIssue(index))
                    }
                    ClickOutcome::Ignored => HealthEventHandling::Ignored,
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
                    self.click.cancel();
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

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
enum RunnerEditorHit {
    Field(RunnerEditorField),
}

/// Reusable mature input session for new, edit, and raw-repair runner flows.
#[derive(Clone, Debug, Default)]
pub(crate) struct RunnerEditorSession {
    name: LineInput,
    command: LineInput,
    signature: Option<(String, String, bool)>,
    clicks: ClickRegionRegistry<RunnerEditorHit>,
    click: ClickTracker<RunnerEditorHit>,
    name_editable: Option<EditableGeometry>,
    command_editable: Option<EditableGeometry>,
    footer: ActionFooterSession<RunnerEditorAction>,
}

impl RunnerEditorSession {
    /// Cancel armed field and footer targets before a pointer discontinuity.
    pub(crate) fn cancel_click(&mut self) {
        self.click.cancel();
        self.footer.cancel_click();
    }

    #[cfg(test)]
    pub(crate) fn perturb_agent_review_state(&mut self) {
        self.signature = Some(("agent-review".to_owned(), String::new(), false));
    }

    pub(crate) fn agent_review_snapshot(
        &self,
    ) -> Result<AgentReviewNode, AgentReviewSnapshotError> {
        let Self {
            name,
            command,
            signature,
            clicks,
            click,
            name_editable,
            command_editable,
            footer,
        } = self;
        Ok(snapshot_node(
            "runner_editor",
            [
                ("name", snapshot_value("runner_editor.name", name)?),
                ("command", snapshot_value("runner_editor.command", command)?),
                ("signature", serde_json::json!(signature)),
                ("clicks", runner_editor_clicks_snapshot(clicks)?),
                (
                    "click",
                    click
                        .pressed()
                        .map(runner_editor_hit_snapshot)
                        .transpose()?
                        .unwrap_or(serde_json::Value::Null),
                ),
                (
                    "name_editable",
                    name_editable
                        .map(editable_geometry_snapshot)
                        .unwrap_or(serde_json::Value::Null),
                ),
                (
                    "command_editable",
                    command_editable
                        .map(editable_geometry_snapshot)
                        .unwrap_or(serde_json::Value::Null),
                ),
                (
                    "footer",
                    snapshot_value("runner_editor.footer", &footer.agent_review_snapshot()?)?,
                ),
            ],
        ))
    }

    pub(crate) fn advertised(&self) -> &[(Rect, LocalKey, RunnerEditorAction)] {
        self.footer.advertised()
    }

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
        let hint_content = Paragraph::new(text(locale, "{{prompt}} marks where the prompt text goes. Each word becomes one argument — quotes group words, and no shell is involved."))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::DarkGray));
        let hint_height =
            u16::try_from(hint_content.line_count(expected_inner_width)).unwrap_or(u16::MAX);
        let panel = centered(
            area,
            72,
            10_u16
                .saturating_add(hint_height)
                .saturating_add(footer_height),
        );
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
            Constraint::Length(hint_height),
            Constraint::Min(1),
            Constraint::Length(footer_height.min(inner.height)),
        ])
        .areas(inner);
        let name_label = if view.name_is_locked() {
            format!("🔒 {}", text(locale, "Name"))
        } else {
            text(locale, "Name, e.g. aider").into_owned()
        };
        self.name_editable = render_line_input(
            frame,
            name,
            &self.name,
            false,
            view.focused() == RunnerEditorField::Name,
            &name_label,
        );
        self.command_editable = render_line_input(
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
        frame.render_widget(hint_content, hint);
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
        if cancels_pointer_press(&event) {
            self.cancel_click();
        }
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if let Some(action) = self.footer.handle_key(&key) {
                    return RunnerEditorEventHandling::Action(action);
                }
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
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Down(_) | MouseEventKind::Up(_) => {
                    match self.footer.handle_mouse(&mouse) {
                        ActionFooterMouse::Action(action) => {
                            self.click.cancel();
                            return RunnerEditorEventHandling::Action(action);
                        }
                        ActionFooterMouse::Armed => {
                            self.click.cancel();
                            return RunnerEditorEventHandling::Consumed;
                        }
                        ActionFooterMouse::Scrolled | ActionFooterMouse::Ignored => {}
                    }
                    let target = self.clicks.handle_click(mouse.column, mouse.row);
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                        && let Some(RunnerEditorHit::Field(field)) = target
                    {
                        let editable = match field {
                            RunnerEditorField::Name => self.name_editable,
                            RunnerEditorField::Command => self.command_editable,
                        };
                        let input = match field {
                            RunnerEditorField::Name => &mut self.name,
                            RunnerEditorField::Command => &mut self.command,
                        };
                        if let Some(editable) = editable {
                            let _ = editable.place_cursor(input, mouse.column, mouse.row);
                        }
                    }
                    match self.click.update(&mouse, target) {
                        ClickOutcome::Armed => RunnerEditorEventHandling::Consumed,
                        ClickOutcome::Activated(RunnerEditorHit::Field(field)) => {
                            RunnerEditorEventHandling::Action(RunnerEditorAction::Focus(field))
                        }
                        ClickOutcome::Ignored => RunnerEditorEventHandling::Ignored,
                    }
                }
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    if matches!(
                        self.footer.handle_mouse(&mouse),
                        ActionFooterMouse::Scrolled
                    ) {
                        self.click.cancel();
                        RunnerEditorEventHandling::Consumed
                    } else {
                        RunnerEditorEventHandling::Ignored
                    }
                }
                MouseEventKind::Moved
                | MouseEventKind::Drag(_)
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight => RunnerEditorEventHandling::Ignored,
            },
            Event::FocusGained | Event::FocusLost | Event::Key(_) | Event::Resize(_, _) => {
                RunnerEditorEventHandling::Ignored
            }
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
    }
}

fn editable_geometry_snapshot(geometry: EditableGeometry) -> serde_json::Value {
    serde_json::json!({
        "content": snapshot_rect(geometry.content()),
        "visual_scroll": geometry.visual_scroll(),
        "secret": geometry.secret(),
    })
}

fn health_hit_snapshot(hit: &HealthHit) -> serde_json::Value {
    let HealthHit::Issue(index) = hit;
    serde_json::json!({"issue": index})
}

fn health_clicks_snapshot(clicks: &ClickRegionRegistry<HealthHit>) -> serde_json::Value {
    serde_json::Value::Array(
        clicks
            .regions()
            .iter()
            .map(|region| {
                serde_json::json!({
                    "area": snapshot_rect(region.area),
                    "target": health_hit_snapshot(&region.data),
                })
            })
            .collect(),
    )
}

fn runner_editor_hit_snapshot(
    hit: &RunnerEditorHit,
) -> Result<serde_json::Value, AgentReviewSnapshotError> {
    let RunnerEditorHit::Field(field) = hit;
    Ok(serde_json::json!({
        "field": snapshot_value("runner_editor.hit.field", field)?,
    }))
}

fn runner_editor_clicks_snapshot(
    clicks: &ClickRegionRegistry<RunnerEditorHit>,
) -> Result<serde_json::Value, AgentReviewSnapshotError> {
    clicks
        .regions()
        .iter()
        .map(|region| {
            Ok(serde_json::json!({
                "area": snapshot_rect(region.area),
                "target": runner_editor_hit_snapshot(&region.data)?,
            }))
        })
        .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()
        .map(serde_json::Value::Array)
}

#[cfg(test)]
mod agent_review_tests {
    use super::*;

    #[test]
    fn management_snapshot_covers_every_owned_hit_and_nested_editor_shape() {
        let mut health_clicks = ClickRegionRegistry::new();
        health_clicks.register(Rect::new(0, 0, 1, 1), HealthHit::Issue(2));
        assert_eq!(
            health_clicks_snapshot(&health_clicks)[0]["target"]["issue"],
            2
        );
        let press = ratatui_crossterm::crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let mut health = HealthScreenSession {
            clicks: health_clicks,
            ..HealthScreenSession::default()
        };
        assert_eq!(
            health.click.update(&press, Some(&HealthHit::Issue(2))),
            ClickOutcome::Armed,
        );
        let health_json = serde_json::to_string(&health.agent_review_snapshot().unwrap()).unwrap();
        assert!(health_json.contains("\"click\""));

        let mut editor_clicks = ClickRegionRegistry::new();
        editor_clicks.register(
            Rect::new(0, 0, 1, 1),
            RunnerEditorHit::Field(RunnerEditorField::Command),
        );
        assert_eq!(
            runner_editor_clicks_snapshot(&editor_clicks).unwrap()[0]["target"]["field"],
            "command"
        );

        let mut editor = RunnerEditorSession {
            name: LineInput::new("name".to_owned()),
            command: LineInput::new("command".to_owned()),
            signature: Some(("name".to_owned(), "command".to_owned(), true)),
            clicks: editor_clicks,
            name_editable: Some(EditableGeometry::new(Rect::new(1, 1, 4, 1), 2, false)),
            command_editable: Some(EditableGeometry::new(Rect::new(1, 2, 4, 1), 1, true)),
            ..RunnerEditorSession::default()
        };
        assert_eq!(
            editor.click.update(
                &press,
                Some(&RunnerEditorHit::Field(RunnerEditorField::Command)),
            ),
            ClickOutcome::Armed,
        );
        let json = serde_json::to_string(&editor.agent_review_snapshot().unwrap()).unwrap();
        assert!(json.contains("name_editable"));
        assert!(json.contains("command_editable"));
        assert!(json.contains("\"click\""));
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
        RunnerEditorError::NameTaken => "Another row already uses this runner name.",
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
        ActionFooterItem::new(
            LocalKey::Enter,
            text(locale, "Jump to entry"),
            HealthAction::Jump,
        ),
        ActionFooterItem::new(
            LocalKey::Control('r'),
            text(locale, "Rebuild index"),
            HealthAction::Rebuild,
        ),
        ActionFooterItem::new(LocalKey::Escape, text(locale, "Back"), HealthAction::Back),
    ]
}

pub(crate) fn runner_editor_footer_items(
    locale: Locale,
) -> Vec<ActionFooterItem<RunnerEditorAction>> {
    vec![
        ActionFooterItem::new_group(
            LocalKey::Enter,
            text(locale, "Save"),
            RunnerEditorAction::Submit,
        ),
        ActionFooterItem::new(
            LocalKey::Escape,
            text(locale, "Cancel"),
            RunnerEditorAction::Cancel,
        ),
        ActionFooterItem::new(
            LocalKey::NextField,
            text(locale, "Next field"),
            RunnerEditorAction::FocusNext,
        ),
        ActionFooterItem::new(
            LocalKey::PreviousField,
            text(locale, "Previous field"),
            RunnerEditorAction::FocusPrevious,
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
    use unicode_width::UnicodeWidthStr as _;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn advertised_key(hint: &str) -> Event {
        let (code, modifiers) = [
            ("Enter", KeyCode::Enter, KeyModifiers::NONE),
            ("Esc", KeyCode::Esc, KeyModifiers::NONE),
            ("Tab/↓", KeyCode::Tab, KeyModifiers::NONE),
            ("Shift+Tab/↑", KeyCode::BackTab, KeyModifiers::SHIFT),
            ("Ctrl+N", KeyCode::Char('n'), KeyModifiers::CONTROL),
            ("Ctrl+R", KeyCode::Char('r'), KeyModifiers::CONTROL),
            ("e", KeyCode::Char('e'), KeyModifiers::NONE),
            ("d", KeyCode::Char('d'), KeyModifiers::NONE),
            ("y", KeyCode::Char('y'), KeyModifiers::NONE),
        ]
        .into_iter()
        .find_map(|(candidate, code, modifiers)| (candidate == hint).then_some((code, modifiers)))
        .unwrap();
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

    fn health_click(
        session: &mut HealthScreenSession,
        view: &HealthView,
        column: u16,
        row: u16,
    ) -> HealthEventHandling {
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::Down(MouseButton::Left), column, row),
                view
            ),
            HealthEventHandling::Consumed,
        );
        session.handle_event(
            mouse_event(MouseEventKind::Up(MouseButton::Left), column, row),
            view,
        )
    }

    fn editor_click(
        session: &mut RunnerEditorSession,
        view: &RunnerEditorView,
        column: u16,
        row: u16,
    ) -> RunnerEditorEventHandling {
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::Down(MouseButton::Left), column, row),
                view
            ),
            RunnerEditorEventHandling::Consumed,
        );
        session.handle_event(
            mouse_event(MouseEventKind::Up(MouseButton::Left), column, row),
            view,
        )
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

    fn row_text(buffer: &ratatui_core::buffer::Buffer, area: Rect) -> String {
        let mut rendered = String::new();
        for y in area.y..area.bottom() {
            let mut x = area.x;
            while x < area.right() {
                let symbol = buffer[(x, y)].symbol();
                rendered.push_str(symbol);
                x = x.saturating_add(
                    u16::try_from(symbol.width().max(1)).expect("one cell symbol fits a row"),
                );
            }
            rendered.push('\n');
        }
        rendered
    }

    fn styled_area(buffer: &ratatui_core::buffer::Buffer, foreground: Color) -> Option<Rect> {
        let cells = (0..buffer.area.height).flat_map(|y| {
            (0..buffer.area.width)
                .filter(move |&x| buffer[(x, y)].fg == foreground)
                .map(move |x| (x, y))
        });
        let (minimum_x, minimum_y, maximum_x, maximum_y) =
            cells.fold(None, |bounds: Option<(u16, u16, u16, u16)>, (x, y)| {
                Some(bounds.map_or((x, y, x, y), |(min_x, min_y, max_x, max_y)| {
                    (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                }))
            })?;
        Some(Rect::new(
            minimum_x,
            minimum_y,
            maximum_x.saturating_sub(minimum_x).saturating_add(1),
            maximum_y.saturating_sub(minimum_y).saturating_add(1),
        ))
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
                let hint = item.advertised_key();
                assert_eq!(
                    health_session.handle_event(advertised_key(&hint), &health_view),
                    HealthEventHandling::Action(item.typed_action().clone()),
                    "Health key {hint} at {width}x{height}",
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
                let hint = item.advertised_key();
                assert_eq!(
                    editor_session.handle_event(advertised_key(&hint), &editor_view),
                    RunnerEditorEventHandling::Action(item.typed_action().clone()),
                    "runner editor key {hint} at {width}x{height}",
                );
            }
        }
    }

    #[test]
    fn management_keyboard_contracts_work_before_the_first_frame() {
        let health_view = health();
        let mut health_session = HealthScreenSession::default();
        assert_eq!(
            health_session.handle_event(key(KeyCode::Enter), &health_view),
            HealthEventHandling::Action(HealthAction::Jump)
        );

        let editor_view = RunnerEditorView::new();
        let mut editor_session = RunnerEditorSession::default();
        assert_eq!(
            editor_session.handle_event(key(KeyCode::Enter), &editor_view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::Submit)
        );
        assert_eq!(
            editor_session.handle_event(key(KeyCode::Tab), &editor_view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::FocusNext)
        );
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
            session.handle_event(
                mouse_event(MouseEventKind::Down(MouseButton::Left), area.x, area.y),
                &view,
            ),
            HealthEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::Up(MouseButton::Left), area.x, area.y),
                &view,
            ),
            HealthEventHandling::Action(HealthAction::ActivateIssue(1))
        );
        for (needle, expected) in [
            ("Enter Jump to entry", HealthAction::Jump),
            ("Ctrl+R Rebuild index", HealthAction::Rebuild),
            ("Esc Back", HealthAction::Back),
        ] {
            let (x, y) = text_position(terminal.backend().buffer(), needle);
            assert_eq!(
                session.handle_event(
                    mouse_event(MouseEventKind::Down(MouseButton::Left), x, y),
                    &view,
                ),
                HealthEventHandling::Consumed
            );
            assert_eq!(
                session.handle_event(
                    mouse_event(MouseEventKind::Up(MouseButton::Left), x, y),
                    &view,
                ),
                HealthEventHandling::Action(expected),
                "visible Health chip must be clickable: {needle}"
            );
        }
    }

    #[test]
    fn health_footer_press_cancels_an_armed_issue_row() {
        let view = health();
        let mut session = HealthScreenSession::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 25)).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        let issue = session.issue_areas[0].1;
        let footer = text_position(terminal.backend().buffer(), "Enter Jump to entry");

        assert_eq!(
            session.handle_event(mouse(issue.x, issue.y), &view),
            HealthEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(mouse(footer.0, footer.1), &view),
            HealthEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::Up(MouseButton::Left), footer.0, footer.1),
                &view,
            ),
            HealthEventHandling::Action(HealthAction::Jump)
        );
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::Up(MouseButton::Left), issue.x, issue.y),
                &view,
            ),
            HealthEventHandling::Ignored,
            "the earlier issue press stayed armed after the footer completed a click"
        );
    }

    #[test]
    fn management_owners_cancel_armed_targets_before_a_late_release() {
        let health_view = health();
        let mut health_session = HealthScreenSession::default();
        let mut health_terminal = Terminal::new(TestBackend::new(100, 25)).unwrap();
        health_terminal
            .draw(|frame| {
                health_session.render(frame, frame.area(), &health_view, Locale::En);
            })
            .unwrap();
        let first_issue = health_session.issue_areas[0].1;
        let second_issue = health_session.issue_areas[1].1;
        assert_eq!(
            health_session.handle_event(mouse(first_issue.x, first_issue.y), &health_view),
            HealthEventHandling::Consumed
        );
        assert_eq!(
            health_session.handle_event(
                mouse_event(
                    MouseEventKind::Drag(MouseButton::Left),
                    second_issue.x,
                    second_issue.y,
                ),
                &health_view,
            ),
            HealthEventHandling::Ignored
        );
        assert_eq!(
            health_session.handle_event(
                mouse_event(
                    MouseEventKind::Up(MouseButton::Left),
                    first_issue.x,
                    first_issue.y,
                ),
                &health_view,
            ),
            HealthEventHandling::Ignored,
            "a Health issue activated after a drag cancelled its press"
        );

        let health_footer =
            text_position(health_terminal.backend().buffer(), "Enter Jump to entry");
        assert_eq!(
            health_session.handle_event(mouse(health_footer.0, health_footer.1), &health_view),
            HealthEventHandling::Consumed
        );
        assert_eq!(
            health_session.handle_event(Event::Resize(80, 18), &health_view),
            HealthEventHandling::Ignored
        );
        assert_eq!(
            health_session.handle_event(
                mouse_event(
                    MouseEventKind::Up(MouseButton::Left),
                    health_footer.0,
                    health_footer.1,
                ),
                &health_view,
            ),
            HealthEventHandling::Ignored,
            "a Health footer action survived a resize"
        );

        let editor_view = RunnerEditorView::new();
        let mut editor_session = RunnerEditorSession::default();
        let mut editor_terminal = Terminal::new(TestBackend::new(90, 20)).unwrap();
        editor_terminal
            .draw(|frame| {
                editor_session.render(frame, frame.area(), &editor_view, Locale::En);
            })
            .unwrap();
        let editor_footer = text_position(editor_terminal.backend().buffer(), "Enter Save");
        assert_eq!(
            editor_session.handle_event(mouse(editor_footer.0, editor_footer.1), &editor_view,),
            RunnerEditorEventHandling::Consumed
        );
        assert_eq!(
            editor_session.handle_event(
                mouse_event(
                    MouseEventKind::Drag(MouseButton::Left),
                    editor_footer.0,
                    editor_footer.1,
                ),
                &editor_view,
            ),
            RunnerEditorEventHandling::Ignored
        );
        assert_eq!(
            editor_session.handle_event(
                mouse_event(
                    MouseEventKind::Up(MouseButton::Left),
                    editor_footer.0,
                    editor_footer.1,
                ),
                &editor_view,
            ),
            RunnerEditorEventHandling::Ignored,
            "a RunnerEditor footer action survived a drag"
        );
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
            health_click(&mut session, &view, x, y),
            HealthEventHandling::Action(HealthAction::Jump)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Down), &view),
            HealthEventHandling::Consumed
        );
        assert!(session.summary_scroll.scroll_offset() > 0);
    }

    #[test]
    fn health_issues_reject_mismatch_and_nonprimary_activation() {
        let view = health();
        let mut session = HealthScreenSession::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 25)).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        let first = session.issue_areas[0].1;
        let second = session.issue_areas[1].1;
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::Down(MouseButton::Right), first.x, first.y),
                &view,
            ),
            HealthEventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::Up(MouseButton::Left), first.x, first.y),
                &view,
            ),
            HealthEventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::Down(MouseButton::Left), first.x, first.y),
                &view,
            ),
            HealthEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                mouse_event(MouseEventKind::Up(MouseButton::Left), second.x, second.y),
                &view,
            ),
            HealthEventHandling::Ignored
        );
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
                editor_click(&mut session, &view, x, y),
                RunnerEditorEventHandling::Action(expected),
                "visible editor chip must be clickable: {needle}"
            );
        }
        let save = text_position(terminal.backend().buffer(), "Enter Save");
        assert_eq!(
            editor_click(&mut session, &view, save.0, save.1),
            RunnerEditorEventHandling::Action(RunnerEditorAction::Submit)
        );
        let cancel = text_position(terminal.backend().buffer(), "Esc Cancel");
        assert_eq!(
            editor_click(&mut session, &view, cancel.0, cancel.1),
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
    fn runner_editor_keeps_the_complete_localized_hint_when_space_is_available() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
            for width in [40, 72, 101] {
                let mut view = RunnerEditorView::new();
                view.reduce(RunnerEditorAction::Submit);
                let mut session = RunnerEditorSession::default();
                let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
                terminal
                    .draw(|frame| session.render(frame, frame.area(), &view, locale))
                    .unwrap();
                let screen = lines(terminal.backend().buffer());
                let compact: String = screen
                    .chars()
                    .filter(|c| !c.is_whitespace() && *c != '│')
                    .collect();
                let hint: String = text(locale, "{{prompt}} marks where the prompt text goes. Each word becomes one argument — quotes group words, and no shell is involved.")
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                assert!(compact.contains(&hint), "{locale:?} {width}: {screen}");
                let error: String = runner_editor_error(&RunnerEditorError::NameRequired, locale)
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                assert!(compact.contains(&error), "{locale:?} {width}: {screen}");
                for action in [RunnerEditorAction::Submit, RunnerEditorAction::Cancel] {
                    assert!(session.advertised().iter().any(|(rect, _, advertised)| {
                        !rect.is_empty() && advertised == &action
                    }));
                }
            }
        }
    }

    #[test]
    fn critical_runner_editor_actions_start_visible_in_every_review_profile() {
        for (locale, width, height) in [
            (Locale::En, 80, 24),
            (Locale::ZhCn, 120, 30),
            (Locale::ZhTw, 40, 40),
            (Locale::Pseudo, 120, 12),
        ] {
            let view = RunnerEditorView::new();
            let mut session = RunnerEditorSession::default();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| session.render(frame, frame.area(), &view, locale))
                .unwrap();
            for action in [RunnerEditorAction::Submit, RunnerEditorAction::Cancel] {
                assert!(
                    session
                        .advertised()
                        .iter()
                        .any(|(rect, _, advertised)| !rect.is_empty() && advertised == &action),
                    "{locale:?} {width}x{height} does not show {action:?}"
                );
            }
        }
    }

    #[test]
    fn runner_editor_line_input_click_places_each_caret_before_typing() {
        let mut view = RunnerEditorView::new();
        view.reduce(RunnerEditorAction::SetName("abcdef".to_owned()));
        let mut session = RunnerEditorSession::default();
        let mut terminal = Terminal::new(TestBackend::new(90, 20)).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        let (x, y) = text_position(terminal.backend().buffer(), "abcdef");
        let _ = editor_click(&mut session, &view, x.saturating_add(2), y);
        assert_eq!(
            session.handle_event(key(KeyCode::Char('X')), &view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::SetName("abXcdef".to_owned()))
        );

        view.reduce(RunnerEditorAction::SetName("name".to_owned()));
        view.reduce(RunnerEditorAction::SetCommand("abcdef".to_owned()));
        view.reduce(RunnerEditorAction::Focus(RunnerEditorField::Command));
        terminal
            .draw(|frame| session.render(frame, frame.area(), &view, Locale::En))
            .unwrap();
        let (x, y) = text_position(terminal.backend().buffer(), "abcdef");
        let _ = editor_click(&mut session, &view, x.saturating_add(2), y);
        assert_eq!(
            session.handle_event(key(KeyCode::Char('X')), &view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::SetCommand("abXcdef".to_owned()))
        );
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
    fn management_widgets_render_safely_in_a_tiny_terminal() {
        let health = health();
        let mut health_session = HealthScreenSession::default();
        let backend = TestBackend::new(24, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| health_session.render(frame, frame.area(), &health, Locale::En))
            .unwrap();

        let editor = RunnerEditorView::edit(&row(0, None, 2));
        let mut editor_session = RunnerEditorSession::default();
        terminal
            .draw(|frame| editor_session.render(frame, frame.area(), &editor, Locale::En))
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
    fn every_runner_editor_error_renders_exact_localized_red_text() {
        let cases = [
            (
                "",
                "agent {{prompt}}",
                RunnerEditorError::NameRequired,
                "A name is required.",
            ),
            (
                "name",
                "\"",
                RunnerEditorError::UnbalancedQuotes,
                "Unbalanced quotes in the command.",
            ),
            (
                "name",
                "",
                RunnerEditorError::EmptyCommand,
                "Type the agent's command, e.g. mycli run {{prompt}}",
            ),
            (
                "name",
                "agent",
                RunnerEditorError::PromptSlotCount,
                "The command needs the {{prompt}} slot exactly once — that's where the rendered prompt lands.",
            ),
            (
                "name",
                "{{prompt}}",
                RunnerEditorError::PromptInProgram,
                "{{prompt}} can't be the command itself — the first word must be the program to run.",
            ),
            (
                "name",
                "agent {{other}} {{prompt}}",
                RunnerEditorError::UnsupportedHole,
                "Runner commands take only the {{prompt}} slot — single-brace text is literal, and other {{holes}} aren't supported.",
            ),
        ];
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            for (name, command, error, source) in &cases {
                let mut view = RunnerEditorView::new();
                view.reduce(RunnerEditorAction::SetName((*name).to_owned()));
                view.reduce(RunnerEditorAction::SetCommand((*command).to_owned()));
                view.reduce(RunnerEditorAction::Submit);
                assert_eq!(view.error(), Some(error));
                assert_eq!(view.host_error(), None);

                let mut session = RunnerEditorSession::default();
                let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
                terminal
                    .draw(|frame| session.render(frame, frame.area(), &view, locale))
                    .unwrap();
                let expected = text(locale, source);
                let red_area = styled_area(terminal.backend().buffer(), Color::Red)
                    .expect("a validation error owns a visible red band");
                let actual = row_text(terminal.backend().buffer(), red_area);
                let mut expected_terminal =
                    Terminal::new(TestBackend::new(red_area.width, red_area.height)).unwrap();
                expected_terminal
                    .draw(|frame| {
                        frame.render_widget(
                            Paragraph::new(expected.as_ref())
                                .wrap(Wrap { trim: false })
                                .style(Style::default().fg(Color::Red)),
                            frame.area(),
                        );
                    })
                    .unwrap();
                let expected_grid = row_text(
                    expected_terminal.backend().buffer(),
                    expected_terminal.backend().buffer().area,
                );
                assert_eq!(
                    actual, expected_grid,
                    "error={error:?}, locale={locale:?}, expected={expected:?}"
                );
            }
        }
    }

    /// The duplicate-name refusal is a typed editor error, so the frontend localizes it.
    #[test]
    fn a_duplicate_agent_name_refusal_localizes_at_render_time() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            assert_eq!(
                runner_editor_error(&RunnerEditorError::NameTaken, locale),
                text(locale, "Another row already uses this runner name.").into_owned()
            );
        }
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
            editor_click(&mut editor_session, &editor_view, name.0, name.1),
            RunnerEditorEventHandling::Action(RunnerEditorAction::Focus(RunnerEditorField::Name))
        );
        editor_view.reduce(RunnerEditorAction::Focus(RunnerEditorField::Command));
        assert!(matches!(
            editor_session.handle_event(Event::Paste("cmd".to_owned()), &editor_view),
            RunnerEditorEventHandling::Action(RunnerEditorAction::SetCommand(_))
        ));
    }
}
