use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use skit_core::EntrySummary;
use skit_tui::{Action, App, render};

fn entries() -> Vec<EntrySummary> {
    vec![
        EntrySummary {
            slug: "alpha".to_owned(),
            name: "Alpha".to_owned(),
            kind: "python".to_owned(),
            mode: "copy".to_owned(),
            description: "First script".to_owned(),
            source: "/tmp/alpha.py".to_owned(),
        },
        EntrySummary {
            slug: "beta".to_owned(),
            name: "Beta".to_owned(),
            kind: "shell".to_owned(),
            mode: "reference".to_owned(),
            description: "Second script".to_owned(),
            source: "/tmp/beta.sh".to_owned(),
        },
    ]
}

#[test]
fn library_screen_renders_entries_and_visible_key_hints() -> Result<(), Box<dyn std::error::Error>> {
    let backend = TestBackend::new(64, 12);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new(entries());

    terminal.draw(|frame| render(frame, &mut app))?;

    let buffer = terminal.backend().buffer();
    let text = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("skit"));
    assert!(text.contains("Alpha"));
    assert!(text.contains("Beta"));
    assert!(text.contains("↑/↓ move"));
    assert!(text.contains("q quit"));
    Ok(())
}

#[test]
fn keyboard_navigation_moves_selection_and_quits() {
    let mut app = App::new(entries());
    assert_eq!(app.selected().map(|entry| entry.slug.as_str()), Some("alpha"));

    let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.handle_key(down), Action::None);
    assert_eq!(app.selected().map(|entry| entry.slug.as_str()), Some("beta"));

    let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.handle_key(up), Action::None);
    assert_eq!(app.selected().map(|entry| entry.slug.as_str()), Some("alpha"));

    let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(app.handle_key(quit), Action::Quit);

    let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.handle_key(escape), Action::Quit);
}

#[test]
fn empty_library_navigation_is_safe() {
    let mut app = App::new(Vec::new());
    assert!(app.selected().is_none());

    let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.handle_key(down), Action::None);
    assert!(app.selected().is_none());
}
