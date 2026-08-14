use std::{
    fs,
    path::{Path, PathBuf},
};

use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_i18n::Locale;
use skit_tui::{
    FilePickerEvent, FilePickerGeometry, FilePickerHit, FilePickerSession, render_file_picker,
};
use skit_ui::{PathOutputPolicy, PathPickerState, PathSelectionMode, PickerPurpose};
use tempfile::TempDir;

fn tree() -> TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir(temp.path().join("sub")).expect("subdir");
    fs::write(temp.path().join("data.csv"), b"x").expect("data");
    fs::write(temp.path().join("draft.txt"), b"x").expect("draft");
    fs::write(temp.path().join("sub/inner.txt"), b"x").expect("inner");
    fs::write(temp.path().join(".hidden"), b"x").expect("hidden");
    temp
}

fn session(root: &Path) -> FilePickerSession {
    FilePickerSession::new(PathPickerState::new(
        PickerPurpose::Argument,
        root.to_path_buf(),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(root.to_path_buf()),
        false,
    ))
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn type_text(session: &mut FilePickerSession, value: &str) {
    let geometry = FilePickerGeometry::default();
    for character in value.chars() {
        assert_eq!(
            session.handle_event(key(KeyCode::Char(character)), &geometry),
            Some(FilePickerEvent::Changed)
        );
    }
}

fn visible_names(session: &FilePickerSession) -> Vec<String> {
    let explorer = session.explorer();
    explorer.filtered_indices.as_ref().map_or_else(
        || explorer.entries.iter().map(|entry| entry.name.clone()).collect(),
        |indices| {
            indices
                .iter()
                .filter_map(|index| explorer.entries.get(*index))
                .map(|entry| entry.name.clone())
                .collect()
        },
    )
}

fn render(session: &mut FilePickerSession, width: u16, height: u16) -> (String, FilePickerGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    let mut geometry = FilePickerGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_file_picker(frame, frame.area(), session, Locale::En);
        })
        .expect("draw picker");
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    (text, geometry)
}

fn click_for(target: &FilePickerHit, geometry: &FilePickerGeometry) -> Event {
    let hit = geometry
        .hits
        .iter()
        .find(|hit| &hit.target == target)
        .unwrap_or_else(|| panic!("missing hit target {target:?}"));
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.area.x,
        row: hit.area.y,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn test_picker_enter_descends_then_picks_and_filter_clears() {
    let temp = tree();
    let root = temp.path();
    let mut picker = session(root);
    let geometry = FilePickerGeometry::default();

    type_text(&mut picker, "su");
    assert_eq!(visible_names(&picker), vec!["sub"]);
    assert_eq!(
        picker.handle_event(key(KeyCode::Enter), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(picker.current_dir(), &root.join("sub"));
    assert_eq!(visible_names(&picker), vec!["..", "inner.txt"]);
    assert_eq!(
        picker.handle_event(key(KeyCode::Enter), &geometry),
        Some(FilePickerEvent::Accepted(vec![PathBuf::from("sub/inner.txt")]))
    );
}

#[test]
fn test_picker_arrows_steer_highlight_without_leaving_the_filter() {
    let temp = tree();
    let mut picker = session(temp.path());
    let geometry = FilePickerGeometry::default();

    type_text(&mut picker, "d");
    assert_eq!(visible_names(&picker), vec!["data.csv", "draft.txt"]);
    assert_eq!(
        picker.handle_event(key(KeyCode::Down), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(
        picker.handle_event(key(KeyCode::PageDown), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(
        picker.handle_event(key(KeyCode::PageUp), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(
        picker.handle_event(key(KeyCode::Char('a')), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(visible_names(&picker), vec!["data.csv", "draft.txt"]);
}

#[test]
fn test_picker_prefix_matches_outrank_substring_hits() {
    let temp = tree();
    fs::create_dir(temp.path().join("Anaconda")).expect("Anaconda");
    let mut picker = session(temp.path());
    type_text(&mut picker, "da");
    assert_eq!(visible_names(&picker), vec!["data.csv", "Anaconda"]);
    assert_eq!(
        picker.handle_event(key(KeyCode::Enter), &FilePickerGeometry::default()),
        Some(FilePickerEvent::Accepted(vec![PathBuf::from("data.csv")]))
    );
}

#[test]
fn test_picker_filter_is_case_insensitive_substring() {
    let temp = tree();
    fs::write(temp.path().join("README.md"), b"x").expect("README");
    let mut picker = session(temp.path());
    type_text(&mut picker, "eadm");
    assert_eq!(visible_names(&picker), vec!["README.md"]);
}

#[test]
fn test_picker_row_click_is_the_mouse_path() {
    let temp = tree();
    let mut picker = session(temp.path());
    type_text(&mut picker, "data");
    let (_text, geometry) = render(&mut picker, 100, 30);
    let index = visible_names(&picker)
        .iter()
        .position(|name| name == "data.csv")
        .expect("data row");
    assert_eq!(
        picker.handle_event(click_for(&FilePickerHit::Entry(index), &geometry), &geometry),
        Some(FilePickerEvent::Accepted(vec![PathBuf::from("data.csv")]))
    );
}

#[test]
fn test_picker_zero_match_enter_is_a_noop() {
    let temp = tree();
    let mut picker = session(temp.path());
    type_text(&mut picker, "zzz-no-such");
    assert!(visible_names(&picker).is_empty());
    let before = picker.current_dir().clone();
    assert_eq!(
        picker.handle_event(key(KeyCode::Enter), &FilePickerGeometry::default()),
        None
    );
    assert_eq!(picker.current_dir(), &before);
}

#[test]
fn test_picker_filtering_hides_the_pinned_row() {
    let temp = tree();
    let mut picker = session(temp.path());
    type_text(&mut picker, "d");
    assert_eq!(visible_names(&picker), vec!["data.csv", "draft.txt"]);
    let (_text, geometry) = render(&mut picker, 100, 30);
    assert!(
        geometry
            .hits
            .iter()
            .all(|hit| hit.target != FilePickerHit::CurrentDirectory),
        "a non-empty filter must hide the pinned current-directory row"
    );
}

#[test]
fn test_picker_backspace_ascends_only_on_empty_filter() {
    let temp = tree();
    let root = temp.path();
    let mut picker = FilePickerSession::new(PathPickerState::new(
        PickerPurpose::Argument,
        root.join("sub"),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(root.to_path_buf()),
        false,
    ));
    let geometry = FilePickerGeometry::default();

    type_text(&mut picker, "in");
    assert_eq!(
        picker.handle_event(key(KeyCode::Backspace), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(picker.current_dir(), &root.join("sub"));
    assert_eq!(visible_names(&picker), vec!["inner.txt"]);
    assert_eq!(
        picker.handle_event(key(KeyCode::Backspace), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(picker.current_dir(), &root.join("sub"));
    assert_eq!(
        picker.handle_event(key(KeyCode::Backspace), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(picker.current_dir(), root);
}

#[test]
fn test_picker_backspace_noops_at_the_filesystem_root() {
    let root = PathBuf::from(std::path::MAIN_SEPARATOR.to_string());
    let mut picker = FilePickerSession::new(PathPickerState::new(
        PickerPurpose::Argument,
        root.clone(),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::Absolute,
        false,
    ));
    let before = picker.current_dir().clone();
    assert_eq!(
        picker.handle_event(key(KeyCode::Backspace), &FilePickerGeometry::default()),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(picker.current_dir(), &before);
}

#[test]
fn test_list_filtered_reveals_hidden_only_behind_a_dot_filter() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join(".env"), b"x").expect("env");
    fs::write(temp.path().join("readme"), b"x").expect("readme");
    let mut picker = session(temp.path());
    type_text(&mut picker, "en");
    assert!(visible_names(&picker).is_empty());
    assert_eq!(
        picker.handle_event(key(KeyCode::Esc), &FilePickerGeometry::default()),
        Some(FilePickerEvent::Changed)
    );
    type_text(&mut picker, ".en");
    assert_eq!(visible_names(&picker), vec![".env"]);
}

#[test]
fn test_list_filtered_dir_sorts_before_an_earlier_file_within_a_rank() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir(temp.path().join("xz")).expect("xz");
    fs::write(temp.path().join("xa"), b"x").expect("xa");
    let mut picker = session(temp.path());
    type_text(&mut picker, "x");
    assert_eq!(visible_names(&picker), vec!["xz", "xa"]);
}

#[test]
fn test_list_filtered_tiebreak_is_case_insensitive() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Force the public filtered-order path instead of trusting filesystem enumeration order. If
    // the tie-break compares original case, "xa.txt" incorrectly sorts before "x_Z.txt" because
    // ASCII 'Z' precedes lowercase 'a'; case-folded sorting correctly keeps x_Z before xa.
    fs::write(temp.path().join("x_Z.txt"), b"x").expect("mixed case");
    fs::write(temp.path().join("xa.txt"), b"x").expect("a");
    let mut picker = session(temp.path());
    type_text(&mut picker, "x");
    assert_eq!(visible_names(&picker), vec!["x_Z.txt", "xa.txt"]);
}

#[test]
fn test_picker_pinned_row_shows_its_label() {
    let temp = tree();
    let mut picker = session(temp.path());
    let (text, geometry) = render(&mut picker, 100, 30);
    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| hit.target == FilePickerHit::CurrentDirectory),
        "the current-directory row must be a real selectable row"
    );
    assert!(
        text.contains("(use this directory)"),
        "the pinned row must render the frozen Python label exactly"
    );
}

#[test]
fn test_picker_ascend_repopulates_the_parent_listing() {
    let temp = tree();
    let root = temp.path();
    let mut picker = FilePickerSession::new(PathPickerState::new(
        PickerPurpose::Argument,
        root.join("sub"),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(root.to_path_buf()),
        false,
    ));
    assert_eq!(
        picker.handle_event(key(KeyCode::Backspace), &FilePickerGeometry::default()),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(picker.current_dir(), root);
    let names = visible_names(&picker);
    assert!(names.iter().any(|name| name == "sub"));
    assert!(names.iter().any(|name| name == "data.csv"));
}
