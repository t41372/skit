//! Application-preference widgets.

use std::{cmp::Ordering, collections::HashMap, fmt::Display};

use ratatui_core::{
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::Style,
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui_interact::{
    components::{
        Button, ButtonState, ButtonVariant, ListPicker, ListPickerState, ScrollableContentState,
        Select, SelectAction, SelectState, handle_scrollable_content_key,
        handle_scrollable_content_mouse, handle_select_key,
    },
    state::FocusManager,
    traits::{ClickRegion, ClickRegionRegistry},
};
use ratatui_widgets::{clear::Clear, paragraph::Paragraph, paragraph::Wrap};
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, PreferencesField,
    RunnerDraftMarker, RunnerDraftRow, RunnerDraftState, ThemeChoice, runner_row_taken_by_its_key,
};
use skit_application::runner_management::{EditableArgvDialect, join_editable_argv};
use skit_application::{AgentScope, AgentTarget};
use skit_i18n::{Locale, Localize, format_text, text};
use skit_ui::{
    ChoicePresentation, PreferencesAction, PreferencesControl, PreferencesControlId,
    PreferencesControlKind, PreferencesDisplayText, PreferencesRunnerListControl,
    PreferencesTextPlacement, PreferencesView,
};
use tui_input::{Input as LineInput, backend::crossterm::EventHandler as _};
use unicode_width::UnicodeWidthStr as _;

use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    RunnerChip, ScreenFocusInventory, ScreenTarget, ScreenTargetError, ScreenTargetHit,
    ScreenTargetInventory,
    agent_review::{
        AgentReviewNode, AgentReviewSnapshotError, button as snapshot_button,
        focus as snapshot_focus, list_picker as snapshot_list_picker, node as snapshot_node,
        path_value as snapshot_path, rect as snapshot_rect, scroll as snapshot_scroll,
        select as snapshot_select, value as snapshot_value,
    },
    footer::ActionFooterStyle,
    pointer::{ClickOutcome, ClickTracker, EditableGeometry, is_primary_down},
    rowclip::RowClip,
    session::{radio_option_width, render_line_input_band, render_radio_option},
    theme::{self, Panel, Status, padded_panel},
    viewport::AlignmentSignature,
};

/// The cells each unbordered control keeps on its left for the focus marker.
const FOCUS_GUTTER_CELLS: u16 = 2;

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

/// Result from the blocking agent-install overlay, which accepts every event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AgentSkillOverlayEventHandling {
    /// Dispatch a semantic action through the Preferences reducer.
    Action(PreferencesAction),
    /// Keep the event inside the modal overlay.
    Consumed,
}

#[cfg(test)]
mod agent_review_tests {
    use super::*;

    #[test]
    fn widget_hashmap_insertion_order_does_not_change_snapshot_bytes() {
        let mut first = PreferencesWidgetSession::default();
        first.widgets.insert(
            PreferencesControlId::Language,
            PreferencesWidget::Button(ButtonState::enabled()),
        );
        first.widgets.insert(
            PreferencesControlId::Editor,
            PreferencesWidget::Input(LineInput::new("editor".to_owned())),
        );
        first.editables.insert(
            PreferencesControlId::Language,
            EditableGeometry::new(Rect::new(0, 0, 4, 1), 0, false),
        );
        first.editables.insert(
            PreferencesControlId::Editor,
            EditableGeometry::new(Rect::new(0, 1, 4, 1), 1, false),
        );

        let mut second = PreferencesWidgetSession::default();
        second.widgets.insert(
            PreferencesControlId::Editor,
            PreferencesWidget::Input(LineInput::new("editor".to_owned())),
        );
        second.widgets.insert(
            PreferencesControlId::Language,
            PreferencesWidget::Button(ButtonState::enabled()),
        );
        second.editables.insert(
            PreferencesControlId::Editor,
            EditableGeometry::new(Rect::new(0, 1, 4, 1), 1, false),
        );
        second.editables.insert(
            PreferencesControlId::Language,
            EditableGeometry::new(Rect::new(0, 0, 4, 1), 0, false),
        );

        let first = serde_json::to_vec(&first.agent_review_snapshot().unwrap()).unwrap();
        let second = serde_json::to_vec(&second.agent_review_snapshot().unwrap()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn preference_snapshot_covers_every_widget_signature_and_hit_variant() {
        let mut session = PreferencesWidgetSession {
            signature: Some(PreferencesSignature(vec![
                (PreferencesControlId::Editor, PreferencesControlShape::Text),
                (
                    PreferencesControlId::Language,
                    PreferencesControlShape::Choice {
                        options: vec!["en".to_owned()],
                        presentation: ChoicePresentation::Picker,
                    },
                ),
                (
                    PreferencesControlId::NewRunner,
                    PreferencesControlShape::Button,
                ),
            ])),
            ..PreferencesWidgetSession::default()
        };
        session.widgets.insert(
            PreferencesControlId::Language,
            PreferencesWidget::Choice {
                state: SelectState::with_selected(1, 0),
                values: vec!["en".to_owned()],
                labels: vec!["English".to_owned()],
                presentation: ChoicePresentation::Radio,
                buttons: vec![ButtonState::toggled(true)],
                select_area: Some(Rect::new(0, 0, 4, 1)),
                dropdown_panel: Some(Rect::new(0, 1, 4, 2)),
                dropdown_regions: vec![
                    ClickRegion::new(Rect::new(0, 0, 1, 1), SelectAction::Focus),
                    ClickRegion::new(Rect::new(1, 0, 1, 1), SelectAction::Open),
                    ClickRegion::new(Rect::new(2, 0, 1, 1), SelectAction::Close),
                    ClickRegion::new(Rect::new(3, 0, 1, 1), SelectAction::Select(0)),
                ],
            },
        );
        session
            .focus
            .register_all([PreferencesControlId::Editor, PreferencesControlId::Language]);
        session
            .control_areas
            .push((PreferencesControlId::Language, Rect::new(0, 0, 4, 1)));
        session.clicks.register(
            Rect::new(0, 0, 1, 1),
            PreferencesHit::Control(PreferencesControlId::Language),
        );
        session.clicks.register(
            Rect::new(1, 0, 1, 1),
            PreferencesHit::Radio {
                id: PreferencesControlId::Language,
                option: 0,
            },
        );
        session.clicks.register(
            Rect::new(2, 0, 1, 1),
            PreferencesHit::Dropdown {
                id: PreferencesControlId::Language,
                option: 0,
            },
        );
        session
            .clicks
            .register(Rect::new(3, 0, 1, 1), PreferencesHit::RunnerRow(0));
        session.clicks.register(
            Rect::new(4, 0, 1, 1),
            PreferencesHit::RunnerChip {
                index: 0,
                chip: RunnerChip::Edit,
            },
        );
        session.runner_row_areas.push((0, Rect::new(0, 3, 4, 1)));
        session
            .runner_chip_areas
            .push((0, RunnerChip::Remove, Rect::new(2, 3, 2, 1)));
        session
            .agent_clicks
            .register(Rect::new(0, 1, 1, 1), AgentSkillHit::Target(0));
        session
            .agent_clicks
            .register(Rect::new(1, 1, 1, 1), AgentSkillHit::Cancel);
        session.agent_target_areas.push((0, Rect::new(0, 1, 1, 1)));
        session.agent_list_area = Rect::new(0, 1, 4, 2);
        session.agent_cancel_area = Some(Rect::new(1, 1, 1, 1));
        session.agent_signature = Some(vec![AgentTarget {
            name: "codex".to_owned(),
            scope: AgentScope::User,
            base: std::path::PathBuf::from("/agent"),
        }]);
        session.editables.insert(
            PreferencesControlId::Editor,
            EditableGeometry::new(Rect::new(0, 2, 4, 1), 2, false),
        );
        assert!(AlignmentSignature::update(
            &mut session.alignment,
            PreferencesControlId::Language,
            Rect::new(0, 0, 80, 24),
            (5, 1, 2),
        ));
        let press = ratatui_crossterm::crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(ratatui_crossterm::crossterm::event::MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            session.click.update(
                &press,
                Some(&PreferencesHit::Dropdown {
                    id: PreferencesControlId::Language,
                    option: 0,
                }),
            ),
            ClickOutcome::Armed,
        );
        assert_eq!(
            session
                .agent_click
                .update(&press, Some(&AgentSkillHit::Target(0))),
            ClickOutcome::Armed,
        );
        session.pending_ensure_focus = true;
        let json = serde_json::to_string(&session.agent_review_snapshot().unwrap()).unwrap();
        for expected in [
            "choice",
            "radio",
            "dropdown",
            "dropdown_panel",
            "dropdown_regions",
            "runner_row",
            "runner_chip",
            "runner_row_areas",
            "runner_chip_areas",
            "agent_clicks",
            "agent_signature",
            "alignment",
            "editables",
        ] {
            assert!(json.contains(expected), "missing {expected}");
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
struct PreferencesSignature(Vec<(PreferencesControlId, PreferencesControlShape)>);

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
enum PreferencesControlShape {
    Text,
    Choice {
        options: Vec<String>,
        presentation: ChoicePresentation,
    },
    Button,
    RunnerList {
        rows: usize,
    },
}

#[derive(Clone, Debug)]
enum PreferencesWidget {
    Input(LineInput),
    RunnerList(PreferencesRunnerListControl),
    Choice {
        state: SelectState,
        values: Vec<String>,
        labels: Vec<String>,
        presentation: ChoicePresentation,
        buttons: Vec<ButtonState>,
        select_area: Option<Rect>,
        dropdown_panel: Option<Rect>,
        dropdown_regions: Vec<ClickRegion<SelectAction>>,
    },
    Button(ButtonState),
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
enum PreferencesHit {
    Control(PreferencesControlId),
    Radio {
        id: PreferencesControlId,
        option: usize,
    },
    Dropdown {
        id: PreferencesControlId,
        option: usize,
    },
    /// One agent row, which the click moves the cursor to.
    RunnerRow(usize),
    /// One command chip of the agent-list cursor row.
    RunnerChip {
        index: usize,
        chip: RunnerChip,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
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
#[derive(Clone, Debug, Default)]
pub(crate) struct PreferencesWidgetSession {
    signature: Option<PreferencesSignature>,
    widgets: HashMap<PreferencesControlId, PreferencesWidget>,
    focus: FocusManager<PreferencesControlId>,
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
    content_height: usize,
    control_areas: Vec<(PreferencesControlId, Rect)>,
    runner_row_areas: Vec<(usize, Rect)>,
    runner_chip_areas: Vec<(usize, RunnerChip, Rect)>,
    editables: HashMap<PreferencesControlId, EditableGeometry>,
    clicks: ClickRegionRegistry<PreferencesHit>,
    click: ClickTracker<PreferencesHit>,
    agent_picker: ListPickerState,
    agent_picker_height: usize,
    agent_list_area: Rect,
    agent_cancel: ButtonState,
    agent_clicks: ClickRegionRegistry<AgentSkillHit>,
    agent_click: ClickTracker<AgentSkillHit>,
    agent_signature: Option<Vec<AgentTarget>>,
    agent_target_areas: Vec<(usize, Rect)>,
    agent_cancel_area: Option<Rect>,
    pending_ensure_focus: bool,
    alignment: Option<AlignmentSignature<PreferencesControlId, (usize, usize, usize)>>,
}

impl PreferencesWidgetSession {
    pub(crate) fn screen_target_inventory(
        &self,
        view: &PreferencesView,
    ) -> Result<ScreenTargetInventory, ScreenTargetError> {
        if let Some(picker) = view.agent_skill_install() {
            if self.agent_signature.as_deref() != Some(picker.targets()) {
                return Err(ScreenTargetError::StaleSession);
            }
            let available = picker
                .targets()
                .iter()
                .map(|target| ScreenTarget::AgentSkill {
                    name: target.name.clone(),
                    scope: target.scope,
                })
                .collect::<Vec<_>>();
            let hits = self
                .agent_target_areas
                .iter()
                .filter(|(_, rect)| !rect.is_empty())
                .map(|(index, rect)| {
                    let target = picker
                        .targets()
                        .get(*index)
                        .ok_or(ScreenTargetError::StaleSession)?;
                    Ok(ScreenTargetHit {
                        target: ScreenTarget::AgentSkill {
                            name: target.name.clone(),
                            scope: target.scope,
                        },
                        rect: *rect,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(ScreenTargetInventory {
                available,
                focus: None,
                hits,
            });
        }

        let controls = view.controls();
        let signature = PreferencesSignature(
            controls
                .iter()
                .map(|control| (control.id, control_shape(control)))
                .collect(),
        );
        if self.signature.as_ref() != Some(&signature) {
            return Err(ScreenTargetError::StaleSession);
        }
        let order = self
            .focus
            .elements()
            .iter()
            .copied()
            .map(ScreenTarget::Preferences)
            .collect::<Vec<_>>();
        let current = self.focus.current().copied().map(ScreenTarget::Preferences);
        let dropdowns = self
            .widgets
            .values()
            .filter_map(|widget| match widget {
                PreferencesWidget::Choice {
                    state,
                    presentation: ChoicePresentation::Picker,
                    dropdown_panel,
                    ..
                } if state.is_open => *dropdown_panel,
                _ => None,
            })
            .collect::<Vec<_>>();
        let visible = |rect: &Rect| {
            !rect.is_empty()
                && dropdowns
                    .iter()
                    .all(|dropdown| rect.intersection(*dropdown).is_empty())
        };
        let rows = view.draft().runner_rows();
        let mut hits = self
            .control_areas
            .iter()
            // The agent list publishes one target per row, so the list rect is not a target.
            .filter(|(id, rect)| *id != PreferencesControlId::Runners && visible(rect))
            .map(|(id, rect)| ScreenTargetHit {
                target: ScreenTarget::Preferences(*id),
                rect: *rect,
            })
            .collect::<Vec<_>>();
        for (index, rect) in self
            .runner_row_areas
            .iter()
            .filter(|(_, rect)| visible(rect))
        {
            hits.push(ScreenTargetHit {
                target: runner_row_target(rows, *index)?,
                rect: *rect,
            });
        }
        for (index, chip, rect) in self
            .runner_chip_areas
            .iter()
            .filter(|(_, _, rect)| visible(rect))
        {
            hits.push(ScreenTargetHit {
                target: ScreenTarget::RunnerChip {
                    row: *index,
                    chip: *chip,
                },
                rect: *rect,
            });
        }
        let mut available = order.clone();
        available.extend(
            (0..rows.len())
                .map(|index| runner_row_target(rows, index))
                .collect::<Result<Vec<_>, _>>()?,
        );
        // Only the focused list paints chips, so only it advertises them.
        if view.focused() == PreferencesControlId::Runners {
            let cursor = view.runner_cursor();
            available.extend(
                rows.get(cursor)
                    .map(|row| runner_row_chips(row, runner_row_taken_by_its_key(rows, cursor)))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|chip| ScreenTarget::RunnerChip {
                        row: cursor,
                        chip: chip.chip,
                    }),
            );
        }
        Ok(ScreenTargetInventory {
            available,
            focus: Some(ScreenFocusInventory { current, order }),
            hits,
        })
    }

    pub(crate) fn cancel_underlay_click(&mut self) {
        self.click.cancel();
    }

    pub(crate) fn cancel_click(&mut self) {
        self.cancel_underlay_click();
        self.agent_click.cancel();
    }

    #[cfg(test)]
    pub(crate) fn perturb_agent_review_state(&mut self) {
        self.visible_height = self.visible_height.saturating_add(1);
    }

    pub(crate) fn agent_review_snapshot(
        &self,
    ) -> Result<AgentReviewNode, AgentReviewSnapshotError> {
        let Self {
            signature,
            widgets,
            focus,
            scroll,
            viewport,
            visible_height,
            content_height,
            control_areas,
            runner_row_areas,
            runner_chip_areas,
            editables,
            clicks,
            click,
            agent_picker,
            agent_picker_height,
            agent_list_area,
            agent_cancel,
            agent_clicks,
            agent_click,
            agent_signature,
            agent_target_areas,
            agent_cancel_area,
            pending_ensure_focus,
            alignment,
        } = self;
        let mut widgets = widgets
            .iter()
            .map(|(id, widget)| {
                let id_value = snapshot_value("preferences.widget.id", id)?;
                let state = match widget {
                    PreferencesWidget::Input(input) => serde_json::json!({
                        "kind": "input",
                        "state": snapshot_value("preferences.widget.input", input)?,
                    }),
                    PreferencesWidget::Choice {
                        state,
                        values,
                        labels,
                        presentation,
                        buttons,
                        select_area,
                        dropdown_panel,
                        dropdown_regions,
                    } => {
                        let presentation =
                            snapshot_value("preferences.widget.presentation", presentation)?;
                        let dropdown_regions = dropdown_regions
                            .iter()
                            .map(|region| {
                                let action = match region.data {
                                    SelectAction::Focus => serde_json::json!("focus"),
                                    SelectAction::Open => serde_json::json!("open"),
                                    SelectAction::Close => serde_json::json!("close"),
                                    SelectAction::Select(index) => {
                                        serde_json::json!({"select": index})
                                    }
                                };
                                serde_json::json!({
                                    "area": snapshot_rect(region.area),
                                    "action": action,
                                })
                            })
                            .collect::<Vec<_>>();
                        serde_json::json!({
                            "kind": "choice",
                            "state": snapshot_select(state),
                            "values": values,
                            "labels": labels,
                            "presentation": presentation,
                            "buttons": buttons.iter().map(snapshot_button).collect::<Vec<_>>(),
                            "select_area": select_area
                                .map(snapshot_rect)
                                .unwrap_or(serde_json::Value::Null),
                            "dropdown_panel": dropdown_panel
                                .map(snapshot_rect)
                                .unwrap_or(serde_json::Value::Null),
                            "dropdown_regions": dropdown_regions,
                        })
                    }
                    PreferencesWidget::Button(state) => serde_json::json!({
                        "kind": "button",
                        "state": snapshot_button(state),
                    }),
                    PreferencesWidget::RunnerList(list) => serde_json::json!({
                        "kind": "runner_list",
                        "state": snapshot_value("preferences.widget.runner_list", list)?,
                    }),
                };
                Ok((
                    id_value.to_string(),
                    serde_json::json!({"id": id_value, "state": state}),
                ))
            })
            .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()?;
        widgets.sort_by(|left, right| left.0.cmp(&right.0));
        let widgets = widgets
            .into_iter()
            .map(|(_, widget)| widget)
            .collect::<Vec<_>>();
        let control_areas = control_areas
            .iter()
            .map(|(id, area)| {
                Ok(serde_json::json!({
                    "id": snapshot_value("preferences.control_area.id", id)?,
                    "area": snapshot_rect(*area),
                }))
            })
            .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()?;
        let mut editables = editables
            .iter()
            .map(|(id, geometry)| {
                let id_value = snapshot_value("preferences.editable.id", id)?;
                Ok((
                    id_value.to_string(),
                    serde_json::json!({
                        "id": id_value,
                        "geometry": editable_geometry_snapshot(*geometry),
                    }),
                ))
            })
            .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()?;
        editables.sort_by(|left, right| left.0.cmp(&right.0));
        let editables = editables
            .into_iter()
            .map(|(_, editable)| editable)
            .collect::<Vec<_>>();
        let click = click
            .pressed()
            .map(preferences_hit_snapshot)
            .transpose()?
            .unwrap_or(serde_json::Value::Null);
        let agent_click = agent_click
            .pressed()
            .map(agent_hit_snapshot)
            .unwrap_or(serde_json::Value::Null);
        let agent_signature = agent_signature
            .as_ref()
            .map(|targets| {
                targets
                    .iter()
                    .map(agent_target_snapshot)
                    .collect::<Vec<_>>()
            })
            .map(serde_json::Value::Array)
            .unwrap_or(serde_json::Value::Null);
        let alignment = alignment
            .as_ref()
            .map(|alignment| {
                Ok(serde_json::json!({
                    "focus": snapshot_value(
                        "preferences.alignment.focus",
                        alignment.focus(),
                    )?,
                    "viewport_width": alignment.viewport_width(),
                    "viewport_height": alignment.viewport_height(),
                    "reflow": snapshot_value(
                        "preferences.alignment.reflow",
                        alignment.reflow(),
                    )?,
                }))
            })
            .transpose()?
            .unwrap_or(serde_json::Value::Null);
        let agent_target_areas = agent_target_areas
            .iter()
            .map(|(index, area)| serde_json::json!({"index": index, "area": snapshot_rect(*area)}))
            .collect::<Vec<_>>();
        let runner_row_areas = runner_row_areas
            .iter()
            .map(|(index, area)| serde_json::json!({"index": index, "area": snapshot_rect(*area)}))
            .collect::<Vec<_>>();
        Ok(snapshot_node(
            "preferences",
            [
                (
                    "signature",
                    signature
                        .as_ref()
                        .map(preferences_signature_snapshot)
                        .transpose()?
                        .unwrap_or(serde_json::Value::Null),
                ),
                ("widgets", serde_json::json!(widgets)),
                ("focus", snapshot_focus("preferences.focus", focus)?),
                ("scroll", snapshot_scroll(scroll)),
                ("viewport", snapshot_rect(*viewport)),
                ("visible_height", serde_json::json!(visible_height)),
                ("content_height", serde_json::json!(content_height)),
                ("control_areas", serde_json::json!(control_areas)),
                ("runner_row_areas", serde_json::json!(runner_row_areas)),
                (
                    "runner_chip_areas",
                    runner_chip_areas_snapshot(runner_chip_areas)?,
                ),
                ("editables", serde_json::json!(editables)),
                ("clicks", preferences_clicks_snapshot(clicks)?),
                ("click", click),
                ("agent_picker", snapshot_list_picker(agent_picker)),
                (
                    "agent_picker_height",
                    serde_json::json!(agent_picker_height),
                ),
                ("agent_list_area", snapshot_rect(*agent_list_area)),
                ("agent_cancel", snapshot_button(agent_cancel)),
                ("agent_clicks", agent_clicks_snapshot(agent_clicks)),
                ("agent_click", agent_click),
                ("agent_signature", agent_signature),
                ("agent_target_areas", serde_json::json!(agent_target_areas)),
                (
                    "agent_cancel_area",
                    agent_cancel_area
                        .map(snapshot_rect)
                        .unwrap_or(serde_json::Value::Null),
                ),
                (
                    "pending_ensure_focus",
                    serde_json::json!(pending_ensure_focus),
                ),
                ("alignment", alignment),
            ],
        ))
    }

    pub(crate) fn focused_owns_vertical_navigation(&self, view: &PreferencesView) -> bool {
        matches!(
            self.widgets.get(&view.focused()),
            Some(PreferencesWidget::Choice { .. } | PreferencesWidget::RunnerList(_))
        )
    }

    pub(crate) fn focused_dropdown_is_open(&self, view: &PreferencesView) -> bool {
        matches!(
            self.widgets.get(&view.focused()),
            Some(PreferencesWidget::Choice {
                state,
                presentation: ChoicePresentation::Picker,
                ..
            }) if state.is_open
        )
    }

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
        self.editables.clear();
        self.control_areas.clear();
        self.runner_row_areas.clear();
        self.runner_chip_areas.clear();
        for widget in self.widgets.values_mut() {
            if let PreferencesWidget::Choice {
                presentation: ChoicePresentation::Picker,
                select_area,
                dropdown_panel,
                dropdown_regions,
                ..
            } = widget
            {
                *select_area = None;
                *dropdown_panel = None;
                dropdown_regions.clear();
            }
        }

        let block = padded_panel(text(locale, "Preferences").into_owned(), Panel::Preferences);
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
        self.scroll
            .set_scroll_offset(self.scroll.scroll_offset().min(maximum));
        let focused_item = items.iter().find(|item| {
            matches!(&item.item, RenderItem::Control(control) if control.id == view.focused())
        });
        let reflow = focused_item.map_or((self.content_height, 0, 0), |item| {
            (self.content_height, item.start, item.height)
        });
        let alignment_changed =
            AlignmentSignature::update(&mut self.alignment, view.focused(), inner, reflow);
        if (self.pending_ensure_focus || alignment_changed)
            && let Some(item) = focused_item
        {
            self.ensure_visible(item.start, item.height);
            self.pending_ensure_focus = false;
        }

        for item in &items {
            let Some(clip) = self.visible_band(item.start, item.height) else {
                continue;
            };
            match &item.item {
                RenderItem::Spacer => {}
                RenderItem::Heading(value) => clip.paint_paragraph(
                    frame.buffer_mut(),
                    Paragraph::new(value.as_str()).style(theme::heading()),
                ),
                RenderItem::Copy(value) => clip.paint_paragraph(
                    frame.buffer_mut(),
                    Paragraph::new(value.as_str())
                        .wrap(Wrap { trim: false })
                        .style(theme::hint()),
                ),
                RenderItem::Control(control) => {
                    self.render_control(frame, clip, control, view, locale);
                }
            }
        }
        self.render_open_dropdowns(frame);
        if let Some(picker) = view.agent_skill_install() {
            self.render_agent_skill_picker(frame, area, picker, locale);
        } else {
            if self.agent_signature.take().is_some() {
                self.agent_click.cancel();
            }
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
        if let Some(picker) = view.agent_skill_install() {
            return match self.handle_agent_skill_overlay_event(event, view, picker) {
                AgentSkillOverlayEventHandling::Action(action) => {
                    PreferencesEventHandling::Action(action)
                }
                AgentSkillOverlayEventHandling::Consumed => PreferencesEventHandling::Consumed,
            };
        }
        self.sync(view, Locale::En);
        if matches!(event, Event::FocusGained | Event::FocusLost) {
            self.cancel_click();
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
            ) {
                self.click.cancel();
                for widget in self.widgets.values_mut() {
                    let PreferencesWidget::Choice {
                        state,
                        presentation: ChoicePresentation::Picker,
                        dropdown_regions,
                        ..
                    } = widget
                    else {
                        continue;
                    };
                    if state.is_open
                        && dropdown_regions
                            .iter()
                            .any(|region| region.contains(mouse.column, mouse.row))
                    {
                        if mouse.kind == MouseEventKind::ScrollUp {
                            state.highlight_prev();
                        } else {
                            state.highlight_next();
                        }
                        state.ensure_visible(dropdown_regions.len().max(1));
                        return PreferencesEventHandling::Consumed;
                    }
                }
            }
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
            let dropdown = self.widgets.iter().find_map(|(id, widget)| {
                let PreferencesWidget::Choice {
                    state,
                    presentation: ChoicePresentation::Picker,
                    dropdown_regions,
                    dropdown_panel,
                    ..
                } = widget
                else {
                    return None;
                };
                state.is_open.then_some(())?;
                dropdown_regions
                    .iter()
                    .rev()
                    .find(|region| region.contains(mouse.column, mouse.row))
                    .and_then(|region| match region.data {
                        SelectAction::Select(option) => {
                            Some(PreferencesHit::Dropdown { id: *id, option })
                        }
                        SelectAction::Focus | SelectAction::Open | SelectAction::Close => None,
                    })
                    .or_else(|| {
                        let in_panel = dropdown_panel
                            .is_some_and(|panel| panel.contains((mouse.column, mouse.row).into()));
                        (in_panel || self.clicks.handle_click(mouse.column, mouse.row).is_none())
                            .then_some(PreferencesHit::Control(*id))
                    })
            });
            let target =
                dropdown.or_else(|| self.clicks.handle_click(mouse.column, mouse.row).cloned());
            if is_primary_down(mouse)
                && let Some(PreferencesHit::Control(id)) = target.as_ref()
                && let Some(editable) = self.editables.get(id).copied()
                && let Some(PreferencesWidget::Input(input)) = self.widgets.get_mut(id)
            {
                let _ = editable.place_cursor(input, mouse.column, mouse.row);
            }
            return match self.click.update(mouse, target.as_ref()) {
                ClickOutcome::Armed => PreferencesEventHandling::Consumed,
                ClickOutcome::Activated(PreferencesHit::Dropdown { id, option }) => self
                    .widgets
                    .get_mut(&id)
                    .and_then(|widget| match widget {
                        PreferencesWidget::Choice { state, values, .. } => {
                            state.close();
                            values.get(option)
                        }
                        PreferencesWidget::Input(_)
                        | PreferencesWidget::Button(_)
                        | PreferencesWidget::RunnerList(_) => None,
                    })
                    .map_or(PreferencesEventHandling::Consumed, |value| {
                        choice_action(id, value)
                    }),
                ClickOutcome::Activated(hit) => self.activate_hit(hit, view),
                ClickOutcome::Ignored => PreferencesEventHandling::Ignored,
            };
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
            Some(PreferencesWidget::RunnerList(_)) => {
                if let Some(action) = runner_list_action(key.code) {
                    return PreferencesEventHandling::Action(action);
                }
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
        if self.agent_signature.as_deref() != Some(picker.targets()) {
            self.agent_click.cancel();
            self.agent_signature = Some(picker.targets().to_vec());
        }
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
            Panel::Picker,
        );
        let inner = block.inner(panel);
        frame.render_widget(block, panel);

        let preview_height = u16::from(!picker.targets().is_empty() && inner.height >= 3);
        let cancel_height = inner.height.min(1);
        let [list_area, preview_area, cancel_area] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(preview_height),
            Constraint::Length(cancel_height),
        ])
        .areas(inner);
        self.agent_list_area = list_area;
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
                .style(theme::hint()),
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
                ListPicker::new(&labels, &self.agent_picker).style(theme::list_picker_style()),
                list_area,
            );
            for visible in 0..self.agent_picker_height.min(picker.targets().len()) {
                let index = usize::from(self.agent_picker.scroll).saturating_add(visible);
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
                    Paragraph::new(target.skills_dir().display().to_string()).style(theme::hint()),
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
                .style(theme::action_button_style())
                .render_stateful(button_area, frame.buffer_mut());
            theme::patch_focus(frame.buffer_mut(), region.area, self.agent_cancel.focused);
            self.agent_cancel_area = Some(region.area);
            self.agent_clicks
                .register(region.area, AgentSkillHit::Cancel);
        }
    }

    pub(crate) fn handle_agent_skill_overlay_event(
        &mut self,
        event: Event,
        view: &PreferencesView,
        picker: &skit_ui::AgentSkillInstallView,
    ) -> AgentSkillOverlayEventHandling {
        self.sync(view, Locale::En);
        if matches!(event, Event::FocusGained | Event::FocusLost) {
            self.cancel_click();
        }
        self.handle_agent_skill_event(event, picker)
    }

    fn handle_agent_skill_event(
        &mut self,
        event: Event,
        picker: &skit_ui::AgentSkillInstallView,
    ) -> AgentSkillOverlayEventHandling {
        let selected = picker.selected().unwrap_or_default();
        let selection = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => {
                    return AgentSkillOverlayEventHandling::Action(
                        PreferencesAction::CloseAgentSkillTargets,
                    );
                }
                KeyCode::Enter if picker.selected().is_some() => {
                    return AgentSkillOverlayEventHandling::Action(
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
                _ => return AgentSkillOverlayEventHandling::Consumed,
            },
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) && self
                    .agent_list_area
                    .contains((mouse.column, mouse.row).into()) =>
            {
                self.agent_click.cancel();
                Some(if mouse.kind == MouseEventKind::ScrollUp {
                    selected.saturating_sub(1)
                } else {
                    selected
                        .saturating_add(1)
                        .min(picker.targets().len().saturating_sub(1))
                })
            }
            Event::Mouse(mouse) => {
                let target = self.agent_clicks.handle_click(mouse.column, mouse.row);
                return match self.agent_click.update(&mouse, target) {
                    ClickOutcome::Activated(AgentSkillHit::Target(index)) => {
                        AgentSkillOverlayEventHandling::Action(
                            PreferencesAction::ActivateAgentSkillTarget(index),
                        )
                    }
                    ClickOutcome::Activated(AgentSkillHit::Cancel) => {
                        AgentSkillOverlayEventHandling::Action(
                            PreferencesAction::CloseAgentSkillTargets,
                        )
                    }
                    ClickOutcome::Armed | ClickOutcome::Ignored => {
                        AgentSkillOverlayEventHandling::Consumed
                    }
                };
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Paste(_)
            | Event::Key(_)
            | Event::Resize(_, _) => return AgentSkillOverlayEventHandling::Consumed,
        };
        selection.filter(|_| !picker.targets().is_empty()).map_or(
            AgentSkillOverlayEventHandling::Consumed,
            |index| {
                AgentSkillOverlayEventHandling::Action(PreferencesAction::SelectAgentSkillTarget(
                    index,
                ))
            },
        )
    }

    pub(crate) fn sync(&mut self, view: &PreferencesView, locale: Locale) {
        let controls = view.controls();
        let signature = PreferencesSignature(
            controls
                .iter()
                .map(|control| (control.id, control_shape(control)))
                .collect(),
        );
        if self.signature.as_ref() != Some(&signature) {
            self.click.cancel();
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
        clip: RowClip,
        control: &PreferencesControl,
        view: &PreferencesView,
        locale: Locale,
    ) {
        let area = clip.area();
        self.control_areas.push((control.id, area));
        let focused = view.focused() == control.id;
        let label = text(locale, &control.label);
        let Some(widget) = self.widgets.get_mut(&control.id) else {
            return;
        };
        match widget {
            PreferencesWidget::Input(state) => {
                if let Some(editable) =
                    render_line_input_band(frame, clip, state, false, focused, &label, None)
                {
                    self.editables.insert(control.id, editable);
                }
                if state.value().is_empty()
                    && let PreferencesControlKind::Text(model) = &control.kind
                    && let Some(content) = clip.row(1)
                {
                    let inner = Rect::new(
                        content.x.saturating_add(1),
                        content.y,
                        content.width.saturating_sub(2),
                        1,
                    );
                    frame.render_widget(
                        Paragraph::new(text(locale, &model.placeholder)).style(theme::hint()),
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
                let placeholder = text(locale, "Select");
                if clip.is_full() {
                    let region = Select::new(labels, state)
                        .label(&label)
                        .placeholder(&placeholder)
                        .style(theme::select_style())
                        .render_stateful(frame, area);
                    theme::patch_idle_border(frame.buffer_mut(), region.area, state.focused);
                    *select_area = Some(region.area);
                } else {
                    let style = theme::select_style();
                    let display = state
                        .selected_index
                        .and_then(|index| labels.get(index))
                        .map_or(placeholder.as_ref(), String::as_str);
                    let display_style = if state.selected_index.is_some() {
                        Style::default().fg(style.text_fg)
                    } else {
                        Style::default().fg(style.placeholder_fg)
                    };
                    let border = if state.focused {
                        style.focused_border
                    } else {
                        style.unfocused_border
                    };
                    clip.paint_bordered_paragraph(
                        frame.buffer_mut(),
                        Paragraph::new(Line::from(vec![
                            Span::styled(display, display_style),
                            Span::styled(
                                format!(" {}", style.dropdown_indicator),
                                Style::default().fg(border),
                            ),
                        ])),
                        Line::from(format!(" {label} ")),
                        Style::default().fg(border),
                        0,
                    );
                    *select_area = Some(area);
                }
                if let Some(select_area) = *select_area {
                    self.clicks
                        .register(select_area, PreferencesHit::Control(control.id));
                }
            }
            PreferencesWidget::Choice {
                state,
                labels,
                presentation: ChoicePresentation::Radio,
                buttons,
                ..
            } => {
                render_radio_band(
                    frame,
                    clip,
                    control,
                    &label,
                    RadioBand {
                        labels,
                        buttons,
                        focused,
                        selected: state.selected_index,
                        active: state.selected_index.unwrap_or(state.highlighted_index),
                    },
                    &mut self.clicks,
                );
                state.ensure_visible(1);
            }
            PreferencesWidget::RunnerList(list) => {
                render_runner_list(
                    frame,
                    clip,
                    list,
                    focused,
                    locale,
                    &mut RunnerListTargets {
                        clicks: &mut self.clicks,
                        rows: &mut self.runner_row_areas,
                        chips: &mut self.runner_chip_areas,
                    },
                );
            }
            PreferencesWidget::Button(state) => {
                if focused {
                    paint_focus_marker(frame, area);
                }
                let door = options_band(area);
                // The button measures its label in characters. The paint uses display cells, so
                // the click rect must use the painted width or a wide label reaches past it.
                let width = u16::try_from(label.as_ref().width().saturating_add(2))
                    .unwrap_or(u16::MAX)
                    .min(door.width);
                let painted = Rect::new(door.x, door.y, width, 1);
                Button::new(&label, state)
                    .variant(ButtonVariant::SingleLine)
                    .alignment(Alignment::Left)
                    .style(theme::action_button_style())
                    .render_stateful(painted, frame.buffer_mut());
                theme::patch_focus(frame.buffer_mut(), painted, state.focused);
                self.clicks
                    .register(painted, PreferencesHit::Control(control.id));
                self.control_areas
                    .last_mut()
                    .expect("control area was inserted")
                    .1 = painted;
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
                dropdown_panel,
                dropdown_regions,
                ..
            } = widget
            else {
                continue;
            };
            if state.is_open {
                let Some(anchor) = *select_area else {
                    *dropdown_panel = None;
                    dropdown_regions.clear();
                    continue;
                };
                let style = theme::select_style();
                *dropdown_panel =
                    select_dropdown_panel(anchor, screen, labels.len(), style.max_visible_options);
                *dropdown_regions = Select::new(labels, state)
                    .style(style)
                    .render_dropdown(frame, anchor, screen);
            } else {
                *dropdown_panel = None;
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
            is_open_select_key(key.code)
        } else {
            is_closed_select_key(key.code)
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
                    state.toggle();
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
                // The agent list registers one target per row, so no click names the list itself.
                Some(PreferencesWidget::RunnerList(_)) | None => PreferencesEventHandling::Ignored,
            },
            PreferencesHit::Radio { id, option } => self
                .widgets
                .get(&id)
                .and_then(|widget| match widget {
                    PreferencesWidget::Choice { values, .. } => values.get(option),
                    PreferencesWidget::Input(_)
                    | PreferencesWidget::Button(_)
                    | PreferencesWidget::RunnerList(_) => None,
                })
                .map_or(PreferencesEventHandling::Ignored, |value| {
                    choice_action(id, value)
                }),
            PreferencesHit::Dropdown { .. } => PreferencesEventHandling::Consumed,
            PreferencesHit::RunnerRow(index) => {
                PreferencesEventHandling::Action(PreferencesAction::RunnerCursor(index))
            }
            PreferencesHit::RunnerChip { chip, .. } => {
                PreferencesEventHandling::Action(match chip {
                    RunnerChip::Edit => PreferencesAction::EditRunner,
                    RunnerChip::Remove => PreferencesAction::ToggleRunnerRemoval,
                })
            }
        }
    }

    fn move_focus(&mut self, forward: bool) -> PreferencesEventHandling {
        PreferencesEventHandling::Action(self.move_focus_action(forward))
    }

    pub(crate) fn move_focus_action(&mut self, forward: bool) -> PreferencesAction {
        if forward {
            self.focus.next();
        } else {
            self.focus.prev();
        }
        // The session moves its own cursor here, so the next sync sees no change and would never
        // scroll. Without this the keyboard can focus a control that is never drawn at all.
        self.pending_ensure_focus = true;
        PreferencesAction::Focus(
            self.focus
                .current()
                .copied()
                .expect("the Preferences focus ring is empty"),
        )
    }

    fn ensure_visible(&mut self, start: usize, height: usize) {
        let offset = self.scroll.scroll_offset();
        let end = start.saturating_add(height);
        let next = match start.cmp(&offset) {
            Ordering::Less => start,
            Ordering::Equal | Ordering::Greater => {
                offset.max(end.saturating_sub(self.visible_height))
            }
        };
        self.scroll.set_scroll_offset(next);
    }

    fn visible_band(&self, start: usize, height: usize) -> Option<RowClip> {
        let offset = self.scroll.scroll_offset();
        let viewport_end = offset.saturating_add(self.visible_height);
        let end = start.saturating_add(height);
        if end <= offset || start >= viewport_end {
            return None;
        }
        let clipped_start = start.max(offset);
        let clipped_end = end.min(viewport_end);
        Some(RowClip::new(
            height,
            clipped_start.saturating_sub(start),
            Rect::new(
                self.viewport.x,
                self.viewport.y.saturating_add(
                    u16::try_from(clipped_start.saturating_sub(offset))
                        .expect("the Preferences band starts inside its viewport"),
                ),
                self.viewport.width,
                u16::try_from(clipped_end.saturating_sub(clipped_start))
                    .expect("the Preferences band height fits its viewport"),
            ),
        ))
    }
}

fn editable_geometry_snapshot(geometry: EditableGeometry) -> serde_json::Value {
    serde_json::json!({
        "content": snapshot_rect(geometry.content()),
        "visual_scroll": geometry.visual_scroll(),
        "secret": geometry.secret(),
    })
}

fn agent_target_snapshot(target: &AgentTarget) -> serde_json::Value {
    let AgentTarget { name, scope, base } = target;
    serde_json::json!({
        "name": name,
        "scope": scope,
        "base": snapshot_path(base),
    })
}

fn preferences_signature_snapshot(
    signature: &PreferencesSignature,
) -> Result<serde_json::Value, AgentReviewSnapshotError> {
    let PreferencesSignature(controls) = signature;
    controls
        .iter()
        .map(|(id, shape)| {
            let shape = match shape {
                PreferencesControlShape::Text => serde_json::json!("text"),
                PreferencesControlShape::Choice {
                    options,
                    presentation,
                } => {
                    let presentation =
                        snapshot_value("preferences.signature.presentation", presentation)?;
                    serde_json::json!({
                        "choice": {
                            "options": options,
                            "presentation": presentation,
                        }
                    })
                }
                PreferencesControlShape::Button => serde_json::json!("button"),
                PreferencesControlShape::RunnerList { rows } => {
                    serde_json::json!({ "runner_list": { "rows": rows } })
                }
            };
            Ok(serde_json::json!({
                "id": snapshot_value("preferences.signature.id", id)?,
                "shape": shape,
            }))
        })
        .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()
        .map(serde_json::Value::Array)
}

fn preferences_hit_snapshot(
    hit: &PreferencesHit,
) -> Result<serde_json::Value, AgentReviewSnapshotError> {
    match hit {
        PreferencesHit::Control(id) => Ok(serde_json::json!({
            "control": snapshot_value("preferences.hit.control", id)?,
        })),
        PreferencesHit::Radio { id, option } => Ok(serde_json::json!({
            "radio": {
                "id": snapshot_value("preferences.hit.radio.id", id)?,
                "option": option,
            }
        })),
        PreferencesHit::Dropdown { id, option } => Ok(serde_json::json!({
            "dropdown": {
                "id": snapshot_value("preferences.hit.dropdown.id", id)?,
                "option": option,
            }
        })),
        PreferencesHit::RunnerRow(index) => Ok(serde_json::json!({ "runner_row": index })),
        PreferencesHit::RunnerChip { index, chip } => Ok(serde_json::json!({
            "runner_chip": {
                "index": index,
                "chip": snapshot_value("preferences.hit.runner_chip.chip", chip)?,
            }
        })),
    }
}

fn runner_chip_areas_snapshot(
    areas: &[(usize, RunnerChip, Rect)],
) -> Result<serde_json::Value, AgentReviewSnapshotError> {
    areas
        .iter()
        .map(|(index, chip, area)| {
            Ok(serde_json::json!({
                "index": index,
                "chip": snapshot_value("preferences.runner_chip_area.chip", chip)?,
                "area": snapshot_rect(*area),
            }))
        })
        .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()
        .map(serde_json::Value::Array)
}

fn preferences_clicks_snapshot(
    clicks: &ClickRegionRegistry<PreferencesHit>,
) -> Result<serde_json::Value, AgentReviewSnapshotError> {
    clicks
        .regions()
        .iter()
        .map(|region| {
            let target = preferences_hit_snapshot(&region.data)?;
            Ok(serde_json::json!({
                "area": snapshot_rect(region.area),
                "target": target,
            }))
        })
        .collect::<Result<Vec<_>, AgentReviewSnapshotError>>()
        .map(serde_json::Value::Array)
}

fn agent_hit_snapshot(hit: &AgentSkillHit) -> serde_json::Value {
    match hit {
        AgentSkillHit::Target(index) => serde_json::json!({"target": index}),
        AgentSkillHit::Cancel => serde_json::json!("cancel"),
    }
}

fn agent_clicks_snapshot(clicks: &ClickRegionRegistry<AgentSkillHit>) -> serde_json::Value {
    serde_json::Value::Array(
        clicks
            .regions()
            .iter()
            .map(|region| {
                let target = agent_hit_snapshot(&region.data);
                serde_json::json!({
                    "area": snapshot_rect(region.area),
                    "target": target,
                })
            })
            .collect(),
    )
}

fn render_radio_band(
    frame: &mut Frame,
    clip: RowClip,
    control: &PreferencesControl,
    label: &str,
    band: RadioBand<'_>,
    clicks: &mut ClickRegionRegistry<PreferencesHit>,
) {
    let label_rows = usize::from(!label.is_empty());
    let placements = radio_placements(
        band.labels,
        options_band(clip.area()).width,
        radio_options_stack(control.id, clip.area().width),
    );
    if band.focused
        && let Some(row) = focus_marker_row(clip, &placements, band.active, label_rows)
    {
        paint_focus_marker(frame, row);
    }
    for (source, row) in clip.rows() {
        let body = options_band(row);
        if source < label_rows {
            frame.render_widget(Paragraph::new(label).style(theme::text()), body);
            continue;
        }
        let target_row = source.saturating_sub(label_rows);
        for (index, ((option_label, button), placement)) in band
            .labels
            .iter()
            .zip(band.buttons.iter())
            .zip(placements.iter())
            .enumerate()
        {
            if placement.row != target_row {
                continue;
            }
            let rect = render_radio_option(
                frame.buffer_mut(),
                Rect::new(
                    body.x.saturating_add(placement.offset),
                    row.y,
                    placement.width,
                    1,
                ),
                option_label,
                button,
                band.focused,
                band.selected == Some(index),
            );
            clicks.register(
                rect,
                PreferencesHit::Radio {
                    id: control.id,
                    option: index,
                },
            );
        }
    }
}

/// The options of one radio group and the focus that paints them.
struct RadioBand<'a> {
    labels: &'a [String],
    buttons: &'a [ButtonState],
    focused: bool,
    selected: Option<usize>,
    active: usize,
}

/// The place of one painted radio option inside its band.
struct RadioPlacement {
    row: usize,
    offset: u16,
    width: u16,
}

/// Place every option of one radio band in the given width.
///
/// The layout and the paint both use this function, so a row count and a painted row always agree.
fn radio_placements<S: AsRef<str>>(labels: &[S], width: u16, stacked: bool) -> Vec<RadioPlacement> {
    let mut placements = Vec::with_capacity(labels.len());
    let mut row = 0_usize;
    let mut offset = 0_u16;
    for label in labels {
        let cells = radio_option_width(label.as_ref(), width);
        if offset > 0 && (stacked || offset.saturating_add(cells) > width) {
            offset = 0;
            row = row.saturating_add(1);
        }
        placements.push(RadioPlacement {
            row,
            offset,
            width: cells,
        });
        offset = offset.saturating_add(cells).saturating_add(1);
    }
    placements
}

/// Return the row that gets the focus marker of one radio band.
///
/// The marker follows the selected option. A wheel scroll can move that option off screen while
/// the other options stay, so the first visible row of the control then keeps the cue.
fn focus_marker_row(
    clip: RowClip,
    placements: &[RadioPlacement],
    active: usize,
    label_rows: usize,
) -> Option<Rect> {
    placements
        .get(active)
        .map(|placement| placement.row.saturating_add(label_rows))
        .and_then(|source| clip.row(source))
        .or_else(|| clip.rows().next().map(|(_, row)| row))
}

/// Return the walker target of one agent row.
fn runner_row_target(
    rows: &[RunnerDraftRow],
    index: usize,
) -> Result<ScreenTarget, ScreenTargetError> {
    let row = rows.get(index).ok_or(ScreenTargetError::StaleSession)?;
    Ok(ScreenTarget::Runner {
        row: index,
        name: row.name().map(str::to_owned),
    })
}

/// Cells between two neighbouring command chips of one agent row.
const RUNNER_CHIP_GAP: u16 = 1;

/// The cells one agent row keeps for its own text: a name column, a command column, and one
/// separator each. A row with fewer cells beside its chips is too short to hold them.
const RUNNER_ROW_MINIMUM_CELLS: u16 = 18;

/// The cells one agent row keeps for its own name beside a staging note.
const RUNNER_ROW_NAME_CELLS: usize = 6;

/// The cells between the text of one agent row and its staging note.
const RUNNER_NOTE_GAP: usize = 2;

/// One painted source row of the agent list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunnerListRow {
    /// One staged draft row.
    Row(usize),
    /// One line of the command chips of one draft row.
    Chips(usize, usize),
}

/// One command the agent-list cursor row offers.
#[derive(Clone, Copy, Debug)]
struct RunnerRowChip {
    chip: RunnerChip,
    key: &'static str,
    label: &'static str,
}

/// The click and walker targets one painted agent list publishes.
struct RunnerListTargets<'a> {
    clicks: &'a mut ClickRegionRegistry<PreferencesHit>,
    rows: &'a mut Vec<(usize, Rect)>,
    chips: &'a mut Vec<(usize, RunnerChip, Rect)>,
}

/// Return the commands of one agent row in paint order.
///
/// Every key the row accepts has a chip, so a pointer user needs no key and a keyboard user needs
/// no memory. A row that no editor can open offers the removal alone.
fn runner_row_chips(row: &RunnerDraftRow, taken: bool) -> Vec<RunnerRowChip> {
    // The change on the key already takes this row, so the row commands nothing of its own.
    if taken {
        return Vec::new();
    }
    let mut chips = Vec::with_capacity(2);
    if row.is_editable() {
        chips.push(RunnerRowChip {
            chip: RunnerChip::Edit,
            key: "Enter",
            label: "Edit",
        });
    }
    chips.push(RunnerRowChip {
        chip: RunnerChip::Remove,
        key: "Del",
        label: if row.is_removed() {
            "Restore"
        } else {
            "Remove"
        },
    });
    chips
}

/// Return the painted width of one command chip.
fn runner_chip_width(chip: &RunnerRowChip, locale: Locale) -> u16 {
    u16::try_from(
        chip.key
            .width()
            .saturating_add(text(locale, chip.label).as_ref().width())
            .saturating_add(3),
    )
    .unwrap_or(u16::MAX)
}

/// Return the cells every command chip of one row takes together.
fn runner_chips_width(chips: &[RunnerRowChip], locale: Locale) -> u16 {
    chips
        .iter()
        .map(|chip| runner_chip_width(chip, locale))
        .reduce(|total, width| total.saturating_add(RUNNER_CHIP_GAP).saturating_add(width))
        .unwrap_or_default()
}

/// Place the command chips of one row: beside it while they fit, and on lines below otherwise.
///
/// The chips keep the row when the row can still hold a name column, a command column, one
/// separator each, and the staging marker the row carries. A row too short for all of them moves
/// its chips below, where they take as many lines as they need: a chip names the key it answers,
/// so a dropped chip would take the mouse path to that key away. The test is the fit, not the
/// terminal tier, so every row wide enough keeps its commands in place.
fn runner_chip_placement(
    width: u16,
    row: &RunnerDraftRow,
    taken: bool,
    locale: Locale,
) -> RunnerChipPlacement {
    let chips = runner_row_chips(row, taken);
    let band = width.saturating_sub(FOCUS_GUTTER_CELLS);
    if band
        >= RUNNER_ROW_MINIMUM_CELLS
            .saturating_add(runner_chips_width(&chips, locale))
            .saturating_add(runner_marker_cells(row, taken, locale))
    {
        return RunnerChipPlacement::Inline(chips);
    }
    RunnerChipPlacement::Below(runner_chip_lines(&chips, band, locale))
}

/// Where the command chips of one agent row go.
#[derive(Debug)]
enum RunnerChipPlacement {
    /// Beside the text of the row, in paint order.
    Inline(Vec<RunnerRowChip>),
    /// On lines of their own. No line means the band holds no complete chip.
    Below(Vec<Vec<RunnerRowChip>>),
}

impl RunnerChipPlacement {
    /// Return the lines the chips take below their row, and zero while they sit beside it.
    fn below_lines(&self) -> usize {
        match self {
            Self::Inline(_) => 0,
            Self::Below(lines) => lines.len(),
        }
    }
}

/// Place the command chips of the cursor row of one focused agent list.
///
/// Only the cursor row of the focused list carries chips, so the layout and the paint each place
/// them once and share the result.
fn runner_cursor_placement(
    list: &PreferencesRunnerListControl,
    focused: bool,
    width: u16,
    locale: Locale,
) -> Option<RunnerChipPlacement> {
    let row = focused.then(|| list.rows.get(list.cursor)).flatten()?;
    Some(runner_chip_placement(
        width,
        row,
        runner_row_taken_by_its_key(&list.rows, list.cursor),
        locale,
    ))
}

/// Pack the command chips of one row into the lines one band can hold.
///
/// Only a chip wider than a line of its own has nowhere to go.
fn runner_chip_lines(
    chips: &[RunnerRowChip],
    width: u16,
    locale: Locale,
) -> Vec<Vec<RunnerRowChip>> {
    let mut lines: Vec<Vec<RunnerRowChip>> = Vec::new();
    let mut used = 0_u16;
    for chip in chips {
        let cells = runner_chip_width(chip, locale);
        if cells > width {
            continue;
        }
        let next = used.saturating_add(RUNNER_CHIP_GAP).saturating_add(cells);
        match lines.last_mut() {
            Some(line) if next <= width => {
                line.push(*chip);
                used = next;
            }
            _ => {
                lines.push(vec![*chip]);
                used = cells;
            }
        }
    }
    lines
}

/// Return the cells the staging marker of one row keeps at the right end, with its separator.
fn runner_marker_cells(row: &RunnerDraftRow, taken: bool, locale: Locale) -> u16 {
    runner_row_notes(row, taken, locale)
        .last()
        .map_or(0, |marker| {
            u16::try_from(marker.width().saturating_add(RUNNER_NOTE_GAP)).unwrap_or(u16::MAX)
        })
}

/// Place every source row of one agent list, with `below` chip lines under the cursor row.
///
/// The layout and the paint both use this function, so a row count and a painted row always agree.
fn runner_list_rows(list: &PreferencesRunnerListControl, below: usize) -> Vec<RunnerListRow> {
    let mut rows = Vec::with_capacity(list.rows.len().saturating_add(below));
    for index in 0..list.rows.len() {
        rows.push(RunnerListRow::Row(index));
        if index == list.cursor {
            rows.extend((0..below).map(|line| RunnerListRow::Chips(index, line)));
        }
    }
    rows
}

/// Paint one agent row for every staged draft row, with the commands of the cursor row.
///
/// Every key the list accepts is one typed reducer action, so the row itself carries no state.
/// The focus marker follows the cursor row, and keeps the first visible row when a scroll moved
/// that row off screen. This is the rule every radio band follows.
fn render_runner_list(
    frame: &mut Frame,
    clip: RowClip,
    list: &PreferencesRunnerListControl,
    focused: bool,
    locale: Locale,
    targets: &mut RunnerListTargets<'_>,
) {
    let placement = runner_cursor_placement(list, focused, clip.area().width, locale);
    let sources = runner_list_rows(
        list,
        placement
            .as_ref()
            .map_or(0, RunnerChipPlacement::below_lines),
    );
    if focused
        && let Some(marker) = sources
            .iter()
            .position(|source| *source == RunnerListRow::Row(list.cursor))
            .and_then(|source| clip.row(source))
            .or_else(|| clip.rows().next().map(|(_, row)| row))
    {
        paint_focus_marker(frame, marker);
    }
    for (source, painted) in sources.iter().enumerate() {
        let Some(row_area) = clip.row(source) else {
            continue;
        };
        let (RunnerListRow::Row(index) | RunnerListRow::Chips(index, _)) = *painted;
        let row = &list.rows[index];
        let taken = runner_row_taken_by_its_key(&list.rows, index);
        let band = options_band(row_area);
        let mut content = band;
        if let Some(placement) = placement.as_ref().filter(|_| index == list.cursor) {
            let painted_chips: &[RunnerRowChip] = match (painted, placement) {
                (RunnerListRow::Chips(_, line), RunnerChipPlacement::Below(lines)) => {
                    lines.get(*line).map_or(&[], Vec::as_slice)
                }
                (RunnerListRow::Row(_), RunnerChipPlacement::Inline(chips)) => {
                    content.width = content
                        .width
                        .saturating_sub(runner_chips_width(chips, locale));
                    chips
                }
                // The layout plans a chip line only below a row whose chips moved there.
                _ => &[],
            };
            if !painted_chips.is_empty() {
                paint_runner_chips(frame, band, index, painted_chips, locale, targets);
            }
        }
        // A click anywhere on the row, its gutter, or its chip line moves the cursor to it. The
        // walker addresses one rectangle per draft row, so only the row itself is a target.
        targets
            .clicks
            .register(row_area, PreferencesHit::RunnerRow(index));
        if matches!(painted, RunnerListRow::Row(_)) {
            frame.render_widget(
                Paragraph::new(Line::from(runner_row_spans(
                    row,
                    taken,
                    locale,
                    content.width,
                ))),
                content,
            );
            targets.rows.push((index, row_area));
        }
    }
}

/// Paint the command chips of one agent row at the right end of `band`.
fn paint_runner_chips(
    frame: &mut Frame,
    band: Rect,
    index: usize,
    chips: &[RunnerRowChip],
    locale: Locale,
    targets: &mut RunnerListTargets<'_>,
) {
    let total = runner_chips_width(chips, locale);
    let mut x = band.x.saturating_add(band.width.saturating_sub(total));
    let style = ActionFooterStyle::default().button();
    for chip in chips {
        let width = runner_chip_width(chip, locale);
        let label = text(locale, chip.label);
        let region = Button::new(label.as_ref(), &ButtonState::enabled())
            .icon(chip.key)
            .variant(ButtonVariant::SingleLine)
            .style(style.clone())
            .render_stateful(Rect::new(x, band.y, width, 1), frame.buffer_mut());
        targets.clicks.register(
            region.area,
            PreferencesHit::RunnerChip {
                index,
                chip: chip.chip,
            },
        );
        targets.chips.push((index, chip.chip, region.area));
        x = x.saturating_add(width).saturating_add(RUNNER_CHIP_GAP);
    }
}

/// Build the complete text of one agent row inside `width` cells.
///
/// The name keeps its cells first, the staging note keeps the right end, and the command column
/// takes what is left with an ellipsis. A note that leaves no room for a name is dropped.
fn runner_row_spans(
    row: &RunnerDraftRow,
    taken: bool,
    locale: Locale,
    width: u16,
) -> Vec<Span<'static>> {
    let cells = usize::from(width);
    // The notes come widest first, so a row too short for the prompt count still keeps the marker
    // that says what the save does to the row.
    let (note, reserved) = runner_row_notes(row, taken, locale)
        .into_iter()
        .find(|note| note.width().saturating_add(RUNNER_ROW_NAME_CELLS) <= cells)
        .map_or_else(
            || (String::new(), 0),
            |note| {
                let reserved = note.width().saturating_add(RUNNER_NOTE_GAP);
                (note, reserved)
            },
        );
    let body = cells.saturating_sub(reserved);
    let label = clip_cells(&runner_row_label(row), body);
    let mut used = label.width();
    let mut spans = vec![Span::styled(label, theme::text())];
    for (value, style) in [
        (
            row.argv().map_or_else(String::new, |argv| {
                join_editable_argv(argv, EditableArgvDialect::host())
            }),
            theme::hint(),
        ),
        (
            runner_row_reason(row, locale),
            theme::status(Status::Danger),
        ),
    ] {
        let room = body.saturating_sub(used).saturating_sub(2);
        if value.is_empty() || room == 0 {
            continue;
        }
        let shown = clip_cells(&value, room);
        used = used.saturating_add(shown.width()).saturating_add(2);
        spans.push(Span::styled(format!("  {shown}"), style));
    }
    if reserved > 0 {
        let pad = cells.saturating_sub(used).saturating_sub(note.width());
        spans.push(Span::styled(
            format!("{}{note}", " ".repeat(pad)),
            theme::hint(),
        ));
    }
    spans
}

/// Return the staging notes of one agent row, widest first.
///
/// The prompts a removal strands are an extra on the staging marker. A row with room for one note
/// only keeps the marker, because the marker says what the save does to the row.
fn runner_row_notes(row: &RunnerDraftRow, taken: bool, locale: Locale) -> Vec<String> {
    // The save takes this row with the key it repeats, whatever the row itself carries.
    let marker = if taken {
        Some(RunnerDraftMarker::Removed)
    } else {
        row.marker()
    };
    let Some(marker) = marker else {
        return Vec::new();
    };
    let marker = text(locale, runner_marker_key(marker)).into_owned();
    let pinned = match row {
        RunnerDraftRow::Existing { row: stored, .. } if row.is_removed() => stored.pinned_count,
        RunnerDraftRow::Existing { .. } | RunnerDraftRow::Added { .. } => 0,
    };

    if pinned == 0 {
        return vec![marker];
    }
    vec![
        format!("{marker} · {}", runner_pin_note(pinned, locale)),
        marker,
    ]
}

/// Return the note that counts the prompts one staged removal leaves without an agent.
fn runner_pin_note(pinned: usize, locale: Locale) -> String {
    let template = if pinned == 1 {
        "{} prompt pins this runner and will need another runner before it can run again."
    } else {
        "{} prompts pin this runner and will need another runner before they can run again."
    };
    format_text(locale, template, &[&pinned])
}

/// Return the localized reason of one malformed row that keeps its stored shape.
fn runner_row_reason(row: &RunnerDraftRow, locale: Locale) -> String {
    match row {
        RunnerDraftRow::Existing { row: stored, state } => stored
            .reason
            .as_deref()
            .filter(|_| !matches!(state, RunnerDraftState::Edited(_)))
            .map_or_else(String::new, |code| runner_reason(code, locale)),
        RunnerDraftRow::Added { .. } => String::new(),
    }
}

/// Return `value` inside `cells` display columns, with `…` when it loses content.
fn clip_cells(value: &str, cells: usize) -> String {
    if value.width() <= cells {
        return value.to_owned();
    }
    let mut shown = String::new();
    let mut used = 0_usize;
    for grapheme in value.graphemes(true) {
        let next = used.saturating_add(grapheme.width());
        if next > cells.saturating_sub(1) {
            break;
        }
        shown.push_str(grapheme);
        used = next;
    }
    if cells > 0 {
        shown.push('…');
    }
    shown
}

/// Return the localized text of one malformed-row reason code.
pub(crate) fn runner_reason(code: &str, locale: Locale) -> String {
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

/// Return the stable row label: the agent name, or the raw shape of a malformed row.
fn runner_row_label(row: &RunnerDraftRow) -> String {
    match row {
        RunnerDraftRow::Existing { row: stored, .. } => row
            .name()
            .map_or_else(|| format!("⚠ {}", stored.descriptor), str::to_owned),
        RunnerDraftRow::Added { name, .. } => name.clone(),
    }
}

const fn runner_marker_key(marker: RunnerDraftMarker) -> &'static str {
    match marker {
        RunnerDraftMarker::Added => "Added",
        RunnerDraftMarker::Edited => "Edited",
        RunnerDraftMarker::Removed => "Will be removed",
    }
}

/// Map one key on the focused agent list to its typed reducer action.
fn runner_list_action(code: KeyCode) -> Option<PreferencesAction> {
    match code {
        KeyCode::Up => Some(PreferencesAction::RunnerCursorPrevious),
        KeyCode::Down => Some(PreferencesAction::RunnerCursorNext),
        KeyCode::Enter => Some(PreferencesAction::EditRunner),
        KeyCode::Delete | KeyCode::Backspace => Some(PreferencesAction::ToggleRunnerRemoval),
        _ => None,
    }
}

/// Return the cells of one control row that hold content instead of the focus marker.
fn options_band(row: Rect) -> Rect {
    Rect::new(
        row.x.saturating_add(FOCUS_GUTTER_CELLS),
        row.y,
        row.width.saturating_sub(FOCUS_GUTTER_CELLS),
        1,
    )
}

/// Paint the focus marker in the gutter of one control row.
fn paint_focus_marker(frame: &mut Frame, row: Rect) {
    frame.buffer_mut().set_stringn(
        row.x,
        row.y,
        "▶ ",
        usize::from(FOCUS_GUTTER_CELLS.min(row.width)),
        theme::marker(),
    );
}

fn is_open_select_key(code: KeyCode) -> bool {
    [
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Char(' '),
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
    ]
    .contains(&code)
}

fn is_closed_select_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Down)
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
            let height = control_height(&control, locale, width, view.focused() == control.id);
            push_item(
                &mut positioned,
                &mut start,
                RenderItem::Control(control),
                height,
            );
            if view.error().is_some()
                && positioned.last().is_some_and(|item| {
                    matches!(&item.item, RenderItem::Control(control)
                        if Some(control.id) == view.error_control())
                })
            {
                let error = view
                    .error()
                    .expect("validation error was checked")
                    .message()
                    .localize(locale);
                push_item(&mut positioned, &mut start, RenderItem::Copy(error), 1);
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

fn control_height(
    control: &PreferencesControl,
    locale: Locale,
    width: u16,
    focused: bool,
) -> usize {
    match &control.kind {
        PreferencesControlKind::Text(_) => 3,
        PreferencesControlKind::Choice(choice)
            if choice.presentation == ChoicePresentation::Picker =>
        {
            3
        }
        PreferencesControlKind::Choice(choice) => {
            let labels = choice
                .options
                .iter()
                .map(|option| text(locale, &option.label))
                .collect::<Vec<_>>();
            let rows = radio_placements(
                &labels,
                width.saturating_sub(FOCUS_GUTTER_CELLS),
                radio_options_stack(control.id, width),
            )
            .last()
            .map_or(1, |placement| placement.row.saturating_add(1));
            usize::from(!control.label.is_empty()).saturating_add(rows)
        }
        PreferencesControlKind::Button => 1,
        // The reducer omits the list when the draft has no rows, so the list is never empty.
        PreferencesControlKind::RunnerList(list) => list.rows.len().saturating_add(
            runner_cursor_placement(list, focused, width, locale)
                .as_ref()
                .map_or(0, RunnerChipPlacement::below_lines),
        ),
    }
}

fn radio_options_stack(id: PreferencesControlId, width: u16) -> bool {
    !matches!(
        id,
        PreferencesControlId::MirrorMaster
            | PreferencesControlId::PypiChoice
            | PreferencesControlId::GithubChoice
            | PreferencesControlId::NpmChoice
    ) || crate::layout::is_narrow(width)
}

fn select_dropdown_panel(
    anchor: Rect,
    screen: Rect,
    option_count: usize,
    maximum_visible: u16,
) -> Option<Rect> {
    if option_count == 0 {
        return None;
    }
    let visible_count = (option_count as u16).min(maximum_visible);
    let desired_height = visible_count.saturating_add(2);
    let space_below = screen.height.saturating_sub(anchor.y + anchor.height);
    let space_above = anchor.y.saturating_sub(screen.y);
    let (dropdown_y, available) = if space_below >= desired_height {
        (anchor.y + anchor.height, space_below)
    } else if space_above >= desired_height {
        (anchor.y.saturating_sub(desired_height), space_above)
    } else {
        (anchor.y + anchor.height, space_below)
    };
    let panel = Rect::new(
        anchor.x,
        dropdown_y,
        anchor.width,
        desired_height.min(available),
    );
    (!panel.is_empty()).then_some(panel)
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
        PreferencesControlKind::RunnerList(list) => PreferencesControlShape::RunnerList {
            rows: list.rows.len(),
        },
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
                select_area: None,
                dropdown_panel: None,
                dropdown_regions: Vec::new(),
            }
        }
        PreferencesControlKind::Button => PreferencesWidget::Button(ButtonState::enabled()),
        PreferencesControlKind::RunnerList(list) => PreferencesWidget::RunnerList(list.clone()),
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
            if state.selected_index != selected {
                state.selected_index = selected;
                state.highlighted_index = selected.unwrap_or_default();
            }
            for (index, button) in buttons.iter_mut().enumerate() {
                button.toggled = selected == Some(index);
            }
            *labels = choice
                .options
                .iter()
                .map(|option| text(locale, &option.label).into_owned())
                .collect();
        }
        (PreferencesWidget::RunnerList(state), PreferencesControlKind::RunnerList(list)) => {
            *state = list.clone();
        }
        (PreferencesWidget::Input(_), _)
        | (PreferencesWidget::Choice { .. }, _)
        | (PreferencesWidget::Button(_), _)
        | (PreferencesWidget::RunnerList(_), _) => {}
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
        PreferencesWidget::RunnerList(_) => {}
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
        | PreferencesControlId::Theme
        | PreferencesControlId::InteractiveForm
        | PreferencesControlId::AfterRun
        | PreferencesControlId::Javascript
        | PreferencesControlId::Runners
        | PreferencesControlId::NewRunner
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
        PreferencesControlId::Theme => {
            PreferencesAction::SetTheme(ThemeChoice::from_config(value).unwrap_or_default())
        }
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
        | PreferencesControlId::Runners
        | PreferencesControlId::NewRunner
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
        PreferencesControlId::NewRunner => {
            PreferencesEventHandling::Action(PreferencesAction::NewRunner)
        }
        PreferencesControlId::InstallAgentSkill => {
            PreferencesEventHandling::Action(PreferencesAction::InstallAgentSkill)
        }
        PreferencesControlId::Language
        | PreferencesControlId::Theme
        | PreferencesControlId::Editor
        | PreferencesControlId::InteractiveForm
        | PreferencesControlId::AfterRun
        | PreferencesControlId::Javascript
        | PreferencesControlId::BashPath
        | PreferencesControlId::Runners
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
    use ratatui_core::style::Color;
    use std::path::PathBuf;

    use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use skit_application::preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesField, PreferencesSnapshot, ThemeChoice,
    };
    use skit_application::{AgentScope, AgentTarget};
    use skit_i18n::Locale;
    use skit_ui::{
        FormInputKind, PreferencesAction, PreferencesChoiceControl, PreferencesControlId,
        PreferencesOption, PreferencesTextControl, PreferencesView,
    };

    use super::*;

    #[test]
    fn dropdown_panel_prefers_available_space_and_has_no_empty_panel() {
        let screen = Rect::new(0, 0, 40, 12);
        let anchor = Rect::new(3, 8, 12, 1);
        assert_eq!(select_dropdown_panel(anchor, screen, 0, 8), None);
        assert_eq!(
            select_dropdown_panel(anchor, screen, 3, 8),
            Some(Rect::new(3, 3, 12, 5))
        );
    }
    use crate::theme::{ACCENT, BOX_INDIGO, SELECT_BG};

    /// Return the editable command one argv gets on this host, with both dialects pinned.
    ///
    /// The agent list paints its command column through [`EditableArgvDialect::host()`], so the
    /// quoting follows the platform. The two assertions keep the POSIX text and the Windows text
    /// of the same argv under test on every host, and the return value is what this host paints.
    fn host_editable_command(argv: &[&str], posix: &str, windows: &str) -> String {
        let argv: Vec<String> = argv.iter().map(|word| (*word).to_owned()).collect();
        assert_eq!(join_editable_argv(&argv, EditableArgvDialect::Posix), posix);
        assert_eq!(
            join_editable_argv(&argv, EditableArgvDialect::Windows),
            windows
        );
        join_editable_argv(&argv, EditableArgvDialect::host())
    }

    fn preferences_runner_row(index: usize, name: &str) -> skit_ui::RunnerRow {
        let identity = skit_ui::RunnerRowIdentity {
            index: Some(index),
            snapshot_token: format!("token-{index}"),
        };
        skit_ui::RunnerRow {
            key_identities: vec![identity.clone()],
            identity,
            name: Some(name.to_owned()),
            argv: Some(vec![name.to_owned(), "{{prompt}}".to_owned()]),
            reason: None,
            descriptor: format!("prompt.runners[{index}]"),
            pinned_count: 0,
        }
    }

    fn view() -> PreferencesView {
        PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: Some("vim".to_owned()),
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            theme: ThemeChoice::Terminal,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runners: vec![
                preferences_runner_row(0, "claude"),
                preferences_runner_row(1, "codex"),
            ],
            mirror: MirrorConfiguration::default(),
        }))
    }

    fn complete_view() -> PreferencesView {
        let mut view = PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: Some("vim".to_owned()),
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            theme: ThemeChoice::Terminal,
            javascript: JavascriptChoice::Automatic,
            bash_path: Some(String::new()),
            runners: vec![
                preferences_runner_row(0, "claude"),
                preferences_runner_row(1, "codex"),
            ],
            mirror: MirrorConfiguration::default(),
        }));
        for field in [
            PreferencesField::PypiMirror,
            PreferencesField::GithubMirror,
            PreferencesField::NpmMirror,
        ] {
            view.update(PreferencesAction::ChooseMirror {
                field,
                choice: MirrorChoice::Custom,
            });
        }
        view
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn mouse(area: Rect, kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        })
    }

    const fn control_id(hit: &PreferencesHit) -> PreferencesControlId {
        match hit {
            PreferencesHit::Control(id)
            | PreferencesHit::Radio { id, .. }
            | PreferencesHit::Dropdown { id, .. } => *id,
            // Every agent row and every row chip belongs to the agent list.
            PreferencesHit::RunnerRow(_) | PreferencesHit::RunnerChip { .. } => {
                PreferencesControlId::Runners
            }
        }
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
    fn the_agent_list_paints_one_row_per_draft_row_with_its_staging_marker() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::NewRunner);
        view.update(PreferencesAction::RunnerStaged(
            skit_ui::RunnerSaveRequest {
                name: "my-agent".to_owned(),
                argv: vec!["my-agent".to_owned(), "{{prompt}}".to_owned()],
                target: skit_ui::RunnerSaveTarget::New,
            },
        ));
        view.update(PreferencesAction::RunnerCursor(1));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        view.update(PreferencesAction::RunnerCursor(0));
        view.update(PreferencesAction::RunnerStaged(
            skit_ui::RunnerSaveRequest {
                name: "claude".to_owned(),
                argv: vec![
                    "claude".to_owned(),
                    "--fast".to_owned(),
                    "{{prompt}}".to_owned(),
                ],
                target: skit_ui::RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: Vec::new(),
                },
            },
        ));

        let terminal = draw(&mut session, &view, 80, 40, Locale::En);
        let rendered = text(terminal.backend().buffer());

        assert!(rendered.contains("claude"), "{rendered}");
        let painted = host_editable_command(
            &["claude", "--fast", "{{prompt}}"],
            "claude --fast '{{prompt}}'",
            "claude --fast {{prompt}}",
        );
        assert!(rendered.contains(&painted), "{rendered}");
        assert!(rendered.contains("Will be removed"), "{rendered}");
        assert!(rendered.contains("Added"), "{rendered}");
        assert!(rendered.contains("Edited"), "{rendered}");
        let area = session
            .control_area(PreferencesControlId::Runners)
            .expect("the agent list is visible");
        // Three draft rows: a row this wide still holds the chips of the cursor row.
        assert_eq!(area.height, 3);
        // A row too short for its chips takes a line for them, and it leaves with the focus.
        let _ = draw(&mut session, &view, 46, 40, Locale::En);
        assert_eq!(
            session
                .control_area(PreferencesControlId::Runners)
                .expect("the agent list is visible")
                .height,
            4
        );
        view.update(PreferencesAction::Focus(PreferencesControlId::NewRunner));
        let _ = draw(&mut session, &view, 46, 40, Locale::En);
        assert_eq!(
            session
                .control_area(PreferencesControlId::Runners)
                .expect("the agent list is visible")
                .height,
            3
        );
    }

    #[test]
    fn the_agent_cursor_marker_follows_the_focused_row_and_leaves_with_the_focus() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));

        let terminal = draw(&mut session, &view, 80, 40, Locale::En);
        let area = session
            .control_area(PreferencesControlId::Runners)
            .expect("the agent list is visible");
        let markers = |terminal: &Terminal<TestBackend>| {
            let buffer = terminal.backend().buffer();
            (buffer.area.y..buffer.area.bottom())
                .flat_map(|row| {
                    (buffer.area.x..buffer.area.right()).map(move |column| (column, row))
                })
                .filter(|position| buffer[*position].symbol() == "▶")
                .collect::<Vec<_>>()
        };
        assert_eq!(markers(&terminal), [(area.x, area.y)]);

        view.update(PreferencesAction::RunnerCursorNext);
        let terminal = draw(&mut session, &view, 80, 40, Locale::En);
        assert_eq!(markers(&terminal), [(area.x, area.y + 1)]);

        view.update(PreferencesAction::Focus(PreferencesControlId::NewRunner));
        let terminal = draw(&mut session, &view, 80, 40, Locale::En);
        let door = session
            .control_area(PreferencesControlId::NewRunner)
            .expect("the new-agent door is visible");
        assert_eq!(markers(&terminal), [(door.x.saturating_sub(2), door.y)]);
    }

    /// A scroll can hide the cursor row while the rest of the list stays. The cue must survive.
    #[test]
    fn a_clipped_agent_list_keeps_its_focus_cue_on_the_first_visible_row() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        let _ = draw(&mut session, &view, 80, 40, Locale::En);
        let control = view
            .control(PreferencesControlId::Runners)
            .expect("the agent list is reachable");

        let height = control_height(&control, Locale::En, 44, true);
        assert_eq!(
            height, 3,
            "the focused cursor row keeps a chip line at 44 cells"
        );
        let mut terminal = Terminal::new(TestBackend::new(44, 1)).unwrap();
        terminal
            .draw(|frame| {
                session.render_control(
                    frame,
                    RowClip::new(height, height - 1, frame.area()),
                    &control,
                    &view,
                    Locale::En,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "▶");
        assert_eq!(buffer[(0, 0)].fg, ACCENT);
        let row = (0..buffer.area.width)
            .map(|column| buffer[(column, 0)].symbol())
            .collect::<String>();
        assert!(row.contains("codex"), "{row}");
    }

    /// Return the rectangle of every chip the latest frame painted.
    fn chip_areas(session: &PreferencesWidgetSession) -> Vec<(usize, RunnerChip, Rect)> {
        session.runner_chip_areas.clone()
    }

    /// Return the text of one painted terminal row.
    fn row_text(buffer: &Buffer, row: u16) -> String {
        (buffer.area.x..buffer.area.right())
            .map(|column| buffer[(column, row)].symbol())
            .collect()
    }

    /// Return the text of one painted terminal row without the filler cell of a wide glyph.
    fn row_glyphs(buffer: &Buffer, row: u16) -> String {
        let mut rendered = String::new();
        let mut column = buffer.area.x;
        while column < buffer.area.right() {
            let symbol = buffer[(column, row)].symbol();
            rendered.push_str(symbol);
            column = column.saturating_add(u16::try_from(symbol.width().max(1)).unwrap_or(1));
        }
        rendered
    }

    #[test]
    fn only_the_cursor_row_of_the_focused_list_offers_its_command_chips() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        let _ = draw(&mut session, &view, 120, 40, Locale::En);
        assert!(
            chip_areas(&session).is_empty(),
            "an unfocused list advertises no chip"
        );
        assert!(
            session
                .screen_target_inventory(&view)
                .unwrap()
                .available
                .iter()
                .all(|target| !matches!(target, ScreenTarget::RunnerChip { .. })),
            "an unfocused list publishes no chip target"
        );

        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        let terminal = draw(&mut session, &view, 120, 40, Locale::En);
        let area = session
            .control_area(PreferencesControlId::Runners)
            .expect("the agent list is visible");
        // A wide tier keeps the chips beside the cursor row.
        assert_eq!(area.height, 2);
        let chips = chip_areas(&session);
        assert_eq!(
            chips
                .iter()
                .map(|(index, chip, area)| (*index, *chip, area.y))
                .collect::<Vec<_>>(),
            [
                (0, RunnerChip::Edit, area.y),
                (0, RunnerChip::Remove, area.y),
            ]
        );
        let cursor_row = row_text(terminal.backend().buffer(), area.y);
        assert!(cursor_row.contains("Enter Edit"), "{cursor_row}");
        assert!(cursor_row.contains("Del Remove"), "{cursor_row}");
        let next_row = row_text(terminal.backend().buffer(), area.y + 1);
        assert!(!next_row.contains("Edit"), "{next_row}");

        view.update(PreferencesAction::RunnerCursorNext);
        let _ = draw(&mut session, &view, 120, 40, Locale::En);
        assert!(
            chip_areas(&session)
                .iter()
                .all(|(index, _, chip_area)| *index == 1 && chip_area.y == area.y + 1)
        );
    }

    #[test]
    fn every_agent_chip_click_dispatches_the_action_of_its_key() {
        for (chip, expected) in [
            (RunnerChip::Edit, PreferencesAction::EditRunner),
            (RunnerChip::Remove, PreferencesAction::ToggleRunnerRemoval),
        ] {
            let mut session = PreferencesWidgetSession::default();
            let mut view = view();
            view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
            let _ = draw(&mut session, &view, 120, 40, Locale::En);
            let area = chip_areas(&session)
                .into_iter()
                .find_map(|(_, painted, area)| (painted == chip).then_some(area))
                .expect("the cursor row paints both chips");

            let mut handling = PreferencesEventHandling::Ignored;
            for kind in [
                MouseEventKind::Down(MouseButton::Left),
                MouseEventKind::Up(MouseButton::Left),
            ] {
                handling = session.handle_event(mouse(area, kind), &view);
            }
            assert_eq!(handling, PreferencesEventHandling::Action(expected));
        }
    }

    #[test]
    fn a_row_staged_for_removal_offers_the_restore_chip_alone() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        view.update(PreferencesAction::ToggleRunnerRemoval);

        let terminal = draw(&mut session, &view, 120, 40, Locale::En);
        let area = session
            .control_area(PreferencesControlId::Runners)
            .expect("the agent list is visible");
        let cursor_row = row_text(terminal.backend().buffer(), area.y);

        assert!(cursor_row.contains("Del Restore"), "{cursor_row}");
        assert!(!cursor_row.contains("Enter Edit"), "{cursor_row}");
        assert_eq!(
            chip_areas(&session)
                .iter()
                .map(|(_, chip, _)| *chip)
                .collect::<Vec<_>>(),
            [RunnerChip::Remove]
        );

        view.update(PreferencesAction::ToggleRunnerRemoval);
        let terminal = draw(&mut session, &view, 120, 40, Locale::En);
        let cursor_row = row_text(terminal.backend().buffer(), area.y);
        assert!(cursor_row.contains("Enter Edit"), "{cursor_row}");
        assert!(cursor_row.contains("Del Remove"), "{cursor_row}");
    }

    /// The fit rule pins one exact cell: a name column, a command column, and their separators.
    #[test]
    fn the_chip_line_appears_one_cell_below_the_row_that_still_fits_its_chips() {
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        let control = view
            .control(PreferencesControlId::Runners)
            .expect("the agent list is reachable");
        // "Enter Edit" and "Del Remove" take 25 cells, and the row keeps 18 of its own.
        assert_eq!(
            control_height(&control, Locale::En, 45, true),
            2,
            "the exact fit must keep the chips on the cursor row"
        );
        assert_eq!(
            control_height(&control, Locale::En, 44, true),
            3,
            "one cell less than the exact fit must move the chips below"
        );
    }

    /// A row with room for its name, its command, and its chips keeps every chip on the row.
    #[test]
    fn a_row_that_can_hold_its_chips_keeps_them_beside_the_command_column() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            let mut session = PreferencesWidgetSession::default();
            let mut view = view();
            view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
            let terminal = draw(&mut session, &view, 80, 40, locale);
            let area = session
                .control_area(PreferencesControlId::Runners)
                .expect("the agent list is visible");
            assert_eq!(area.height, 2, "{locale:?}: two rows and no chip line");
            let cursor_row = row_glyphs(terminal.backend().buffer(), area.y);
            assert!(cursor_row.contains("claude"), "{locale:?}: {cursor_row}");
            for label in ["Edit", "Remove"] {
                assert!(
                    cursor_row.contains(skit_i18n::text(locale, label).as_ref()),
                    "{locale:?}: {cursor_row}"
                );
            }
            let chips = chip_areas(&session);
            assert!(
                chips
                    .iter()
                    .all(|(index, _, chip)| *index == 0 && chip.y == area.y),
                "{locale:?}: {chips:?}"
            );
        }
    }

    /// A narrow terminal has no room beside the command column, so the chips take their own line.
    #[test]
    fn a_narrow_tier_moves_the_chips_below_the_cursor_row_and_keeps_the_name() {
        // 46 cells hold both chips on one line below the row; 24 need a line for each.
        for (width, chip_lines) in [(46_u16, 1_u16), (24, 2)] {
            let mut session = PreferencesWidgetSession::default();
            let mut view = view();
            view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
            let terminal = draw(&mut session, &view, width, 40, Locale::En);
            let buffer = terminal.backend().buffer();
            let area = session
                .control_area(PreferencesControlId::Runners)
                .expect("the agent list is visible");
            assert_eq!(area.height, 2 + chip_lines, "{width}");

            let cursor_row = row_text(buffer, area.y);
            assert!(cursor_row.contains("claude"), "{width}: {cursor_row}");
            assert!(!cursor_row.contains("Edit"), "{width}: {cursor_row}");
            let chips = chip_areas(&session);
            assert_eq!(chips.len(), 2, "{width}: {chips:?}");
            assert!(
                chips.iter().all(|(index, _, chip_area)| *index == 0
                    && (area.y + 1..area.y + 1 + chip_lines).contains(&chip_area.y)
                    && chip_area.right() <= buffer.area.right()
                    && !chip_area.is_empty()),
                "{width}: {chips:?}"
            );
            // The second draft row keeps the line below the chips.
            assert!(
                row_text(buffer, area.y + 1 + chip_lines).contains("codex"),
                "{width}"
            );
            // The chip line belongs to its row, and the walker still sees one rect per row.
            assert_eq!(session.runner_row_areas.len(), 2, "{width}");
            assert_eq!(
                session
                    .clicks
                    .handle_click(area.x, area.y + 1)
                    .expect("the chip line moves the cursor"),
                &PreferencesHit::RunnerRow(0),
                "{width}"
            );
        }
    }

    #[test]
    fn the_command_column_loses_its_cells_before_the_name_column_does() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        let terminal = draw(&mut session, &view, 30, 40, Locale::En);
        let area = session
            .control_area(PreferencesControlId::Runners)
            .expect("the agent list is visible");
        let cursor_row = row_text(terminal.backend().buffer(), area.y);

        assert!(cursor_row.contains("claude"), "{cursor_row}");
        assert!(cursor_row.contains('…'), "{cursor_row}");
        assert!(!cursor_row.contains("{{prompt}}"), "{cursor_row}");
    }

    #[test]
    fn a_tiny_terminal_paints_the_agent_list_without_a_panic() {
        for width in 1_u16..=12 {
            let mut session = PreferencesWidgetSession::default();
            let mut view = view();
            view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
            view.update(PreferencesAction::ToggleRunnerRemoval);
            let terminal = draw(&mut session, &view, width, 12, Locale::En);
            let buffer = terminal.backend().buffer();
            assert!(
                chip_areas(&session)
                    .iter()
                    .all(|(_, _, area)| area.right() <= buffer.area.right()),
                "{width}"
            );
        }
    }

    /// A chip is the mouse path to its key, so a short band wraps the chips instead of dropping one.
    #[test]
    fn a_short_chip_band_wraps_the_chips_and_keeps_every_key_clickable() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            let mut session = PreferencesWidgetSession::default();
            let mut view = view();
            view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
            let terminal = draw(&mut session, &view, 24, 12, locale);
            let buffer = terminal.backend().buffer();
            let chips = chip_areas(&session);

            assert_eq!(chips.len(), 2, "{locale:?}: {chips:?}");
            for (index, chip, area) in &chips {
                assert_eq!(*index, 0, "{locale:?}");
                let label = match chip {
                    RunnerChip::Edit => "Edit",
                    RunnerChip::Remove => "Remove",
                };
                let painted = row_glyphs(buffer, area.y);
                assert!(
                    painted.contains(skit_i18n::text(locale, label).as_ref()),
                    "{locale:?}: {painted}"
                );
                assert!(area.right() <= buffer.area.right(), "{locale:?}: {area:?}");
                assert_eq!(
                    session.clicks.handle_click(area.x, area.y),
                    Some(&PreferencesHit::RunnerChip {
                        index: 0,
                        chip: *chip
                    }),
                    "{locale:?}: {chip:?} is not clickable"
                );
            }
            // Each chip took a line of its own, and the rows below still follow.
            assert_ne!(chips[0].2.y, chips[1].2.y, "{locale:?}");
        }
    }

    #[test]
    fn a_staged_removal_counts_the_prompts_it_leaves_without_an_agent() {
        for (pinned, expected) in [
            (
                1_usize,
                "1 prompt pins this runner and will need another runner before it can run again.",
            ),
            (
                3,
                "3 prompts pin this runner and will need another runner before they can run again.",
            ),
        ] {
            let mut pinned_row = preferences_runner_row(0, "claude");
            pinned_row.pinned_count = pinned;
            let mut view =
                PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
                    language: String::new(),
                    available_languages: vec!["en".to_owned()],
                    effective_language: "en".to_owned(),
                    editor: String::new(),
                    editor_fallback: None,
                    form: InteractiveFormChoice::Tui,
                    after_run: AfterRunChoice::Exit,
                    theme: ThemeChoice::Terminal,
                    javascript: JavascriptChoice::Automatic,
                    bash_path: None,
                    runners: vec![pinned_row],
                    mirror: MirrorConfiguration::default(),
                }));
            let mut session = PreferencesWidgetSession::default();
            let terminal = draw(&mut session, &view, 160, 40, Locale::En);
            assert!(!text(terminal.backend().buffer()).contains(expected));

            view.update(PreferencesAction::ToggleRunnerRemoval);
            let terminal = draw(&mut session, &view, 160, 40, Locale::En);
            let rendered = text(terminal.backend().buffer());
            assert!(rendered.contains(expected), "{rendered}");
        }
    }

    /// The prompt count is an extra on the staging marker, so a short row drops the count first.
    #[test]
    fn a_short_agent_row_drops_its_prompt_count_before_its_staging_marker() {
        let mut pinned_row = preferences_runner_row(0, "claude");
        pinned_row.pinned_count = 2;
        let mut view = PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: None,
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            theme: ThemeChoice::Terminal,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runners: vec![pinned_row],
            mirror: MirrorConfiguration::default(),
        }));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        let rows = view.draft().runner_rows();
        let painted = |width| {
            runner_row_spans(&rows[0], false, Locale::En, width)
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };

        let wide = painted(200);
        assert!(wide.contains("Will be removed · 2 prompts pin"), "{wide}");
        let short = painted(30);
        assert!(short.contains("Will be removed"), "{short}");
        assert!(!short.contains("prompts pin"), "{short}");
        // A row too short for the marker alone keeps its name and drops every note.
        let tiny = painted(8);
        assert!(tiny.contains("claude"), "{tiny}");
        assert!(!tiny.contains("Will be"), "{tiny}");
    }

    /// The staging marker outranks the command chips, so a removal always says what the save does.
    #[test]
    fn a_staged_removal_keeps_its_marker_at_the_width_that_still_fits_the_chips() {
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        let mut session = PreferencesWidgetSession::default();
        let terminal = draw(&mut session, &view, 47, 40, Locale::En);
        let area = session
            .control_area(PreferencesControlId::Runners)
            .expect("the agent list is visible");
        let rendered = text(terminal.backend().buffer());

        assert!(rendered.contains("Will be removed"), "{rendered}");
        // The chips take the line below, because the row itself now carries the marker.
        assert_eq!(area.height, 3, "two rows and one chip line");
        let chips = chip_areas(&session);
        assert!(!chips.is_empty(), "the chips stay reachable");
        assert!(
            chips
                .iter()
                .all(|(index, _, chip)| *index == 0 && chip.y == area.y + 1),
            "{chips:?}"
        );
    }

    /// A duplicate row goes with the key it repeats, so it says so and commands nothing.
    #[test]
    fn a_row_the_key_change_takes_says_so_and_offers_no_command() {
        let mut duplicate = preferences_runner_row(1, "claude");
        duplicate.reason = Some("duplicate".to_owned());
        let mut view = PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: None,
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            theme: ThemeChoice::Terminal,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runners: vec![preferences_runner_row(0, "claude"), duplicate],
            mirror: MirrorConfiguration::default(),
        }));
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        let mut session = PreferencesWidgetSession::default();

        // Nothing is staged yet, so the duplicate row keeps its own commands.
        view.update(PreferencesAction::RunnerCursor(1));
        let terminal = draw(&mut session, &view, 160, 40, Locale::En);
        let area = session
            .control_area(PreferencesControlId::Runners)
            .expect("the agent list is visible");
        assert!(!row_text(terminal.backend().buffer(), area.y + 1).contains("Will be removed"));
        assert!(!chip_areas(&session).is_empty());

        // The key removal takes the duplicate with it.
        view.update(PreferencesAction::RunnerCursor(0));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        view.update(PreferencesAction::RunnerCursor(1));
        let terminal = draw(&mut session, &view, 160, 40, Locale::En);
        let buffer = terminal.backend().buffer();

        for row in [area.y, area.y + 1] {
            let painted = row_text(buffer, row);
            assert!(painted.contains("Will be removed"), "{painted}");
        }
        assert!(
            chip_areas(&session).is_empty(),
            "a row the key takes offers no command of its own"
        );
        // The walker sees the same list, so it advertises no command for that row either.
        let inventory = session.screen_target_inventory(&view).unwrap();
        assert!(
            !inventory
                .available
                .iter()
                .any(|target| matches!(target, ScreenTarget::RunnerChip { .. })),
            "{:?}",
            inventory.available
        );

        // A narrow terminal keeps no line for chips the row does not have.
        let _ = draw(&mut session, &view, 30, 40, Locale::En);
        assert_eq!(
            session
                .control_area(PreferencesControlId::Runners)
                .expect("the agent list is visible")
                .height,
            2,
            "a row with no chips takes no chip line"
        );
    }

    /// The chips pack into lines: the exact fit keeps one line, one cell less takes two.
    #[test]
    fn the_chip_packing_pins_the_cell_that_separates_one_line_from_two() {
        let chips = [
            RunnerRowChip {
                chip: RunnerChip::Edit,
                key: "Enter",
                label: "Edit",
            },
            RunnerRowChip {
                chip: RunnerChip::Remove,
                key: "Del",
                label: "Remove",
            },
        ];
        let exact = runner_chips_width(&chips, Locale::En);

        assert_eq!(runner_chip_lines(&chips, exact, Locale::En).len(), 1);
        assert_eq!(runner_chip_lines(&chips, exact - 1, Locale::En).len(), 2);
        // A band too short for one complete chip holds none of them.
        let widest = runner_chip_width(&chips[1], Locale::En);
        assert_eq!(runner_chip_lines(&chips[1..], widest, Locale::En).len(), 1);
        assert!(runner_chip_lines(&chips[1..], widest - 1, Locale::En).is_empty());
    }

    /// A row no prompt pins must not invent a dependency warning of its own.
    #[test]
    fn an_unpinned_staged_removal_prints_no_pin_warning() {
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        let mut session = PreferencesWidgetSession::default();
        let terminal = draw(&mut session, &view, 160, 40, Locale::En);
        let rendered = text(terminal.backend().buffer());

        assert!(rendered.contains("Will be removed"), "{rendered}");
        assert!(!rendered.contains("0 prompt"), "{rendered}");
        assert!(!rendered.contains("prompts pin"), "{rendered}");
        assert!(!rendered.contains("prompt pins"), "{rendered}");
    }

    /// A refused restore must say why, on the row band the user is looking at.
    #[test]
    fn a_refused_restore_prints_its_reason_under_the_agent_list() {
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        view.update(PreferencesAction::ToggleRunnerRemoval);
        view.update(PreferencesAction::NewRunner);
        view.update(PreferencesAction::RunnerStaged(
            skit_ui::RunnerSaveRequest {
                name: "claude".to_owned(),
                argv: vec!["claude".to_owned(), "{{prompt}}".to_owned()],
                target: skit_ui::RunnerSaveTarget::New,
            },
        ));
        view.update(PreferencesAction::RunnerCursor(0));
        view.update(PreferencesAction::ToggleRunnerRemoval);

        let mut session = PreferencesWidgetSession::default();
        let terminal = draw(&mut session, &view, 160, 40, Locale::En);
        let rendered = text(terminal.backend().buffer());

        assert!(
            rendered.contains("Another row already uses this runner name."),
            "{rendered}"
        );

        // The refusal outlives the keystroke that raised it, so it must keep naming the agent
        // list instead of following the focus onto a control it says nothing about.
        view.update(PreferencesAction::Next);
        view.update(PreferencesAction::Next);
        let terminal = draw(&mut session, &view, 160, 40, Locale::En);
        let list = session
            .control_area(PreferencesControlId::Runners)
            .expect("the agent list is visible");
        let reason = (0..terminal.backend().buffer().area.height)
            .find(|row| row_text(terminal.backend().buffer(), *row).contains("Another row already"))
            .expect("the refusal stays on screen");
        assert_eq!(reason, list.bottom(), "the refusal left the agent list");
        assert_ne!(view.focused(), PreferencesControlId::Runners);
    }

    #[test]
    fn a_malformed_row_keeps_its_raw_shape_visible_and_has_no_command_column() {
        let mut session = PreferencesWidgetSession::default();
        let view = PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: None,
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            theme: ThemeChoice::Terminal,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runners: vec![skit_ui::RunnerRow {
                identity: skit_ui::RunnerRowIdentity {
                    index: None,
                    snapshot_token: "container".to_owned(),
                },
                name: None,
                argv: None,
                reason: Some("row-not-table".to_owned()),
                descriptor: "prompt.runners".to_owned(),
                key_identities: Vec::new(),
                pinned_count: 0,
            }],
            mirror: MirrorConfiguration::default(),
        }));

        let terminal = draw(&mut session, &view, 160, 40, Locale::En);
        let rendered = text(terminal.backend().buffer());

        assert!(rendered.contains("⚠ prompt.runners"), "{rendered}");
        assert!(
            rendered.contains("This runner row isn't a table."),
            "{rendered}"
        );
    }

    /// Every malformed-row reason code prints its own localized text on its own agent row.
    #[test]
    fn every_runner_reason_renders_its_localized_text_on_its_own_row() {
        let cases = [
            (
                "prompt-section-not-table",
                "the prompt value is not a table; repair it before runner management",
            ),
            (
                "runners-not-list",
                "the prompt.runners value is not a list; repair it before runner management",
            ),
            (
                "empty",
                "Type the agent's command, e.g. mycli run {{prompt}}",
            ),
            (
                "prompt-slot-count",
                "The command needs the {{prompt}} slot exactly once — that's where the rendered prompt lands.",
            ),
            (
                "prompt-in-binary",
                "{{prompt}} can't be the command itself — the first word must be the program to run.",
            ),
            (
                "stray-hole",
                "Runner commands take only the {{prompt}} slot — single-brace text is literal, and other {{holes}} aren't supported.",
            ),
            ("name", "A name is required."),
            ("argv-type", "The command must be a list of text arguments."),
            ("row-not-table", "This runner row isn't a table."),
            ("duplicate", "Another row already uses this runner name."),
            ("future-code", "This runner row is malformed."),
        ];
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            for (code, source) in cases {
                let mut malformed = preferences_runner_row(0, "claude");
                malformed.name = None;
                malformed.argv = None;
                malformed.reason = Some(code.to_owned());
                let mut session = PreferencesWidgetSession::default();
                let view =
                    PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
                        language: String::new(),
                        available_languages: vec!["en".to_owned()],
                        effective_language: "en".to_owned(),
                        editor: String::new(),
                        editor_fallback: None,
                        form: InteractiveFormChoice::Tui,
                        after_run: AfterRunChoice::Exit,
                        theme: ThemeChoice::Terminal,
                        javascript: JavascriptChoice::Automatic,
                        bash_path: None,
                        runners: vec![malformed],
                        mirror: MirrorConfiguration::default(),
                    }));
                let terminal = draw(&mut session, &view, 240, 40, locale);
                let area = session.runner_row_areas[0].1;
                let rendered = row_glyphs(terminal.backend().buffer(), area.y);
                let expected = skit_i18n::text(locale, source);
                assert!(
                    rendered.contains(expected.as_ref()),
                    "reason={code:?}, locale={locale:?}, row={rendered:?}, expected={expected:?}"
                );
            }
        }
    }

    /// A malformed row with a raw shape is repairable. Enter opens the repair editor on it.
    #[test]
    fn a_repairable_malformed_row_offers_both_chips_and_a_shapeless_one_offers_removal() {
        let repairable = skit_ui::RunnerRow {
            identity: skit_ui::RunnerRowIdentity {
                index: Some(0),
                snapshot_token: "row-0".to_owned(),
            },
            name: None,
            argv: Some(vec!["agent".to_owned(), "{{prompt}}".to_owned()]),
            reason: Some("name".to_owned()),
            descriptor: "prompt.runners[0]".to_owned(),
            key_identities: Vec::new(),
            pinned_count: 0,
        };
        let shapeless = skit_ui::RunnerRow {
            identity: skit_ui::RunnerRowIdentity {
                index: None,
                snapshot_token: "container".to_owned(),
            },
            name: None,
            argv: None,
            reason: Some("row-not-table".to_owned()),
            descriptor: "prompt.runners".to_owned(),
            key_identities: Vec::new(),
            pinned_count: 0,
        };
        let mut view = PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: None,
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            theme: ThemeChoice::Terminal,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runners: vec![repairable, shapeless],
            mirror: MirrorConfiguration::default(),
        }));
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));

        let mut session = PreferencesWidgetSession::default();
        let _ = draw(&mut session, &view, 160, 40, Locale::En);
        assert_eq!(
            chip_areas(&session)
                .iter()
                .map(|(_, chip, _)| *chip)
                .collect::<Vec<_>>(),
            [RunnerChip::Edit, RunnerChip::Remove]
        );
        // The repair keeps the raw reason until the editor stages new values.
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &view),
            PreferencesEventHandling::Action(PreferencesAction::EditRunner)
        );
        view.update(PreferencesAction::RunnerStaged(
            skit_ui::RunnerSaveRequest {
                name: "repaired".to_owned(),
                argv: vec!["repaired".to_owned(), "{{prompt}}".to_owned()],
                target: skit_ui::RunnerSaveTarget::RawRow {
                    expected: skit_ui::RunnerRowIdentity {
                        index: Some(0),
                        snapshot_token: "row-0".to_owned(),
                    },
                },
            },
        ));
        let terminal = draw(&mut session, &view, 160, 40, Locale::En);
        let rendered = text(terminal.backend().buffer());
        assert!(!rendered.contains("A name is required."), "{rendered}");
        assert!(rendered.contains("Edited"), "{rendered}");

        view.update(PreferencesAction::RunnerCursor(1));
        let _ = draw(&mut session, &view, 160, 40, Locale::En);
        assert_eq!(
            chip_areas(&session)
                .iter()
                .map(|(_, chip, _)| *chip)
                .collect::<Vec<_>>(),
            [RunnerChip::Remove]
        );
        assert_eq!(
            view.update(PreferencesAction::EditRunner),
            skit_ui::PreferencesEffect::None
        );
    }

    #[test]
    fn the_agent_list_publishes_one_walker_target_per_row_and_its_cursor_chips() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: None,
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            theme: ThemeChoice::Terminal,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runners: vec![
                preferences_runner_row(0, "claude"),
                preferences_runner_row(1, "claude"),
                skit_ui::RunnerRow {
                    identity: skit_ui::RunnerRowIdentity {
                        index: None,
                        snapshot_token: "container".to_owned(),
                    },
                    name: None,
                    argv: None,
                    reason: Some("row-not-table".to_owned()),
                    descriptor: "prompt.runners".to_owned(),
                    key_identities: Vec::new(),
                    pinned_count: 0,
                },
            ],
            mirror: MirrorConfiguration::default(),
        }));
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        let _ = draw(&mut session, &view, 120, 40, Locale::En);

        let inventory = session.screen_target_inventory(&view).unwrap();
        // A repeated name stays addressable, and a malformed row publishes no name at all.
        let rows = [
            ScreenTarget::Runner {
                row: 0,
                name: Some("claude".to_owned()),
            },
            ScreenTarget::Runner {
                row: 1,
                name: Some("claude".to_owned()),
            },
            ScreenTarget::Runner { row: 2, name: None },
        ];
        for target in &rows {
            assert!(inventory.available.contains(target), "{target:?}");
            assert_eq!(
                inventory
                    .hits
                    .iter()
                    .filter(|hit| hit.target == *target)
                    .count(),
                1,
                "{target:?}"
            );
        }
        for chip in [RunnerChip::Edit, RunnerChip::Remove] {
            let target = ScreenTarget::RunnerChip { row: 0, chip };
            assert!(inventory.available.contains(&target), "{target:?}");
            assert!(
                inventory.hits.iter().any(|hit| hit.target == target),
                "{target:?}"
            );
        }
        // The list itself is not a hit: every cell of it belongs to one row.
        assert!(
            !inventory
                .hits
                .iter()
                .any(|hit| hit.target == ScreenTarget::Preferences(PreferencesControlId::Runners)),
            "{:?}",
            inventory.hits
        );
        assert!(
            inventory
                .available
                .contains(&ScreenTarget::Preferences(PreferencesControlId::Runners))
        );

        view.update(PreferencesAction::RunnerCursor(2));
        let _ = draw(&mut session, &view, 120, 40, Locale::En);
        let inventory = session.screen_target_inventory(&view).unwrap();
        assert!(!inventory.available.contains(&ScreenTarget::RunnerChip {
            row: 2,
            chip: RunnerChip::Edit,
        }));
        assert!(inventory.available.contains(&ScreenTarget::RunnerChip {
            row: 2,
            chip: RunnerChip::Remove,
        }));
    }

    #[test]
    fn the_focused_agent_list_maps_every_advertised_key_to_one_typed_action() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));
        let _ = draw(&mut session, &view, 80, 40, Locale::En);
        assert!(session.focused_owns_vertical_navigation(&view));

        // A control with its own rows or options takes Down. A door and an input do not.
        for id in [
            PreferencesControlId::Editor,
            PreferencesControlId::NewRunner,
        ] {
            view.update(PreferencesAction::Focus(id));
            assert!(
                !session.focused_owns_vertical_navigation(&view),
                "{id:?} must leave vertical navigation to the focus ring"
            );
        }
        view.update(PreferencesAction::Focus(PreferencesControlId::Runners));

        for (code, expected) in [
            (KeyCode::Down, PreferencesAction::RunnerCursorNext),
            (KeyCode::Up, PreferencesAction::RunnerCursorPrevious),
            (KeyCode::Enter, PreferencesAction::EditRunner),
            (KeyCode::Delete, PreferencesAction::ToggleRunnerRemoval),
            (KeyCode::Backspace, PreferencesAction::ToggleRunnerRemoval),
        ] {
            assert_eq!(
                session.handle_event(key(code, KeyModifiers::NONE), &view),
                PreferencesEventHandling::Action(expected),
                "{code:?} must reach the reducer"
            );
        }
        assert_eq!(
            session.handle_event(key(KeyCode::Char('x'), KeyModifiers::NONE), &view),
            PreferencesEventHandling::Ignored
        );
    }

    #[test]
    fn clicking_an_agent_row_moves_the_cursor_there_and_focuses_the_list() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        let _ = draw(&mut session, &view, 80, 40, Locale::En);
        let area = session
            .control_area(PreferencesControlId::Runners)
            .expect("the agent list is visible");

        // The gutter belongs to the row, so the complete row width moves the cursor.
        for (row, expected) in [(area.y, 0_usize), (area.y + 1, 1)] {
            let mut handling = PreferencesEventHandling::Ignored;
            for kind in [
                MouseEventKind::Down(MouseButton::Left),
                MouseEventKind::Up(MouseButton::Left),
            ] {
                handling = session.handle_event(
                    Event::Mouse(MouseEvent {
                        kind,
                        column: area.x,
                        row,
                        modifiers: KeyModifiers::NONE,
                    }),
                    &view,
                );
            }
            assert_eq!(
                handling,
                PreferencesEventHandling::Action(PreferencesAction::RunnerCursor(expected))
            );
        }
        view.update(PreferencesAction::RunnerCursor(1));
        assert_eq!(view.focused(), PreferencesControlId::Runners);
        assert_eq!(view.runner_cursor(), 1);

        assert_eq!(
            button_action(PreferencesControlId::NewRunner),
            PreferencesEventHandling::Action(PreferencesAction::NewRunner)
        );
    }

    #[test]
    fn language_picker_mouse_dismissal_owns_the_anchor_panel_and_outside() {
        for target in ["anchor", "panel", "outside"] {
            let mut session = PreferencesWidgetSession::default();
            let view = view();
            let _ = draw(&mut session, &view, 80, 30, Locale::En);
            let anchor = session
                .control_area(PreferencesControlId::Language)
                .unwrap();
            for kind in [
                MouseEventKind::Down(MouseButton::Left),
                MouseEventKind::Up(MouseButton::Left),
            ] {
                assert_eq!(
                    session.handle_event(mouse(anchor, kind), &view),
                    PreferencesEventHandling::Consumed
                );
            }
            let _ = draw(&mut session, &view, 80, 30, Locale::En);
            assert!(!language_picker_snapshot(&session).0.is_empty());
            let area = match target {
                "anchor" => anchor,
                "panel" => language_picker_panel(&session),
                _ => Rect::new(79, 29, 1, 1),
            };
            assert_eq!(
                session.handle_event(mouse(area, MouseEventKind::Down(MouseButton::Left)), &view),
                PreferencesEventHandling::Consumed
            );
            let _ = draw(&mut session, &view, 80, 30, Locale::En);
            assert!(
                !language_picker_snapshot(&session).0.is_empty(),
                "press alone must not close the picker"
            );
            assert_eq!(
                session.handle_event(mouse(area, MouseEventKind::Up(MouseButton::Left)), &view),
                PreferencesEventHandling::Consumed
            );
            let _ = draw(&mut session, &view, 80, 30, Locale::En);
            assert!(
                language_picker_snapshot(&session).0.is_empty(),
                "{target} must dismiss the picker without activating another control"
            );
        }
    }

    fn open_language_picker(session: &mut PreferencesWidgetSession) {
        let widget = session
            .widgets
            .get_mut(&PreferencesControlId::Language)
            .expect("the language control exists");
        assert!(matches!(widget, PreferencesWidget::Choice { .. }));
        if let PreferencesWidget::Choice { state, .. } = widget {
            state.open();
        }
    }

    fn language_picker_snapshot(
        session: &PreferencesWidgetSession,
    ) -> (Vec<ClickRegion<SelectAction>>, usize, Vec<String>) {
        let widget = session
            .widgets
            .get(&PreferencesControlId::Language)
            .expect("the language control exists");
        assert!(matches!(widget, PreferencesWidget::Choice { .. }));
        let mut snapshot = None;
        if let PreferencesWidget::Choice {
            state,
            dropdown_regions,
            values,
            ..
        } = widget
        {
            snapshot = Some((
                dropdown_regions.clone(),
                state.highlighted_index,
                values.clone(),
            ));
        }
        snapshot.expect("the language control is a picker")
    }

    fn language_picker_panel(session: &PreferencesWidgetSession) -> Rect {
        let widget = session
            .widgets
            .get(&PreferencesControlId::Language)
            .unwrap();
        let mut panel = None;
        if let PreferencesWidget::Choice { dropdown_panel, .. } = widget {
            panel = *dropdown_panel;
        }
        panel.expect("the rendered language picker has a panel")
    }

    #[test]
    fn screen_target_inventory_refuses_stale_control_and_agent_shapes() {
        let preferences = view();
        assert_eq!(
            PreferencesWidgetSession::default().screen_target_inventory(&preferences),
            Err(ScreenTargetError::StaleSession)
        );

        let target = AgentTarget {
            name: "codex".to_owned(),
            scope: AgentScope::User,
            base: PathBuf::from("/tmp/codex"),
        };
        let mut agent_view = view();
        agent_view.update(PreferencesAction::PresentAgentSkillTargets(vec![
            target.clone(),
        ]));
        assert_eq!(
            PreferencesWidgetSession::default().screen_target_inventory(&agent_view),
            Err(ScreenTargetError::StaleSession)
        );

        let mut stale_index = PreferencesWidgetSession {
            agent_signature: Some(vec![target]),
            ..PreferencesWidgetSession::default()
        };
        stale_index
            .agent_target_areas
            .push((1, Rect::new(0, 0, 1, 1)));
        assert_eq!(
            stale_index.screen_target_inventory(&agent_view),
            Err(ScreenTargetError::StaleSession)
        );
    }

    #[test]
    fn an_open_preferences_select_drops_its_anchor_when_scrolled_out() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let _ = draw(&mut session, &view, 40, 8, Locale::En);
        open_language_picker(&mut session);
        session
            .scroll
            .set_scroll_offset(session.maximum_scroll_offset());
        let _ = draw(&mut session, &view, 40, 8, Locale::En);
        let (dropdown_regions, _, _) = language_picker_snapshot(&session);
        assert!(
            dropdown_regions.is_empty(),
            "a clipped Preferences select reused its stale on-screen anchor"
        );
    }

    #[test]
    fn open_dropdown_omits_every_underlay_hit_behind_its_full_panel() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let screen = Rect::new(0, 0, 40, 15);
        let _ = draw(&mut session, &view, screen.width, screen.height, Locale::En);
        open_language_picker(&mut session);
        let terminal = draw(&mut session, &view, screen.width, screen.height, Locale::En);

        let (option_regions, _, _) = language_picker_snapshot(&session);
        let panel = language_picker_panel(&session);
        assert!(!panel.is_empty(), "the dropdown paints a panel");
        for (x, y, symbol) in [
            (panel.x, panel.y, "┌"),
            (panel.right() - 1, panel.y, "┐"),
            (panel.x, panel.bottom() - 1, "└"),
            (panel.right() - 1, panel.bottom() - 1, "┘"),
        ] {
            assert_eq!(terminal.backend().buffer()[(x, y)].symbol(), symbol);
        }
        assert!(
            !option_regions.is_empty(),
            "the open dropdown paints options"
        );
        let covered = session
            .control_areas
            .iter()
            .filter(|(_, area)| !area.intersection(panel).is_empty())
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            !covered.is_empty(),
            "the real dropdown panel {panel:?} covers at least one underlay control: {:?}",
            session.control_areas
        );
        session.control_areas.push((
            PreferencesControlId::NewRunner,
            Rect::new(panel.x, panel.bottom().saturating_sub(1), 1, 1),
        ));

        let inventory = session.screen_target_inventory(&view).unwrap();
        assert!(
            inventory
                .hits
                .iter()
                .all(|hit| hit.rect.intersection(panel).is_empty()),
            "the full dropdown panel must occlude every intersecting underlay hit"
        );
    }

    #[test]
    fn clipped_open_dropdown_keeps_its_panel_when_no_option_row_fits() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let screen = Rect::new(0, 0, 40, 7);
        let _ = draw(&mut session, &view, screen.width, screen.height, Locale::En);
        open_language_picker(&mut session);
        let terminal = draw(&mut session, &view, screen.width, screen.height, Locale::En);

        let (option_regions, _, _) = language_picker_snapshot(&session);
        let panel = language_picker_panel(&session);
        assert!(!panel.is_empty(), "the clipped dropdown paints its border");
        let buffer = terminal.backend().buffer();
        let corners = (
            buffer[(panel.x, panel.y)].symbol(),
            buffer[(panel.right() - 1, panel.y)].symbol(),
        );
        assert!([("┌", "┐"), ("└", "┘")].contains(&corners));
        assert!(
            option_regions.is_empty(),
            "no option row fits inside the clipped panel"
        );
        session
            .control_areas
            .push((PreferencesControlId::NewRunner, panel));

        let inventory = session.screen_target_inventory(&view).unwrap();
        assert!(
            inventory
                .hits
                .iter()
                .all(|hit| hit.rect.intersection(panel).is_empty()),
            "the panel must occlude underlay hits even without option regions"
        );
    }

    #[test]
    fn moving_focus_closes_the_previous_picker_and_removes_its_option_hits() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Language));
        let _ = draw(&mut session, &view, 80, 30, Locale::En);
        open_language_picker(&mut session);
        let _ = draw(&mut session, &view, 80, 30, Locale::En);
        assert!(!language_picker_snapshot(&session).0.is_empty());

        view.update(PreferencesAction::Focus(PreferencesControlId::Editor));
        let _ = draw(&mut session, &view, 80, 30, Locale::En);
        assert!(language_picker_snapshot(&session).0.is_empty());
    }

    #[test]
    fn preferences_picker_preserves_equal_identity_and_cancels_changed_option_shapes() {
        let make_view = |languages: Vec<String>| {
            let mut view =
                PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
                    language: String::new(),
                    available_languages: languages,
                    effective_language: "en".to_owned(),
                    editor: String::new(),
                    editor_fallback: Some("vim".to_owned()),
                    form: InteractiveFormChoice::Tui,
                    after_run: AfterRunChoice::Exit,
                    theme: ThemeChoice::Terminal,
                    javascript: JavascriptChoice::Automatic,
                    bash_path: None,
                    runners: Vec::new(),
                    mirror: MirrorConfiguration::default(),
                }));
            view.update(PreferencesAction::Focus(PreferencesControlId::Language));
            view
        };
        let old = make_view(vec!["en".to_owned(), "zh-CN".to_owned()]);
        let replacement = make_view(vec!["en".to_owned(), "zh-TW".to_owned()]);
        let mut session = PreferencesWidgetSession::default();
        let _ = draw(&mut session, &old, 80, 30, Locale::En);
        open_language_picker(&mut session);
        let _ = draw(&mut session, &old, 80, 30, Locale::En);
        let routed_option_point = |regions: &[ClickRegion<SelectAction>], option: usize| {
            (0..30)
                .flat_map(|row| (0..80).map(move |column| (column, row)))
                .find(|(column, row)| {
                    regions
                        .iter()
                        .rev()
                        .find(|region| region.contains(*column, *row))
                        .is_some_and(|region| region.data == SelectAction::Select(option))
                })
                .map(|(column, row)| Rect::new(column, row, 1, 1))
                .expect("the requested option owns one topmost cell")
        };
        let (old_regions, _, old_values) = language_picker_snapshot(&session);
        let old_option = old_values
            .iter()
            .position(|value| value == "zh-CN")
            .expect("zh-CN value exists");
        let old_area = routed_option_point(&old_regions, old_option);

        assert_eq!(
            session.handle_event(
                mouse(old_area, MouseEventKind::Down(MouseButton::Left)),
                &old,
            ),
            PreferencesEventHandling::Consumed
        );
        let _ = draw(&mut session, &old, 80, 30, Locale::En);
        assert_eq!(
            session.handle_event(mouse(old_area, MouseEventKind::Up(MouseButton::Left)), &old,),
            PreferencesEventHandling::Action(PreferencesAction::SetLanguage("zh-CN".to_owned()))
        );

        open_language_picker(&mut session);
        let _ = draw(&mut session, &old, 80, 30, Locale::En);
        assert_eq!(
            session.handle_event(
                mouse(old_area, MouseEventKind::Down(MouseButton::Left)),
                &old,
            ),
            PreferencesEventHandling::Consumed
        );
        let _ = draw(&mut session, &replacement, 80, 30, Locale::En);
        open_language_picker(&mut session);
        let _ = draw(&mut session, &replacement, 80, 30, Locale::En);
        let (replacement_regions, _, replacement_values) = language_picker_snapshot(&session);
        let replacement_option = replacement_values
            .iter()
            .position(|value| value == "zh-TW")
            .expect("zh-TW value exists");
        assert_eq!(replacement_option, old_option);
        let replacement_area = routed_option_point(&replacement_regions, replacement_option);
        assert_eq!(replacement_area, old_area);
        assert_eq!(
            session.handle_event(
                mouse(replacement_area, MouseEventKind::Up(MouseButton::Left),),
                &replacement,
            ),
            PreferencesEventHandling::Ignored,
            "a new option identity reused an armed index",
        );
    }

    #[test]
    fn open_preferences_select_owns_wheel_before_the_underlay() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let _ = draw(&mut session, &view, 40, 8, Locale::En);
        open_language_picker(&mut session);
        let _ = draw(&mut session, &view, 40, 8, Locale::En);
        let (dropdown_regions, before_highlight, _) = language_picker_snapshot(&session);
        let option = dropdown_regions
            .first()
            .expect("a language option must be visible")
            .area;
        let before_underlay = session.scroll_offset();

        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: option.x,
                    row: option.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(session.scroll_offset(), before_underlay);
        assert_eq!(
            language_picker_snapshot(&session).1,
            before_highlight.saturating_add(1)
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollUp,
                    column: option.x,
                    row: option.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(session.scroll_offset(), before_underlay);
        assert_eq!(language_picker_snapshot(&session).1, before_highlight);
    }

    #[test]
    fn an_open_preferences_select_does_not_steal_outside_wheel_or_unrelated_keys() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let _ = draw(&mut session, &view, 40, 8, Locale::En);
        open_language_picker(&mut session);
        let _ = draw(&mut session, &view, 40, 8, Locale::En);
        let (regions, before_highlight, _) = language_picker_snapshot(&session);
        let outside = (session.viewport.y..session.viewport.bottom())
            .flat_map(|row| {
                (session.viewport.x..session.viewport.right()).map(move |column| (column, row))
            })
            .find(|(column, row)| !regions.iter().any(|region| region.contains(*column, *row)))
            .expect("the short viewport has a cell outside the dropdown");
        let before_scroll = session.scroll_offset();

        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: outside.0,
                    row: outside.1,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert!(session.scroll_offset() > before_scroll);
        assert_eq!(language_picker_snapshot(&session).1, before_highlight);
        assert_eq!(
            session.handle_event(key(KeyCode::F(2), KeyModifiers::NONE), &view),
            PreferencesEventHandling::Ignored
        );
    }

    #[test]
    fn nonoption_and_stale_dropdown_hits_are_inert() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let _ = draw(&mut session, &view, 80, 30, Locale::En);
        let area = Rect::new(0, 0, 1, 1);

        let language = session
            .widgets
            .get_mut(&PreferencesControlId::Language)
            .expect("the language control exists");
        assert!(matches!(language, PreferencesWidget::Choice { .. }));
        if let PreferencesWidget::Choice {
            state,
            dropdown_regions,
            ..
        } = language
        {
            state.open();
            dropdown_regions.clear();
            dropdown_regions.push(ClickRegion::new(area, SelectAction::Focus));
        }
        session.clicks.clear();
        assert_eq!(
            session.handle_event(mouse(area, MouseEventKind::Down(MouseButton::Left)), &view,),
            PreferencesEventHandling::Consumed
        );

        assert_eq!(
            session.handle_event(mouse(area, MouseEventKind::Up(MouseButton::Left)), &view),
            PreferencesEventHandling::Consumed
        );

        if let Some(PreferencesWidget::Choice {
            dropdown_regions, ..
        }) = session.widgets.get_mut(&PreferencesControlId::Language)
        {
            dropdown_regions.clear();
        }
        session.clicks.register(
            area,
            PreferencesHit::Dropdown {
                id: PreferencesControlId::Editor,
                option: 0,
            },
        );
        assert_eq!(
            session.handle_event(mouse(area, MouseEventKind::Down(MouseButton::Left)), &view,),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(mouse(area, MouseEventKind::Up(MouseButton::Left)), &view,),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.activate_hit(
                PreferencesHit::Dropdown {
                    id: PreferencesControlId::Language,
                    option: 0,
                },
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
    }

    /// A wrapped Preferences sentence keeps its later rows when its top is above the viewport.
    #[test]
    fn a_top_clipped_preferences_copy_shows_its_surviving_later_rows() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let _ = draw(&mut session, &view, 24, 4, Locale::En);
        let copy = layout_items(&view, Locale::En, session.viewport.width)
            .into_iter()
            .find(|item| matches!(&item.item, RenderItem::Copy(value) if value.contains("$VISUAL")))
            .expect("the editor fallback copy is present");
        assert!(copy.height >= 3, "the copy must wrap across later rows");
        session
            .scroll
            .set_scroll_offset(copy.start.saturating_add(1));

        let terminal = draw(&mut session, &view, 24, 4, Locale::En);
        let rendered = text(terminal.backend().buffer());

        assert!(
            rendered.contains("$EDITOR)"),
            "the surviving final copy row is missing:\n{rendered}"
        );
        assert!(
            !rendered.contains("Empty means: vim"),
            "the clipped first row restarted inside the band:\n{rendered}"
        );
    }

    #[test]
    fn a_top_clipped_focused_preferences_picker_shows_its_placeholder_row() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        session.sync(&view, Locale::En);
        let control = view
            .control(PreferencesControlId::Language)
            .expect("the language picker is present");
        if let Some(PreferencesWidget::Choice { state, .. }) =
            session.widgets.get_mut(&PreferencesControlId::Language)
        {
            state.selected_index = None;
        }
        let mut terminal = Terminal::new(TestBackend::new(24, 2)).unwrap();

        terminal
            .draw(|frame| {
                session.render_control(
                    frame,
                    RowClip::new(3, 1, frame.area()),
                    &control,
                    &view,
                    Locale::En,
                );
            })
            .unwrap();
        let rendered = text(terminal.backend().buffer());

        assert!(
            rendered.contains("Select"),
            "the surviving picker placeholder is missing:\n{rendered}"
        );
    }

    #[test]
    fn test_backend_renders_the_complete_colored_preferences_surface() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let terminal = draw(&mut session, &view, 120, 50, Locale::En);
        let buffer = terminal.backend().buffer();
        let rendered = text(buffer);

        for expected in [
            "Preferences",
            "Interface language",
            "Currently in effect: en",
            "Empty means: vim (from $VISUAL / $EDITOR)",
            "Mini form — opens in place, fully clickable",
            "Quit skit — leave the run's output in the terminal",
            "Agents (prompt runners)",
            "The AI agents that run prompt entries.",
            "Agent Skill",
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
    fn rendered_preference_sections_keep_spacers_and_declared_help_order() {
        let mut session = PreferencesWidgetSession::default();
        let view = complete_view();
        let terminal = draw(&mut session, &view, 120, 120, Locale::En);
        let rows = terminal
            .backend()
            .buffer()
            .content()
            .chunks(120)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>();
        let find_row = |needle: &str| {
            rows.iter()
                .position(|row| row.contains(needle))
                .unwrap_or_else(|| panic!("missing rendered Preferences text {needle:?}"))
        };

        for heading in [
            "Editor",
            "Interactive form",
            "After a run (from this menu)",
            "JavaScript runtime",
            "Shell on Windows",
            "Agents (prompt runners)",
            "Download mirrors (mainland-China acceleration)",
        ] {
            let heading_row = find_row(heading);
            assert!(heading_row > 0);
            assert!(
                rows[heading_row - 1]
                    .trim_matches(|character| matches!(character, ' ' | '│'))
                    .is_empty(),
                "section {heading:?} has no blank spacer before it",
            );
        }

        let interactive_control = find_row("Mini form — opens in place, fully clickable");
        let interactive_help = find_row(
            "Used by terminal runs: `skit run` parameter prompts and the `skit add` review panel.",
        );
        assert!(interactive_control < interactive_help);

        let mirror_heading = find_row("Download mirrors (mainland-China acceleration)");
        let mirror_help =
            find_row("Each ecosystem is its own choice — mirror vendors differ per axis.");
        let mirror_control = find_row("Master switch — \"off\" pauses mirrors");
        assert!(mirror_heading < mirror_help && mirror_help < mirror_control);
    }

    #[test]
    fn input_uses_a_real_cursor_and_emits_complete_unicode_values() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Editor));
        let _ = draw(&mut session, &view, 80, 24, Locale::En);

        for character in ['a', '\u{301}', '🧑'] {
            let handling = session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
                &view,
            );
            assert!(matches!(handling, PreferencesEventHandling::Action(_)));
            if let PreferencesEventHandling::Action(action) = handling {
                view.update(action);
            }
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
    fn an_equal_model_render_preserves_the_mouse_selected_input_caret() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::SetEditor("abcdef".to_owned()));
        view.update(PreferencesAction::Focus(PreferencesControlId::Editor));
        let _ = draw(&mut session, &view, 80, 24, Locale::En);
        let area = session
            .control_area(PreferencesControlId::Editor)
            .expect("visible editor input");
        let point = Rect::new(area.x.saturating_add(3), area.y.saturating_add(1), 1, 1);
        assert_eq!(
            session.handle_event(mouse(point, MouseEventKind::Down(MouseButton::Left)), &view,),
            PreferencesEventHandling::Consumed
        );
        assert!(matches!(
            session.handle_event(mouse(point, MouseEventKind::Up(MouseButton::Left)), &view,),
            PreferencesEventHandling::Action(PreferencesAction::Focus(
                PreferencesControlId::Editor
            ))
        ));

        let _ = draw(&mut session, &view, 80, 24, Locale::En);
        assert_eq!(
            session.handle_event(key(KeyCode::Char('X'), KeyModifiers::NONE), &view),
            PreferencesEventHandling::Action(PreferencesAction::SetEditor("abXcdef".to_owned()))
        );
    }

    #[test]
    fn mouse_buttons_and_keyboard_navigation_share_typed_actions() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        let _ = draw(&mut session, &view, 120, 44, Locale::En);
        let area = session
            .control_area(PreferencesControlId::NewRunner)
            .expect("visible New agent button");
        let handling = session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
            &view,
        );
        assert_eq!(handling, PreferencesEventHandling::Consumed);
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::NewRunner)
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
    fn preference_hits_require_a_mouse_button_press() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let _ = draw(&mut session, &view, 120, 44, Locale::En);
        let area = session
            .control_area(PreferencesControlId::NewRunner)
            .expect("visible New agent button");

        for kind in [
            MouseEventKind::Moved,
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Drag(MouseButton::Left),
        ] {
            assert_eq!(
                session.handle_event(
                    Event::Mouse(MouseEvent {
                        kind,
                        column: area.x,
                        row: area.y,
                        modifiers: KeyModifiers::NONE,
                    }),
                    &view,
                ),
                PreferencesEventHandling::Ignored,
                "a preference hit must ignore {kind:?}"
            );
        }
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::NewRunner)
        );
    }

    #[test]
    fn every_non_dropdown_preference_control_requires_a_matching_primary_release() {
        let mut session = PreferencesWidgetSession::default();
        let view = complete_view();
        let _ = draw(&mut session, &view, 140, 160, Locale::En);

        let mut targets = Vec::new();
        for row in 0..160 {
            for column in 0..140 {
                let Some(hit) = session.clicks.handle_click(column, row).cloned() else {
                    continue;
                };
                let is_non_dropdown = !matches!(hit, PreferencesHit::Dropdown { .. })
                    && !matches!(
                        session.widgets.get(&control_id(&hit)),
                        Some(PreferencesWidget::Choice {
                            presentation: ChoicePresentation::Picker,
                            ..
                        })
                    );
                if is_non_dropdown && !targets.iter().any(|(registered, _, _)| registered == &hit) {
                    targets.push((hit, column, row));
                }
            }
        }

        for (id, widget) in &session.widgets {
            if matches!(
                widget,
                PreferencesWidget::Choice {
                    presentation: ChoicePresentation::Picker,
                    ..
                }
            ) {
                continue;
            }
            assert!(
                targets.iter().any(|(hit, _, _)| control_id(hit) == *id),
                "non-dropdown control {id:?} has no tested pointer target"
            );
        }
        assert!(!targets.is_empty());

        for (target, column, row) in &targets {
            session.click.cancel();
            assert_eq!(
                session.handle_event(
                    mouse(
                        Rect::new(*column, *row, 1, 1),
                        MouseEventKind::Down(MouseButton::Left),
                    ),
                    &view,
                ),
                PreferencesEventHandling::Consumed,
                "Down activated {target:?}"
            );
            let released = session.handle_event(
                mouse(
                    Rect::new(*column, *row, 1, 1),
                    MouseEventKind::Up(MouseButton::Left),
                ),
                &view,
            );
            assert_eq!(
                std::mem::discriminant(&released),
                std::mem::discriminant(&PreferencesEventHandling::Action(PreferencesAction::Save,)),
                "same-target Up did not activate {target:?}"
            );
        }

        let (_, first_column, first_row) = &targets[0];
        let (_, second_column, second_row) = &targets[1];
        for (cancel, label) in [
            (MouseEventKind::Down(MouseButton::Right), "right click"),
            (MouseEventKind::Down(MouseButton::Middle), "middle click"),
        ] {
            session.click.cancel();
            assert_eq!(
                session.handle_event(
                    mouse(
                        Rect::new(*first_column, *first_row, 1, 1),
                        MouseEventKind::Down(MouseButton::Left),
                    ),
                    &view,
                ),
                PreferencesEventHandling::Consumed
            );
            assert_eq!(
                session.handle_event(
                    mouse(Rect::new(*first_column, *first_row, 1, 1), cancel),
                    &view,
                ),
                PreferencesEventHandling::Ignored,
                "{label} was not rejected"
            );
            assert_eq!(
                session.handle_event(
                    mouse(
                        Rect::new(*first_column, *first_row, 1, 1),
                        MouseEventKind::Up(MouseButton::Left),
                    ),
                    &view,
                ),
                PreferencesEventHandling::Ignored,
                "{label} did not cancel the armed control"
            );
        }

        for (release_column, release_row, label) in [
            (*second_column, *second_row, "a different control"),
            (0, 0, "outside the Preferences controls"),
        ] {
            session.click.cancel();
            assert_eq!(
                session.handle_event(
                    mouse(
                        Rect::new(*first_column, *first_row, 1, 1),
                        MouseEventKind::Down(MouseButton::Left),
                    ),
                    &view,
                ),
                PreferencesEventHandling::Consumed
            );
            assert_eq!(
                session.handle_event(
                    mouse(
                        Rect::new(release_column, release_row, 1, 1),
                        MouseEventKind::Up(MouseButton::Left),
                    ),
                    &view,
                ),
                PreferencesEventHandling::Ignored,
                "release over {label} activated the pressed control"
            );
            assert_eq!(
                session.handle_event(
                    mouse(
                        Rect::new(*first_column, *first_row, 1, 1),
                        MouseEventKind::Up(MouseButton::Left),
                    ),
                    &view,
                ),
                PreferencesEventHandling::Ignored,
                "a cancelled control accepted a later release"
            );
        }
    }

    #[test]
    fn a_press_outside_preferences_controls_is_ignored() {
        let mut session = PreferencesWidgetSession::default();
        let view = view();
        let _ = draw(&mut session, &view, 80, 24, Locale::En);

        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: u16::MAX,
                    row: u16::MAX,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Ignored
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

    /// Tabbing to a control below the fold must bring it into view.
    ///
    /// A scroll affordance is what the code drew; the control being on screen is what the user
    /// gets. Asserting the affordance passed on the run form while focus sat off screen, so this
    /// asserts the outcome: the focused control's rectangle lies inside the viewport.
    #[test]
    fn tabbing_to_a_control_below_the_fold_brings_it_into_view() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        let _ = draw(&mut session, &view, 52, 10, Locale::En);
        assert!(
            session.maximum_scroll_offset() > 0,
            "this fixture must overflow for the test to mean anything"
        );

        for _ in 0..12 {
            let handling = session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
                &view,
            );
            if let PreferencesEventHandling::Action(action) = handling {
                view.update(action);
            }
            let _ = draw(&mut session, &view, 52, 10, Locale::En);
            let focused = view.focused();
            let area = session
                .control_areas
                .iter()
                .find_map(|(id, area)| (*id == focused).then_some(*area))
                .unwrap_or_else(|| panic!("the focused control {focused:?} was not rendered"));
            let viewport = session.viewport;
            assert!(
                area.y >= viewport.y
                    && area.y.saturating_add(area.height)
                        <= viewport.y.saturating_add(viewport.height),
                "focus moved to {focused:?} at {area:?}, outside the viewport {viewport:?}"
            );
        }
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
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
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
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
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

    #[test]
    fn complete_preferences_routes_every_input_choice_button_and_shortcut() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = complete_view();
        let _ = draw(&mut session, &view, 120, 120, Locale::En);

        for id in [
            PreferencesControlId::Editor,
            PreferencesControlId::BashPath,
            PreferencesControlId::PypiUrl,
            PreferencesControlId::GithubUrl,
            PreferencesControlId::NpmUrl,
        ] {
            view.update(PreferencesAction::Focus(id));
            let _ = draw(&mut session, &view, 120, 120, Locale::En);
            let handling = session.handle_event(Event::Paste("值".to_owned()), &view);
            assert!(matches!(handling, PreferencesEventHandling::Action(_)));
            if let PreferencesEventHandling::Action(action) = handling {
                view.update(action);
            }
        }
        assert_eq!(view.draft().editor, "值");
        assert_eq!(view.draft().bash_path.as_deref(), Some("值"));

        view.update(PreferencesAction::Focus(PreferencesControlId::Editor));
        let _ = draw(&mut session, &view, 120, 120, Locale::En);
        let handling = session.handle_event(key(KeyCode::Char('x'), KeyModifiers::NONE), &view);
        assert!(matches!(handling, PreferencesEventHandling::Action(_)));
        let _ = draw(&mut session, &view, 120, 120, Locale::En);
        assert_eq!(
            session.handle_event(key(KeyCode::Left, KeyModifiers::NONE), &view),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Up, KeyModifiers::NONE), &view),
            PreferencesEventHandling::Action(PreferencesAction::Focus(
                PreferencesControlId::Language,
            ))
        );

        for (code, modifiers, expected) in [
            (
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
                PreferencesEventHandling::Action(PreferencesAction::Save),
            ),
            (
                KeyCode::Esc,
                KeyModifiers::NONE,
                PreferencesEventHandling::Action(PreferencesAction::Close),
            ),
        ] {
            assert_eq!(session.handle_event(key(code, modifiers), &view), expected);
        }
        // No control answers the two chords. The focused input cuts its value with Ctrl+K.
        assert_eq!(
            session.handle_event(key(KeyCode::Char('o'), KeyModifiers::CONTROL), &view),
            PreferencesEventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('k'), KeyModifiers::CONTROL), &view),
            PreferencesEventHandling::Action(PreferencesAction::SetEditor(String::new()))
        );
        assert_eq!(
            session.handle_event(key(KeyCode::PageDown, KeyModifiers::NONE), &view),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(Event::FocusGained, &view),
            PreferencesEventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                &view,
            ),
            PreferencesEventHandling::Ignored
        );

        for (id, expected) in [
            (
                PreferencesControlId::NewRunner,
                PreferencesAction::NewRunner,
            ),
            (
                PreferencesControlId::InstallAgentSkill,
                PreferencesAction::InstallAgentSkill,
            ),
        ] {
            view.update(PreferencesAction::Focus(id));
            let _ = draw(&mut session, &view, 120, 120, Locale::En);
            assert_eq!(
                session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &view),
                PreferencesEventHandling::Action(expected)
            );
        }
        view.update(PreferencesAction::Focus(
            PreferencesControlId::InteractiveForm,
        ));
        let _ = draw(&mut session, &view, 120, 120, Locale::En);
        for code in [KeyCode::Char('o'), KeyCode::Char('k')] {
            assert_eq!(
                session.handle_event(key(code, KeyModifiers::CONTROL), &view),
                PreferencesEventHandling::Ignored,
                "a focused radio group answers no agent chord"
            );
        }
        assert_eq!(
            session.handle_event(key(KeyCode::Left, KeyModifiers::NONE), &view),
            PreferencesEventHandling::Action(PreferencesAction::SetInteractiveForm(
                InteractiveFormChoice::Tui,
            ))
        );
        assert_eq!(
            session.handle_event(key(KeyCode::F(2), KeyModifiers::NONE), &view),
            PreferencesEventHandling::Ignored
        );

        let _ = draw(&mut session, &view, 44, 8, Locale::En);
        for _ in 0..40 {
            let _ = session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: session.viewport.x,
                    row: session.viewport.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            );
        }
        assert!(session.scroll_offset() > 0);
        let _ = draw(&mut session, &view, 120, 120, Locale::En);
        assert_eq!(session.scroll_offset(), 0);
    }

    #[test]
    fn preferences_shortcuts_buttons_and_inputs_keep_distinct_key_owners() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(
            PreferencesControlId::InteractiveForm,
        ));
        let _ = draw(&mut session, &view, 100, 40, Locale::En);
        assert_eq!(
            session.handle_event(key(KeyCode::Char('s'), KeyModifiers::NONE), &view),
            PreferencesEventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('k'), KeyModifiers::NONE), &view),
            PreferencesEventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('s'), KeyModifiers::CONTROL), &view),
            PreferencesEventHandling::Action(PreferencesAction::Save)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('k'), KeyModifiers::CONTROL), &view),
            PreferencesEventHandling::Ignored
        );

        view.update(PreferencesAction::Focus(PreferencesControlId::Editor));
        let _ = draw(&mut session, &view, 100, 40, Locale::En);
        assert_eq!(
            session.handle_event(key(KeyCode::F(2), KeyModifiers::NONE), &view),
            PreferencesEventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('k'), KeyModifiers::CONTROL), &view),
            PreferencesEventHandling::Consumed,
            "an input owns Ctrl+K as ordinary input",
        );

        view.update(PreferencesAction::Focus(PreferencesControlId::NewRunner));
        let _ = draw(&mut session, &view, 100, 40, Locale::En);
        assert_eq!(
            session.handle_event(key(KeyCode::Char('z'), KeyModifiers::NONE), &view),
            PreferencesEventHandling::Ignored
        );
        for code in [KeyCode::Enter, KeyCode::Char(' ')] {
            assert_eq!(
                session.handle_event(key(code, KeyModifiers::NONE), &view),
                PreferencesEventHandling::Action(PreferencesAction::NewRunner)
            );
        }
    }

    #[test]
    fn select_dropdown_and_mapping_matrix_keep_real_geometry_and_typed_values() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = complete_view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Editor));
        let _ = draw(&mut session, &view, 90, 120, Locale::ZhCn);
        let language = session
            .control_area(PreferencesControlId::Language)
            .expect("the language selector must be visible");
        assert_eq!(
            session.handle_event(mouse(language, MouseEventKind::Moved), &view),
            PreferencesEventHandling::Ignored
        );
        assert_eq!(
            session.handle_event(
                mouse(language, MouseEventKind::Down(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                mouse(language, MouseEventKind::Up(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::Focus(
                PreferencesControlId::Language,
            ))
        );
        view.update(PreferencesAction::Focus(PreferencesControlId::Language));
        let _ = draw(&mut session, &view, 90, 120, Locale::ZhCn);
        assert_eq!(
            session.handle_event(
                mouse(language, MouseEventKind::Down(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                mouse(language, MouseEventKind::Up(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        let _ = draw(&mut session, &view, 90, 120, Locale::ZhCn);
        assert!(language_picker_snapshot(&session).0.is_empty());
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &view),
            PreferencesEventHandling::Consumed
        );
        let _ = draw(&mut session, &view, 90, 120, Locale::ZhCn);
        let dropdown_area = |id| match session.widgets.get(&id) {
            Some(PreferencesWidget::Choice {
                dropdown_regions, ..
            }) => dropdown_regions
                .get(1)
                .map_or(Rect::default(), |region| region.area),
            _ => Rect::default(),
        };
        let dropdown = dropdown_area(PreferencesControlId::Language);
        assert!(!dropdown.is_empty());
        assert!(dropdown_area(PreferencesControlId::Editor).is_empty());
        assert_eq!(
            session.handle_event(
                mouse(dropdown, MouseEventKind::Down(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert!(matches!(
            session.handle_event(
                mouse(dropdown, MouseEventKind::Up(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::SetLanguage(_))
        ));

        view.update(PreferencesAction::SetLanguage("zh-TW".to_owned()));
        let _ = draw(&mut session, &view, 90, 120, Locale::ZhTw);
        view.update(PreferencesAction::Focus(PreferencesControlId::Language));
        let _ = draw(&mut session, &view, 90, 120, Locale::En);
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &view),
            PreferencesEventHandling::Consumed
        );
        for code in [
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Char(' '),
            KeyCode::Esc,
        ] {
            assert!(matches!(
                session.handle_event(key(code, KeyModifiers::NONE), &view),
                PreferencesEventHandling::Consumed | PreferencesEventHandling::Action(_)
            ));
        }
        assert_eq!(
            session.handle_event(Event::Paste("ignored".to_owned()), &view),
            PreferencesEventHandling::Ignored
        );

        for value in ["en", "zh-CN"] {
            assert!(matches!(
                choice_action(PreferencesControlId::Language, value),
                PreferencesEventHandling::Action(PreferencesAction::SetLanguage(_))
            ));
        }
        for value in ["plain", "tui"] {
            assert!(matches!(
                choice_action(PreferencesControlId::InteractiveForm, value),
                PreferencesEventHandling::Action(PreferencesAction::SetInteractiveForm(_))
            ));
        }
        for value in ["stay", "exit"] {
            assert!(matches!(
                choice_action(PreferencesControlId::AfterRun, value),
                PreferencesEventHandling::Action(PreferencesAction::SetAfterRun(_))
            ));
        }
        for value in ["deno", "bun", "node", "auto"] {
            assert!(matches!(
                choice_action(PreferencesControlId::Javascript, value),
                PreferencesEventHandling::Action(PreferencesAction::SetJavascript(_))
            ));
        }
        for (id, field) in [
            (
                PreferencesControlId::PypiChoice,
                PreferencesField::PypiMirror,
            ),
            (
                PreferencesControlId::GithubChoice,
                PreferencesField::GithubMirror,
            ),
            (PreferencesControlId::NpmChoice, PreferencesField::NpmMirror),
        ] {
            for value in ["custom", "off", "preset"] {
                assert!(matches!(
                    choice_action(id, value),
                    PreferencesEventHandling::Action(PreferencesAction::ChooseMirror {
                        field: actual,
                        ..
                    }) if actual == field
                ));
            }
        }
        for invalid in [
            PreferencesControlId::Editor,
            PreferencesControlId::BashPath,
            PreferencesControlId::NewRunner,
            PreferencesControlId::InstallAgentSkill,
            PreferencesControlId::PypiUrl,
            PreferencesControlId::GithubUrl,
            PreferencesControlId::NpmUrl,
        ] {
            assert_eq!(
                choice_action(invalid, "off"),
                PreferencesEventHandling::Ignored
            );
        }
        for invalid in [
            PreferencesControlId::Language,
            PreferencesControlId::InteractiveForm,
            PreferencesControlId::AfterRun,
            PreferencesControlId::Javascript,
            PreferencesControlId::MirrorMaster,
            PreferencesControlId::PypiChoice,
            PreferencesControlId::GithubChoice,
            PreferencesControlId::NpmChoice,
        ] {
            assert_eq!(
                input_action(invalid, String::new()),
                PreferencesEventHandling::Ignored
            );
            assert_eq!(button_action(invalid), PreferencesEventHandling::Ignored);
        }
        for invalid_input in [
            PreferencesControlId::NewRunner,
            PreferencesControlId::InstallAgentSkill,
        ] {
            assert_eq!(
                input_action(invalid_input, String::new()),
                PreferencesEventHandling::Ignored
            );
        }
        assert_eq!(
            button_action(PreferencesControlId::NewRunner),
            PreferencesEventHandling::Action(PreferencesAction::NewRunner)
        );
        assert_eq!(
            button_action(PreferencesControlId::InstallAgentSkill),
            PreferencesEventHandling::Action(PreferencesAction::InstallAgentSkill)
        );
        assert!(matches!(
            choice_action(PreferencesControlId::MirrorMaster, "on"),
            PreferencesEventHandling::Action(PreferencesAction::SetMirrorMaster(true))
        ));

        assert_eq!(
            session.activate_hit(
                PreferencesHit::Radio {
                    id: PreferencesControlId::InteractiveForm,
                    option: 1,
                },
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::SetInteractiveForm(
                InteractiveFormChoice::Plain,
            ))
        );
        assert_eq!(
            session.activate_hit(PreferencesHit::Control(PreferencesControlId::Editor), &view),
            PreferencesEventHandling::Action(PreferencesAction::Focus(
                PreferencesControlId::Editor,
            ))
        );
        assert_eq!(
            session.activate_hit(
                PreferencesHit::Control(PreferencesControlId::InteractiveForm),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::Focus(
                PreferencesControlId::InteractiveForm,
            ))
        );
        assert_eq!(
            session.activate_hit(
                PreferencesHit::Radio {
                    id: PreferencesControlId::Editor,
                    option: usize::MAX,
                },
                &view,
            ),
            PreferencesEventHandling::Ignored
        );
        session.widgets.remove(&PreferencesControlId::Editor);
        assert_eq!(
            session.activate_hit(PreferencesHit::Control(PreferencesControlId::Editor), &view,),
            PreferencesEventHandling::Ignored
        );

        let items = layout_items(&view, Locale::En, 54);
        assert!(!items.is_empty());
        let mirror_labels = view
            .controls()
            .into_iter()
            .find_map(|control| match control.kind {
                PreferencesControlKind::Choice(choice)
                    if control.id == PreferencesControlId::PypiChoice =>
                {
                    Some(
                        choice
                            .options
                            .iter()
                            .map(|option| skit_i18n::text(Locale::En, &option.label).into_owned())
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            })
            .expect("the PyPI mirror radio must be present");
        assert!(
            radio_placements(&mirror_labels, 20, false)
                .last()
                .expect("the PyPI mirror radio has options")
                .row
                > 0
        );
    }

    #[test]
    fn agent_picker_and_validation_error_cover_boundaries_wheel_and_modal_priority() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::PresentAgentSkillTargets(
            (0..10)
                .map(|index| AgentTarget {
                    name: format!("agent-{index}"),
                    scope: if index % 2 == 0 {
                        AgentScope::User
                    } else {
                        AgentScope::Project
                    },
                    base: PathBuf::from(format!("/tmp/agent-{index}")),
                })
                .collect(),
        ));
        let _ = draw(&mut session, &view, 90, 18, Locale::ZhTw);
        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
        ] {
            assert!(matches!(
                session.handle_event(key(code, KeyModifiers::NONE), &view),
                PreferencesEventHandling::Action(PreferencesAction::SelectAgentSkillTarget(_))
            ));
        }
        assert_eq!(
            session.handle_event(key(KeyCode::F(2), KeyModifiers::NONE), &view),
            PreferencesEventHandling::Consumed
        );
        let wheel_area = session.agent_target_area(0).expect("visible agent target");
        for kind in [MouseEventKind::ScrollUp, MouseEventKind::ScrollDown] {
            assert!(matches!(
                session.handle_event(
                    Event::Mouse(MouseEvent {
                        kind,
                        column: wheel_area.x,
                        row: wheel_area.y,
                        modifiers: KeyModifiers::NONE,
                    }),
                    &view,
                ),
                PreferencesEventHandling::Action(PreferencesAction::SelectAgentSkillTarget(_))
            ));
        }
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        for event in [
            Event::FocusGained,
            Event::Paste("ignored".to_owned()),
            Event::Resize(30, 8),
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 4,
                row: 4,
                modifiers: KeyModifiers::NONE,
            }),
        ] {
            assert_eq!(
                session.handle_event(event, &view),
                PreferencesEventHandling::Consumed
            );
        }

        let mut invalid = complete_view();
        invalid.update(PreferencesAction::SetMirrorUrl {
            field: PreferencesField::GithubMirror,
            value: "http://not-https".to_owned(),
        });
        let _ = invalid.update(PreferencesAction::Save);
        let terminal = draw(&mut session, &invalid, 72, 20, Locale::ZhCn);
        assert!(!text(terminal.backend().buffer()).trim().is_empty());
        assert!(invalid.error().is_some());
    }

    #[test]
    fn agent_picker_wheel_is_contained_and_reaches_the_last_clickable_target() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::PresentAgentSkillTargets(
            (0..20)
                .map(|index| AgentTarget {
                    name: format!("agent-{index:02}"),
                    scope: AgentScope::User,
                    base: PathBuf::from(format!("/tmp/agent-{index:02}")),
                })
                .collect(),
        ));
        let _ = draw(&mut session, &view, 46, 8, Locale::En);
        let list = session.agent_list_area;
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Consumed,
            "the modal owns outside wheel input without moving its list",
        );
        for _ in 0..30 {
            let handling = session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: list.x,
                    row: list.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            );
            if let PreferencesEventHandling::Action(action) = handling {
                view.update(action);
            }
        }
        assert_eq!(view.agent_skill_install().unwrap().selected(), Some(19));
        let _ = draw(&mut session, &view, 46, 8, Locale::En);
        let last = session
            .agent_target_area(19)
            .expect("last target is visible");
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: last.x,
                    row: last.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column: last.x,
                    row: last.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::ActivateAgentSkillTarget(19))
        );
    }

    #[test]
    fn agent_picker_drag_and_nonprimary_events_cancel_a_pressed_target() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::PresentAgentSkillTargets(vec![
            AgentTarget {
                name: "codex".to_owned(),
                scope: AgentScope::User,
                base: PathBuf::from("/tmp/codex"),
            },
        ]));
        let _ = draw(&mut session, &view, 60, 10, Locale::En);
        let target = session.agent_target_area(0).expect("visible agent target");

        for cancel_kind in [
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Right),
            MouseEventKind::Down(MouseButton::Middle),
        ] {
            session.cancel_click();
            assert_eq!(
                session.handle_event(
                    mouse(target, MouseEventKind::Down(MouseButton::Left)),
                    &view,
                ),
                PreferencesEventHandling::Consumed
            );
            assert_eq!(
                session.handle_event(mouse(target, cancel_kind), &view),
                PreferencesEventHandling::Consumed
            );
            assert_eq!(
                session.handle_event(mouse(target, MouseEventKind::Up(MouseButton::Left)), &view,),
                PreferencesEventHandling::Consumed,
                "a cancelled press must not activate on a later primary release",
            );
        }
        assert_eq!(
            session.handle_event(
                mouse(target, MouseEventKind::Down(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(Event::FocusLost, &view),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(mouse(target, MouseEventKind::Up(MouseButton::Left)), &view,),
            PreferencesEventHandling::Consumed,
            "focus loss must cancel an armed agent target",
        );
    }

    #[test]
    fn agent_picker_height_and_key_kind_match_the_visible_target_state() {
        let target = AgentTarget {
            name: "codex".to_owned(),
            scope: AgentScope::User,
            base: PathBuf::from("/tmp/codex"),
        };
        let path = target.skills_dir().display().to_string();
        let mut session = PreferencesWidgetSession::default();
        let mut populated = view();
        populated.update(PreferencesAction::PresentAgentSkillTargets(vec![target]));

        let short = draw(&mut session, &populated, 80, 4, Locale::En);
        assert!(!text(short.backend().buffer()).contains(&path));
        let tall = draw(&mut session, &populated, 80, 5, Locale::En);
        assert!(text(tall.backend().buffer()).contains(&path));
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                &populated,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &populated),
            PreferencesEventHandling::Action(PreferencesAction::ConfirmAgentSkillTarget)
        );

        let mut empty = view();
        empty.update(PreferencesAction::PresentAgentSkillTargets(Vec::new()));
        let _ = draw(&mut session, &empty, 80, 5, Locale::En);
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &empty),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Esc,
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                &empty,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Esc, KeyModifiers::NONE), &empty),
            PreferencesEventHandling::Action(PreferencesAction::CloseAgentSkillTargets)
        );
    }

    #[test]
    fn agent_picker_cancels_an_armed_index_when_the_target_identity_changes() {
        let target = |name: &str| AgentTarget {
            name: name.to_owned(),
            scope: AgentScope::User,
            base: PathBuf::from(format!("/tmp/{name}")),
        };
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::PresentAgentSkillTargets(vec![target(
            "alpha",
        )]));
        let _ = draw(&mut session, &view, 60, 10, Locale::En);
        let old = session.agent_target_area(0).expect("alpha target row");
        assert_eq!(
            session.handle_event(mouse(old, MouseEventKind::Down(MouseButton::Left)), &view),
            PreferencesEventHandling::Consumed
        );

        view.update(PreferencesAction::PresentAgentSkillTargets(vec![target(
            "beta",
        )]));
        let _ = draw(&mut session, &view, 60, 10, Locale::En);
        let replacement = session.agent_target_area(0).expect("beta target row");
        assert_eq!(replacement, old);
        assert_eq!(
            session.handle_event(
                mouse(replacement, MouseEventKind::Up(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );

        assert_eq!(
            session.handle_event(
                mouse(replacement, MouseEventKind::Down(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        let _ = draw(&mut session, &view, 60, 10, Locale::En);
        assert_eq!(
            session.handle_event(
                mouse(replacement, MouseEventKind::Up(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::ActivateAgentSkillTarget(0))
        );
    }

    #[test]
    fn preferences_alignment_distinguishes_above_equal_and_below_viewport_edges() {
        let mut session = PreferencesWidgetSession {
            visible_height: 3,
            ..PreferencesWidgetSession::default()
        };
        session.scroll.set_lines(vec![String::new(); 20]);

        for (offset, start, height, expected) in
            [(5, 4, 1, 4), (5, 5, 3, 5), (5, 5, 5, 7), (5, 7, 2, 6)]
        {
            session.scroll.set_scroll_offset(offset);
            session.ensure_visible(start, height);
            assert_eq!(session.scroll_offset(), expected);
        }
    }

    #[test]
    fn preferences_choice_actions_cover_after_run_and_every_javascript_runtime() {
        assert_eq!(
            choice_action(PreferencesControlId::AfterRun, "stay"),
            PreferencesEventHandling::Action(PreferencesAction::SetAfterRun(AfterRunChoice::Stay,))
        );
        assert_eq!(
            choice_action(PreferencesControlId::AfterRun, "exit"),
            PreferencesEventHandling::Action(PreferencesAction::SetAfterRun(AfterRunChoice::Exit,))
        );
        for (value, expected) in [
            ("automatic", JavascriptChoice::Automatic),
            ("deno", JavascriptChoice::Deno),
            ("bun", JavascriptChoice::Bun),
            ("node", JavascriptChoice::Node),
        ] {
            assert_eq!(
                choice_action(PreferencesControlId::Javascript, value),
                PreferencesEventHandling::Action(PreferencesAction::SetJavascript(expected)),
            );
        }
    }

    #[test]
    fn rendered_after_run_and_javascript_options_dispatch_their_exact_typed_actions() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        let families = [
            (
                PreferencesControlId::AfterRun,
                vec![
                    (
                        PreferencesAction::SetAfterRun(AfterRunChoice::Exit),
                        "Quit skit — leave the run's output in the terminal",
                    ),
                    (
                        PreferencesAction::SetAfterRun(AfterRunChoice::Stay),
                        "Return to the Library immediately",
                    ),
                ],
            ),
            (
                PreferencesControlId::Javascript,
                vec![
                    (
                        PreferencesAction::SetJavascript(JavascriptChoice::Automatic),
                        "Automatic — the first of deno / bun / node found",
                    ),
                    (
                        PreferencesAction::SetJavascript(JavascriptChoice::Deno),
                        "deno",
                    ),
                    (
                        PreferencesAction::SetJavascript(JavascriptChoice::Bun),
                        "bun",
                    ),
                    (
                        PreferencesAction::SetJavascript(JavascriptChoice::Node),
                        "node",
                    ),
                ],
            ),
        ];
        for (id, expected) in families {
            view.update(PreferencesAction::Focus(id));
            let terminal = draw(&mut session, &view, 140, 80, Locale::En);
            for (option, (action, label)) in expected.into_iter().enumerate() {
                let point = (0..80)
                    .flat_map(|row| (0..140).map(move |column| (column, row)))
                    .find(|(column, row)| {
                        session.clicks.handle_click(*column, *row)
                            == Some(&PreferencesHit::Radio { id, option })
                    })
                    .expect("rendered radio option has a hit");
                let row_text = (0..140)
                    .map(|column| terminal.backend().buffer()[(column, point.1)].symbol())
                    .collect::<String>();
                assert!(
                    row_text.contains(skit_i18n::text(Locale::En, label).as_ref()),
                    "option {option} is not bound to its visible label: {row_text}",
                );
                let area = Rect::new(point.0, point.1, 1, 1);
                assert_eq!(
                    session
                        .handle_event(mouse(area, MouseEventKind::Down(MouseButton::Left)), &view,),
                    PreferencesEventHandling::Consumed
                );
                assert_eq!(
                    session
                        .handle_event(mouse(area, MouseEventKind::Up(MouseButton::Left)), &view,),
                    PreferencesEventHandling::Action(action)
                );
            }
        }
    }

    /// Report the row, the glyph cell, and the chip background of every option of one radio
    /// control.
    fn radio_option_pixels(
        session: &PreferencesWidgetSession,
        buffer: &Buffer,
        id: PreferencesControlId,
        options: usize,
    ) -> Vec<(u16, String, Color, Color)> {
        let mut origins = vec![None; options];
        for row in buffer.area.y..buffer.area.bottom() {
            for column in buffer.area.x..buffer.area.right() {
                if let Some(PreferencesHit::Radio { id: hit, option }) =
                    session.clicks.handle_click(column, row)
                    && *hit == id
                    && origins[*option].is_none_or(|(first, _)| column < first)
                {
                    origins[*option] = Some((column, row));
                }
            }
        }
        origins
            .into_iter()
            .enumerate()
            .map(|(option, origin)| {
                let (column, row) =
                    origin.unwrap_or_else(|| panic!("option {option} of {id:?} has no hit"));
                let glyph = &buffer[(column, row)];
                (
                    row,
                    glyph.symbol().to_owned(),
                    glyph.fg,
                    buffer[(column.saturating_add(2), row)].bg,
                )
            })
            .collect()
    }

    /// Return every cell that holds the focus marker.
    fn focus_markers(buffer: &Buffer) -> Vec<(u16, u16)> {
        (buffer.area.y..buffer.area.bottom())
            .flat_map(|row| (buffer.area.x..buffer.area.right()).map(move |column| (column, row)))
            .filter(|position| buffer[*position].symbol() == "▶")
            .collect()
    }

    /// Return the registered cells of one radio option, or `None` when the option is off screen.
    fn radio_option_rect(
        session: &PreferencesWidgetSession,
        area: Rect,
        id: PreferencesControlId,
        option: usize,
    ) -> Option<Rect> {
        let mut rect: Option<Rect> = None;
        for row in area.y..area.bottom() {
            for column in area.x..area.right() {
                if session.clicks.handle_click(column, row)
                    == Some(&PreferencesHit::Radio { id, option })
                {
                    rect = Some(rect.map_or(Rect::new(column, row, 1, 1), |current: Rect| {
                        Rect::new(
                            current.x,
                            current.y,
                            column.saturating_add(1).saturating_sub(current.x),
                            1,
                        )
                    }));
                }
            }
        }
        rect
    }

    /// Return the index of the selected option of one radio control.
    fn selected_option(view: &PreferencesView, id: PreferencesControlId) -> usize {
        let controls = view.controls();
        let control = controls
            .iter()
            .find(|control| control.id == id)
            .unwrap_or_else(|| panic!("{id:?} is not in the view"));
        let PreferencesControlKind::Choice(choice) = &control.kind else {
            panic!("{id:?} is not a choice control");
        };
        choice
            .options
            .iter()
            .position(|option| option.value == choice.selected)
            .unwrap_or_else(|| panic!("{id:?} selects no option"))
    }

    #[test]
    #[should_panic(expected = "is not a choice control")]
    fn selected_option_refuses_a_control_that_is_not_a_choice() {
        let _ = selected_option(&complete_view(), PreferencesControlId::Editor);
    }

    /// The Preferences controls that show their focus with a border.
    fn is_bordered(control: &PreferencesControl) -> bool {
        matches!(&control.kind, PreferencesControlKind::Text(_))
            || matches!(
                &control.kind,
                PreferencesControlKind::Choice(choice)
                    if choice.presentation == ChoicePresentation::Picker
            )
    }

    /// Report whether one control area holds an accent cell.
    fn has_accent(buffer: &Buffer, area: Rect) -> bool {
        (area.y..area.bottom())
            .flat_map(|row| (area.x..area.right()).map(move |column| (column, row)))
            .any(|position| buffer[position].fg == ACCENT)
    }

    /// A wheel event over the Preferences form.
    fn wheel(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 4,
            row: 5,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// A scrolled radio group must keep its focus cue on a row the user can see.
    ///
    /// The wheel does not realign the form, so the selected option can leave the screen while the
    /// other options stay. Without a marker on a visible row the next arrow key edits a control
    /// that shows no focus at all.
    #[test]
    fn a_scrolled_focused_radio_group_keeps_its_marker_on_a_visible_row() {
        for (id, width) in [
            (PreferencesControlId::Javascript, 80),
            (PreferencesControlId::MirrorMaster, 60),
        ] {
            let mut session = PreferencesWidgetSession::default();
            let mut view = complete_view();
            view.update(PreferencesAction::Focus(id));
            let selected = selected_option(&view, id);
            let _ = draw(&mut session, &view, width, 14, Locale::En);
            let mut clipped = 0_usize;
            let mut stopped_at_first_row = 0_usize;
            let mut clipped_offset = None;
            {
                let mut assert_marker = |session: &mut PreferencesWidgetSession, step: String| {
                    let terminal = draw(session, &view, width, 14, Locale::En);
                    let buffer = terminal.backend().buffer();
                    let markers = focus_markers(buffer);
                    let Some(area) = session.control_area(id) else {
                        assert!(
                            markers.is_empty(),
                            "{id:?} is off screen at {step} but kept a marker",
                        );
                        clipped = clipped.saturating_add(1);
                        return true;
                    };
                    assert_eq!(markers.len(), 1, "{id:?} has no single marker at {step}");
                    assert_eq!(markers[0].0, area.x, "{id:?} marker left the gutter");
                    let selected_row =
                        radio_option_rect(session, area, id, selected).map(|rect| rect.y);
                    if selected_row.is_none() {
                        stopped_at_first_row = stopped_at_first_row.saturating_add(1);
                    }
                    assert_eq!(
                        markers[0].1,
                        selected_row.unwrap_or(area.y),
                        "{id:?} marker is not on the selected row or the first visible row at {step}",
                    );
                    false
                };
                for offset in 0..=session.maximum_scroll_offset() {
                    session.scroll.set_scroll_offset(offset);
                    if assert_marker(&mut session, format!("offset {offset}")) {
                        clipped_offset = Some(offset);
                    }
                }
                for notch in 1..=6_usize {
                    assert_eq!(
                        session.handle_event(wheel(MouseEventKind::ScrollUp), &view),
                        PreferencesEventHandling::Consumed
                    );
                    assert_marker(&mut session, format!("{notch} wheel notches"));
                }
            }
            assert!(clipped > 0, "the scroll never moved {id:?} off screen");
            assert!(
                stopped_at_first_row > 0,
                "the scroll never clipped the selected option of {id:?} alone",
            );

            let offset = clipped_offset.expect("the sweep never scrolled the control off screen");
            session.scroll.set_scroll_offset(offset);
            let terminal = draw(&mut session, &view, width, 14, Locale::En);
            assert!(
                focus_markers(terminal.backend().buffer()).is_empty(),
                "a control that is off screen kept a marker",
            );
            assert!(session.control_area(id).is_none());

            let action = session.move_focus_action(true);
            view.update(action);
            assert_ne!(view.focused(), id, "the focus ring did not move");
            let terminal = draw(&mut session, &view, width, 14, Locale::En);
            let buffer = terminal.backend().buffer();
            let focused = view.focused();
            let area = session
                .control_area(focused)
                .expect("a focus move must bring the focused control into view");
            let controls = view.controls();
            let control = controls
                .iter()
                .find(|control| control.id == focused)
                .expect("the focused control is in the view");
            if is_bordered(control) {
                assert!(
                    has_accent(buffer, area),
                    "{focused:?} lost its accent border"
                );
            } else {
                assert_eq!(
                    focus_markers(buffer).len(),
                    1,
                    "{focused:?} lost its marker"
                );
            }
        }
    }

    /// The painted label and the click rect must cover the same cells in every locale.
    ///
    /// `ratatui-interact` measures a label in characters. A Chinese label takes two cells for each
    /// character, so the visible text went past the rect that receives the click.
    #[test]
    fn a_localized_door_and_radio_option_click_their_complete_painted_label() {
        for (locale, door_label, option_label) in [
            (
                Locale::ZhTw,
                "New agent…",
                "Quit skit — leave the run's output in the terminal",
            ),
            (
                Locale::ZhCn,
                "New agent…",
                "Quit skit — leave the run's output in the terminal",
            ),
        ] {
            let mut session = PreferencesWidgetSession::default();
            let mut view = view();
            view.update(PreferencesAction::Focus(PreferencesControlId::NewRunner));
            let terminal = draw(&mut session, &view, 120, 44, locale);
            let door = session
                .control_area(PreferencesControlId::NewRunner)
                .expect("visible New agent door");
            let shown = skit_i18n::text(locale, door_label);
            assert_eq!(
                door.width,
                u16::try_from(shown.as_ref().width().saturating_add(2)).unwrap(),
                "the door click rect does not cover its painted label",
            );
            assert_eq!(
                terminal.backend().buffer()[(door.right().saturating_sub(1), door.y)].bg,
                ACCENT,
                "the door paints past its click rect",
            );
            let last = Rect::new(door.right().saturating_sub(1), door.y, 1, 1);
            assert_eq!(
                session.handle_event(mouse(last, MouseEventKind::Down(MouseButton::Left)), &view),
                PreferencesEventHandling::Consumed
            );
            assert_eq!(
                session.handle_event(mouse(last, MouseEventKind::Up(MouseButton::Left)), &view),
                PreferencesEventHandling::Action(PreferencesAction::NewRunner)
            );
            let beyond = Rect::new(door.right(), door.y, 1, 1);
            assert_eq!(
                session.handle_event(
                    mouse(beyond, MouseEventKind::Down(MouseButton::Left)),
                    &view
                ),
                PreferencesEventHandling::Ignored,
                "the cell after the door label is a click target",
            );

            let area = session
                .control_area(PreferencesControlId::AfterRun)
                .expect("visible after-run group");
            let option = radio_option_rect(&session, area, PreferencesControlId::AfterRun, 0)
                .expect("the first after-run option is visible");
            let shown = skit_i18n::text(locale, option_label);
            assert_eq!(
                option.width,
                u16::try_from(shown.as_ref().width().saturating_add(4)).unwrap(),
                "the option click rect does not cover its glyph and painted label",
            );
            assert_eq!(
                terminal.backend().buffer()[(option.right().saturating_sub(1), option.y)].bg,
                SELECT_BG,
                "the option paints past its click rect",
            );
            let last = Rect::new(option.right().saturating_sub(1), option.y, 1, 1);
            assert_eq!(
                session.handle_event(mouse(last, MouseEventKind::Down(MouseButton::Left)), &view),
                PreferencesEventHandling::Consumed
            );
            assert_eq!(
                session.handle_event(mouse(last, MouseEventKind::Up(MouseButton::Left)), &view),
                PreferencesEventHandling::Action(PreferencesAction::SetAfterRun(
                    AfterRunChoice::Exit
                ))
            );
            let beyond = Rect::new(option.right(), option.y, 1, 1);
            assert_eq!(
                session.handle_event(
                    mouse(beyond, MouseEventKind::Down(MouseButton::Left)),
                    &view
                ),
                PreferencesEventHandling::Ignored,
                "the cell after the option label is a click target",
            );
        }
    }

    /// Every radio group must show its value with one filled glyph.
    #[test]
    fn every_radio_control_paints_one_filled_glyph_and_hollow_options() {
        for locale in [Locale::En, Locale::ZhTw] {
            let mut session = PreferencesWidgetSession::default();
            let view = complete_view();
            let terminal = draw(&mut session, &view, 140, 160, locale);
            let buffer = terminal.backend().buffer();
            for control in &view.controls() {
                let PreferencesControlKind::Choice(choice) = &control.kind else {
                    continue;
                };
                if choice.presentation != ChoicePresentation::Radio {
                    continue;
                }
                let pixels =
                    radio_option_pixels(&session, buffer, control.id, choice.options.len());
                let filled = pixels.iter().filter(|(_, glyph, ..)| glyph == "◉").count();
                assert_eq!(filled, 1, "{:?} has {filled} filled glyphs", control.id);
                for (_, glyph, foreground, _) in &pixels {
                    assert!(
                        glyph == "◉" || glyph == "○",
                        "{:?} painted an option without a value glyph",
                        control.id,
                    );
                    assert_eq!(*foreground, ACCENT, "{:?} glyph is not accent", control.id);
                }
            }
        }
    }

    #[test]
    fn javascript_radio_pixels_follow_the_exact_selected_model_option() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::Javascript));

        let automatic = draw(&mut session, &view, 140, 80, Locale::En);
        let javascript = session
            .control_area(PreferencesControlId::Javascript)
            .expect("visible JavaScript group");
        let after_run = session
            .control_area(PreferencesControlId::AfterRun)
            .expect("visible after-run group");
        let stacked = |area: Rect, offset: u16, glyph: &str, background: Color| {
            (
                area.y.saturating_add(offset),
                glyph.to_owned(),
                ACCENT,
                background,
            )
        };
        assert_eq!(
            radio_option_pixels(
                &session,
                automatic.backend().buffer(),
                PreferencesControlId::Javascript,
                4,
            ),
            [
                stacked(javascript, 0, "◉", ACCENT),
                stacked(javascript, 1, "○", Color::Reset),
                stacked(javascript, 2, "○", Color::Reset),
                stacked(javascript, 3, "○", Color::Reset),
            ],
            "the focused group must stack its options and paint the selected one on the accent",
        );
        assert_eq!(
            radio_option_pixels(
                &session,
                automatic.backend().buffer(),
                PreferencesControlId::AfterRun,
                2,
            ),
            [
                stacked(after_run, 0, "◉", SELECT_BG),
                stacked(after_run, 1, "○", Color::Reset),
            ],
            "an unfocused group must keep its selected option on the select background",
        );

        view.update(PreferencesAction::SetJavascript(JavascriptChoice::Deno));
        let deno = draw(&mut session, &view, 140, 80, Locale::En);
        assert_eq!(
            radio_option_pixels(
                &session,
                deno.backend().buffer(),
                PreferencesControlId::Javascript,
                4,
            ),
            [
                stacked(javascript, 0, "○", Color::Reset),
                stacked(javascript, 1, "◉", ACCENT),
                stacked(javascript, 2, "○", Color::Reset),
                stacked(javascript, 3, "○", Color::Reset),
            ],
        );
    }

    #[test]
    fn every_focused_preference_control_shows_exactly_one_focus_cue() {
        for (width, height) in [(140_u16, 160_u16), (80, 14)] {
            for locale in [Locale::En, Locale::ZhTw] {
                let mut session = PreferencesWidgetSession::default();
                let mut view = complete_view();
                let controls = view.controls();
                for control in &controls {
                    view.update(PreferencesAction::Focus(control.id));
                    let terminal = draw(&mut session, &view, width, height, locale);
                    let buffer = terminal.backend().buffer();
                    let markers = focus_markers(buffer);
                    let id = control.id;
                    let area = session
                        .control_area(id)
                        .expect("a focus move must bring the focused control into view");
                    if is_bordered(control) {
                        assert!(markers.is_empty(), "{id:?} painted a marker on a border");
                        assert!(has_accent(buffer, area), "{id:?} lost its accent border");
                    } else {
                        assert_eq!(markers.len(), 1, "{id:?} has no single focus marker");
                        let gutter = if matches!(control.kind, PreferencesControlKind::Button) {
                            area.x.saturating_sub(2)
                        } else {
                            area.x
                        };
                        assert_eq!(markers[0].0, gutter, "{id:?} marker left the gutter");
                        assert!(
                            (area.y..area.bottom()).contains(&markers[0].1),
                            "{id:?} marker left its rows",
                        );
                        assert_eq!(buffer[markers[0]].fg, ACCENT, "{id:?} marker is not accent");
                    }
                    for other in controls.iter().filter(|other| other.id != control.id) {
                        if !is_bordered(other) {
                            continue;
                        }
                        let Some(other_area) = session.control_area(other.id) else {
                            continue;
                        };
                        assert!(
                            !has_accent(buffer, other_area),
                            "{:?} kept an accent border while {id:?} owns the focus",
                            other.id,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_preference_door_paints_its_label_inside_its_click_rect() {
        let mut session = PreferencesWidgetSession::default();
        let mut view = view();
        view.update(PreferencesAction::Focus(PreferencesControlId::NewRunner));
        let terminal = draw(&mut session, &view, 120, 44, Locale::En);
        let area = session
            .control_area(PreferencesControlId::NewRunner)
            .expect("visible New agent door");
        let gutter = area.x.saturating_sub(2);
        assert_eq!(gutter, session.viewport.x, "the door lost its gutter");

        let buffer = terminal.backend().buffer();
        let label = skit_i18n::text(Locale::En, "New agent…").into_owned();
        let painted = (area.x..buffer.area.right())
            .map(|column| buffer[(column, area.y)].symbol())
            .collect::<String>();
        assert!(
            painted.starts_with(&format!(" {label} ")),
            "the door text does not start two cells right of the gutter: {painted}",
        );
        assert_eq!(
            area.width,
            u16::try_from(label.width().saturating_add(2)).unwrap(),
            "the click rect does not cover the painted label",
        );

        let first_character = Rect::new(area.x.saturating_add(1), area.y, 1, 1);
        assert_eq!(
            session.handle_event(
                mouse(first_character, MouseEventKind::Down(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Consumed
        );
        assert_eq!(
            session.handle_event(
                mouse(first_character, MouseEventKind::Up(MouseButton::Left)),
                &view,
            ),
            PreferencesEventHandling::Action(PreferencesAction::NewRunner)
        );

        let marker = Rect::new(gutter, area.y, 1, 1);
        assert_eq!(buffer[(gutter, area.y)].symbol(), "▶");
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            assert_eq!(
                session.handle_event(mouse(marker, kind), &view),
                PreferencesEventHandling::Ignored,
                "the focus marker must not be a click target"
            );
        }
    }

    #[test]
    fn preferences_radio_exact_fit_keeps_the_full_nonzero_origin_row_clickable() {
        let labels = vec!["A".repeat(34), "B".repeat(35)];
        let options = labels
            .iter()
            .enumerate()
            .map(|(index, label)| PreferencesOption {
                value: index.to_string(),
                label: label.clone(),
            })
            .collect::<Vec<_>>();
        let control = PreferencesControl {
            id: PreferencesControlId::PypiChoice,
            label: String::new(),
            help: String::new(),
            kind: PreferencesControlKind::Choice(PreferencesChoiceControl {
                options: options.clone(),
                selected: "0".to_owned(),
                presentation: ChoicePresentation::Radio,
            }),
        };
        assert_eq!(
            radio_placements(&labels, 78, false)
                .last()
                .expect("the exact-fit band has options")
                .row,
            0,
            "the exact fit must place both options on one row",
        );
        assert_eq!(
            control_height(&control, Locale::En, 80, false),
            1,
            "the gutter must leave the exact-fit row whole",
        );
        assert_eq!(
            control_height(&control, Locale::En, 79, false),
            2,
            "one cell less than the exact fit must wrap the last option",
        );

        let mut terminal = Terminal::new(TestBackend::new(110, 4)).unwrap();
        let mut clicks = ClickRegionRegistry::new();
        let buttons = vec![ButtonState::default(), ButtonState::default()];
        terminal
            .draw(|frame| {
                render_radio_band(
                    frame,
                    RowClip::new(1, 0, Rect::new(20, 1, 80, 1)),
                    &control,
                    "",
                    RadioBand {
                        labels: &labels,
                        buttons: &buttons,
                        focused: false,
                        selected: Some(0),
                        active: 0,
                    },
                    &mut clicks,
                );
            })
            .unwrap();
        let row = (20..100)
            .map(|column| terminal.backend().buffer()[(column, 1)].symbol())
            .collect::<String>();
        assert!(
            row.contains(&"B".repeat(35)),
            "exact-fit option missing: {row}"
        );
        assert_eq!(
            clicks.handle_click(99, 1),
            Some(&PreferencesHit::Radio {
                id: PreferencesControlId::PypiChoice,
                option: 1,
            })
        );
        assert_eq!(
            clicks.handle_click(20, 1),
            None,
            "the focus gutter must not select the first option",
        );
        assert_eq!(
            clicks.handle_click(22, 1),
            Some(&PreferencesHit::Radio {
                id: PreferencesControlId::PypiChoice,
                option: 0,
            }),
            "the value glyph must select its own option",
        );

        let mut wrapped = ClickRegionRegistry::new();
        terminal
            .draw(|frame| {
                render_radio_band(
                    frame,
                    RowClip::new(2, 0, Rect::new(20, 1, 79, 2)),
                    &control,
                    "",
                    RadioBand {
                        labels: &labels,
                        buttons: &buttons,
                        focused: false,
                        selected: Some(0),
                        active: 0,
                    },
                    &mut wrapped,
                );
            })
            .unwrap();
        assert_eq!(
            wrapped.handle_click(22, 2),
            Some(&PreferencesHit::Radio {
                id: PreferencesControlId::PypiChoice,
                option: 1,
            }),
            "one cell less than the exact fit must paint the last option on the next row",
        );
    }

    #[test]
    fn ephemeral_widget_shapes_resync_and_reject_stale_control_pairings() {
        let text_control = PreferencesControl {
            id: PreferencesControlId::Editor,
            label: "Editor".to_owned(),
            help: "Help".to_owned(),
            kind: PreferencesControlKind::Text(PreferencesTextControl {
                value: "one".to_owned(),
                kind: FormInputKind::Text,
                placeholder: "placeholder".to_owned(),
            }),
        };
        let missing_choice = PreferencesControl {
            id: PreferencesControlId::Language,
            label: "Language".to_owned(),
            help: String::new(),
            kind: PreferencesControlKind::Choice(PreferencesChoiceControl {
                options: vec![PreferencesOption {
                    value: "en".to_owned(),
                    label: "en".to_owned(),
                }],
                selected: "missing".to_owned(),
                presentation: ChoicePresentation::Picker,
            }),
        };
        let radio = PreferencesControl {
            id: PreferencesControlId::MirrorMaster,
            label: "Mirror".to_owned(),
            help: String::new(),
            kind: PreferencesControlKind::Choice(PreferencesChoiceControl {
                options: vec![
                    PreferencesOption {
                        value: "on".to_owned(),
                        label: "on".to_owned(),
                    },
                    PreferencesOption {
                        value: "off".to_owned(),
                        label: "off".to_owned(),
                    },
                ],
                selected: "on".to_owned(),
                presentation: ChoicePresentation::Radio,
            }),
        };
        let button = PreferencesControl {
            id: PreferencesControlId::NewRunner,
            label: "New agent…".to_owned(),
            help: String::new(),
            kind: PreferencesControlKind::Button,
        };

        let mut input = widget(&text_control, Locale::En);
        let mut changed_text = text_control.clone();
        if let PreferencesControlKind::Text(model) = &mut changed_text.kind {
            model.value = "two".to_owned();
        }
        sync_widget(&mut input, &changed_text, Locale::ZhCn);
        assert!(matches!(input, PreferencesWidget::Input(ref state) if state.value() == "two"));

        let mut picker = widget(&missing_choice, Locale::En);
        assert!(matches!(
            picker,
            PreferencesWidget::Choice { ref state, .. } if state.selected_index.is_none()
        ));
        let mut selected = missing_choice.clone();
        if let PreferencesControlKind::Choice(choice) = &mut selected.kind {
            choice.selected = "en".to_owned();
        }
        sync_widget(&mut picker, &selected, Locale::ZhTw);
        assert!(matches!(
            picker,
            PreferencesWidget::Choice { ref state, .. } if state.selected_index == Some(0)
        ));

        let mut radio_widget = widget(&radio, Locale::En);
        sync_widget(&mut radio_widget, &radio, Locale::ZhCn);
        set_widget_focus(&mut radio_widget, true);
        set_widget_focus(&mut radio_widget, false);
        let mut button_widget = widget(&button, Locale::En);
        set_widget_focus(&mut button_widget, true);
        set_widget_focus(&mut input, true);

        sync_widget(&mut input, &radio, Locale::En);
        sync_widget(&mut radio_widget, &text_control, Locale::En);
        sync_widget(&mut button_widget, &text_control, Locale::En);

        assert_eq!(control_height(&missing_choice, Locale::En, 20, false), 3);
        assert_eq!(control_height(&radio, Locale::En, 120, false), 2);
        assert!(radio_options_stack(PreferencesControlId::Language, 120));
        assert!(radio_options_stack(PreferencesControlId::MirrorMaster, 20));
        assert!(!radio_options_stack(
            PreferencesControlId::MirrorMaster,
            120
        ));
        assert_eq!(centered(Rect::new(2, 3, 20, 10), 8, 4).width, 8);

        for (id, expected_field) in [
            (PreferencesControlId::Editor, None),
            (PreferencesControlId::BashPath, None),
            (
                PreferencesControlId::PypiUrl,
                Some(PreferencesField::PypiMirror),
            ),
            (
                PreferencesControlId::GithubUrl,
                Some(PreferencesField::GithubMirror),
            ),
            (
                PreferencesControlId::NpmUrl,
                Some(PreferencesField::NpmMirror),
            ),
        ] {
            let action = input_action(id, "value".to_owned());
            assert!(matches!(action, PreferencesEventHandling::Action(_)));
            if let (
                Some(expected),
                PreferencesEventHandling::Action(PreferencesAction::SetMirrorUrl { field, .. }),
            ) = (expected_field, action)
            {
                assert_eq!(field, expected);
            }
        }

        let mut session = PreferencesWidgetSession::default();
        let view = complete_view();
        let _ = draw(&mut session, &view, 120, 120, Locale::En);
        assert_eq!(
            session.handle_open_select_key(
                PreferencesControlId::InteractiveForm,
                &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            ),
            None
        );
        assert_eq!(
            session.handle_paste(PreferencesControlId::Language, "ignored"),
            PreferencesEventHandling::Ignored
        );
        session.widgets.remove(&PreferencesControlId::Editor);
        let editor = view
            .controls()
            .into_iter()
            .find(|control| control.id == PreferencesControlId::Editor)
            .expect("editor control exists");
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|frame| {
                session.render_control(
                    frame,
                    RowClip::new(3, 0, frame.area()),
                    &editor,
                    &view,
                    Locale::En,
                );
            })
            .unwrap();
        assert!(session.control_area(PreferencesControlId::Editor).is_some());
    }
}
