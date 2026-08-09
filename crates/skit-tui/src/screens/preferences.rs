//! Application-preference widgets.

use std::{collections::HashMap, fmt::Display};

use ratatui_core::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui_interact::{
    components::{
        Button, ButtonState, ButtonVariant, ListPicker, ListPickerState, ListPickerStyle,
        ScrollableContentState, Select, SelectAction, SelectState, handle_scrollable_content_key,
        handle_scrollable_content_mouse, handle_select_key, handle_select_mouse,
    },
    state::FocusManager,
    traits::{ClickRegion, ClickRegionRegistry},
};
use ratatui_widgets::{clear::Clear, paragraph::Paragraph, paragraph::Wrap};
use skit_application::AgentScope;
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, PreferencesField,
};
use skit_i18n::{Locale, Localize, format_text, text};
use skit_ui::{
    ChoicePresentation, PreferencesAction, PreferencesControl, PreferencesControlId,
    PreferencesControlKind, PreferencesDisplayText, PreferencesOption, PreferencesTextPlacement,
    PreferencesView,
};
use tui_input::{Input as LineInput, backend::crossterm::EventHandler as _};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    session::{radio_style, render_line_input, select_style},
    theme::{ACCENT, BOX_DIM, BOX_INDIGO, padded_panel},
};

/// Result of one Preferences widget event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PreferencesEventHandling {
    /// Dispatch a semantic action through the Preferences reducer.
    Action(PreferencesAction),
    /// The widget changed ephemeral state such as a cursor or scroll offset.
    Consumed,
    /// No Preferences control accepted the event.
    Ignored,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreferencesSignature(Vec<(PreferencesControlId, PreferencesControlShape)>);

#[derive(Clone, Debug, Eq, PartialEq)]
enum PreferencesControlShape {
    Text,
    Choice {
        options: Vec<String>,
        presentation: ChoicePresentation,
    },
    Button,
}

#[derive(Debug)]
enum PreferencesWidget {
    Input(LineInput),
    Choice {
        state: SelectState,
        values: Vec<String>,
        labels: Vec<String>,
        presentation: ChoicePresentation,
        buttons: Vec<ButtonState>,
        select_area: Rect,
        dropdown_regions: Vec<ClickRegion<SelectAction>>,
    },
    Button(ButtonState),
}

#[derive(Clone, Debug)]
enum PreferencesHit {
    Control(PreferencesControlId),
    Radio {
        id: PreferencesControlId,
        option: usize,
    },
}

#[derive(Clone, Debug)]
enum AgentSkillHit {
    Target(usize),
    Cancel,
}

#[derive(Clone, Debug)]
enum RenderItem {
    Spacer,
    Heading(String),
    Copy(String),
    Control(PreferencesControl),
}

#[derive(Clone, Debug)]
struct PositionedItem {
    start: usize,
    height: usize,
    item: RenderItem,
}

/// Ephemeral state for mature Preferences widgets.
#[derive(Debug, Default)]
pub(crate) struct PreferencesWidgetSession {
    signature: Option<PreferencesSignature>,
    widgets: HashMap<PreferencesControlId, PreferencesWidget>,
    focus: FocusManager<PreferencesControlId>,
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
    content_height: usize,
    control_areas: Vec<(PreferencesControlId, Rect)>,
    clicks: ClickRegionRegistry<PreferencesHit>,
    agent_picker: ListPickerState,
    agent_picker_height: usize,
    agent_cancel: ButtonState,
    agent_clicks: ClickRegionRegistry<AgentSkillHit>,
    agent_target_areas: Vec<(usize, Rect)>,
    agent_cancel_area: Option<Rect>,
    pending_ensure_focus: bool,
}

impl PreferencesWidgetSession {
    /// Render the complete Preferences workflow.
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &PreferencesView,
        locale: Locale,
    ) {
        self.sync(view, locale);
        self.clicks.clear();
        self.control_areas.clear();

        let block = padded_panel(text(locale, "Preferences").into_owned(), BOX_INDIGO);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.viewport = inner;
        self.visible_height = usize::from(inner.height);

        let items = layout_items(view, locale, inner.width);
        self.content_height = items
            .last()
            .map_or(0, |item| item.start.saturating_add(item.height));
        self.scroll
            .set_lines(vec![String::new(); self.content_height]);
        let maximum = self.maximum_scroll_offset();
        if self.scroll.scroll_offset() > maximum {
            self.scroll.set_scroll_offset(maximum);
        }
        if self.pending_ensure_focus
            && let Some(item) = items.iter().find(|item| {
                matches!(&item.item, RenderItem::Control(control) if control.id == view.focused())
            })
        {
            self.ensure_visible(item.start, item.height);
            self.pending_ensure_focus = false;
        }

        for item in &items {
            let Some(visible) = self.visible_rect(item.start, item.height) else {
                continue;
            };
            match &item.item {
                RenderItem::Spacer => {}
                RenderItem::Heading(value) => frame.render_widget(
                    Paragraph::new(value.as_str())
                        .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
                    visible,
                ),
                RenderItem::Copy(value) => frame.render_widget(
                    Paragraph::new(value.as_str())
                        .wrap(Wrap { trim: false })
                        .style(Style::default().fg(Color::DarkGray)),
                    visible,
                ),
                RenderItem::Control(control) => {
                    self.render_control(frame, visible, control, view, locale);
                }
            }
        }
        self.render_open_dropdowns(frame);
        if let Some(picker) = view.agent_skill_install() {
            self.render_agent_skill_picker(frame, area, picker, locale);
        } else {
            self.agent_clicks.clear();
            self.agent_target_areas.clear();
            self.agent_cancel_area = None;
        }
    }

    /// Dispatch one terminal event through the active Preferences widget.
    #[must_use]
    pub(crate) fn handle_event(
        &mut self,
        event: Event,
        view: &PreferencesView,
    ) -> PreferencesEventHandling {
        self.sync(view, Locale::En);
        if let Some(picker) = view.agent_skill_install() {
            return self.handle_agent_skill_event(event, picker);
        }
        let focused = view.focused();

        if let Event::Key(key) = &event
            && key.kind != KeyEventKind::Release
            && let Some(handling) = self.handle_open_select_key(focused, key)
        {
            return handling;
        }
        if let Event::Mouse(mouse) = &event {
            if matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            ) && handle_scrollable_content_mouse(
                &mut self.scroll,
                mouse,
                self.viewport,
                self.visible_height,
            )
            .is_some()
            {
                return PreferencesEventHandling::Consumed;
            }
            if let Some(handling) = self.handle_select_mouse(mouse) {
                return handling;
            }
            if let Some(hit) = self.clicks.handle_click(mouse.column, mouse.row).cloned() {
                return self.activate_hit(hit, view);
            }
            return PreferencesEventHandling::Ignored;
        }
        if let Event::Paste(value) = event {
            return self.handle_paste(focused, &value);
        }
        let Event::Key(key) = event else {
            return PreferencesEventHandling::Ignored;
        };
        if key.kind == KeyEventKind::Release {
            return PreferencesEventHandling::Ignored;
        }

        if let Some(PreferencesWidget::Input(state)) = self.widgets.get_mut(&focused) {
            let before = state.value().to_owned();
            if state.handle_event(&Event::Key(key)).is_some() {
                return if before == state.value() {
                    PreferencesEventHandling::Consumed
                } else {
                    input_action(focused, state.value().to_owned())
                };
            }
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('s'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                return PreferencesEventHandling::Action(PreferencesAction::Save);
            }
            (KeyCode::Esc, _) => {
                return PreferencesEventHandling::Action(PreferencesAction::Close);
            }
            (KeyCode::Tab, _) => return self.move_focus(true),
            (KeyCode::BackTab, _) => return self.move_focus(false),
            (KeyCode::PageUp | KeyCode::PageDown, _) => {
                let _ = handle_scrollable_content_key(&mut self.scroll, &key, self.visible_height);
                return PreferencesEventHandling::Consumed;
            }
            (KeyCode::Char('o'), modifiers)
                if modifiers.contains(KeyModifiers::CONTROL)
                    && !matches!(
                        self.widgets.get(&focused),
                        Some(PreferencesWidget::Input(_))
                    ) =>
            {
                return PreferencesEventHandling::Action(PreferencesAction::ManageAgents);
            }
            (KeyCode::Char('k'), modifiers)
                if modifiers.contains(KeyModifiers::CONTROL)
                    && !matches!(
                        self.widgets.get(&focused),
                        Some(PreferencesWidget::Input(_))
                    ) =>
            {
                return PreferencesEventHandling::Action(PreferencesAction::InstallAgentSkill);
            }
            _ => {}
        }

        match self.widgets.get_mut(&focused) {
            Some(PreferencesWidget::Choice {
                state,
                values,
                presentation: ChoicePresentation::Radio,
                ..
            }) => {
                let next = match key.code {
                    KeyCode::Right | KeyCode::Down => Some(true),
                    KeyCode::Left | KeyCode::Up => Some(false),
                    _ => None,
                };
                if let Some(forward) = next {
                    let current = state.selected_index.unwrap_or_default();
                    let selected = if forward {
                        current
                            .saturating_add(1)
                            .min(values.len().saturating_sub(1))
                    } else {
                        current.saturating_sub(1)
                    };
                    state.select(selected);
                    return values
                        .get(selected)
                        .map_or(PreferencesEventHandling::Consumed, |value| {
                            choice_action(focused, value)
                        });
                }
            }
            Some(PreferencesWidget::Button(_))
                if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) =>
            {
                return button_action(focused);
            }
            Some(PreferencesWidget::Input(_))
                if matches!(key.code, KeyCode::Down | KeyCode::Up) =>
            {
                return self.move_focus(key.code == KeyCode::Down);
            }
            Some(PreferencesWidget::Choice { .. })
            | Some(PreferencesWidget::Button(_))
            | Some(PreferencesWidget::Input(_))
            | None => {}
        }
        PreferencesEventHandling::Ignored
    }

    #[cfg(test)]
    fn control_area(&self, id: PreferencesControlId) -> Option<Rect> {
        self.control_areas
            .iter()
            .find_map(|(candidate, area)| (*candidate == id).then_some(*area))
    }

    #[cfg(test)]
    const fn agent_cancel_area(&self) -> Option<Rect> {
        self.agent_cancel_area
    }

    #[cfg(test)]
    fn agent_target_area(&self, index: usize) -> Option<Rect> {
        self.agent_target_areas
            .iter()
            .find_map(|(candidate, area)| (*candidate == index).then_some(*area))
    }

    #[cfg(test)]
    fn scroll_offset(&self) -> usize {
        self.scroll.scroll_offset()
    }

    fn maximum_scroll_offset(&self) -> usize {
        self.content_height.saturating_sub(self.visible_height)
    }

    fn render_agent_skill_picker(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        picker: &skit_ui::AgentSkillInstallView,
        locale: Locale,
    ) {
        self.agent_clicks.clear();
        self.agent_target_areas.clear();
        self.agent_cancel_area = None;
        self.agent_picker.set_total(picker.targets().len());
        if let Some(selected) = picker.selected() {
            self.agent_picker.select(selected);
        }

        let list_rows = picker.targets().len().clamp(1, 8);
        let desired_height = u16::try_from(list_rows.saturating_add(4)).unwrap_or(u16::MAX);
        let panel = centered(area, 76, desired_height.max(5));
        frame.render_widget(Clear, panel);
        let block = padded_panel(
            text(locale, "Teach an AI agent to use skit").into_owned(),
            BOX_INDIGO,
        );
        let inner = block.inner(panel);
        frame.render_widget(block, panel);

        let preview_height = u16::from(!picker.targets().is_empty() && inner.height >= 3);
        let cancel_height = u16::from(inner.height > 0);
        let [list_area, preview_area, cancel_area] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(preview_height),
            Constraint::Length(cancel_height),
        ])
        .areas(inner);
        self.agent_picker_height = usize::from(list_area.height);
        self.agent_picker
            .ensure_visible(self.agent_picker_height.max(1));

        if picker.targets().is_empty() {
            frame.render_widget(
                Paragraph::new(text(
                    locale,
                    "No agent directories detected (~/.claude, ~/.codex, ./.agents, …). Install by hand with: skit agent install --to DIR",
                ))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::DarkGray)),
                list_area,
            );
        } else {
            let labels = picker
                .targets()
                .iter()
                .map(|target| {
                    let scope = match target.scope {
                        AgentScope::User => text(locale, "user"),
                        AgentScope::Project => text(locale, "project"),
                    };
                    format!("{} ({scope})", target.name)
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                ListPicker::new(&labels, &self.agent_picker).style(ListPickerStyle {
                    selected_style: Style::default()
                        .fg(Color::Black)
                        .bg(ACCENT)
                        .add_modifier(Modifier::BOLD),
                    normal_style: Style::default().fg(Color::White),
                    indicator_style: Style::default().fg(ACCENT),
                    border_style: Style::default(),
                    indicator: "▶ ",
                    indicator_empty: "  ",
                    bordered: false,
                }),
                list_area,
            );
            for visible in 0..self.agent_picker_height {
                let index = usize::from(self.agent_picker.scroll).saturating_add(visible);
                if index >= picker.targets().len() {
                    break;
                }
                let target_area = Rect::new(
                    list_area.x,
                    list_area
                        .y
                        .saturating_add(u16::try_from(visible).unwrap_or(u16::MAX)),
                    list_area.width,
                    1,
                );
                self.agent_clicks
                    .register(target_area, AgentSkillHit::Target(index));
                self.agent_target_areas.push((index, target_area));
            }
            if let Some(target) = picker.selected_target() {
                frame.render_widget(
                    Paragraph::new(target.skills_dir().display().to_string())
                        .style(Style::default().fg(Color::DarkGray)),
                    preview_area,
                );
            }
        }

        if !cancel_area.is_empty() {
            let label = text(locale, "Cancel");
            let width = u16::try_from(label.as_ref().width().saturating_add(2))
                .unwrap_or(u16::MAX)
                .min(cancel_area.width);
            let button_area = Rect::new(cancel_area.x, cancel_area.y, width, 1);
            let region = Button::new(&label, &self.agent_cancel)
                .variant(ButtonVariant::SingleLine)
                .style(
                    ratatui_interact::components::ButtonStyle::new(ButtonVariant::SingleLine)
                        .focused(Color::White, ACCENT)
                        .unfocused(Color::White, BOX_DIM),
                )
                .render_stateful(button_area, frame.buffer_mut());
            self.agent_cancel_area = Some(region.area);
            self.agent_clicks
                .register(region.area, AgentSkillHit::Cancel);
        }
    }

    fn handle_agent_skill_event(
        &mut self,
        event: Event,
        picker: &skit_ui::AgentSkillInstallView,
    ) -> PreferencesEventHandling {
        let selected = picker.selected().unwrap_or_default();
        let selection = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => {
                    return PreferencesEventHandling::Action(
                        PreferencesAction::CloseAgentSkillTargets,
                    );
                }
                KeyCode::Enter if picker.selected().is_some() => {
                    return PreferencesEventHandling::Action(
                        PreferencesAction::ConfirmAgentSkillTarget,
                    );
                }
                KeyCode::Up => Some(selected.saturating_sub(1)),
                KeyCode::Down => Some(
                    selected
                        .saturating_add(1)
                        .min(picker.targets().len().saturating_sub(1)),
                ),
                KeyCode::Home => Some(0),
                KeyCode::End => Some(picker.targets().len().saturating_sub(1)),
                KeyCode::PageUp => Some(selected.saturating_sub(self.agent_picker_height.max(1))),
                KeyCode::PageDown => Some(
                    selected
                        .saturating_add(self.agent_picker_height.max(1))
                        .min(picker.targets().len().saturating_sub(1)),
                ),
                _ => return PreferencesEventHandling::Consumed,
            },
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) =>
            {
                Some(match mouse.kind {
                    MouseEventKind::ScrollUp => selected.saturating_sub(1),
                    MouseEventKind::ScrollDown => selected
                        .saturating_add(1)
                        .min(picker.targets().len().saturating_sub(1)),
                    _ => selected,
                })
            }
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                return match self
                    .agent_clicks
                    .handle_click(mouse.column, mouse.row)
                    .cloned()
                {
                    Some(AgentSkillHit::Target(index)) => PreferencesEventHandling::Action(
                        PreferencesAction::ActivateAgentSkillTarget(index),
                    ),
                    Some(AgentSkillHit::Cancel) => {
                        PreferencesEventHandling::Action(PreferencesAction::CloseAgentSkillTargets)
                    }
                    None => PreferencesEventHandling::Consumed,
                };
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Key(_)
            | Event::Resize(_, _) => return PreferencesEventHandling::Consumed,
        };
        selection.filter(|_| !picker.targets().is_empty()).map_or(
            PreferencesEventHandling::Consumed,
            |index| {
                PreferencesEventHandling::Action(PreferencesAction::SelectAgentSkillTarget(index))
            },
        )
    }

    fn sync(&mut self, view: &PreferencesView, locale: Locale) {
        let controls = view.controls();
        let signature = PreferencesSignature(
            controls
                .iter()
                .map(|control| (control.id, control_shape(control)))
                .collect(),
        );
        if self.signature.as_ref() != Some(&signature) {
            self.widgets = controls
                .iter()
                .map(|control| (control.id, widget(control, locale)))
                .collect();
            self.focus.clear();
            self.focus
                .register_all(controls.iter().map(|control| control.id));
            self.signature = Some(signature);
            self.pending_ensure_focus = true;
        } else {
            for control in &controls {
                if let Some(widget) = self.widgets.get_mut(&control.id) {
                    sync_widget(widget, control, locale);
                }
            }
        }
        if self.focus.current() != Some(&view.focused()) {
            self.focus.set(view.focused());
            self.pending_ensure_focus = true;
        }
        for (id, widget) in &mut self.widgets {
            set_widget_focus(widget, self.focus.is_focused(id));
        }
    }

    fn render_control(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        control: &PreferencesControl,
        view: &PreferencesView,
        locale: Locale,
    ) {
        self.control_areas.push((control.id, area));
        let focused = view.focused() == control.id;
        let label = text(locale, &control.label);
        let Some(widget) = self.widgets.get_mut(&control.id) else {
            return;
        };
        match widget {
            PreferencesWidget::Input(state) => {
                render_line_input(frame, area, state, false, focused, &label);
                if state.value().is_empty()
                    && let PreferencesControlKind::Text(model) = &control.kind
                {
                    let inner = Rect::new(
                        area.x.saturating_add(1),
                        area.y.saturating_add(1),
                        area.width.saturating_sub(2),
                        area.height.saturating_sub(2).min(1),
                    );
                    frame.render_widget(
                        Paragraph::new(text(locale, &model.placeholder))
                            .style(Style::default().fg(Color::DarkGray)),
                        inner,
                    );
                }
                self.clicks
                    .register(area, PreferencesHit::Control(control.id));
            }
            PreferencesWidget::Choice {
                state,
                labels,
                presentation: ChoicePresentation::Picker,
                select_area,
                ..
            } => {
                let region = Select::new(labels, state)
                    .label(&label)
                    .placeholder(&text(locale, "Select"))
                    .style(select_style())
                    .render_stateful(frame, area);
                *select_area = region.area;
                self.clicks
                    .register(region.area, PreferencesHit::Control(control.id));
            }
            PreferencesWidget::Choice {
                state,
                labels,
                presentation: ChoicePresentation::Radio,
                buttons,
                ..
            } => {
                let mut y = area.y;
                if !label.is_empty() {
                    frame.render_widget(
                        Paragraph::new(label.as_ref()).style(Style::default().fg(Color::White)),
                        Rect::new(area.x, y, area.width, 1),
                    );
                    y = y.saturating_add(1);
                }
                let mut x = area.x;
                for (index, (option_label, button)) in labels.iter().zip(buttons.iter()).enumerate()
                {
                    let wanted = u16::try_from(option_label.width().saturating_add(2))
                        .unwrap_or(u16::MAX)
                        .min(area.width.max(1));
                    if x > area.x && x.saturating_add(wanted) > area.right() {
                        x = area.x;
                        y = y.saturating_add(1);
                    }
                    if y >= area.bottom() {
                        break;
                    }
                    let option_area = Rect::new(x, y, wanted.min(area.right() - x), 1);
                    let region = Button::new(option_label, button)
                        .variant(ButtonVariant::Toggle)
                        .style(radio_style())
                        .render_stateful(option_area, frame.buffer_mut());
                    self.clicks.register(
                        region.area,
                        PreferencesHit::Radio {
                            id: control.id,
                            option: index,
                        },
                    );
                    x = x.saturating_add(wanted).saturating_add(1);
                }
                state.ensure_visible(1);
            }
            PreferencesWidget::Button(state) => {
                let shown = text(locale, &control.label);
                let region = Button::new(&shown, state)
                    .variant(ButtonVariant::SingleLine)
                    .style(
                        ratatui_interact::components::ButtonStyle::new(ButtonVariant::SingleLine)
                            .focused(Color::White, ACCENT)
                            .unfocused(Color::White, BOX_DIM),
                    )
                    .render_stateful(area, frame.buffer_mut());
                self.clicks
                    .register(region.area, PreferencesHit::Control(control.id));
                self.control_areas
                    .last_mut()
                    .expect("control area was inserted")
                    .1 = region.area;
            }
        }
    }

    fn render_open_dropdowns(&mut self, frame: &mut Frame) {
        let screen = frame.area();
        for widget in self.widgets.values_mut() {
            let PreferencesWidget::Choice {
                state,
                labels,
                presentation: ChoicePresentation::Picker,
                select_area,
                dropdown_regions,
                ..
            } = widget
            else {
                continue;
            };
            if state.is_open {
                *dropdown_regions = Select::new(labels, state)
                    .style(select_style())
                    .render_dropdown(frame, *select_area, screen);
            } else {
                dropdown_regions.clear();
            }
        }
    }

    fn handle_open_select_key(
        &mut self,
        focused: PreferencesControlId,
        key: &KeyEvent,
    ) -> Option<PreferencesEventHandling> {
        let PreferencesWidget::Choice {
            state,
            values,
            presentation: ChoicePresentation::Picker,
            ..
        } = self.widgets.get_mut(&focused)?
        else {
            return None;
        };
        let relevant = if state.is_open {
            matches!(
                key.code,
                KeyCode::Esc
                    | KeyCode::Enter
                    | KeyCode::Char(' ')
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            )
        } else {
            matches!(
                key.code,
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Down
            )
        };
        if !relevant {
            return None;
        }
        Some(match handle_select_key(key, state) {
            Some(SelectAction::Select(index)) => values
                .get(index)
                .map_or(PreferencesEventHandling::Consumed, |value| {
                    choice_action(focused, value)
                }),
            Some(SelectAction::Focus | SelectAction::Open | SelectAction::Close) | None => {
                PreferencesEventHandling::Consumed
            }
        })
    }

    fn handle_select_mouse(&mut self, mouse: &MouseEvent) -> Option<PreferencesEventHandling> {
        for (id, widget) in &mut self.widgets {
            let PreferencesWidget::Choice {
                state,
                values,
                presentation: ChoicePresentation::Picker,
                select_area,
                dropdown_regions,
                ..
            } = widget
            else {
                continue;
            };
            if !state.is_open {
                continue;
            }
            if let Some(action) = handle_select_mouse(mouse, state, *select_area, dropdown_regions)
            {
                return Some(match action {
                    SelectAction::Select(index) => values
                        .get(index)
                        .map_or(PreferencesEventHandling::Consumed, |value| {
                            choice_action(*id, value)
                        }),
                    SelectAction::Focus | SelectAction::Open | SelectAction::Close => {
                        PreferencesEventHandling::Consumed
                    }
                });
            }
        }
        None
    }

    fn handle_paste(
        &mut self,
        focused: PreferencesControlId,
        value: &str,
    ) -> PreferencesEventHandling {
        let Some(PreferencesWidget::Input(state)) = self.widgets.get_mut(&focused) else {
            return PreferencesEventHandling::Ignored;
        };
        for character in value.chars() {
            let _ = state.handle(tui_input::InputRequest::InsertChar(character));
        }
        input_action(focused, state.value().to_owned())
    }

    fn activate_hit(
        &mut self,
        hit: PreferencesHit,
        view: &PreferencesView,
    ) -> PreferencesEventHandling {
        match hit {
            PreferencesHit::Control(id) => match self.widgets.get_mut(&id) {
                Some(PreferencesWidget::Button(_)) => button_action(id),
                Some(PreferencesWidget::Choice {
                    state,
                    presentation: ChoicePresentation::Picker,
                    ..
                }) => {
                    state.open();
                    if view.focused() == id {
                        PreferencesEventHandling::Consumed
                    } else {
                        PreferencesEventHandling::Action(PreferencesAction::Focus(id))
                    }
                }
                Some(PreferencesWidget::Input(_))
                | Some(PreferencesWidget::Choice {
                    presentation: ChoicePresentation::Radio,
                    ..
                }) => PreferencesEventHandling::Action(PreferencesAction::Focus(id)),
                None => PreferencesEventHandling::Ignored,
            },
            PreferencesHit::Radio { id, option } => self
                .widgets
                .get(&id)
                .and_then(|widget| match widget {
                    PreferencesWidget::Choice { values, .. } => values.get(option),
                    PreferencesWidget::Input(_) | PreferencesWidget::Button(_) => None,
                })
                .map_or(PreferencesEventHandling::Ignored, |value| {
                    choice_action(id, value)
                }),
        }
    }

    fn move_focus(&mut self, forward: bool) -> PreferencesEventHandling {
        if forward {
            self.focus.next();
        } else {
            self.focus.prev();
        }
        self.focus
            .current()
            .copied()
            .map_or(PreferencesEventHandling::Consumed, |id| {
                PreferencesEventHandling::Action(PreferencesAction::Focus(id))
            })
    }

    fn ensure_visible(&mut self, start: usize, height: usize) {
        let offset = self.scroll.scroll_offset();
        let end = start.saturating_add(height);
        if start < offset {
            self.scroll.set_scroll_offset(start);
        } else if end > offset.saturating_add(self.visible_height) {
            self.scroll
                .set_scroll_offset(end.saturating_sub(self.visible_height));
        }
    }

    fn visible_rect(&self, start: usize, height: usize) -> Option<Rect> {
        let offset = self.scroll.scroll_offset();
        let viewport_end = offset.saturating_add(self.visible_height);
        let end = start.saturating_add(height);
        if end <= offset || start >= viewport_end {
            return None;
        }
        let clipped_start = start.max(offset);
        let clipped_end = end.min(viewport_end);
        Some(Rect::new(
            self.viewport.x,
            self.viewport.y.saturating_add(
                u16::try_from(clipped_start.saturating_sub(offset)).unwrap_or(u16::MAX),
            ),
            self.viewport.width,
            u16::try_from(clipped_end.saturating_sub(clipped_start)).unwrap_or(u16::MAX),
        ))
    }
}

fn layout_items(view: &PreferencesView, locale: Locale, width: u16) -> Vec<PositionedItem> {
    let mut positioned = Vec::new();
    let mut start = 0_usize;
    for (index, section) in view.sections().into_iter().enumerate() {
        if index > 0 {
            push_item(&mut positioned, &mut start, RenderItem::Spacer, 1);
        }
        push_item(
            &mut positioned,
            &mut start,
            RenderItem::Heading(format_display(locale, &section.title)),
            1,
        );
        if section.help_placement == PreferencesTextPlacement::BeforeControls {
            push_copy(&mut positioned, &mut start, locale, &section.help, width);
        }
        if section.status_placement == PreferencesTextPlacement::BeforeControls {
            for status in &section.status {
                push_copy(&mut positioned, &mut start, locale, status, width);
            }
        }
        for control in section.controls {
            let height = control_height(&control, locale, width);
            push_item(
                &mut positioned,
                &mut start,
                RenderItem::Control(control),
                height,
            );
            if view.error().is_some() && positioned.last().is_some_and(|item| {
                matches!(&item.item, RenderItem::Control(control) if control.id == view.focused())
            }) {
                let error = view
                    .error()
                    .expect("validation error was checked")
                    .message()
                    .localize(locale);
                push_item(
                    &mut positioned,
                    &mut start,
                    RenderItem::Copy(error),
                    1,
                );
            }
        }
        if section.status_placement == PreferencesTextPlacement::AfterControls {
            for status in &section.status {
                push_copy(&mut positioned, &mut start, locale, status, width);
            }
        }
        if section.help_placement == PreferencesTextPlacement::AfterControls {
            push_copy(&mut positioned, &mut start, locale, &section.help, width);
        }
    }
    positioned
}

fn push_copy(
    positioned: &mut Vec<PositionedItem>,
    start: &mut usize,
    locale: Locale,
    value: &PreferencesDisplayText,
    width: u16,
) {
    if value.key.is_empty() {
        return;
    }
    let shown = format_display(locale, value);
    let height = Paragraph::new(shown.as_str())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
        .max(1);
    push_item(positioned, start, RenderItem::Copy(shown), height);
}

fn push_item(
    positioned: &mut Vec<PositionedItem>,
    start: &mut usize,
    item: RenderItem,
    height: usize,
) {
    positioned.push(PositionedItem {
        start: *start,
        height,
        item,
    });
    *start = start.saturating_add(height);
}

fn control_height(control: &PreferencesControl, locale: Locale, width: u16) -> usize {
    match &control.kind {
        PreferencesControlKind::Text(_) => 3,
        PreferencesControlKind::Choice(choice)
            if choice.presentation == ChoicePresentation::Picker =>
        {
            3
        }
        PreferencesControlKind::Choice(choice) => usize::from(!control.label.is_empty())
            .saturating_add(radio_rows(&choice.options, locale, width).max(1)),
        PreferencesControlKind::Button => 1,
    }
}

fn radio_rows(options: &[PreferencesOption], locale: Locale, width: u16) -> usize {
    let mut rows = 1_usize;
    let mut used = 0_u16;
    for option in options {
        let label = text(locale, &option.label);
        let item = u16::try_from(label.as_ref().width().saturating_add(2))
            .unwrap_or(u16::MAX)
            .min(width.max(1));
        if used > 0 && used.saturating_add(item) > width {
            rows = rows.saturating_add(1);
            used = 0;
        }
        used = used.saturating_add(item).saturating_add(1);
    }
    rows
}

fn centered(area: Rect, maximum_width: u16, desired_height: u16) -> Rect {
    let [column] = Layout::horizontal([Constraint::Length(maximum_width.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [panel] = Layout::vertical([Constraint::Length(desired_height.min(area.height))])
        .flex(Flex::Center)
        .areas(column);
    panel
}

fn format_display(locale: Locale, value: &PreferencesDisplayText) -> String {
    if value.arguments.is_empty() {
        return text(locale, &value.key).into_owned();
    }
    let arguments = value
        .arguments
        .iter()
        .map(|argument| argument as &dyn Display)
        .collect::<Vec<_>>();
    format_text(locale, &value.key, &arguments)
}

fn control_shape(control: &PreferencesControl) -> PreferencesControlShape {
    match &control.kind {
        PreferencesControlKind::Text(_) => PreferencesControlShape::Text,
        PreferencesControlKind::Choice(choice) => PreferencesControlShape::Choice {
            options: choice
                .options
                .iter()
                .map(|option| option.value.clone())
                .collect(),
            presentation: choice.presentation,
        },
        PreferencesControlKind::Button => PreferencesControlShape::Button,
    }
}

fn widget(control: &PreferencesControl, locale: Locale) -> PreferencesWidget {
    match &control.kind {
        PreferencesControlKind::Text(model) => {
            PreferencesWidget::Input(LineInput::new(model.value.clone()))
        }
        PreferencesControlKind::Choice(choice) => {
            let selected = choice
                .options
                .iter()
                .position(|option| option.value == choice.selected);
            let mut state = selected.map_or_else(
                || SelectState::new(choice.options.len()),
                |index| SelectState::with_selected(choice.options.len(), index),
            );
            state.focused = false;
            PreferencesWidget::Choice {
                state,
                values: choice
                    .options
                    .iter()
                    .map(|option| option.value.clone())
                    .collect(),
                labels: choice
                    .options
                    .iter()
                    .map(|option| text(locale, &option.label).into_owned())
                    .collect(),
                presentation: choice.presentation,
                buttons: (0..choice.options.len())
                    .map(|index| ButtonState::toggled(selected == Some(index)))
                    .collect(),
                select_area: Rect::default(),
                dropdown_regions: Vec::new(),
            }
        }
        PreferencesControlKind::Button => PreferencesWidget::Button(ButtonState::enabled()),
    }
}

fn sync_widget(widget: &mut PreferencesWidget, control: &PreferencesControl, locale: Locale) {
    match (widget, &control.kind) {
        (PreferencesWidget::Input(state), PreferencesControlKind::Text(model))
            if state.value() != model.value =>
        {
            *state = LineInput::new(model.value.clone());
        }
        (
            PreferencesWidget::Choice {
                state,
                labels,
                buttons,
                ..
            },
            PreferencesControlKind::Choice(choice),
        ) => {
            let selected = choice
                .options
                .iter()
                .position(|option| option.value == choice.selected);
            state.selected_index = selected;
            state.highlighted_index = selected.unwrap_or_default();
            for (index, button) in buttons.iter_mut().enumerate() {
                button.toggled = selected == Some(index);
            }
            *labels = choice
                .options
                .iter()
                .map(|option| text(locale, &option.label).into_owned())
                .collect();
        }
        (PreferencesWidget::Input(_), _)
        | (PreferencesWidget::Choice { .. }, _)
        | (PreferencesWidget::Button(_), _) => {}
    }
}

fn set_widget_focus(widget: &mut PreferencesWidget, focused: bool) {
    match widget {
        PreferencesWidget::Input(_) => {}
        PreferencesWidget::Choice { state, buttons, .. } => {
            state.focused = focused;
            let active = state.selected_index.unwrap_or(state.highlighted_index);
            for (index, button) in buttons.iter_mut().enumerate() {
                button.set_focused(focused && index == active);
            }
            if !focused {
                state.close();
            }
        }
        PreferencesWidget::Button(state) => state.set_focused(focused),
    }
}

fn input_action(id: PreferencesControlId, value: String) -> PreferencesEventHandling {
    let action = match id {
        PreferencesControlId::Editor => PreferencesAction::SetEditor(value),
        PreferencesControlId::BashPath => PreferencesAction::SetBashPath(value),
        PreferencesControlId::PypiUrl => PreferencesAction::SetMirrorUrl {
            field: PreferencesField::PypiMirror,
            value,
        },
        PreferencesControlId::GithubUrl => PreferencesAction::SetMirrorUrl {
            field: PreferencesField::GithubMirror,
            value,
        },
        PreferencesControlId::NpmUrl => PreferencesAction::SetMirrorUrl {
            field: PreferencesField::NpmMirror,
            value,
        },
        PreferencesControlId::Language
        | PreferencesControlId::InteractiveForm
        | PreferencesControlId::AfterRun
        | PreferencesControlId::Javascript
        | PreferencesControlId::ManageAgents
        | PreferencesControlId::InstallAgentSkill
        | PreferencesControlId::MirrorMaster
        | PreferencesControlId::PypiChoice
        | PreferencesControlId::GithubChoice
        | PreferencesControlId::NpmChoice => return PreferencesEventHandling::Ignored,
    };
    PreferencesEventHandling::Action(action)
}

fn choice_action(id: PreferencesControlId, value: &str) -> PreferencesEventHandling {
    let action = match id {
        PreferencesControlId::Language => PreferencesAction::SetLanguage(value.to_owned()),
        PreferencesControlId::InteractiveForm => {
            PreferencesAction::SetInteractiveForm(if value == "plain" {
                InteractiveFormChoice::Plain
            } else {
                InteractiveFormChoice::Tui
            })
        }
        PreferencesControlId::AfterRun => PreferencesAction::SetAfterRun(if value == "stay" {
            AfterRunChoice::Stay
        } else {
            AfterRunChoice::Exit
        }),
        PreferencesControlId::Javascript => PreferencesAction::SetJavascript(match value {
            "deno" => JavascriptChoice::Deno,
            "bun" => JavascriptChoice::Bun,
            "node" => JavascriptChoice::Node,
            _ => JavascriptChoice::Automatic,
        }),
        PreferencesControlId::MirrorMaster => PreferencesAction::SetMirrorMaster(value == "on"),
        PreferencesControlId::PypiChoice => PreferencesAction::ChooseMirror {
            field: PreferencesField::PypiMirror,
            choice: mirror_choice(value),
        },
        PreferencesControlId::GithubChoice => PreferencesAction::ChooseMirror {
            field: PreferencesField::GithubMirror,
            choice: mirror_choice(value),
        },
        PreferencesControlId::NpmChoice => PreferencesAction::ChooseMirror {
            field: PreferencesField::NpmMirror,
            choice: mirror_choice(value),
        },
        PreferencesControlId::Editor
        | PreferencesControlId::BashPath
        | PreferencesControlId::ManageAgents
        | PreferencesControlId::InstallAgentSkill
        | PreferencesControlId::PypiUrl
        | PreferencesControlId::GithubUrl
        | PreferencesControlId::NpmUrl => return PreferencesEventHandling::Ignored,
    };
    PreferencesEventHandling::Action(action)
}

fn mirror_choice(value: &str) -> MirrorChoice {
    match value {
        "custom" => MirrorChoice::Custom,
        "off" => MirrorChoice::Off,
        preset => MirrorChoice::Preset(preset.to_owned()),
    }
}

fn button_action(id: PreferencesControlId) -> PreferencesEventHandling {
    match id {
        PreferencesControlId::ManageAgents => {
            PreferencesEventHandling::Action(PreferencesAction::ManageAgents)
        }
        PreferencesControlId::InstallAgentSkill => {
            PreferencesEventHandling::Action(PreferencesAction::InstallAgentSkill)
        }
        PreferencesControlId::Language
        | PreferencesControlId::Editor
        | PreferencesControlId::InteractiveForm
        | PreferencesControlId::AfterRun
        | PreferencesControlId::Javascript
        | PreferencesControlId::BashPath
        | PreferencesControlId::MirrorMaster
        | PreferencesControlId::PypiChoice
        | PreferencesControlId::PypiUrl
        | PreferencesControlId::GithubChoice
        | PreferencesControlId::GithubUrl
        | PreferencesControlId::NpmChoice
        | PreferencesControlId::NpmUrl => PreferencesEventHandling::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::{
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use skit_application::preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesSnapshot,
    };
    use skit_application::{AgentScope, AgentTarget};
    use skit_i18n::Locale;
    use skit_ui::{PreferencesAction, PreferencesControlId, PreferencesView};

    use super::{PreferencesEventHandling, PreferencesWidgetSession};
    use crate::theme::{ACCENT, BOX_INDIGO};

    fn view() -> PreferencesView {
        PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: Some("vim".to_owned()),
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runner_names: vec!["claude".to_owned(), "codex".to_owned()],
            mirror: MirrorConfiguration::default(),
        }))
    }

    fn draw(
        session: &mut PreferencesWidgetSession,
        view: &PreferencesView,
        width: u16,
        height: u16,
        locale: Locale,
    ) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| session.render(frame, frame.area(), view, locale))
            .unwrap();
        terminal
    }

    fn text(buffer: &Buffer) -> String {
        buffer
            .content()
            .chunks(usize::from(buffer.area.width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_backend_renders_the_complete_colored_preferences_surface() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let terminal = draw(&mut session, &view, 120, 44, Locale::En);
        let buffer = terminal.backend().buffer();
        let rendered = text(buffer);

        for expected in [
            "Preferences",
            "Interface language",
            "Currently in effect: en",
            "Empty means: vim (from $VISUAL / $EDITOR)",
            "Mini form — opens in place, fully clickable",
            "Quit skit — leave the run's output in the terminal",
            "2 agents configured: claude, codex",
            "Download mirrors (mainland-China acceleration)",
            "PyPI index (Python packages)",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected:?}\n{rendered}"
            );
        }
        assert!(buffer.content().iter().any(|cell| cell.fg == BOX_INDIGO));
        assert!(buffer.content().iter().any(|cell| cell.fg == ACCENT));
    }

    #[test]
    fn input_uses_a_real_cursor_and_emits_complete_unicode_values() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Editor));
        let _ = draw(&mut session, &view, 80, 24, Locale::En);

        for character in ['a', '\u{301}', '🧑'] {
            let PreferencesEventHandling::Action(action) = session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
                &view,
            ) else {
                panic!("text input must emit a reducer action");
            };
            view.update(action);
        }
        let terminal = draw(&mut session, &view, 80, 24, Locale::En);

        assert_eq!(view.draft().editor, "a\u{301}🧑");
        assert!(
            session
                .control_area(PreferencesControlId::Editor)
                .expect("visible editor")
                .contains(terminal.backend().cursor_position())
        );
    }

    #[test]
    fn mouse_buttons_and_keyboard_navigation_share_typed_actions() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        let _ = draw(&mut session, &view, 120, 44, Locale::En);
        let area = session
            .control_area(PreferencesControlId::ManageAgents)
            .expect("visible Manage agents button");
        let handling = session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
        );
        assert_eq!(
            handling,
            PreferencesEventHandling::Action(PreferencesAction::ManageAgents)
        );

        view.update(PreferencesAction::Focus(
            PreferencesControlId::InteractiveForm,
        ));
        let _ = draw(&mut session, &view, 120, 44, Locale::En);
        let handling = session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            &view,
        );
        assert_eq!(
            handling,
            PreferencesEventHandling::Action(PreferencesAction::SetInteractiveForm(
                InteractiveFormChoice::Plain,
            ))
        );
    }

    #[test]
    fn short_terminals_keep_the_whole_form_wheel_reachable() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let _ = draw(&mut session, &view, 52, 10, Locale::En);
        assert!(session.maximum_scroll_offset() > 0);

        let handling = session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 4,
                row: 5,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
        );
        assert_eq!(handling, PreferencesEventHandling::Consumed);
        assert!(session.scroll_offset() > 0);
    }

    #[test]
    fn agent_skill_picker_is_visible_and_uses_the_same_typed_keyboard_and_mouse_paths() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::PresentAgentSkillTargets(vec![
            AgentTarget {
                name: "claude".to_owned(),
                scope: AgentScope::User,
                base: PathBuf::from("/home/demo/.claude"),
            },
            AgentTarget {
                name: "codex".to_owned(),
                scope: AgentScope::Project,
                base: PathBuf::from("/work/.codex"),
            },
        ]));
        let terminal = draw(&mut session, &view, 100, 28, Locale::En);
        let rendered = text(terminal.backend().buffer());
        assert!(rendered.contains("Teach an AI agent to use skit"));
        assert!(rendered.contains("claude (user)"));
        assert!(rendered.contains("codex (project)"));

        let target = session.agent_target_area(0).expect("visible target row");
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: target.x,
                    row: target.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::ActivateAgentSkillTarget(0))
        );

        let handling = session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            &view,
        );
        assert_eq!(
            handling,
            PreferencesEventHandling::Action(PreferencesAction::SelectAgentSkillTarget(1))
        );
        view.update(PreferencesAction::SelectAgentSkillTarget(1));
        let handling = session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &view,
        );
        assert_eq!(
            handling,
            PreferencesEventHandling::Action(PreferencesAction::ConfirmAgentSkillTarget)
        );

        let _ = draw(&mut session, &view, 100, 28, Locale::En);
        let cancel = session.agent_cancel_area().expect("visible cancel button");
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: cancel.x,
                    row: cancel.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::CloseAgentSkillTargets)
        );
    }

    #[test]
    fn empty_agent_skill_picker_explains_the_manual_path_and_can_close() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::PresentAgentSkillTargets(Vec::new()));
        let terminal = draw(&mut session, &view, 72, 12, Locale::En);
        assert!(text(terminal.backend().buffer()).contains("skit agent install --to DIR"));
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::CloseAgentSkillTargets)
        );
    }
}
