use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use serde_json::{Value, json};

use crate::sandbox::{SafeProfileId, SandboxMetadata, SandboxPlatform};
use crate::{
    ChunkDescriptor, EventChainIdentity, ObjectKind, ObjectRef, Presentation, ProfileManifest,
    RectSnapshot, StyledCellSnapshot, StyledFrameSnapshot, TimelineBoundary, TimelinePhase,
    TimelineRow, TransitionCause, canonical_json_bytes, sha256_hex,
};

use crate::bundle::{
    BundleLayout, CAST_FILE, CHUNK_DIRECTORY, ChunkViewCursor, FINAL_LIVENESS_FILE,
    FRAME_VIEW_DIRECTORY, MANIFEST_FILE, OBJECT_DIRECTORY, OPERATIONS_FILE, PROFILE_DIRECTORY,
    PROFILE_FILE, ProfileExpectation, README_FILE, REVIEW_CORPUS_DIRECTORY_PREFIX, REVIEW_FILE,
    REVIEW_PROGRESS_FILE, REVIEWER_MUTABLE_FILES, RUN_FILE, RunMetadata, TIMELINE_FILE,
    VIEW_DIRECTORY, bundle_digest, cast_canvas, chunk_json_bytes, chunk_json_path, chunk_view_path,
    compare_regenerated_corpus, decode_canonical_bytes, decode_timeline_ndjson, frame_view_bytes,
    is_reviewer_mutable, kind_directory, maximum_canvas, object_path, object_view_bytes,
    object_view_path, profile_directory, rebuild_presented_cast, review_corpus_directory_name,
    source_revision, timeline_ndjson_bytes, validate_profile_manifest, validate_review_report,
    validate_run_metadata,
};

const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

static SMOKE_PROFILE: LazyLock<SafeProfileId> =
    LazyLock::new(|| SafeProfileId::try_from("smoke").unwrap());
static OTHER_PROFILE: LazyLock<SafeProfileId> =
    LazyLock::new(|| SafeProfileId::try_from("other").unwrap());

fn safe_profile(value: &str) -> SafeProfileId {
    SafeProfileId::try_from(value).unwrap()
}

fn sandbox_metadata(profile_ids: &[&str]) -> SandboxMetadata {
    let profiles = profile_ids
        .iter()
        .map(|profile_id| safe_profile(profile_id))
        .collect::<BTreeSet<_>>();
    SandboxMetadata::stable(SandboxPlatform::Linux, "/tmp/skit-ui-walker-v1", profiles).unwrap()
}

fn digest(marker: char) -> String {
    marker.to_string().repeat(64)
}

fn reference(kind: ObjectKind, marker: char) -> ObjectRef {
    ObjectRef {
        kind,
        sha256: digest(marker),
    }
}

fn references() -> [ObjectRef; 5] {
    [
        reference(ObjectKind::Reducer, 'a'),
        reference(ObjectKind::Host, 'b'),
        reference(ObjectKind::Session, 'c'),
        reference(ObjectKind::StyledFrame, 'd'),
        reference(ObjectKind::Geometry, 'e'),
    ]
}

fn cell(x: u16, y: u16, symbol: &str) -> StyledCellSnapshot {
    StyledCellSnapshot {
        x,
        y,
        symbol: symbol.to_owned(),
        foreground: crate::ColorSnapshot::Reset,
        background: crate::ColorSnapshot::Reset,
        underline: crate::ColorSnapshot::Reset,
        modifiers: Vec::new(),
        diff: crate::CellDiffSnapshot::None,
        skip: false,
    }
}

fn styled_frame(label: &str) -> StyledFrameSnapshot {
    let mut cells = vec![
        cell(0, 0, label),
        cell(1, 0, "\""),
        cell(2, 0, "B"),
        cell(3, 0, " "),
        cell(0, 1, " "),
        cell(1, 1, " "),
        cell(2, 1, " "),
        cell(3, 1, " "),
    ];
    cells[1].foreground = crate::ColorSnapshot::Red;
    cells[2].foreground = crate::ColorSnapshot::Red;
    StyledFrameSnapshot {
        area: RectSnapshot {
            x: 0,
            y: 0,
            width: 4,
            height: 2,
        },
        cursor_position: crate::PositionSnapshot { x: 3, y: 1 },
        cursor_visible: true,
        cells,
    }
}

fn timeline_row(sequence: u32, refs: &[ObjectRef; 5]) -> TimelineRow {
    let initial = sequence == 0;
    TimelineRow {
        schema: 3,
        profile: safe_profile("smoke"),
        sequence,
        phase: TimelinePhase::Operations,
        event_chain: sequence.checked_sub(1).map(|sequence| EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence,
        }),
        operation_index: (!initial).then_some(0),
        boundary: if initial {
            TimelineBoundary::Initial
        } else {
            TimelineBoundary::Session
        },
        cause: if initial {
            TransitionCause::Initial
        } else {
            TransitionCause::Session {
                requested: json!({"operation": "click"}),
                resolved: json!({"semantic_target": {"command": "open"}}),
                event: json!({"key": "enter"}),
                handling: json!("consumed"),
            }
        },
        presentation: Presentation::Presented,
        reducer: refs[0].clone(),
        host: refs[1].clone(),
        session: refs[2].clone(),
        styled_frame: refs[3].clone(),
        geometry: refs[4].clone(),
        locale: "en".to_owned(),
        viewport: RectSnapshot {
            x: 0,
            y: 0,
            width: 4,
            height: 2,
        },
        liveness: None,
        previous_row_sha256: None,
    }
}

fn chunk(id: &str, start: u32, end: u32) -> ChunkDescriptor {
    ChunkDescriptor {
        id: id.to_owned(),
        profile: safe_profile("smoke"),
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

fn profile_manifest() -> ProfileManifest {
    ProfileManifest {
        id: safe_profile("smoke"),
        initial_locale: "en".to_owned(),
        initial_viewport: RectSnapshot {
            x: 0,
            y: 0,
            width: 4,
            height: 2,
        },
        operations_sha256: digest('a'),
        cast_sha256: digest('b'),
        cast_byte_size: 1,
        row_count: 1,
        maximum_chunk_rows: 24,
        chunks: vec![chunk("smoke-0000", 0, 1)],
    }
}

fn expectation() -> ProfileExpectation<'static> {
    ProfileExpectation {
        id: &SMOKE_PROFILE,
        locale: "en",
        viewport: RectSnapshot {
            x: 0,
            y: 0,
            width: 4,
            height: 2,
        },
        operations_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    }
}

#[test]
fn paths_cover_every_kind_and_the_frame_view_asymmetry() {
    assert_eq!(OBJECT_DIRECTORY, "objects");
    assert_eq!(VIEW_DIRECTORY, "views");
    assert_eq!(FRAME_VIEW_DIRECTORY, "frames");
    assert_eq!(PROFILE_DIRECTORY, "profiles");
    assert_eq!(CHUNK_DIRECTORY, "chunks");
    assert_eq!(PROFILE_FILE, "profile.json");
    assert_eq!(TIMELINE_FILE, "timeline.ndjson");
    assert_eq!(CAST_FILE, "trace.cast");
    assert_eq!(OPERATIONS_FILE, "operations.json");
    assert_eq!(FINAL_LIVENESS_FILE, "final-liveness.json");
    assert_eq!(RUN_FILE, "run.json");

    for (kind, directory) in [
        (ObjectKind::Reducer, "reducer"),
        (ObjectKind::Host, "host"),
        (ObjectKind::Session, "session"),
        (ObjectKind::StyledFrame, "styled-frame"),
        (ObjectKind::Geometry, "geometry"),
    ] {
        let reference = reference(kind, 'a');
        assert_eq!(kind_directory(kind), directory);
        assert_eq!(
            object_path(&reference),
            PathBuf::from("objects")
                .join(directory)
                .join(format!("{}.json", reference.sha256))
        );
        let expected_view = if kind == ObjectKind::StyledFrame {
            PathBuf::from("views/frames").join(format!("{}.txt", reference.sha256))
        } else {
            PathBuf::from("views")
                .join(directory)
                .join(format!("{}.json", reference.sha256))
        };
        assert_eq!(object_view_path(&reference), expected_view);
    }

    assert_eq!(
        profile_directory(&SMOKE_PROFILE),
        PathBuf::from("profiles/smoke")
    );
    assert_eq!(
        chunk_json_path(&SMOKE_PROFILE, "smoke-0000"),
        PathBuf::from("profiles/smoke/chunks/smoke-0000.json")
    );
    assert_eq!(
        chunk_view_path(&SMOKE_PROFILE, "smoke-0000"),
        PathBuf::from("profiles/smoke/chunks/smoke-0000.md")
    );
}

#[test]
fn readable_object_and_frame_views_match_the_legacy_bytes() {
    assert_eq!(
        object_view_bytes(&json!({"z": 1, "a": {"b": 2}})).unwrap(),
        b"{\n  \"a\": {\n    \"b\": 2\n  },\n  \"z\": 1\n}\n"
    );

    let expected = concat!(
        "size: 4x2\n",
        "cursor: visible at column 3, row 1\n\n",
        "000: \"A\\\"B\"\n",
        "001: \"\"\n\n",
        "styles:\n",
        "- row 0, columns 1..3: {\"background\":{\"kind\":\"reset\"},\"diff\":{\"kind\":\"none\"},\"foreground\":{\"kind\":\"red\"},\"modifiers\":[],\"skip\":false,\"underline\":{\"kind\":\"reset\"}}\n",
    );
    assert_eq!(
        frame_view_bytes(&styled_frame("A")).unwrap(),
        expected.as_bytes()
    );
    assert!(!expected.contains("columns 3..4"));

    let mut hidden = styled_frame("A");
    hidden.cursor_visible = false;
    assert!(
        String::from_utf8(frame_view_bytes(&hidden).unwrap())
            .unwrap()
            .starts_with("size: 4x2\ncursor: hidden\n")
    );
}

#[test]
fn timeline_and_chunk_json_bytes_are_canonical_and_round_trip() {
    let refs = references();
    let rows = vec![timeline_row(0, &refs), timeline_row(1, &refs)];
    let bytes = timeline_ndjson_bytes(&rows).unwrap();
    assert!(bytes.ends_with(b"\n"));
    assert!(!bytes.ends_with(b"\n\n"));
    assert_eq!(decode_timeline_ndjson(&bytes).unwrap(), rows);

    let expected_chunk = canonical_json_bytes(&serde_json::to_value(&rows).unwrap()).unwrap();
    assert_eq!(chunk_json_bytes(&rows).unwrap(), expected_chunk);
    assert!(!expected_chunk.ends_with(b"\n"));

    let canonical = br#"{"a":1,"b":2}"#;
    assert_eq!(
        decode_canonical_bytes(canonical).unwrap(),
        json!({"a": 1, "b": 2})
    );
    assert_eq!(
        decode_canonical_bytes(br#"{ "a": 1, "b": 2 }"#)
            .unwrap_err()
            .to_string(),
        "JSON bytes are not canonical"
    );
}

#[test]
fn timeline_decoder_rejects_each_newline_and_canonicality_violation() {
    let refs = references();
    let rows = vec![timeline_row(0, &refs), timeline_row(1, &refs)];
    let bytes = timeline_ndjson_bytes(&rows).unwrap();

    assert_eq!(
        decode_timeline_ndjson(&bytes[..bytes.len() - 1])
            .unwrap_err()
            .to_string(),
        "timeline must have exactly one trailing newline"
    );
    let mut two_trailing = bytes.clone();
    two_trailing.push(b'\n');
    assert_eq!(
        decode_timeline_ndjson(&two_trailing)
            .unwrap_err()
            .to_string(),
        "timeline must have exactly one trailing newline"
    );

    let first_newline = bytes.iter().position(|byte| *byte == b'\n').unwrap();
    let mut empty_line = bytes[..=first_newline].to_vec();
    empty_line.push(b'\n');
    empty_line.extend_from_slice(&bytes[first_newline + 1..]);
    assert_eq!(
        decode_timeline_ndjson(&empty_line).unwrap_err().to_string(),
        "timeline contains an empty row"
    );

    let mut noncanonical = vec![b' '];
    noncanonical.extend_from_slice(&bytes);
    assert_eq!(
        decode_timeline_ndjson(&noncanonical)
            .unwrap_err()
            .to_string(),
        "timeline row is not canonical JSON"
    );
}

#[test]
fn chunk_views_pin_markdown_and_keep_first_appearance_per_cursor() {
    let refs = references();
    let mut rows = vec![timeline_row(0, &refs), timeline_row(1, &refs)];
    rows[1].presentation = Presentation::NotPresented;
    rows[1].host = reference(ObjectKind::Host, 'f');
    rows[1].geometry = reference(ObjectKind::Geometry, '0');
    let first = chunk("smoke-0000", 0, 1);
    let mut second = chunk("smoke-0001", 1, 2);
    second.first_chain = Some(EventChainIdentity {
        phase: TimelinePhase::Operations,
        sequence: 0,
    });

    let mut cursor = ChunkViewCursor::new();
    let first_bytes = cursor.chunk_view_bytes(&rows, &first).unwrap();
    let expected = format!(
        concat!(
            "# smoke-0000\n\n",
            "checkpoints: 0..1\n",
            "chain: first=None last=Some(EventChainIdentity {{ phase: Operations, sequence: 0 }}) continues_previous=false continues_next=false\n\n",
            "- checkpoint 0: phase=Operations boundary=Initial presentation=presented operation=None changed=reducer,host,session,frame,geometry\n",
            "  - cause: initial\n",
            "  - first object: objects/reducer/{}.json\n",
            "    readable: views/reducer/{}.json\n",
            "  - first object: objects/host/{}.json\n",
            "    readable: views/host/{}.json\n",
            "  - first object: objects/session/{}.json\n",
            "    readable: views/session/{}.json\n",
            "  - first object: objects/styled-frame/{}.json\n",
            "    readable: views/frames/{}.txt\n",
            "  - first object: objects/geometry/{}.json\n",
            "    readable: views/geometry/{}.json\n",
        ),
        refs[0].sha256,
        refs[0].sha256,
        refs[1].sha256,
        refs[1].sha256,
        refs[2].sha256,
        refs[2].sha256,
        refs[3].sha256,
        refs[3].sha256,
        refs[4].sha256,
        refs[4].sha256,
    );
    assert_eq!(first_bytes, expected.as_bytes());

    let continuation = cursor.chunk_view_bytes(&rows, &second).unwrap();
    let continuation = String::from_utf8(continuation).unwrap();
    let checkpoint = continuation
        .lines()
        .find(|line| line.starts_with("- checkpoint 1:"))
        .unwrap();
    assert_eq!(
        checkpoint,
        "- checkpoint 1: phase=Operations boundary=Session presentation=not_presented operation=Some(0) changed=host,geometry"
    );
    assert!(
        continuation.contains(
            "  - cause: session operation=click event=key resolved=open handling=consumed"
        )
    );
    assert_eq!(continuation.matches("first object").count(), 2);
    assert!(continuation.contains(&format!(
        "first object: objects/host/{}.json",
        rows[1].host.sha256
    )));
    assert!(continuation.contains(&format!(
        "first object: objects/geometry/{}.json",
        rows[1].geometry.sha256
    )));

    let mut independent = ChunkViewCursor::new();
    let independent = independent.chunk_view_bytes(&rows, &second).unwrap();
    assert_eq!(
        String::from_utf8(independent)
            .unwrap()
            .matches("first object")
            .count(),
        5
    );

    let outside = chunk("outside", 99, 100);
    assert_eq!(
        cursor
            .chunk_view_bytes(&rows, &outside)
            .unwrap_err()
            .to_string(),
        "chunk range is outside the timeline"
    );
}

#[test]
fn maximum_canvas_is_component_wise_and_order_independent() {
    let initial = (80, 24);
    let viewports = [(120, 12), (40, 40)];
    assert_eq!(
        viewports.into_iter().fold(initial, maximum_canvas),
        (120, 40)
    );
    assert_eq!(
        viewports.into_iter().rev().fold(initial, maximum_canvas),
        (120, 40)
    );
    assert_eq!(maximum_canvas(initial, initial), initial);
}

#[test]
fn cast_canvas_and_rebuild_use_all_rows_and_only_presented_frames() {
    let refs = references();
    let mut rows = vec![
        timeline_row(0, &refs),
        timeline_row(1, &refs),
        timeline_row(2, &refs),
    ];
    rows[0].viewport.width = 4;
    rows[0].viewport.height = 8;
    rows[1].viewport.width = 10;
    rows[1].viewport.height = 3;
    rows[2].viewport.width = 6;
    rows[2].viewport.height = 5;
    assert_eq!(cast_canvas(&rows).unwrap(), (10, 8));
    assert_eq!(
        cast_canvas(&[]).unwrap_err().to_string(),
        "cannot build a cast for an empty timeline"
    );

    for row in &mut rows {
        row.viewport.width = 4;
        row.viewport.height = 2;
    }
    rows[0].styled_frame = reference(ObjectKind::StyledFrame, 'a');
    rows[1].styled_frame = reference(ObjectKind::StyledFrame, 'b');
    rows[1].presentation = Presentation::NotPresented;
    rows[2].styled_frame = reference(ObjectKind::StyledFrame, 'c');
    let cast = rebuild_presented_cast(&rows, |reference| match reference.sha256.as_bytes()[0] {
        b'a' => Ok(styled_frame("A")),
        b'b' => panic!("the loader must not read a non-presented frame"),
        b'c' => Ok(styled_frame("C")),
        _ => Err("unknown frame".to_owned()),
    })
    .unwrap();
    let events = cast
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["term"], json!({"cols": 4, "rows": 2}));
    assert_eq!(events[2][0], json!(0.2));

    let error =
        rebuild_presented_cast(&rows[..1], |_| Err("frame load failed".to_owned())).unwrap_err();
    assert_eq!(error.to_string(), "frame load failed");
}

#[test]
#[allow(clippy::clone_on_copy)]
fn profile_manifest_validation_checks_every_field_and_expectation() {
    let valid = profile_manifest();
    let expected = expectation();
    assert_eq!(expected.clone(), expectation());
    assert_eq!(
        format!("{expected:?}"),
        "ProfileExpectation { id: SafeProfileId(\"smoke\"), locale: \"en\", viewport: RectSnapshot { x: 0, y: 0, width: 4, height: 2 }, operations_sha256: \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" }"
    );
    validate_profile_manifest(&valid, expected).unwrap();

    let mutations: [fn(&mut ProfileManifest); 9] = [
        |profile| profile.id = safe_profile("other"),
        |profile| profile.initial_locale = "other".to_owned(),
        |profile| profile.initial_viewport.width = 9,
        |profile| profile.operations_sha256 = digest('c'),
        |profile| profile.cast_sha256 = "invalid".to_owned(),
        |profile| profile.cast_byte_size = 0,
        |profile| profile.row_count = 0,
        |profile| profile.maximum_chunk_rows = 0,
        |profile| profile.chunks.clear(),
    ];
    for mutate in mutations {
        let mut corrupt = valid.clone();
        mutate(&mut corrupt);
        assert!(validate_profile_manifest(&corrupt, expected).is_err());
    }

    for mismatch in [
        ProfileExpectation {
            id: &OTHER_PROFILE,
            ..expected
        },
        ProfileExpectation {
            locale: "other",
            ..expected
        },
        ProfileExpectation {
            viewport: RectSnapshot {
                width: 9,
                ..expected.viewport
            },
            ..expected
        },
        ProfileExpectation {
            operations_sha256: ZERO_DIGEST,
            ..expected
        },
    ] {
        assert!(validate_profile_manifest(&valid, mismatch).is_err());
    }
}

fn run_metadata() -> RunMetadata {
    RunMetadata {
        schema: 2,
        source_revision: "revision".to_owned(),
        deterministic_result: "passed".to_owned(),
        operation_count: 2,
        operations_sha256: digest('a'),
        final_liveness_sha256: digest('b'),
        profiles: vec![safe_profile("a"), safe_profile("b")],
        sandbox: sandbox_metadata(&["a", "b"]),
    }
}

#[test]
fn run_metadata_validation_rejects_every_invalid_contract() {
    let valid = run_metadata();
    validate_run_metadata(&valid).unwrap();

    let mut corrupt = valid.clone();
    corrupt.schema = 1;
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.deterministic_result = "failed".to_owned();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.source_revision.clear();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.operation_count = 0;
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.operations_sha256 = "invalid".to_owned();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.final_liveness_sha256 = "invalid".to_owned();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.profiles.clear();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.profiles = vec![safe_profile("b"), safe_profile("a")];
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid;
    corrupt.profiles = vec![safe_profile("a"), safe_profile("a")];
    assert_eq!(
        validate_run_metadata(&corrupt).unwrap_err().to_string(),
        "run metadata profiles do not match sandbox metadata"
    );
}

#[test]
fn bundle_digest_is_stable_and_sensitive_to_profiles_and_revision() {
    let mut left = BTreeMap::new();
    left.insert(safe_profile("b"), digest('b'));
    left.insert(safe_profile("a"), digest('a'));
    let mut right = BTreeMap::new();
    right.insert(safe_profile("a"), digest('a'));
    right.insert(safe_profile("b"), digest('b'));
    let sandbox = sandbox_metadata(&["a", "b"]);

    let baseline = bundle_digest(&left, "revision", &sandbox).unwrap();
    assert_eq!(
        baseline,
        bundle_digest(&right, "revision", &sandbox).unwrap()
    );
    right.insert(safe_profile("b"), digest('c'));
    assert_ne!(
        baseline,
        bundle_digest(&right, "revision", &sandbox).unwrap()
    );
    assert_ne!(baseline, bundle_digest(&left, "other", &sandbox).unwrap());
}

#[test]
fn source_revision_matches_the_legacy_clean_and_dirty_byte_shape() {
    assert_eq!(
        source_revision(" head\n", &[], &[], |_| {
            panic!("a clean tree must not read an untracked file")
        })
        .unwrap(),
        "head"
    );

    assert_eq!(
        source_revision("head\n", b"diff", &[], |_| unreachable!()).unwrap(),
        format!("head+worktree:{}", sha256_hex(b"diff"))
    );

    let mut expected_bytes = b"diff".to_vec();
    for (path, contents) in [("a.txt", b"A".as_slice()), ("b.bin", b"BC".as_slice())] {
        expected_bytes.extend_from_slice(&(path.len() as u64).to_be_bytes());
        expected_bytes.extend_from_slice(path.as_bytes());
        expected_bytes.extend_from_slice(&(contents.len() as u64).to_be_bytes());
        expected_bytes.extend_from_slice(contents);
    }
    let mut reads = Vec::new();
    let revision = source_revision("head", b"diff", b"a.txt\0b.bin\0", |path| {
        reads.push(path.to_owned());
        match path {
            "a.txt" => Ok(b"A".to_vec()),
            "b.bin" => Ok(b"BC".to_vec()),
            _ => Err("unexpected path".to_owned()),
        }
    })
    .unwrap();
    assert_eq!(reads, ["a.txt", "b.bin"]);
    assert_eq!(
        revision,
        format!("head+worktree:{}", sha256_hex(&expected_bytes))
    );

    assert_eq!(
        source_revision("head", &[], b"lost\0", |_| Err("read failed".to_owned()))
            .unwrap_err()
            .to_string(),
        "read failed"
    );
    assert!(source_revision("head", &[], &[0xff, 0], |_| Ok(Vec::new())).is_err());
}

#[test]
fn bundle_layout_derives_the_exact_tree_from_profiles_and_rows() {
    let profile = profile_manifest();
    let profile_id = profile.id.clone();
    let refs = references();
    let rows = vec![timeline_row(0, &refs)];
    let mut actual = BundleLayout::default();
    actual.expect_bundle(&profile, &rows).unwrap();

    let mut expected = BundleLayout::default();
    for file in [RUN_FILE, OPERATIONS_FILE, FINAL_LIVENESS_FILE] {
        expected.insert_file(file);
    }
    for directory in [OBJECT_DIRECTORY, VIEW_DIRECTORY, PROFILE_DIRECTORY] {
        expected.insert_directory(directory);
    }
    for kind in [
        ObjectKind::Reducer,
        ObjectKind::Host,
        ObjectKind::Session,
        ObjectKind::StyledFrame,
        ObjectKind::Geometry,
    ] {
        expected.insert_directory(PathBuf::from(OBJECT_DIRECTORY).join(kind_directory(kind)));
        if kind != ObjectKind::StyledFrame {
            expected.insert_directory(PathBuf::from(VIEW_DIRECTORY).join(kind_directory(kind)));
        }
    }
    expected.insert_directory(PathBuf::from(VIEW_DIRECTORY).join(FRAME_VIEW_DIRECTORY));
    expected.insert_directory(profile_directory(&profile_id));
    expected.insert_directory(profile_directory(&profile_id).join(CHUNK_DIRECTORY));
    expected.insert_file(profile_directory(&profile_id).join(PROFILE_FILE));
    expected.insert_file(profile_directory(&profile_id).join(TIMELINE_FILE));
    expected.insert_file(profile_directory(&profile_id).join(CAST_FILE));
    for descriptor in &profile.chunks {
        expected.insert_file(chunk_json_path(&profile_id, &descriptor.id));
        expected.insert_file(chunk_view_path(&profile_id, &descriptor.id));
    }
    for reference in &refs {
        expected.insert_file(object_path(reference));
        expected.insert_file(object_view_path(reference));
    }

    assert_eq!(actual, expected);
    actual.compare(&expected).unwrap();
    assert!(
        !actual
            .directories
            .contains(&PathBuf::from("views/styled-frame"))
    );
}

#[test]
fn bundle_layout_comparison_reports_missing_and_extra_entries() {
    let mut expected = BundleLayout::default();
    expected.insert_file("required.json");
    expected.insert_directory("required");

    let actual = BundleLayout::default();
    assert_eq!(
        expected.compare(&actual).unwrap_err().to_string(),
        "bundle is missing file: required.json"
    );

    let mut actual = expected.clone();
    actual.insert_file("extra.json");
    assert_eq!(
        expected.compare(&actual).unwrap_err().to_string(),
        "bundle has an extra file: extra.json"
    );

    let mut actual = BundleLayout::default();
    actual.insert_file("required.json");
    assert_eq!(
        expected.compare(&actual).unwrap_err().to_string(),
        "bundle is missing directory: required"
    );

    let mut actual = expected.clone();
    actual.insert_directory("extra");
    assert_eq!(
        expected.compare(&actual).unwrap_err().to_string(),
        "bundle has an extra directory: extra"
    );
}

#[test]
fn chunk_views_cover_reducer_host_and_fallback_cause_summaries() {
    let refs = references();
    let mut rows = vec![
        timeline_row(0, &refs),
        timeline_row(1, &refs),
        timeline_row(2, &refs),
    ];
    rows[1].boundary = TimelineBoundary::UserAction;
    rows[1].presentation = Presentation::NotPresented;
    rows[1].cause = TransitionCause::Reducer {
        requested: json!({"synthetic": "final_liveness"}),
        resolved: json!({"input": {"target": "footer"}}),
        event: json!("enter"),
        action: json!({"open": true}),
        emitted: json!(7),
    };
    rows[2].boundary = TimelineBoundary::HostAction;
    rows[2].cause = TransitionCause::Host {
        operation_index: Some(0),
        round: 0,
        request: json!({"reload": true}),
        response: json!("surface"),
        emitted: Value::Null,
    };

    let descriptor = chunk("causes", 0, 3);
    let view = String::from_utf8(
        ChunkViewCursor::new()
            .chunk_view_bytes(&rows, &descriptor)
            .unwrap(),
    )
    .unwrap();
    assert!(view.contains(
        "cause: reducer operation=final_liveness event=enter resolved=footer action=open effect=unknown"
    ));
    assert!(view.contains("cause: host round=0 request=reload response=surface effect=unknown"));
}

#[test]
fn reviewer_mutable_files_are_exactly_the_two_root_level_review_files() {
    assert_eq!(
        REVIEWER_MUTABLE_FILES,
        ["review.md", "review-progress.json"]
    );
    for name in REVIEWER_MUTABLE_FILES {
        assert!(is_reviewer_mutable(Path::new(name)), "{name}");
    }

    for lookalike in [
        "",
        "./review.md",
        "./review-progress.json",
        "review.md/",
        "/review.md",
        "review.md.bak",
        "Review.md",
        "review-progress.json.tmp",
        "profiles/smoke/chunks/review.md",
        "profiles/smoke/review-progress.json",
        README_FILE,
        MANIFEST_FILE,
        RUN_FILE,
    ] {
        assert!(!is_reviewer_mutable(Path::new(lookalike)), "{lookalike}");
    }
}

#[test]
fn review_corpus_directory_name_binds_the_prefix_to_a_valid_digest() {
    assert_eq!(REVIEW_CORPUS_DIRECTORY_PREFIX, "corpus-");
    assert_eq!(
        review_corpus_directory_name(&digest('a')).unwrap(),
        format!("corpus-{}", digest('a'))
    );
    assert_eq!(
        review_corpus_directory_name(ZERO_DIGEST).unwrap(),
        format!("{REVIEW_CORPUS_DIRECTORY_PREFIX}{ZERO_DIGEST}")
    );

    let uppercase = digest('a').to_uppercase();
    let short = digest('a')[1..].to_owned();
    let long = format!("{}0", digest('a'));
    let nonhex = digest('g');
    for invalid in [
        "",
        "invalid",
        uppercase.as_str(),
        short.as_str(),
        long.as_str(),
        nonhex.as_str(),
    ] {
        assert_eq!(
            review_corpus_directory_name(invalid)
                .unwrap_err()
                .to_string(),
            format!("invalid lowercase SHA-256 digest: {invalid}")
        );
    }
}

fn corpus_files(entries: &[(&str, &str)]) -> BTreeMap<PathBuf, Vec<u8>> {
    entries
        .iter()
        .map(|(path, text)| (PathBuf::from(path), text.as_bytes().to_vec()))
        .collect()
}

fn regenerated_corpus() -> BTreeMap<PathBuf, Vec<u8>> {
    corpus_files(&[
        (RUN_FILE, "run"),
        (README_FILE, "guide"),
        ("profiles/smoke/chunks/review.md", "chunk"),
        (REVIEW_FILE, "# UI review\n"),
        (REVIEW_PROGRESS_FILE, "{}\n"),
    ])
}

#[test]
fn regenerated_corpus_comparison_accepts_only_reviewer_edits() {
    let regenerated = regenerated_corpus();
    compare_regenerated_corpus(&regenerated, &regenerated).unwrap();

    let mut edited = regenerated.clone();
    edited.insert(
        PathBuf::from(REVIEW_FILE),
        b"# UI review\n\nFinding.\n".to_vec(),
    );
    edited.insert(
        PathBuf::from(REVIEW_PROGRESS_FILE),
        b"{\"complete\":true}\n".to_vec(),
    );
    compare_regenerated_corpus(&regenerated, &edited).unwrap();
}

#[test]
fn regenerated_corpus_comparison_refuses_every_immutable_difference() {
    let regenerated = regenerated_corpus();

    for immutable in [RUN_FILE, README_FILE, "profiles/smoke/chunks/review.md"] {
        let mut changed = regenerated.clone();
        changed.insert(PathBuf::from(immutable), b"other".to_vec());
        assert_eq!(
            compare_regenerated_corpus(&regenerated, &changed)
                .unwrap_err()
                .to_string(),
            format!("review corpus changed an immutable file: {immutable}")
        );
    }

    for missing in [RUN_FILE, REVIEW_FILE, REVIEW_PROGRESS_FILE] {
        let mut installed = regenerated.clone();
        installed.remove(Path::new(missing));
        assert_eq!(
            compare_regenerated_corpus(&regenerated, &installed)
                .unwrap_err()
                .to_string(),
            format!("review corpus is missing a file: {missing}")
        );

        let mut staged = regenerated.clone();
        staged.remove(Path::new(missing));
        assert_eq!(
            compare_regenerated_corpus(&staged, &regenerated)
                .unwrap_err()
                .to_string(),
            format!("review corpus has an extra file: {missing}")
        );
    }

    let mut extra = regenerated.clone();
    extra.insert(PathBuf::from(MANIFEST_FILE), b"manifest".to_vec());
    assert_eq!(
        compare_regenerated_corpus(&regenerated, &extra)
            .unwrap_err()
            .to_string(),
        format!("review corpus has an extra file: {MANIFEST_FILE}")
    );

    let empty = BTreeMap::new();
    compare_regenerated_corpus(&empty, &empty).unwrap();
    assert_eq!(
        compare_regenerated_corpus(&empty, &regenerated)
            .unwrap_err()
            .to_string(),
        format!("review corpus has an extra file: {README_FILE}")
    );
}

#[test]
fn review_report_format_accepts_one_trailing_newline_and_refuses_every_other_shape() {
    for report in [
        b"# UI review\n".as_slice(),
        b"\n".as_slice(),
        "# UI review\n\n\u{7f16}\u{8f91}\u{3002}\n".as_bytes(),
    ] {
        validate_review_report(report).unwrap();
    }

    for (report, message) in [
        (
            b"# UI review\n\n\xff\n".as_slice(),
            "the UI review report is not UTF-8",
        ),
        (b"".as_slice(), "the UI review report is empty"),
        (
            b"# UI review\n\nFinding.".as_slice(),
            "the UI review report must have exactly one trailing newline",
        ),
        (
            b"# UI review\n\nFinding.\n\n".as_slice(),
            "the UI review report must have exactly one trailing newline",
        ),
    ] {
        assert_eq!(
            validate_review_report(report).unwrap_err().to_string(),
            message
        );
    }
}
