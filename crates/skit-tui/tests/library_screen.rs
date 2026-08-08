use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use skit_core::EntrySummary;
use skit_tui::{Action, App, render};

fn entries() -> Vec<EntrySummary> {
    vec![
        EntrySummary {
            slug: "alpha".into(),
            name: "Alpha".into(),
            kind: "python".into(),
            mode: "copy".into(),
            description: "First script".into(),
            source: "/tmp/alpha.py".into(),
            dir: std::path::PathBuf::from("/tmp/alpha"),
        },
        EntrySummary {
            slug: "beta".into(),
            name: "Beta".into(),
            kind: "shell".into(),
            mode: "reference".into(),
            description: "Second script".into(),
            source: "/tmp/beta.sh".into(),
            dir: std::path::PathBuf::from("/tmp/beta"),
        },
    ]
}

#[test]
fn library_screen_renders_entries_and_visible_key_hints() -> Result<(), Box<dyn std::error::Error>>
{
    let backend = TestBackend::new(64, 12);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new(entries());
    terminal.draw(|frame| render(frame, &mut app))?;
    let text = terminal
        .backend()
        .buffer()
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
    assert_eq!(app.selected().map(|e| e.slug.as_str()), Some("alpha"));
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        Action::None
    );
    assert_eq!(app.selected().map(|e| e.slug.as_str()), Some("beta"));
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        Action::None
    );
    assert_eq!(app.selected().map(|e| e.slug.as_str()), Some("alpha"));
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        Action::Quit
    );
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Action::Quit
    )
}

#[test]
fn empty_library_navigation_is_safe() {
    let mut app = App::new(Vec::new());
    assert!(app.selected().is_none());
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        Action::None
    );
    assert!(app.selected().is_none())
}
