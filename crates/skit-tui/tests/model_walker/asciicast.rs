use std::{fmt::Write as _, io, time::Duration};

use ratatui_core::{
    backend::{Backend, TestBackend},
    buffer::Buffer,
    layout::{Position, Size},
    style::{Color, Modifier, Style},
    terminal::Terminal,
    text::{Line, Span},
};
use ratatui_widgets::paragraph::Paragraph;
use serde_json::Value;

#[derive(Debug)]
pub(super) struct AsciicastRecorder {
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
    pub(super) fn new(cols: u16, rows: u16) -> io::Result<Self> {
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

    pub(super) fn record_frame(
        &mut self,
        interval: Duration,
        backend: &TestBackend,
    ) -> io::Result<bool> {
        let pending_interval = self.pending_interval.saturating_add(interval);
        let cursor_visible = backend.cursor_visible();
        let frame = ScreenSnapshot {
            buffer: backend.buffer().clone(),
            cursor_position: if cursor_visible {
                backend.cursor_position()
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

    pub(super) fn as_bytes(&self) -> &[u8] {
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
    if color == Color::Reset {
        return Ok(());
    }
    if *has_color {
        output.push(';');
    } else {
        output.push_str("\u{1b}[");
        *has_color = true;
    }
    write!(output, "{prefix};")?;
    match color {
        Color::Reset => unreachable!("reset colors return before serialization"),
        Color::Black => output.push_str("5;0"),
        Color::Red => output.push_str("5;1"),
        Color::Green => output.push_str("5;2"),
        Color::Yellow => output.push_str("5;3"),
        Color::Blue => output.push_str("5;4"),
        Color::Magenta => output.push_str("5;5"),
        Color::Cyan => output.push_str("5;6"),
        Color::Gray => output.push_str("5;7"),
        Color::DarkGray => output.push_str("5;8"),
        Color::LightRed => output.push_str("5;9"),
        Color::LightGreen => output.push_str("5;10"),
        Color::LightYellow => output.push_str("5;11"),
        Color::LightBlue => output.push_str("5;12"),
        Color::LightMagenta => output.push_str("5;13"),
        Color::LightCyan => output.push_str("5;14"),
        Color::White => output.push_str("5;15"),
        Color::Rgb(red, green, blue) => write!(output, "2;{red};{green};{blue}")?,
        Color::Indexed(index) => write!(output, "5;{index}")?,
    }
    Ok(())
}

fn write_json_line(output: &mut Vec<u8>, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *output, value).map_err(io::Error::other)?;
    output.push(b'\n');
    Ok(())
}

fn terminal<'a>(
    width: u16,
    height: u16,
    line: Line<'a>,
    cursor: Option<(u16, u16)>,
) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(Paragraph::new(line), frame.area());
            if let Some(cursor) = cursor {
                frame.set_cursor_position(cursor);
            }
        })
        .unwrap();
    terminal
}

fn json_lines(recorder: &AsciicastRecorder) -> Vec<Value> {
    recorder
        .as_bytes()
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

fn output_data(event: &Value) -> &str {
    event.as_array().unwrap()[2].as_str().unwrap()
}

#[test]
fn rejects_zero_sized_headers_and_resize_frames() {
    for (cols, rows) in [(0, 1), (1, 0), (0, 0)] {
        let error = AsciicastRecorder::new(cols, rows).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    let mut recorder = AsciicastRecorder::new(1, 1).unwrap();
    let before = recorder.as_bytes().to_vec();
    for backend in [TestBackend::new(0, 1), TestBackend::new(1, 0)] {
        let error = recorder
            .record_frame(Duration::from_millis(750), &backend)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            recorder.as_bytes(),
            before,
            "a refused resize must be atomic"
        );
    }

    let smallest = terminal(1, 1, Line::from("x"), None);
    assert!(
        recorder
            .record_frame(Duration::from_millis(125), smallest.backend())
            .unwrap(),
        "the recorder accepts the smallest nonzero terminal"
    );
    let lines = json_lines(&recorder);
    assert_eq!(
        lines[1].as_array().unwrap()[0],
        serde_json::json!(0.125),
        "a refused resize must not leak its interval into the next frame"
    );
}

#[test]
fn writes_v3_ndjson_with_relative_intervals_and_deduplicates_unchanged_frames() {
    let mut recorder = AsciicastRecorder::new(4, 2).unwrap();
    let first = terminal(4, 2, Line::from("A"), None);

    assert!(
        recorder
            .record_frame(Duration::from_millis(125), first.backend())
            .unwrap()
    );
    assert!(
        !recorder
            .record_frame(Duration::from_millis(250), first.backend())
            .unwrap()
    );

    let second = terminal(4, 2, Line::from("B"), None);
    assert!(
        recorder
            .record_frame(Duration::from_millis(375), second.backend())
            .unwrap()
    );

    let lines = json_lines(&recorder);
    assert_eq!(
        lines[0],
        serde_json::json!({"version": 3, "term": {"cols": 4, "rows": 2}})
    );
    assert_eq!(lines.len(), 3, "the unchanged screen must emit no event");
    assert_eq!(lines[1].as_array().unwrap()[0], serde_json::json!(0.125));
    assert_eq!(lines[1].as_array().unwrap()[1], serde_json::json!("o"));
    assert_eq!(lines[2].as_array().unwrap()[0], serde_json::json!(0.625));
    assert_eq!(lines[2].as_array().unwrap()[1], serde_json::json!("o"));

    let mut same_walk = AsciicastRecorder::new(4, 2).unwrap();
    same_walk
        .record_frame(Duration::from_millis(125), first.backend())
        .unwrap();
    same_walk
        .record_frame(Duration::from_millis(250), first.backend())
        .unwrap();
    same_walk
        .record_frame(Duration::from_millis(375), second.backend())
        .unwrap();
    assert_eq!(recorder.as_bytes(), same_walk.as_bytes());
}

#[test]
fn emits_resize_before_a_zero_interval_full_frame() {
    let mut recorder = AsciicastRecorder::new(4, 2).unwrap();
    let resized = terminal(6, 3, Line::from("resized"), None);

    recorder
        .record_frame(Duration::from_millis(50), resized.backend())
        .unwrap();

    let lines = json_lines(&recorder);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1], serde_json::json!([0.05, "r", "6x3"]));
    assert_eq!(lines[2].as_array().unwrap()[0], serde_json::json!(0.0));
    assert_eq!(lines[2].as_array().unwrap()[1], serde_json::json!("o"));
    assert!(output_data(&lines[2]).starts_with("\u{1b}[0m\u{1b}[2J\u{1b}[1;1H"));
}

#[test]
fn ignores_cursor_position_while_the_cursor_is_hidden() {
    let mut recorder = AsciicastRecorder::new(4, 2).unwrap();
    let mut hidden = terminal(4, 2, Line::from("same"), None);

    assert!(
        recorder
            .record_frame(Duration::from_millis(10), hidden.backend())
            .unwrap()
    );
    hidden
        .backend_mut()
        .set_cursor_position(Position { x: 3, y: 1 })
        .unwrap();
    assert!(
        !recorder
            .record_frame(Duration::from_millis(20), hidden.backend())
            .unwrap(),
        "a hidden cursor position is not visible screen state"
    );

    assert_eq!(json_lines(&recorder).len(), 2);
}

#[test]
fn full_frames_preserve_wide_symbols_style_and_cursor_state() {
    let style = Style::default()
        .fg(Color::Rgb(1, 2, 3))
        .bg(Color::Indexed(42))
        .add_modifier(Modifier::BOLD);
    let visible = terminal(6, 1, Line::from(Span::styled("界🙂", style)), Some((4, 0)));
    let hidden = terminal(6, 1, Line::from(Span::styled("界🙂", style)), None);
    let mut recorder = AsciicastRecorder::new(6, 1).unwrap();

    assert!(
        recorder
            .record_frame(Duration::ZERO, visible.backend())
            .unwrap()
    );
    assert!(
        recorder
            .record_frame(Duration::from_millis(100), hidden.backend())
            .unwrap(),
        "a cursor-only change must produce a frame"
    );

    let lines = json_lines(&recorder);
    let visible_output = output_data(&lines[1]);
    let hidden_output = output_data(&lines[2]);
    for output in [visible_output, hidden_output] {
        assert!(output.starts_with("\u{1b}[0m\u{1b}[2J\u{1b}[1;1H"));
        assert_eq!(output.matches('界').count(), 1);
        assert_eq!(output.matches('🙂').count(), 1);
        assert!(
            output.contains("\u{1b}[1m"),
            "bold SGR was absent: {output:?}"
        );
        assert!(
            output.contains("\u{1b}[38;2;1;2;3;48;5;42m"),
            "color SGR was absent: {output:?}"
        );
        assert!(!output.contains("界 "));
        assert!(
            output.contains("\u{1b}[1;3H🙂"),
            "the emoji did not start after the CJK continuation cell: {output:?}"
        );
    }
    assert!(visible_output.ends_with("\u{1b}[1;5H\u{1b}[?25h"));
    assert!(hidden_output.ends_with("\u{1b}[?25l"));
}

#[test]
fn full_frames_preserve_combining_zwj_and_variation_selector_graphemes() {
    let text = "e\u{301} 👩\u{200d}💻 ❤️";
    let frame = terminal(16, 1, Line::from(text), None);
    let mut recorder = AsciicastRecorder::new(16, 1).unwrap();

    recorder
        .record_frame(Duration::ZERO, frame.backend())
        .unwrap();

    let lines = json_lines(&recorder);
    let output = output_data(&lines[1]);
    for grapheme in ["e\u{301}", "👩\u{200d}💻", "❤️"] {
        assert_eq!(
            output.matches(grapheme).count(),
            1,
            "grapheme was changed during ANSI serialization: {output:?}"
        );
    }
}

#[test]
fn portable_style_serialization_covers_every_named_color() {
    let colors = [
        (Color::Black, 0),
        (Color::Red, 1),
        (Color::Green, 2),
        (Color::Yellow, 3),
        (Color::Blue, 4),
        (Color::Magenta, 5),
        (Color::Cyan, 6),
        (Color::Gray, 7),
        (Color::DarkGray, 8),
        (Color::LightRed, 9),
        (Color::LightGreen, 10),
        (Color::LightYellow, 11),
        (Color::LightBlue, 12),
        (Color::LightMagenta, 13),
        (Color::LightCyan, 14),
        (Color::White, 15),
    ];
    for (color, index) in colors {
        let mut output = String::new();
        write_style(
            &mut output,
            (color, Color::Reset, Color::Reset, Modifier::empty()),
        )
        .unwrap();
        assert_eq!(output, format!("\u{1b}[0m\u{1b}[38;5;{index}m"));
    }
}

#[test]
fn portable_style_serialization_covers_modifiers_and_three_color_channels() {
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
        let mut output = String::new();
        write_style(
            &mut output,
            (Color::Reset, Color::Reset, Color::Reset, modifier),
        )
        .unwrap();
        assert_eq!(output, format!("\u{1b}[0m\u{1b}[{code}m"));
    }

    let mut output = String::new();
    write_style(
        &mut output,
        (
            Color::Rgb(1, 2, 3),
            Color::Indexed(42),
            Color::LightCyan,
            Modifier::UNDERLINED,
        ),
    )
    .unwrap();
    assert_eq!(
        output,
        "\u{1b}[0m\u{1b}[4m\u{1b}[38;2;1;2;3;48;5;42;58;5;14m"
    );
}
