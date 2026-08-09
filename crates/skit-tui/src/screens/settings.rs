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
use ratatui_interact::components::{CheckBox, CheckBoxState, ScrollableContentState};
use ratatui_textarea::TextArea as RichTextArea;
use ratatui_widgets::paragraph::{Paragraph, Wrap};
use skit_form::field::{ChoiceOption, Field, FieldKind, ReadOnlyReason, TypedValue};
use skit_i18n::{Locale, format_text, text};
use skit_ui::{SettingsNote, SettingsSectionId, SettingsView};
use tui_input::Input as LineInput;

use crate::{
    session::{checkbox_style, new_textarea, render_line_input, render_textarea, textarea_text},
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
    /// One section heading.
    Heading(String),
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
    fn sync(&mut self, view: &SettingsView) {
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

    /// Put the focused control inside the viewport.
    ///
    /// This runs on every render, and it is the only thing that moves the offset on its own. The
    /// invariant is the one a person can see — the control that owns the keyboard is drawn — so it
    /// is checked against the rows this same render laid out, never against a remembered position.
    ///
    /// There is no deferred flag. A flag records that focus moved and asks a later call to act on
    /// it, and every call site that moves focus has to remember to set it. Here the focused key is
    /// read from the model each frame, so a move made anywhere, by any input, is followed with no
    /// cooperation at all. The same read covers the case a flag cannot see: a resize or a section
    /// that appeared moves the rows underneath a focus that never changed.
    fn follow_focus(&mut self, focused: &str, total: usize) {
        let maximum = total.saturating_sub(self.visible_height);
        if self.scroll.scroll_offset() > maximum {
            self.scroll.set_scroll_offset(maximum);
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
            self.scroll
                .set_scroll_offset(end.saturating_sub(self.visible_height));
        }
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
    for item in &items {
        if let Item::Control { key, .. } = &item.item {
            session.spans.insert(key.clone(), (item.start, item.height));
        }
    }
    let focused = view.focused().to_owned();
    session.follow_focus(&focused, total);

    let mut hits = Vec::new();
    for item in &items {
        let Some(rect) = session.visible_rect(item.start, item.height) else {
            continue;
        };
        match &item.item {
            Item::Spacer => {}
            Item::Heading(value) => frame.render_widget(
                Paragraph::new(value.as_str())
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
                let Some(field) = view.field(key) else {
                    continue;
                };
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
    SettingsScreenGeometry {
        body,
        first_visible: session.scroll.scroll_offset(),
        hits,
    }
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
            Item::Heading(text(locale, section.id.title()).into_owned()),
            1,
        );
        for note in &section.notes {
            let shown = note_text(locale, note);
            let height = wrapped_height(&shown, width);
            push(&mut items, &mut start, Item::Copy(shown), height);
        }
        for field in &section.fields {
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
        FieldKind::Multiline => 6,
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
    view.sections
        .iter()
        .flat_map(|section| section.fields.iter())
}

#[cfg(test)]
mod tests {
    use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
    use skit_form::field::{
        ArgumentDialect, FieldCapabilities, FieldOwner, FieldValue, ReadOnlyReason,
    };
    use skit_ui::{
        DependencyFlavor, NAME_KEY, SettingsAction, SettingsInputs, SettingsSection,
        SettingsSectionId, WORKDIR_CUSTOM, WORKDIR_KEY,
    };

    use super::{
        ChoiceOption, Field, FieldKind, Locale, SettingsControlId, SettingsScreenGeometry,
        SettingsScreenSession, SettingsView, TypedValue, render_settings,
    };

    /// The recorded demo terminal: 1280x780 at 12.19px per column and 26.33px per row, less 20px of
    /// padding. Every geometry assertion here uses it, so a regression is one a viewer would see.
    const DEMO_WIDTH: u16 = 101;
    const DEMO_HEIGHT: u16 = 28;

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
        assert_eq!(headings.len(), 6, "{headings:?}");

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

    /// A screen with nothing to edit still draws what it has.
    ///
    /// The keyboard has no stop at all here, so the alignment pass has no row to work from. It must
    /// leave the offset alone rather than reach for a row that is not there.
    #[test]
    fn a_screen_with_nothing_to_edit_still_draws() {
        let mut view = prompt_view();
        view.sections.clear();
        view.sections.push(SettingsSection {
            id: SettingsSectionId::Storage,
            notes: Vec::new(),
            fields: vec![Field::read_only(
                "linked",
                "Source",
                FieldOwner::Declared,
                FieldValue::text("/home/ada/brief.md"),
                ReadOnlyReason::ReferenceMode,
            )],
        });
        assert!(view.focusable_keys().is_empty());
        assert_eq!(view.focused(), "");

        let mut session = SettingsScreenSession::default();
        let (terminal, geometry) = draw(&mut session, &view, DEMO_WIDTH, DEMO_HEIGHT);
        let frame = rendered(terminal.backend().buffer());
        assert_eq!(geometry.first_visible, 0);
        assert!(frame.contains("Source: /home/ada/brief.md"), "{frame}");
        assert!(frame.contains("skit doesn't write to this file"), "{frame}");
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
        view.sections.push(SettingsSection {
            id: SettingsSectionId::Basics,
            notes: Vec::new(),
            fields: vec![
                Field::new(
                    "kind:secret",
                    "Secret",
                    FieldKind::Secret,
                    FieldOwner::Declared,
                    FieldValue::text("hunter2"),
                ),
                Field::new(
                    "kind:boolean",
                    "Dry run",
                    FieldKind::Boolean,
                    FieldOwner::Declared,
                    FieldValue::boolean(true),
                ),
                Field::new(
                    "kind:number",
                    "Retries",
                    FieldKind::Number { integer: true },
                    FieldOwner::Declared,
                    FieldValue::Explicit(TypedValue::Integer(3)),
                ),
                Field::new(
                    "kind:arguments",
                    "Extra arguments",
                    FieldKind::ArgumentList {
                        dialect: ArgumentDialect::Posix,
                    },
                    FieldOwner::Declared,
                    FieldValue::Explicit(TypedValue::Arguments(vec!["--force".to_owned()])),
                ),
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
                Field::read_only(
                    "kind:read-only",
                    "Delivery",
                    FieldOwner::Declared,
                    FieldValue::text("flag"),
                    ReadOnlyReason::FixedAtAddTime,
                ),
            ],
        });

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
}
