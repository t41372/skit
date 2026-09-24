//! Help and confirmation overlays.

use ratatui_core::{
    buffer::Buffer,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui_interact::components::{
    Button, ButtonState, ButtonVariant, DialogConfig, DialogFocusTarget, DialogState, PopupDialog,
    ScrollableContentState, handle_scrollable_content_key, handle_scrollable_content_mouse,
};
use ratatui_interact::traits::{ContainerAction, EventResult};
use ratatui_widgets::{
    block::Block,
    borders::Borders,
    clear::Clear,
    paragraph::{Paragraph, Wrap},
};
use skit_i18n::{Locale, text};
use skit_ui::{CommandContext, UiCommand, command_specs};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    EventHandling, HitRegion, HitTarget, ViewGeometry,
    agent_review::{
        AgentReviewNode, color as snapshot_color, node as snapshot_node, rect as snapshot_rect,
        scroll as snapshot_scroll,
    },
    pointer::{ClickOutcome, ClickTracker},
    theme::{self, Panel, padded_panel},
};

/// Result of one mature confirmation-dialog event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfirmRemoveEvent {
    Submit,
    Close,
    Consumed,
    Ignored,
}

fn compact_dialog_button_areas(
    inner: Rect,
    buttons: &[(String, ContainerAction)],
) -> Vec<(usize, Rect)> {
    let widths = buttons
        .iter()
        .map(|(label, _)| u16::try_from(label.width().saturating_add(2)).unwrap_or(u16::MAX))
        .collect::<Vec<_>>();
    dialog_button_areas(inner, &widths, true)
}

/// Lay out one row of dialog buttons at the bottom of `inner`, or stack them when the row does
/// not fit. A row is centered or starts at the left edge.
fn dialog_button_areas(inner: Rect, widths: &[u16], centered: bool) -> Vec<(usize, Rect)> {
    if inner.is_empty() || widths.is_empty() {
        return Vec::new();
    }
    let widths = widths
        .iter()
        .map(|width| (*width).min(inner.width))
        .collect::<Vec<_>>();
    let gaps = u16::try_from(widths.len().saturating_sub(1)).unwrap_or(u16::MAX);
    let horizontal_width = widths
        .iter()
        .copied()
        .fold(0_u16, u16::saturating_add)
        .saturating_add(gaps);
    if horizontal_width <= inner.width {
        let mut x = if centered {
            inner
                .x
                .saturating_add(inner.width.saturating_sub(horizontal_width) / 2)
        } else {
            inner.x
        };
        let y = inner.bottom().saturating_sub(1);
        return widths
            .into_iter()
            .enumerate()
            .map(|(index, width)| {
                let area = Rect::new(x, y, width, 1);
                x = x.saturating_add(width).saturating_add(1);
                (index, area)
            })
            .collect();
    }
    let needed_height = u16::try_from(widths.len()).unwrap_or(u16::MAX);
    if needed_height > inner.height {
        return Vec::new();
    }
    let first_y = inner.bottom().saturating_sub(needed_height);
    widths
        .into_iter()
        .enumerate()
        .map(|(index, width)| {
            let x = inner
                .x
                .saturating_add(inner.width.saturating_sub(width) / 2);
            let y = first_y.saturating_add(u16::try_from(index).unwrap_or(u16::MAX));
            (index, Rect::new(x, y, width, 1))
        })
        .collect()
}

/// One action of a confirmation dialog, drawn as a footer chip: its key, then its label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DialogChip {
    pub(crate) key: &'static str,
    pub(crate) label: String,
    pub(crate) command: UiCommand,
}

impl DialogChip {
    /// The painted text without its padding: `y Remove`.
    fn text(&self) -> String {
        format!("{} {}", self.key, self.label)
    }

    /// The cells the chip paints, padding included: ` y Remove `.
    fn width(&self) -> u16 {
        u16::try_from(self.text().width().saturating_add(2)).unwrap_or(u16::MAX)
    }
}

/// The actions of one confirmation dialog, in their visible order.
///
/// The command registry is the one source of each key and label, so a chip always names a key
/// that does what the chip says. Version 0.4 draws the same chips (`src/skit/tui.py:160-165`,
/// `src/skit/tui_settings.py:58-63`).
pub(crate) fn dialog_chips(context: CommandContext, locale: Locale) -> Vec<DialogChip> {
    command_specs(context)
        .filter(|spec| spec.footer)
        .filter_map(|spec| {
            spec.bindings.first().map(|binding| DialogChip {
                key: binding.hint,
                label: text(locale, spec.label).into_owned(),
                command: spec.command,
            })
        })
        .collect()
}

/// Draw one dialog action as a chip: the key as a keycap, then the label.
///
/// The focused chip is the focus ring: it is reversed in the terminal theme and filled with the
/// accent in the `skit` theme, so the ring shows without a color too.
fn render_dialog_chip(buffer: &mut Buffer, area: Rect, chip: &DialogChip, focused: bool) {
    let mut state = ButtonState::enabled();
    state.set_focused(focused);
    let region = Button::new(&chip.label, &state)
        .icon(chip.key)
        .variant(ButtonVariant::SingleLine)
        .style(theme::dialog_chip_style())
        .render_stateful(area, buffer);
    theme::patch_chip_key(buffer, area, chip.key);
    theme::patch_focus(buffer, region.area, focused);
}

/// Persistent mature dialog state for entry removal.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConfirmRemoveSession {
    dialog: DialogState<()>,
    config: Option<DialogConfig>,
    screen: Rect,
    click: ClickTracker<DialogFocusTarget>,
    identity: Option<(String, bool)>,
}

impl ConfirmRemoveSession {
    #[cfg(test)]
    pub(crate) fn perturb_agent_review_state(&mut self) {
        self.screen.width = self.screen.width.saturating_add(1);
    }

    pub(crate) fn agent_review_snapshot(&self) -> AgentReviewNode {
        let Self {
            dialog,
            config,
            screen,
            click,
            identity,
        } = self;
        let focus_target = |target: &DialogFocusTarget| match target {
            DialogFocusTarget::Child(index) => serde_json::json!({"child": index}),
            DialogFocusTarget::Button(index) => serde_json::json!({"button": index}),
            DialogFocusTarget::Close => serde_json::json!("close"),
        };
        let action = |action: &ContainerAction| match action {
            ContainerAction::Close => serde_json::json!("close"),
            ContainerAction::Submit => serde_json::json!("submit"),
            ContainerAction::Custom(name) => serde_json::json!({"custom": name}),
        };
        let config = config.as_ref().map(|config| {
            serde_json::json!({
                "title": config.title,
                "width_percent": config.width_percent,
                "height_percent": config.height_percent,
                "min_width": config.min_width,
                "min_height": config.min_height,
                "max_width": config.max_width,
                "max_height": config.max_height,
                "border_color": snapshot_color(config.border_color),
                "focused_border_color": snapshot_color(config.focused_border_color),
                "close_on_escape": config.close_on_escape,
                "close_on_outside_click": config.close_on_outside_click,
                "buttons": config.buttons.iter().map(|(label, target)| serde_json::json!({
                    "label": label,
                    "action": action(target),
                })).collect::<Vec<_>>(),
            })
        });
        let dialog = serde_json::json!({
            "children": null,
            "focus": {
                "elements": dialog.focus.elements().iter().map(focus_target).collect::<Vec<_>>(),
                "current_index": dialog.focus.current_index(),
            },
            "click_regions": dialog.click_regions.regions().iter().map(|region| serde_json::json!({
                "area": snapshot_rect(region.area),
                "target": focus_target(&region.data),
            })).collect::<Vec<_>>(),
            "visible": dialog.visible,
        });
        let click = click
            .pressed()
            .map(focus_target)
            .unwrap_or(serde_json::Value::Null);
        snapshot_node(
            "confirm_remove",
            [
                ("dialog", dialog),
                ("config", serde_json::json!(config)),
                ("screen", snapshot_rect(*screen)),
                ("click", click),
                ("identity", serde_json::json!(identity)),
            ],
        )
    }

    /// Put the focus back on the safe button for the next time the dialog opens.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Each visible chip, and whether a primary press holds it.
    pub(crate) fn pointer_targets(&self) -> Vec<(Rect, bool)> {
        self.dialog
            .click_regions
            .regions()
            .iter()
            .map(|region| (region.area, self.click.pressed() == Some(&region.data)))
            .collect()
    }

    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        name: &str,
        original_file_preserved: bool,
        locale: Locale,
    ) -> ViewGeometry {
        let identity = (name.to_owned(), original_file_preserved);
        if self.identity.as_ref() != Some(&identity) {
            self.click.cancel();
            self.identity = Some(identity);
        }
        // The safe button registers first, so the dialog opens with the focus on Keep.
        if self.dialog.focus.is_empty() {
            self.dialog.register_button(1);
            self.dialog.register_button(0);
        }
        self.dialog.show();
        self.screen = frame.area();
        let chips = dialog_chips(CommandContext::ConfirmRemove, locale);
        let title = text(locale, "Confirm removal");
        let config = DialogConfig::new(title.as_ref())
            .width_percent(72)
            .height_percent(38)
            .min_size(34, 7)
            .max_size(90, 12)
            .border_color(theme::panel_color(Panel::Dialog))
            .focused_border_color(theme::panel_color(Panel::Dialog))
            .close_on_outside_click(false)
            .buttons(
                chips
                    .iter()
                    .map(|chip| {
                        let action = if chip.command == UiCommand::Submit {
                            ContainerAction::Submit
                        } else {
                            ContainerAction::Close
                        };
                        (chip.text(), action)
                    })
                    .collect(),
            );
        let popup_area =
            PopupDialog::new(&config, &mut self.dialog, |_, _, ()| {}).calculate_area(frame.area());
        let chips_width = chips
            .iter()
            .map(|chip| chip.width().saturating_add(1))
            .fold(0_u16, u16::saturating_add);
        // A dialog too small for its message and one row of chips takes the whole body and
        // stacks the chips when they do not fit side by side.
        let roomy = popup_area.width >= 30
            && popup_area.height >= 5
            && chips_width <= popup_area.width.saturating_sub(2);
        let dialog_area = if roomy { popup_area } else { area };
        self.dialog.click_regions.clear();
        frame.render_widget(Clear, dialog_area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::panel_border(Panel::Dialog))
            .title(Line::from(format!(" {title} ")).centered())
            .title_style(theme::border_title());
        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);
        // A roomy dialog keeps one blank row under its chips, as the dialog widget lays it out.
        let band = if roomy {
            Rect {
                height: inner.height.saturating_sub(1),
                ..inner
            }
        } else {
            inner
        };
        let button_areas = compact_dialog_button_areas(band, &config.buttons);
        let message_bottom = button_areas
            .iter()
            .map(|(_, area)| area.y)
            .min()
            .unwrap_or_else(|| inner.bottom());
        let message_area = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            message_bottom.saturating_sub(inner.y),
        );
        let mut lines = vec![Line::from(format!(
            "{} {name}?",
            text(locale, "Remove this entry:")
        ))];
        if original_file_preserved && roomy {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                text(locale, "Your original file will not be deleted."),
                theme::muted(),
            )));
        }
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            message_area,
        );
        for (index, button_area) in button_areas {
            render_dialog_chip(
                frame.buffer_mut(),
                button_area,
                &chips[index],
                self.dialog.is_button_focused(index),
            );
            self.dialog
                .click_regions
                .register(button_area, DialogFocusTarget::Button(index));
        }
        self.config = Some(config);
        ViewGeometry::default()
    }

    pub(crate) fn handle_event(&mut self, event: &Event) -> ConfirmRemoveEvent {
        let Some(config) = self.config.as_ref() else {
            return ConfirmRemoveEvent::Ignored;
        };
        if matches!(
            event,
            Event::FocusGained
                | Event::FocusLost
                | Event::Key(_)
                | Event::Paste(_)
                | Event::Resize(_, _)
                | Event::Mouse(ratatui_crossterm::crossterm::event::MouseEvent {
                    kind: MouseEventKind::ScrollUp
                        | MouseEventKind::ScrollDown
                        | MouseEventKind::ScrollLeft
                        | MouseEventKind::ScrollRight,
                    ..
                })
        ) {
            self.click.cancel();
        }
        let result = match event {
            // Tab and Shift+Tab move the focus ring, and Enter activates the button under it.
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && (matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
                        || (key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE)) =>
            {
                let mut popup = PopupDialog::new(config, &mut self.dialog, |_, _, ()| {});
                popup.handle_key(*key)
            }
            Event::Mouse(mouse) => {
                let target = self
                    .dialog
                    .click_regions
                    .handle_click(mouse.column, mouse.row);
                match self.click.update(mouse, target) {
                    ClickOutcome::Armed => EventResult::Consumed,
                    ClickOutcome::Activated(_) => {
                        let mut activation = *mouse;
                        activation.kind = MouseEventKind::Down(MouseButton::Left);
                        let mut popup = PopupDialog::new(config, &mut self.dialog, |_, _, ()| {});
                        popup.handle_mouse_with_screen(activation, self.screen)
                    }
                    ClickOutcome::Ignored => EventResult::NotHandled,
                }
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Key(_)
            | Event::Paste(_)
            | Event::Resize(_, _) => EventResult::NotHandled,
        };
        match result {
            EventResult::Action(ContainerAction::Submit) => ConfirmRemoveEvent::Submit,
            EventResult::Action(ContainerAction::Close) => ConfirmRemoveEvent::Close,
            EventResult::Action(ContainerAction::Custom(_)) | EventResult::Consumed => {
                ConfirmRemoveEvent::Consumed
            }
            EventResult::NotHandled => ConfirmRemoveEvent::Ignored,
        }
    }
}

/// Persistent scroll state for the complete keyboard reminder.
#[derive(Clone, Debug, Default)]
pub(crate) struct HelpScreenSession {
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
}

impl HelpScreenSession {
    #[cfg(test)]
    pub(crate) fn perturb_agent_review_state(&mut self) {
        self.visible_height = self.visible_height.saturating_add(1);
    }

    pub(crate) fn agent_review_snapshot(&self) -> AgentReviewNode {
        let Self {
            scroll,
            viewport,
            visible_height,
        } = self;
        snapshot_node(
            "help",
            [
                ("scroll", snapshot_scroll(scroll)),
                ("viewport", snapshot_rect(*viewport)),
                ("visible_height", serde_json::json!(visible_height)),
            ],
        )
    }

    pub(crate) fn render(&mut self, frame: &mut Frame, area: Rect, locale: Locale) -> ViewGeometry {
        let rows = command_specs(CommandContext::LibraryBrowse)
            .filter(|spec| spec.help)
            .filter_map(|spec| {
                spec.bindings.first().map(|binding| {
                    // Quit names every key that the footer names, then the double interrupt that
                    // works from any screen.
                    let hint = if spec.command == UiCommand::Quit {
                        let keys = spec
                            .bindings
                            .iter()
                            .map(|binding| binding.hint)
                            .collect::<Vec<_>>()
                            .join(" / ");
                        format!("{keys} / Ctrl+C Ctrl+C")
                    } else {
                        binding.hint.to_owned()
                    };
                    (hint, spec.label)
                })
            })
            .collect::<Vec<_>>();
        let hint_width = rows
            .iter()
            .map(|(hint, _)| hint.width())
            .max()
            .unwrap_or_default()
            .max(20);
        let lines = rows
            .into_iter()
            .map(|(hint, label)| {
                Line::from(vec![
                    Span::styled(
                        format!("{hint:>hint_width$}"),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::raw(text(locale, label)),
                ])
            })
            .collect::<Vec<_>>();
        let base_block = Block::default()
            .borders(Borders::ALL)
            .title(text(locale, "Help"));
        self.viewport = base_block.inner(area);
        self.visible_height = usize::from(self.viewport.height);
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let line_count = paragraph.line_count(self.viewport.width);
        self.scroll.set_lines(vec![String::new(); line_count]);
        let maximum = line_count.saturating_sub(self.visible_height);
        self.scroll
            .set_scroll_offset(self.scroll.scroll_offset().min(maximum));
        let indicator = match (
            self.scroll.is_at_top(),
            self.scroll.is_at_bottom(self.visible_height),
        ) {
            (true, true) => "",
            (true, false) => " ↓",
            (false, true) => " ↑",
            (false, false) => " ↑↓",
        };
        frame.render_widget(
            Block::default().borders(Borders::ALL).title(format!(
                "{}{}",
                text(locale, "Help"),
                indicator
            )),
            area,
        );
        frame.render_widget(
            paragraph.scroll((
                u16::try_from(self.scroll.scroll_offset()).unwrap_or(u16::MAX),
                0,
            )),
            self.viewport,
        );
        ViewGeometry {
            rows: self.viewport,
            first_visible: self.scroll.scroll_offset(),
            hits: Vec::new(),
            detail_pane_visible: false,
        }
    }

    pub(crate) fn handle_event(&mut self, event: &Event) -> bool {
        match event {
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && key.modifiers == KeyModifiers::NONE
                    && matches!(
                        key.code,
                        KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::PageUp
                            | KeyCode::PageDown
                            | KeyCode::Home
                            | KeyCode::End
                    ) =>
            {
                handle_scrollable_content_key(&mut self.scroll, key, self.visible_height).is_some()
            }
            Event::Mouse(mouse) => handle_scrollable_content_mouse(
                &mut self.scroll,
                mouse,
                self.viewport,
                self.visible_height,
            )
            .is_some(),
            Event::FocusGained
            | Event::FocusLost
            | Event::Key(_)
            | Event::Paste(_)
            | Event::Resize(_, _) => false,
        }
    }
}

#[cfg(test)]
mod agent_review_tests {
    use super::*;

    #[test]
    fn modal_snapshot_covers_every_dialog_focus_action_and_click_shape() {
        let mut session = ConfirmRemoveSession::default();
        session.dialog.focus.register_all([
            DialogFocusTarget::Child(0),
            DialogFocusTarget::Button(1),
            DialogFocusTarget::Close,
        ]);
        for (index, target) in [
            DialogFocusTarget::Child(0),
            DialogFocusTarget::Button(1),
            DialogFocusTarget::Close,
        ]
        .into_iter()
        .enumerate()
        {
            session
                .dialog
                .click_regions
                .register(Rect::new(index as u16, 0, 1, 1), target);
        }
        session.dialog.visible = true;
        session.config = Some(DialogConfig {
            buttons: vec![
                ("Close".to_owned(), ContainerAction::Close),
                ("Submit".to_owned(), ContainerAction::Submit),
                (
                    "Custom".to_owned(),
                    ContainerAction::Custom("custom".to_owned()),
                ),
            ],
            ..DialogConfig::default()
        });
        session.screen = Rect::new(1, 2, 3, 4);
        session.identity = Some(("entry".to_owned(), true));
        let press = ratatui_crossterm::crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            session
                .click
                .update(&press, Some(&DialogFocusTarget::Button(1))),
            ClickOutcome::Armed,
        );
        let json = serde_json::to_string(&session.agent_review_snapshot()).unwrap();
        for expected in [
            "child", "button", "close", "submit", "custom", "identity", "click",
        ] {
            assert!(json.contains(expected), "missing {expected}");
        }

        let help = HelpScreenSession {
            viewport: Rect::new(1, 2, 3, 4),
            visible_height: 4,
            ..HelpScreenSession::default()
        };
        assert!(
            serde_json::to_string(&help.agent_review_snapshot())
                .unwrap()
                .contains("visible_height")
        );
    }
}
/// The focus ring of the discard guard.
///
/// The guard opens with the focus on Keep editing. A guard exists to catch a reflex, so the
/// answer that a reflexive Enter reaches must keep the work.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConfirmDiscardSession {
    discard_focused: bool,
}

impl ConfirmDiscardSession {
    #[cfg(test)]
    pub(crate) fn perturb_agent_review_state(&mut self) {
        self.discard_focused = !self.discard_focused;
    }

    pub(crate) fn agent_review_snapshot(&self) -> AgentReviewNode {
        let Self { discard_focused } = self;
        snapshot_node(
            "confirm_discard",
            [("discard_focused", serde_json::json!(discard_focused))],
        )
    }

    /// Put the focus back on Keep editing for the next time the guard opens.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Move the focus ring with Tab and Shift+Tab, and activate the focused chip with Enter.
    ///
    /// Every other event belongs to the command registry (`y`, `Esc`, `n`) or to the chips' click
    /// targets.
    pub(crate) fn handle_event(&mut self, event: &Event) -> Option<EventHandling> {
        let Event::Key(key) = event else {
            return None;
        };
        if key.kind == KeyEventKind::Release {
            return None;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Tab, KeyModifiers::NONE)
            | (KeyCode::BackTab, KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                self.discard_focused = !self.discard_focused;
                Some(EventHandling::Consumed)
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                Some(EventHandling::Action(if self.discard_focused {
                    skit_ui::Action::DiscardChanges
                } else {
                    skit_ui::Action::KeepEditing
                }))
            }
            _ => None,
        }
    }

    pub(crate) fn render(&self, frame: &mut Frame, area: Rect, locale: Locale) -> ViewGeometry {
        let [panel] = Layout::vertical([Constraint::Length(7)])
            .flex(Flex::Center)
            .areas(area);
        let [panel] = Layout::horizontal([Constraint::Length(52)])
            .flex(Flex::Center)
            .areas(panel);
        // Version 0.4 shows the question once, inside an untitled border
        // (`src/skit/tui_settings.py:42-65`). The header names the surface; the
        // panel itself must not repeat the body sentence as a title.
        let block = padded_panel(String::new(), Panel::Dialog);
        let inner = block.inner(panel);
        frame.render_widget(block, panel);

        let chips = dialog_chips(CommandContext::ConfirmDiscard, locale);
        let widths = chips.iter().map(DialogChip::width).collect::<Vec<_>>();
        let button_areas = dialog_button_areas(inner, &widths, false);
        let message_bottom = button_areas
            .iter()
            .map(|(_, area)| area.y)
            .min()
            .unwrap_or_else(|| inner.bottom());
        frame.render_widget(
            Paragraph::new(text(locale, "Discard unsaved changes?")),
            Rect::new(
                inner.x,
                inner.y,
                inner.width,
                message_bottom.saturating_sub(inner.y),
            ),
        );
        let mut hits = Vec::with_capacity(button_areas.len());
        for (index, button_area) in button_areas {
            let chip = &chips[index];
            let focused = (chip.command == UiCommand::DiscardChanges) == self.discard_focused;
            render_dialog_chip(frame.buffer_mut(), button_area, chip, focused);
            hits.push(HitRegion {
                rect: button_area,
                action: HitTarget::Command(chip.command),
            });
        }

        ViewGeometry {
            rows: inner,
            first_visible: 0,
            detail_pane_visible: false,
            hits,
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::{KeyEvent, MouseButton, MouseEvent};

    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn mouse(area: Rect, kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn find_text(buffer: &Buffer, needle: &str) -> Rect {
        for row in (0..buffer.area.height).rev() {
            let text = (0..buffer.area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>();
            if let Some(byte) = text.find(needle) {
                let column = text[..byte].chars().count();
                return Rect::new(u16::try_from(column).unwrap_or(u16::MAX), row, 1, 1);
            }
        }
        Rect::default()
    }

    #[test]
    fn confirm_remove_uses_real_dialog_buttons_tabs_and_reverse_events() {
        let mut session = ConfirmRemoveSession::default();
        assert_eq!(
            session.handle_event(&key(KeyCode::Tab)),
            ConfirmRemoveEvent::Ignored
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), "Alpha", true, Locale::En);
            })
            .unwrap();
        let remove = find_text(terminal.backend().buffer(), "Remove");
        let keep = find_text(terminal.backend().buffer(), "Keep");
        assert!(!remove.is_empty());
        assert!(!keep.is_empty());
        assert!(find_text(terminal.backend().buffer(), "not present").is_empty());
        for code in [KeyCode::Tab, KeyCode::BackTab] {
            assert_eq!(
                session.handle_event(&key(code)),
                ConfirmRemoveEvent::Consumed
            );
        }
        assert_eq!(
            session.handle_event(&mouse(remove, MouseEventKind::Down(MouseButton::Left))),
            ConfirmRemoveEvent::Consumed
        );
        assert_eq!(
            session.handle_event(&mouse(remove, MouseEventKind::Up(MouseButton::Left))),
            ConfirmRemoveEvent::Submit
        );
        assert_eq!(
            session.handle_event(&mouse(keep, MouseEventKind::Down(MouseButton::Left))),
            ConfirmRemoveEvent::Consumed
        );
        assert_eq!(
            session.handle_event(&mouse(keep, MouseEventKind::Up(MouseButton::Left))),
            ConfirmRemoveEvent::Close
        );
        for event in [
            mouse(remove, MouseEventKind::Moved),
            key(KeyCode::Enter),
            Event::Paste("ignored".to_owned()),
            Event::FocusGained,
            Event::Resize(40, 10),
        ] {
            assert_eq!(session.handle_event(&event), ConfirmRemoveEvent::Ignored);
        }
        let release = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Tab,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ));
        assert_eq!(session.handle_event(&release), ConfirmRemoveEvent::Ignored);

        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), "Beta", false, Locale::ZhTw);
            })
            .unwrap();
        assert!(!find_text(terminal.backend().buffer(), "Beta").is_empty());
    }

    // Ways this can fail: Enter does nothing, Enter removes while the focus ring is on Keep, the
    // ring does not move, or a key release moves it.
    #[test]
    fn enter_activates_the_chip_under_the_focus_ring_which_opens_on_keep() {
        let draw = |session: &mut ConfirmRemoveSession| {
            let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
            terminal
                .draw(|frame| {
                    let _ = session.render(frame, frame.area(), "Alpha", true, Locale::En);
                })
                .unwrap();
        };
        let mut keep = ConfirmRemoveSession::default();
        draw(&mut keep);
        assert_eq!(
            keep.handle_event(&key(KeyCode::Enter)),
            ConfirmRemoveEvent::Close,
            "the dialog opens with the focus ring on Keep",
        );

        let mut remove = ConfirmRemoveSession::default();
        draw(&mut remove);
        assert_eq!(
            remove.handle_event(&key(KeyCode::Tab)),
            ConfirmRemoveEvent::Consumed
        );
        assert_eq!(
            remove.handle_event(&key(KeyCode::Enter)),
            ConfirmRemoveEvent::Submit
        );

        let mut back = ConfirmRemoveSession::default();
        draw(&mut back);
        assert_eq!(
            back.handle_event(&Event::Key(KeyEvent::new(
                KeyCode::BackTab,
                KeyModifiers::SHIFT
            ))),
            ConfirmRemoveEvent::Consumed
        );
        assert_eq!(
            back.handle_event(&Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::CONTROL
            ))),
            ConfirmRemoveEvent::Ignored,
            "a modified Enter is not the activation key",
        );
        assert_eq!(
            back.handle_event(&key(KeyCode::Enter)),
            ConfirmRemoveEvent::Submit
        );

        // A focus that moved stays behind the dialog until it opens again.
        back.reset();
        draw(&mut back);
        assert_eq!(
            back.handle_event(&key(KeyCode::Enter)),
            ConfirmRemoveEvent::Close
        );
    }

    #[test]
    fn confirm_remove_leaves_non_navigation_keys_and_releases_to_the_root_owner() {
        let mut session = ConfirmRemoveSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), "Alpha", true, Locale::En);
            })
            .unwrap();

        for code in [KeyCode::Char('y'), KeyCode::Esc, KeyCode::Char('n')] {
            assert_eq!(
                session.handle_event(&key(code)),
                ConfirmRemoveEvent::Ignored,
                "{code:?} belongs to the root confirmation command",
            );
        }
        assert_eq!(
            session.handle_event(&Event::Key(KeyEvent::new_with_kind(
                KeyCode::Tab,
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ))),
            ConfirmRemoveEvent::Ignored,
            "a key release must not move dialog focus",
        );
        assert_eq!(
            session.handle_event(&key(KeyCode::Tab)),
            ConfirmRemoveEvent::Consumed,
            "a Tab press must still move dialog focus",
        );
    }

    #[test]
    fn confirm_remove_cancels_a_press_when_another_event_owner_intervenes() {
        let mut session = ConfirmRemoveSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), "Alpha", true, Locale::En);
            })
            .unwrap();
        let remove = find_text(terminal.backend().buffer(), "Remove");

        for (interrupt, expected) in [
            (Event::Resize(40, 10), ConfirmRemoveEvent::Ignored),
            (
                mouse(remove, MouseEventKind::ScrollDown),
                ConfirmRemoveEvent::Ignored,
            ),
            (key(KeyCode::Tab), ConfirmRemoveEvent::Consumed),
        ] {
            assert_eq!(
                session.handle_event(&mouse(remove, MouseEventKind::Down(MouseButton::Left))),
                ConfirmRemoveEvent::Consumed
            );
            assert_eq!(session.handle_event(&interrupt), expected);
            assert_eq!(
                session.handle_event(&mouse(remove, MouseEventKind::Up(MouseButton::Left))),
                ConfirmRemoveEvent::Ignored,
                "{interrupt:?} left a stale Remove press armed",
            );
        }
    }

    #[test]
    fn confirm_remove_cancels_an_armed_button_when_the_entry_identity_changes() {
        let mut session = ConfirmRemoveSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), "Alpha", true, Locale::En);
            })
            .unwrap();
        let remove = find_text(terminal.backend().buffer(), "Remove");
        assert_eq!(
            session.handle_event(&mouse(remove, MouseEventKind::Down(MouseButton::Left))),
            ConfirmRemoveEvent::Consumed
        );
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), "Beta", false, Locale::En);
            })
            .unwrap();
        let replacement = find_text(terminal.backend().buffer(), "Remove");
        assert_eq!(replacement, remove);
        assert_eq!(
            session.handle_event(&mouse(replacement, MouseEventKind::Up(MouseButton::Left))),
            ConfirmRemoveEvent::Ignored
        );

        assert_eq!(
            session.handle_event(&mouse(replacement, MouseEventKind::Down(MouseButton::Left))),
            ConfirmRemoveEvent::Consumed
        );
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), "Beta", false, Locale::En);
            })
            .unwrap();
        assert_eq!(
            session.handle_event(&mouse(replacement, MouseEventKind::Up(MouseButton::Left))),
            ConfirmRemoveEvent::Submit
        );
    }

    #[test]
    fn help_scrolls_by_every_advertised_key_and_wheel_then_clamps_on_growth() {
        let mut session = HelpScreenSession::default();
        let mut terminal = Terminal::new(TestBackend::new(38, 6)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), Locale::ZhCn);
            })
            .unwrap();
        for code in [
            KeyCode::Down,
            KeyCode::PageDown,
            KeyCode::End,
            KeyCode::Up,
            KeyCode::PageUp,
            KeyCode::Home,
        ] {
            assert!(session.handle_event(&key(code)));
        }
        let viewport = session.viewport;
        assert!(session.handle_event(&mouse(viewport, MouseEventKind::ScrollDown)));
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), Locale::ZhCn);
            })
            .unwrap();
        assert!(!find_text(terminal.backend().buffer(), "↑↓").is_empty());
        assert!(session.handle_event(&mouse(viewport, MouseEventKind::ScrollUp)));
        for event in [
            mouse(viewport, MouseEventKind::Moved),
            key(KeyCode::Char('x')),
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL)),
            Event::Paste("ignored".to_owned()),
            Event::FocusLost,
            Event::Resize(80, 20),
        ] {
            assert!(!session.handle_event(&event));
        }
        assert!(session.handle_event(&key(KeyCode::End)));
        terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let geometry = session.render(frame, frame.area(), Locale::En);
                assert_eq!(geometry.first_visible, 0);
            })
            .unwrap();
    }

    #[test]
    fn help_assigns_the_double_interrupt_hint_only_to_the_quit_row() {
        let mut session = HelpScreenSession::default();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), Locale::En);
            })
            .unwrap();

        let rows = terminal
            .backend()
            .buffer()
            .content()
            .chunks(120)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>();
        let quit_label = command_specs(CommandContext::LibraryBrowse)
            .find(|spec| spec.command == UiCommand::Quit)
            .map(|spec| text(Locale::En, spec.label).into_owned())
            .expect("Help has a Quit command");
        let other_label = command_specs(CommandContext::LibraryBrowse)
            .find(|spec| spec.help && spec.command != UiCommand::Quit)
            .map(|spec| text(Locale::En, spec.label).into_owned())
            .expect("Help has a non-Quit command");
        let quit_row = rows
            .iter()
            .find(|row| row.contains(&quit_label))
            .expect("Quit is visible in the tall Help viewport");
        let other_row = rows
            .iter()
            .find(|row| row.contains(&other_label))
            .expect("a non-Quit command is visible");

        assert!(quit_row.contains("q / Esc / Ctrl+C Ctrl+C"), "{quit_row}");
        assert!(!other_row.contains("Ctrl+C Ctrl+C"), "{other_row}");
    }

    #[test]
    fn discard_overlay_keeps_both_visible_chips_clickable_in_every_locale() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            for (width, height) in [(60, 12), (24, 5)] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let mut geometry = ViewGeometry::default();
                terminal
                    .draw(|frame| {
                        geometry =
                            ConfirmDiscardSession::default().render(frame, frame.area(), locale);
                    })
                    .unwrap();
                assert_eq!(geometry.hits.len(), 2);
                assert!(geometry.hits.iter().all(|hit| !hit.rect.is_empty()));
                assert!(matches!(
                    geometry.hits[0].action,
                    HitTarget::Command(UiCommand::DiscardChanges)
                ));
                assert!(matches!(
                    geometry.hits[1].action,
                    HitTarget::Command(UiCommand::KeepEditing)
                ));
            }
        }
    }

    fn chip_background(buffer: &Buffer, needle: &str) -> ratatui_core::style::Color {
        let at = find_text(buffer, needle);
        buffer[(at.x, at.y)].bg
    }

    // Ways this can fail: the guard opens on Discard, Tab or Shift+Tab does not move the ring,
    // Enter activates the chip that is not under the ring, or the chips show no keys.
    #[test]
    fn the_discard_guard_shows_key_chips_and_a_focus_ring_that_opens_on_keep_editing() {
        let draw = |session: &ConfirmDiscardSession| {
            let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
            terminal
                .draw(|frame| {
                    let _ = session.render(frame, frame.area(), Locale::En);
                })
                .unwrap();
            terminal.backend().buffer().clone()
        };
        let mut session = ConfirmDiscardSession::default();
        let buffer = draw(&session);
        assert!(!find_text(&buffer, " y Discard ").is_empty());
        assert!(!find_text(&buffer, " Esc Keep editing ").is_empty());
        let accent = theme::panel_color(Panel::Dialog);
        assert_eq!(chip_background(&buffer, "Keep editing"), accent);
        assert_ne!(chip_background(&buffer, "Discard"), accent);
        assert_eq!(
            session.handle_event(&key(KeyCode::Enter)),
            Some(EventHandling::Action(skit_ui::Action::KeepEditing))
        );

        assert_eq!(
            session.handle_event(&key(KeyCode::Tab)),
            Some(EventHandling::Consumed)
        );
        let buffer = draw(&session);
        assert_eq!(chip_background(&buffer, "Discard"), accent);
        assert_ne!(chip_background(&buffer, "Keep editing"), accent);
        assert_eq!(
            session.handle_event(&key(KeyCode::Enter)),
            Some(EventHandling::Action(skit_ui::Action::DiscardChanges))
        );

        assert_eq!(
            session.handle_event(&Event::Key(KeyEvent::new(
                KeyCode::BackTab,
                KeyModifiers::SHIFT
            ))),
            Some(EventHandling::Consumed)
        );
        assert_eq!(
            session.handle_event(&key(KeyCode::Enter)),
            Some(EventHandling::Action(skit_ui::Action::KeepEditing))
        );

        for event in [
            key(KeyCode::Char('y')),
            key(KeyCode::Esc),
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Tab,
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
            mouse(
                Rect::new(0, 0, 1, 1),
                MouseEventKind::Down(MouseButton::Left),
            ),
            Event::Paste("y".to_owned()),
        ] {
            assert_eq!(session.handle_event(&event), None, "{event:?}");
        }

        session.perturb_agent_review_state();
        session.reset();
        assert_eq!(
            session.handle_event(&key(KeyCode::Enter)),
            Some(EventHandling::Action(skit_ui::Action::KeepEditing)),
            "a guard that opens again starts on Keep editing",
        );
        assert!(
            serde_json::to_string(&session.agent_review_snapshot())
                .unwrap()
                .contains("discard_focused")
        );
    }

    #[test]
    fn the_remove_dialog_shows_key_chips_with_the_ring_on_keep() {
        let mut session = ConfirmRemoveSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), "Alpha", true, Locale::En);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        assert!(!find_text(&buffer, " y Remove ").is_empty());
        assert!(!find_text(&buffer, " Esc Keep ").is_empty());
        let accent = theme::panel_color(Panel::Dialog);
        assert_eq!(chip_background(&buffer, "Esc Keep"), accent);
        assert_ne!(chip_background(&buffer, "y Remove"), accent);
        assert_eq!(session.pointer_targets().len(), 2);
        assert!(
            session
                .pointer_targets()
                .iter()
                .all(|(_, pressed)| !pressed)
        );
    }

    #[test]
    fn compact_confirm_remove_keeps_both_actions_visible_and_operable() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
            for height in [5, 6] {
                let mut session = ConfirmRemoveSession::default();
                let mut terminal = Terminal::new(TestBackend::new(24, height)).unwrap();
                terminal
                    .draw(|frame| {
                        let _ = session.render(frame, frame.area(), "Alpha", true, locale);
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                assert_eq!(buffer[(0, 0)].symbol(), "┌");
                assert_eq!(buffer[(23, 0)].symbol(), "┐");
                assert_eq!(buffer[(0, height - 1)].symbol(), "└");
                assert_eq!(buffer[(23, height - 1)].symbol(), "┘");

                let regions = session
                    .dialog
                    .click_regions
                    .regions()
                    .iter()
                    .map(|region| (region.area, region.data))
                    .collect::<Vec<_>>();
                let labels = [
                    format!("y {}", text(locale, "Remove")),
                    format!("Esc {}", text(locale, "Keep")),
                ];
                assert_eq!(regions.len(), 2, "{locale:?} at 24x{height}");
                for (index, (area, target)) in regions.iter().enumerate() {
                    assert_eq!(
                        target,
                        &DialogFocusTarget::Button(index),
                        "{locale:?} at 24x{height}",
                    );
                    assert!(
                        !area.is_empty() && area.right() <= 24 && area.bottom() <= height,
                        "{locale:?} at 24x{height}: {area:?}",
                    );
                    assert_eq!(
                        usize::from(area.width),
                        labels[index].width().saturating_add(2),
                        "{locale:?} button {index} was clipped at 24x{height}",
                    );
                    assert!(
                        (area.x..area.right())
                            .any(|column| { !buffer[(column, area.y)].symbol().trim().is_empty() }),
                        "{locale:?} button {index} has no visible text at 24x{height}",
                    );
                }
                assert!(
                    regions[0].0.intersection(regions[1].0).is_empty(),
                    "{locale:?} compact actions overlap at 24x{height}",
                );
                assert_eq!(
                    session.dialog.current_focus(),
                    Some(&DialogFocusTarget::Button(1)),
                    "{locale:?} compact focus at 24x{height}",
                );
                assert!(session.dialog.is_visible());

                for (index, expected) in [
                    (0, ConfirmRemoveEvent::Submit),
                    (1, ConfirmRemoveEvent::Close),
                ] {
                    let mut mouse_session = session.clone();
                    let area = regions[index].0;
                    assert_eq!(
                        mouse_session
                            .handle_event(&mouse(area, MouseEventKind::Down(MouseButton::Left),)),
                        ConfirmRemoveEvent::Consumed,
                    );
                    assert_eq!(
                        mouse_session
                            .handle_event(&mouse(area, MouseEventKind::Up(MouseButton::Left),)),
                        expected,
                    );
                }

                let mut keep = session.clone();
                assert_eq!(
                    keep.handle_event(&key(KeyCode::Enter)),
                    ConfirmRemoveEvent::Close,
                    "Enter activates the focused Keep chip",
                );
                let mut remove = session;
                assert_eq!(
                    remove.handle_event(&key(KeyCode::Tab)),
                    ConfirmRemoveEvent::Consumed
                );
                assert_eq!(
                    remove.dialog.current_focus(),
                    Some(&DialogFocusTarget::Button(0)),
                );
                assert_eq!(
                    remove.handle_event(&key(KeyCode::Enter)),
                    ConfirmRemoveEvent::Submit,
                    "Enter activates the Remove chip after the focus moves",
                );
            }
        }

        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
            let mut session = ConfirmRemoveSession::default();
            let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
            terminal
                .draw(|frame| {
                    let _ = session.render(frame, frame.area(), "Alpha", true, locale);
                })
                .unwrap();
            assert!(session.dialog.click_regions.regions().is_empty());
            assert_eq!(
                session.handle_event(&key(KeyCode::Esc)),
                ConfirmRemoveEvent::Ignored,
                "the shared command registry owns Escape",
            );
        }
    }

    #[test]
    fn discard_overlay_renders_the_full_keep_editing_label_in_both_chinese_locales() {
        for (locale, keep) in [(Locale::ZhCn, "继续编辑"), (Locale::ZhTw, "繼續編輯")] {
            let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
            terminal
                .draw(|frame| {
                    let _ = ConfirmDiscardSession::default().render(frame, frame.area(), locale);
                })
                .unwrap();
            // Ratatui's TestBackend exposes the continuation cell of each wide glyph as a space.
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                rendered.replace(' ', "").contains(keep),
                "missing {keep}: {rendered}"
            );
        }
    }

    #[test]
    fn the_compact_button_row_refuses_a_panel_that_is_too_small_for_one_column() {
        let buttons = vec![
            ("Remove".to_owned(), ContainerAction::Submit),
            ("Keep editing".to_owned(), ContainerAction::Close),
        ];

        assert_eq!(
            compact_dialog_button_areas(Rect::new(0, 0, 4, 2), &buttons),
            vec![(0, Rect::new(0, 0, 4, 1)), (1, Rect::new(0, 1, 4, 1))]
        );
        assert!(compact_dialog_button_areas(Rect::new(0, 0, 4, 1), &buttons).is_empty());
    }
}
