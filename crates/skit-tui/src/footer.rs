//! Responsive command footer with persistent mature scrolling state.

use ratatui_core::{
    layout::Rect,
    style::{Color, Style},
    terminal::Frame,
};
use ratatui_crossterm::crossterm::event::{KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui_interact::components::{
    Button, ButtonState, ButtonStyle, ButtonVariant, ScrollableContentState,
};
use ratatui_widgets::{
    block::Block,
    borders::{BorderType, Borders},
    paragraph::Paragraph,
};
use skit_i18n::{Locale, format_text, render as localize, text};
use skit_ui::{
    CommandContext, LibraryState, Screen, UiBinding, UiCommand, UiCommandSpec, UiKey, command_specs,
};
use unicode_width::UnicodeWidthStr as _;

use crate::{HitRegion, HitTarget, local_action::LocalKey};

const PILL_BACKGROUND: Color = Color::Rgb(0x2A, 0x21, 0x1C);
const PILL_FOREGROUND: Color = Color::Rgb(0xD9, 0x77, 0x57);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FooterInputOwnership {
    pub(crate) vertical_navigation: bool,
    pub(crate) run_submit: bool,
    pub(crate) preferences_input: bool,
    pub(crate) escape: bool,
}

/// One typed command in a responsive local action footer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActionFooterItem<A> {
    key: LocalKey,
    label: String,
    action: A,
    starts_group: bool,
}

impl<A> ActionFooterItem<A> {
    /// Add an action after the preceding action, wrapping when necessary.
    pub(crate) fn new(key: LocalKey, label: impl Into<String>, action: A) -> Self {
        Self {
            key,
            label: label.into(),
            action,
            starts_group: false,
        }
    }

    /// Add an action on a new row so related actions stay together.
    pub(crate) fn new_group(key: LocalKey, label: impl Into<String>, action: A) -> Self {
        Self {
            starts_group: true,
            ..Self::new(key, label, action)
        }
    }

    #[cfg(test)]
    pub(crate) fn advertised_key(&self) -> String {
        self.key.hint()
    }

    #[cfg(test)]
    pub(crate) fn typed_action(&self) -> &A {
        &self.action
    }
}

/// Colors for a local action footer.
#[derive(Clone, Debug)]
pub(crate) struct ActionFooterStyle {
    button: ButtonStyle,
}

impl ActionFooterStyle {
    /// Use the supplied foreground and background for each command chip.
    pub(crate) fn new(foreground: Color, background: Color) -> Self {
        Self {
            button: ButtonStyle::new(ButtonVariant::SingleLine)
                .focused(foreground, background)
                .unfocused(foreground, background),
        }
    }
}

impl Default for ActionFooterStyle {
    fn default() -> Self {
        Self::new(PILL_FOREGROUND, PILL_BACKGROUND)
    }
}

/// Result of a mouse event sent to a local action footer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ActionFooterMouse<A> {
    /// The user clicked a visible command chip.
    Action(A),
    /// The footer scrolled to another row of commands.
    Scrolled,
    /// The event was outside the footer or had no effect.
    Ignored,
}

/// Persistent mature scroll and click state for a typed local action footer.
#[derive(Clone, Debug)]
pub(crate) struct ActionFooterSession<A: Clone> {
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
    clicks: ratatui_interact::traits::ClickRegionRegistry<A>,
    advertised: Vec<(Rect, LocalKey, A)>,
}

impl<A: Clone> Default for ActionFooterSession<A> {
    fn default() -> Self {
        Self {
            scroll: ScrollableContentState::default(),
            viewport: Rect::default(),
            visible_height: 0,
            clicks: ratatui_interact::traits::ClickRegionRegistry::default(),
            advertised: Vec::new(),
        }
    }
}

impl<A: Clone> ActionFooterSession<A> {
    pub(crate) fn handle_key(&self, key: &KeyEvent) -> Option<A> {
        (key.kind != KeyEventKind::Release)
            .then(|| {
                self.advertised.iter().find_map(|(_, local_key, action)| {
                    local_key.accepts(key).then(|| action.clone())
                })
            })
            .flatten()
    }

    /// Render all commands with wrapping and vertical scrolling.
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        items: &[ActionFooterItem<A>],
        style: ActionFooterStyle,
    ) {
        self.clicks.clear();
        self.advertised.clear();
        self.visible_height = usize::from(area.height);
        let content_width = action_footer_content_width(area.width);
        self.viewport = Rect::new(area.x, area.y, content_width, area.height);
        let (chips, rows) = action_footer_chips(items, content_width);
        self.scroll.set_lines(vec![String::new(); rows]);
        let maximum_offset = rows.saturating_sub(self.visible_height);
        if self.scroll.scroll_offset() > maximum_offset {
            self.scroll.set_scroll_offset(maximum_offset);
        }

        let offset = self.scroll.scroll_offset();
        let end = offset.saturating_add(self.visible_height);
        for chip in chips
            .into_iter()
            .filter(|chip| chip.row >= offset && chip.row < end)
        {
            let y = area
                .y
                .saturating_add(u16::try_from(chip.row.saturating_sub(offset)).unwrap_or(u16::MAX));
            let chip_area = Rect::new(
                area.x.saturating_add(chip.x),
                y,
                chip.width.min(content_width.saturating_sub(chip.x)),
                1,
            );
            let key_hint = chip.item.key.hint();
            let region = Button::new(&chip.item.label, &ButtonState::enabled())
                .icon(&key_hint)
                .variant(ButtonVariant::SingleLine)
                .style(style.button.clone())
                .render_stateful(chip_area, frame.buffer_mut());
            self.clicks.register(region.area, chip.item.action.clone());
            if region.area.width > 0 && region.area.height > 0 {
                self.advertised
                    .push((region.area, chip.item.key, chip.item.action.clone()));
            }
        }

        let at_top = self.scroll.is_at_top();
        let at_bottom = self.scroll.is_at_bottom(self.visible_height);
        let indicator =
            ["", "↓", "↑", "↕"][usize::from(!at_top).saturating_mul(2) + usize::from(!at_bottom)];
        if !indicator.is_empty() {
            frame.render_widget(
                Paragraph::new(indicator).style(Style::default().fg(PILL_FOREGROUND)),
                Rect::new(
                    self.viewport.right(),
                    area.y,
                    area.right().saturating_sub(self.viewport.right()),
                    1,
                ),
            );
        }
    }

    /// Dispatch a click or wheel event through the footer's mature state.
    pub(crate) fn handle_mouse(&mut self, mouse: &MouseEvent) -> ActionFooterMouse<A> {
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            return self
                .clicks
                .handle_click(mouse.column, mouse.row)
                .cloned()
                .map_or(ActionFooterMouse::Ignored, ActionFooterMouse::Action);
        }
        if handle_footer_scroll(&mut self.scroll, mouse, self.viewport, self.visible_height) {
            return ActionFooterMouse::Scrolled;
        }
        ActionFooterMouse::Ignored
    }

    pub(crate) fn advertised(&self) -> &[(Rect, LocalKey, A)] {
        &self.advertised
    }
}

/// Return the rows needed to show every local footer action at this width.
pub(crate) fn action_footer_required_height<A>(width: u16, items: &[ActionFooterItem<A>]) -> u16 {
    let (_, rows) = action_footer_chips(items, action_footer_content_width(width));
    u16::try_from(rows).unwrap_or(u16::MAX)
}

#[derive(Debug)]
struct PositionedActionFooterItem<'a, A> {
    item: &'a ActionFooterItem<A>,
    row: usize,
    x: u16,
    width: u16,
}

fn action_footer_chips<A>(
    items: &[ActionFooterItem<A>],
    width: u16,
) -> (Vec<PositionedActionFooterItem<'_, A>>, usize) {
    if items.is_empty() || width == 0 {
        return (Vec::new(), 0);
    }
    let mut row = 0_usize;
    let mut x = 0_u16;
    let mut chips = Vec::with_capacity(items.len());
    for item in items {
        if item.starts_group && !chips.is_empty() {
            row = row.saturating_add(1);
            x = 0;
        }
        let chip_width = u16::try_from(
            item.key
                .hint()
                .width()
                .saturating_add(item.label.width())
                .saturating_add(3),
        )
        .unwrap_or(u16::MAX)
        .min(width);
        if x > 0 && x.saturating_add(chip_width) > width {
            row = row.saturating_add(1);
            x = 0;
        }
        chips.push(PositionedActionFooterItem {
            item,
            row,
            x,
            width: chip_width,
        });
        x = x.saturating_add(chip_width).saturating_add(2);
    }
    (chips, row.saturating_add(1))
}

fn action_footer_content_width(width: u16) -> u16 {
    if width > 2 {
        width.saturating_sub(2)
    } else {
        width
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FooterSession {
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
}

#[derive(Debug)]
struct Chip {
    key: String,
    label: String,
    command: UiCommand,
    row: usize,
    x: u16,
    width: u16,
}

pub(crate) fn required_height(
    width: u16,
    terminal_height: u16,
    header_height: u16,
    state: &LibraryState,
    locale: Locale,
    ownership: FooterInputOwnership,
) -> u16 {
    let available = terminal_height
        .saturating_sub(header_height)
        .saturating_sub(u16::from(terminal_height > header_height));
    if available == 0 {
        return 0;
    }
    let inner_width = width.saturating_sub(2);
    if inner_width == 0 {
        return 2.min(available);
    }
    let (_, rows) = chips(state, locale, inner_width, ownership);
    let tiny = terminal_height <= 6;
    let visible_rows = rows.min(if tiny {
        1
    } else {
        row_budget(terminal_height, state.command_context())
    });
    let border_rows = if tiny || available <= 2 { 0 } else { 2 };
    let desired = u16::try_from(visible_rows)
        .unwrap_or(u16::MAX)
        .saturating_add(u16::from(has_note(state, inner_width)))
        .saturating_add(border_rows);
    desired.min(available).max(1)
}

pub(crate) fn is_suppressed(state: &LibraryState) -> bool {
    matches!(
        state.screen(),
        Screen::Add(_) | Screen::Health(_) | Screen::Runners(_)
    ) || matches!(
        state.screen(),
        Screen::Preferences(view) if view.agent_skill_install().is_some()
    ) || matches!(
        state.modal(),
        Some(skit_ui::ModalState::RunnerEditor { .. })
    )
}

impl FooterSession {
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        state: &LibraryState,
        locale: Locale,
        ownership: FooterInputOwnership,
    ) -> Vec<HitRegion> {
        let compact = area.height <= 2;
        let base_block = Block::default()
            .borders(if compact { Borders::NONE } else { Borders::ALL })
            .border_type(BorderType::Rounded);
        let inner = base_block.inner(area);
        let note_rows = usize::from(has_note(state, inner.width));
        self.visible_height = usize::from(inner.height).saturating_sub(note_rows);
        let mut content_width = inner.width;
        let (mut positioned, mut rows) = chips(state, locale, content_width, ownership);
        if compact && rows > self.visible_height && content_width > 1 {
            content_width = content_width.saturating_sub(1);
            (positioned, rows) = chips(state, locale, content_width, ownership);
        }
        self.viewport = Rect::new(
            inner.x,
            inner.y,
            content_width,
            u16::try_from(self.visible_height).unwrap_or(u16::MAX),
        );
        self.scroll.set_lines(vec![String::new(); rows]);
        let maximum_offset = rows.saturating_sub(self.visible_height);
        if self.scroll.scroll_offset() > maximum_offset {
            self.scroll.set_scroll_offset(maximum_offset);
        }
        let indicator = match (
            self.scroll.is_at_top(),
            self.scroll.is_at_bottom(self.visible_height),
        ) {
            (true, true) => None,
            (true, false) => Some(" ↓ "),
            (false, true) => Some(" ↑ "),
            (false, false) => Some(" ↑↓ "),
        };
        if compact {
            frame.render_widget(base_block, area);
            if let Some(title) = indicator {
                frame.render_widget(
                    Paragraph::new(title.trim()),
                    Rect::new(inner.right().saturating_sub(1), inner.y, 1, 1),
                );
            }
        } else {
            frame.render_widget(
                indicator.map_or(base_block.clone(), |title| base_block.clone().title(title)),
                area,
            );
        }

        let offset = self.scroll.scroll_offset();
        let end = offset.saturating_add(self.visible_height);
        let mut hits = Vec::new();
        for chip in positioned
            .into_iter()
            .filter(|chip| chip.row >= offset && chip.row < end)
        {
            let y = inner
                .y
                .saturating_add(u16::try_from(chip.row.saturating_sub(offset)).unwrap_or(u16::MAX));
            let chip_area = Rect::new(
                inner.x.saturating_add(chip.x),
                y,
                chip.width.min(content_width.saturating_sub(chip.x)),
                1,
            );
            let mut state = ButtonState::enabled();
            state.set_focused(true);
            let region = Button::new(&chip.label, &state)
                .icon(&chip.key)
                .variant(ButtonVariant::SingleLine)
                .style(footer_button_style())
                .render_stateful(chip_area, frame.buffer_mut());
            hits.push(HitRegion {
                rect: region.area,
                action: HitTarget::Command(chip.command),
            });
        }

        if let Some(status) = state.status() {
            let status_area = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
            frame.render_widget(Paragraph::new(localize(locale, status)), status_area);
        } else if matches!(state.screen(), Screen::Library) {
            let status = default_library_status(state, locale);
            let status_area = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
            frame.render_widget(Paragraph::new(status), status_area);
            if !state.diagnostics().is_empty() && inner.width > 50 {
                let note = format!(
                    "{} {}",
                    state.diagnostics().len(),
                    text(locale, "damaged entries hidden")
                );
                let width = u16::try_from(note.width())
                    .unwrap_or(u16::MAX)
                    .min(inner.width);
                let note_area = Rect::new(
                    inner.right().saturating_sub(width),
                    inner.bottom().saturating_sub(1),
                    width,
                    1,
                );
                frame.render_widget(Paragraph::new(note), note_area);
            }
        }
        hits
    }

    pub(crate) fn handle_mouse(&mut self, mouse: &MouseEvent) -> bool {
        handle_footer_scroll(&mut self.scroll, mouse, self.viewport, self.visible_height)
    }
}

pub(crate) fn handle_footer_scroll(
    scroll: &mut ScrollableContentState,
    mouse: &MouseEvent,
    viewport: Rect,
    visible_height: usize,
) -> bool {
    if mouse.column < viewport.x
        || mouse.column >= viewport.right()
        || mouse.row < viewport.y
        || mouse.row >= viewport.bottom()
    {
        return false;
    }
    let page = visible_height.max(1);
    match mouse.kind {
        MouseEventKind::ScrollUp => scroll.scroll_up(page),
        MouseEventKind::ScrollDown => scroll.scroll_down(page, page),
        _ => return false,
    }
    true
}

fn footer_button_style() -> ButtonStyle {
    ButtonStyle::new(ButtonVariant::SingleLine)
        .focused(PILL_FOREGROUND, PILL_BACKGROUND)
        .unfocused(PILL_FOREGROUND, PILL_BACKGROUND)
}

fn chips(
    state: &LibraryState,
    locale: Locale,
    inner_width: u16,
    ownership: FooterInputOwnership,
) -> (Vec<Chip>, usize) {
    let mut row = 0_usize;
    let mut x = 0_u16;
    let mut chips = Vec::new();
    for group in footer_groups(state, locale, ownership) {
        if group.is_empty() {
            continue;
        }
        if !chips.is_empty() {
            row = row.saturating_add(1);
            x = 0;
        }
        for (key, label, command) in group {
            let width = u16::try_from(key.width().saturating_add(label.width()).saturating_add(3))
                .unwrap_or(u16::MAX)
                .min(inner_width);
            if x > 0 && x.saturating_add(width) > inner_width {
                row = row.saturating_add(1);
                x = 0;
            }
            chips.push(Chip {
                key,
                label,
                command,
                row,
                x,
                width,
            });
            x = x.saturating_add(width).saturating_add(2);
        }
    }
    let rows = usize::from(!chips.is_empty()).saturating_add(row);
    (chips, rows)
}

fn footer_groups(
    state: &LibraryState,
    locale: Locale,
    ownership: FooterInputOwnership,
) -> Vec<Vec<(String, String, UiCommand)>> {
    let labels = command_specs(state.command_context())
        .filter(|spec| spec.footer)
        .filter(|spec| state.command_enabled(spec.command))
        .filter_map(|spec| {
            let bindings = displayed_bindings(*spec, ownership);
            let binding = bindings.first()?;
            // Version 0.4's shared navigation hint is two key-only pills that name BOTH keys for
            // each direction (`src/skit/tui_footer.py:82-94`): the arrows already say which way,
            // and a footer that advertises only Tab strands anyone who tabs one field too far. The
            // full words stay on the binding, so the help screen still shows them.
            if matches!(
                spec.command,
                UiCommand::FocusNext | UiCommand::FocusPrevious
            ) {
                let keys = bindings
                    .iter()
                    .map(|binding| match binding.key {
                        // The arrow reads as the direction; spelling it "Down" says nothing the
                        // glyph does not (`src/skit/tui_footer.py:88-89`).
                        UiKey::Down | UiKey::Up => binding.compact_hint,
                        _ => binding.hint,
                    })
                    .collect::<Vec<_>>()
                    .join("/");
                return Some((keys, String::new(), spec.command));
            }
            let label = match (spec.command, state.form()) {
                (UiCommand::Submit, Some(form)) => text(locale, &form.submit_label),
                _ => text(locale, spec.label),
            };
            Some((binding.hint.to_owned(), label.into_owned(), spec.command))
        })
        .collect::<Vec<_>>();
    if state.command_context() != CommandContext::LibraryBrowse {
        return vec![labels];
    }

    let (local, global) = labels.into_iter().partition(|(_, _, command)| {
        matches!(
            command,
            UiCommand::Run
                | UiCommand::Rerun
                | UiCommand::Settings
                | UiCommand::Edit
                | UiCommand::Rename
                | UiCommand::Remove
        )
    });
    vec![local, global]
}

pub(crate) fn advertised_bindings(
    state: &LibraryState,
    command: UiCommand,
    ownership: FooterInputOwnership,
) -> Vec<UiBinding> {
    command_specs(state.command_context())
        .find(|spec| spec.footer && spec.command == command && state.command_enabled(command))
        .map_or_else(Vec::new, |spec| displayed_bindings(*spec, ownership))
}

fn displayed_bindings(spec: UiCommandSpec, ownership: FooterInputOwnership) -> Vec<UiBinding> {
    if ownership.preferences_input
        && matches!(
            spec.command,
            UiCommand::ManageAgents | UiCommand::InstallAgentSkill
        )
    {
        return Vec::new();
    }
    if ownership.escape {
        let bindings = spec
            .bindings
            .iter()
            .copied()
            .filter(|binding| binding.key != UiKey::Escape)
            .collect::<Vec<_>>();
        if bindings.len() != spec.bindings.len() {
            return bindings;
        }
    }
    let owned_key = match spec.command {
        UiCommand::Submit if ownership.run_submit && spec.context == CommandContext::RunForm => {
            Some(UiKey::Character('r'))
        }
        UiCommand::FocusNext if ownership.vertical_navigation => Some(UiKey::Tab),
        UiCommand::FocusPrevious if ownership.vertical_navigation => Some(UiKey::BackTab),
        _ => None,
    };
    if let Some(key) = owned_key {
        return spec
            .bindings
            .iter()
            .copied()
            .filter(|binding| {
                binding.key == key
                    && (spec.command != UiCommand::Submit || binding.modifiers.control)
            })
            .collect();
    }
    if matches!(
        spec.command,
        UiCommand::FocusNext | UiCommand::FocusPrevious
    ) {
        return spec
            .bindings
            .iter()
            .copied()
            .filter(|binding| !matches!(binding.key, UiKey::Enter))
            .collect();
    }
    spec.bindings.first().copied().into_iter().collect()
}

fn has_note(state: &LibraryState, _inner_width: u16) -> bool {
    matches!(state.screen(), Screen::Library) || state.status().is_some()
}

fn row_budget(terminal_height: u16, context: CommandContext) -> usize {
    let library = matches!(
        context,
        CommandContext::LibraryBrowse | CommandContext::LibrarySearch
    );
    match (terminal_height, library) {
        (28.., _) => usize::MAX,
        (16..=27, true) => 6,
        (16..=27, false) => 3,
        (10..=15, true) => 2,
        _ => 1,
    }
}

fn default_library_status(state: &LibraryState, locale: Locale) -> String {
    if state.entry_count() == 0 {
        return text(locale, "Your entries will appear here.").into_owned();
    }
    let template = if state.entry_count() == 1 {
        "{}/{} entry"
    } else {
        "{}/{} entries"
    };
    format_text(
        locale,
        template,
        &[&state.visible_entry_count(), &state.entry_count()],
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ratatui_core::{backend::TestBackend, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::{
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use skit_domain::parameters::{ParamDecl, ParameterValue};
    use skit_ui::{Action, RunFormView};

    use super::*;
    use crate::screens::management::{
        health_footer_items, runner_action_footer_items, runner_editor_footer_items,
        runner_manager_footer_items, runner_removal_footer_items,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum TestAction {
        First,
        Second,
        Third,
        Fourth,
    }

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn scroll_down(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn action_footer_wraps_every_chip_and_keeps_each_visible_chip_clickable() {
        let items = [
            ActionFooterItem::new(LocalKey::Character('1'), "First action", TestAction::First),
            ActionFooterItem::new(
                LocalKey::Character('2'),
                "Second action",
                TestAction::Second,
            ),
            ActionFooterItem::new(LocalKey::Character('3'), "Third action", TestAction::Third),
        ];
        assert_eq!(action_footer_required_height(22, &items), 3);

        let backend = TestBackend::new(22, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut session = ActionFooterSession::default();
        terminal
            .draw(|frame| {
                session.render(frame, frame.area(), &items, ActionFooterStyle::default());
            })
            .unwrap();

        for (row, action) in [TestAction::First, TestAction::Second, TestAction::Third]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                session.handle_mouse(&click(1, u16::try_from(row).unwrap())),
                ActionFooterMouse::Action(action)
            );
        }
    }

    #[test]
    fn action_footer_scrolls_to_chips_that_do_not_fit_the_viewport() {
        let items = [
            ActionFooterItem::new(LocalKey::Character('1'), "First action", TestAction::First),
            ActionFooterItem::new(
                LocalKey::Character('2'),
                "Second action",
                TestAction::Second,
            ),
            ActionFooterItem::new(LocalKey::Character('3'), "Third action", TestAction::Third),
            ActionFooterItem::new(
                LocalKey::Character('4'),
                "Fourth action",
                TestAction::Fourth,
            ),
        ];
        let backend = TestBackend::new(22, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut session = ActionFooterSession::default();
        terminal
            .draw(|frame| {
                session.render(frame, frame.area(), &items, ActionFooterStyle::default());
            })
            .unwrap();
        assert_eq!(
            session.handle_mouse(&scroll_down(1, 0)),
            ActionFooterMouse::Scrolled
        );
        let bottom = session.scroll.scroll_offset();
        assert_eq!(
            session.handle_mouse(&MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 1,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
            ActionFooterMouse::Scrolled
        );
        assert!(session.scroll.scroll_offset() < bottom);
        assert_eq!(
            session.handle_mouse(&scroll_down(1, 0)),
            ActionFooterMouse::Scrolled
        );
        terminal
            .draw(|frame| {
                session.render(frame, frame.area(), &items, ActionFooterStyle::default());
            })
            .unwrap();
        assert_eq!(
            session.handle_mouse(&scroll_down(1, 0)),
            ActionFooterMouse::Scrolled
        );
        assert_eq!(
            session.handle_mouse(&MouseEvent {
                kind: MouseEventKind::Moved,
                column: 1,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
            ActionFooterMouse::Ignored
        );
        assert_eq!(
            session.handle_mouse(&click(80, 20)),
            ActionFooterMouse::Ignored
        );
        terminal
            .draw(|frame| {
                session.render(frame, frame.area(), &items, ActionFooterStyle::default());
            })
            .unwrap();
        assert_eq!(
            session.handle_mouse(&scroll_down(1, 0)),
            ActionFooterMouse::Scrolled
        );
        terminal
            .draw(|frame| {
                session.render(frame, frame.area(), &items, ActionFooterStyle::default());
            })
            .unwrap();
        assert_eq!(
            session.handle_mouse(&click(1, 0)),
            ActionFooterMouse::Action(TestAction::Fourth)
        );

        let mut tall = Terminal::new(TestBackend::new(22, 5)).unwrap();
        tall.draw(|frame| {
            session.render(frame, frame.area(), &items, ActionFooterStyle::default());
        })
        .unwrap();
        assert_eq!(session.scroll.scroll_offset(), 0);
    }

    #[test]
    fn action_footer_group_starts_on_a_new_row() {
        let items = [
            ActionFooterItem::new(LocalKey::Character('1'), "One", TestAction::First),
            ActionFooterItem::new_group(LocalKey::Character('2'), "Two", TestAction::Second),
        ];
        assert_eq!(action_footer_required_height(80, &items), 2);
        assert_eq!(action_footer_required_height(0, &items), 0);
        assert_eq!(action_footer_required_height(1, &items), 2);
        assert_eq!(action_footer_required_height::<TestAction>(20, &[]), 0);
        assert_eq!(action_footer_content_width(0), 0);
        assert_eq!(action_footer_content_width(2), 2);
    }

    fn assert_local_action_inventory<A>(items: Vec<ActionFooterItem<A>>)
    where
        A: Clone + std::fmt::Debug + Eq,
    {
        let expected = items
            .iter()
            .map(|item| item.action.clone())
            .collect::<Vec<_>>();
        for (width, height) in [(120, 3), (46, 2), (24, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut session = ActionFooterSession::default();
            let mut seen = Vec::new();
            for _ in 0..32 {
                terminal
                    .draw(|frame| {
                        session.render(frame, frame.area(), &items, ActionFooterStyle::default());
                    })
                    .unwrap();
                for y in 0..height {
                    for x in 0..width {
                        if let ActionFooterMouse::Action(action) =
                            session.handle_mouse(&click(x, y))
                            && !seen.contains(&action)
                        {
                            seen.push(action);
                        }
                    }
                }
                if seen.len() == expected.len() {
                    break;
                }
                assert_eq!(
                    session.handle_mouse(&scroll_down(0, 0)),
                    ActionFooterMouse::Scrolled,
                    "local footer stopped before every action at {width}x{height}: {seen:?}"
                );
            }
            assert!(
                seen.len() == expected.len() && expected.iter().all(|item| seen.contains(item)),
                "local footer dropped an action at {width}x{height}: expected={expected:?} seen={seen:?}"
            );
        }
    }

    #[test]
    fn every_management_local_action_uses_the_scrollable_footer_at_every_size_tier() {
        assert_local_action_inventory(health_footer_items(Locale::En));
        assert_local_action_inventory(runner_editor_footer_items(Locale::En));
        assert_local_action_inventory(runner_manager_footer_items(Locale::En));
        assert_local_action_inventory(runner_action_footer_items(Locale::En, true));
        assert_local_action_inventory(runner_action_footer_items(Locale::En, false));
        assert_local_action_inventory(runner_removal_footer_items(Locale::En));
    }

    #[test]
    fn run_form_command_registry_renders_insert_and_reset_in_both_chinese_locales() {
        let mut declaration = ParamDecl::new("name");
        declaration.default = Some(ParameterValue::String("World".to_owned()));
        let form = RunFormView::from_declarations(
            "greet",
            "greet",
            &[declaration],
            &BTreeMap::new(),
            &[],
            "",
            &BTreeMap::new(),
            "",
        );
        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Run(Box::new(form))));

        let commands = command_specs(CommandContext::RunForm).collect::<Vec<_>>();
        let insert = commands
            .iter()
            .find(|spec| spec.command == UiCommand::InsertValue)
            .unwrap();
        let reset = commands
            .iter()
            .find(|spec| spec.command == UiCommand::ResetDefault)
            .unwrap();
        assert_eq!(
            (insert.bindings[0].hint, insert.label),
            ("Ctrl+T", "Insert value")
        );
        assert_eq!(
            (reset.bindings[0].hint, reset.label),
            ("Ctrl+O", "Reset to default")
        );

        for (locale, insert_label, reset_label) in [
            (Locale::ZhCn, "插入值", "恢复默认值"),
            (Locale::ZhTw, "插入值", "恢復預設值"),
        ] {
            let mut session = FooterSession::default();
            let mut terminal = Terminal::new(TestBackend::new(100, 4)).unwrap();
            terminal
                .draw(|frame| {
                    let _ = session.render(
                        frame,
                        frame.area(),
                        &state,
                        locale,
                        FooterInputOwnership::default(),
                    );
                })
                .unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            let compact = rendered.replace(' ', "");
            assert!(
                compact.contains(&format!("Ctrl+T{insert_label}")),
                "{rendered}"
            );
            assert!(
                compact.contains(&format!("Ctrl+O{reset_label}")),
                "{rendered}"
            );
            assert!(!rendered.contains("Insert value"), "{rendered}");
            assert!(!rendered.contains("Reset to default"), "{rendered}");
        }
    }

    #[test]
    fn complete_footer_wheel_clamps_when_the_terminal_grows() {
        use skit_application::LibraryScan;
        use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};

        let state = LibraryState::from_scan(LibraryScan {
            entries: vec![EntrySummary {
                slug: Slug::parse("alpha").unwrap(),
                name: "Alpha".to_owned(),
                kind: EntryKind::parse("command").unwrap(),
                mode: StorageMode::Copy,
                description: String::new(),
                target: None,
            }],
            diagnostics: Vec::new(),
        });
        let mut session = FooterSession::default();
        let mut terminal = Terminal::new(TestBackend::new(20, 4)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(
                    frame,
                    frame.area(),
                    &state,
                    Locale::En,
                    FooterInputOwnership::default(),
                );
            })
            .unwrap();
        for _ in 0..4 {
            let _ = session.handle_mouse(&MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: session.viewport.x,
                row: session.viewport.y,
                modifiers: KeyModifiers::NONE,
            });
        }
        assert!(session.scroll.scroll_offset() > 0);
        let mut grown = Terminal::new(TestBackend::new(100, 16)).unwrap();
        grown
            .draw(|frame| {
                let hits = session.render(
                    frame,
                    frame.area(),
                    &state,
                    Locale::ZhCn,
                    FooterInputOwnership::default(),
                );
                assert!(!hits.is_empty());
            })
            .unwrap();
        assert_eq!(session.scroll.scroll_offset(), 0);
        assert!(!session.handle_mouse(&MouseEvent {
            kind: MouseEventKind::Moved,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        }));
    }

    #[test]
    fn one_line_settings_receipts_render_the_warning_in_three_locales() {
        for (locale, status, warning) in [
            (
                Locale::En,
                "Settings saved — Dropped GONE: it no longer exists in the script.",
                "Dropped GONE",
            ),
            (
                Locale::ZhCn,
                "设置已保存 — 已移除 GONE：它已不存在于脚本中。",
                "已移除 GONE",
            ),
            (
                Locale::ZhTw,
                "設定已儲存 — 已移除 GONE：它已不存在於指令稿中。",
                "已移除 GONE",
            ),
        ] {
            let mut state = LibraryState::default();
            state.update(skit_ui::Action::SetStatus(status.to_owned()));
            let mut session = FooterSession::default();
            let mut terminal = Terminal::new(TestBackend::new(100, 8)).unwrap();
            terminal
                .draw(|frame| {
                    let _ = session.render(
                        frame,
                        frame.area(),
                        &state,
                        locale,
                        FooterInputOwnership::default(),
                    );
                })
                .unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                rendered
                    .replace(' ', "")
                    .contains(&warning.replace(' ', "")),
                "{rendered}"
            );
        }
    }
}
#[test]
fn widget_owned_keys_are_removed_from_the_visible_shared_footer_contract() {
    assert!(
        advertised_bindings(
            &LibraryState::default(),
            UiCommand::Previous,
            FooterInputOwnership::default(),
        )
        .is_empty(),
        "a non-footer registry shortcut is not a printed footer binding"
    );
    let run = FooterInputOwnership {
        vertical_navigation: true,
        run_submit: true,
        preferences_input: false,
        escape: true,
    };
    let displayed = |context, command, ownership| {
        let spec = command_specs(context)
            .find(|spec| spec.command == command)
            .unwrap();
        displayed_bindings(*spec, ownership)
    };
    assert_eq!(
        displayed(CommandContext::RunForm, UiCommand::Submit, run)
            .iter()
            .map(|binding| binding.key)
            .collect::<Vec<_>>(),
        [UiKey::Character('r')]
    );
    assert_eq!(
        displayed(CommandContext::RunForm, UiCommand::FocusNext, run)
            .iter()
            .map(|binding| binding.key)
            .collect::<Vec<_>>(),
        [UiKey::Tab]
    );
    assert_eq!(
        displayed(CommandContext::RunForm, UiCommand::FocusPrevious, run)
            .iter()
            .map(|binding| binding.key)
            .collect::<Vec<_>>(),
        [UiKey::BackTab]
    );
    assert!(displayed(CommandContext::RunForm, UiCommand::Back, run).is_empty());

    let preferences = FooterInputOwnership {
        vertical_navigation: false,
        run_submit: false,
        preferences_input: true,
        escape: false,
    };
    for command in [UiCommand::ManageAgents, UiCommand::InstallAgentSkill] {
        assert!(
            displayed(CommandContext::Preferences, command, preferences).is_empty(),
            "{command:?} must not advertise a chord that the input owns"
        );
    }
}
