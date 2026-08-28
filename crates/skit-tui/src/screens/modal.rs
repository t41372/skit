//! Help and confirmation overlays.

use ratatui_core::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui_interact::components::{
    Button, ButtonState, ButtonStyle, ButtonVariant, DialogConfig, DialogState, PopupDialog,
    ScrollableContentState, handle_scrollable_content_key, handle_scrollable_content_mouse,
};
use ratatui_interact::traits::{ContainerAction, EventResult};
use ratatui_widgets::{
    block::Block,
    borders::Borders,
    paragraph::{Paragraph, Wrap},
};
use skit_i18n::{Locale, text};
use skit_ui::{CommandContext, UiCommand, command_specs};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    HitRegion, HitTarget, ViewGeometry,
    theme::{ACCENT, BOX_DIM, padded_panel},
};

/// Result of one mature confirmation-dialog event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfirmRemoveEvent {
    Submit,
    Close,
    Consumed,
    Ignored,
}

/// Persistent mature dialog state for entry removal.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConfirmRemoveSession {
    dialog: DialogState<()>,
    config: Option<DialogConfig>,
    screen: Rect,
}

impl ConfirmRemoveSession {
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        name: &str,
        original_file_preserved: bool,
        locale: Locale,
    ) -> ViewGeometry {
        if self.dialog.focus.is_empty() {
            self.dialog.register_button(1);
            self.dialog.register_button(0);
        }
        self.dialog.show();
        self.screen = frame.area();
        let config = DialogConfig::new(text(locale, "Confirm removal"))
            .width_percent(72)
            .height_percent(38)
            .min_size(34, 7)
            .max_size(90, 12)
            .border_color(ACCENT)
            .focused_border_color(ACCENT)
            .close_on_outside_click(false)
            .buttons(vec![
                (text(locale, "Remove").into_owned(), ContainerAction::Submit),
                (text(locale, "Keep").into_owned(), ContainerAction::Close),
            ]);
        let mut popup = PopupDialog::new(&config, &mut self.dialog, |frame, area, ()| {
            let mut lines = vec![Line::from(format!(
                "{} {name}?",
                text(locale, "Remove this entry:")
            ))];
            if original_file_preserved {
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    text(locale, "Your original file will not be deleted."),
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
        });
        popup.render(frame);
        self.config = Some(config);
        ViewGeometry::default()
    }

    pub(crate) fn handle_event(&mut self, event: &Event) -> ConfirmRemoveEvent {
        let Some(config) = self.config.as_ref() else {
            return ConfirmRemoveEvent::Ignored;
        };
        let mut popup = PopupDialog::new(config, &mut self.dialog, |_, _, ()| {});
        let result = match event {
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) =>
            {
                popup.handle_key(*key)
            }
            Event::Mouse(mouse) => popup.handle_mouse_with_screen(*mouse, self.screen),
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
    pub(crate) fn render(&mut self, frame: &mut Frame, area: Rect, locale: Locale) -> ViewGeometry {
        let lines = command_specs(CommandContext::LibraryBrowse)
            .filter(|spec| spec.help)
            .filter_map(|spec| {
                spec.bindings.first().map(|binding| {
                    let hint = if spec.command == UiCommand::Quit {
                        "Ctrl+C Ctrl+C / Esc"
                    } else {
                        binding.hint
                    };
                    Line::from(vec![
                        Span::styled(
                            format!("{hint:>20}"),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  "),
                        Span::raw(text(locale, spec.label)),
                    ])
                })
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
        if self.scroll.scroll_offset() > maximum {
            self.scroll.set_scroll_offset(maximum);
        }
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
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) =>
            {
                handle_scrollable_content_mouse(
                    &mut self.scroll,
                    mouse,
                    self.viewport,
                    self.visible_height,
                )
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
pub(crate) fn discard_changes(frame: &mut Frame, area: Rect, locale: Locale) -> ViewGeometry {
    let [panel] = Layout::vertical([Constraint::Length(7)])
        .flex(Flex::Center)
        .areas(area);
    let [panel] = Layout::horizontal([Constraint::Length(52)])
        .flex(Flex::Center)
        .areas(panel);
    // Version 0.4 shows the question once, inside an untitled border
    // (`src/skit/tui_settings.py:42-65`). The header names the surface; the
    // panel itself must not repeat the body sentence as a title.
    let block = padded_panel(String::new(), ACCENT);
    let inner = block.inner(panel);
    frame.render_widget(block, panel);

    let [message, actions] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    frame.render_widget(
        Paragraph::new(text(locale, "Discard unsaved changes?")),
        message,
    );
    let discard = text(locale, "Discard");
    let keep = text(locale, "Keep editing");
    let discard_width =
        u16::try_from(discard.as_ref().width().saturating_add(4)).unwrap_or(u16::MAX);
    let keep_width = u16::try_from(keep.as_ref().width().saturating_add(4)).unwrap_or(u16::MAX);
    let [discard_area, keep_area, _] = Layout::horizontal([
        Constraint::Length(discard_width),
        Constraint::Length(keep_width),
        Constraint::Min(0),
    ])
    .spacing(1)
    .areas(actions);
    let style = ButtonStyle::new(ButtonVariant::SingleLine)
        .focused(Color::Black, ACCENT)
        .unfocused(Color::White, BOX_DIM);
    let discard_region = Button::new(&discard, &ButtonState::default())
        .variant(ButtonVariant::SingleLine)
        .style(style.clone())
        .render_stateful(discard_area, frame.buffer_mut());
    let keep_region = Button::new(&keep, &ButtonState::default())
        .variant(ButtonVariant::SingleLine)
        .style(style)
        .render_stateful(keep_area, frame.buffer_mut());

    ViewGeometry {
        rows: inner,
        first_visible: 0,
        detail_pane_visible: false,
        hits: vec![
            HitRegion {
                rect: discard_region.area,
                action: HitTarget::Command(skit_ui::UiCommand::DiscardChanges),
            },
            HitRegion {
                rect: keep_region.area,
                action: HitTarget::Command(skit_ui::UiCommand::KeepEditing),
            },
        ],
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
                let _ = session.render(frame, "Alpha", true, Locale::En);
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
            ConfirmRemoveEvent::Submit
        );
        assert_eq!(
            session.handle_event(&mouse(keep, MouseEventKind::Down(MouseButton::Left))),
            ConfirmRemoveEvent::Close
        );
        for event in [
            mouse(remove, MouseEventKind::Moved),
            mouse(remove, MouseEventKind::Up(MouseButton::Left)),
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
                let _ = session.render(frame, "Beta", false, Locale::ZhTw);
            })
            .unwrap();
        assert!(!find_text(terminal.backend().buffer(), "Beta").is_empty());
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
    fn discard_overlay_keeps_both_visible_chips_clickable_in_every_locale() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            for (width, height) in [(60, 12), (24, 5)] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let mut geometry = ViewGeometry::default();
                terminal
                    .draw(|frame| {
                        geometry = discard_changes(frame, frame.area(), locale);
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

    #[test]
    fn discard_overlay_renders_the_full_keep_editing_label_in_both_chinese_locales() {
        for (locale, keep) in [(Locale::ZhCn, "继续编辑"), (Locale::ZhTw, "繼續編輯")] {
            let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
            terminal
                .draw(|frame| {
                    let _ = discard_changes(frame, frame.area(), locale);
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
}
