//! The color roles that every screen and external widget draws with.
//!
//! A screen names a role, never a color. Each role resolves through the theme of the frame being
//! drawn. `render_with_session` sets that theme for the duration of one frame, and a frame drawn
//! without a session (a unit test on a `TestBackend`) uses the `skit` theme.
//!
//! Roles are split by their version 0.4 counterpart even where today's values are equal, so a
//! parity repair changes one role instead of every site. `docs/design/terminal-palette.md` lists
//! the counterpart of each role.

use std::cell::Cell;

use ratatui_core::text::{Line, Span};
use ratatui_core::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use unicode_width::UnicodeWidthStr as _;

use crate::appearance::ColorDepth;
use ratatui_interact::components::{ButtonStyle, ButtonVariant, CheckBoxStyle, ListPickerStyle};
use ratatui_widgets::{
    block::{Block, Padding},
    borders::{BorderType, Borders},
};

// Unit tests compare rendered cells with these values. Production code names a role instead.
pub(crate) const ACCENT: Color = Color::Rgb(0xD9, 0x77, 0x57);
pub(crate) const SELECT_BG: Color = Color::Rgb(0x5A, 0x2D, 0x1E);
pub(crate) const SELECT_FG: Color = Color::Rgb(0xEE, 0xEE, 0xEE);
pub(crate) const BOX_GREEN: Color = Color::Rgb(0x3D, 0x7B, 0x46);
pub(crate) const BOX_INDIGO: Color = Color::Rgb(0x4B, 0x44, 0xB0);
pub(crate) const BOX_MAROON: Color = Color::Rgb(0x92, 0x35, 0x35);
pub(crate) const BOX_DIM: Color = Color::Rgb(0x3A, 0x3A, 0x3A);
const PILL_BACKGROUND: Color = Color::Rgb(0x2A, 0x21, 0x1C);
/// Version 0.4's scrollbar color (`theme.py:100`).
const SCROLLBAR: Color = Color::Rgb(0x4A, 0x41, 0x3C);

/// The palette that a frame draws with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Theme {
    /// The version 0.4 look: a fixed btop-style palette with a terracotta accent, in the forms
    /// that the terminal's color depth can show.
    Skit(ColorDepth),
    /// The terminal's own palette. Attributes carry every state, and the accent hue, when the
    /// background is known, colors only borders, glyphs, and markers.
    Terminal(Option<Color>),
}

impl Default for Theme {
    fn default() -> Self {
        Self::Skit(ColorDepth::TrueColor)
    }
}

thread_local! {
    static CURRENT: Cell<Theme> = const { Cell::new(Theme::Skit(ColorDepth::TrueColor)) };
}

/// The theme of the frame being drawn on this thread.
fn current() -> Theme {
    CURRENT.with(Cell::get)
}

/// Draw with `theme` until the scope ends, then restore the previous theme.
///
/// The restore also runs when a render panics, so a later frame cannot inherit the wrong theme.
pub(crate) struct ThemeScope {
    previous: Theme,
}

impl ThemeScope {
    pub(crate) fn enter(theme: Theme) -> Self {
        Self {
            previous: CURRENT.with(|current| current.replace(theme)),
        }
    }
}

impl Drop for ThemeScope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.previous));
    }
}

/// A fixed `skit` color in the form that the frame's color depth can show.
///
/// The 256-color and 16-color forms are Rich 15.0.0's own conversions (`color.py:512-568`), which
/// version 0.4 applied to the same colors. The census checks that no 24-bit color reaches a
/// terminal of lower depth.
fn fixed(color: Color) -> Color {
    let Theme::Skit(depth) = current() else {
        return color;
    };
    match depth {
        ColorDepth::TrueColor => color,
        ColorDepth::EightBit => match color {
            ACCENT => Color::Indexed(173),
            SELECT_BG => Color::Indexed(52),
            SELECT_FG => Color::Indexed(254),
            BOX_GREEN => Color::Indexed(65),
            BOX_INDIGO => Color::Indexed(61),
            BOX_MAROON => Color::Indexed(95),
            BOX_DIM => Color::Indexed(237),
            PILL_BACKGROUND => Color::Indexed(16),
            SCROLLBAR => Color::Indexed(238),
            other => other,
        },
        ColorDepth::Standard => match color {
            ACCENT => Color::LightRed,
            SELECT_BG | BOX_GREEN | BOX_DIM | SCROLLBAR => Color::DarkGray,
            SELECT_FG => Color::White,
            BOX_INDIGO => Color::LightBlue,
            BOX_MAROON => Color::Yellow,
            PILL_BACKGROUND => Color::Black,
            other => other,
        },
    }
}

/// A bordered panel. Each panel has its own border tint in the `skit` theme.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Panel {
    Library,
    Detail,
    Health,
    Settings,
    Preferences,
    Picker,
    Run,
    Form,
    Add,
    Dialog,
}

/// The border color of `panel`.
pub(crate) fn panel_color(panel: Panel) -> Color {
    match current() {
        Theme::Skit(_) => match panel {
            Panel::Library | Panel::Health => fixed(BOX_GREEN),
            Panel::Detail | Panel::Settings | Panel::Preferences | Panel::Picker => {
                fixed(BOX_INDIGO)
            }
            Panel::Run | Panel::Form | Panel::Add => fixed(BOX_MAROON),
            Panel::Dialog => fixed(ACCENT),
        },
        Theme::Terminal(accent) => match panel {
            Panel::Dialog => accent.unwrap_or(Color::Reset),
            _ => Color::Reset,
        },
    }
}

/// The border style of `panel`.
///
/// The terminal theme dims every panel border except a dialog's, which holds the focus.
pub(crate) fn panel_border(panel: Panel) -> Style {
    match (current(), panel) {
        (Theme::Terminal(_), Panel::Dialog) | (Theme::Skit(_), _) => {
            Style::default().fg(panel_color(panel))
        }
        (Theme::Terminal(_), _) => dim_text(),
    }
}

pub(crate) fn panel_block(label: String, panel: Panel) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(panel_border(panel))
        .title_style(title())
        .title(format!(" {label} "))
}

pub(crate) fn padded_panel(label: String, panel: Panel) -> Block<'static> {
    panel_block(label, panel).padding(Padding::horizontal(1))
}

/// Body text, list items, labels, and typed values. Version 0.4: `ansi_default`.
///
/// The color is an explicit reset, not an absent one, so the text keeps the default foreground
/// inside a styled parent, as the bright white it replaces did.
pub(crate) fn text() -> Style {
    match current() {
        Theme::Skit(_) => Style::default().fg(Color::Reset),
        Theme::Terminal(_) => Style::default().fg(Color::Reset),
    }
}

/// A panel title. Version 0.4: `ansi_bright_white`, bold.
pub(crate) fn title() -> Style {
    match current() {
        Theme::Skit(_) => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        Theme::Terminal(_) => bold_text(),
    }
}

/// The header row of the library table. Version 0.4: `ansi_bright_white`, bold.
pub(crate) fn table_header() -> Style {
    match current() {
        Theme::Skit(_) => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        Theme::Terminal(_) => bold_text(),
    }
}

/// A title drawn on a border. Ratatui draws a title over border cells, so a title without a
/// color of its own shows the border color; the terminal theme keeps it in the default foreground.
pub(crate) fn border_title() -> Style {
    match current() {
        Theme::Skit(_) => Style::default(),
        Theme::Terminal(_) => Style::default().fg(Color::Reset),
    }
}

/// A section heading inside a panel. Version 0.4: `$accent`.
pub(crate) fn heading() -> Style {
    match current() {
        Theme::Skit(_) => Style::default()
            .fg(fixed(ACCENT))
            .add_modifier(Modifier::BOLD),
        Theme::Terminal(_) => bold_text(),
    }
}

/// The `required` mark beside a run form field. Version 0.4: `$accent`.
pub(crate) fn required() -> Style {
    match current() {
        Theme::Skit(_) => Style::default()
            .fg(fixed(ACCENT))
            .add_modifier(Modifier::BOLD),
        Theme::Terminal(_) => bold_text(),
    }
}

/// The name of the entry that the detail pane shows. Version 0.4: bold `$accent`.
pub(crate) fn emphasis() -> Style {
    match current() {
        Theme::Skit(_) => Style::default()
            .fg(fixed(ACCENT))
            .add_modifier(Modifier::BOLD),
        Theme::Terminal(_) => bold_text(),
    }
}

/// A hint, a note under a field, or other secondary copy that the port drew in bright black.
/// Version 0.4: `[dim]`.
pub(crate) fn hint() -> Style {
    match current() {
        Theme::Skit(_) => Style::default()
            .fg(Color::Reset)
            .add_modifier(Modifier::DIM),
        Theme::Terminal(_) => dim_text(),
    }
}

/// Secondary copy that is already dim.
pub(crate) fn muted() -> Style {
    match current() {
        Theme::Skit(_) => Style::default().add_modifier(Modifier::DIM),
        Theme::Terminal(_) => Style::default().add_modifier(Modifier::DIM),
    }
}

/// The untyped rest of a path suggestion after the typed text.
pub(crate) fn suggestion() -> Style {
    match current() {
        Theme::Skit(_) => Style::default().fg(Color::DarkGray),
        Theme::Terminal(_) => dim_text(),
    }
}

/// A scrollbar beside a panel. Version 0.4: `#4A413C` for every panel.
pub(crate) fn scrollbar() -> Style {
    match current() {
        Theme::Skit(_) => Style::default().fg(fixed(SCROLLBAR)),
        Theme::Terminal(_) => dim_text(),
    }
}

/// A key name inside copy, such as `Ctrl+N`.
pub(crate) fn key_hint() -> Style {
    match current() {
        Theme::Skit(_) => Style::default().fg(fixed(ACCENT)),
        Theme::Terminal(_) => bold_text(),
    }
}

/// A focus marker or a choice glyph.
pub(crate) fn marker() -> Style {
    match current() {
        Theme::Skit(_) => Style::default().fg(fixed(ACCENT)),
        Theme::Terminal(accent) => {
            accent.map_or_else(bold_text, |accent| Style::default().fg(accent))
        }
    }
}

/// A notice or a question line in the add flow.
pub(crate) fn notice() -> Style {
    match current() {
        Theme::Skit(_) => Style::default().fg(fixed(ACCENT)),
        Theme::Terminal(_) => Style::default().fg(Color::Reset),
    }
}

/// The outcome that a status line reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Status {
    Success,
    Warning,
    Danger,
}

/// The color of a status line or glyph.
pub(crate) fn status_color(status: Status) -> Color {
    match current() {
        Theme::Skit(_) => match status {
            Status::Success => Color::Green,
            Status::Warning => Color::Yellow,
            Status::Danger => Color::Red,
        },
        Theme::Terminal(_) => match status {
            Status::Success => Color::Green,
            Status::Warning => Color::Yellow,
            Status::Danger => Color::Red,
        },
    }
}

/// Status text. The terminal theme keeps the text in the default foreground, because green and
/// yellow are unreadable on light backgrounds; a danger line is bold instead.
pub(crate) fn status(status: Status) -> Style {
    match current() {
        Theme::Skit(_) => Style::default().fg(status_color(status)),
        Theme::Terminal(_) => match status {
            Status::Danger => bold_text(),
            Status::Success | Status::Warning => Style::default().fg(Color::Reset),
        },
    }
}

/// A status glyph: ✓, ✗, ⚠, or →.
pub(crate) fn status_glyph(status: Status) -> Style {
    Style::default().fg(status_color(status))
}

/// The spans of a status line. The terminal theme colors only a leading glyph and keeps the
/// text in the status text style.
pub(crate) fn status_spans(line: String, status: Status) -> Vec<Span<'static>> {
    if matches!(current(), Theme::Terminal(_))
        && let Some(rest) = ["✓ ", "✗ ", "⚠ ", "→ "]
            .iter()
            .find_map(|glyph| line.strip_prefix(glyph).map(|rest| (*glyph, rest)))
    {
        let (glyph, rest) = rest;
        return vec![
            Span::styled(glyph, status_glyph(status)),
            Span::styled(rest.to_owned(), self::status(status)),
        ];
    }
    vec![Span::styled(line, self::status(status))]
}

/// A status line with the line style of `Line::styled`, split as [`status_spans`] splits it.
pub(crate) fn status_line(line: String, status: Status) -> Line<'static> {
    match current() {
        Theme::Skit(_) => Line::styled(line, self::status(status)),
        Theme::Terminal(_) => Line::from(status_spans(line, status)),
    }
}

/// The border color of an input, a text area, or a select.
pub(crate) fn border_color(focused: bool) -> Color {
    match current() {
        Theme::Skit(_) => {
            if focused {
                fixed(ACCENT)
            } else {
                fixed(BOX_DIM)
            }
        }
        Theme::Terminal(accent) => {
            if focused {
                accent.unwrap_or(Color::Reset)
            } else {
                Color::Reset
            }
        }
    }
}

pub(crate) fn border(focused: bool) -> Style {
    match current() {
        Theme::Terminal(_) if !focused => dim_text(),
        Theme::Skit(_) | Theme::Terminal(_) => Style::default().fg(border_color(focused)),
    }
}

/// The selected row of a list, a table, or an option set.
pub(crate) fn selection() -> Style {
    match current() {
        Theme::Skit(_) => Style::default().fg(fixed(SELECT_FG)).bg(fixed(SELECT_BG)),
        Theme::Terminal(_) => Style::default().add_modifier(Modifier::REVERSED),
    }
}

/// The cursor cell of a text area.
pub(crate) fn caret(focused: bool) -> Style {
    match current() {
        Theme::Skit(_) => {
            if focused {
                Style::default().fg(Color::Black).bg(fixed(ACCENT))
            } else {
                text()
            }
        }
        Theme::Terminal(_) => {
            if focused {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                text()
            }
        }
    }
}

/// A list of choices with an arrow on the selected row.
pub(crate) fn list_picker_style() -> ListPickerStyle {
    match current() {
        Theme::Skit(_) => ListPickerStyle {
            selected_style: Style::default()
                .fg(Color::Black)
                .bg(fixed(ACCENT))
                .add_modifier(Modifier::BOLD),
            normal_style: text(),
            indicator_style: marker(),
            border_style: Style::default(),
            indicator: "▶ ",
            indicator_empty: "  ",
            bordered: false,
        },
        Theme::Terminal(_) => ListPickerStyle {
            selected_style: Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            normal_style: text(),
            indicator_style: marker(),
            border_style: Style::default(),
            indicator: "▶ ",
            indicator_empty: "  ",
            bordered: false,
        },
    }
}

pub(crate) fn checkbox_style() -> CheckBoxStyle {
    match current() {
        Theme::Skit(_) => CheckBoxStyle::unicode()
            .focused_fg(fixed(ACCENT))
            .unfocused_fg(Color::Reset)
            .checked_fg(Color::Green),
        Theme::Terminal(_) => CheckBoxStyle {
            disabled_fg: Color::Reset,
            ..CheckBoxStyle::unicode()
                .focused_fg(Color::Reset)
                .unfocused_fg(Color::Reset)
                .checked_fg(Color::Reset)
        },
    }
}

pub(crate) fn select_style() -> ratatui_interact::components::SelectStyle {
    match current() {
        Theme::Skit(_) => ratatui_interact::components::SelectStyle {
            focused_border: border_color(true),
            unfocused_border: border_color(false),
            dropdown_border: fixed(ACCENT),
            highlight_style: selection(),
            text_fg: Color::Reset,
            option_style: text(),
            ..ratatui_interact::components::SelectStyle::default()
        },
        Theme::Terminal(accent) => ratatui_interact::components::SelectStyle {
            focused_border: accent.unwrap_or(Color::Reset),
            unfocused_border: Color::Reset,
            disabled_border: Color::Reset,
            dropdown_border: accent.unwrap_or(Color::Reset),
            highlight_style: selection(),
            text_fg: Color::Reset,
            placeholder_fg: Color::Reset,
            option_style: text(),
            ..ratatui_interact::components::SelectStyle::default()
        },
    }
}

/// The storage and runner selects of the add review. The `skit` theme keeps the default colors
/// of `ratatui-interact` there, as the port drew them.
pub(crate) fn review_select_style() -> ratatui_interact::components::SelectStyle {
    match current() {
        Theme::Skit(_) => ratatui_interact::components::SelectStyle::default(),
        Theme::Terminal(_) => select_style(),
    }
}

/// The candidate check boxes of the add review. The `skit` theme keeps the default colors of
/// `ratatui-interact` there, as the port drew them.
pub(crate) fn review_checkbox_style() -> CheckBoxStyle {
    match current() {
        Theme::Skit(_) => CheckBoxStyle::default(),
        Theme::Terminal(_) => CheckBoxStyle {
            focused_fg: Color::Reset,
            unfocused_fg: Color::Reset,
            disabled_fg: Color::Reset,
            checked_fg: Color::Reset,
            ..CheckBoxStyle::default()
        },
    }
}

pub(crate) fn radio_style() -> ButtonStyle {
    match current() {
        Theme::Skit(_) => ButtonStyle::new(ButtonVariant::Toggle)
            .focused(fixed(SELECT_FG), fixed(SELECT_BG))
            .unfocused(Color::Reset, Color::Reset)
            .toggled(fixed(SELECT_FG), fixed(SELECT_BG)),
        Theme::Terminal(_) => plain_button(ButtonVariant::Toggle),
    }
}

/// The style of the selected option of the focused radio group.
///
/// `ratatui-interact` reads the toggled flag before the focused flag, so the focused colour of
/// [`radio_style`] never reaches a selected option. This style puts the focus on the toggled
/// colours instead.
pub(crate) fn focused_radio_style() -> ButtonStyle {
    match current() {
        Theme::Skit(_) => radio_style().toggled(fixed(SELECT_FG), fixed(ACCENT)),
        Theme::Terminal(_) => plain_button(ButtonVariant::Toggle),
    }
}

/// A button in a confirmation dialog.
pub(crate) fn dialog_button_style() -> ButtonStyle {
    match current() {
        Theme::Skit(_) => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(Color::Black, fixed(ACCENT))
            .unfocused(Color::White, fixed(BOX_DIM)),
        Theme::Terminal(_) => plain_button(ButtonVariant::SingleLine),
    }
}

/// An action button inside a panel, such as a Preferences action.
pub(crate) fn action_button_style() -> ButtonStyle {
    match current() {
        Theme::Skit(_) => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(Color::White, fixed(ACCENT))
            .unfocused(Color::White, fixed(BOX_DIM)),
        Theme::Terminal(_) => plain_button(ButtonVariant::SingleLine),
    }
}

/// A chip on a run form row, such as `▾ insert`.
pub(crate) fn run_chip_style() -> ButtonStyle {
    match current() {
        Theme::Skit(_) => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(Color::White, fixed(ACCENT))
            .unfocused(fixed(ACCENT), fixed(SELECT_BG)),
        Theme::Terminal(_) => plain_button(ButtonVariant::SingleLine),
    }
}

/// A command chip in a footer.
pub(crate) fn footer_chip_style() -> ButtonStyle {
    match current() {
        Theme::Skit(_) => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(fixed(ACCENT), fixed(PILL_BACKGROUND))
            .unfocused(fixed(ACCENT), fixed(PILL_BACKGROUND)),
        Theme::Terminal(_) => plain_button(ButtonVariant::SingleLine),
    }
}

/// The arrow that shows more footer rows above or below.
pub(crate) fn footer_indicator() -> Style {
    match current() {
        Theme::Skit(_) => Style::default().fg(fixed(ACCENT)),
        Theme::Terminal(accent) => Style::default().fg(accent.unwrap_or(Color::Reset)),
    }
}

/// A command chip in the action row at the bottom of a health or runner dialog.
pub(crate) fn dialog_footer_chip_style() -> ButtonStyle {
    match current() {
        Theme::Skit(_) => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(Color::White, fixed(BOX_DIM))
            .unfocused(Color::White, fixed(BOX_DIM)),
        Theme::Terminal(_) => plain_button(ButtonVariant::SingleLine),
    }
}

fn bold_text() -> Style {
    Style::default()
        .fg(Color::Reset)
        .add_modifier(Modifier::BOLD)
}

fn dim_text() -> Style {
    Style::default()
        .fg(Color::Reset)
        .add_modifier(Modifier::DIM)
}

/// A button with the terminal's own colors. A post-pass shows its state with attributes.
fn plain_button(variant: ButtonVariant) -> ButtonStyle {
    ButtonStyle {
        disabled_fg: Color::Reset,
        pressed_fg: Color::Reset,
        pressed_bg: Color::Reset,
        ..ButtonStyle::new(variant)
            .focused(Color::Reset, Color::Reset)
            .unfocused(Color::Reset, Color::Reset)
            .toggled(Color::Reset, Color::Reset)
    }
}

/// Show focus on a widget that can only take colors: reverse its area in the terminal theme.
pub(crate) fn patch_focus(buffer: &mut Buffer, area: Rect, focused: bool) {
    if focused && matches!(current(), Theme::Terminal(_)) {
        buffer.set_style(area, Style::default().add_modifier(Modifier::REVERSED));
    }
}

/// Remove the colors that a third-party widget chose for itself, in the terminal theme.
pub(crate) fn patch_plain(buffer: &mut Buffer, area: Rect) {
    if matches!(current(), Theme::Terminal(_)) {
        buffer.set_style(area, Style::default().fg(Color::Reset).bg(Color::Reset));
    }
}

/// Draw the key of a footer chip as a keycap in the terminal theme.
///
/// A chip shows ` key label `. The keycap covers ` key ` and takes the accent, or the default
/// foreground when the background is unknown, reversed. The label stays plain.
pub(crate) fn patch_chip_key(buffer: &mut Buffer, area: Rect, key: &str) {
    if let Theme::Terminal(accent) = current() {
        buffer.set_style(area, Style::default().remove_modifier(Modifier::BOLD));
        let width = u16::try_from(key.width().saturating_add(2))
            .unwrap_or(u16::MAX)
            .min(area.width);
        buffer.set_style(
            Rect::new(area.x, area.y, width, area.height),
            Style::default()
                .fg(accent.unwrap_or(Color::Reset))
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        );
    }
}

/// Dim the border of an idle bordered widget that can only take a border color, in the terminal
/// theme.
pub(crate) fn patch_idle_border(buffer: &mut Buffer, area: Rect, focused: bool) {
    if focused || !matches!(current(), Theme::Terminal(_)) || area.width == 0 || area.height == 0 {
        return;
    }
    let dim = Style::default().add_modifier(Modifier::DIM);
    let right = area.right().saturating_sub(1);
    let bottom = area.bottom().saturating_sub(1);
    for x in area.left()..area.right() {
        buffer.set_style(Rect::new(x, area.y, 1, 1), dim);
        buffer.set_style(Rect::new(x, bottom, 1, 1), dim);
    }
    for y in area.top()..area.bottom() {
        buffer.set_style(Rect::new(area.x, y, 1, 1), dim);
        buffer.set_style(Rect::new(right, y, 1, 1), dim);
    }
}

/// Keep the title on the top border of `area` in the default foreground, in the terminal theme.
///
/// This serves a third-party dialog that draws its own title over a colored border.
pub(crate) fn patch_border_title(buffer: &mut Buffer, area: Rect) {
    if !matches!(current(), Theme::Terminal(_)) || area.width == 0 || area.height == 0 {
        return;
    }
    for x in area.left()..area.right() {
        let cell = &mut buffer[(x, area.y)];
        if cell.symbol().chars().any(char::is_alphanumeric) {
            cell.fg = Color::Reset;
        }
    }
}
