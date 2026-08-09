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
#[derive(Debug, Default)]
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
        drop(popup);
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
#[derive(Debug, Default)]
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
        Block::default()
            .borders(Borders::ALL)
            .title(format!("{}{}", text(locale, "Help"), indicator)),
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
    let block = padded_panel(
        text(locale, "Discard unsaved changes?").into_owned(),
        ACCENT,
    );
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
        u16::try_from(discard.chars().count().saturating_add(4)).unwrap_or(u16::MAX);
    let keep_width = u16::try_from(keep.chars().count().saturating_add(4)).unwrap_or(u16::MAX);
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
