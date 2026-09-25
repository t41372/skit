//! How a session paints color on the terminal.

use crate::theme::Theme;
use ratatui_core::{
    backend::{Backend, ClearType, WindowSize},
    buffer::Cell,
    layout::{Position, Size},
    style::Color,
};
use skit_application::preferences::AccentChoice;

/// How many colors the terminal can show.
///
/// The composition root decides it the way version 0.4 does, through Rich's color-system
/// detection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColorDepth {
    /// 24-bit color.
    #[default]
    TrueColor,
    /// The xterm 256-color palette.
    EightBit,
    /// The 16 standard ANSI colors.
    Standard,
}

/// The palette that the user picked with the `theme` setting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThemeName {
    /// The version 0.4 look: a fixed palette with a terracotta accent.
    #[default]
    Skit,
    /// The terminal's own palette.
    Terminal,
}

/// Whether the terminal background is dark or light, when the terminal says.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Background {
    /// The terminal did not answer, or the question was not asked.
    #[default]
    Unknown,
    /// A dark background.
    Dark,
    /// A light background.
    Light,
}

/// The color choices of one terminal session.
///
/// The composition root builds this value. The terminal adapter never reads the environment
/// itself.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Appearance {
    no_color: bool,
    depth: ColorDepth,
    theme: ThemeName,
    background: Background,
    accent: AccentChoice,
}

impl Appearance {
    /// Paint no color when `no_color` is true.
    ///
    /// Every cell then uses the terminal's default foreground, background, and underline color,
    /// and keeps its bold, dim, and reverse attributes. Version 0.4 applies the same filter to its
    /// output (Textual `NoColor`).
    #[must_use]
    pub const fn with_no_color(self, no_color: bool) -> Self {
        Self { no_color, ..self }
    }

    /// Draw the fixed colors of the `skit` theme in the forms that `depth` can show.
    #[must_use]
    pub const fn with_color_depth(self, depth: ColorDepth) -> Self {
        Self { depth, ..self }
    }

    /// Draw with the palette that the `theme` setting names.
    #[must_use]
    pub const fn with_theme(self, theme: ThemeName) -> Self {
        Self { theme, ..self }
    }

    /// Pick the accent of the terminal theme for this background.
    #[must_use]
    pub const fn with_background(self, background: Background) -> Self {
        Self { background, ..self }
    }

    /// Draw the terminal theme with the hue that the `accent` setting names.
    #[must_use]
    pub const fn with_accent(self, accent: AccentChoice) -> Self {
        Self { accent, ..self }
    }

    /// The theme that frames of this session draw with.
    ///
    /// With `accent = auto`, the terminal theme takes cyan on a dark background and magenta on a
    /// light one, the ANSI hues that stay readable on every default profile of that kind
    /// (`docs/design/terminal-palette.md`), and no hue on an unknown background. Any other accent
    /// names its hue, or no hue, whatever the background.
    pub(crate) const fn theme(self) -> Theme {
        match self.theme {
            ThemeName::Skit => Theme::Skit(self.depth),
            ThemeName::Terminal => Theme::Terminal(match (self.accent, self.background) {
                (AccentChoice::Auto, Background::Dark) | (AccentChoice::Cyan, _) => {
                    Some(Color::Cyan)
                }
                (AccentChoice::Auto, Background::Light) | (AccentChoice::Magenta, _) => {
                    Some(Color::Magenta)
                }
                (AccentChoice::Auto, Background::Unknown) | (AccentChoice::None, _) => None,
                (AccentChoice::Red, _) => Some(Color::Red),
                (AccentChoice::Green, _) => Some(Color::Green),
                (AccentChoice::Yellow, _) => Some(Color::Yellow),
                (AccentChoice::Blue, _) => Some(Color::Blue),
                (AccentChoice::BrightRed, _) => Some(Color::LightRed),
                (AccentChoice::BrightGreen, _) => Some(Color::LightGreen),
                (AccentChoice::BrightYellow, _) => Some(Color::LightYellow),
                (AccentChoice::BrightBlue, _) => Some(Color::LightBlue),
                (AccentChoice::BrightMagenta, _) => Some(Color::LightMagenta),
                (AccentChoice::BrightCyan, _) => Some(Color::LightCyan),
            }),
        }
    }

    /// Wrap the backend that a session draws on, so its output follows this appearance.
    pub(crate) const fn backend<B>(self, inner: B) -> AppearanceBackend<B> {
        AppearanceBackend {
            inner,
            appearance: self,
        }
    }
}

/// A backend that applies an [`Appearance`] to the cells it writes.
///
/// Ratatui compares frames before the filter, with their colors. The terminal therefore receives
/// the same cell writes as in a colored session, only without the colors. The filter also reaches
/// the colors that third-party widgets choose for themselves.
pub(crate) struct AppearanceBackend<B> {
    inner: B,
    appearance: Appearance,
}

impl<B: Backend> Backend for AppearanceBackend<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        if !self.appearance.no_color {
            return self.inner.draw(content);
        }
        let cells = content
            .map(|(x, y, cell)| {
                let mut cell = cell.clone();
                cell.fg = Color::Reset;
                cell.bg = Color::Reset;
                cell.underline_color = Color::Reset;
                (x, y, cell)
            })
            .collect::<Vec<_>>();
        self.inner
            .draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use ratatui_core::{
        backend::{Backend, TestBackend},
        layout::{Position, Size},
    };

    use super::Appearance;

    /// The forwarders that no full-screen session calls reach the inner backend unchanged.
    ///
    /// `docs/design/terminal-palette.md` (phase 1) lists the ways a forwarder can fail; the
    /// comments name the case each check stops.
    #[test]
    fn forwarders_reach_the_inner_backend() {
        for no_color in [false, true] {
            let mut backend = Appearance::default()
                .with_no_color(no_color)
                .backend(TestBackend::with_lines(["ab", "cd", "ef"]));

            // Case 3: the inner backend's size, not a value of the wrapper's own.
            let size = backend.window_size().unwrap();
            assert_eq!(size.columns_rows, Size::new(2, 3));
            assert_eq!(size.pixels, backend.inner.window_size().unwrap().pixels);

            // Cases 2 and 4: two lines appended on the last row scroll two rows away.
            backend.set_cursor_position(Position::new(0, 2)).unwrap();
            backend.append_lines(2).unwrap();
            backend.inner.assert_scrollback_lines(["ab", "cd"]);

            // Case 1: the whole screen is empty, also the row that the scroll kept.
            backend.clear().unwrap();
            backend.inner.assert_buffer_lines(["  ", "  ", "  "]);
        }
    }
}
