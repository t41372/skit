//! What the entry-settings screen shows at rest, in the terminal the oracle's tape records.
//!
//! Every assertion here goes through the whole composition — header, panel, body and footer — for
//! the reason this file exists at all. The screen's own unit tests asked whether each section
//! *becomes* reachable while the keyboard walks, which stayed true while four of six sections were
//! off screen on the first frame and nothing said the screen continued. Replaying the oracle's
//! `shots.tape` is what asked the other question, and only a test at this size, through this entry
//! point, can hold the answer.

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use skit_domain::parameters::{ParamDecl, ParameterValue};
use skit_i18n::Locale;
use skit_tui::{TuiSession, ViewGeometry, render_with_session};
use skit_ui::{Action, LibraryState, Screen, SettingsInputs, SettingsView};

/// The recorded demo terminal: 1280x780 at 12.19px per column and 26.33px per row, less 20px of
/// padding — the same size `docs/assets/demo/shots.tape` runs the oracle at.
const DEMO_WIDTH: u16 = 101;
const DEMO_HEIGHT: u16 = 28;

fn rendered(buffer: &Buffer) -> String {
    buffer
        .content()
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The entry the demo tape opens settings on: a stored python copy with one managed constant.
fn banner_view() -> SettingsView {
    let mut message = ParamDecl::new("MESSAGE");
    message.default = Some(ParameterValue::String("Hello from skit".to_owned()));
    SettingsView::from_inputs(&SettingsInputs {
        selector: "banner".to_owned(),
        kind: "python".to_owned(),
        name: "banner".to_owned(),
        description: "Print a boxed message a few times — settings live at the top.".to_owned(),
        source: "/demo/banner.py".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        has_analyzer: true,
        managed: vec![message],
        ..SettingsInputs::default()
    })
}

fn draw(width: u16, height: u16) -> String {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(banner_view()))));
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    rendered(terminal.backend().buffer())
}

/// The first frame reaches the parameter section, with no key pressed.
///
/// Version 0.4's own frame at this size shows Basics, Storage, the four working-directory options,
/// and `Parameters (the run form's fields)` with its first row. Anything less is a regression
/// against the oracle rather than a matter of taste.
#[test]
fn the_recorded_terminal_reaches_the_parameter_section_on_the_first_frame() {
    let frame = draw(DEMO_WIDTH, DEMO_HEIGHT);
    for expected in [
        "Basics",
        "banner",
        "Renaming keeps everything",
        "Storage",
        "Run in (working directory)",
        // The fourth option, which used to be the first thing cut off.
        "A fixed folder (type it below)",
        "Parameters (the run form's fields)",
        "MESSAGE",
    ] {
        assert!(
            frame.contains(expected),
            "the first frame does not reach {expected}:\n{frame}"
        );
    }
}

/// The description box spends the three rows version 0.4 spends on it.
///
/// Version 0.4's description is a single-line `Input` (`src/skit/tui_settings.py:394-399`), three
/// rows with its border. Keeping the multiline kind keeps the line breaks a person can type, but
/// six rows for one field is three rows taken from every screen below it — which is a third of what
/// the parameter section needs to be on screen at all.
#[test]
fn the_description_box_is_as_tall_as_the_one_version_04_draws() {
    let frame = draw(DEMO_WIDTH, DEMO_HEIGHT);
    let lines = frame.lines().collect::<Vec<_>>();
    let top = lines
        .iter()
        .position(|line| line.contains("Description (shown in the Library)"))
        .expect("the description box is not on screen");
    assert!(
        lines[top + 2].contains('\u{2518}'),
        "the description box is taller than three rows:\n{frame}"
    );
}

/// The screen titles itself, so it takes no shared header.
///
/// Version 0.4 puts the entry name on the panel border and nowhere else
/// (`src/skit/tui_settings.py:869-871`). Printing it twice spent three rows saying the same thing,
/// and those were three of the rows the sections below needed.
#[test]
fn the_settings_screen_prints_its_title_once() {
    let frame = draw(DEMO_WIDTH, DEMO_HEIGHT);
    let titles = frame
        .lines()
        .filter(|line| line.contains("Entry settings"))
        .count();
    assert_eq!(titles, 1, "the title is printed more than once:\n{frame}");
}

/// Whatever is still below the fold carries a mark that says so.
///
/// Version 0.4 hosts the body in a `VerticalScroll`, and its scrollbar is the only thing that tells
/// a reader the screen continues. Sections below the fold with no mark beside them are sections a
/// person has no reason to look for.
#[test]
fn content_below_the_fold_says_that_it_continues() {
    let frame = draw(DEMO_WIDTH, DEMO_HEIGHT);
    assert!(
        frame.contains('█') || frame.contains('▐') || frame.contains('║'),
        "content continues with no scroll affordance:\n{frame}"
    );

    // A screen tall enough for everything needs no affordance, so this is not a widget that is
    // always there — it is a statement about the content.
    let tall = draw(DEMO_WIDTH, 80);
    assert!(
        tall.contains("Needs (external commands)"),
        "the tall frame should show the last section:\n{tall}"
    );
    assert!(
        !tall.contains('█'),
        "a screen with nothing below the fold drew a scroll affordance:\n{tall}"
    );
}

/// The rename sentence sits under the box it is about.
///
/// Version 0.4 yields the heading, the name, the sentence, then the description
/// (`src/skit/tui_settings.py:388-400`).
#[test]
fn the_rename_sentence_follows_the_name_it_explains() {
    let frame = draw(DEMO_WIDTH, DEMO_HEIGHT);
    let row_of = |needle: &str| {
        frame
            .lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} is not on screen:\n{frame}"))
    };
    // The name *box*, not the panel title, which also carries the entry name.
    assert!(
        row_of("\u{250c}Name") < row_of("Renaming keeps everything"),
        "the sentence sits above the name box it explains:\n{frame}"
    );
    assert!(
        row_of("Renaming keeps everything") < row_of("Storage"),
        "{frame}"
    );
}
