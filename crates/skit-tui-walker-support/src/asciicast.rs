//! Deterministic asciicast v3 recording for complete styled walker frames.

use std::{fmt::Write as _, io, time::Duration};

use ratatui_core::{
    backend::TestBackend,
    buffer::Buffer,
    layout::{Position, Size},
    style::{Color, Modifier},
};
use serde_json::Value;

use crate::StyledFrameSnapshot;

/// Stable elapsed time represented by one walker checkpoint.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(100);

/// An in-memory asciicast v3 recorder for complete terminal frames.
#[derive(Debug)]
pub struct AsciicastRecorder {
    output: Vec<u8>,
    last_frame: Option<ScreenSnapshot>,
    last_size: Size,
    pending_interval: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScreenSnapshot {
    buffer: Buffer,
    cursor_position: Position,
    cursor_visible: bool,
}

impl AsciicastRecorder {
    /// Create a recorder with one nonzero initial terminal size.
    pub fn new(cols: u16, rows: u16) -> io::Result<Self> {
        validate_size(Size {
            width: cols,
            height: rows,
        })?;
        let mut output = Vec::new();
        write_json_line(
            &mut output,
            &serde_json::json!({"version": 3, "term": {"cols": cols, "rows": rows}}),
        )?;
        Ok(Self {
            output,
            last_frame: None,
            last_size: Size {
                width: cols,
                height: rows,
            },
            pending_interval: Duration::ZERO,
        })
    }

    /// Record one complete `TestBackend` frame.
    ///
    /// The return value is `true` when visible terminal state produced a cast event.
    pub fn record_frame(&mut self, interval: Duration, backend: &TestBackend) -> io::Result<bool> {
        self.record_buffer(
            interval,
            backend.buffer(),
            backend.cursor_position(),
            backend.cursor_visible(),
        )
    }

    /// Record one complete logical buffer and its final cursor state.
    ///
    /// The return value is `true` when visible terminal state produced a cast event.
    pub fn record_buffer(
        &mut self,
        interval: Duration,
        buffer: &Buffer,
        cursor_position: Position,
        cursor_visible: bool,
    ) -> io::Result<bool> {
        let pending_interval = self.pending_interval.saturating_add(interval);
        let frame = ScreenSnapshot {
            buffer: buffer.clone(),
            cursor_position: if cursor_visible {
                cursor_position
            } else {
                Position::ORIGIN
            },
            cursor_visible,
        };
        if self.last_frame.as_ref() == Some(&frame) {
            self.pending_interval = pending_interval;
            return Ok(false);
        }

        let size = frame.buffer.area.as_size();
        validate_size(size)?;
        let ansi = render_ansi_frame(&frame)?;
        let mut events = Vec::new();
        if size != self.last_size {
            write_json_line(
                &mut events,
                &serde_json::json!([
                    pending_interval.as_secs_f64(),
                    "r",
                    format!("{}x{}", size.width, size.height)
                ]),
            )?;
            write_json_line(&mut events, &serde_json::json!([0.0, "o", ansi]))?;
        } else {
            write_json_line(
                &mut events,
                &serde_json::json!([pending_interval.as_secs_f64(), "o", ansi]),
            )?;
        }

        self.output.extend(events);
        self.last_frame = Some(frame);
        self.last_size = size;
        self.pending_interval = Duration::ZERO;
        Ok(true)
    }

    /// Rebuild and record one stored styled-frame snapshot.
    ///
    /// The return value is `true` when visible terminal state produced a cast event.
    pub fn record_snapshot(
        &mut self,
        interval: Duration,
        frame: &StyledFrameSnapshot,
    ) -> io::Result<bool> {
        let buffer = frame.to_buffer().map_err(io::Error::other)?;
        self.record_buffer(
            interval,
            &buffer,
            Position {
                x: frame.cursor_position.x,
                y: frame.cursor_position.y,
            },
            frame.cursor_visible,
        )
    }

    /// Accumulate elapsed time without adding a cast event.
    pub fn skip(&mut self, interval: Duration) {
        self.pending_interval = self.pending_interval.saturating_add(interval);
    }

    /// Return the complete asciicast v3 NDJSON bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.output
    }
}

fn validate_size(size: Size) -> io::Result<()> {
    if size.width == 0 || size.height == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "asciicast terminal size must be nonzero, got {}x{}",
                size.width, size.height
            ),
        ));
    }
    Ok(())
}

fn render_ansi_frame(frame: &ScreenSnapshot) -> io::Result<String> {
    let blank = Buffer::empty(frame.buffer.area);
    let mut output = String::from("\u{1b}[0m\u{1b}[2J\u{1b}[1;1H");
    let mut style = (Color::Reset, Color::Reset, Color::Reset, Modifier::empty());
    let mut last_position = None;
    for (x, y, cell) in blank.diff_iter(&frame.buffer) {
        if !matches!(last_position, Some((last_x, last_y)) if x == last_x + 1 && y == last_y) {
            write_position(&mut output, Position { x, y }).map_err(io::Error::other)?;
        }
        last_position = Some((x, y));

        let next_style = (cell.fg, cell.bg, cell.underline_color, cell.modifier);
        if next_style != style {
            write_style(&mut output, next_style).map_err(io::Error::other)?;
            style = next_style;
        }
        output.push_str(cell.symbol());
    }
    output.push_str("\u{1b}[0m");
    if frame.cursor_visible {
        write_position(&mut output, frame.cursor_position).map_err(io::Error::other)?;
        output.push_str("\u{1b}[?25h");
    } else {
        output.push_str("\u{1b}[?25l");
    }
    Ok(output)
}

fn write_position(output: &mut String, position: Position) -> std::fmt::Result {
    write!(
        output,
        "\u{1b}[{};{}H",
        u32::from(position.y) + 1,
        u32::from(position.x) + 1
    )
}

fn write_style(
    output: &mut String,
    (foreground, background, underline, modifiers): (Color, Color, Color, Modifier),
) -> std::fmt::Result {
    output.push_str("\u{1b}[0m");
    for (modifier, code) in [
        (Modifier::BOLD, 1),
        (Modifier::DIM, 2),
        (Modifier::ITALIC, 3),
        (Modifier::UNDERLINED, 4),
        (Modifier::SLOW_BLINK, 5),
        (Modifier::RAPID_BLINK, 6),
        (Modifier::REVERSED, 7),
        (Modifier::HIDDEN, 8),
        (Modifier::CROSSED_OUT, 9),
    ] {
        if modifiers.contains(modifier) {
            write!(output, "\u{1b}[{code}m")?;
        }
    }

    let mut has_color = false;
    write_color(output, &mut has_color, 38, foreground)?;
    write_color(output, &mut has_color, 48, background)?;
    write_color(output, &mut has_color, 58, underline)?;
    if has_color {
        output.push('m');
    }
    Ok(())
}

fn write_color(
    output: &mut String,
    has_color: &mut bool,
    prefix: u8,
    color: Color,
) -> std::fmt::Result {
    let suffix = match color {
        Color::Reset => return Ok(()),
        Color::Black => "5;0".to_owned(),
        Color::Red => "5;1".to_owned(),
        Color::Green => "5;2".to_owned(),
        Color::Yellow => "5;3".to_owned(),
        Color::Blue => "5;4".to_owned(),
        Color::Magenta => "5;5".to_owned(),
        Color::Cyan => "5;6".to_owned(),
        Color::Gray => "5;7".to_owned(),
        Color::DarkGray => "5;8".to_owned(),
        Color::LightRed => "5;9".to_owned(),
        Color::LightGreen => "5;10".to_owned(),
        Color::LightYellow => "5;11".to_owned(),
        Color::LightBlue => "5;12".to_owned(),
        Color::LightMagenta => "5;13".to_owned(),
        Color::LightCyan => "5;14".to_owned(),
        Color::White => "5;15".to_owned(),
        Color::Rgb(red, green, blue) => format!("2;{red};{green};{blue}"),
        Color::Indexed(index) => format!("5;{index}"),
    };
    if *has_color {
        output.push(';');
    } else {
        output.push_str("\u{1b}[");
        *has_color = true;
    }
    write!(output, "{prefix};{suffix}")
}

fn write_json_line(output: &mut Vec<u8>, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *output, value).map_err(io::Error::other)?;
    output.push(b'\n');
    Ok(())
}

#[cfg(test)]
mod tests;
