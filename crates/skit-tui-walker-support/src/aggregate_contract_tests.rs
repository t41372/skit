use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use serde_json::{Value, json};

use crate::{
    ChunkDescriptor, CorpusManifest, ObjectKind, ObjectRef, Presentation, ProfileManifest,
    RectSnapshot, ReviewProgress, ReviewVerdict, TimelineBoundary, TimelinePhase, TimelineRow,
    TransitionCause, canonical_json_bytes, decode_manifest, decode_review_progress, manifest_bytes,
    manifest_digest, required_review_profiles, review_progress_bytes, sha256_hex,
    validate_completed_review_claim, validate_manifest,
};
use crate::{
    bundle::{
        BundleLayout, CAST_FILE, COVERAGE_FILE, FINAL_LIVENESS_FILE, MANIFEST_FILE,
        OPERATIONS_FILE, PROFILE_FILE, README_FILE, REVIEW_FILE, REVIEW_GUIDE,
        REVIEW_PROGRESS_FILE, REVIEW_TEMPLATE, RUN_FILE, RunMetadata, TIMELINE_FILE, object_path,
        object_view_path, profile_directory, validate_review_corpus_manifest,
        validate_review_corpus_run, validate_review_files, validate_run_metadata,
    },
    sandbox::{SafeProfileId, SandboxMetadata, SandboxPlatform},
};

const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const PROFILES: [(&str, &str, u16, u16); 4] = [
    ("en-80x24", "en", 80, 24),
    ("zh-cn-120x30", "zh-CN", 120, 30),
    ("zh-tw-40x40", "zh-TW", 40, 40),
    ("pseudo-120x12", "x-pseudo", 120, 12),
];

fn safe_profile(id: &str) -> SafeProfileId {
    SafeProfileId::try_from(id).unwrap()
}

fn profile_manifest(id: &str, locale: &str, width: u16, height: u16) -> ProfileManifest {
    let id = safe_profile(id);
    ProfileManifest {
        id: id.clone(),
        initial_locale: locale.to_owned(),
        initial_viewport: RectSnapshot {
            x: 0,
            y: 0,
            width,
            height,
        },
        operations_sha256: ZERO_DIGEST.to_owned(),
        cast_sha256: ZERO_DIGEST.to_owned(),
        cast_byte_size: 1,
        row_count: 1,
        maximum_chunk_rows: 24,
        chunks: vec![ChunkDescriptor {
            id: format!("{id}-0000"),
            profile: id,
            start_sequence: 0,
            end_sequence: 1,
            row_count: 1,
            sha256: ZERO_DIGEST.to_owned(),
            first_row_sha256: ZERO_DIGEST.to_owned(),
            last_row_sha256: ZERO_DIGEST.to_owned(),
            first_chain: None,
            last_chain: Some(crate::EventChainIdentity {
                phase: TimelinePhase::Operations,
                sequence: 0,
            }),
            continues_previous_chain: false,
            continues_next_chain: false,
        }],
    }
}

fn manifest() -> CorpusManifest {
    CorpusManifest {
        schema: 1,
        operations_sha256: ZERO_DIGEST.to_owned(),
        profiles: PROFILES
            .map(|(id, locale, width, height)| profile_manifest(id, locale, width, height))
            .to_vec(),
    }
}

fn profile_value(profile: &ProfileManifest) -> Value {
    let chunk = &profile.chunks[0];
    json!({
        "id": profile.id,
        "initial_locale": profile.initial_locale,
        "initial_viewport": {"x": 0, "y": 0, "width": profile.initial_viewport.width, "height": profile.initial_viewport.height},
        "operations_sha256": ZERO_DIGEST,
        "cast_sha256": ZERO_DIGEST,
        "cast_byte_size": 1,
        "row_count": 1,
        "maximum_chunk_rows": 24,
        "chunks": [{
            "id": chunk.id,
            "profile": profile.id,
            "start_sequence": 0,
            "end_sequence": 1,
            "row_count": 1,
            "sha256": ZERO_DIGEST,
            "first_row_sha256": ZERO_DIGEST,
            "last_row_sha256": ZERO_DIGEST,
            "first_chain": null,
            "last_chain": {"phase": "operations", "sequence": 0},
            "continues_previous_chain": false,
            "continues_next_chain": false,
        }],
    })
}

fn canonical(value: &Value) -> Vec<u8> {
    canonical_json_bytes(value).unwrap()
}

fn pending_progress(manifest: &CorpusManifest) -> ReviewProgress {
    ReviewProgress {
        manifest_sha256: manifest_digest(manifest).unwrap(),
        complete: false,
        review_sha256: None,
        reviewed_chunks: BTreeMap::new(),
        profile_verdicts: BTreeMap::new(),
    }
}

fn completed_progress(
    manifest: &CorpusManifest,
    report: &[u8],
    verdict: ReviewVerdict,
) -> ReviewProgress {
    ReviewProgress {
        manifest_sha256: manifest_digest(manifest).unwrap(),
        complete: true,
        review_sha256: Some(sha256_hex(report)),
        reviewed_chunks: manifest
            .profiles
            .iter()
            .flat_map(|profile| &profile.chunks)
            .map(|chunk| (chunk.id.clone(), chunk.sha256.clone()))
            .collect(),
        profile_verdicts: manifest
            .profiles
            .iter()
            .map(|profile| (profile.id.clone(), verdict))
            .collect(),
    }
}

fn reference(kind: ObjectKind, marker: char) -> ObjectRef {
    ObjectRef {
        kind,
        sha256: marker.to_string().repeat(64),
    }
}

fn row(profile: &SafeProfileId, marker: char) -> TimelineRow {
    TimelineRow {
        schema: 3,
        profile: profile.clone(),
        sequence: 0,
        phase: TimelinePhase::Operations,
        event_chain: None,
        operation_index: None,
        boundary: TimelineBoundary::Initial,
        cause: TransitionCause::Initial,
        presentation: Presentation::Presented,
        reducer: reference(ObjectKind::Reducer, marker),
        host: reference(ObjectKind::Host, marker),
        session: reference(ObjectKind::Session, marker),
        styled_frame: reference(ObjectKind::StyledFrame, marker),
        geometry: reference(ObjectKind::Geometry, marker),
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

fn rows_by_profile(manifest: &CorpusManifest) -> BTreeMap<SafeProfileId, Vec<TimelineRow>> {
    manifest
        .profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            let marker = char::from(b'a' + u8::try_from(index).unwrap());
            (profile.id.clone(), vec![row(&profile.id, marker)])
        })
        .collect()
}

#[test]
fn aggregate_contract_exposes_exact_root_names_and_legacy_review_bytes() {
    assert_eq!(MANIFEST_FILE, "manifest.json");
    assert_eq!(COVERAGE_FILE, "coverage.json");
    assert_eq!(REVIEW_PROGRESS_FILE, "review-progress.json");
    assert_eq!(README_FILE, "README.md");
    assert_eq!(REVIEW_FILE, "review.md");
    assert_eq!(
        REVIEW_GUIDE,
        b"# UI walker review corpus\n\nRead `manifest.json`, then review every chunk in profile order. Read each checkpoint row. Open every object when its digest first appears. Use `views/` for readable state and frame projections. Every checkpoint keeps its complete objects. A row with `presentation=not_presented` is causal evidence from an in-flight reducer or host state. Read its complete state, but do not treat its frame as visible terminal output. You must not report that frame as flicker, a blank UI, stale UI, or another visible defect unless later presented evidence shows the defect. Write findings and the review method in `review.md`. Record each reviewed chunk digest, each profile verdict, and the SHA-256 digest of the completed report in `review-progress.json`. A completed progress file is the local reviewer's attestation that it checked every declared chunk. The validator binds that attestation, the per-profile verdicts, and the report bytes to this corpus.\n"
    );
    assert_eq!(
        REVIEW_TEMPLATE,
        b"# UI review\n\n## Findings\n\n## Review method\n"
    );
}

#[test]
fn required_profile_accessor_and_manifest_order_are_exact() {
    let profiles = required_review_profiles();
    assert_eq!(profiles.len(), 4);
    for (actual, (id, locale, width, height)) in profiles.iter().zip(PROFILES) {
        assert_eq!(actual.id, id);
        assert_eq!(actual.locale, locale);
        assert_eq!(
            actual.viewport,
            RectSnapshot {
                x: 0,
                y: 0,
                width,
                height
            }
        );
    }

    let mut permuted = manifest();
    permuted.profiles.swap(0, 1);
    assert!(validate_manifest(&permuted).is_err());
    assert!(manifest_bytes(&permuted).is_err());
    assert!(decode_manifest(&canonical(&serde_json::to_value(permuted).unwrap())).is_err());
}

#[test]
fn manifest_codec_has_exact_canonical_bytes_and_round_trips_without_newline() {
    let manifest = manifest();
    let expected = json!({
        "schema": 1,
        "operations_sha256": ZERO_DIGEST,
        "profiles": manifest.profiles.iter().map(profile_value).collect::<Vec<_>>(),
    });
    let bytes = manifest_bytes(&manifest).unwrap();
    assert_eq!(bytes, canonical(&expected));
    assert!(!bytes.ends_with(b"\n"));
    assert_eq!(decode_manifest(&bytes).unwrap(), manifest);
}

#[test]
fn manifest_codec_rejects_each_missing_unknown_and_noncanonical_shape() {
    let valid = serde_json::to_value(manifest()).unwrap();
    for field in ["schema", "operations_sha256", "profiles"] {
        let mut value = valid.clone();
        value.as_object_mut().unwrap().remove(field);
        assert!(decode_manifest(&canonical(&value)).is_err(), "{field}");

        let mut value = valid.clone();
        value[format!("unknown_{field}")] = json!(true);
        assert!(decode_manifest(&canonical(&value)).is_err(), "{field}");
    }
    for field in [
        "id",
        "initial_locale",
        "initial_viewport",
        "operations_sha256",
        "cast_sha256",
        "cast_byte_size",
        "row_count",
        "maximum_chunk_rows",
        "chunks",
    ] {
        let mut value = valid.clone();
        value["profiles"][0].as_object_mut().unwrap().remove(field);
        assert!(decode_manifest(&canonical(&value)).is_err(), "{field}");

        let mut value = valid.clone();
        value["profiles"][0][format!("unknown_{field}")] = json!(true);
        assert!(decode_manifest(&canonical(&value)).is_err(), "{field}");
    }

    let bytes = canonical(&valid);
    let mut leading_space = vec![b' '];
    leading_space.extend(&bytes);
    assert!(decode_manifest(&leading_space).is_err());
    let mut trailing_newline = bytes;
    trailing_newline.push(b'\n');
    assert!(decode_manifest(&trailing_newline).is_err());
}

#[test]
fn manifest_codec_rejects_every_nested_missing_and_unknown_field() {
    let valid = serde_json::to_value(manifest()).unwrap();
    for field in ["x", "y", "width", "height"] {
        let mut value = valid.clone();
        value["profiles"][0]["initial_viewport"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            decode_manifest(&canonical(&value)).is_err(),
            "viewport {field}"
        );

        let mut value = valid.clone();
        value["profiles"][0]["initial_viewport"][format!("unknown_{field}")] = json!(true);
        assert!(
            decode_manifest(&canonical(&value)).is_err(),
            "viewport {field}"
        );
    }

    for field in [
        "id",
        "profile",
        "start_sequence",
        "end_sequence",
        "row_count",
        "sha256",
        "first_row_sha256",
        "last_row_sha256",
        "first_chain",
        "last_chain",
        "continues_previous_chain",
        "continues_next_chain",
    ] {
        let mut value = valid.clone();
        value["profiles"][0]["chunks"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            decode_manifest(&canonical(&value)).is_err(),
            "chunk {field}"
        );

        let mut value = valid.clone();
        value["profiles"][0]["chunks"][0][format!("unknown_{field}")] = json!(true);
        assert!(
            decode_manifest(&canonical(&value)).is_err(),
            "chunk {field}"
        );
    }

    for field in ["phase", "sequence"] {
        let mut value = valid.clone();
        value["profiles"][0]["chunks"][0]["last_chain"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            decode_manifest(&canonical(&value)).is_err(),
            "chain {field}"
        );

        let mut value = valid.clone();
        value["profiles"][0]["chunks"][0]["last_chain"][format!("unknown_{field}")] = json!(true);
        assert!(
            decode_manifest(&canonical(&value)).is_err(),
            "chain {field}"
        );
    }
}

#[test]
fn manifest_codec_rejects_nonobject_and_nonarray_nested_shapes() {
    let valid = serde_json::to_value(manifest()).unwrap();

    for replacement in [json!("manifest"), json!([]), Value::Null] {
        assert_eq!(
            decode_manifest(&canonical(&replacement))
                .unwrap_err()
                .to_string(),
            "manifest fields are not exact"
        );
    }

    for replacement in [json!("profiles"), json!({}), Value::Null] {
        let mut value = valid.clone();
        value["profiles"] = replacement;
        assert_eq!(
            decode_manifest(&canonical(&value)).unwrap_err().to_string(),
            "manifest profiles is not an array"
        );
    }

    for replacement in [json!("profile"), json!([]), Value::Null] {
        let mut value = valid.clone();
        value["profiles"][0] = replacement;
        assert_eq!(
            decode_manifest(&canonical(&value)).unwrap_err().to_string(),
            "manifest profile fields are not exact"
        );
    }

    for replacement in [json!("viewport"), json!([]), Value::Null] {
        let mut value = valid.clone();
        value["profiles"][0]["initial_viewport"] = replacement;
        assert_eq!(
            decode_manifest(&canonical(&value)).unwrap_err().to_string(),
            "manifest viewport fields are not exact"
        );
    }

    for replacement in [json!("chunks"), json!({}), Value::Null] {
        let mut value = valid.clone();
        value["profiles"][0]["chunks"] = replacement;
        assert_eq!(
            decode_manifest(&canonical(&value)).unwrap_err().to_string(),
            "manifest chunks is not an array"
        );
    }

    for replacement in [json!("chunk"), json!([]), Value::Null] {
        let mut value = valid.clone();
        value["profiles"][0]["chunks"][0] = replacement;
        assert_eq!(
            decode_manifest(&canonical(&value)).unwrap_err().to_string(),
            "manifest chunk fields are not exact"
        );
    }

    for chain in ["first_chain", "last_chain"] {
        for replacement in [json!("chain"), json!([])] {
            let mut value = valid.clone();
            value["profiles"][0]["chunks"][0][chain] = replacement;
            assert_eq!(
                decode_manifest(&canonical(&value)).unwrap_err().to_string(),
                "manifest event chain fields are not exact"
            );
        }
    }
    assert_eq!(
        valid["profiles"][0]["chunks"][0]["first_chain"],
        Value::Null
    );
    decode_manifest(&canonical(&valid)).unwrap();
}

#[test]
fn review_progress_codec_has_exact_line_bytes_and_rejects_wire_mutations() {
    let manifest = manifest();
    let progress = pending_progress(&manifest);
    let expected = json!({
        "manifest_sha256": progress.manifest_sha256,
        "complete": false,
        "review_sha256": null,
        "reviewed_chunks": {},
        "profile_verdicts": {},
    });
    let mut expected_bytes = canonical(&expected);
    expected_bytes.push(b'\n');
    let bytes = review_progress_bytes(&manifest, &progress).unwrap();
    assert_eq!(bytes, expected_bytes);
    assert_eq!(decode_review_progress(&manifest, &bytes).unwrap(), progress);

    let mut wrong_manifest = progress.clone();
    wrong_manifest.manifest_sha256 = ZERO_DIGEST.to_owned();
    assert!(review_progress_bytes(&manifest, &wrong_manifest).is_err());
    let mut wrong_manifest_bytes = canonical(&serde_json::to_value(wrong_manifest).unwrap());
    wrong_manifest_bytes.push(b'\n');
    assert!(decode_review_progress(&manifest, &wrong_manifest_bytes).is_err());

    let valid = serde_json::to_value(&progress).unwrap();
    for field in [
        "manifest_sha256",
        "complete",
        "review_sha256",
        "reviewed_chunks",
        "profile_verdicts",
    ] {
        let mut value = valid.clone();
        value.as_object_mut().unwrap().remove(field);
        let mut bytes = canonical(&value);
        bytes.push(b'\n');
        assert!(
            decode_review_progress(&manifest, &bytes).is_err(),
            "{field}"
        );

        let mut value = valid.clone();
        value[format!("unknown_{field}")] = json!(true);
        let mut bytes = canonical(&value);
        bytes.push(b'\n');
        assert!(
            decode_review_progress(&manifest, &bytes).is_err(),
            "{field}"
        );
    }

    let canonical = canonical(&valid);
    assert!(decode_review_progress(&manifest, &canonical).is_err());
    let mut double_newline = canonical.clone();
    double_newline.extend(b"\n\n");
    assert!(decode_review_progress(&manifest, &double_newline).is_err());
    let mut noncanonical = vec![b' '];
    noncanonical.extend(canonical);
    noncanonical.push(b'\n');
    assert!(decode_review_progress(&manifest, &noncanonical).is_err());
}

#[test]
fn aggregate_layout_unions_all_profiles_objects_and_root_files() {
    let manifest = manifest();
    let rows = rows_by_profile(&manifest);
    let mut layout = BundleLayout::default();
    layout.expect_review_corpus(&manifest, &rows).unwrap();

    for file in [
        RUN_FILE,
        OPERATIONS_FILE,
        FINAL_LIVENESS_FILE,
        MANIFEST_FILE,
        COVERAGE_FILE,
        REVIEW_PROGRESS_FILE,
        README_FILE,
        REVIEW_FILE,
    ] {
        assert!(layout.files.contains(&PathBuf::from(file)), "{file}");
    }
    for profile in &manifest.profiles {
        let root = profile_directory(&profile.id);
        assert!(layout.files.contains(&root.join(PROFILE_FILE)));
        assert!(layout.files.contains(&root.join(TIMELINE_FILE)));
        assert!(layout.files.contains(&root.join(CAST_FILE)));
        for reference in [
            &rows[&profile.id][0].reducer,
            &rows[&profile.id][0].host,
            &rows[&profile.id][0].session,
            &rows[&profile.id][0].styled_frame,
            &rows[&profile.id][0].geometry,
        ] {
            assert!(layout.files.contains(&object_path(reference)));
            assert!(layout.files.contains(&object_view_path(reference)));
        }
    }
}

#[test]
fn aggregate_layout_rejects_missing_extra_and_wrong_profile_rows() {
    let manifest = manifest();
    let rows = rows_by_profile(&manifest);

    let mut missing = rows.clone();
    missing.remove(&manifest.profiles[0].id);
    assert!(
        BundleLayout::default()
            .expect_review_corpus(&manifest, &missing)
            .is_err()
    );

    let mut extra = rows.clone();
    let extra_id = safe_profile("extra");
    extra.insert(extra_id.clone(), vec![row(&extra_id, 'f')]);
    assert!(
        BundleLayout::default()
            .expect_review_corpus(&manifest, &extra)
            .is_err()
    );

    let mut wrong = rows;
    wrong.get_mut(&manifest.profiles[0].id).unwrap()[0].profile = safe_profile("wrong");
    assert!(
        BundleLayout::default()
            .expect_review_corpus(&manifest, &wrong)
            .is_err()
    );

    let mut wrong_count = rows_by_profile(&manifest);
    wrong_count
        .get_mut(&manifest.profiles[0].id)
        .unwrap()
        .push(row(&manifest.profiles[0].id, 'f'));
    assert!(
        BundleLayout::default()
            .expect_review_corpus(&manifest, &wrong_count)
            .is_err()
    );
}

#[test]
fn aggregate_layout_rolls_back_seeded_receiver_after_every_late_error() {
    let manifest = manifest();
    let valid_rows = rows_by_profile(&manifest);
    let seeded = || {
        let mut layout = BundleLayout::default();
        layout.insert_file("seed.json");
        layout.insert_directory("seed-directory");
        layout
    };
    let assert_rollback = |rows: &BTreeMap<SafeProfileId, Vec<TimelineRow>>| {
        let mut layout = seeded();
        let before = layout.clone();
        assert!(layout.expect_review_corpus(&manifest, rows).is_err());
        assert_eq!(layout, before);
    };

    let mut missing = valid_rows.clone();
    missing.remove(&manifest.profiles[3].id);
    assert_rollback(&missing);

    let mut extra = valid_rows.clone();
    let extra_id = safe_profile("extra");
    extra.insert(extra_id.clone(), vec![row(&extra_id, 'f')]);
    assert_rollback(&extra);

    let mut wrong_count = valid_rows.clone();
    wrong_count
        .get_mut(&manifest.profiles[3].id)
        .unwrap()
        .push(row(&manifest.profiles[3].id, 'f'));
    assert_rollback(&wrong_count);

    let mut wrong_profile = valid_rows.clone();
    wrong_profile.get_mut(&manifest.profiles[3].id).unwrap()[0].profile = safe_profile("wrong");
    assert_rollback(&wrong_profile);

    let mut invalid_digest = valid_rows;
    invalid_digest.get_mut(&manifest.profiles[3].id).unwrap()[0]
        .geometry
        .sha256 = "INVALID".to_owned();
    assert_rollback(&invalid_digest);

    let mut successful = seeded();
    successful
        .expect_review_corpus(&manifest, &rows_by_profile(&manifest))
        .unwrap();
    assert!(successful.files.contains(&PathBuf::from("seed.json")));
    assert!(
        successful
            .directories
            .contains(&PathBuf::from("seed-directory"))
    );
}

#[test]
fn completed_review_claim_rejects_pending_template_digest_and_newline_errors() {
    let manifest = manifest();
    let report = b"# UI review\n\nNo findings.\n";
    assert!(
        validate_completed_review_claim(&manifest, &pending_progress(&manifest), report).is_err()
    );
    let mut invalid_progress = pending_progress(&manifest);
    invalid_progress.manifest_sha256 = ZERO_DIGEST.to_owned();
    assert_eq!(
        validate_completed_review_claim(&manifest, &invalid_progress, REVIEW_TEMPLATE)
            .unwrap_err()
            .to_string(),
        "progress refers to a different manifest"
    );
    let template = completed_progress(&manifest, REVIEW_TEMPLATE, ReviewVerdict::NoFindings);
    assert!(validate_completed_review_claim(&manifest, &template, REVIEW_TEMPLATE).is_err());

    let mut wrong_digest = completed_progress(&manifest, report, ReviewVerdict::NoFindings);
    wrong_digest.review_sha256 = Some(ZERO_DIGEST.to_owned());
    assert!(validate_completed_review_claim(&manifest, &wrong_digest, report).is_err());

    let no_newline = b"# UI review\n\nNo findings.";
    let progress = completed_progress(&manifest, no_newline, ReviewVerdict::NoFindings);
    assert!(validate_completed_review_claim(&manifest, &progress, no_newline).is_err());
    let double_newline = b"# UI review\n\nNo findings.\n\n";
    let progress = completed_progress(&manifest, double_newline, ReviewVerdict::NoFindings);
    assert!(validate_completed_review_claim(&manifest, &progress, double_newline).is_err());
}

#[test]
fn completed_review_claim_accepts_no_findings_and_findings_verdicts() {
    let manifest = manifest();
    for (report, verdict) in [
        (
            b"# UI review\n\nNo findings.\n".as_slice(),
            ReviewVerdict::NoFindings,
        ),
        (
            b"# UI review\n\nFinding: clipped footer.\n".as_slice(),
            ReviewVerdict::FindingsInReport,
        ),
    ] {
        let progress = completed_progress(&manifest, report, verdict);
        validate_completed_review_claim(&manifest, &progress, report).unwrap();
    }
}

#[test]
fn aggregate_layout_root_file_set_additions_are_exact() {
    let manifest = manifest();
    let rows = rows_by_profile(&manifest);
    let mut aggregate = BundleLayout::default();
    aggregate.expect_review_corpus(&manifest, &rows).unwrap();

    let mut singles = BundleLayout::default();
    for profile in &manifest.profiles {
        singles.expect_bundle(profile, &rows[&profile.id]).unwrap();
    }
    let additions = aggregate
        .files
        .difference(&singles.files)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        additions,
        [
            MANIFEST_FILE,
            COVERAGE_FILE,
            REVIEW_PROGRESS_FILE,
            README_FILE,
            REVIEW_FILE,
        ]
        .map(PathBuf::from)
        .into_iter()
        .collect()
    );
}

const SANDBOX_ORDER: [&str; 4] = ["en-80x24", "pseudo-120x12", "zh-cn-120x30", "zh-tw-40x40"];

fn review_run(profile_ids: &[&str]) -> RunMetadata {
    let sandbox = SandboxMetadata::stable(
        SandboxPlatform::Linux,
        "/tmp/skit-ui-walker-v1",
        profile_ids
            .iter()
            .map(|id| safe_profile(id))
            .collect::<BTreeSet<_>>(),
    )
    .unwrap();
    RunMetadata {
        schema: 2,
        source_revision: "revision".to_owned(),
        deterministic_result: "passed".to_owned(),
        operation_count: 1,
        operations_sha256: ZERO_DIGEST.to_owned(),
        final_liveness_sha256: ZERO_DIGEST.to_owned(),
        profiles: sandbox.profiles().keys().cloned().collect(),
        sandbox,
    }
}

fn profile_names(run: &RunMetadata) -> Vec<&str> {
    run.profiles.iter().map(SafeProfileId::as_str).collect()
}

#[test]
fn review_corpus_run_needs_a_stable_sandbox_and_the_required_profile_set() {
    let valid = review_run(&SANDBOX_ORDER);
    assert_eq!(profile_names(&valid), SANDBOX_ORDER);
    assert_ne!(
        profile_names(&valid),
        required_review_profiles()
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>()
    );
    validate_review_corpus_run(&valid).unwrap();

    let random_sandbox = SandboxMetadata::random(
        SandboxPlatform::Linux,
        "/tmp/skit-ui-walker-random",
        safe_profile("en-80x24"),
    )
    .unwrap();
    let mut random = valid.clone();
    random.profiles = random_sandbox.profiles().keys().cloned().collect();
    random.sandbox = random_sandbox;
    assert_eq!(
        validate_review_corpus_run(&random).unwrap_err().to_string(),
        "review corpus needs a stable sandbox"
    );

    for profile_ids in [
        ["en-80x24", "zh-cn-120x30", "zh-tw-40x40"].as_slice(),
        [
            "en-80x24",
            "extra",
            "pseudo-120x12",
            "zh-cn-120x30",
            "zh-tw-40x40",
        ]
        .as_slice(),
        ["en-80x24", "pseudo-120x13", "zh-cn-120x30", "zh-tw-40x40"].as_slice(),
    ] {
        assert_eq!(
            validate_review_corpus_run(&review_run(profile_ids))
                .unwrap_err()
                .to_string(),
            "review corpus run profiles are not the required profiles",
            "{profile_ids:?}"
        );
    }

    let mut failed = valid;
    failed.deterministic_result = "failed".to_owned();
    assert_eq!(
        validate_review_corpus_run(&failed).unwrap_err().to_string(),
        "run metadata result did not pass"
    );
}

#[test]
fn review_corpus_manifest_binds_the_run_digest_stored_profiles_and_profile_set() {
    let manifest = manifest();
    let run = review_run(&SANDBOX_ORDER);
    validate_review_corpus_manifest(&manifest, &run, &manifest.profiles).unwrap();

    let mut other_digest = run.clone();
    other_digest.operations_sha256 = "a".repeat(64);
    assert_eq!(
        validate_review_corpus_manifest(&manifest, &other_digest, &manifest.profiles)
            .unwrap_err()
            .to_string(),
        "manifest operation digest differs from the run"
    );

    let mut permuted = manifest.profiles.clone();
    permuted.swap(0, 1);
    assert_eq!(
        permuted
            .iter()
            .map(|profile| &profile.id)
            .collect::<BTreeSet<_>>(),
        manifest
            .profiles
            .iter()
            .map(|profile| &profile.id)
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        validate_review_corpus_manifest(&manifest, &run, &permuted)
            .unwrap_err()
            .to_string(),
        "stored profiles differ from the manifest"
    );

    let mut changed = manifest.profiles.clone();
    changed[0].cast_byte_size = 2;
    assert_eq!(
        validate_review_corpus_manifest(&manifest, &run, &changed)
            .unwrap_err()
            .to_string(),
        "stored profiles differ from the manifest"
    );
    assert_eq!(
        validate_review_corpus_manifest(&manifest, &run, &manifest.profiles[..3])
            .unwrap_err()
            .to_string(),
        "stored profiles differ from the manifest"
    );

    let mut foreign = run;
    foreign.profiles = ["en-80x24", "extra", "zh-cn-120x30", "zh-tw-40x40"]
        .map(safe_profile)
        .to_vec();
    assert!(validate_run_metadata(&foreign).is_err());
    assert_eq!(
        validate_review_corpus_manifest(&manifest, &foreign, &manifest.profiles)
            .unwrap_err()
            .to_string(),
        "run profiles differ from the manifest profiles"
    );

    let mut invalid = manifest;
    invalid.schema = 2;
    assert_eq!(
        validate_review_corpus_manifest(&invalid, &review_run(&SANDBOX_ORDER), &invalid.profiles)
            .unwrap_err()
            .to_string(),
        "manifest has an unsupported schema"
    );
}

fn partial_progress(manifest: &CorpusManifest, report: &[u8]) -> ReviewProgress {
    let chunk = &manifest.profiles[0].chunks[0];
    ReviewProgress {
        manifest_sha256: manifest_digest(manifest).unwrap(),
        complete: false,
        review_sha256: Some(sha256_hex(report)),
        reviewed_chunks: BTreeMap::from([(chunk.id.clone(), chunk.sha256.clone())]),
        profile_verdicts: BTreeMap::new(),
    }
}

#[test]
fn review_files_accept_a_partial_progress_and_a_completed_claim() {
    let manifest = manifest();
    let report = b"# UI review\n\nFinding: clipped footer.\n".as_slice();

    let pending = pending_progress(&manifest);
    let pending_bytes = review_progress_bytes(&manifest, &pending).unwrap();
    assert_eq!(
        validate_review_files(&manifest, REVIEW_GUIDE, &pending_bytes, REVIEW_TEMPLATE).unwrap(),
        pending
    );

    let partial = partial_progress(&manifest, report);
    let partial_bytes = review_progress_bytes(&manifest, &partial).unwrap();
    assert_eq!(
        validate_review_files(&manifest, REVIEW_GUIDE, &partial_bytes, report).unwrap(),
        partial
    );

    let complete = completed_progress(&manifest, report, ReviewVerdict::FindingsInReport);
    let complete_bytes = review_progress_bytes(&manifest, &complete).unwrap();
    assert_eq!(
        validate_review_files(&manifest, REVIEW_GUIDE, &complete_bytes, report).unwrap(),
        complete
    );
}

#[test]
fn review_files_refuse_an_edited_guide_and_every_invalid_report() {
    let manifest = manifest();
    let report = b"# UI review\n\nFinding: clipped footer.\n".as_slice();
    let pending = review_progress_bytes(&manifest, &pending_progress(&manifest)).unwrap();

    let mut edited_guide = REVIEW_GUIDE.to_vec();
    edited_guide.extend_from_slice(b"extra\n");
    assert_eq!(
        validate_review_files(&manifest, &edited_guide, &pending, report)
            .unwrap_err()
            .to_string(),
        "README.md is not the review guide"
    );
    assert_eq!(
        validate_review_files(
            &manifest,
            &REVIEW_GUIDE[..REVIEW_GUIDE.len() - 1],
            &pending,
            report
        )
        .unwrap_err()
        .to_string(),
        "README.md is not the review guide"
    );

    for (bytes, message) in [
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
            validate_review_files(&manifest, REVIEW_GUIDE, &pending, bytes)
                .unwrap_err()
                .to_string(),
            message
        );
    }
}

#[test]
fn review_files_refuse_an_invalid_progress_and_every_false_claim() {
    let manifest = manifest();
    let report = b"# UI review\n\nFinding: clipped footer.\n".as_slice();

    assert_eq!(
        validate_review_files(&manifest, REVIEW_GUIDE, b"{}\n", report)
            .unwrap_err()
            .to_string(),
        "review progress is missing review_sha256"
    );

    let mut stale = partial_progress(&manifest, report);
    let stale_bytes = {
        stale.reviewed_chunks =
            BTreeMap::from([(manifest.profiles[0].chunks[0].id.clone(), "a".repeat(64))]);
        let mut bytes = canonical(&serde_json::to_value(&stale).unwrap());
        bytes.push(b'\n');
        bytes
    };
    assert_eq!(
        validate_review_files(&manifest, REVIEW_GUIDE, &stale_bytes, report)
            .unwrap_err()
            .to_string(),
        "progress has an unknown or stale chunk digest"
    );

    let mut false_claim = pending_progress(&manifest);
    false_claim.review_sha256 = Some(ZERO_DIGEST.to_owned());
    let false_claim_bytes = review_progress_bytes(&manifest, &false_claim).unwrap();
    assert!(!false_claim.complete);
    assert_eq!(
        validate_review_files(&manifest, REVIEW_GUIDE, &false_claim_bytes, report)
            .unwrap_err()
            .to_string(),
        "review progress claims a different report digest"
    );

    let template_claim = completed_progress(&manifest, REVIEW_TEMPLATE, ReviewVerdict::NoFindings);
    let template_bytes = review_progress_bytes(&manifest, &template_claim).unwrap();
    assert_eq!(
        template_claim.review_sha256,
        Some(sha256_hex(REVIEW_TEMPLATE))
    );
    assert_eq!(
        validate_review_files(&manifest, REVIEW_GUIDE, &template_bytes, REVIEW_TEMPLATE)
            .unwrap_err()
            .to_string(),
        "the UI review report is still the unedited template"
    );
}

#[test]
fn completed_review_claim_refuses_a_report_with_an_invalid_byte_format() {
    let manifest = manifest();
    for (report, message) in [
        (
            b"# UI review\n\n\xff\n".as_slice(),
            "the UI review report is not UTF-8",
        ),
        (b"".as_slice(), "the UI review report is empty"),
    ] {
        let progress = completed_progress(&manifest, report, ReviewVerdict::NoFindings);
        assert_eq!(
            validate_completed_review_claim(&manifest, &progress, report)
                .unwrap_err()
                .to_string(),
            message
        );
    }
}
