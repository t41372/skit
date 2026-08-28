use std::collections::BTreeSet;

use ratatui_core::{backend::TestBackend, layout::Rect, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{Event, KeyModifiers, MouseEvent, MouseEventKind};
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddScreenSession, FilePickerHit, FilePickerSession, PromptCandidatePickerSession,
    SettingsScreenSession, render_add, render_file_picker, render_prompt_candidate_picker,
    render_settings,
};
use skit_ui::{
    AddAction, AddEffect, AddWorkflowState, ChoicePicker, PathOutputPolicy, PathPickerState,
    PathSelectionMode, PickerItem, PickerMode, PickerPurpose, SettingsInputs, SettingsView,
};

fn assert_empty_rect(rect: Rect, viewport: Rect, owner: &str) {
    assert_eq!(
        rect.height, 0,
        "{owner} produced a positive-height child: {rect:?}"
    );
    assert!(
        rect.x >= viewport.x
            && rect.right() <= viewport.right()
            && rect.y >= viewport.y
            && rect.bottom() <= viewport.bottom(),
        "{owner} produced a child outside its empty viewport: child={rect:?} viewport={viewport:?}"
    );
}

#[test]
fn zero_height_local_viewports_do_not_emit_positive_children_or_hits() {
    let viewport = Rect::new(4, 5, 24, 0);
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();

    let add = AddWorkflowState::new(Vec::new());
    let mut add_session = AddScreenSession::default();
    terminal
        .draw(|frame| {
            let geometry = render_add(frame, viewport, &add, &mut add_session, Locale::En);
            assert_empty_rect(geometry.body, viewport, "Add body");
            for hit in geometry.hits {
                assert_empty_rect(hit.area, viewport, "Add hit");
            }
        })
        .unwrap();

    let settings = SettingsView::from_inputs(&SettingsInputs::default());
    let mut settings_session = SettingsScreenSession::default();
    terminal
        .draw(|frame| {
            let geometry = render_settings(
                frame,
                viewport,
                &settings,
                &mut settings_session,
                Locale::En,
            );
            assert_empty_rect(geometry.body, viewport, "Settings body");
            for hit in geometry.hits {
                assert_empty_rect(hit.area, viewport, "Settings hit");
            }
        })
        .unwrap();

    let picker = ChoicePicker::new(
        PickerMode::Multiple,
        vec![PickerItem::new("name".to_owned(), "name")],
        Vec::new(),
    );
    let mut prompt_session = PromptCandidatePickerSession::new(picker);
    terminal
        .draw(|frame| {
            let geometry =
                render_prompt_candidate_picker(frame, viewport, &mut prompt_session, Locale::En);
            assert_empty_rect(geometry.search, viewport, "Prompt picker search");
            assert_empty_rect(geometry.rows, viewport, "Prompt picker rows");
            for hit in geometry.hits {
                assert_empty_rect(hit.area, viewport, "Prompt picker hit");
            }
        })
        .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let contract = PathPickerState::new(
        PickerPurpose::Argument,
        directory.path().to_path_buf(),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(directory.path().to_path_buf()),
        false,
    );
    let mut file_session = FilePickerSession::new(contract);
    terminal
        .draw(|frame| {
            let geometry = render_file_picker(frame, viewport, &mut file_session, Locale::En);
            assert_empty_rect(geometry.search, viewport, "File picker search");
            assert_empty_rect(geometry.rows, viewport, "File picker rows");
            for hit in geometry.hits {
                assert_empty_rect(hit.area, viewport, "File picker hit");
            }
        })
        .unwrap();
}

#[test]
fn one_row_surfaces_do_not_fabricate_body_hits_below_a_zero_height_body() {
    let viewport = Rect::new(4, 5, 24, 1);
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();

    let settings = SettingsView::from_inputs(&SettingsInputs::default());
    let mut settings_session = SettingsScreenSession::default();
    terminal
        .draw(|frame| {
            let geometry = render_settings(
                frame,
                viewport,
                &settings,
                &mut settings_session,
                Locale::En,
            );
            assert_eq!(geometry.body.height, 0);
            assert!(
                geometry.hits.is_empty(),
                "Settings fabricated hits for a zero-height body: {:?}",
                geometry.hits
            );
        })
        .unwrap();

    let picker = ChoicePicker::new(
        PickerMode::Multiple,
        vec![PickerItem::new("name".to_owned(), "name")],
        Vec::new(),
    );
    let mut prompt_session = PromptCandidatePickerSession::new(picker);
    terminal
        .draw(|frame| {
            let geometry =
                render_prompt_candidate_picker(frame, viewport, &mut prompt_session, Locale::En);
            assert_eq!(geometry.search.height, 0);
            assert_eq!(geometry.rows.height, 0);
            assert!(
                geometry.hits.is_empty(),
                "Prompt picker fabricated hits below its zero-height rows: {:?}",
                geometry.hits
            );
        })
        .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let contract = PathPickerState::new(
        PickerPurpose::Argument,
        directory.path().to_path_buf(),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(directory.path().to_path_buf()),
        false,
    );
    let mut file_session = FilePickerSession::new(contract);
    terminal
        .draw(|frame| {
            let geometry = render_file_picker(frame, viewport, &mut file_session, Locale::En);
            assert_eq!(geometry.search.height, 0);
            assert_eq!(geometry.rows.height, 0);
            assert!(
                geometry.hits.is_empty(),
                "File picker fabricated hits below its zero-height rows: {:?}",
                geometry.hits
            );
        })
        .unwrap();
}

#[test]
fn add_footer_clamps_its_offset_when_only_height_grows() {
    let state = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(24, 13)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &state, &mut session, Locale::En);
        })
        .unwrap();

    for _ in 0..12 {
        let footer = geometry
            .hits
            .iter()
            .find(|hit| hit.area.y >= geometry.body.bottom())
            .expect("the short Add footer has a visible hit")
            .area;
        let _ = session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: footer.x,
                row: footer.y,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        );
        terminal
            .draw(|frame| {
                geometry = render_add(frame, frame.area(), &state, &mut session, Locale::En);
            })
            .unwrap();
    }

    let mut grown = Terminal::new(TestBackend::new(24, 14)).unwrap();
    grown
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &state, &mut session, Locale::En);
        })
        .unwrap();
    let footer_rows = geometry
        .hits
        .iter()
        .filter(|hit| {
            matches!(
                hit.target,
                AddControlId::NextField | AddControlId::PreviousField
            )
        })
        .map(|hit| hit.area.y)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        footer_rows.len(),
        2,
        "height growth must expose both local footer rows after clamping"
    );
}

#[test]
fn file_picker_footer_clamps_its_offset_when_only_height_grows() {
    let directory = tempfile::tempdir().unwrap();
    let contract = PathPickerState::new(
        PickerPurpose::Argument,
        directory.path().to_path_buf(),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(directory.path().to_path_buf()),
        false,
    );
    let mut session = FilePickerSession::new(contract);
    let mut terminal = Terminal::new(TestBackend::new(52, 11)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_file_picker(frame, frame.area(), &mut session, Locale::En);
        })
        .unwrap();

    for _ in 0..12 {
        let footer = geometry
            .hits
            .iter()
            .find(|hit| {
                matches!(
                    hit.target,
                    FilePickerHit::Accept
                        | FilePickerHit::Cancel
                        | FilePickerHit::Up
                        | FilePickerHit::Hidden
                )
            })
            .expect("the compact file footer has a visible hit")
            .area;
        let _ = session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: footer.x,
                row: footer.y,
                modifiers: KeyModifiers::NONE,
            }),
            &geometry,
        );
        terminal
            .draw(|frame| {
                geometry = render_file_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
    }

    let mut grown = Terminal::new(TestBackend::new(52, 12)).unwrap();
    grown
        .draw(|frame| {
            geometry = render_file_picker(frame, frame.area(), &mut session, Locale::En);
        })
        .unwrap();
    let footer_rows = geometry
        .hits
        .iter()
        .filter(|hit| {
            matches!(
                hit.target,
                FilePickerHit::Accept
                    | FilePickerHit::Cancel
                    | FilePickerHit::Up
                    | FilePickerHit::Hidden
            )
        })
        .map(|hit| hit.area.y)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        footer_rows.len(),
        2,
        "height growth must expose both file-picker footer rows after clamping"
    );
}

#[test]
fn add_long_problem_note_reaches_its_wrapped_sentinel() {
    let mut state = AddWorkflowState::new(Vec::new());
    let _ = state.reduce(AddAction::SetSourcePath("missing.py".to_owned()));
    let request = state
        .reduce(AddAction::Continue)
        .into_iter()
        .find_map(|effect| match effect {
            AddEffect::InspectSource { request, .. } => Some(request),
            _ => None,
        })
        .expect("Continue requests source inspection");
    let _ = state.reduce(AddAction::SourceInspected {
        request,
        result: Err(format!(
            "{}ADD-END",
            "a long source inspection problem ".repeat(16)
        )),
    });
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(24, 8)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| geometry = render_add(frame, frame.area(), &state, &mut session, Locale::En))
        .unwrap();

    let mut last = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    for _ in 0..40 {
        if last.contains("ADD-END") {
            break;
        }
        let _ = session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: geometry.body.x,
                row: geometry.body.y,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        );
        terminal
            .draw(|frame| {
                geometry = render_add(frame, frame.area(), &state, &mut session, Locale::En);
            })
            .unwrap();
        last = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
    }
    assert!(last.contains("ADD-END"), "{last}");
}
