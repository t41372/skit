//! Entry settings and preset-management widgets.
//!
//! The screen is a list of sections, and a section is a heading, any explanatory lines, and its
//! fields. Version 0.4 composes it the same way (`src/skit/tui_settings.py:388-407`), and a section
//! that does not apply is absent rather than empty (`:423-426`, `:440-446`, `:541-542`).
//!
//! Two rules keep this renderer honest.
//!
//! It invents no visibility rule of its own. A field is drawn when the model says a person can
//! reach it, so the working-directory path box appears and disappears with its choice
//! (`src/skit/tui_settings.py:483-491`) without this file naming that field at all.
//!
//! It caches no model state. A selected option, a toggle, and a read-only line are read from the
//! field again on every render, so a list that changed while the screen was open can never leave a
//! stale mark behind. Only a text cursor lives here, because only a text cursor is the terminal's
//! own.

use std::{collections::BTreeMap, fmt::Display};

use ratatui_core::{
    layout::Rect,
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui_interact::components::{
    CheckBox, CheckBoxState, ScrollableContentState, handle_scrollable_content_key,
    handle_scrollable_content_mouse,
};
use ratatui_textarea::TextArea as RichTextArea;
use ratatui_widgets::{
    paragraph::{Paragraph, Wrap},
    scrollbar::{Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use skit_form::field::{ChoiceOption, Field, FieldKind, FieldValue, ReadOnlyReason, TypedValue};
use skit_i18n::{Locale, format_text, text};
use skit_ui::{SettingsAction, SettingsItem, SettingsNote, SettingsSectionId, SettingsView};
use tui_input::{Input as LineInput, InputRequest, backend::crossterm::EventHandler as _};

use crate::{
    session::{
        TextAreaEventHandling, checkbox_style, edit_textarea, new_textarea, render_line_input,
        render_textarea, textarea_text,
    },
    theme::{ACCENT, BOX_INDIGO, SELECT_BG, SELECT_FG, padded_panel},
};

/// Typed focus and mouse identity. User-visible English never selects behavior.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SettingsControlId {
    /// One editable control, named by the model's own stable field key.
    Field(String),
    /// One option of a closed option set.
    ///
    /// The option carries its value, not its position. Version 0.4 keys its runner picker by value
    /// for the same reason: a list that changed while the screen was open must never shift what an
    /// index means (`src/skit/tui_settings.py:552-556`).
    Option {
        /// Stable field key that owns the option set.
        field: String,
        /// Stored value of the option.
        value: String,
    },
    /// The affordance that defines a new prompt runner without leaving the screen.
    NewRunner,
}

/// One clickable screen region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsHitRegion {
    /// Terminal rectangle.
    pub area: Rect,
    /// Semantic target.
    pub target: SettingsControlId,
}

/// Terminal-only result. The host dispatches [`SettingsAction`] through the reducer.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsScreenEvent {
    /// Frontend-neutral reducer action.
    Action(SettingsAction),
    /// Ephemeral cursor or scroll state changed and nothing else.
    Changed,
}

/// Responsive settings geometry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SettingsScreenGeometry {
    /// Scrollable body viewport.
    pub body: Rect,
    /// First visible virtual row.
    pub first_visible: usize,
    /// Keyboard-equivalent mouse regions.
    pub hits: Vec<SettingsHitRegion>,
}

/// Which terminal control one field kind needs.
#[derive(Clone, Debug, Eq, PartialEq)]
enum ControlShape {
    /// One line of text with a cursor.
    Line {
        /// Whether the terminal must not show the characters.
        secret: bool,
    },
    /// Text that keeps its line breaks.
    Body,
    /// An on or off glyph.
    Toggle,
    /// A closed option set, drawn as one row for each option.
    Options {
        /// Option values in display order.
        values: Vec<String>,
        /// Whether more than one option can be on at once.
        multiple: bool,
    },
    /// Text a person reads but cannot change.
    Static,
}

/// One laid-out screen element and the virtual rows it occupies.
#[derive(Clone, Debug)]
struct Positioned {
    start: usize,
    height: usize,
    item: Item,
}

#[derive(Clone, Debug)]
enum Item {
    /// One blank row between sections.
    Spacer,
    /// One section heading and the key a deep link scrolls to it by.
    Heading {
        /// Shown text.
        text: String,
        /// Stable scroll key of the section.
        anchor: String,
    },
    /// One explanatory line.
    Copy(String),
    /// One field, named by its key. The renderer resolves it against the view again.
    Control {
        /// Stable field key.
        key: String,
        /// Shown label, empty when the section heading already says it.
        label: String,
    },
    /// The new-runner affordance.
    NewRunner(String),
}

/// Everything one control needs to draw itself.
///
/// The field is resolved from the view again in the render pass, so nothing here is a copy of a
/// value a save would keep.
#[derive(Clone, Copy, Debug)]
struct ControlDraw<'a> {
    /// The field this control edits.
    field: &'a Field,
    /// Shown label, empty when the section heading already says it.
    label: &'a str,
    /// Whether this control owns the keyboard.
    focused: bool,
    /// Presentation locale.
    locale: Locale,
}

/// Ephemeral terminal state for the settings screen.
///
/// Only a text cursor lives here. Every other mark a control shows is read from the model again on
/// each render, so nothing in this session can disagree with what a save would keep.
#[derive(Debug, Default)]
pub struct SettingsScreenSession {
    signature: Option<Vec<(String, ControlShape)>>,
    inputs: BTreeMap<String, LineInput>,
    bodies: BTreeMap<String, Box<RichTextArea<'static>>>,
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
    spans: BTreeMap<String, (usize, usize)>,
    rendered: BTreeMap<String, Rect>,
    undo_group: usize,
    redo_group: usize,
    aligned: Option<Alignment>,
}

/// What the viewport was last aligned for.
///
/// Every member is read from live data on each render and compared to the last render's. This is a
/// change detector, never a resolver: nothing is ever looked up through it, so a stale member can
/// only cost one extra alignment, never a wrong one.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Alignment {
    /// Key of the control that owned the keyboard.
    focused: String,
    /// Rows the viewport could show.
    visible_height: usize,
    /// Rows the screen laid out.
    total: usize,
}

impl SettingsScreenSession {
    /// Return where one field sat in the last render, at the full size it asked for.
    ///
    /// A field the viewport did not reach has no rectangle, because nothing was drawn for it. A
    /// field the viewport reached only in part keeps its whole rectangle, so a caller can tell
    /// "on screen" from "on screen and complete". A clipped rectangle could not: clipping makes
    /// every drawn control fit the viewport by construction.
    #[must_use]
    pub fn field_area(&self, key: &str) -> Option<Rect> {
        self.rendered.get(key).copied()
    }

    /// Rebuild the text cursors when the control shape changes, and refresh their text otherwise.
    pub(crate) fn sync(&mut self, view: &SettingsView) {
        let signature = fields(view)
            .map(|field| (field.key.clone(), shape(field)))
            .collect::<Vec<_>>();
        if self.signature.as_ref() != Some(&signature) {
            self.signature = Some(signature);
            self.inputs.clear();
            self.bodies.clear();
            for field in fields(view) {
                let value = field.value().as_text();
                match shape(field) {
                    ControlShape::Body => {
                        self.bodies
                            .insert(field.key.clone(), Box::new(new_textarea(&value)));
                    }
                    ControlShape::Line { .. } => {
                        self.inputs.insert(field.key.clone(), LineInput::new(value));
                    }
                    ControlShape::Toggle | ControlShape::Options { .. } | ControlShape::Static => {}
                }
            }
            return;
        }
        for field in fields(view) {
            let value = field.value().as_text();
            if let Some(body) = self.bodies.get_mut(&field.key)
                && textarea_text(body) != value
            {
                **body = new_textarea(&value);
            }
            if let Some(input) = self.inputs.get_mut(&field.key)
                && input.value() != value
            {
                *input = LineInput::new(value);
            }
        }
    }

    /// Put the focused control inside the viewport when something moved it out.
    ///
    /// There is no deferred flag. A flag records that focus moved and asks a later call to act on
    /// it, so every call site that moves focus has to remember to set it. Here the focused key is
    /// read from the model each render, so a move made anywhere, by any input, is followed with no
    /// cooperation at all.
    ///
    /// It aligns when the keyboard went somewhere new, and when the rows moved underneath a focus
    /// that did not — a resize, or a control that appeared. It does not align when neither
    /// happened, which is exactly the render after a wheel scroll: the reader put the viewport
    /// there, and it stays until the keyboard gives a new intent.
    ///
    /// The comparison is deliberately not "did the offset change". That answer is true on the
    /// render right after a scroll and false on the one after that, so the viewport would spring
    /// back one frame later.
    fn follow_focus(&mut self, focused: &str, total: usize) {
        let maximum = total.saturating_sub(self.visible_height);
        if self.scroll.scroll_offset() > maximum {
            self.scroll.set_scroll_offset(maximum);
        }
        let current = Alignment {
            focused: focused.to_owned(),
            visible_height: self.visible_height,
            total,
        };
        let settled = self.aligned.as_ref() == Some(&current);
        self.aligned = Some(current);
        if settled {
            return;
        }
        // A screen with nothing to edit has no focused row, and it still draws.
        let Some((start, height)) = self.spans.get(focused).copied() else {
            return;
        };
        let offset = self.scroll.scroll_offset();
        let end = start.saturating_add(height);
        if start < offset {
            self.scroll.set_scroll_offset(start);
        } else if end > offset.saturating_add(self.visible_height) {
            // Show as much of the target as fits, and never past its first row. A target taller
            // than the viewport would otherwise arrive showing only its tail, which for a section
            // means the heading and the opening sentence are the parts that scroll away.
            self.scroll
                .set_scroll_offset(end.saturating_sub(self.visible_height).min(start));
        }
    }

    /// Dispatch one terminal event through the focused settings control.
    ///
    /// `None` means no control claimed the event, so the caller maps it through the shared command
    /// registry. Every chord the footer advertises reaches the registry that way: a control never
    /// sees a `Ctrl` chord except the text undo pair, so `Ctrl+S` cannot be eaten by a text box.
    #[must_use]
    pub fn handle_event(
        &mut self,
        event: Event,
        view: &SettingsView,
        geometry: &SettingsScreenGeometry,
    ) -> Option<SettingsScreenEvent> {
        self.sync(view);
        if let Event::Mouse(mouse) = &event {
            return self.handle_mouse(mouse, view, geometry);
        }
        if let Event::Paste(value) = &event {
            return self.handle_paste(view, value);
        }
        let Event::Key(key) = event else {
            return None;
        };
        if key.kind == KeyEventKind::Release {
            return None;
        }
        // The nav pair wins over every control, so a text box can never strand the keyboard.
        match key.code {
            KeyCode::Tab => return Some(SettingsScreenEvent::Action(SettingsAction::FocusNext)),
            KeyCode::BackTab => {
                return Some(SettingsScreenEvent::Action(SettingsAction::FocusPrevious));
            }
            _ => {}
        }
        let undo_pair = matches!(key.code, KeyCode::Char('z' | 'y'));
        if key.modifiers.contains(KeyModifiers::CONTROL) && !undo_pair {
            return None;
        }
        let focused = view.focused();
        let field = view.field(focused)?;
        self.handle_field_key(focused, field, key)
            .or_else(|| self.scroll_key(key))
    }

    fn handle_field_key(
        &mut self,
        focused: &str,
        field: &Field,
        key: KeyEvent,
    ) -> Option<SettingsScreenEvent> {
        match &field.kind {
            FieldKind::Multiline => self.edit_body(focused, key),
            FieldKind::Boolean if matches!(key.code, KeyCode::Char(' ') | KeyCode::Enter) => {
                set_field(
                    focused,
                    FieldValue::boolean(field.value().as_text() != "true"),
                )
            }
            FieldKind::Boolean => nav(key),
            FieldKind::SingleChoice { options } | FieldKind::MultiChoice { options } => {
                choice_key(field, options, key)
            }
            FieldKind::ReadOnly => None,
            FieldKind::Text
            | FieldKind::Secret
            | FieldKind::Number { .. }
            | FieldKind::Path { .. }
            | FieldKind::ArgumentList { .. } => self.edit_line(focused, key),
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: &MouseEvent,
        view: &SettingsView,
        geometry: &SettingsScreenGeometry,
    ) -> Option<SettingsScreenEvent> {
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            return handle_scrollable_content_mouse(
                &mut self.scroll,
                mouse,
                self.viewport,
                self.visible_height,
            )
            .map(|_| SettingsScreenEvent::Changed);
        }
        if !matches!(mouse.kind, MouseEventKind::Down(_)) {
            return None;
        }
        let target = geometry
            .hits
            .iter()
            .find(|hit| hit.area.contains((mouse.column, mouse.row).into()))
            .map(|hit| hit.target.clone())?;
        match target {
            // A click on a toggle both moves the keyboard and flips it, which is the one thing a
            // person means by clicking a checkbox.
            SettingsControlId::Field(key) => {
                let field = view.field(&key)?;
                if matches!(field.kind, FieldKind::Boolean) {
                    return set_field(&key, FieldValue::boolean(field.value().as_text() != "true"));
                }
                Some(SettingsScreenEvent::Action(SettingsAction::Focus { key }))
            }
            SettingsControlId::Option { field, value } => {
                let owner = view.field(&field)?;
                set_field(&field, picked(owner, &value))
            }
            SettingsControlId::NewRunner => {
                Some(SettingsScreenEvent::Action(SettingsAction::NewRunner))
            }
        }
    }

    fn handle_paste(&mut self, view: &SettingsView, value: &str) -> Option<SettingsScreenEvent> {
        let focused = view.focused().to_owned();
        if let Some(body) = self.bodies.get_mut(&focused) {
            body.insert_str(value);
            self.undo_group = 1;
            self.redo_group = 0;
            return set_field(&focused, FieldValue::text(textarea_text(body)));
        }
        let input = self.inputs.get_mut(&focused)?;
        for character in value.chars() {
            let _ = input.handle(InputRequest::InsertChar(character));
        }
        set_field(&focused, FieldValue::text(input.value()))
    }

    fn edit_body(&mut self, key: &str, event: KeyEvent) -> Option<SettingsScreenEvent> {
        let body = self.bodies.get_mut(key)?;
        let before = textarea_text(body);
        match edit_textarea(body, event, &mut self.undo_group, &mut self.redo_group) {
            TextAreaEventHandling::Ignored => return None,
            TextAreaEventHandling::VerticalBoundary => return nav(event),
            TextAreaEventHandling::Consumed => {}
        }
        let after = textarea_text(body);
        Some(if before == after {
            SettingsScreenEvent::Changed
        } else {
            SettingsScreenEvent::Action(SettingsAction::SetField {
                key: key.to_owned(),
                value: FieldValue::text(after),
            })
        })
    }

    fn edit_line(&mut self, key: &str, event: KeyEvent) -> Option<SettingsScreenEvent> {
        let input = self.inputs.get_mut(key)?;
        let before = input.value().to_owned();
        if input.handle_event(&Event::Key(event)).is_none() {
            // A single-line box does not use the vertical arrows, so they move the keyboard.
            return nav(event);
        }
        Some(if before == input.value() {
            SettingsScreenEvent::Changed
        } else {
            SettingsScreenEvent::Action(SettingsAction::SetField {
                key: key.to_owned(),
                value: FieldValue::text(input.value()),
            })
        })
    }

    fn scroll_key(&mut self, key: KeyEvent) -> Option<SettingsScreenEvent> {
        handle_scrollable_content_key(&mut self.scroll, &key, self.visible_height)
            .map(|_| SettingsScreenEvent::Changed)
    }

    /// Return where one virtual span lands on screen at its full height, clipped by nothing.
    fn requested_rect(&self, start: usize, height: usize) -> Rect {
        let offset = self.scroll.scroll_offset();
        Rect::new(
            self.viewport.x,
            self.viewport
                .y
                .saturating_add(u16::try_from(start.saturating_sub(offset)).unwrap_or(u16::MAX)),
            self.viewport.width,
            u16::try_from(height).unwrap_or(u16::MAX),
        )
    }

    /// Return where one virtual span lands on screen, or nothing when it is off the viewport.
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

    fn render_field(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        requested: Rect,
        draw: &ControlDraw<'_>,
        hits: &mut Vec<SettingsHitRegion>,
    ) {
        let ControlDraw {
            field,
            label,
            focused,
            locale,
        } = *draw;
        self.rendered.insert(field.key.clone(), requested);
        match &field.kind {
            FieldKind::Multiline => {
                if let Some(body) = self.bodies.get_mut(&field.key) {
                    render_textarea(frame, area, body, focused, label);
                }
                hits.push(SettingsHitRegion {
                    area,
                    target: SettingsControlId::Field(field.key.clone()),
                });
            }
            FieldKind::Boolean => {
                let mut state = CheckBoxState::new(field.value().as_text() == "true");
                state.set_focused(focused);
                frame.render_widget(CheckBox::new(label, &state).style(checkbox_style()), area);
                hits.push(SettingsHitRegion {
                    area,
                    target: SettingsControlId::Field(field.key.clone()),
                });
            }
            FieldKind::SingleChoice { options } | FieldKind::MultiChoice { options } => {
                render_options(frame, area, draw, options, hits);
            }
            FieldKind::ReadOnly => render_read_only(frame, area, draw),
            FieldKind::Text
            | FieldKind::Secret
            | FieldKind::Number { .. }
            | FieldKind::Path { .. }
            | FieldKind::ArgumentList { .. } => {
                if let Some(input) = self.inputs.get(&field.key) {
                    let secret = matches!(field.kind, FieldKind::Secret);
                    render_line_input(frame, area, input, secret, focused, label);
                    if input.value().is_empty() && !field.help.is_empty() && area.height >= 3 {
                        frame.render_widget(
                            Paragraph::new(text(locale, &field.help))
                                .style(Style::default().fg(Color::DarkGray)),
                            Rect::new(
                                area.x.saturating_add(1),
                                area.y.saturating_add(1),
                                area.width.saturating_sub(2),
                                1,
                            ),
                        );
                    }
                }
                hits.push(SettingsHitRegion {
                    area,
                    target: SettingsControlId::Field(field.key.clone()),
                });
            }
        }
    }
}

/// Move the keyboard when a control does not use the vertical arrows itself.
///
/// Version 0.4 gives every form screen the same arrow twins for Tab, and they fire only where the
/// focused widget lets the key through (`src/skit/tui_footer.py:72-79`).
fn nav(key: KeyEvent) -> Option<SettingsScreenEvent> {
    match key.code {
        KeyCode::Down => Some(SettingsScreenEvent::Action(SettingsAction::FocusNext)),
        KeyCode::Up => Some(SettingsScreenEvent::Action(SettingsAction::FocusPrevious)),
        _ => None,
    }
}

fn set_field(key: &str, value: FieldValue) -> Option<SettingsScreenEvent> {
    Some(SettingsScreenEvent::Action(SettingsAction::SetField {
        key: key.to_owned(),
        value,
    }))
}

/// Return the value one field holds after a person picks one option.
///
/// A closed single choice replaces its value; an open one adds or removes the option. Both read the
/// field's own current value, so a list that changed while the screen was open cannot make an
/// option mean a different row.
fn picked(field: &Field, value: &str) -> FieldValue {
    let FieldKind::MultiChoice { options } = &field.kind else {
        return FieldValue::Explicit(TypedValue::Choice(value.to_owned()));
    };
    let mut selected = options
        .iter()
        .filter(|option| option.value != value && is_selected(field, option))
        .map(|option| option.value.clone())
        .collect::<Vec<_>>();
    if !options
        .iter()
        .any(|option| option.value == value && is_selected(field, option))
        && let Some(position) = options.iter().position(|option| option.value == value)
    {
        // Keep the field's own option order, so the stored list never depends on click order.
        let ahead = options[..position]
            .iter()
            .filter(|option| selected.contains(&option.value))
            .count();
        selected.insert(ahead, value.to_owned());
    }
    FieldValue::Explicit(TypedValue::Choices(selected))
}

/// Move a closed option set with the arrows, or take the highlighted one with Space or Enter.
fn choice_key(
    field: &Field,
    options: &[ChoiceOption],
    key: KeyEvent,
) -> Option<SettingsScreenEvent> {
    if options.is_empty() {
        return None;
    }
    let multiple = matches!(field.kind, FieldKind::MultiChoice { .. });
    let current = options
        .iter()
        .position(|option| is_selected(field, option))
        .unwrap_or_default();
    let next = match key.code {
        // An open set has no single cursor to walk, so the arrows move the keyboard instead.
        KeyCode::Down | KeyCode::Right if !multiple => current
            .saturating_add(1)
            .min(options.len().saturating_sub(1)),
        KeyCode::Up | KeyCode::Left if !multiple => current.saturating_sub(1),
        KeyCode::Char(' ') | KeyCode::Enter => current,
        _ => return nav(key),
    };
    let option = options.get(next)?;
    set_field(&field.key, picked(field, &option.value))
}

/// Draw the entry-settings screen and return its mouse hit map.
#[must_use]
pub fn render_settings(
    frame: &mut Frame,
    area: Rect,
    view: &SettingsView,
    session: &mut SettingsScreenSession,
    locale: Locale,
) -> SettingsScreenGeometry {
    session.sync(view);
    // Version 0.4 titles the panel with the entry name it opened with
    // (`src/skit/tui_settings.py:869-871`).
    let block = padded_panel(
        format_text(locale, "Entry settings · {}", &[&view.title]),
        BOX_INDIGO,
    );
    let body = block.inner(area);
    frame.render_widget(block, area);
    session.viewport = body;
    session.visible_height = usize::from(body.height).max(1);

    let items = layout_items(view, locale, body.width);
    let total = items
        .last()
        .map_or(0, |item| item.start.saturating_add(item.height));
    session.scroll.set_lines(vec![String::new(); total.max(1)]);
    session.spans.clear();
    session.rendered.clear();
    for (index, item) in items.iter().enumerate() {
        match &item.item {
            Item::Control { key, .. } => {
                session.spans.insert(key.clone(), (item.start, item.height));
            }
            // A section is a scroll target too, so a deep link can put one under the eye even when
            // it has nothing to edit (`src/skit/tui_settings.py:876-882`). The span covers the
            // whole section, not its heading alone: landing on a bare heading would leave the
            // sentence it introduces below the fold, and that sentence is the entire answer for an
            // entry with no presets.
            Item::Heading { anchor, .. } => {
                let end = items
                    .iter()
                    .skip(index.saturating_add(1))
                    .find(|later| matches!(later.item, Item::Heading { .. }))
                    .map_or(total, |later| later.start);
                session
                    .spans
                    .insert(anchor.clone(), (item.start, end.saturating_sub(item.start)));
            }
            Item::Spacer | Item::Copy(_) | Item::NewRunner(_) => {}
        }
    }
    let focused = view.focused().to_owned();
    // The model names what the viewport must show: the section a deep link asked for while that
    // anchor is still held, and the focused control once the reader takes over.
    let anchor = view.revealed().map_or(focused.clone(), section_anchor);
    session.follow_focus(&anchor, total);

    let mut hits = Vec::new();
    for item in &items {
        let Some(rect) = session.visible_rect(item.start, item.height) else {
            continue;
        };
        match &item.item {
            Item::Spacer => {}
            Item::Heading { text, .. } => frame.render_widget(
                Paragraph::new(text.as_str())
                    .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
                rect,
            ),
            Item::Copy(value) => frame.render_widget(
                Paragraph::new(value.as_str())
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(Color::DarkGray)),
                rect,
            ),
            Item::Control { key, label } => {
                if let Some(field) = view.field(key) {
                    let requested = session.requested_rect(item.start, item.height);
                    session.render_field(
                        frame,
                        rect,
                        requested,
                        &ControlDraw {
                            field,
                            label,
                            focused: *key == focused,
                            locale,
                        },
                        &mut hits,
                    );
                }
            }
            Item::NewRunner(label) => {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("  Ctrl+N ", Style::default().fg(ACCENT)),
                        Span::styled(label.as_str(), Style::default().fg(Color::White)),
                    ])),
                    rect,
                );
                hits.push(SettingsHitRegion {
                    area: rect,
                    target: SettingsControlId::NewRunner,
                });
            }
        }
    }
    // Version 0.4 hosts the body in a `VerticalScroll`, whose scrollbar is the only thing that says
    // the screen continues (`src/skit/tui_settings.py:388`, and visible in its shipped frame). With
    // the sections below the fold and no mark beside them, a reader has nothing to act on.
    if total > usize::from(body.height) {
        let mut scrollbar = ScrollbarState::new(total.saturating_sub(usize::from(body.height)))
            .position(session.scroll.scroll_offset())
            .viewport_content_length(usize::from(body.height));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight).style(settings_scrollbar_style()),
            area,
            &mut scrollbar,
        );
    }

    SettingsScreenGeometry {
        body,
        first_visible: session.scroll.scroll_offset(),
        hits,
    }
}

/// The settings scroll affordance's colour, shared with the run form's.
fn settings_scrollbar_style() -> Style {
    Style::default().fg(BOX_INDIGO)
}

/// Lay every section out into virtual rows.
///
/// A field is laid out when the model says a person can reach it, plus every read-only row, which
/// is never a stop but is always shown. That is the whole visibility rule: the model decides, and
/// the gate the working directory owns (`src/skit/tui_settings.py:483-491`) is not repeated here.
fn layout_items(view: &SettingsView, locale: Locale, width: u16) -> Vec<Positioned> {
    let stops = view.focusable_keys();
    let mut items = Vec::new();
    let mut start = 0_usize;
    for (index, section) in view.sections.iter().enumerate() {
        if index > 0 {
            push(&mut items, &mut start, Item::Spacer, 1);
        }
        push(
            &mut items,
            &mut start,
            Item::Heading {
                text: text(locale, section.id.title()).into_owned(),
                anchor: section_anchor(section.id),
            },
            1,
        );
        for element in &section.items {
            let field = match element {
                SettingsItem::Note(note) => {
                    let shown = note_text(locale, note);
                    let height = wrapped_height(&shown, width);
                    push(&mut items, &mut start, Item::Copy(shown), height);
                    continue;
                }
                SettingsItem::Field(field) => field.as_ref(),
            };
            if field.kind.editable() && !stops.contains(&field.key.as_str()) {
                continue;
            }
            let label = shown_label(field, section.id, locale);
            let height = control_height(field, &label, locale, width);
            push(
                &mut items,
                &mut start,
                Item::Control {
                    key: field.key.clone(),
                    label,
                },
                height,
            );
            if field.capabilities.new_runner {
                // Version 0.4 puts the door to define a custom agent right beside the picker, and
                // the key hint is the click target (`src/skit/tui_runner.py:513-516`).
                push(
                    &mut items,
                    &mut start,
                    Item::NewRunner(text(locale, "New agent…").into_owned()),
                    1,
                );
            }
        }
    }
    items
}

/// Return the stable scroll key of one section.
///
/// It is derived from the typed section, never from its shown heading: user-visible English must
/// never select behavior, and a translated heading would break the anchor in every other locale.
fn section_anchor(id: SettingsSectionId) -> String {
    format!("section:{id:?}")
}

fn push(items: &mut Vec<Positioned>, start: &mut usize, item: Item, height: usize) {
    items.push(Positioned {
        start: *start,
        height,
        item,
    });
    *start = start.saturating_add(height);
}

/// Return the label one control shows, empty when its section heading already says it.
///
/// Version 0.4 gives the working directory, the runner, and the needs list a section heading and
/// then a bare control (`src/skit/tui_settings.py:447-470`, `:544-558`, `:860-866`). Comparing the
/// catalog keys, not the translated text, keeps that true in every locale.
fn shown_label(field: &Field, section: SettingsSectionId, locale: Locale) -> String {
    if field.label == section.title() {
        return String::new();
    }
    if field.translate_label {
        return text(locale, &field.label).into_owned();
    }
    field.label.clone()
}

fn control_height(field: &Field, label: &str, locale: Locale, width: u16) -> usize {
    match &field.kind {
        // The same three rows a single-line box takes, because version 0.4 spends exactly that on
        // the one field this kind serves here — its description is an `Input`
        // (`src/skit/tui_settings.py:394-399`). Keeping the kind keeps the line breaks a person can
        // type; spending six rows on it pushed three sections off a recorded terminal.
        FieldKind::Multiline => 3,
        FieldKind::Boolean => 1,
        FieldKind::SingleChoice { options } | FieldKind::MultiChoice { options } => {
            usize::from(!label.is_empty()).saturating_add(options.len().max(1))
        }
        FieldKind::ReadOnly => 1_usize.saturating_add(read_only_note_height(field, locale, width)),
        FieldKind::Text
        | FieldKind::Secret
        | FieldKind::Number { .. }
        | FieldKind::Path { .. }
        | FieldKind::ArgumentList { .. } => 3,
    }
}

fn read_only_note_height(field: &Field, locale: Locale, width: u16) -> usize {
    field.read_only_reason.map_or(0, |reason| {
        wrapped_height(&text(locale, refusal(reason)), width)
    })
}

/// Draw a closed option set as one row for each option.
///
/// Every mark comes from the field itself, so an option list that changed while the screen was open
/// cannot leave a stale tick behind.
fn render_options(
    frame: &mut Frame,
    area: Rect,
    draw: &ControlDraw<'_>,
    options: &[ChoiceOption],
    hits: &mut Vec<SettingsHitRegion>,
) {
    let ControlDraw {
        field,
        label,
        focused,
        locale,
    } = *draw;
    let multiple = matches!(field.kind, FieldKind::MultiChoice { .. });
    let mut y = area.y;
    if !label.is_empty() {
        frame.render_widget(
            Paragraph::new(label).style(Style::default().fg(Color::White)),
            Rect::new(area.x, y, area.width, 1),
        );
        y = y.saturating_add(1);
    }
    for option in options {
        if y >= area.y.saturating_add(area.height) {
            break;
        }
        let selected = is_selected(field, option);
        let glyph = match (multiple, selected) {
            (true, true) => "☑",
            (true, false) => "☐",
            (false, true) => "◉",
            (false, false) => "○",
        };
        let row = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if focused && selected { "▶ " } else { "  " },
                    Style::default().fg(ACCENT),
                ),
                Span::styled(format!("{glyph} "), Style::default().fg(ACCENT)),
                Span::styled(
                    option_text(locale, option),
                    if focused && selected {
                        Style::default()
                            .fg(SELECT_FG)
                            .bg(SELECT_BG)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
            ])),
            row,
        );
        hits.push(SettingsHitRegion {
            area: row,
            target: SettingsControlId::Option {
                field: field.key.clone(),
                value: option.value.clone(),
            },
        });
        y = y.saturating_add(1);
    }
}

/// Draw a row a person reads but cannot change, and say why.
///
/// This is text, not a control that refuses. A greyed input would read as something the user failed
/// to click, so the reason the field itself carries is shown instead.
fn render_read_only(frame: &mut Frame, area: Rect, draw: &ControlDraw<'_>) {
    let ControlDraw {
        field,
        label,
        locale,
        ..
    } = *draw;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if label.is_empty() {
                    String::new()
                } else {
                    format!("{label}: ")
                },
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(field.value().as_text(), Style::default().fg(Color::White)),
        ])),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if let Some(reason) = field.read_only_reason
        && area.height > 1
    {
        frame.render_widget(
            Paragraph::new(text(locale, refusal(reason)))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::DarkGray)),
            Rect::new(
                area.x,
                area.y.saturating_add(1),
                area.width,
                area.height.saturating_sub(1),
            ),
        );
    }
}

/// Return the catalog key that explains one refusal.
const fn refusal(reason: ReadOnlyReason) -> &'static str {
    match reason {
        ReadOnlyReason::SourceDeclares => "The script declares this. Change it in the source.",
        // Version 0.4's own wording for a linked file (`src/skit/tui_settings.py:598-604`).
        ReadOnlyReason::ReferenceMode => {
            "skit doesn't write to this file — maintain the [tool.skit] definitions in the source directly."
        }
        ReadOnlyReason::FixedAtAddTime => {
            "Set when the entry was added. A different command changes it."
        }
        ReadOnlyReason::Derived => "This value follows another field.",
    }
}

/// Report whether one option is on, reading the field's own value again.
fn is_selected(field: &Field, option: &ChoiceOption) -> bool {
    match field.value().explicit() {
        Some(TypedValue::Choices(values)) => values.contains(&option.value),
        Some(value) => value.as_text() == option.value,
        None => false,
    }
}

/// Return the text one option shows.
///
/// An option whose label is its own value is user data, so it is shown exactly as stored — a runner
/// named after a catalog phrase must not come back translated. An option with separate wording is
/// application copy, and it carries its inserted value in `detail` when it has one
/// (`src/skit/tui_settings.py:554-557`).
fn option_text(locale: Locale, option: &ChoiceOption) -> String {
    if option.label == option.value {
        return option.value.clone();
    }
    if option.detail.is_empty() {
        return text(locale, &option.label).into_owned();
    }
    format_text(locale, &option.label, &[&option.detail])
}

fn note_text(locale: Locale, note: &SettingsNote) -> String {
    if note.arguments.is_empty() {
        return text(locale, &note.text).into_owned();
    }
    let arguments = note
        .arguments
        .iter()
        .map(|argument| argument as &dyn Display)
        .collect::<Vec<_>>();
    format_text(locale, &note.text, &arguments)
}

fn wrapped_height(value: &str, width: u16) -> usize {
    Paragraph::new(value)
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
        .max(1)
}

fn shape(field: &Field) -> ControlShape {
    match &field.kind {
        FieldKind::Multiline => ControlShape::Body,
        FieldKind::Secret => ControlShape::Line { secret: true },
        FieldKind::Boolean => ControlShape::Toggle,
        FieldKind::SingleChoice { options } | FieldKind::MultiChoice { options } => {
            ControlShape::Options {
                values: options.iter().map(|option| option.value.clone()).collect(),
                multiple: matches!(field.kind, FieldKind::MultiChoice { .. }),
            }
        }
        FieldKind::ReadOnly => ControlShape::Static,
        FieldKind::Text
        | FieldKind::Number { .. }
        | FieldKind::Path { .. }
        | FieldKind::ArgumentList { .. } => ControlShape::Line { secret: false },
    }
}

fn fields(view: &SettingsView) -> impl Iterator<Item = &Field> {
    view.fields()
}

#[cfg(test)]
mod tests {
    use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::MouseButton;
    use std::collections::BTreeMap;

    use skit_domain::parameters::{ParamDecl, ParameterValue};
    use skit_form::field::{
        ArgumentDialect, FieldCapabilities, FieldOwner, FieldValue, ReadOnlyReason,
    };
    use skit_ui::{
        DESCRIPTION_KEY, DependencyFlavor, MANAGE_KEY, NAME_KEY, NEEDS_KEY, SettingsAction,
        SettingsInputs, SettingsSection, SettingsSectionId, WORKDIR_CUSTOM, WORKDIR_KEY,
    };

    use super::{
        ChoiceOption, Event, Field, FieldKind, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
        Locale, MouseEvent, MouseEventKind, Rect, SettingsControlId, SettingsItem,
        SettingsScreenEvent, SettingsScreenGeometry, SettingsScreenSession, SettingsView,
        TypedValue, choice_key, is_selected, option_text, render_settings,
    };

    /// The recorded demo terminal: 1280x780 at 12.19px per column and 26.33px per row, less 20px of
    /// padding. Every geometry assertion here uses it, so a regression is one a viewer would see.
    const DEMO_WIDTH: u16 = 101;
    const DEMO_HEIGHT: u16 = 28;

    fn settings_inputs() -> SettingsInputs {
        SettingsInputs {
            selector: "brief".to_owned(),
            kind: "prompt".to_owned(),
            name: "Brief".to_owned(),
            description: "Summarize a document".to_owned(),
            source: "/home/ada/brief.md".to_owned(),
            workdir: "invoke".to_owned(),
            runner: "claude".to_owned(),
            supports_modes: true,
            has_original_file: true,
            has_stored_name: true,
            has_analyzer: true,
            pinnable_interpreter: true,
            dependency_flavor: Some(DependencyFlavor::Uv),
            effective_dependencies: vec!["requests>=2,<3".to_owned()],
            effective_requires_python: ">=3.11".to_owned(),
            needs: vec!["ffmpeg".to_owned()],
            configured_runners: vec!["claude".to_owned(), "codex".to_owned()],
            ..SettingsInputs::default()
        }
    }

    fn prompt_view() -> SettingsView {
        SettingsView::from_inputs(&SettingsInputs {
            selector: "brief".to_owned(),
            kind: "prompt".to_owned(),
            name: "Brief".to_owned(),
            description: "Summarize a document".to_owned(),
            source: "/home/ada/brief.md".to_owned(),
            workdir: "invoke".to_owned(),
            runner: "claude".to_owned(),
            supports_modes: true,
            has_original_file: true,
            has_stored_name: true,
            has_analyzer: true,
            pinnable_interpreter: true,
            dependency_flavor: Some(DependencyFlavor::Uv),
            effective_dependencies: vec!["requests>=2,<3".to_owned()],
            effective_requires_python: ">=3.11".to_owned(),
            needs: vec!["ffmpeg".to_owned()],
            configured_runners: vec!["claude".to_owned(), "codex".to_owned()],
            ..SettingsInputs::default()
        })
    }

    fn draw(
        session: &mut SettingsScreenSession,
        view: &SettingsView,
        width: u16,
        height: u16,
    ) -> (Terminal<TestBackend>, SettingsScreenGeometry) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut geometry = SettingsScreenGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_settings(frame, frame.area(), view, session, Locale::En);
            })
            .unwrap();
        (terminal, geometry)
    }

    fn rendered(buffer: &Buffer) -> String {
        buffer
            .content()
            .chunks(usize::from(buffer.area.width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Walking the keyboard from top to bottom must show every section.
    ///
    /// A section the eye never reaches is a capability the user cannot find, which is what version
    /// 0.4 avoids by scrolling the body to the section a deep link names
    /// (`src/skit/tui_settings.py:872-882`).
    #[test]
    fn every_section_comes_into_view_while_the_keyboard_walks_the_screen() {
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        let headings = view
            .sections
            .iter()
            .map(|section| section.id.title())
            .collect::<Vec<_>>();
        assert_eq!(headings.len(), 8, "{headings:?}");

        let mut seen = Vec::new();
        let stops = view.focusable_keys().len();
        let mut every_frame_showed_everything = true;
        for _ in 0..=stops {
            let (terminal, _) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
            let frame = rendered(terminal.backend().buffer());
            every_frame_showed_everything &= headings.iter().all(|heading| frame.contains(heading));
            for heading in &headings {
                if frame.contains(heading) && !seen.contains(heading) {
                    seen.push(heading);
                }
            }
            view.update(SettingsAction::FocusNext);
        }
        assert!(
            !every_frame_showed_everything,
            "this terminal must be too small to hold the screen, or the test claims nothing"
        );
        for heading in &headings {
            assert!(
                seen.contains(heading),
                "{heading:?} never came into view; saw {seen:?}"
            );
        }
    }

    /// The focused control must be inside the viewport after every move.
    ///
    /// This asserts the outcome, not the affordance. An earlier test asserted that a scrollbar
    /// existed and passed while the focused control was not drawn at all.
    #[test]
    fn the_focused_control_stays_inside_the_viewport_after_every_move() {
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        let stops = view
            .focusable_keys()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut scrolled = false;
        for _ in 0..stops.len().saturating_mul(2) {
            let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
            scrolled |= geometry.first_visible > 0;
            let focused = view.focused();
            let area = session
                .field_area(focused)
                .unwrap_or_else(|| panic!("the focused control {focused:?} was not rendered"));
            assert!(
                area.y >= geometry.body.y
                    && area.y.saturating_add(area.height)
                        <= geometry.body.y.saturating_add(geometry.body.height),
                "focus moved to {focused:?} at {area:?}, outside the viewport {:?}",
                geometry.body
            );
            view.update(SettingsAction::FocusNext);
        }
        assert!(
            scrolled,
            "this terminal must be too small to hold the screen, or the test claims nothing"
        );

        // Walking back has the same contract.
        for _ in 0..stops.len().saturating_mul(2) {
            view.update(SettingsAction::FocusPrevious);
            let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
            let focused = view.focused();
            let area = session
                .field_area(focused)
                .unwrap_or_else(|| panic!("the focused control {focused:?} was not rendered"));
            assert!(
                area.y >= geometry.body.y
                    && area.y.saturating_add(area.height)
                        <= geometry.body.y.saturating_add(geometry.body.height),
                "focus moved back to {focused:?} at {area:?}, outside the viewport {:?}",
                geometry.body
            );
        }
    }

    /// A terminal that grows back does not leave the screen scrolled past its own end.
    ///
    /// Nothing else moves the offset, so the clamp is the only thing that can return the blank rows
    /// a taller terminal would otherwise show under the last section.
    #[test]
    fn a_terminal_that_grows_back_shows_no_rows_past_the_end() {
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        for _ in 0..view.focusable_keys().len() {
            view.update(SettingsAction::FocusNext);
        }
        let (_, small) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
        assert!(
            small.first_visible > 0,
            "the small terminal must have scrolled for this to claim anything"
        );

        // Tall enough for every row at once.
        let (terminal, tall) = draw(&mut session, &view, DEMO_WIDTH, 90);
        assert_eq!(
            tall.first_visible, 0,
            "the screen stayed scrolled past its end"
        );
        let frame = rendered(terminal.backend().buffer());
        assert!(
            frame.contains("Basics"),
            "the first section came back: {frame}"
        );
    }

    /// The parameter section draws every control a person needs and answers both inputs.
    ///
    /// Version 0.4 puts the name and the script's own metadata inside the keep toggle's label
    /// (`src/skit/tui_settings.py:99-101`), lists unmanaged constants as checkboxes under one
    /// sentence (`:624-631`), and gives the resync a chord (`:408-415`). This asserts what a person
    /// sees and what a click does, not that a widget was constructed.
    #[test]
    fn the_parameter_section_draws_its_rows_and_answers_the_keyboard_and_the_mouse() {
        let mut greeting = ParamDecl::new("GREETING");
        greeting.default = Some(ParameterValue::String("world".to_owned()));
        let mut view = SettingsView::from_inputs(&SettingsInputs {
            selector: "tool".to_owned(),
            kind: "python".to_owned(),
            name: "Tool".to_owned(),
            workdir: "invoke".to_owned(),
            has_original_file: true,
            has_stored_name: true,
            has_analyzer: true,
            managed: vec![greeting],
            candidates: vec!["WIDTH".to_owned()],
            ..SettingsInputs::default()
        });
        let mut session = SettingsScreenSession::default();
        let (terminal, geometry) = draw(&mut session, &view, DEMO_WIDTH, 120);
        let frame = rendered(terminal.backend().buffer());

        let row_of = |needle: &str| {
            frame
                .lines()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle} is not on screen:\n{frame}"))
        };
        let heading = row_of("Parameters (the run form's fields)");
        // The row reads out its name, its type and its default in one line, exactly as v0.4 does.
        let managed_row = row_of("GREETING  str 'world'");
        let offer = row_of("Detected but not yet managed — tick to manage:");
        let candidate = row_of("WIDTH");
        let resync = row_of("Read the parameter definitions from the script again on save");
        // Explanation and control are one stream: the offer's sentence follows the rows it comes
        // after and introduces the checkboxes it labels (`src/skit/tui_settings.py:607-637`).
        assert!(
            heading < managed_row && managed_row < offer && offer < candidate,
            "the section is out of order:\n{frame}"
        );
        assert!(candidate < resync, "{frame}");

        // Clicking the candidate ticks it, and the ticked set is what a save carries.
        let hit = geometry
            .hits
            .iter()
            .find(|hit| {
                hit.target
                    == SettingsControlId::Option {
                        field: MANAGE_KEY.to_owned(),
                        value: "WIDTH".to_owned(),
                    }
            })
            .expect("the candidate is not a click target");
        let area = hit.area;
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(
            view.submitted_values()
                .get(MANAGE_KEY)
                .map(FieldValue::as_text)
                .as_deref(),
            Some("WIDTH")
        );

        // The keep toggle answers Space, which is the unmanage a person reaches by keyboard.
        view.update(SettingsAction::Focus {
            key: "parameter:GREETING:keep".to_owned(),
        });
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            key(KeyCode::Char(' '), KeyModifiers::NONE),
        );
        assert_eq!(
            view.submitted_values().get("parameter:GREETING:keep"),
            Some(&FieldValue::boolean(false))
        );
    }

    /// Pressing `s` in the Library must land the eye on the presets, with or without any.
    ///
    /// Version 0.4 scrolls the body to the section the deep link names (`src/skit/tui.py:991-992`,
    /// `src/skit/tui_settings.py:876-882`). The empty case is the one that matters most: the whole
    /// answer to "where are my presets" is one sentence, and a viewport left at the top would hide
    /// it behind five other sections.
    #[test]
    fn the_preset_deep_link_puts_the_section_on_screen_even_with_nothing_to_focus() {
        let empty = SettingsView::from_inputs(&SettingsInputs {
            revealed: Some(SettingsSectionId::Presets),
            ..settings_inputs()
        });
        let mut session = SettingsScreenSession::default();
        let (terminal, _) = draw(&mut session, &empty, DEMO_WIDTH, DEMO_HEIGHT);
        let frame = rendered(terminal.backend().buffer());
        assert!(
            frame.contains("None yet — press Ctrl+S inside the run form to save one."),
            "the deep link did not reach the presets:\n{frame}"
        );

        // Without the deep link the same screen opens at the top, where that sentence is not.
        let plain = SettingsView::from_inputs(&settings_inputs());
        let mut session = SettingsScreenSession::default();
        let (terminal, _) = draw(&mut session, &plain, DEMO_WIDTH, DEMO_HEIGHT);
        let top = rendered(terminal.backend().buffer());
        assert!(top.contains("Basics"), "{top}");
        assert!(
            !top.contains("None yet — press Ctrl+S"),
            "the screen is short enough that this test proves nothing:\n{top}"
        );

        // With presets, the deep link lands the keyboard on the first one, and Space deletes it.
        let mut listed = SettingsView::from_inputs(&SettingsInputs {
            revealed: Some(SettingsSectionId::Presets),
            presets: BTreeMap::from([(
                "nightly".to_owned(),
                BTreeMap::from([("mode".to_owned(), "fast".to_owned())]),
            )]),
            ..settings_inputs()
        });
        let mut session = SettingsScreenSession::default();
        let (terminal, geometry) = draw(&mut session, &listed, DEMO_WIDTH, DEMO_HEIGHT);
        let frame = rendered(terminal.backend().buffer());
        assert!(frame.contains("nightly  mode=fast"), "{frame}");
        dispatch(
            &mut session,
            &mut listed,
            &geometry,
            key(KeyCode::Char(' '), KeyModifiers::NONE),
        );
        assert_eq!(
            listed.submitted_values().get("preset:nightly"),
            Some(&FieldValue::boolean(false)),
            "Space on the deep-linked preset must be the delete"
        );

        // A section taller than the viewport must arrive showing its start, not its tail. The
        // keyboard lands on the first preset, so bottom-aligning would put the focused control off
        // screen — the exact failure a viewport rule has to rule out.
        let many = (0..40)
            .map(|index| {
                (
                    format!("preset-{index:02}"),
                    BTreeMap::from([("mode".to_owned(), "fast".to_owned())]),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let crowded = SettingsView::from_inputs(&SettingsInputs {
            revealed: Some(SettingsSectionId::Presets),
            presets: many,
            ..settings_inputs()
        });
        assert_eq!(crowded.focused(), "preset:preset-00");
        let mut session = SettingsScreenSession::default();
        let (terminal, _) = draw(&mut session, &crowded, DEMO_WIDTH, DEMO_HEIGHT);
        let frame = rendered(terminal.backend().buffer());
        assert!(
            frame.contains("Presets"),
            "the heading scrolled away:\n{frame}"
        );
        assert!(
            frame.contains("preset-00  mode=fast"),
            "the focused preset was not drawn:\n{frame}"
        );
    }

    /// A screen with nothing to edit still draws what it has.
    ///
    /// The keyboard has no stop at all here, so the alignment pass has no row to work from. It must
    /// leave the offset alone rather than reach for a row that is not there.
    #[test]
    fn a_screen_with_nothing_to_edit_still_draws() {
        let mut view = prompt_view();
        view.sections.clear();
        view.sections.push(SettingsSection::new(
            SettingsSectionId::Storage,
            vec![SettingsItem::field(Field::read_only(
                "linked",
                "Source",
                FieldOwner::Declared,
                FieldValue::text("/home/ada/brief.md"),
                ReadOnlyReason::ReferenceMode,
            ))],
        ));
        assert!(view.focusable_keys().is_empty());
        assert_eq!(view.focused(), "");

        let mut session = SettingsScreenSession::default();
        let (terminal, geometry) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
        let frame = rendered(terminal.backend().buffer());
        assert_eq!(geometry.first_visible, 0);
        assert!(frame.contains("Source: /home/ada/brief.md"), "{frame}");
        assert!(frame.contains("skit doesn't write to this file"), "{frame}");
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn click(area: Rect) -> Event {
        mouse(area, MouseEventKind::Down(MouseButton::Left))
    }

    fn mouse(area: Rect, kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// Drive one event and apply whatever action it produced.
    fn dispatch(
        session: &mut SettingsScreenSession,
        view: &mut SettingsView,
        geometry: &SettingsScreenGeometry,
        event: Event,
    ) -> Option<SettingsScreenEvent> {
        let handled = session.handle_event(event, view, geometry);
        if let Some(SettingsScreenEvent::Action(action)) = handled.clone() {
            view.update(action);
        }
        handled
    }

    /// Typing reaches the focused box, and the model keeps every character.
    #[test]
    fn typing_reaches_the_focused_box_and_the_model_keeps_it() {
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        assert_eq!(view.focused(), NAME_KEY);

        for character in ['!', '\u{301}', '🧑'] {
            dispatch(
                &mut session,
                &mut view,
                &geometry,
                key(KeyCode::Char(character), KeyModifiers::NONE),
            );
        }
        assert_eq!(
            view.field(NAME_KEY).unwrap().value().as_text(),
            "Brief!\u{301}🧑"
        );
        assert!(view.is_dirty(), "typing is an edit the discard guard sees");

        // The cursor a person sees is the terminal's own.
        let (terminal, _) = draw(&mut session, &view, DEMO_WIDTH, 90);
        assert!(
            session
                .field_area(NAME_KEY)
                .unwrap()
                .contains(terminal.backend().cursor_position()),
            "the cursor must sit in the focused box"
        );
    }

    /// A control chord is never eaten by a text box, so the footer's keys always work.
    ///
    /// Version 0.4 puts Save on `Ctrl+S` and Back on Esc for this screen
    /// (`src/skit/tui_settings.py:408-420`). Both must reach the shared registry from inside an
    /// input, which is where a person is when they finish typing.
    #[test]
    fn a_control_chord_is_never_eaten_by_a_text_box() {
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();

        // Both text controls, because they have different appetites. A multi-line body carries
        // emacs-style editing bindings, so `Ctrl+N` reads as "next line" to it unless the chord is
        // taken away first.
        for focus in [NAME_KEY, DESCRIPTION_KEY] {
            view.update(SettingsAction::Focus {
                key: focus.to_owned(),
            });
            let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
            let before = view.field(focus).unwrap().value().as_text();
            for chord in [
                key(KeyCode::Char('s'), KeyModifiers::CONTROL),
                key(KeyCode::Char('n'), KeyModifiers::CONTROL),
                key(KeyCode::Esc, KeyModifiers::NONE),
            ] {
                assert_eq!(
                    session.handle_event(chord.clone(), &view, &geometry),
                    None,
                    "{chord:?} must fall through to the command registry from {focus}"
                );
            }
            assert_eq!(
                view.field(focus).unwrap().value().as_text(),
                before,
                "a chord must not edit {focus}"
            );
        }
        assert!(!view.is_dirty(), "no chord is an edit");
    }

    /// Tab and the arrow twins move the keyboard, and a choice keeps the arrows for its own rows.
    ///
    /// Version 0.4 gives every form footer both directions and lets the focused widget win the
    /// arrows when it needs them (`src/skit/tui_footer.py:72-94`).
    #[test]
    fn tab_always_moves_focus_and_a_choice_keeps_the_arrows_for_its_options() {
        // A real multiline value keeps vertical arrows while the cursor can move, then yields Up
        // at the top boundary to the same previous-field action as Shift+Tab.
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        view.set_value(DESCRIPTION_KEY, FieldValue::text("first\nmiddle\nlast"));
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        assert_eq!(view.focused(), NAME_KEY);
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            key(KeyCode::Tab, KeyModifiers::NONE),
        );
        assert_eq!(view.focused(), DESCRIPTION_KEY);
        assert_eq!(session.bodies[DESCRIPTION_KEY].cursor().0, 2);
        for row in [1, 0] {
            assert_eq!(
                dispatch(
                    &mut session,
                    &mut view,
                    &geometry,
                    key(KeyCode::Up, KeyModifiers::NONE),
                ),
                Some(SettingsScreenEvent::Changed)
            );
            assert_eq!(view.focused(), DESCRIPTION_KEY);
            assert_eq!(session.bodies[DESCRIPTION_KEY].cursor().0, row);
        }
        assert_eq!(
            dispatch(
                &mut session,
                &mut view,
                &geometry,
                key(KeyCode::Up, KeyModifiers::NONE),
            ),
            Some(SettingsScreenEvent::Action(SettingsAction::FocusPrevious))
        );
        assert_eq!(view.focused(), NAME_KEY);
        assert_eq!(
            view.field(DESCRIPTION_KEY).unwrap().value().as_text(),
            "first\nmiddle\nlast"
        );

        // The initial cursor is at Bottom+End. Down at that boundary yields the next form field.
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        view.set_value(DESCRIPTION_KEY, FieldValue::text("first\nmiddle\nlast"));
        view.update(SettingsAction::Focus {
            key: DESCRIPTION_KEY.to_owned(),
        });
        let focusable = view.focusable_keys();
        let next = focusable
            .iter()
            .position(|key| *key == DESCRIPTION_KEY)
            .and_then(|index| focusable.get(index + 1))
            .expect("description has a next focus stop")
            .to_string();
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        assert_eq!(session.bodies[DESCRIPTION_KEY].cursor().0, 2);
        assert_eq!(
            dispatch(
                &mut session,
                &mut view,
                &geometry,
                key(KeyCode::Down, KeyModifiers::NONE),
            ),
            Some(SettingsScreenEvent::Action(SettingsAction::FocusNext))
        );
        assert_eq!(view.focused(), next);

        // Shift owns selection even at a vertical boundary. A plain boundary arrow also stays in
        // the textarea while a selection is active, so focus cannot leave with a latent selection.
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        view.set_value(DESCRIPTION_KEY, FieldValue::text("first\nmiddle\nlast"));
        view.update(SettingsAction::Focus {
            key: DESCRIPTION_KEY.to_owned(),
        });
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        for row in [1, 0, 0] {
            assert_eq!(
                dispatch(
                    &mut session,
                    &mut view,
                    &geometry,
                    key(KeyCode::Up, KeyModifiers::SHIFT),
                ),
                Some(SettingsScreenEvent::Changed)
            );
            assert_eq!(view.focused(), DESCRIPTION_KEY);
            assert_eq!(session.bodies[DESCRIPTION_KEY].cursor().0, row);
            assert!(session.bodies[DESCRIPTION_KEY].selection_range().is_some());
        }
        let selection = session.bodies[DESCRIPTION_KEY].selection_range();
        assert_eq!(
            dispatch(
                &mut session,
                &mut view,
                &geometry,
                key(KeyCode::Up, KeyModifiers::NONE),
            ),
            Some(SettingsScreenEvent::Changed)
        );
        assert_eq!(view.focused(), DESCRIPTION_KEY);
        assert_eq!(session.bodies[DESCRIPTION_KEY].selection_range(), selection);

        // On a closed option set the arrows walk the options and clamp at both ends.
        view.update(SettingsAction::Focus {
            key: WORKDIR_KEY.to_owned(),
        });
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        assert_eq!(view.field(WORKDIR_KEY).unwrap().value().as_text(), "invoke");
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            key(KeyCode::Up, KeyModifiers::NONE),
        );
        assert_eq!(view.field(WORKDIR_KEY).unwrap().value().as_text(), "store");
        assert_eq!(
            view.focused(),
            WORKDIR_KEY,
            "the option set kept the keyboard"
        );
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            key(KeyCode::Down, KeyModifiers::NONE),
        );
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            key(KeyCode::Down, KeyModifiers::NONE),
        );
        assert_eq!(
            view.field(WORKDIR_KEY).unwrap().value().as_text(),
            WORKDIR_CUSTOM
        );
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            key(KeyCode::Down, KeyModifiers::NONE),
        );
        assert_eq!(
            view.field(WORKDIR_KEY).unwrap().value().as_text(),
            WORKDIR_CUSTOM,
            "the last option holds rather than wrapping"
        );
        // Choosing the typed folder reveals its box, and Tab now reaches it.
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            key(KeyCode::Tab, KeyModifiers::NONE),
        );
        assert_eq!(view.focused(), skit_ui::WORKDIR_PATH_KEY);
    }

    /// Every control a person can click does what its key does.
    ///
    /// Product rule 2: each action is available by keyboard and mouse, and each visible affordance
    /// is a click target.
    #[test]
    fn every_visible_control_has_a_mouse_twin() {
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);

        // A field reacts to the button press only. Hover and release neither move focus nor edit.
        let description = geometry
            .hits
            .iter()
            .find(|hit| hit.target == SettingsControlId::Field(DESCRIPTION_KEY.to_owned()))
            .expect("the description box is clickable");
        let description_value = view
            .field(DESCRIPTION_KEY)
            .unwrap()
            .value()
            .as_text()
            .to_owned();
        for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
            assert_eq!(
                session.handle_event(mouse(description.area, kind), &view, &geometry),
                None
            );
            assert_eq!(view.focused(), NAME_KEY);
            assert_eq!(
                view.field(DESCRIPTION_KEY).unwrap().value().as_text(),
                description_value
            );
        }
        dispatch(&mut session, &mut view, &geometry, click(description.area));
        assert_eq!(view.focused(), DESCRIPTION_KEY);

        // Clicking a text box moves the keyboard to it.
        let needs = geometry
            .hits
            .iter()
            .find(|hit| hit.target == SettingsControlId::Field(NEEDS_KEY.to_owned()))
            .expect("the needs box is clickable");
        dispatch(&mut session, &mut view, &geometry, click(needs.area));
        assert_eq!(view.focused(), NEEDS_KEY);

        // Clicking one option picks it, by value, and takes the keyboard with it.
        let store = geometry
            .hits
            .iter()
            .find(|hit| {
                hit.target
                    == SettingsControlId::Option {
                        field: WORKDIR_KEY.to_owned(),
                        value: "store".to_owned(),
                    }
            })
            .expect("every option is clickable");
        dispatch(&mut session, &mut view, &geometry, click(store.area));
        assert_eq!(view.field(WORKDIR_KEY).unwrap().value().as_text(), "store");
        assert_eq!(view.focused(), WORKDIR_KEY);

        // The new-agent chip is the same door Ctrl+N opens.
        let door = geometry
            .hits
            .iter()
            .find(|hit| hit.target == SettingsControlId::NewRunner)
            .expect("the new-agent chip is clickable");
        assert_eq!(
            session.handle_event(click(door.area), &view, &geometry),
            Some(SettingsScreenEvent::Action(SettingsAction::NewRunner))
        );
    }

    /// The wheel moves the viewport and keeps it there until the keyboard goes somewhere new.
    #[test]
    fn the_wheel_keeps_what_the_reader_scrolled_to() {
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
        assert_eq!(geometry.first_visible, 0);

        let wheel = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: geometry.body.x,
            row: geometry.body.y,
            modifiers: KeyModifiers::NONE,
        });
        for _ in 0..4 {
            assert_eq!(
                session.handle_event(wheel.clone(), &view, &geometry),
                Some(SettingsScreenEvent::Changed)
            );
        }
        let (_, scrolled) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
        assert!(scrolled.first_visible > 0, "the wheel moved the viewport");

        // Re-rendering does not yank it back, on this frame or any later one. Springing back one
        // frame late is the defect an "did the offset change" test would miss.
        for _ in 0..3 {
            let (_, again) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
            assert_eq!(
                again.first_visible, scrolled.first_visible,
                "the viewport must stay where the reader put it"
            );
        }

        // Moving the keyboard is a new intent, and the control it lands on comes back into view.
        dispatch(
            &mut session,
            &mut view,
            &scrolled,
            key(KeyCode::Tab, KeyModifiers::NONE),
        );
        assert_eq!(view.focused(), DESCRIPTION_KEY);
        let (_, followed) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
        let area = session
            .field_area(DESCRIPTION_KEY)
            .expect("the focused box is drawn");
        assert!(
            area.y >= followed.body.y
                && area.y.saturating_add(area.height)
                    <= followed.body.y.saturating_add(followed.body.height),
            "focus landed at {area:?}, outside the viewport {:?}",
            followed.body
        );
    }

    /// A toggle answers Space and a click, and a read-only row answers neither.
    #[test]
    fn a_toggle_answers_the_keyboard_and_the_mouse_and_read_only_text_answers_neither() {
        let mut view = prompt_view();
        view.sections.push(SettingsSection::new(
            SettingsSectionId::Basics,
            vec![
                SettingsItem::field(Field::new(
                    "kind:boolean",
                    "Dry run",
                    FieldKind::Boolean,
                    FieldOwner::Declared,
                    FieldValue::boolean(false),
                )),
                SettingsItem::field(Field::read_only(
                    "kind:read-only",
                    "Delivery",
                    FieldOwner::Declared,
                    FieldValue::text("flag"),
                    ReadOnlyReason::FixedAtAddTime,
                )),
            ],
        ));
        let mut session = SettingsScreenSession::default();
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);

        view.update(SettingsAction::Focus {
            key: "kind:boolean".to_owned(),
        });
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            key(KeyCode::Char(' '), KeyModifiers::NONE),
        );
        assert_eq!(
            view.field("kind:boolean").unwrap().value().as_text(),
            "true"
        );

        let toggle = geometry
            .hits
            .iter()
            .find(|hit| hit.target == SettingsControlId::Field("kind:boolean".to_owned()))
            .expect("a toggle is clickable");
        dispatch(&mut session, &mut view, &geometry, click(toggle.area));
        assert_eq!(
            view.field("kind:boolean").unwrap().value().as_text(),
            "false",
            "one click is one flip"
        );

        // Read-only text has no target at all, so a click on it reaches the registry instead.
        assert!(
            !geometry
                .hits
                .iter()
                .any(|hit| hit.target == SettingsControlId::Field("kind:read-only".to_owned()))
        );
    }

    /// The panel names the entry, and every option of a closed set is its own click target.
    #[test]
    fn the_screen_titles_the_entry_and_offers_every_option_to_the_mouse() {
        let mut session = SettingsScreenSession::default();
        let view = prompt_view();
        let (terminal, geometry) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
        let frame = rendered(terminal.backend().buffer());
        assert!(frame.contains("Entry settings · Brief"), "{frame}");
        assert!(frame.contains("Renaming keeps everything"), "{frame}");

        // The name box carries the value the screen opened with.
        assert!(frame.contains("Brief"), "{frame}");
        assert!(
            geometry
                .hits
                .iter()
                .any(|hit| hit.target == SettingsControlId::Field(NAME_KEY.to_owned())),
            "the name box must be clickable"
        );
    }

    /// The working-directory path box appears and disappears with its own choice.
    ///
    /// The renderer names no field here: it draws what the model says a person can reach
    /// (`src/skit/tui_settings.py:483-491`).
    #[test]
    fn the_custom_path_box_follows_its_choice_without_a_rule_of_its_own() {
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        let _ = draw(&mut session, &view, DEMO_WIDTH, 60);
        assert!(session.field_area(skit_ui::WORKDIR_PATH_KEY).is_none());

        view.update(SettingsAction::SetField {
            key: WORKDIR_KEY.to_owned(),
            value: FieldValue::Explicit(TypedValue::Choice(WORKDIR_CUSTOM.to_owned())),
        });
        let (terminal, geometry) = draw(&mut session, &view, DEMO_WIDTH, 60);
        assert!(session.field_area(skit_ui::WORKDIR_PATH_KEY).is_some());
        assert!(
            rendered(terminal.backend().buffer()).contains("/absolute/path"),
            "the typed folder needs its prompt"
        );
        assert!(
            geometry.hits.iter().any(|hit| hit.target
                == SettingsControlId::Option {
                    field: WORKDIR_KEY.to_owned(),
                    value: WORKDIR_CUSTOM.to_owned(),
                }),
            "every option is a click target, keyed by value"
        );
    }

    /// A prompt keeps the door to define a new agent open, beside its picker.
    #[test]
    fn the_runner_picker_carries_the_new_agent_affordance() {
        let mut session = SettingsScreenSession::default();
        let view = prompt_view();
        let (terminal, geometry) = draw(&mut session, &view, DEMO_WIDTH, 60);
        let frame = rendered(terminal.backend().buffer());
        assert!(frame.contains("New agent…"), "{frame}");
        assert!(frame.contains("Ctrl+N"), "{frame}");
        assert!(
            geometry
                .hits
                .iter()
                .any(|hit| hit.target == SettingsControlId::NewRunner),
            "the key hint is the click target"
        );
        // Configured runners are user data and are never looked up in the catalog.
        assert!(frame.contains("claude"), "{frame}");
        assert!(frame.contains("codex"), "{frame}");
    }

    /// Every field kind the model can carry draws a control a person can use.
    ///
    /// The six settings sections use four kinds today. The rest are reachable through the same
    /// `Field` model, so each one gets a control rather than a silent blank row.
    #[test]
    fn every_field_kind_draws_a_control() {
        let mut view = prompt_view();
        view.sections.push(SettingsSection::new(
            SettingsSectionId::Basics,
            vec![
                SettingsItem::field(Field::new(
                    "kind:secret",
                    "Secret",
                    FieldKind::Secret,
                    FieldOwner::Declared,
                    FieldValue::text("hunter2"),
                )),
                SettingsItem::field(Field::new(
                    "kind:boolean",
                    "Dry run",
                    FieldKind::Boolean,
                    FieldOwner::Declared,
                    FieldValue::boolean(true),
                )),
                SettingsItem::field(Field::new(
                    "kind:number",
                    "Retries",
                    FieldKind::Number { integer: true },
                    FieldOwner::Declared,
                    FieldValue::Explicit(TypedValue::Integer(3)),
                )),
                SettingsItem::field(Field::new(
                    "kind:arguments",
                    "Extra arguments",
                    FieldKind::ArgumentList {
                        dialect: ArgumentDialect::Posix,
                    },
                    FieldOwner::Declared,
                    FieldValue::Explicit(TypedValue::Arguments(vec!["--force".to_owned()])),
                )),
                SettingsItem::field(
                    Field::new(
                        "kind:multi",
                        "Lanes",
                        FieldKind::MultiChoice {
                            options: vec![ChoiceOption::plain("fast"), ChoiceOption::plain("slow")],
                        },
                        FieldOwner::Declared,
                        FieldValue::Explicit(TypedValue::Choices(vec!["slow".to_owned()])),
                    )
                    .with_capabilities(FieldCapabilities::default()),
                ),
                SettingsItem::field(Field::read_only(
                    "kind:read-only",
                    "Delivery",
                    FieldOwner::Declared,
                    FieldValue::text("flag"),
                    ReadOnlyReason::FixedAtAddTime,
                )),
                SettingsItem::field(Field::read_only(
                    "kind:source-declares",
                    "Type",
                    FieldOwner::SourceBlock,
                    FieldValue::text("int"),
                    ReadOnlyReason::SourceDeclares,
                )),
                SettingsItem::field(Field::read_only(
                    "kind:derived",
                    "Env target",
                    FieldOwner::Declared,
                    FieldValue::text("COUNT"),
                    ReadOnlyReason::Derived,
                )),
            ],
        ));

        let mut session = SettingsScreenSession::default();
        let (terminal, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        let frame = rendered(terminal.backend().buffer());

        assert!(
            frame.contains("•••••••"),
            "a secret must never show: {frame}"
        );
        assert!(frame.contains("Dry run"), "{frame}");
        assert!(frame.contains("Retries"), "{frame}");
        assert!(frame.contains("--force"), "{frame}");
        assert!(frame.contains("☑"), "a ticked multi-choice needs its glyph");
        assert!(
            frame.contains("☐"),
            "an unticked multi-choice needs its glyph"
        );
        // A read-only row is text with its reason, never a control that refuses a click.
        assert!(frame.contains("Delivery: flag"), "{frame}");
        assert!(
            frame.contains("Set when the entry was added"),
            "a refusal must say why: {frame}"
        );
        assert!(
            !geometry
                .hits
                .iter()
                .any(|hit| hit.target == SettingsControlId::Field("kind:read-only".to_owned())),
            "a read-only row is not a click target"
        );
        assert!(
            geometry.hits.iter().any(|hit| hit.target
                == SettingsControlId::Option {
                    field: "kind:multi".to_owned(),
                    value: "fast".to_owned(),
                }),
            "each multi-choice option is its own target"
        );
    }

    #[test]
    fn paste_sync_release_and_control_kinds_keep_typed_state_and_reverse_events() {
        let mut session = SettingsScreenSession::default();
        let mut view = prompt_view();
        view.sections.push(SettingsSection::new(
            SettingsSectionId::Basics,
            vec![SettingsItem::field(Field::read_only(
                "source:delivery",
                "",
                FieldOwner::Declared,
                FieldValue::text("flag"),
                ReadOnlyReason::FixedAtAddTime,
            ))],
        ));
        view.update(SettingsAction::Focus {
            key: DESCRIPTION_KEY.to_owned(),
        });
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        let pasted = dispatch(
            &mut session,
            &mut view,
            &geometry,
            Event::Paste("\nnew line".to_owned()),
        );
        assert!(matches!(
            pasted,
            Some(SettingsScreenEvent::Action(SettingsAction::SetField { ref key, .. }))
                if key == DESCRIPTION_KEY
        ));
        assert!(
            view.field(DESCRIPTION_KEY)
                .expect("description exists")
                .value()
                .as_text()
                .contains("new line")
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Left, KeyModifiers::NONE), &view, &geometry,),
            Some(SettingsScreenEvent::Changed)
        );
        assert!(matches!(
            dispatch(
                &mut session,
                &mut view,
                &geometry,
                key(KeyCode::Char('x'), KeyModifiers::NONE),
            ),
            Some(SettingsScreenEvent::Action(SettingsAction::SetField { ref key, .. }))
                if key == DESCRIPTION_KEY
        ));

        view.set_value(DESCRIPTION_KEY, FieldValue::text("replacement\nbody"));
        let _ = draw(&mut session, &view, DEMO_WIDTH, 90);
        view.update(SettingsAction::Focus {
            key: NAME_KEY.to_owned(),
        });
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        assert!(matches!(
            dispatch(
                &mut session,
                &mut view,
                &geometry,
                Event::Paste(" pasted".to_owned()),
            ),
            Some(SettingsScreenEvent::Action(SettingsAction::SetField { ref key, .. }))
                if key == NAME_KEY
        ));
        assert_eq!(
            session.handle_event(key(KeyCode::Left, KeyModifiers::NONE), &view, &geometry,),
            Some(SettingsScreenEvent::Changed)
        );
        view.set_value(NAME_KEY, FieldValue::text("Host replacement"));
        let _ = draw(&mut session, &view, DEMO_WIDTH, 90);

        for event in [
            Event::FocusGained,
            Event::Resize(40, 10),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Enter,
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
        ] {
            assert_eq!(session.handle_event(event, &view, &geometry), None);
        }
        assert_eq!(
            session.handle_event(
                mouse(Rect::new(0, 0, 1, 1), MouseEventKind::Moved),
                &view,
                &geometry,
            ),
            None
        );

        let boolean = view
            .fields()
            .find(|field| matches!(field.kind, FieldKind::Boolean))
            .expect("the complete settings view has a toggle")
            .key
            .clone();
        view.update(SettingsAction::Focus {
            key: boolean.clone(),
        });
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        assert_eq!(
            session.handle_event(key(KeyCode::Down, KeyModifiers::NONE), &view, &geometry),
            Some(SettingsScreenEvent::Action(SettingsAction::FocusNext))
        );

        let read_only = view
            .fields()
            .find(|field| matches!(field.kind, FieldKind::ReadOnly))
            .expect("the complete settings view has read-only source facts")
            .key
            .clone();
        let read_only_field = view
            .field(&read_only)
            .expect("the read-only field stays addressable")
            .clone();
        assert_eq!(
            session.handle_field_key(
                &read_only,
                &read_only_field,
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            ),
            None
        );

        let single = view
            .fields()
            .find(|field| matches!(field.kind, FieldKind::SingleChoice { .. }))
            .expect("the complete settings view has one closed choice")
            .key
            .clone();
        view.update(SettingsAction::Focus {
            key: single.clone(),
        });
        let (_, geometry) = draw(&mut session, &view, DEMO_WIDTH, 90);
        assert!(matches!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &view, &geometry),
            Some(SettingsScreenEvent::Action(SettingsAction::SetField { key, .. }))
                if key == single
        ));

        let options = vec![ChoiceOption::plain("one"), ChoiceOption::plain("two")];
        let multiple = Field::new(
            "multiple",
            "Multiple",
            FieldKind::MultiChoice {
                options: options.clone(),
            },
            FieldOwner::Declared,
            FieldValue::Explicit(TypedValue::Choices(Vec::new())),
        );
        assert_eq!(
            session.handle_field_key(
                "multiple",
                &multiple,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            ),
            Some(SettingsScreenEvent::Action(SettingsAction::FocusNext))
        );

        let empty_choice = Field::new(
            "empty",
            "Empty",
            FieldKind::SingleChoice {
                options: Vec::new(),
            },
            FieldOwner::Declared,
            FieldValue::Inherit,
        );
        assert_eq!(
            choice_key(
                &empty_choice,
                &[],
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            ),
            None
        );
        let option = ChoiceOption::labelled("custom", "Custom {}").with_detail("/tmp/config");
        assert!(option_text(Locale::En, &option).contains("/tmp/config"));
        assert!(!is_selected(&empty_choice, &option));
    }
}
