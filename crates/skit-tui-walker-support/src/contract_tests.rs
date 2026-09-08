use std::num::NonZeroU16;

use ratatui_core::{
    buffer::{Buffer, CellDiffOption},
    layout::{Position, Rect},
    style::{Color, Modifier},
};
use serde_json::{Value, json};

use super::*;

const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn profile(value: &str) -> SafeProfileId {
    SafeProfileId::try_from(value).unwrap()
}

fn reference(kind: ObjectKind) -> ObjectRef {
    ObjectRef {
        kind,
        sha256: ZERO_DIGEST.to_owned(),
    }
}

fn row(
    sequence: u32,
    phase: TimelinePhase,
    chain: Option<EventChainIdentity>,
    operation_index: Option<u32>,
    boundary: TimelineBoundary,
    cause: TransitionCause,
    presentation: Presentation,
) -> TimelineRow {
    TimelineRow {
        schema: 3,
        profile: profile("en-80x24"),
        sequence,
        phase,
        event_chain: chain,
        operation_index,
        boundary,
        cause,
        presentation,
        reducer: reference(ObjectKind::Reducer),
        host: reference(ObjectKind::Host),
        session: reference(ObjectKind::Session),
        styled_frame: reference(ObjectKind::StyledFrame),
        geometry: reference(ObjectKind::Geometry),
        locale: "en".to_owned(),
        viewport: RectSnapshot {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        },
        liveness: None,
        previous_row_sha256: None,
    }
}

fn relink(rows: &mut [TimelineRow]) {
    let mut previous = None;
    for (sequence, row) in rows.iter_mut().enumerate() {
        row.sequence = u32::try_from(sequence).unwrap();
        row.previous_row_sha256 = previous;
        previous = Some(timeline_row_digest(row).unwrap());
    }
}

fn session(handling: Value, resolved: Value) -> TransitionCause {
    TransitionCause::Session {
        requested: json!("operation"),
        resolved,
        event: json!("event"),
        handling,
    }
}

fn reducer(emitted: Value) -> TransitionCause {
    TransitionCause::Reducer {
        requested: json!("operation"),
        resolved: json!({"input": {"kind": "event", "event": "event"}}),
        event: json!("event"),
        action: json!("action"),
        emitted,
    }
}

fn host(emitted: Value) -> TransitionCause {
    TransitionCause::Host {
        operation_index: Some(0),
        round: 0,
        request: json!("request"),
        response: json!("response"),
        emitted,
    }
}

fn complete_timeline() -> Vec<TimelineRow> {
    let operation_chain = Some(EventChainIdentity {
        phase: TimelinePhase::Operations,
        sequence: 0,
    });
    let liveness_chain = Some(EventChainIdentity {
        phase: TimelinePhase::FinalLiveness,
        sequence: 0,
    });
    let mut rows = vec![
        row(
            0,
            TimelinePhase::Operations,
            None,
            None,
            TimelineBoundary::Initial,
            TransitionCause::Initial,
            Presentation::Presented,
        ),
        row(
            1,
            TimelinePhase::Operations,
            operation_chain,
            Some(0),
            TimelineBoundary::UserAction,
            TransitionCause::Reducer {
                requested: json!({"operation": 0}),
                resolved: json!({"input": {"kind": "event", "event": "enter"}}),
                event: json!("enter"),
                action: json!("open"),
                emitted: json!("open"),
            },
            Presentation::NotPresented,
        ),
        row(
            2,
            TimelinePhase::Operations,
            operation_chain,
            Some(0),
            TimelineBoundary::HostAction,
            TransitionCause::Host {
                operation_index: Some(0),
                round: 0,
                request: json!("open"),
                response: json!("opened"),
                emitted: json!("none"),
            },
            Presentation::Presented,
        ),
        row(
            3,
            TimelinePhase::FinalLiveness,
            liveness_chain,
            None,
            TimelineBoundary::Session,
            TransitionCause::Session {
                requested: json!({"synthetic": "final_liveness"}),
                resolved: json!({"terminal": "library"}),
                event: json!({"synthetic": "already_terminal"}),
                handling: json!("already_terminal"),
            },
            Presentation::Presented,
        ),
    ];
    rows.last_mut().unwrap().liveness = Some(LivenessResult::Passed);
    relink(&mut rows);
    rows
}

#[test]
fn object_validation_rejects_an_extra_envelope_member() {
    let (mut reference, bytes) =
        build_object(ObjectKind::Reducer, &json!({"screen": "library"})).unwrap();
    let mut envelope: Value = serde_json::from_slice(&bytes).unwrap();
    envelope
        .as_object_mut()
        .unwrap()
        .insert("extra".to_owned(), json!(true));
    let bytes = canonical_json_bytes(&envelope).unwrap();
    reference.sha256 = sha256_hex(&bytes);

    assert_eq!(
        validate_object(&reference, &bytes).unwrap_err().to_string(),
        "typed object does not match its stored bytes"
    );
}

#[test]
fn presentation_classifier_follows_the_complete_transition_table() {
    let cases = [
        (
            "initial",
            TimelinePhase::Operations,
            TransitionCause::Initial,
            Presentation::Presented,
        ),
        (
            "session",
            TimelinePhase::Operations,
            session(json!("consumed"), json!({"event": "event"})),
            Presentation::Presented,
        ),
        (
            "noop",
            TimelinePhase::Operations,
            session(
                json!("not_applicable"),
                json!({"input": {"kind": "not_applicable"}}),
            ),
            Presentation::Presented,
        ),
        (
            "operation_already_quit",
            TimelinePhase::Operations,
            session(json!("already_terminal"), json!({"terminal": "quit"})),
            Presentation::Presented,
        ),
        (
            "user_none",
            TimelinePhase::Operations,
            reducer(json!("none")),
            Presentation::Presented,
        ),
        (
            "user_effect",
            TimelinePhase::Operations,
            reducer(json!("reload")),
            Presentation::NotPresented,
        ),
        (
            "user_quit",
            TimelinePhase::Operations,
            reducer(json!("quit")),
            Presentation::NotPresented,
        ),
        (
            "host_none",
            TimelinePhase::Operations,
            host(json!("none")),
            Presentation::Presented,
        ),
        (
            "host_effect",
            TimelinePhase::Operations,
            host(json!("reload")),
            Presentation::NotPresented,
        ),
        (
            "host_quit",
            TimelinePhase::Operations,
            host(json!("quit")),
            Presentation::NotPresented,
        ),
        (
            "liveness_session",
            TimelinePhase::FinalLiveness,
            session(json!("ignored"), json!({"event": "escape"})),
            Presentation::Presented,
        ),
        (
            "liveness_quit_value_ignored",
            TimelinePhase::FinalLiveness,
            session(json!("ignored"), json!({"terminal": "quit"})),
            Presentation::Presented,
        ),
        (
            "liveness_user_none",
            TimelinePhase::FinalLiveness,
            reducer(json!("none")),
            Presentation::Presented,
        ),
        (
            "liveness_user_effect",
            TimelinePhase::FinalLiveness,
            reducer(json!("reload")),
            Presentation::NotPresented,
        ),
        (
            "liveness_host_none",
            TimelinePhase::FinalLiveness,
            host(json!("none")),
            Presentation::Presented,
        ),
        (
            "liveness_host_effect",
            TimelinePhase::FinalLiveness,
            host(json!("reload")),
            Presentation::NotPresented,
        ),
        (
            "already_library",
            TimelinePhase::FinalLiveness,
            session(json!("already_terminal"), json!({"terminal": "library"})),
            Presentation::Presented,
        ),
        (
            "already_quit",
            TimelinePhase::FinalLiveness,
            session(json!("already_terminal"), json!({"terminal": "quit"})),
            Presentation::NotPresented,
        ),
    ];

    for (name, phase, cause, expected) in cases {
        let checkpoint = row(
            0,
            phase,
            None,
            None,
            TimelineBoundary::Initial,
            cause,
            expected,
        );
        assert_eq!(expected_presentation(&checkpoint), expected, "{name}");
    }
    assert_eq!(
        serde_json::to_value(Presentation::Presented).unwrap(),
        json!("presented")
    );
    assert_eq!(
        serde_json::to_value(Presentation::NotPresented).unwrap(),
        json!("not_presented")
    );
}

#[test]
fn timeline_schema_and_validator_reject_incorrect_presentation_labels() {
    let rows = complete_timeline();
    validate_timeline(&rows, 1).unwrap();

    for index in 0..rows.len() {
        let mut corrupt = rows.clone();
        corrupt[index].presentation = match corrupt[index].presentation {
            Presentation::Presented => Presentation::NotPresented,
            Presentation::NotPresented => Presentation::Presented,
        };
        relink(&mut corrupt);
        assert!(validate_timeline(&corrupt, 1).is_err(), "row {index}");
    }
    for schema in [1, 2] {
        let mut old = rows.clone();
        old[3].schema = schema;
        relink(&mut old);
        assert!(validate_timeline(&old, 1).is_err());
    }
}

#[test]
fn presentation_changes_are_pinned_by_row_and_chunk_digests() {
    let rows = complete_timeline();
    let profile = profile("en-80x24");
    let chunks = build_chunks(&profile, &rows, 2).unwrap();
    let original_row = timeline_row_digest(&rows[1]).unwrap();

    let mut changed = rows.clone();
    changed[1].presentation = Presentation::Presented;
    relink(&mut changed);
    let changed_chunks = build_chunks(&profile, &changed, 2).unwrap();

    assert_ne!(timeline_row_digest(&changed[1]).unwrap(), original_row);
    assert_ne!(changed_chunks[0].sha256, chunks[0].sha256);
    assert!(validate_chunks(&profile, &changed, &chunks, 2).is_err());
}

#[test]
#[allow(deprecated)]
fn styled_frame_to_buffer_is_lossless_for_every_style_and_diff_variant() {
    let colors = [
        Color::Reset,
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
        Color::Rgb(1, 2, 3),
        Color::Indexed(42),
    ];
    let area = Rect::new(3, 4, u16::try_from(colors.len()).unwrap(), 1);
    let mut buffer = Buffer::empty(area);
    for (cell, color) in buffer.content.iter_mut().zip(colors) {
        cell.set_symbol("x").set_fg(color);
    }
    buffer.content[0].modifier = Modifier::BOLD
        | Modifier::DIM
        | Modifier::ITALIC
        | Modifier::UNDERLINED
        | Modifier::SLOW_BLINK
        | Modifier::RAPID_BLINK
        | Modifier::REVERSED
        | Modifier::HIDDEN
        | Modifier::CROSSED_OUT;
    buffer.content[1]
        .set_bg(Color::Rgb(4, 5, 6))
        .set_diff_option(CellDiffOption::Skip);
    buffer.content[2].underline_color = Color::Indexed(7);
    buffer.content[2].set_diff_option(CellDiffOption::AlwaysUpdate);
    buffer.content[3].set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap()));
    buffer.content[4].set_skip(true);
    buffer.content[15].set_symbol("👩\u{200d}💻");

    let snapshot = StyledFrameSnapshot::from_buffer(
        &buffer,
        Position {
            x: area.x + 1,
            y: area.y,
        },
        true,
    );
    assert_eq!(snapshot.to_buffer().unwrap(), buffer);

    let mut corrupt = snapshot;
    corrupt.cells.pop();
    assert!(corrupt.to_buffer().is_err());
}

fn manifest_chunk(profile: &SafeProfileId, id: &str, start: u32, end: u32) -> ChunkDescriptor {
    ChunkDescriptor {
        id: id.to_owned(),
        profile: profile.clone(),
        start_sequence: start,
        end_sequence: end,
        row_count: end - start,
        sha256: ZERO_DIGEST.to_owned(),
        first_row_sha256: ZERO_DIGEST.to_owned(),
        last_row_sha256: ZERO_DIGEST.to_owned(),
        first_chain: None,
        last_chain: Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 0,
        }),
        continues_previous_chain: false,
        continues_next_chain: false,
    }
}

fn profile_manifest(id: &str, locale: &str, width: u16, height: u16) -> ProfileManifest {
    let profile = profile(id);
    ProfileManifest {
        id: profile.clone(),
        initial_locale: locale.to_owned(),
        initial_viewport: RectSnapshot {
            x: 0,
            y: 0,
            width,
            height,
        },
        operations_sha256: sha256_hex(b"operations"),
        cast_sha256: sha256_hex(format!("cast-{id}").as_bytes()),
        cast_byte_size: 1,
        row_count: 1,
        maximum_chunk_rows: 1,
        chunks: vec![manifest_chunk(&profile, &format!("{id}-0000"), 0, 1)],
    }
}

fn corpus_manifest() -> CorpusManifest {
    CorpusManifest {
        schema: 1,
        operations_sha256: sha256_hex(b"operations"),
        profiles: vec![
            profile_manifest("en-80x24", "en", 80, 24),
            profile_manifest("zh-cn-120x30", "zh-CN", 120, 30),
            profile_manifest("zh-tw-40x40", "zh-TW", 40, 40),
            profile_manifest("pseudo-120x12", "x-pseudo", 120, 12),
        ],
    }
}

fn assert_manifest_error(manifest: &CorpusManifest, expected: &str) {
    assert_eq!(
        validate_manifest(manifest).unwrap_err().to_string(),
        expected
    );
}

fn assert_chunk_range_error(profile: &ProfileManifest, expected: &str) {
    assert_eq!(
        validate_chunk_ranges(profile).unwrap_err().to_string(),
        expected
    );
}

#[test]
fn manifest_error_messages_and_precedence_are_stable() {
    let valid = corpus_manifest();
    validate_manifest(&valid).unwrap();

    let mut corrupt = valid.clone();
    corrupt.schema = 2;
    corrupt.operations_sha256 = "INVALID-OPERATIONS".to_owned();
    assert_manifest_error(&corrupt, "manifest has an unsupported schema");
    corrupt.schema = 1;
    assert_manifest_error(
        &corrupt,
        "invalid lowercase SHA-256 digest: INVALID-OPERATIONS",
    );

    let mut corrupt = valid.clone();
    corrupt.profiles.pop();
    assert_manifest_error(
        &corrupt,
        "manifest must contain the exact four review profiles",
    );

    let mut corrupt = valid.clone();
    corrupt.profiles[0].id = profile("other");
    corrupt.profiles[1].initial_locale = "other".to_owned();
    assert_manifest_error(&corrupt, "manifest profiles are not in the required order");

    let mut corrupt = valid.clone();
    corrupt.profiles[0].initial_locale = "other".to_owned();
    corrupt.profiles[0].initial_viewport.width = 1;
    assert_manifest_error(&corrupt, "manifest profile initial locale is invalid");
    corrupt.profiles[0].initial_locale = "en".to_owned();
    corrupt.profiles[0].operations_sha256 = ZERO_DIGEST.to_owned();
    assert_manifest_error(&corrupt, "manifest profile initial viewport is invalid");
    corrupt.profiles[0].initial_viewport.width = 80;
    corrupt.profiles[0].cast_sha256 = "INVALID-CAST".to_owned();
    assert_manifest_error(&corrupt, "manifest profile operation digest is invalid");
    corrupt.profiles[0].operations_sha256 = valid.operations_sha256.clone();
    corrupt.profiles[0].cast_byte_size = 0;
    assert_manifest_error(&corrupt, "invalid lowercase SHA-256 digest: INVALID-CAST");
    corrupt.profiles[0].cast_sha256 = ZERO_DIGEST.to_owned();
    corrupt.profiles[0].row_count = 0;
    assert_manifest_error(&corrupt, "manifest profile cast is empty");
    corrupt.profiles[0].cast_byte_size = 1;
    corrupt.profiles[0].chunks.clear();
    assert_manifest_error(&corrupt, "manifest profile has no rows");
    corrupt.profiles[0].row_count = 1;
    corrupt.profiles[0].maximum_chunk_rows = 0;
    assert_manifest_error(&corrupt, "manifest profile has no chunks");

    let mut corrupt = valid.clone();
    corrupt.profiles[0].maximum_chunk_rows = 0;
    corrupt.profiles[1].chunks[0].id = corrupt.profiles[0].chunks[0].id.clone();
    assert_manifest_error(&corrupt, "manifest chunk row limit must be nonzero");

    let mut corrupt = valid;
    corrupt.profiles[1].chunks[0].id = corrupt.profiles[0].chunks[0].id.clone();
    assert_manifest_error(&corrupt, "manifest chunk ids are not globally unique");
}

#[test]
fn chunk_range_error_messages_and_precedence_are_stable() {
    let valid = profile_manifest("en-80x24", "en", 80, 24);
    validate_chunk_ranges(&valid).unwrap();

    let mut corrupt = valid.clone();
    corrupt.maximum_chunk_rows = 0;
    corrupt.chunks[0].profile = profile("other");
    assert_chunk_range_error(&corrupt, "manifest chunk row limit must be nonzero");
    corrupt.maximum_chunk_rows = 1;
    corrupt.chunks[0].sha256 = "INVALID-CHUNK".to_owned();
    assert_chunk_range_error(&corrupt, "chunk profile or id is invalid");
    corrupt.chunks[0].profile = corrupt.id.clone();
    corrupt.chunks[0].first_row_sha256 = "INVALID-FIRST".to_owned();
    assert_chunk_range_error(&corrupt, "invalid lowercase SHA-256 digest: INVALID-CHUNK");
    corrupt.chunks[0].sha256 = ZERO_DIGEST.to_owned();
    corrupt.chunks[0].last_row_sha256 = "INVALID-LAST".to_owned();
    assert_chunk_range_error(&corrupt, "invalid lowercase SHA-256 digest: INVALID-FIRST");
    corrupt.chunks[0].first_row_sha256 = ZERO_DIGEST.to_owned();
    corrupt.chunks[0].start_sequence = 1;
    corrupt.chunks[0].continues_previous_chain = true;
    assert_chunk_range_error(&corrupt, "invalid lowercase SHA-256 digest: INVALID-LAST");
    corrupt.chunks[0].last_row_sha256 = ZERO_DIGEST.to_owned();
    assert_chunk_range_error(&corrupt, "manifest chunk ranges have a gap or overlap");
    corrupt.chunks[0].start_sequence = 0;
    assert_chunk_range_error(&corrupt, "manifest chunk continuation metadata is invalid");

    let mut mismatch = valid.clone();
    mismatch.row_count = 3;
    mismatch.maximum_chunk_rows = 1;
    let mut first = manifest_chunk(&mismatch.id, "first", 0, 1);
    first.continues_next_chain = true;
    let mut second = manifest_chunk(&mismatch.id, "second", 1, 2);
    second.first_chain = Some(EventChainIdentity {
        phase: TimelinePhase::Operations,
        sequence: 1,
    });
    second.last_chain = second.first_chain;
    second.continues_previous_chain = true;
    mismatch.chunks = vec![first, second];
    assert_chunk_range_error(&mismatch, "manifest chunk continuation mismatch");

    let mut uncovered = valid.clone();
    uncovered.row_count = 2;
    assert_chunk_range_error(
        &uncovered,
        "manifest chunks do not cover their profile rows",
    );

    let mut duplicate = valid;
    duplicate.row_count = 2;
    let mut second = manifest_chunk(&duplicate.id, &duplicate.chunks[0].id, 1, 2);
    second.first_chain = Some(EventChainIdentity {
        phase: TimelinePhase::Operations,
        sequence: 1,
    });
    second.last_chain = second.first_chain;
    duplicate.chunks.push(second);
    assert_chunk_range_error(&duplicate, "chunk profile or id is invalid");
}
