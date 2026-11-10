//! Responsive command footer with persistent mature scrolling state.

use std::borrow::Cow;

use ratatui_core::{layout::Rect, terminal::Frame};
use ratatui_crossterm::crossterm::event::{KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui_interact::components::{
    Button, ButtonState, ButtonStyle, ButtonVariant, ScrollableContentState,
};
use ratatui_widgets::{
    block::Block,
    borders::{BorderType, Borders},
    paragraph::{Paragraph, Wrap},
};
use skit_i18n::{Locale, catalog, format_text, text};
use skit_ui::{
    CommandContext, LibraryState, Screen, UiBinding, UiCommand, UiCommandSpec, UiKey, command_specs,
};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    HitRegion, HitTarget,
    agent_review::{
        AgentReviewNode, AgentReviewSnapshotError, clicks as snapshot_clicks, local_key_value,
        node as snapshot_node, rect as snapshot_rect, scroll as snapshot_scroll,
        value as snapshot_value,
    },
    layout::ViewportProfile,
    local_action::LocalKey,
    pointer::{ClickDispatch, ClickOutcome, ClickTracker},
    theme,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FooterInputOwnership {
    pub(crate) vertical_navigation: bool,
    pub(crate) run_submit: bool,
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
    /// Return the button style one command chip paints with.
    pub(crate) fn button(&self) -> ButtonStyle {
        self.button.clone()
    }

    /// The chips of a dialog's action row.
    pub(crate) fn dialog() -> Self {
        Self {
            button: theme::dialog_footer_chip_style(),
        }
    }
}

impl Default for ActionFooterStyle {
    fn default() -> Self {
        Self {
            button: theme::footer_chip_style(),
        }
    }
}

/// Result of a mouse event sent to a local action footer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ActionFooterMouse<A> {
    /// The user clicked a visible command chip.
    Action(A),
    /// A primary press armed a command and needs a repaint-free consume.
    Armed,
    /// The footer scrolled to another row of commands.
    Scrolled,
    /// The event was outside the footer or had no effect.
    Ignored,
}

/// Persistent mature scroll and click state for a typed local action footer.
#[derive(Clone, Debug)]
pub(crate) struct ActionFooterSession<A: Clone + Eq> {
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
    clicks: ratatui_interact::traits::ClickRegionRegistry<A>,
    click: ClickTracker<A>,
    advertised: Vec<(Rect, LocalKey, A)>,
}

impl<A: Clone + Eq> Default for ActionFooterSession<A> {
    fn default() -> Self {
        Self {
            scroll: ScrollableContentState::default(),
            viewport: Rect::default(),
            visible_height: 0,
            clicks: ratatui_interact::traits::ClickRegionRegistry::default(),
            click: ClickTracker::default(),
            advertised: Vec::new(),
        }
    }
}

impl<A: Clone + Eq> ActionFooterSession<A> {
    /// Cancel an armed command before an owner or layout transition.
    pub(crate) fn cancel_click(&mut self) {
        self.click.cancel();
    }

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
        self.scroll
            .set_scroll_offset(self.scroll.scroll_offset().min(maximum_offset));

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
            // The button measures its label in characters; the chip area measures it in cells.
            theme::patch_chip_key(frame.buffer_mut(), chip_area, &key_hint);
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
                Paragraph::new(indicator).style(theme::footer_indicator()),
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
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            self.click.cancel();
            return if handle_footer_scroll(
                &mut self.scroll,
                mouse,
                self.viewport,
                self.visible_height,
            ) {
                ActionFooterMouse::Scrolled
            } else {
                ActionFooterMouse::Ignored
            };
        }
        let target = self.clicks.handle_click(mouse.column, mouse.row);
        match self.click.dispatch(mouse, target) {
            ClickDispatch::Captured(outcome) => match outcome {
                ClickOutcome::Activated(action) => ActionFooterMouse::Action(action),
                ClickOutcome::Armed => ActionFooterMouse::Armed,
                ClickOutcome::Ignored => ActionFooterMouse::Ignored,
            },
            ClickDispatch::Unowned => ActionFooterMouse::Ignored,
        }
    }

    pub(crate) fn advertised(&self) -> &[(Rect, LocalKey, A)] {
        &self.advertised
    }

    /// Each visible chip, and whether a primary press holds it.
    pub(crate) fn pointer_targets(&self) -> Vec<(Rect, bool)> {
        self.advertised
            .iter()
            .map(|(area, _, action)| (*area, self.click.pressed() == Some(action)))
            .collect()
    }
}

impl<A> ActionFooterSession<A>
where
    A: Clone + Eq + serde::Serialize,
{
    pub(crate) fn agent_review_snapshot(
        &self,
    ) -> Result<AgentReviewNode, AgentReviewSnapshotError> {
        let Self {
            scroll,
            viewport,
            visible_height,
            clicks,
            click,
            advertised,
        } = self;
        let advertised = advertised
            .iter()
            .map(|(area, key, action)| {
                Ok(serde_json::json!({
                    "area": snapshot_rect(*area),
                    "key": local_key_value(*key),
                    "action": snapshot_value("action_footer.advertised.action", action)?,
                }))
            })
            .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()?;
        Ok(snapshot_node(
            "action_footer",
            [
                ("scroll", snapshot_scroll(scroll)),
                ("viewport", snapshot_rect(*viewport)),
                ("visible_height", serde_json::json!(visible_height)),
                ("clicks", snapshot_clicks("action_footer.clicks", clicks)?),
                (
                    "pressed",
                    click
                        .pressed()
                        .map_or(Ok(serde_json::Value::Null), |action| {
                            snapshot_value("action_footer.pressed", action)
                        })?,
                ),
                ("advertised", serde_json::json!(advertised)),
            ],
        ))
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
        if x.saturating_add(chip_width) > width {
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

#[cfg(test)]
mod agent_review_tests {
    use skit_ui::HealthAction;

    use super::*;

    #[test]
    fn action_footer_snapshot_keeps_scroll_click_and_advertised_order() {
        let mut session = ActionFooterSession::<HealthAction>::default();
        session.scroll.set_lines(vec![String::new(), String::new()]);
        session.scroll.set_scroll_offset(1);
        session.viewport = Rect::new(1, 2, 3, 4);
        session.visible_height = 2;
        session
            .clicks
            .register(Rect::new(1, 2, 1, 1), HealthAction::Back);
        let _ = session.click.update(
            &MouseEvent {
                kind: ratatui_crossterm::crossterm::event::MouseEventKind::Down(
                    ratatui_crossterm::crossterm::event::MouseButton::Left,
                ),
                column: 1,
                row: 2,
                modifiers: ratatui_crossterm::crossterm::event::KeyModifiers::NONE,
            },
            Some(&HealthAction::Back),
        );
        session.advertised.push((
            Rect::new(1, 2, 1, 1),
            LocalKey::Control('b'),
            HealthAction::Back,
        ));
        let json = serde_json::to_string(&session.agent_review_snapshot().unwrap()).unwrap();
        assert!(json.contains("advertised"));
        assert!(json.contains("character"));
        assert!(json.contains("back"));
        assert!(json.contains("pressed"));
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
    profile: ViewportProfile,
    state: &LibraryState,
    locale: Locale,
    ownership: FooterInputOwnership,
) -> u16 {
    if is_suppressed(state) || profile.width() == 0 || profile.height() == 0 {
        return 0;
    }
    let decorated = !profile.is_short_or_tiny();
    let inner_width = profile
        .width()
        .saturating_sub(if decorated { 2 } else { 0 });
    if inner_width == 0 {
        return 0;
    }
    let (_, rows) = chips(state, locale, inner_width, ownership);
    let library = matches!(
        state.command_context(),
        CommandContext::LibraryBrowse | CommandContext::LibrarySearch
    );
    let visible_rows = rows.min(profile.footer_row_budget(library));
    u16::try_from(visible_rows)
        .unwrap_or(u16::MAX)
        .saturating_add(note_rows(state, locale, inner_width))
        .saturating_add(if decorated { 2 } else { 0 })
        .max(1)
}

/// Return the smallest footer that can show one command row and its note without overlap.
pub(crate) fn minimum_height(
    profile: ViewportProfile,
    state: &LibraryState,
    locale: Locale,
    ownership: FooterInputOwnership,
) -> u16 {
    if is_suppressed(state) || profile.width() == 0 || profile.height() == 0 {
        return 0;
    }
    minimum_content_height(state, locale, profile.width(), ownership)
}

/// Return the smallest complete bordered footer, or zero when decoration is unavailable.
pub(crate) fn decorated_minimum_height(
    profile: ViewportProfile,
    state: &LibraryState,
    locale: Locale,
    ownership: FooterInputOwnership,
) -> u16 {
    if is_suppressed(state)
        || profile.is_short_or_tiny()
        || profile.width() <= 2
        || profile.height() == 0
    {
        return 0;
    }
    let inner_width = profile.width().saturating_sub(2);
    minimum_content_height(state, locale, inner_width, ownership).saturating_add(2)
}

fn minimum_content_height(
    state: &LibraryState,
    locale: Locale,
    inner_width: u16,
    ownership: FooterInputOwnership,
) -> u16 {
    let (_, rows) = chips(state, locale, inner_width, ownership);
    u16::from(rows > 0).saturating_add(note_rows(state, locale, inner_width))
}

/// Report whether a screen that hides the shared footer must still show the status.
///
/// Health, Add, the dialogs, and the agent editor draw their own keys, so they hide the shared
/// footer, and the status line goes with it. A warning or an error stays until the next key, and
/// that key clears it, so the user must see it on the screen where it happened. A receipt or a
/// note stays hidden there: it clears itself, and the screen shows the result of its own work.
pub(crate) fn shows_problem(state: &LibraryState) -> bool {
    state.status().is_some() && state.status_kind().display_time().is_none()
}

/// Return the preferred, smallest, and smallest bordered height of the footer that shows only a
/// problem under a screen that hides the shared footer.
///
/// All three are zero when no problem shows. The border follows the rule of the shared footer.
pub(crate) fn problem_heights(
    profile: ViewportProfile,
    state: &LibraryState,
    locale: Locale,
) -> (u16, u16, u16) {
    if !shows_problem(state) || profile.width() == 0 || profile.height() == 0 {
        return (0, 0, 0);
    }
    let bare = note_rows(state, locale, profile.width());
    if profile.is_short_or_tiny() || profile.width() <= 2 {
        return (bare, bare, 0);
    }
    let bordered = note_rows(state, locale, profile.width() - 2).saturating_add(2);
    (bordered, bare, bordered)
}

/// Draw the footer of a screen that hides the shared footer: the problem status and no keys.
pub(crate) fn render_problem(
    frame: &mut Frame,
    area: Rect,
    state: &LibraryState,
    locale: Locale,
    decorated: bool,
) {
    let block = Block::default()
        .borders(if decorated {
            Borders::ALL
        } else {
            Borders::NONE
        })
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_status(frame, inner, state, locale);
}

/// Draw the status at the bottom of `inner`, and report whether a status shows.
fn render_status(frame: &mut Frame, inner: Rect, state: &LibraryState, locale: Locale) -> bool {
    let Some(status) = shown_status(state, locale) else {
        return false;
    };
    let rows = note_rows(state, locale, inner.width).min(inner.height);
    let status_area = Rect::new(
        inner.x,
        inner.bottom().saturating_sub(rows),
        inner.width,
        rows,
    );
    // The glyph and the color say what kind of news this is before the words do. A long status
    // wraps onto a second row rather than losing its end.
    frame.render_widget(
        Paragraph::new(theme::status_kind_line(&status, state.status_kind()))
            .wrap(Wrap { trim: false }),
        status_area,
    );
    true
}

pub(crate) fn is_suppressed(state: &LibraryState) -> bool {
    matches!(state.screen(), Screen::Add(_) | Screen::Health(_))
        || matches!(
            state.screen(),
            Screen::Preferences(view) if view.agent_skill_install().is_some()
        )
        || matches!(
            state.modal(),
            Some(
                skit_ui::ModalState::ConfirmRemove { .. }
                    | skit_ui::ModalState::ConfirmDiscardChanges
                    | skit_ui::ModalState::RunnerEditor { .. }
            )
        )
}

impl FooterSession {
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
            "footer",
            [
                ("scroll", snapshot_scroll(scroll)),
                ("viewport", snapshot_rect(*viewport)),
                ("visible_height", serde_json::json!(visible_height)),
            ],
        )
    }

    #[cfg(test)]
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        state: &LibraryState,
        locale: Locale,
        ownership: FooterInputOwnership,
    ) -> Vec<HitRegion> {
        self.render_with_decoration(frame, area, state, locale, area.height > 2, ownership)
    }

    pub(crate) fn render_with_decoration(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        state: &LibraryState,
        locale: Locale,
        decorated: bool,
        ownership: FooterInputOwnership,
    ) -> Vec<HitRegion> {
        if area.width == 0 || area.height == 0 {
            self.viewport = area;
            self.visible_height = 0;
            return Vec::new();
        }
        let compact = !decorated;
        let base_block = Block::default()
            .borders(if compact { Borders::NONE } else { Borders::ALL })
            .border_type(BorderType::Rounded);
        let inner = base_block.inner(area);
        let note_rows = note_rows(state, locale, inner.width);
        let note_height = usize::from(note_rows);
        self.visible_height = usize::from(inner.height).saturating_sub(note_height);
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
        self.scroll
            .set_scroll_offset(self.scroll.scroll_offset().min(maximum_offset));
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
                .style(theme::footer_chip_style())
                .render_stateful(chip_area, frame.buffer_mut());
            // The button measures its label in characters; the chip area measures it in cells.
            theme::patch_chip_key(frame.buffer_mut(), chip_area, &chip.key);
            hits.push(HitRegion {
                rect: region.area,
                action: HitTarget::Command(chip.command),
            });
        }

        if !render_status(frame, inner, state, locale) && matches!(state.screen(), Screen::Library)
        {
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
            if x.saturating_add(width) > inner_width {
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
            // Version 0.4's shared navigation hint is two pills that name BOTH keys for each
            // direction (`src/skit/tui_footer.py:82-94`): a footer that advertises only Tab
            // strands anyone who tabs one field too far. Version 0.4 left the pills without
            // words; every footer here names them, as the runner editor always did, so the same
            // chip reads the same on every screen.
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
                return Some((keys, text(locale, spec.label).into_owned(), spec.command));
            }
            // The footer and the help screen name the same quit keys: every chord of the command.
            if spec.command == UiCommand::Quit {
                let keys = bindings
                    .iter()
                    .map(|binding| binding.hint)
                    .collect::<Vec<_>>()
                    .join("/");
                return Some((keys, text(locale, spec.label).into_owned(), spec.command));
            }
            // Enter reads as the verb of the control that owns it, never as a bare "Submit".
            let label = match (spec.command, state.form(), state.preferences()) {
                (UiCommand::Submit, Some(form), _) => text(locale, &form.submit_label),
                (UiCommand::Submit, None, Some(view)) => text(
                    locale,
                    view.activation().map_or(spec.label, |(verb, _)| verb),
                ),
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
    // Quit has two plain keys, `q` and Esc, and the help screen lists both.
    if spec.command == UiCommand::Quit {
        return spec.bindings.to_vec();
    }
    spec.bindings.first().copied().into_iter().collect()
}

/// The most rows that one status takes under the chips.
const STATUS_ROWS: u16 = 2;

/// The status line as the footer shows it.
///
/// Reducer refusals are complete catalog keys. Host receipts already contain localized text and
/// user values; do not translate their fragments again.
fn shown_status<'a>(state: &'a LibraryState, locale: Locale) -> Option<Cow<'a, str>> {
    let status = state.status()?;
    Some(if catalog().iter().any(|row| row.english == status) {
        text(locale, status)
    } else {
        Cow::Borrowed(status)
    })
}

/// The rows of the note under the chips: the entry count of the Library, or a status.
///
/// A status wider than the footer wraps onto a second row and stops there.
fn note_rows(state: &LibraryState, locale: Locale, inner_width: u16) -> u16 {
    let Some(status) = shown_status(state, locale) else {
        return u16::from(matches!(state.screen(), Screen::Library));
    };
    let glyph =
        theme::status_of_kind(state.status_kind()).map_or(0, |(glyph, _)| glyph.width() + 1);
    let width = status.width().saturating_add(glyph);
    if inner_width == 0 || width <= usize::from(inner_width) {
        1
    } else {
        STATUS_ROWS
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
    use skit_application::{Diagnostic, DiagnosticCode, LibraryScan};
    use skit_domain::{
        EntryKind, EntrySummary, Slug, StorageMode,
        parameters::{ParamDecl, ParameterValue},
    };
    use skit_ui::{Action, AddWorkflowState, RunFormView, Screen};

    use super::*;
    use crate::screens::management::{health_footer_items, runner_editor_footer_items};

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

    fn release(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
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

    fn command_library(diagnostics: Vec<Diagnostic>) -> LibraryState {
        LibraryState::from_scan(LibraryScan {
            entries: vec![EntrySummary {
                slug: Slug::parse("alpha").unwrap(),
                name: "Alpha".to_owned(),
                kind: EntryKind::parse("command").unwrap(),
                mode: StorageMode::Copy,
                description: String::new(),
                target: None,
            }],
            diagnostics,
        })
    }

    fn row_text(terminal: &Terminal<TestBackend>, row: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect()
    }

    #[test]
    fn zero_height_footer_clears_geometry_without_painting_a_status_row() {
        let mut session = FooterSession {
            viewport: Rect::new(2, 1, 8, 1),
            visible_height: 1,
            ..FooterSession::default()
        };
        session.scroll.set_lines(vec![String::new(); 3]);
        session.scroll.set_scroll_offset(2);
        let state = LibraryState::default();
        let mut terminal = Terminal::new(TestBackend::new(20, 2)).unwrap();

        let mut hits = Vec::new();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new("BODY-SENTINEL"),
                    Rect::new(0, 0, frame.area().width, 1),
                );
                hits = session.render_with_decoration(
                    frame,
                    Rect::new(0, 1, 20, 0),
                    &state,
                    Locale::En,
                    false,
                    FooterInputOwnership::default(),
                );
            })
            .unwrap();

        assert!(hits.is_empty());
        assert_eq!(session.viewport, Rect::new(0, 1, 20, 0));
        assert_eq!(session.visible_height, 0);
        assert_eq!(session.scroll.scroll_offset(), 2);
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .starts_with("BODY-SENTINEL")
        );
        assert!(!session.handle_mouse(&scroll_down(2, 1)));
    }

    #[test]
    fn action_footer_wraps_every_chip_and_keeps_each_visible_chip_clickable() {
        let items = [
            ActionFooterItem::new(LocalKey::Control('1'), "First action", TestAction::First),
            ActionFooterItem::new(LocalKey::Control('2'), "Second action", TestAction::Second),
            ActionFooterItem::new(LocalKey::Control('3'), "Third action", TestAction::Third),
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
            assert_ne!(
                session.handle_mouse(&click(1, u16::try_from(row).unwrap())),
                ActionFooterMouse::Action(action.clone())
            );
            assert_eq!(
                session.handle_mouse(&release(1, u16::try_from(row).unwrap())),
                ActionFooterMouse::Action(action)
            );
        }
    }

    #[test]
    fn action_footer_scrolls_to_chips_that_do_not_fit_the_viewport() {
        let items = [
            ActionFooterItem::new(LocalKey::Control('1'), "First action", TestAction::First),
            ActionFooterItem::new(LocalKey::Control('2'), "Second action", TestAction::Second),
            ActionFooterItem::new(LocalKey::Control('3'), "Third action", TestAction::Third),
            ActionFooterItem::new(LocalKey::Control('4'), "Fourth action", TestAction::Fourth),
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
        assert_ne!(
            session.handle_mouse(&click(1, 0)),
            ActionFooterMouse::Action(TestAction::Fourth)
        );
        assert_eq!(
            session.handle_mouse(&release(1, 0)),
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
    fn action_footer_never_paints_or_owns_the_row_after_its_half_open_area() {
        let items = [
            ActionFooterItem::new(LocalKey::Control('1'), "First action", TestAction::First),
            ActionFooterItem::new(LocalKey::Control('2'), "Second action", TestAction::Second),
        ];
        assert_eq!(action_footer_required_height(22, &items), 2);

        let mut terminal = Terminal::new(TestBackend::new(22, 2)).unwrap();
        let mut session = ActionFooterSession::default();
        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new("BODY-SENTINEL"), Rect::new(0, 1, 22, 1));
                session.render(
                    frame,
                    Rect::new(0, 0, 22, 1),
                    &items,
                    ActionFooterStyle::default(),
                );
            })
            .unwrap();

        assert!(row_text(&terminal, 1).starts_with("BODY-SENTINEL"));
        assert_eq!(session.handle_mouse(&click(1, 0)), ActionFooterMouse::Armed);
        assert_eq!(
            session.handle_mouse(&release(1, 0)),
            ActionFooterMouse::Action(TestAction::First)
        );
        assert_eq!(
            session.handle_mouse(&click(1, 1)),
            ActionFooterMouse::Ignored
        );
        assert_eq!(
            session.handle_mouse(&release(1, 1)),
            ActionFooterMouse::Ignored
        );
    }

    #[test]
    fn action_footer_release_outside_cancels_the_arm_before_a_late_release() {
        let items = [ActionFooterItem::new(
            LocalKey::Control('1'),
            "First action",
            TestAction::First,
        )];
        let mut terminal = Terminal::new(TestBackend::new(22, 2)).unwrap();
        let mut session = ActionFooterSession::default();
        terminal
            .draw(|frame| {
                session.render(
                    frame,
                    Rect::new(0, 0, 22, 1),
                    &items,
                    ActionFooterStyle::default(),
                );
            })
            .unwrap();

        assert_eq!(session.handle_mouse(&click(1, 0)), ActionFooterMouse::Armed);
        assert_eq!(
            session.handle_mouse(&release(1, 1)),
            ActionFooterMouse::Ignored
        );
        assert_eq!(
            session.handle_mouse(&release(1, 0)),
            ActionFooterMouse::Ignored,
            "a late release resurrected an action cancelled outside the footer"
        );
    }

    #[test]
    fn action_footer_keeps_a_chip_that_ends_exactly_at_the_content_boundary() {
        let items = [
            ActionFooterItem::new(LocalKey::Control('1'), "A", TestAction::First),
            ActionFooterItem::new(LocalKey::Control('2'), "B", TestAction::Second),
        ];
        assert_eq!(action_footer_required_height(24, &items), 1);
        assert_eq!(action_footer_required_height(23, &items), 2);

        let mut terminal = Terminal::new(TestBackend::new(24, 1)).unwrap();
        let mut session = ActionFooterSession::default();
        terminal
            .draw(|frame| {
                session.render(frame, frame.area(), &items, ActionFooterStyle::default());
            })
            .unwrap();
        assert_eq!(
            session.handle_mouse(&click(13, 0)),
            ActionFooterMouse::Armed
        );
        assert_eq!(
            session.handle_mouse(&release(13, 0)),
            ActionFooterMouse::Action(TestAction::Second)
        );
    }

    #[test]
    fn action_footer_indicator_reports_top_middle_bottom_and_no_overflow() {
        let items = [
            ActionFooterItem::new(LocalKey::Control('1'), "First action", TestAction::First),
            ActionFooterItem::new(LocalKey::Control('2'), "Second action", TestAction::Second),
            ActionFooterItem::new(LocalKey::Control('3'), "Third action", TestAction::Third),
            ActionFooterItem::new(LocalKey::Control('4'), "Fourth action", TestAction::Fourth),
        ];
        let mut terminal = Terminal::new(TestBackend::new(22, 1)).unwrap();
        let mut session = ActionFooterSession::default();
        let mut render = |session: &mut ActionFooterSession<TestAction>| {
            terminal
                .draw(|frame| {
                    session.render(frame, frame.area(), &items, ActionFooterStyle::default());
                })
                .unwrap();
            terminal.backend().buffer()[(20, 0)].symbol().to_owned()
        };

        assert_eq!(render(&mut session), "↓");
        assert_eq!(
            session.handle_mouse(&scroll_down(1, 0)),
            ActionFooterMouse::Scrolled
        );
        assert_eq!(render(&mut session), "↕");
        for _ in 0..8 {
            let _ = session.handle_mouse(&scroll_down(1, 0));
        }
        assert_eq!(render(&mut session), "↑");

        let mut grown = Terminal::new(TestBackend::new(22, 4)).unwrap();
        grown
            .draw(|frame| {
                session.render(frame, frame.area(), &items, ActionFooterStyle::default());
            })
            .unwrap();
        assert_eq!(session.scroll.scroll_offset(), 0);
        assert_eq!(grown.backend().buffer()[(20, 0)].symbol(), " ");
    }

    #[test]
    fn action_footer_group_starts_on_a_new_row() {
        let items = [
            ActionFooterItem::new(LocalKey::Control('1'), "One", TestAction::First),
            ActionFooterItem::new_group(LocalKey::Control('2'), "Two", TestAction::Second),
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
                        let pressed = session.handle_mouse(&click(x, y));
                        if pressed == ActionFooterMouse::Armed
                            && let ActionFooterMouse::Action(action) =
                                session.handle_mouse(&release(x, y))
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
        let state = command_library(Vec::new());
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
    fn compact_footer_reserves_the_indicator_column_and_clips_wheel_ownership() {
        let mut state = command_library(Vec::new());
        state.update(Action::BeginSearch);
        let mut session = FooterSession::default();
        let mut terminal = Terminal::new(TestBackend::new(5, 2)).unwrap();
        let mut hits = Vec::new();
        terminal
            .draw(|frame| {
                hits = session.render_with_decoration(
                    frame,
                    frame.area(),
                    &state,
                    Locale::En,
                    false,
                    FooterInputOwnership::default(),
                );
            })
            .unwrap();

        assert_eq!(terminal.backend().buffer()[(4, 0)].symbol(), "↓");
        assert!(hits.iter().all(|hit| hit.rect.right() <= 4));
        assert!(session.handle_mouse(&scroll_down(3, 0)));
        assert!(!session.handle_mouse(&scroll_down(4, 0)));
    }

    #[test]
    fn compact_footer_keeps_a_global_chip_at_its_exact_width_boundary() {
        let mut state = command_library(Vec::new());
        state.update(Action::BeginSearch);
        let (_, rows) = chips(&state, Locale::En, 31, FooterInputOwnership::default());
        assert_eq!(rows, 1);

        let mut session = FooterSession::default();
        let mut terminal = Terminal::new(TestBackend::new(31, 2)).unwrap();
        let mut hits = Vec::new();
        terminal
            .draw(|frame| {
                hits = session.render_with_decoration(
                    frame,
                    frame.area(),
                    &state,
                    Locale::En,
                    false,
                    FooterInputOwnership::default(),
                );
            })
            .unwrap();

        assert!(hits.iter().any(|hit| {
            matches!(hit.action, HitTarget::Command(UiCommand::LeaveSearch)) && hit.rect.y == 0
        }));
    }

    #[test]
    fn footer_diagnostic_note_uses_the_documented_strict_width_breakpoint() {
        let diagnostic = Diagnostic::plain(
            DiagnosticCode::CorruptMetadata,
            Some("bad".to_owned()),
            "bad TOML".to_owned(),
        );
        for (width, expected) in [(50, false), (51, true)] {
            let state = command_library(vec![diagnostic.clone()]);
            let mut session = FooterSession::default();
            let mut terminal = Terminal::new(TestBackend::new(width, 2)).unwrap();
            terminal
                .draw(|frame| {
                    let _ = session.render_with_decoration(
                        frame,
                        frame.area(),
                        &state,
                        Locale::En,
                        false,
                        FooterInputOwnership::default(),
                    );
                })
                .unwrap();
            assert_eq!(
                row_text(&terminal, 1).contains("damaged entries hidden"),
                expected
            );
        }

        let state = command_library(Vec::new());
        let mut session = FooterSession::default();
        let mut terminal = Terminal::new(TestBackend::new(51, 2)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render_with_decoration(
                    frame,
                    frame.area(),
                    &state,
                    Locale::En,
                    false,
                    FooterInputOwnership::default(),
                );
            })
            .unwrap();
        assert!(!row_text(&terminal, 1).contains("damaged entries hidden"));
    }

    #[test]
    fn footer_scroll_owns_only_its_half_open_viewport() {
        let mut scroll = ScrollableContentState::default();
        scroll.set_lines(vec![String::new(); 10]);
        scroll.set_scroll_offset(2);
        let viewport = Rect::new(2, 3, 4, 2);

        for (column, row) in [(1, 3), (6, 3), (2, 2), (2, 5)] {
            assert!(!handle_footer_scroll(
                &mut scroll,
                &scroll_down(column, row),
                viewport,
                2
            ));
            assert_eq!(scroll.scroll_offset(), 2);
        }
        assert!(handle_footer_scroll(
            &mut scroll,
            &scroll_down(2, 3),
            viewport,
            2
        ));
        assert_eq!(scroll.scroll_offset(), 4);
    }

    #[test]
    fn footer_minimums_distinguish_suppression_geometry_and_command_content() {
        let state = command_library(Vec::new());
        let normal = ViewportProfile::new(Rect::new(0, 0, 100, 16));
        let ownership = FooterInputOwnership::default();
        assert_eq!(minimum_height(normal, &state, Locale::En, ownership), 2);
        assert_eq!(
            decorated_minimum_height(normal, &state, Locale::En, ownership),
            4
        );
        assert!(required_height(normal, &state, Locale::En, ownership) >= 4);

        for area in [Rect::new(0, 0, 0, 16), Rect::new(0, 0, 100, 0)] {
            let profile = ViewportProfile::new(area);
            assert_eq!(required_height(profile, &state, Locale::En, ownership), 0);
            assert_eq!(minimum_height(profile, &state, Locale::En, ownership), 0);
            assert_eq!(
                decorated_minimum_height(profile, &state, Locale::En, ownership),
                0
            );
        }
        for area in [
            Rect::new(0, 0, 100, 15),
            Rect::new(0, 0, 2, 16),
            Rect::new(0, 0, 100, 0),
        ] {
            assert_eq!(
                decorated_minimum_height(ViewportProfile::new(area), &state, Locale::En, ownership,),
                0
            );
        }

        let mut suppressed = LibraryState::default();
        suppressed.update(Action::Present(Screen::Add(Box::new(
            AddWorkflowState::new(Vec::new()),
        ))));
        for minimum in [
            required_height(normal, &suppressed, Locale::En, ownership),
            minimum_height(normal, &suppressed, Locale::En, ownership),
            decorated_minimum_height(normal, &suppressed, Locale::En, ownership),
        ] {
            assert_eq!(minimum, 0);
        }
        assert_eq!(
            minimum_content_height(&suppressed, Locale::En, 80, ownership),
            0
        );
        suppressed.update(Action::SetStatus {
            kind: skit_ui::StatusKind::Info,
            message: "Saved".to_owned(),
        });
        assert_eq!(
            minimum_content_height(&suppressed, Locale::En, 80, ownership),
            1
        );
    }

    #[test]
    fn normal_height_non_library_footer_uses_its_three_command_row_budget() {
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
        let profile = ViewportProfile::new(Rect::new(0, 0, 20, 16));
        let ownership = FooterInputOwnership::default();
        assert_eq!(required_height(profile, &state, Locale::En, ownership), 5);

        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        let mut session = FooterSession::default();
        let mut hits = Vec::new();
        terminal
            .draw(|frame| {
                hits = session.render_with_decoration(
                    frame,
                    frame.area(),
                    &state,
                    Locale::En,
                    true,
                    ownership,
                );
            })
            .unwrap();
        let rows = hits
            .iter()
            .map(|hit| hit.rect.y)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn status_translation_preserves_values_and_localizes_complete_catalog_keys() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
            let receipt = format_text(
                locale,
                "Installed the skit Agent Skill: {}",
                &[&"/tmp/on/off/agent/SKILL.md"],
            );
            for (status, expected) in [
                (receipt.clone(), receipt),
                (
                    "A name is required.".to_owned(),
                    text(locale, "A name is required.").into_owned(),
                ),
            ] {
                let mut state = LibraryState::default();
                state.update(skit_ui::Action::SetStatus {
                    kind: skit_ui::StatusKind::Info,
                    message: status,
                });
                let mut session = FooterSession::default();
                let mut terminal = Terminal::new(TestBackend::new(120, 8)).unwrap();
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
                        .contains(&expected.replace(' ', "")),
                    "{locale:?}: {rendered}"
                );
            }
        }
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
                "设置已保存 — 已剔除 GONE:脚本中已找不到。",
                "已剔除 GONE",
            ),
            (
                Locale::ZhTw,
                "設定已儲存 — 已剔除 GONE:腳本中已找不到。",
                "已剔除 GONE",
            ),
        ] {
            let mut state = LibraryState::default();
            state.update(skit_ui::Action::SetStatus {
                kind: skit_ui::StatusKind::Info,
                message: status.to_owned(),
            });
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
}
