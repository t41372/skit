//! The terminal palette shared by all screens and external widgets.

use ratatui_core::style::{Color, Modifier, Style};
use ratatui_widgets::{
    block::{Block, Padding},
    borders::{BorderType, Borders},
};

pub(crate) const ACCENT: Color = Color::Rgb(0xD9, 0x77, 0x57);
pub(crate) const SELECT_BG: Color = Color::Rgb(0x5A, 0x2D, 0x1E);
pub(crate) const SELECT_FG: Color = Color::Rgb(0xEE, 0xEE, 0xEE);
pub(crate) const BOX_GREEN: Color = Color::Rgb(0x3D, 0x7B, 0x46);
pub(crate) const BOX_INDIGO: Color = Color::Rgb(0x4B, 0x44, 0xB0);
pub(crate) const BOX_MAROON: Color = Color::Rgb(0x92, 0x35, 0x35);
pub(crate) const BOX_DIM: Color = Color::Rgb(0x3A, 0x3A, 0x3A);

pub(crate) fn panel_block(title: String, color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .title_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .title(format!(" {title} "))
}

pub(crate) fn padded_panel(title: String, color: Color) -> Block<'static> {
    panel_block(title, color).padding(Padding::horizontal(1))
}
