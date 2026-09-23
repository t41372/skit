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

use ratatui_core::style::{Color, Modifier, Style};
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Theme {
    /// The version 0.4 look: a fixed btop-style palette with a terracotta accent.
    #[default]
    Skit,
}

thread_local! {
    static CURRENT: Cell<Theme> = const { Cell::new(Theme::Skit) };
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
        Theme::Skit => match panel {
            Panel::Library | Panel::Health => BOX_GREEN,
            Panel::Detail | Panel::Settings | Panel::Preferences | Panel::Picker => BOX_INDIGO,
            Panel::Run | Panel::Form | Panel::Add => BOX_MAROON,
            Panel::Dialog => ACCENT,
        },
    }
}

/// The border style of `panel`.
pub(crate) fn panel_border(panel: Panel) -> Style {
    Style::default().fg(panel_color(panel))
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
        Theme::Skit => Style::default().fg(Color::Reset),
    }
}

/// A panel title. Version 0.4: `ansi_bright_white`, bold.
pub(crate) fn title() -> Style {
    match current() {
        Theme::Skit => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    }
}

/// The header row of the library table. Version 0.4: `ansi_bright_white`, bold.
pub(crate) fn table_header() -> Style {
    match current() {
        Theme::Skit => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    }
}

/// A section heading inside a panel. Version 0.4: `$accent`.
pub(crate) fn heading() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    }
}

/// The `required` mark beside a run form field. Version 0.4: `$accent`.
pub(crate) fn required() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    }
}

/// The name of the entry that the detail pane shows. Version 0.4: bold `$accent`.
pub(crate) fn emphasis() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    }
}

/// A hint, a note under a field, or other secondary copy that the port drew in bright black.
/// Version 0.4: `[dim]`.
pub(crate) fn hint() -> Style {
    match current() {
        Theme::Skit => Style::default()
            .fg(Color::Reset)
            .add_modifier(Modifier::DIM),
    }
}

/// Secondary copy that is already dim.
pub(crate) fn muted() -> Style {
    match current() {
        Theme::Skit => Style::default().add_modifier(Modifier::DIM),
    }
}

/// The untyped rest of a path suggestion after the typed text.
pub(crate) fn suggestion() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(Color::DarkGray),
    }
}

/// A scrollbar beside a panel. Version 0.4: `#4A413C` for every panel.
pub(crate) fn scrollbar() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(SCROLLBAR),
    }
}

/// A key name inside copy, such as `Ctrl+N`.
pub(crate) fn key_hint() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(ACCENT),
    }
}

/// A focus marker or a choice glyph.
pub(crate) fn marker() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(ACCENT),
    }
}

/// A notice or a question line in the add flow.
pub(crate) fn notice() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(ACCENT),
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
        Theme::Skit => match status {
            Status::Success => Color::Green,
            Status::Warning => Color::Yellow,
            Status::Danger => Color::Red,
        },
    }
}

pub(crate) fn status(status: Status) -> Style {
    Style::default().fg(status_color(status))
}

/// The border color of an input, a text area, or a select.
pub(crate) fn border_color(focused: bool) -> Color {
    match current() {
        Theme::Skit => {
            if focused {
                ACCENT
            } else {
                BOX_DIM
            }
        }
    }
}

pub(crate) fn border(focused: bool) -> Style {
    Style::default().fg(border_color(focused))
}

/// The selected row of a list, a table, or an option set.
pub(crate) fn selection() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(SELECT_FG).bg(SELECT_BG),
    }
}

/// The cursor cell of a text area.
pub(crate) fn caret(focused: bool) -> Style {
    match current() {
        Theme::Skit => {
            if focused {
                Style::default().fg(Color::Black).bg(ACCENT)
            } else {
                text()
            }
        }
    }
}

/// A list of choices with an arrow on the selected row.
pub(crate) fn list_picker_style() -> ListPickerStyle {
    match current() {
        Theme::Skit => ListPickerStyle {
            selected_style: Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
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
        Theme::Skit => CheckBoxStyle::unicode()
            .focused_fg(ACCENT)
            .unfocused_fg(Color::Reset)
            .checked_fg(Color::Green),
    }
}

pub(crate) fn select_style() -> ratatui_interact::components::SelectStyle {
    match current() {
        Theme::Skit => ratatui_interact::components::SelectStyle {
            focused_border: border_color(true),
            unfocused_border: border_color(false),
            dropdown_border: ACCENT,
            highlight_style: selection(),
            text_fg: Color::Reset,
            option_style: text(),
            ..ratatui_interact::components::SelectStyle::default()
        },
    }
}

pub(crate) fn radio_style() -> ButtonStyle {
    match current() {
        Theme::Skit => ButtonStyle::new(ButtonVariant::Toggle)
            .focused(SELECT_FG, SELECT_BG)
            .unfocused(Color::Reset, Color::Reset)
            .toggled(SELECT_FG, SELECT_BG),
    }
}

/// The style of the selected option of the focused radio group.
///
/// `ratatui-interact` reads the toggled flag before the focused flag, so the focused colour of
/// [`radio_style`] never reaches a selected option. This style puts the focus on the toggled
/// colours instead.
pub(crate) fn focused_radio_style() -> ButtonStyle {
    match current() {
        Theme::Skit => radio_style().toggled(SELECT_FG, ACCENT),
    }
}

/// A button in a confirmation dialog.
pub(crate) fn dialog_button_style() -> ButtonStyle {
    match current() {
        Theme::Skit => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(Color::Black, ACCENT)
            .unfocused(Color::White, BOX_DIM),
    }
}

/// An action button inside a panel, such as a Preferences action.
pub(crate) fn action_button_style() -> ButtonStyle {
    match current() {
        Theme::Skit => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(Color::White, ACCENT)
            .unfocused(Color::White, BOX_DIM),
    }
}

/// A chip on a run form row, such as `▾ insert`.
pub(crate) fn run_chip_style() -> ButtonStyle {
    match current() {
        Theme::Skit => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(Color::White, ACCENT)
            .unfocused(ACCENT, SELECT_BG),
    }
}

/// A command chip in a footer.
pub(crate) fn footer_chip_style() -> ButtonStyle {
    match current() {
        Theme::Skit => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(ACCENT, PILL_BACKGROUND)
            .unfocused(ACCENT, PILL_BACKGROUND),
    }
}

/// The arrow that shows more footer rows above or below.
pub(crate) fn footer_indicator() -> Style {
    match current() {
        Theme::Skit => Style::default().fg(ACCENT),
    }
}

/// A command chip in the action row at the bottom of a health or runner dialog.
pub(crate) fn dialog_footer_chip_style() -> ButtonStyle {
    match current() {
        Theme::Skit => ButtonStyle::new(ButtonVariant::SingleLine)
            .focused(Color::White, BOX_DIM)
            .unfocused(Color::White, BOX_DIM),
    }
}
