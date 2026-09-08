use std::{io, time::Duration};

use ratatui_core::{
    backend::{Backend as _, TestBackend},
    layout::Position,
    style::{Color, Modifier, Style},
    terminal::Terminal,
    text::{Line, Span},
};
use ratatui_widgets::paragraph::Paragraph;
use serde_json::Value;

use super::{AsciicastRecorder, FRAME_INTERVAL, write_style};
use crate::StyledFrameSnapshot;

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
fn rejects_zero_sizes_without_mutating_pending_time() {
    for (cols, rows) in [(0, 1), (1, 0), (0, 0)] {
        assert_eq!(
            AsciicastRecorder::new(cols, rows).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    let mut recorder = AsciicastRecorder::new(1, 1).unwrap();
    let before = recorder.as_bytes().to_vec();
    for backend in [TestBackend::new(0, 1), TestBackend::new(1, 0)] {
        assert_eq!(
            recorder
                .record_frame(Duration::from_millis(750), &backend)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(recorder.as_bytes(), before);
    }
    let smallest = terminal(1, 1, Line::from("x"), None);
    recorder
        .record_frame(Duration::from_millis(125), smallest.backend())
        .unwrap();
    assert_eq!(json_lines(&recorder)[1][0], serde_json::json!(0.125));
}

#[test]
fn relative_intervals_deduplicate_and_skips_accumulate() {
    let first = terminal(4, 2, Line::from("A"), None);
    let second = terminal(4, 2, Line::from("B"), None);
    let mut recorder = AsciicastRecorder::new(4, 2).unwrap();
    assert!(
        recorder
            .record_frame(Duration::from_millis(100), first.backend())
            .unwrap()
    );
    assert!(
        !recorder
            .record_frame(Duration::from_millis(200), first.backend())
            .unwrap()
    );
    recorder.skip(Duration::from_millis(300));
    assert!(
        recorder
            .record_frame(Duration::from_millis(400), second.backend())
            .unwrap()
    );
    let lines = json_lines(&recorder);
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[0],
        serde_json::json!({"version": 3, "term": {"cols": 4, "rows": 2}})
    );
    assert_eq!(lines[1][0], serde_json::json!(0.1));
    assert_eq!(lines[2][0], serde_json::json!(0.9));
    assert!(output_data(&lines[2]).contains('B'));
}

#[test]
fn resize_cursor_style_and_stored_frame_round_trip_are_exact() {
    let style = Style::default()
        .fg(Color::Rgb(1, 2, 3))
        .bg(Color::Indexed(42))
        .add_modifier(Modifier::BOLD);
    let visible = terminal(6, 1, Line::from(Span::styled("界🙂", style)), Some((4, 0)));
    let hidden = terminal(6, 1, Line::from(Span::styled("界🙂", style)), None);
    let snapshot = StyledFrameSnapshot::from_backend(visible.backend());
    let mut live = AsciicastRecorder::new(4, 2).unwrap();
    let mut rebuilt = AsciicastRecorder::new(4, 2).unwrap();
    live.record_frame(FRAME_INTERVAL, visible.backend())
        .unwrap();
    rebuilt.record_snapshot(FRAME_INTERVAL, &snapshot).unwrap();
    assert_eq!(live.as_bytes(), rebuilt.as_bytes());
    let lines = json_lines(&live);
    assert_eq!(lines[1], serde_json::json!([0.1, "r", "6x1"]));
    assert_eq!(lines[2][0], serde_json::json!(0.0));
    let output = output_data(&lines[2]);
    assert!(output.contains("\u{1b}[1m"));
    assert!(output.contains("\u{1b}[38;2;1;2;3;48;5;42m"));
    assert!(output.ends_with("\u{1b}[1;5H\u{1b}[?25h"));

    let mut cursor = AsciicastRecorder::new(6, 1).unwrap();
    cursor
        .record_frame(Duration::ZERO, visible.backend())
        .unwrap();
    assert!(
        cursor
            .record_frame(FRAME_INTERVAL, hidden.backend())
            .unwrap()
    );
    assert!(output_data(json_lines(&cursor).last().unwrap()).ends_with("\u{1b}[?25l"));
}

#[test]
fn hidden_cursor_position_and_complex_graphemes_keep_visible_semantics() {
    let mut hidden = terminal(16, 1, Line::from("e\u{301} 👩\u{200d}💻 ❤️"), None);
    let mut recorder = AsciicastRecorder::new(16, 1).unwrap();
    assert!(
        recorder
            .record_frame(Duration::ZERO, hidden.backend())
            .unwrap()
    );
    hidden
        .backend_mut()
        .set_cursor_position(Position { x: 15, y: 0 })
        .unwrap();
    assert!(
        !recorder
            .record_frame(FRAME_INTERVAL, hidden.backend())
            .unwrap()
    );
    let lines = json_lines(&recorder);
    let output = output_data(&lines[1]);
    for grapheme in ["e\u{301}", "👩\u{200d}💻", "❤️"] {
        assert_eq!(output.matches(grapheme).count(), 1);
    }
}

#[test]
fn portable_style_serialization_covers_named_and_rgb_colors_and_modifiers() {
    for (color, index) in [
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
    ] {
        let mut output = String::new();
        write_style(
            &mut output,
            (color, Color::Reset, Color::Reset, Modifier::empty()),
        )
        .unwrap();
        assert_eq!(output, format!("\u{1b}[0m\u{1b}[38;5;{index}m"));
    }
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
