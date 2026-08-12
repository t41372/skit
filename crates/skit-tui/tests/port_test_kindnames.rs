//! Python parity contracts from `tests/test_kindnames.py` at main@206f9ef.

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::{TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, AddAction, AddEffect, AddStage, AddWorkflowState, LibraryState, Screen, SourceSnapshot,
};

const INTERPRETED_IDS: [&str; 10] = [
    "fish",
    "js",
    "lua",
    "perl",
    "powershell",
    "python",
    "r",
    "ruby",
    "shell",
    "ts",
];

const FULL_IDS: [&str; 12] = [
    "fish",
    "js",
    "lua",
    "perl",
    "powershell",
    "python",
    "r",
    "ruby",
    "shell",
    "ts",
    "exe",
    "prompt",
];

const FULL_LABELS: [&str; 12] = [
    "fish",
    "JavaScript",
    "Lua",
    "Perl",
    "PowerShell",
    "Python",
    "R",
    "Ruby",
    "Shell",
    "TypeScript",
    "A program (run it directly)",
    "A prompt for an AI agent",
];

fn ambiguous_kind_workflow(is_draft: bool) -> AddWorkflowState {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath("mystery.txt".to_owned()));
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::InspectSource { request, .. }] = effects.as_slice() else {
        panic!("continuing an ambiguous path must request one source inspection");
    };
    let request = *request;
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(SourceSnapshot {
            path: "mystery.txt".into(),
            source_record: "mystery.txt".to_owned(),
            bytes: b"plain text without a supported shebang\n".to_vec(),
            permissions: SourcePermissions::default(),
            is_regular: true,
            is_directory: false,
            is_draft,
        }),
    });
    assert_eq!(workflow.stage(), AddStage::Kind);
    workflow
}

fn kind_ids(workflow: &AddWorkflowState) -> Vec<&'static str> {
    workflow
        .kind_picker()
        .expect("ambiguous source must expose the kind picker")
        .choices()
        .iter()
        .map(|kind| kind.as_str())
        .collect()
}

fn draw_kind_picker(workflow: AddWorkflowState) -> Terminal<TestBackend> {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(workflow))));
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 34)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    assert!(geometry.add.is_some());
    terminal
}

fn row_containing(buffer: &Buffer, needle: &str) -> u16 {
    (0..buffer.area.height)
        .find(|row| {
            (0..buffer.area.width)
                .map(|column| buffer[(column, *row)].symbol())
                .collect::<String>()
                .contains(needle)
        })
        .unwrap_or_else(|| panic!("expected rendered kind-choice label: {needle}"))
}

fn rendered_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn test_kind_choices_exact_options_and_order() {
    let workflow = ambiguous_kind_workflow(false);
    assert_eq!(kind_ids(&workflow), FULL_IDS);
    assert_eq!(&FULL_IDS[..INTERPRETED_IDS.len()], INTERPRETED_IDS);

    let terminal = draw_kind_picker(workflow);
    let rows = FULL_LABELS
        .iter()
        .map(|label| row_containing(terminal.backend().buffer(), label))
        .collect::<Vec<_>>();
    for pair in rows.windows(2) {
        assert!(
            pair[0] < pair[1],
            "Python main requires exact kind-choice order; rendered rows were {rows:?}"
        );
    }
}

#[test]
fn test_kind_choices_offer_exe_false_drops_only_exe() {
    let full = ambiguous_kind_workflow(false);
    let gated = ambiguous_kind_workflow(true);
    let full_ids = kind_ids(&full);
    let gated_ids = kind_ids(&gated);
    let expected_gated = full_ids
        .iter()
        .copied()
        .filter(|kind| *kind != "exe")
        .collect::<Vec<_>>();

    assert_eq!(full_ids, FULL_IDS);
    assert_eq!(gated_ids, expected_gated);
    assert_eq!(
        gated_ids,
        [
            "fish",
            "js",
            "lua",
            "perl",
            "powershell",
            "python",
            "r",
            "ruby",
            "shell",
            "ts",
            "prompt",
        ]
    );
    assert_eq!(gated_ids.last().copied(), Some("prompt"));

    let terminal = draw_kind_picker(gated);
    let buffer = terminal.backend().buffer();
    let _ = row_containing(buffer, "A prompt for an AI agent");
    assert!(
        !rendered_text(buffer).contains("A program (run it directly)"),
        "draft kind picker must remove only the executable choice"
    );
}
