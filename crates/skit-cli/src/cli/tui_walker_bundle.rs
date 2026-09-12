//! Immutable bundle storage for real-host walker traces.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use ratatui_core::buffer::CellWidth as _;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use skit_tui_walker_support::{
    CellDiffSnapshot, CorpusManifest, EventChainIdentity, ObjectKind, ObjectRef, ProfileManifest,
    ReviewProgress, StyledFrameSnapshot, TimelineBoundary, TimelinePhase, TimelineRow,
    TransitionCause, build_chunks, build_object,
    bundle::{
        self, BundleLayout, CAST_FILE, CHUNK_DIRECTORY, COVERAGE_FILE, ChunkViewCursor,
        CoverageSummary, EffectCoverage, FINAL_LIVENESS_FILE, FRAME_VIEW_DIRECTORY, MANIFEST_FILE,
        OBJECT_DIRECTORY, OPERATIONS_FILE, PROFILE_DIRECTORY, PROFILE_FILE, ProfileExpectation,
        README_FILE, REVIEW_FILE, REVIEW_GUIDE, REVIEW_PROGRESS_FILE, REVIEW_TEMPLATE, RUN_FILE,
        RunMetadata, TIMELINE_FILE, VIEW_DIRECTORY, validate_coverage_summary,
    },
    canonical_json_bytes, decode_manifest,
    leak_oracle::{
        TextSpan, exact_fact_spans, json_string_occurrences, parse_draft_allocator_token,
    },
    manifest_bytes, manifest_digest, review_progress_bytes,
    sandbox::{
        STABLE_SANDBOX_NAMESPACE, SafeProfileId, SandboxMetadata, SandboxMode, SandboxPlatform,
        SandboxRoots,
    },
    sha256_hex, timeline_row_digest, validate_asciicast_v3, validate_chunks, validate_object,
    validate_timeline_semantics,
};
use tempfile::Builder;

use skit_ui::{AddEffect, Effect, FormPurpose, HostRequest, PreferencesEffect};

use super::tui_real_host::{
    ArtifactLeakOracleFact, LeakOracleFacts, ModifiedLeakOraclePair, RendererDraftFact,
    RendererReviewNameFact, SourceIdentityLeakOraclePair, StableSandboxNamespace,
};
use super::tui_real_sandbox_fs::{ChildName, PinnedDirectory, SandboxFsError};
use super::tui_real_walker::{
    CorpusOperation, LocaleSmokeTarget, RealTrace, RecordedRealCorpus, RecordedRealTrace,
    StableCorpusTracePair, StablePairPhase, canonical_corpus_artifact_values, parse_library_state,
    record_real_locale_smoke, record_real_review_corpus_in, record_real_smoke_main,
    record_real_smoke_replay, record_real_smoke_stable_pair_in, validate_effect_chain_termination,
    validate_reducer_action,
};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use super::tui_real_walker::{
    draft_quarantine_corpus_operations, invalid_corpus_operation,
    rendered_sandbox_root_corpus_operations, review_corpus_contract_operations,
    review_name_cut_corpus_operations,
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use skit_tui_walker_support::{ChunkDescriptor, ReviewVerdict};

const MAXIMUM_CHUNK_ROWS: usize = 24;
const STORE_FIELDS: [&str; 6] = [
    "surface",
    "config",
    "form_state",
    "prompt_runner",
    "drafts",
    "tree",
];

const REQUIRED_SERVED_REQUESTS: [&str; 23] = [
    "add",
    "count_run_glob",
    "edit",
    "health_rebuild",
    "open.add",
    "open.health",
    "open.preferences",
    "open.presets",
    "open.rename",
    "open.run",
    "open.runners",
    "open.settings",
    "preferences",
    "refresh_preferences_after_runners",
    "reload",
    "remove",
    "remove_runner",
    "rerun",
    "save_run_preset",
    "save_runner",
    "submit.rename",
    "submit.run",
    "submit.settings",
];
const REQUIRED_NESTED_OCCURRENCES: [&str; 14] = [
    "add.author_draft",
    "add.cancel",
    "add.commit",
    "add.complete",
    "add.consume_draft",
    "add.delete_draft",
    "add.draft_kept",
    "add.edit_source",
    "add.inspect_source",
    "add.remember_runner",
    "preferences.discover_agent_skill_targets",
    "preferences.install_agent_skill",
    "preferences.manage_agents",
    "preferences.save",
];
const STATICALLY_UNSERVED_EFFECTS: [&str; 8] = [
    "none",
    "preferences.close",
    "preferences.confirm_discard",
    "preferences.none",
    "quit",
    "submit.add",
    "submit.preferences",
    "submit.runners",
];

#[derive(Clone, Copy, Debug)]
struct RequiredEffectCoverageVocabulary {
    served_requests: &'static [&'static str],
    nested_occurrences: &'static [&'static str],
    statically_unserved: &'static [&'static str],
}

const fn required_effect_coverage_vocabulary() -> RequiredEffectCoverageVocabulary {
    RequiredEffectCoverageVocabulary {
        served_requests: &REQUIRED_SERVED_REQUESTS,
        nested_occurrences: &REQUIRED_NESTED_OCCURRENCES,
        statically_unserved: &STATICALLY_UNSERVED_EFFECTS,
    }
}

fn validate_effect_coverage_vocabulary(
    vocabulary: RequiredEffectCoverageVocabulary,
) -> Result<(), String> {
    for (label, names) in [
        ("served request", vocabulary.served_requests),
        ("nested occurrence", vocabulary.nested_occurrences),
        ("statically unserved", vocabulary.statically_unserved),
    ] {
        require(
            names.windows(2).all(|pair| pair[0] < pair[1]),
            &format!("required {label} names are not sorted and unique"),
        )?;
    }
    let mut seen = BTreeSet::new();
    for name in vocabulary
        .served_requests
        .iter()
        .chain(vocabulary.nested_occurrences)
        .chain(vocabulary.statically_unserved)
    {
        require(
            seen.insert(*name),
            "required effect coverage names are not disjoint",
        )?;
    }
    Ok(())
}

fn validate_required_effect_coverage_vocabulary() -> Result<(), String> {
    validate_effect_coverage_vocabulary(required_effect_coverage_vocabulary())
}

#[derive(Debug, Default)]
struct EffectCoverageTally {
    served_requests: BTreeMap<String, u64>,
    nested_occurrences: BTreeMap<String, u64>,
}

impl EffectCoverageTally {
    fn record_served_request(&mut self, effect: &Effect) {
        increment_count(&mut self.served_requests, served_effect_name(effect));
        match effect {
            Effect::Add(effects) => {
                for effect in effects {
                    increment_count(&mut self.nested_occurrences, add_effect_name(effect));
                }
            }
            Effect::Preferences(effect) => increment_count(
                &mut self.nested_occurrences,
                preferences_effect_name(effect),
            ),
            Effect::None
            | Effect::Reload
            | Effect::Quit
            | Effect::Rerun { .. }
            | Effect::Open { .. }
            | Effect::Submit { .. }
            | Effect::CountRunGlob { .. }
            | Effect::SaveRunPreset { .. }
            | Effect::HealthRebuild
            | Effect::SaveRunner { .. }
            | Effect::RemoveRunner(_)
            | Effect::RefreshPreferencesAfterRunners
            | Effect::Edit { .. }
            | Effect::Remove { .. } => {}
        }
    }

    fn measured_summary(self) -> Result<CoverageSummary, String> {
        let summary = CoverageSummary {
            schema: 1,
            effects: EffectCoverage {
                measured: true,
                served_requests: Some(self.served_requests),
                nested_occurrences: Some(self.nested_occurrences),
                statically_unserved: statically_unserved_effects(),
            },
        };
        validate_coverage_summary(&summary).map_err(error_message)?;
        Ok(summary)
    }
}

fn effect_coverage_tally_from_timeline(
    rows: &[TimelineRow],
) -> Result<EffectCoverageTally, String> {
    let mut tally = EffectCoverageTally::default();
    for row in rows {
        let TransitionCause::Host { request, .. } = &row.cause else {
            continue;
        };
        let effect: Effect = serde_json::from_value(request.clone())
            .map_err(|error| format!("timeline host request is not an Effect: {error}"))?;
        let typed = serde_json::to_value(&effect).map_err(error_message)?;
        require(
            typed == *request,
            "timeline host request does not match its exact typed Effect",
        )?;
        tally.record_served_request(&effect);
    }
    Ok(tally)
}

fn validate_required_count_names(
    counts: &BTreeMap<String, u64>,
    required: &[&str],
    label: &str,
) -> Result<(), String> {
    let actual = counts.keys().map(String::as_str).collect::<Vec<_>>();
    require(
        actual == required,
        &format!("measured coverage {label} names do not match the required vocabulary"),
    )
}

pub(super) fn validate_required_effect_coverage_summary(
    summary: &CoverageSummary,
) -> Result<(), String> {
    validate_required_effect_coverage_vocabulary()?;
    validate_coverage_summary(summary).map_err(error_message)?;
    require(
        summary.effects.measured,
        "required effect coverage is not measured",
    )?;
    let vocabulary = required_effect_coverage_vocabulary();
    require(
        summary.effects.statically_unserved == statically_unserved_effects(),
        "measured coverage statically unserved names do not match the required vocabulary",
    )?;
    let served = summary
        .effects
        .served_requests
        .as_ref()
        .expect("validated measured coverage has served request counts");
    let nested = summary
        .effects
        .nested_occurrences
        .as_ref()
        .expect("validated measured coverage has nested occurrence counts");
    validate_required_count_names(served, vocabulary.served_requests, "served request")?;
    validate_required_count_names(nested, vocabulary.nested_occurrences, "nested occurrence")
}

/// Tally one profile timeline. Measurement carries no required-vocabulary policy.
pub(super) fn measured_effect_coverage_from_timeline(
    rows: &[TimelineRow],
) -> Result<CoverageSummary, String> {
    effect_coverage_tally_from_timeline(rows)?.measured_summary()
}

fn merge_effect_coverage_counts(
    aggregate: &mut BTreeMap<String, u64>,
    profile: &BTreeMap<String, u64>,
) -> Result<(), String> {
    for (name, count) in profile {
        let total = aggregate.entry(name.clone()).or_default();
        *total = total
            .checked_add(*count)
            .ok_or_else(|| format!("effect coverage count overflows for {name}"))?;
    }
    Ok(())
}

/// Sum four measured profile summaries in required order with checked addition.
pub(super) fn merge_profile_effect_coverage(
    profiles: &[(SafeProfileId, CoverageSummary)],
) -> Result<CoverageSummary, String> {
    let required_profiles = skit_tui_walker_support::required_review_profiles();
    require(
        profiles.len() == required_profiles.len(),
        "effect coverage does not contain every required review profile",
    )?;
    let mut aggregate = EffectCoverageTally::default();
    for ((profile, summary), required) in profiles.iter().zip(required_profiles) {
        require(
            profile.as_str() == required.id,
            "effect coverage review profiles are not in the required order",
        )?;
        validate_coverage_summary(summary).map_err(error_message)?;
        require(
            summary.effects.measured,
            "a profile effect coverage summary is not measured",
        )?;
        let served = summary
            .effects
            .served_requests
            .as_ref()
            .expect("validated measured coverage has served request counts");
        let nested = summary
            .effects
            .nested_occurrences
            .as_ref()
            .expect("validated measured coverage has nested occurrence counts");
        merge_effect_coverage_counts(&mut aggregate.served_requests, served)?;
        merge_effect_coverage_counts(&mut aggregate.nested_occurrences, nested)?;
    }
    aggregate.measured_summary()
}

/// The final-corpus policy wrapper over the plain merge.
pub(super) fn merge_required_profile_effect_coverage(
    profiles: &[(SafeProfileId, CoverageSummary)],
) -> Result<CoverageSummary, String> {
    for (_, summary) in profiles {
        validate_required_effect_coverage_summary(summary)?;
    }
    let summary = merge_profile_effect_coverage(profiles)?;
    validate_required_effect_coverage_summary(&summary)?;
    Ok(summary)
}

pub(super) fn validate_stored_effect_coverage_summary(
    stored: &CoverageSummary,
    profiles: &[(SafeProfileId, CoverageSummary)],
) -> Result<(), String> {
    let reconstructed = merge_profile_effect_coverage(profiles)?;
    require(
        stored == &reconstructed,
        "stored effect coverage does not match the profile timelines",
    )
}

/// Apply the final review corpus policy to already decoded corpus data.
pub(super) fn validate_final_review_corpus(
    operations_bytes: &[u8],
    operation_count: usize,
    profile_coverage: &[(SafeProfileId, CoverageSummary)],
    stored: &CoverageSummary,
) -> Result<(), String> {
    let (operations, _) = canonical_corpus_artifact_values()?;
    let canonical =
        canonical_json_bytes(&Value::Array(operations.clone())).map_err(error_message)?;
    require(
        operation_count == operations.len(),
        "the review corpus does not have the canonical operation count",
    )?;
    require(
        operations_bytes == canonical,
        "the review corpus operations are not the canonical vector",
    )?;
    let required = merge_required_profile_effect_coverage(profile_coverage)?;
    require(
        stored == &required,
        "the review corpus coverage is not the required aggregate",
    )
}

fn increment_count(counts: &mut BTreeMap<String, u64>, name: &'static str) {
    *counts.entry(name.to_owned()).or_default() += 1;
}

fn served_effect_name(effect: &Effect) -> &'static str {
    match effect {
        Effect::None => "none",
        Effect::Reload => "reload",
        Effect::Quit => "quit",
        Effect::Rerun { .. } => "rerun",
        Effect::Open { request, .. } => open_effect_name(*request),
        Effect::Submit { purpose, .. } => submit_effect_name(*purpose),
        Effect::CountRunGlob { .. } => "count_run_glob",
        Effect::SaveRunPreset { .. } => "save_run_preset",
        Effect::Add(_) => "add",
        Effect::HealthRebuild => "health_rebuild",
        Effect::SaveRunner { .. } => "save_runner",
        Effect::RemoveRunner(_) => "remove_runner",
        Effect::RefreshPreferencesAfterRunners => "refresh_preferences_after_runners",
        Effect::Preferences(_) => "preferences",
        Effect::Edit { .. } => "edit",
        Effect::Remove { .. } => "remove",
    }
}

const fn open_effect_name(request: HostRequest) -> &'static str {
    match request {
        HostRequest::Run => "open.run",
        HostRequest::Add => "open.add",
        HostRequest::Settings => "open.settings",
        HostRequest::Preferences => "open.preferences",
        HostRequest::Health => "open.health",
        HostRequest::Runners => "open.runners",
        HostRequest::Presets => "open.presets",
        HostRequest::Rename => "open.rename",
    }
}

const fn submit_effect_name(purpose: FormPurpose) -> &'static str {
    match purpose {
        FormPurpose::Run => "submit.run",
        FormPurpose::Add => "submit.add",
        FormPurpose::Settings => "submit.settings",
        FormPurpose::Preferences => "submit.preferences",
        FormPurpose::Runners => "submit.runners",
        FormPurpose::Rename => "submit.rename",
    }
}

fn add_effect_name(effect: &AddEffect) -> &'static str {
    match effect {
        AddEffect::InspectSource { .. } => "add.inspect_source",
        AddEffect::AuthorDraft { .. } => "add.author_draft",
        AddEffect::DeleteDraft { .. } => "add.delete_draft",
        AddEffect::EditSource { .. } => "add.edit_source",
        AddEffect::Commit { .. } => "add.commit",
        AddEffect::ConsumeDraft(_) => "add.consume_draft",
        AddEffect::DraftKept(_) => "add.draft_kept",
        AddEffect::RememberRunner(_) => "add.remember_runner",
        AddEffect::Complete(_) => "add.complete",
        AddEffect::Cancel => "add.cancel",
    }
}

fn preferences_effect_name(effect: &PreferencesEffect) -> &'static str {
    match effect {
        PreferencesEffect::None => "preferences.none",
        PreferencesEffect::Save(_) => "preferences.save",
        PreferencesEffect::Close => "preferences.close",
        PreferencesEffect::ConfirmDiscard => "preferences.confirm_discard",
        PreferencesEffect::ManageAgents => "preferences.manage_agents",
        PreferencesEffect::DiscoverAgentSkillTargets => "preferences.discover_agent_skill_targets",
        PreferencesEffect::InstallAgentSkill { .. } => "preferences.install_agent_skill",
    }
}

fn statically_unserved_effects() -> Vec<String> {
    required_effect_coverage_vocabulary()
        .statically_unserved
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

fn effect_fixture(value: Value) -> Effect {
    serde_json::from_value(value).unwrap()
}

fn source_effect_fixture() -> Value {
    json!({
        "path": "source.py",
        "source_record": "source.py",
        "bytes": [],
        "permissions": {"readonly": false, "unix_mode": null},
        "executable": false,
        "is_regular": true,
        "is_directory": false,
        "is_draft": false,
        "identity": null,
    })
}

#[test]
fn effect_coverage_classifier_covers_all_outer_request_and_form_variants() {
    let outer = [
        (json!("none"), "none"),
        (json!("reload"), "reload"),
        (json!("quit"), "quit"),
        (json!({"rerun": {"selector": "entry"}}), "rerun"),
        (
            json!({"open": {"request": "run", "selector": "entry"}}),
            "open.run",
        ),
        (
            json!({"submit": {"purpose": "run", "selector": "entry", "values": {}}}),
            "submit.run",
        ),
        (
            json!({"count_run_glob": {"selector": "entry", "field": 0, "value": "*", "request": {"cwd": "/cwd", "pieces": ["*"]}}}),
            "count_run_glob",
        ),
        (
            json!({"save_run_preset": {"selector": "entry", "name": "preset", "values": {}, "secret_names": []}}),
            "save_run_preset",
        ),
        (json!({"add": []}), "add"),
        (json!("health_rebuild"), "health_rebuild"),
        (
            json!({"save_runner": {"request": {"name": "runner", "argv": ["runner"], "target": "new"}, "owner": "manager"}}),
            "save_runner",
        ),
        (
            json!({"remove_runner": {"named": {"name": "runner", "expected": [], "expected_pinned_count": 0}}}),
            "remove_runner",
        ),
        (
            json!("refresh_preferences_after_runners"),
            "refresh_preferences_after_runners",
        ),
        (json!({"preferences": "manage_agents"}), "preferences"),
        (json!({"edit": {"selector": "entry"}}), "edit"),
        (json!({"remove": {"selector": "entry"}}), "remove"),
    ];
    assert_eq!(outer.len(), 16);
    for (value, expected) in outer {
        assert_eq!(served_effect_name(&effect_fixture(value)), expected);
    }

    let requests = [
        (HostRequest::Run, "open.run"),
        (HostRequest::Add, "open.add"),
        (HostRequest::Settings, "open.settings"),
        (HostRequest::Preferences, "open.preferences"),
        (HostRequest::Health, "open.health"),
        (HostRequest::Runners, "open.runners"),
        (HostRequest::Presets, "open.presets"),
        (HostRequest::Rename, "open.rename"),
    ];
    assert_eq!(requests.len(), 8);
    for (request, expected) in requests {
        assert_eq!(open_effect_name(request), expected);
    }

    let purposes = [
        (FormPurpose::Run, "submit.run"),
        (FormPurpose::Add, "submit.add"),
        (FormPurpose::Settings, "submit.settings"),
        (FormPurpose::Preferences, "submit.preferences"),
        (FormPurpose::Runners, "submit.runners"),
        (FormPurpose::Rename, "submit.rename"),
    ];
    assert_eq!(purposes.len(), 6);
    for (purpose, expected) in purposes {
        assert_eq!(submit_effect_name(purpose), expected);
    }
}

#[test]
fn effect_coverage_classifier_covers_every_nested_variant() {
    let source = source_effect_fixture();
    let add = [
        (
            json!({"inspect_source": {"request": 0, "path": "source.py"}}),
            "add.inspect_source",
        ),
        (
            json!({"author_draft": {"request": 0, "kind": "script"}}),
            "add.author_draft",
        ),
        (
            json!({"delete_draft": {
                "request": 0,
                "draft": {"path": "draft.py", "modified": 1},
            }}),
            "add.delete_draft",
        ),
        (
            json!({"edit_source": {"request": 0, "path": "source.py"}}),
            "add.edit_source",
        ),
        (
            json!({"commit": {
                "request": 0,
                "entry": {
                    "name": "entry",
                    "kind": "command",
                    "mode": "copy",
                    "source": "",
                    "workdir": "invoke",
                    "description": "",
                    "payload": null,
                },
                "source": null,
            }}),
            "add.commit",
        ),
        (json!({"consume_draft": source}), "add.consume_draft"),
        (json!({"draft_kept": "draft.py"}), "add.draft_kept"),
        (json!({"remember_runner": "runner"}), "add.remember_runner"),
        (json!({"complete": "entry"}), "add.complete"),
        (json!("cancel"), "add.cancel"),
    ];
    assert_eq!(add.len(), 10);
    for (value, expected) in add {
        let effect: AddEffect = serde_json::from_value(value).unwrap();
        assert_eq!(add_effect_name(&effect), expected);
    }

    let preferences = [
        (json!("none"), "preferences.none"),
        (json!({"save": {"settings": {}}}), "preferences.save"),
        (json!("close"), "preferences.close"),
        (json!("confirm_discard"), "preferences.confirm_discard"),
        (json!("manage_agents"), "preferences.manage_agents"),
        (
            json!("discover_agent_skill_targets"),
            "preferences.discover_agent_skill_targets",
        ),
        (
            json!({"install_agent_skill": {"skills_dir": "/skills"}}),
            "preferences.install_agent_skill",
        ),
    ];
    assert_eq!(preferences.len(), 7);
    for (value, expected) in preferences {
        let effect: PreferencesEffect = serde_json::from_value(value).unwrap();
        assert_eq!(preferences_effect_name(&effect), expected);
    }
}

#[test]
fn effect_coverage_tally_counts_one_outer_request_and_every_nested_occurrence() {
    let mut tally = EffectCoverageTally::default();
    tally.record_served_request(&effect_fixture(json!({"add": [
        {"inspect_source": {"request": 0, "path": "source.py"}},
        "cancel",
        {"inspect_source": {"request": 1, "path": "source.py"}},
    ]})));
    tally.record_served_request(&Effect::Preferences(PreferencesEffect::ManageAgents));
    tally.record_served_request(&Effect::Reload);

    assert_eq!(
        tally.served_requests,
        BTreeMap::from([
            ("add".to_owned(), 1),
            ("preferences".to_owned(), 1),
            ("reload".to_owned(), 1),
        ])
    );
    assert_eq!(
        tally.nested_occurrences,
        BTreeMap::from([
            ("add.cancel".to_owned(), 1),
            ("add.inspect_source".to_owned(), 2),
            ("preferences.manage_agents".to_owned(), 1),
        ])
    );

    let summary = tally.measured_summary().unwrap();
    assert!(summary.effects.measured);
    assert_eq!(
        summary.effects.statically_unserved,
        statically_unserved_effects()
    );

    let mut invalid = EffectCoverageTally::default();
    invalid.record_served_request(&Effect::Quit);
    assert!(invalid.measured_summary().is_err());
}

#[test]
fn effect_coverage_static_exclusions_are_exact_and_lexical() {
    let names = statically_unserved_effects();
    assert_eq!(
        names,
        [
            "none",
            "preferences.close",
            "preferences.confirm_discard",
            "preferences.none",
            "quit",
            "submit.add",
            "submit.preferences",
            "submit.runners",
        ]
    );
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(!names.iter().any(|name| name.starts_with("runners.")));
    assert!(!names.iter().any(|name| name.contains("edit_draft")));
}

fn coverage_test_row(cause: TransitionCause) -> TimelineRow {
    let object = |kind| ObjectRef {
        kind,
        sha256: "0".repeat(64),
    };
    TimelineRow {
        schema: 3,
        profile: SafeProfileId::try_from("coverage-test").unwrap(),
        sequence: 0,
        phase: TimelinePhase::Operations,
        event_chain: None,
        operation_index: Some(0),
        boundary: TimelineBoundary::HostAction,
        cause,
        presentation: skit_tui_walker_support::Presentation::NotPresented,
        reducer: object(ObjectKind::Reducer),
        host: object(ObjectKind::Host),
        session: object(ObjectKind::Session),
        styled_frame: object(ObjectKind::StyledFrame),
        geometry: object(ObjectKind::Geometry),
        locale: "en".to_owned(),
        viewport: skit_tui_walker_support::RectSnapshot {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
        liveness: None,
        previous_row_sha256: None,
    }
}

fn coverage_host_row(request: Value, emitted: Value) -> TimelineRow {
    coverage_test_row(TransitionCause::Host {
        operation_index: Some(0),
        round: 0,
        request,
        response: json!({"not_an_action": true}),
        emitted,
    })
}

fn coverage_add_effect_values() -> Vec<Value> {
    let source = source_effect_fixture();
    vec![
        json!({"inspect_source": {"request": 0, "path": "source.py"}}),
        json!({"author_draft": {"request": 1, "kind": "script"}}),
        json!({"delete_draft": {
            "request": 2,
            "draft": {"path": "draft.py", "modified": 1},
        }}),
        json!({"edit_source": {"request": 3, "path": "source.py"}}),
        json!({"commit": {
            "request": 4,
            "entry": {
                "name": "entry",
                "kind": "command",
                "mode": "copy",
                "source": "",
                "workdir": "invoke",
                "description": "",
                "payload": null,
            },
            "source": null,
        }}),
        json!({"consume_draft": source}),
        json!({"draft_kept": "draft.py"}),
        json!({"remember_runner": "runner"}),
        json!({"complete": "entry"}),
        json!("cancel"),
    ]
}

#[test]
fn timeline_coverage_counts_only_typed_host_requests_and_all_nested_occurrences() {
    let add = serde_json::to_value(effect_fixture(json!({
        "add": coverage_add_effect_values(),
    })))
    .unwrap();
    let preferences =
        serde_json::to_value(Effect::Preferences(PreferencesEffect::ManageAgents)).unwrap();
    let rows = vec![
        coverage_test_row(TransitionCause::Initial),
        coverage_test_row(TransitionCause::Session {
            requested: json!("reload"),
            resolved: Value::Null,
            event: Value::Null,
            handling: json!("ignored"),
        }),
        coverage_test_row(TransitionCause::Reducer {
            requested: Value::Null,
            resolved: Value::Null,
            event: Value::Null,
            action: Value::Null,
            emitted: json!("reload"),
        }),
        coverage_host_row(add, json!({"not_an_effect": true})),
        coverage_host_row(preferences, json!("reload")),
    ];

    let tally = effect_coverage_tally_from_timeline(&rows).unwrap();
    assert_eq!(
        tally.served_requests,
        BTreeMap::from([("add".to_owned(), 1), ("preferences".to_owned(), 1)])
    );
    assert_eq!(
        tally.nested_occurrences,
        BTreeMap::from([
            ("add.author_draft".to_owned(), 1),
            ("add.cancel".to_owned(), 1),
            ("add.commit".to_owned(), 1),
            ("add.complete".to_owned(), 1),
            ("add.consume_draft".to_owned(), 1),
            ("add.delete_draft".to_owned(), 1),
            ("add.draft_kept".to_owned(), 1),
            ("add.edit_source".to_owned(), 1),
            ("add.inspect_source".to_owned(), 1),
            ("add.remember_runner".to_owned(), 1),
            ("preferences.manage_agents".to_owned(), 1),
        ])
    );
}

#[test]
fn timeline_coverage_refuses_non_effect_and_noncanonical_typed_host_requests() {
    let unknown = json!({
        "open": {"request": "run", "selector": "entry", "unknown": true},
    });
    assert!(serde_json::from_value::<Effect>(unknown.clone()).is_ok());
    let error = effect_coverage_tally_from_timeline(&[coverage_host_row(unknown, Value::Null)])
        .unwrap_err();
    assert!(error.contains("exact typed Effect"), "{error}");

    let error = effect_coverage_tally_from_timeline(&[coverage_host_row(
        json!({"not_an_effect": true}),
        Value::Null,
    )])
    .unwrap_err();
    assert!(error.contains("not an Effect"), "{error}");
}

#[test]
fn required_effect_coverage_vocabulary_is_exact_sorted_unique_and_disjoint() {
    let vocabulary = required_effect_coverage_vocabulary();
    assert_eq!(
        vocabulary.served_requests,
        &[
            "add",
            "count_run_glob",
            "edit",
            "health_rebuild",
            "open.add",
            "open.health",
            "open.preferences",
            "open.presets",
            "open.rename",
            "open.run",
            "open.runners",
            "open.settings",
            "preferences",
            "refresh_preferences_after_runners",
            "reload",
            "remove",
            "remove_runner",
            "rerun",
            "save_run_preset",
            "save_runner",
            "submit.rename",
            "submit.run",
            "submit.settings",
        ]
    );
    assert_eq!(
        vocabulary.nested_occurrences,
        &[
            "add.author_draft",
            "add.cancel",
            "add.commit",
            "add.complete",
            "add.consume_draft",
            "add.delete_draft",
            "add.draft_kept",
            "add.edit_source",
            "add.inspect_source",
            "add.remember_runner",
            "preferences.discover_agent_skill_targets",
            "preferences.install_agent_skill",
            "preferences.manage_agents",
            "preferences.save",
        ]
    );
    assert_eq!(
        vocabulary.statically_unserved,
        &[
            "none",
            "preferences.close",
            "preferences.confirm_discard",
            "preferences.none",
            "quit",
            "submit.add",
            "submit.preferences",
            "submit.runners",
        ]
    );
    validate_required_effect_coverage_vocabulary().unwrap();
}

#[test]
fn required_effect_coverage_vocabulary_refuses_bad_order_and_cross_set_overlap() {
    assert!(
        validate_effect_coverage_vocabulary(RequiredEffectCoverageVocabulary {
            served_requests: &["reload", "add"],
            nested_occurrences: &[],
            statically_unserved: &[],
        })
        .unwrap_err()
        .contains("sorted and unique")
    );
    assert!(
        validate_effect_coverage_vocabulary(RequiredEffectCoverageVocabulary {
            served_requests: &["reload"],
            nested_occurrences: &["reload"],
            statically_unserved: &[],
        })
        .unwrap_err()
        .contains("not disjoint")
    );
}

fn complete_required_coverage(count: u64) -> CoverageSummary {
    let vocabulary = required_effect_coverage_vocabulary();
    CoverageSummary {
        schema: 1,
        effects: EffectCoverage {
            measured: true,
            served_requests: Some(
                vocabulary
                    .served_requests
                    .iter()
                    .map(|name| ((*name).to_owned(), count))
                    .collect(),
            ),
            nested_occurrences: Some(
                vocabulary
                    .nested_occurrences
                    .iter()
                    .map(|name| ((*name).to_owned(), count))
                    .collect(),
            ),
            statically_unserved: statically_unserved_effects(),
        },
    }
}

#[cfg(target_os = "linux")]
fn aggregate_fixture_root() -> &'static str {
    "/tmp/skit-ui-walker-v1"
}

#[cfg(target_os = "windows")]
fn aggregate_fixture_root() -> &'static str {
    r"C:\tmp\skit-ui-walker-v1"
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn aggregate_fixture_corpus() -> RecordedRealCorpus {
    let template = record_real_smoke_main().unwrap();
    let (operations, final_liveness) = canonical_corpus_artifact_values().unwrap();
    let profiles = skit_tui_walker_support::required_review_profiles()
        .iter()
        .map(|required| {
            let id = SafeProfileId::try_from(required.id).unwrap();
            let mut recorded = template.clone();
            recorded.trace.review_profile = id.clone();
            recorded.trace.locale = required.locale.to_owned();
            recorded.trace.viewport = required.viewport;
            recorded.trace.operations = operations.clone();
            recorded.trace.final_liveness_requested = final_liveness.clone();
            for row in &mut recorded.trace.rows {
                row.profile = id.clone();
                row.locale = required.locale.to_owned();
            }
            recorded.sandbox = SandboxMetadata::stable(
                template.sandbox.platform(),
                aggregate_fixture_root(),
                BTreeSet::from([id]),
            )
            .unwrap();
            recorded.leak_oracle_facts = LeakOracleFacts::default();
            recorded
        })
        .collect();
    RecordedRealCorpus::new(profiles).unwrap()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn aggregate_fixture_profiles(operations_sha256: &str) -> Vec<ProfileManifest> {
    skit_tui_walker_support::required_review_profiles()
        .iter()
        .map(|required| {
            let id = SafeProfileId::try_from(required.id).unwrap();
            ProfileManifest {
                id: id.clone(),
                initial_locale: required.locale.to_owned(),
                initial_viewport: required.viewport,
                operations_sha256: operations_sha256.to_owned(),
                cast_sha256: "0".repeat(64),
                cast_byte_size: 1,
                row_count: 1,
                maximum_chunk_rows: u32::try_from(MAXIMUM_CHUNK_ROWS).unwrap(),
                chunks: vec![ChunkDescriptor {
                    id: format!("{id}-0000"),
                    profile: id,
                    start_sequence: 0,
                    end_sequence: 1,
                    row_count: 1,
                    sha256: "1".repeat(64),
                    first_row_sha256: "2".repeat(64),
                    last_row_sha256: "3".repeat(64),
                    first_chain: None,
                    last_chain: Some(EventChainIdentity {
                        phase: TimelinePhase::Operations,
                        sequence: 0,
                    }),
                    continues_previous_chain: false,
                    continues_next_chain: false,
                }],
            }
        })
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn aggregate_root_documents_keep_manifest_and_run_orders_distinct() {
    let corpus = aggregate_fixture_corpus();
    let operations =
        canonical_json_bytes(&Value::Array(corpus.profiles[0].operations.clone())).unwrap();
    let profiles = aggregate_fixture_profiles(&sha256_hex(&operations));
    let profile_coverage = profiles
        .iter()
        .map(|profile| (profile.id.clone(), complete_required_coverage(1)))
        .collect::<Vec<_>>();
    let coverage = merge_required_profile_effect_coverage(&profile_coverage).unwrap();
    let documents =
        ReviewCorpusDocuments::new(&corpus, profiles.clone(), &profile_coverage, "revision")
            .unwrap();
    let directory = tempfile::TempDir::new().unwrap();
    let writer = BundleWriter::create_common(directory.path()).unwrap();
    writer.write_root_documents(&documents).unwrap();
    let invalid_directory = tempfile::TempDir::new().unwrap();
    let invalid_writer = BundleWriter::create_common(invalid_directory.path()).unwrap();
    // The fixture changes the profile and the locale of each row, but it does not seal the row
    // chain again. The writer validates each profile timeline before it writes.
    assert_eq!(
        invalid_writer
            .write_review_corpus(&corpus, "revision")
            .unwrap_err(),
        "timeline predecessor digest does not match"
    );

    assert_eq!(documents.manifest.profiles, profiles);
    assert_eq!(
        documents.run.profiles,
        corpus
            .sandbox
            .profiles()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_ne!(
        documents.run.profiles,
        documents
            .manifest
            .profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(documents.operations, operations);
    assert_eq!(documents.coverage, coverage);
    assert_eq!(
        documents.final_liveness,
        canonical_json_bytes(&corpus.profiles[0].final_liveness_requested).unwrap()
    );
    assert_eq!(documents.readme, REVIEW_GUIDE);
    assert_eq!(documents.review, REVIEW_TEMPLATE);
    assert!(documents.progress.ends_with(b"\n"));
    assert!(!documents.manifest_bytes.ends_with(b"\n"));
    assert!(!documents.coverage_bytes.ends_with(b"\n"));
    assert!(!documents.run_bytes.ends_with(b"\n"));
    for (relative, expected) in [
        (OPERATIONS_FILE, documents.operations.as_slice()),
        (FINAL_LIVENESS_FILE, documents.final_liveness.as_slice()),
        (RUN_FILE, documents.run_bytes.as_slice()),
        (MANIFEST_FILE, documents.manifest_bytes.as_slice()),
        (COVERAGE_FILE, documents.coverage_bytes.as_slice()),
        (REVIEW_PROGRESS_FILE, documents.progress.as_slice()),
        (README_FILE, documents.readme.as_slice()),
        (REVIEW_FILE, documents.review.as_slice()),
    ] {
        assert_eq!(fs::read(directory.path().join(relative)).unwrap(), expected);
    }
}

#[test]
fn required_profile_coverage_refuses_missing_extra_static_and_zero_mutations() {
    let mut valid = complete_required_coverage(1);
    valid
        .effects
        .served_requests
        .as_mut()
        .unwrap()
        .insert("open.run".to_owned(), 7);
    validate_required_effect_coverage_summary(&valid).unwrap();

    let mut missing = valid.clone();
    missing
        .effects
        .served_requests
        .as_mut()
        .unwrap()
        .remove("open.run");
    assert!(validate_required_effect_coverage_summary(&missing).is_err());

    let mut extra_outer = valid.clone();
    extra_outer
        .effects
        .served_requests
        .as_mut()
        .unwrap()
        .insert("unexpected".to_owned(), 1);
    assert!(validate_required_effect_coverage_summary(&extra_outer).is_err());

    let mut missing_nested = valid.clone();
    missing_nested
        .effects
        .nested_occurrences
        .as_mut()
        .unwrap()
        .remove("add.cancel");
    assert!(validate_required_effect_coverage_summary(&missing_nested).is_err());

    let mut extra_nested = valid.clone();
    extra_nested
        .effects
        .nested_occurrences
        .as_mut()
        .unwrap()
        .insert("preferences.unexpected".to_owned(), 1);
    assert!(validate_required_effect_coverage_summary(&extra_nested).is_err());

    let mut wrong_static = valid.clone();
    wrong_static.effects.statically_unserved.pop();
    assert!(validate_required_effect_coverage_summary(&wrong_static).is_err());

    let mut zero = valid;
    zero.effects
        .nested_occurrences
        .as_mut()
        .unwrap()
        .insert("add.cancel".to_owned(), 0);
    assert!(validate_required_effect_coverage_summary(&zero).is_err());

    let unmeasured = CoverageSummary::unmeasured_effects(statically_unserved_effects()).unwrap();
    assert!(validate_required_effect_coverage_summary(&unmeasured).is_err());
}

fn required_effect_fixtures() -> Vec<Effect> {
    let mut effects = vec![
        effect_fixture(json!("reload")),
        effect_fixture(json!({"rerun": {"selector": "entry"}})),
    ];
    for request in [
        "add",
        "health",
        "preferences",
        "presets",
        "rename",
        "run",
        "runners",
        "settings",
    ] {
        effects.push(effect_fixture(
            json!({"open": {"request": request, "selector": "entry"}}),
        ));
    }
    for purpose in ["rename", "run", "settings"] {
        effects.push(effect_fixture(json!({
            "submit": {"purpose": purpose, "selector": "entry", "values": {}},
        })));
    }
    effects.extend([
        effect_fixture(json!({
            "count_run_glob": {
                "selector": "entry",
                "field": 0,
                "value": "*",
                "request": {"cwd": "/cwd", "pieces": ["*"]},
            },
        })),
        effect_fixture(json!({
            "save_run_preset": {
                "selector": "entry",
                "name": "preset",
                "values": {},
                "secret_names": [],
            },
        })),
        effect_fixture(json!({"add": coverage_add_effect_values()})),
        effect_fixture(json!("health_rebuild")),
        effect_fixture(json!({
            "save_runner": {
                "request": {"name": "runner", "argv": ["runner"], "target": "new"},
                "owner": "manager",
            },
        })),
        effect_fixture(json!({
            "remove_runner": {
                "named": {"name": "runner", "expected": [], "expected_pinned_count": 0},
            },
        })),
        effect_fixture(json!("refresh_preferences_after_runners")),
        effect_fixture(json!({"preferences": {"save": {"settings": {}}}})),
        effect_fixture(json!({"preferences": "manage_agents"})),
        effect_fixture(json!({"preferences": "discover_agent_skill_targets"})),
        effect_fixture(json!({
            "preferences": {"install_agent_skill": {"skills_dir": "/skills"}},
        })),
        effect_fixture(json!({"edit": {"selector": "entry"}})),
        effect_fixture(json!({"remove": {"selector": "entry"}})),
    ]);
    effects
}

#[test]
fn complete_timeline_builds_one_valid_required_profile_coverage_summary() {
    let effects = required_effect_fixtures();
    assert_eq!(effects.len(), 26);
    let rows = effects
        .iter()
        .map(|effect| {
            coverage_host_row(
                serde_json::to_value(effect).unwrap(),
                json!({"ignored": "host emitted"}),
            )
        })
        .collect::<Vec<_>>();

    let summary = measured_effect_coverage_from_timeline(&rows).unwrap();
    validate_required_effect_coverage_summary(&summary).unwrap();
    assert_eq!(summary.effects.served_requests.unwrap()["preferences"], 4);
}

fn required_profile_coverages(count: u64) -> Vec<(SafeProfileId, CoverageSummary)> {
    skit_tui_walker_support::required_review_profiles()
        .iter()
        .map(|profile| {
            (
                SafeProfileId::try_from(profile.id).unwrap(),
                complete_required_coverage(count),
            )
        })
        .collect()
}

#[test]
fn four_profile_coverage_merge_is_exact_checked_and_stored_comparable() {
    let profiles = required_profile_coverages(1);
    let aggregate = merge_required_profile_effect_coverage(&profiles).unwrap();
    assert!(
        aggregate
            .effects
            .served_requests
            .as_ref()
            .unwrap()
            .values()
            .all(|count| *count == 4)
    );
    assert!(
        aggregate
            .effects
            .nested_occurrences
            .as_ref()
            .unwrap()
            .values()
            .all(|count| *count == 4)
    );
    validate_stored_effect_coverage_summary(&aggregate, &profiles).unwrap();

    let mut changed = aggregate.clone();
    *changed
        .effects
        .served_requests
        .as_mut()
        .unwrap()
        .get_mut("reload")
        .unwrap() += 1;
    assert!(validate_stored_effect_coverage_summary(&changed, &profiles).is_err());

    let mut wrong_order = profiles.clone();
    wrong_order.swap(0, 1);
    assert!(merge_required_profile_effect_coverage(&wrong_order).is_err());
    let mut wrong_profile = profiles.clone();
    wrong_profile[0].0 = SafeProfileId::try_from("unexpected-profile").unwrap();
    assert!(merge_required_profile_effect_coverage(&wrong_profile).is_err());
    let mut missing = profiles.clone();
    missing.pop();
    assert!(merge_required_profile_effect_coverage(&missing).is_err());
    let mut extra = profiles.clone();
    extra.push((
        SafeProfileId::try_from("extra-profile").unwrap(),
        complete_required_coverage(1),
    ));
    assert!(merge_required_profile_effect_coverage(&extra).is_err());

    let mut overflow = profiles;
    overflow[0]
        .1
        .effects
        .served_requests
        .as_mut()
        .unwrap()
        .insert("reload".to_owned(), u64::MAX);
    assert!(merge_required_profile_effect_coverage(&overflow).is_err());
}

#[test]
fn measurement_without_a_host_request_is_measured_and_empty() {
    let rows = vec![coverage_test_row(TransitionCause::Initial)];
    let summary = measured_effect_coverage_from_timeline(&rows).unwrap();
    assert!(summary.effects.measured);
    assert!(summary.effects.served_requests.as_ref().unwrap().is_empty());
    assert!(
        summary
            .effects
            .nested_occurrences
            .as_ref()
            .unwrap()
            .is_empty()
    );
    assert!(validate_required_effect_coverage_summary(&summary).is_err());

    let profiles = skit_tui_walker_support::required_review_profiles()
        .iter()
        .map(|required| {
            (
                SafeProfileId::try_from(required.id).unwrap(),
                summary.clone(),
            )
        })
        .collect::<Vec<_>>();
    let aggregate = merge_profile_effect_coverage(&profiles).unwrap();
    assert_eq!(aggregate, summary);
    validate_stored_effect_coverage_summary(&aggregate, &profiles).unwrap();
    assert!(merge_required_profile_effect_coverage(&profiles).is_err());

    let mut unmeasured = profiles.clone();
    unmeasured[0].1 = CoverageSummary::unmeasured_effects(statically_unserved_effects()).unwrap();
    assert_eq!(
        merge_profile_effect_coverage(&unmeasured).unwrap_err(),
        "a profile effect coverage summary is not measured"
    );
}

#[test]
fn final_review_corpus_policy_requires_the_canonical_vector_and_vocabulary() {
    let (operations, _) = canonical_corpus_artifact_values().unwrap();
    let canonical = canonical_json_bytes(&Value::Array(operations.clone())).unwrap();
    assert_eq!(canonical.len(), 7839);
    assert_eq!(
        sha256_hex(&canonical),
        "824a42732e08119f3c57286f636d3f12936e3a6b0aa99947c8da17db92d0e922"
    );
    let profiles = required_profile_coverages(1);
    let stored = merge_required_profile_effect_coverage(&profiles).unwrap();
    validate_final_review_corpus(&canonical, operations.len(), &profiles, &stored).unwrap();

    assert_eq!(
        validate_final_review_corpus(&canonical, operations.len() - 1, &profiles, &stored)
            .unwrap_err(),
        "the review corpus does not have the canonical operation count"
    );
    let mut short = canonical.clone();
    short.pop();
    assert_eq!(
        validate_final_review_corpus(&short, operations.len(), &profiles, &stored).unwrap_err(),
        "the review corpus operations are not the canonical vector"
    );
    let mut incomplete = profiles.clone();
    incomplete[0]
        .1
        .effects
        .served_requests
        .as_mut()
        .unwrap()
        .remove("reload");
    assert!(
        validate_final_review_corpus(&canonical, operations.len(), &incomplete, &stored).is_err()
    );
    let mut changed = stored;
    *changed
        .effects
        .served_requests
        .as_mut()
        .unwrap()
        .get_mut("reload")
        .unwrap() += 1;
    assert_eq!(
        validate_final_review_corpus(&canonical, operations.len(), &profiles, &changed)
            .unwrap_err(),
        "the review corpus coverage is not the required aggregate"
    );
}

fn error_message(error: impl Display) -> String {
    error.to_string()
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    condition.then_some(()).ok_or_else(|| message.to_owned())
}

/// Convert one file operation into the shared path-aware error shape.
fn io<T>(result: io::Result<T>, path: &Path) -> Result<T, String> {
    result.map_err(|error| format!("{}: {error}", path.display()))
}

/// Encode the untouched review progress the writer stores with a fresh corpus.
fn template_review_progress_bytes(manifest: &CorpusManifest) -> Result<Vec<u8>, String> {
    let progress = ReviewProgress {
        manifest_sha256: manifest_digest(manifest).map_err(error_message)?,
        complete: false,
        review_sha256: None,
        reviewed_chunks: BTreeMap::new(),
        profile_verdicts: BTreeMap::new(),
    };
    review_progress_bytes(manifest, &progress).map_err(error_message)
}

struct ReviewCorpusDocuments {
    operations: Vec<u8>,
    final_liveness: Vec<u8>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    run: RunMetadata,
    run_bytes: Vec<u8>,
    manifest: CorpusManifest,
    manifest_bytes: Vec<u8>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    coverage: CoverageSummary,
    coverage_bytes: Vec<u8>,
    progress: Vec<u8>,
    readme: Vec<u8>,
    review: Vec<u8>,
}

impl ReviewCorpusDocuments {
    fn new(
        corpus: &RecordedRealCorpus,
        profiles: Vec<ProfileManifest>,
        profile_coverage: &[(SafeProfileId, CoverageSummary)],
        revision: &str,
    ) -> Result<Self, String> {
        let coverage = merge_profile_effect_coverage(profile_coverage)?;
        let validated = RecordedRealCorpus::new(corpus.profiles.clone())?;
        require(
            validated.sandbox == corpus.sandbox,
            "recorded corpus aggregate metadata is not canonical",
        )?;
        let first = corpus
            .profiles
            .first()
            .ok_or_else(|| "the recorded corpus has no profiles".to_owned())?;
        let operations =
            canonical_json_bytes(&Value::Array(first.operations.clone())).map_err(error_message)?;
        let operations_sha256 = sha256_hex(&operations);
        let final_liveness =
            canonical_json_bytes(&first.final_liveness_requested).map_err(error_message)?;
        let final_liveness_sha256 = sha256_hex(&final_liveness);

        let manifest = CorpusManifest {
            schema: 1,
            operations_sha256: operations_sha256.clone(),
            profiles,
        };
        let manifest_bytes = manifest_bytes(&manifest).map_err(error_message)?;
        let coverage_bytes = bundle::coverage_summary_bytes(&coverage).map_err(error_message)?;
        let progress = template_review_progress_bytes(&manifest)?;

        let run = RunMetadata {
            schema: 2,
            source_revision: revision.to_owned(),
            deterministic_result: "passed".to_owned(),
            operation_count: first.operations.len(),
            operations_sha256,
            final_liveness_sha256,
            profiles: corpus.sandbox.profiles().keys().cloned().collect(),
            sandbox: corpus.sandbox.clone(),
        };
        let run_bytes = bundle::run_metadata_bytes(&run).map_err(error_message)?;
        Ok(Self {
            operations,
            final_liveness,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            run,
            run_bytes,
            manifest,
            manifest_bytes,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            coverage,
            coverage_bytes,
            progress,
            readme: REVIEW_GUIDE.to_vec(),
            review: REVIEW_TEMPLATE.to_vec(),
        })
    }
}

struct BundleWriter {
    root: PathBuf,
}

impl BundleWriter {
    fn create_common(root: &Path) -> Result<Self, String> {
        let writer = Self {
            root: root.to_path_buf(),
        };
        for relative in [
            PathBuf::from(OBJECT_DIRECTORY),
            PathBuf::from(VIEW_DIRECTORY),
            PathBuf::from(PROFILE_DIRECTORY),
            PathBuf::from(VIEW_DIRECTORY).join(FRAME_VIEW_DIRECTORY),
        ] {
            let path = writer.root.join(relative);
            io(fs::create_dir_all(&path), &path)?;
        }
        for kind in [
            ObjectKind::Reducer,
            ObjectKind::Host,
            ObjectKind::Session,
            ObjectKind::StyledFrame,
            ObjectKind::Geometry,
        ] {
            let path = writer
                .root
                .join(OBJECT_DIRECTORY)
                .join(bundle::kind_directory(kind));
            io(fs::create_dir_all(&path), &path)?;
            if kind != ObjectKind::StyledFrame {
                let path = writer
                    .root
                    .join(VIEW_DIRECTORY)
                    .join(bundle::kind_directory(kind));
                io(fs::create_dir_all(&path), &path)?;
            }
        }
        Ok(writer)
    }

    fn prepare_profile(&self, profile: &SafeProfileId) -> Result<(), String> {
        let path = self
            .root
            .join(bundle::profile_directory(profile).join(CHUNK_DIRECTORY));
        io(fs::create_dir_all(&path), &path)
    }

    fn create(root: &Path, profile: &SafeProfileId) -> Result<Self, String> {
        let writer = Self::create_common(root)?;
        writer.prepare_profile(profile)?;
        Ok(writer)
    }

    /// Write canonical object bytes after verification against their reference.
    fn put_object(&self, reference: &ObjectRef, bytes: &[u8]) -> Result<(), String> {
        let value = validate_object(reference, bytes).map_err(error_message)?;
        self.write_same_or_new(&bundle::object_path(reference), bytes)?;
        let view = if reference.kind == ObjectKind::StyledFrame {
            let frame: StyledFrameSnapshot =
                serde_json::from_value(value).map_err(error_message)?;
            bundle::frame_view_bytes(&frame).map_err(error_message)?
        } else {
            bundle::object_view_bytes(&value).map_err(error_message)?
        };
        self.write_same_or_new(&bundle::object_view_path(reference), &view)
    }

    /// Compare or create one artifact. Refuse different existing bytes.
    fn write_same_or_new(&self, relative: &Path, bytes: &[u8]) -> Result<(), String> {
        let path = self.root.join(relative);
        if path.exists() {
            let existing = io(fs::read(&path), &path)?;
            require(
                existing == bytes,
                &format!("bundle artifact collision at {}", path.display()),
            )
        } else {
            io(fs::write(&path, bytes), &path)
        }
    }

    fn write_profile_trace(
        &self,
        recorded: &RecordedRealTrace,
        operations_sha256: &str,
    ) -> Result<ProfileManifest, String> {
        let trace = &recorded.trace;
        let operations =
            canonical_json_bytes(&Value::Array(trace.operations.clone())).map_err(error_message)?;
        require(
            sha256_hex(&operations) == operations_sha256,
            "profile operations do not match the shared digest",
        )?;
        self.prepare_profile(&trace.review_profile)?;

        for ((kind, digest), bytes) in &trace.objects {
            self.put_object(
                &ObjectRef {
                    kind: *kind,
                    sha256: digest.clone(),
                },
                bytes,
            )?;
        }

        validate_timeline_semantics(
            &trace.rows,
            &trace.operations,
            &trace.final_liveness_requested,
            &trace.locale,
        )
        .map_err(error_message)?;
        let timeline = bundle::timeline_ndjson_bytes(&trace.rows).map_err(error_message)?;
        let profile_root = bundle::profile_directory(&trace.review_profile);
        self.write_same_or_new(&profile_root.join(TIMELINE_FILE), &timeline)?;

        validate_asciicast_v3(&trace.cast).map_err(error_message)?;
        self.write_same_or_new(&profile_root.join(CAST_FILE), &trace.cast)?;

        let chunks = build_chunks(&trace.review_profile, &trace.rows, MAXIMUM_CHUNK_ROWS)
            .map_err(error_message)?;
        let profile = ProfileManifest {
            id: trace.review_profile.clone(),
            initial_locale: trace.locale.clone(),
            initial_viewport: trace.viewport,
            operations_sha256: operations_sha256.to_owned(),
            cast_sha256: sha256_hex(&trace.cast),
            cast_byte_size: u64::try_from(trace.cast.len()).map_err(error_message)?,
            row_count: u32::try_from(trace.rows.len()).map_err(error_message)?,
            maximum_chunk_rows: u32::try_from(MAXIMUM_CHUNK_ROWS).map_err(error_message)?,
            chunks,
        };
        let profile_bytes =
            canonical_json_bytes(&serde_json::to_value(&profile).map_err(error_message)?)
                .map_err(error_message)?;
        self.write_same_or_new(&profile_root.join(PROFILE_FILE), &profile_bytes)?;

        let mut cursor = ChunkViewCursor::new();
        for chunk in &profile.chunks {
            let start = usize::try_from(chunk.start_sequence).map_err(error_message)?;
            let end = usize::try_from(chunk.end_sequence).map_err(error_message)?;
            let rows = trace
                .rows
                .get(start..end)
                .ok_or_else(|| "chunk range is outside the real trace".to_owned())?;
            let bytes = bundle::chunk_json_bytes(rows).map_err(error_message)?;
            self.write_same_or_new(
                &bundle::chunk_json_path(&trace.review_profile, &chunk.id),
                &bytes,
            )?;
            let view = cursor
                .chunk_view_bytes(&trace.rows, chunk)
                .map_err(error_message)?;
            self.write_same_or_new(
                &bundle::chunk_view_path(&trace.review_profile, &chunk.id),
                &view,
            )?;
        }

        Ok(profile)
    }

    fn write_trace(
        &self,
        recorded: &RecordedRealTrace,
        revision: &str,
    ) -> Result<ProfileManifest, String> {
        let trace = &recorded.trace;
        let operations =
            canonical_json_bytes(&Value::Array(trace.operations.clone())).map_err(error_message)?;
        let operations_sha256 = sha256_hex(&operations);
        self.write_same_or_new(Path::new(OPERATIONS_FILE), &operations)?;

        let final_liveness =
            canonical_json_bytes(&trace.final_liveness_requested).map_err(error_message)?;
        let final_liveness_sha256 = sha256_hex(&final_liveness);
        self.write_same_or_new(Path::new(FINAL_LIVENESS_FILE), &final_liveness)?;
        let profile = self.write_profile_trace(recorded, &operations_sha256)?;

        let run = RunMetadata {
            schema: 2,
            source_revision: revision.to_owned(),
            deterministic_result: "passed".to_owned(),
            operation_count: trace.operations.len(),
            operations_sha256,
            final_liveness_sha256,
            profiles: recorded.sandbox.profiles().keys().cloned().collect(),
            sandbox: recorded.sandbox.clone(),
        };
        require(
            run.profiles.as_slice() == std::slice::from_ref(&trace.review_profile),
            "trace profile does not match its sandbox metadata",
        )?;
        let run_bytes = bundle::run_metadata_bytes(&run).map_err(error_message)?;
        self.write_same_or_new(Path::new(RUN_FILE), &run_bytes)?;
        Ok(profile)
    }

    fn write_root_documents(&self, documents: &ReviewCorpusDocuments) -> Result<(), String> {
        for (relative, bytes) in [
            (OPERATIONS_FILE, documents.operations.as_slice()),
            (FINAL_LIVENESS_FILE, documents.final_liveness.as_slice()),
            (RUN_FILE, documents.run_bytes.as_slice()),
            (MANIFEST_FILE, documents.manifest_bytes.as_slice()),
            (COVERAGE_FILE, documents.coverage_bytes.as_slice()),
            (REVIEW_PROGRESS_FILE, documents.progress.as_slice()),
            (README_FILE, documents.readme.as_slice()),
            (REVIEW_FILE, documents.review.as_slice()),
        ] {
            self.write_same_or_new(Path::new(relative), bytes)?;
        }
        Ok(())
    }

    fn write_review_corpus(
        &self,
        corpus: &RecordedRealCorpus,
        revision: &str,
    ) -> Result<CorpusManifest, String> {
        let validated = RecordedRealCorpus::new(corpus.profiles.clone())?;
        require(
            validated.sandbox == corpus.sandbox,
            "recorded corpus aggregate metadata is not canonical",
        )?;
        let operations =
            canonical_json_bytes(&Value::Array(corpus.profiles[0].trace.operations.clone()))
                .map_err(error_message)?;
        let operations_sha256 = sha256_hex(&operations);
        let mut profiles = Vec::new();
        let mut coverage = Vec::new();
        for recorded in &corpus.profiles {
            profiles.push(self.write_profile_trace(recorded, &operations_sha256)?);
            coverage.push((
                recorded.review_profile.clone(),
                measured_effect_coverage_from_timeline(&recorded.rows)?,
            ));
        }
        let documents = ReviewCorpusDocuments::new(corpus, profiles, &coverage, revision)?;
        self.write_root_documents(&documents)?;
        Ok(documents.manifest)
    }
}

fn walk_tree(
    root: &Path,
    relative: &Path,
    layout: &mut BundleLayout,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), String> {
    let directory = root.join(relative);
    for entry in io(fs::read_dir(&directory), &directory)? {
        let entry = io(entry, &directory)?;
        let child_relative = relative.join(entry.file_name());
        let path = root.join(&child_relative);
        let metadata = io(fs::symlink_metadata(&path), &path)?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "walker bundle contains a symlink: {}",
                child_relative.display()
            ));
        }
        if metadata.is_dir() {
            layout.insert_directory(child_relative.clone());
            walk_tree(root, &child_relative, layout, files)?;
        } else if metadata.is_file() {
            layout.insert_file(child_relative.clone());
            files.insert(child_relative, io(fs::read(&path), &path)?);
        } else {
            return Err(format!(
                "walker bundle contains a non-regular entry: {}",
                child_relative.display()
            ));
        }
    }
    Ok(())
}

fn read_tree(root: &Path) -> Result<(BundleLayout, BTreeMap<PathBuf, Vec<u8>>), String> {
    let metadata = io(fs::symlink_metadata(root), root)?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "walker bundle root is a symlink: {}",
            root.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "walker bundle root is not a directory: {}",
            root.display()
        ));
    }
    let mut layout = BundleLayout::default();
    let mut files = BTreeMap::new();
    walk_tree(root, Path::new(""), &mut layout, &mut files)?;
    Ok((layout, files))
}

fn artifact<'a>(
    files: &'a BTreeMap<PathBuf, Vec<u8>>,
    relative: &Path,
) -> Result<&'a [u8], String> {
    files
        .get(relative)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("walker bundle is missing file: {}", relative.display()))
}

fn decode<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T, String> {
    let value = bundle::decode_canonical_bytes(bytes).map_err(error_message)?;
    let decoded: T = serde_json::from_value(value).map_err(error_message)?;
    let encoded = canonical_json_bytes(&serde_json::to_value(&decoded).map_err(error_message)?)
        .map_err(error_message)?;
    require(
        encoded == bytes,
        "typed JSON value does not match its stored bytes",
    )?;
    Ok(decoded)
}

fn value_array(value: Value) -> Result<Vec<Value>, String> {
    match value {
        Value::Array(values) => Ok(values),
        _ => Err("operations.json is not an operation array".to_owned()),
    }
}

fn object_key(reference: &ObjectRef) -> (ObjectKind, String) {
    (reference.kind, reference.sha256.clone())
}

fn read_objects(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    rows: &[TimelineRow],
) -> Result<BTreeMap<(ObjectKind, String), Value>, String> {
    let mut objects = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        for reference in [
            &row.reducer,
            &row.host,
            &row.session,
            &row.styled_frame,
            &row.geometry,
        ] {
            if seen.insert(object_key(reference)) {
                let bytes = artifact(files, &bundle::object_path(reference))?;
                let value = validate_object(reference, bytes).map_err(error_message)?;
                let expected_view = if reference.kind == ObjectKind::StyledFrame {
                    let frame: StyledFrameSnapshot =
                        serde_json::from_value(value.clone()).map_err(error_message)?;
                    let typed_value = serde_json::to_value(&frame).map_err(error_message)?;
                    let typed_bytes = canonical_json_bytes(&typed_value).map_err(error_message)?;
                    let stored_bytes = canonical_json_bytes(&value).map_err(error_message)?;
                    require(
                        typed_bytes == stored_bytes,
                        "typed styled frame does not match its stored bytes",
                    )?;
                    bundle::frame_view_bytes(&frame).map_err(error_message)?
                } else {
                    bundle::object_view_bytes(&value).map_err(error_message)?
                };
                let actual_view = artifact(files, &bundle::object_view_path(reference))?;
                require(
                    actual_view == expected_view,
                    "object view bytes do not match their canonical object",
                )?;
                objects.insert(object_key(reference), value);
            }
        }
    }
    Ok(objects)
}

fn stored_object<'a>(
    objects: &'a BTreeMap<(ObjectKind, String), Value>,
    reference: &ObjectRef,
) -> Result<&'a Value, String> {
    objects
        .get(&object_key(reference))
        .ok_or_else(|| "a timeline object is absent from the bundle".to_owned())
}

fn host_field<'a>(host: &'a Value, field: &str) -> Result<&'a Value, String> {
    host.get(field)
        .ok_or_else(|| format!("a host object has no {field} field"))
}

fn validate_host_state(
    rows: &[TimelineRow],
    objects: &BTreeMap<(ObjectKind, String), Value>,
) -> Result<(), String> {
    for row in rows {
        let reducer = stored_object(objects, &row.reducer)?;
        let host = stored_object(objects, &row.host)?;
        require(
            host_field(host, "state")? == reducer,
            "a host object does not embed its row reducer state",
        )?;
        host_field(host, "transcript")?;
        for field in STORE_FIELDS {
            host_field(host, field)?;
        }
    }
    for pair in rows.windows(2) {
        let previous = &pair[0];
        let current = &pair[1];
        if current.boundary != TimelineBoundary::HostAction {
            let previous_host = stored_object(objects, &previous.host)?;
            let current_host = stored_object(objects, &current.host)?;
            // State mirrors the reducer by construction. Transcript is drained after each interval.
            for field in STORE_FIELDS {
                require(
                    host_field(previous_host, field)? == host_field(current_host, field)?,
                    "a store-quiet transition changed the host store",
                )?;
            }
        }
    }
    Ok(())
}

fn validate_live_locales(rows: &[TimelineRow], initial_locale: &str) -> Result<(), String> {
    let mut expected = initial_locale.to_owned();
    for row in rows {
        if let TransitionCause::Host { response, .. } = &row.cause {
            let action: skit_ui::Action =
                serde_json::from_value(response.clone()).map_err(error_message)?;
            if let skit_ui::Action::PreferencesSaved { locale, .. } = action {
                let negotiated = skit_i18n::detect_locale(Some(&locale));
                require(
                    locale == negotiated.tag(),
                    "a saved Preferences response locale is not canonical",
                )?;
                expected = negotiated.tag().to_owned();
            }
        }
        require(
            row.locale == expected,
            "timeline locale does not match the live reducer response",
        )?;
    }
    Ok(())
}

fn validate_reducer_replay(
    rows: &[TimelineRow],
    objects: &BTreeMap<(ObjectKind, String), Value>,
) -> Result<(), String> {
    for row in rows {
        parse_library_state(stored_object(objects, &row.reducer)?)?;
    }
    for pair in rows.windows(2) {
        let previous = stored_object(objects, &pair[0].reducer)?;
        let current = stored_object(objects, &pair[1].reducer)?;
        match &pair[1].cause {
            TransitionCause::Initial => {
                return Err("only the first reducer object can be initial".to_owned());
            }
            TransitionCause::Session { .. } => require(
                current == previous,
                "a session checkpoint changed reducer state",
            )?,
            TransitionCause::Reducer {
                action, emitted, ..
            } => validate_reducer_action(previous, current, action, emitted)?,
            TransitionCause::Host {
                response, emitted, ..
            } => validate_reducer_action(previous, current, response, emitted)?,
        }
    }
    Ok(())
}

fn validate_chunks_and_views(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    profile_id: &SafeProfileId,
    profile: &ProfileManifest,
    rows: &[TimelineRow],
) -> Result<(), String> {
    let maximum = usize::try_from(profile.maximum_chunk_rows).map_err(error_message)?;
    validate_chunks(&profile.id, rows, &profile.chunks, maximum).map_err(error_message)?;
    let mut cursor = ChunkViewCursor::new();
    for chunk in &profile.chunks {
        let bytes = artifact(files, &bundle::chunk_json_path(profile_id, &chunk.id))?;
        let chunk_rows: Vec<TimelineRow> = decode(bytes)?;
        require(
            sha256_hex(bytes) == chunk.sha256,
            "chunk digest does not match its canonical row file",
        )?;
        let start = usize::try_from(chunk.start_sequence).map_err(error_message)?;
        let end = usize::try_from(chunk.end_sequence).map_err(error_message)?;
        require(
            rows.get(start..end) == Some(chunk_rows.as_slice()),
            "chunk rows do not match their timeline range",
        )?;
        let expected_view = cursor
            .chunk_view_bytes(rows, chunk)
            .map_err(error_message)?;
        let actual_view = artifact(files, &bundle::chunk_view_path(profile_id, &chunk.id))?;
        require(
            actual_view == expected_view,
            "chunk view bytes do not match their timeline rows",
        )?;
    }
    Ok(())
}

fn validate_cast(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    profile_id: &SafeProfileId,
    profile: &ProfileManifest,
    rows: &[TimelineRow],
    objects: &BTreeMap<(ObjectKind, String), Value>,
) -> Result<(), String> {
    let cast_path = bundle::profile_directory(profile_id).join(CAST_FILE);
    let cast = artifact(files, &cast_path)?;
    validate_asciicast_v3(cast).map_err(error_message)?;
    require(
        sha256_hex(cast) == profile.cast_sha256,
        "profile cast digest does not match trace.cast",
    )?;
    require(
        u64::try_from(cast.len()).map_err(error_message)? == profile.cast_byte_size,
        "profile cast byte size does not match trace.cast",
    )?;

    let rebuilt = bundle::rebuild_presented_cast(rows, |reference| {
        let value = stored_object(objects, reference)?.clone();
        serde_json::from_value(value).map_err(error_message)
    })
    .map_err(error_message)?;
    require(
        rebuilt == cast,
        "stored styled frames did not rebuild trace.cast",
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeakScanMode {
    ZeroTolerance,
    Renderer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForbiddenTextKind {
    SandboxRoot,
    ArtifactPath,
    DraftName,
}

#[derive(Debug)]
struct ForbiddenTextFact {
    text: String,
    kind: ForbiddenTextKind,
    description: &'static str,
}

#[derive(Debug)]
struct InstalledBundleLeakOracle<'a> {
    facts: &'a LeakOracleFacts,
    forbidden_text: Vec<ForbiddenTextFact>,
    sandbox_roots: Vec<&'a str>,
    sandbox: &'a SandboxMetadata,
}

impl<'a> InstalledBundleLeakOracle<'a> {
    fn new(run: &'a RunMetadata, facts: &'a LeakOracleFacts) -> Result<Self, String> {
        Self::for_sandbox(&run.sandbox, facts)
    }

    fn for_sandbox(
        sandbox: &'a SandboxMetadata,
        facts: &'a LeakOracleFacts,
    ) -> Result<Self, String> {
        let mut sandbox_roots = vec![sandbox.root()];
        for fact in facts.artifacts.values() {
            if sandbox
                .profiles()
                .values()
                .any(|root| fact.raw_path_spellings.contains(root))
            {
                sandbox_roots.extend(fact.raw_path_spellings.iter().map(String::as_str));
            }
        }
        let mut forbidden = BTreeMap::new();
        insert_forbidden_text(
            &mut forbidden,
            sandbox.root(),
            ForbiddenTextKind::SandboxRoot,
            "raw sandbox root",
        );
        for profile_root in sandbox.profiles().values() {
            let roots = SandboxRoots::new(sandbox.platform(), profile_root.clone())
                .map_err(error_message)?;
            for root in [
                roots.root(),
                roots.data(),
                roots.state(),
                roots.config(),
                roots.home(),
                roots.cwd(),
                roots.external(),
                roots.system_temp(),
            ] {
                insert_forbidden_text(
                    &mut forbidden,
                    root,
                    ForbiddenTextKind::SandboxRoot,
                    "raw sandbox root",
                );
            }
        }
        for (projected_path, artifact) in &facts.artifacts {
            validate_artifact_fact_disjointness(projected_path, artifact)?;
            for raw in &artifact.raw_path_spellings {
                insert_forbidden_text(
                    &mut forbidden,
                    raw,
                    ForbiddenTextKind::ArtifactPath,
                    "raw artifact path",
                );
            }
        }

        for (projected_path, draft) in &facts.renderer_drafts {
            require(
                projected_path == &draft.projected_path,
                "a renderer draft fact has a mismatched projected path",
            )?;
            insert_forbidden_text(
                &mut forbidden,
                &draft.raw_path,
                ForbiddenTextKind::ArtifactPath,
                "raw draft path",
            );
            for basename in [&draft.raw_kind_picker_basename, &draft.raw_lossy_basename] {
                insert_forbidden_text(
                    &mut forbidden,
                    basename,
                    ForbiddenTextKind::DraftName,
                    "raw draft basename",
                );
                if let Some(stem) = allocator_stem(basename) {
                    insert_forbidden_text(
                        &mut forbidden,
                        stem,
                        ForbiddenTextKind::DraftName,
                        "raw draft stem",
                    );
                }
            }
            for review in &draft.review_names {
                require(
                    review.raw_source_path == draft.raw_path
                        && review.projected_source_path == draft.projected_path,
                    "a renderer review-name fact has mismatched source provenance",
                )?;
                insert_forbidden_text(
                    &mut forbidden,
                    &review.raw_source_path,
                    ForbiddenTextKind::ArtifactPath,
                    "raw draft path",
                );
                insert_forbidden_text(
                    &mut forbidden,
                    &review.raw_name,
                    ForbiddenTextKind::DraftName,
                    "raw draft review name",
                );
            }
        }
        let mut forbidden_text = forbidden
            .into_iter()
            .map(|(text, (kind, description))| ForbiddenTextFact {
                text,
                kind,
                description,
            })
            .collect::<Vec<_>>();
        forbidden_text.sort_by(|left, right| {
            right
                .text
                .len()
                .cmp(&left.text.len())
                .then_with(|| left.text.cmp(&right.text))
        });
        Ok(Self {
            facts,
            forbidden_text,
            sandbox_roots,
            sandbox,
        })
    }

    fn scan_json(&self, path: &Path, value: &Value, mode: LeakScanMode) -> Result<(), String> {
        for occurrence in json_string_occurrences(value) {
            let context = format!("{} {:?}", occurrence.pointer, occurrence.role);
            self.scan_text(path, &context, occurrence.text, mode)?;
        }
        self.validate_artifact_values(path, value, "")
    }

    fn scan_text(
        &self,
        path: &Path,
        context: &str,
        text: &str,
        mode: LeakScanMode,
    ) -> Result<(), String> {
        self.scan_text_with_frame_edge(path, context, text, mode, None)
    }

    fn scan_text_with_frame_edge(
        &self,
        path: &Path,
        context: &str,
        text: &str,
        mode: LeakScanMode,
        right_content_edge: Option<usize>,
    ) -> Result<(), String> {
        let content = right_content_edge.map_or(text, |end| &text[..end]);
        let stable_frame =
            mode == LeakScanMode::Renderer && self.sandbox.mode() == SandboxMode::Stable;
        let stable_roots = if stable_frame {
            self.sandbox_roots
                .iter()
                .flat_map(|root| exact_fact_spans(content, root))
                .collect()
        } else {
            Vec::new()
        };
        for fact in &self.forbidden_text {
            if mode == LeakScanMode::Renderer {
                if fact.kind == ForbiddenTextKind::DraftName {
                    continue;
                }
                if stable_frame
                    && self.sandbox_roots.iter().any(|root| {
                        exact_fact_spans(&fact.text, root)
                            .iter()
                            .any(|span| span.start == 0)
                    })
                {
                    continue;
                }
            }
            if !exact_fact_spans(content, &fact.text).is_empty() {
                return Err(format!(
                    "{}: leak oracle found {} at {context}",
                    path.display(),
                    fact.description
                ));
            }
        }
        for ambient in &self.facts.ambient_paths {
            for span in exact_fact_spans(content, ambient) {
                if stable_roots.iter().any(|root| span_is_inside(span, *root)) {
                    continue;
                }
                if stable_frame
                    && right_content_edge.is_some_and(|end| {
                        let prefix = text[span.start..end].trim_end();
                        prefix.len() > span.end - span.start
                            && self
                                .sandbox_roots
                                .iter()
                                .any(|root| root.starts_with(prefix))
                    })
                {
                    continue;
                }
                return Err(format!(
                    "{}: leak oracle found an ambient path at {context} ({ambient:?} in {text:?})",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    fn validate_artifact_values(
        &self,
        file: &Path,
        value: &Value,
        pointer: &str,
    ) -> Result<(), String> {
        match value {
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    self.validate_artifact_values(
                        file,
                        child,
                        &json_pointer_child(pointer, &index.to_string()),
                    )?;
                }
            }
            Value::Object(object) => {
                if is_typed_artifact_object(object) {
                    self.validate_artifact_value(file, object, pointer)?;
                }
                for (key, child) in object {
                    self.validate_artifact_values(
                        file,
                        child,
                        &json_pointer_child(pointer, &escape_json_pointer(key)),
                    )?;
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
        Ok(())
    }

    fn validate_artifact_value(
        &self,
        file: &Path,
        object: &serde_json::Map<String, Value>,
        pointer: &str,
    ) -> Result<(), String> {
        let identity = object
            .get("identity")
            .filter(|identity| !identity.is_null());
        let modified = object.get("modified");
        if identity.is_none() && modified.is_none() {
            return Ok(());
        }
        let projected_path = object.get("path").and_then(Value::as_str).ok_or_else(|| {
            format!(
                "{}: a typed artifact has no text path at {pointer}",
                file.display()
            )
        })?;
        let fact = self.facts.artifacts.get(projected_path).ok_or_else(|| {
            format!(
                "{}: a typed artifact has an unknown projected path at {pointer}",
                file.display()
            )
        })?;
        if let Some(identity) = identity {
            let bytes = canonical_json_bytes(identity).map_err(error_message)?;
            if fact.source_identities.iter().any(|pair| pair.raw == bytes) {
                return Err(format!(
                    "{}: leak oracle found raw source identity at {pointer}/identity",
                    file.display()
                ));
            }
            if !fact
                .source_identities
                .iter()
                .any(|pair| pair.projected == bytes)
            {
                return Err(format!(
                    "{}: leak oracle found unknown projected source identity at {pointer}/identity",
                    file.display()
                ));
            }
        }
        if let Some(modified) = modified {
            let modified = modified.as_u64().ok_or_else(|| {
                format!(
                    "{}: a typed artifact modified value is not a u64 at {pointer}",
                    file.display()
                )
            })?;
            if fact.modified_values.iter().any(|pair| pair.raw == modified) {
                return Err(format!(
                    "{}: leak oracle found raw draft modified value at {pointer}/modified",
                    file.display()
                ));
            }
            if !fact
                .modified_values
                .iter()
                .any(|pair| pair.projected == modified)
            {
                return Err(format!(
                    "{}: leak oracle found unknown projected draft modified value at {pointer}/modified",
                    file.display()
                ));
            }
        }
        Ok(())
    }
}

fn validate_artifact_fact_disjointness(
    projected_path: &str,
    fact: &ArtifactLeakOracleFact,
) -> Result<(), String> {
    if fact.source_identities.iter().any(|raw_pair| {
        fact.source_identities
            .iter()
            .any(|projected_pair| raw_pair.raw == projected_pair.projected)
    }) {
        return Err(format!(
            "artifact fact raw and projected source identities overlap: {projected_path}"
        ));
    }
    if fact.modified_values.iter().any(|raw_pair| {
        fact.modified_values
            .iter()
            .any(|projected_pair| raw_pair.raw == projected_pair.projected)
    }) {
        return Err(format!(
            "artifact fact raw and projected modified values overlap: {projected_path}"
        ));
    }
    Ok(())
}

fn insert_forbidden_text(
    forbidden: &mut BTreeMap<String, (ForbiddenTextKind, &'static str)>,
    text: &str,
    kind: ForbiddenTextKind,
    description: &'static str,
) {
    if !text.is_empty() {
        forbidden
            .entry(text.to_owned())
            .or_insert((kind, description));
    }
}

fn sandbox_separator(platform: SandboxPlatform) -> char {
    match platform {
        SandboxPlatform::Linux | SandboxPlatform::Macos => '/',
        SandboxPlatform::Windows => '\\',
    }
}

fn allocator_stem(token: &str) -> Option<&str> {
    parse_draft_allocator_token(token)
        .ok()
        .map(|_| &token[.."skit-new-".len() + 6])
}

fn span_is_inside(inner: TextSpan, outer: TextSpan) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn json_pointer_child(parent: &str, child: &str) -> String {
    format!("{parent}/{child}")
}

fn escape_json_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn is_typed_artifact_object(object: &serde_json::Map<String, Value>) -> bool {
    let draft = object.contains_key("path")
        && object.contains_key("modified")
        && object.contains_key("identity")
        && object.contains_key("permissions")
        && object.contains_key("content_hash");
    let source = object.contains_key("path")
        && object.contains_key("source_record")
        && object.contains_key("bytes")
        && object.contains_key("identity")
        && object.contains_key("is_regular")
        && object.contains_key("is_directory")
        && object.contains_key("is_draft");
    draft || source
}

fn scan_json_bytes(
    oracle: &InstalledBundleLeakOracle<'_>,
    path: &Path,
    bytes: &[u8],
    mode: LeakScanMode,
) -> Result<(), String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    oracle.scan_json(path, &value, mode)
}

fn scan_ndjson_bytes(
    oracle: &InstalledBundleLeakOracle<'_>,
    path: &Path,
    bytes: &[u8],
    mode: LeakScanMode,
) -> Result<(), String> {
    for (line_index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.is_empty() && line_index + 1 == bytes.split(|byte| *byte == b'\n').count() {
            continue;
        }
        let value: Value = serde_json::from_slice(line)
            .map_err(|error| format!("{} line {}: {error}", path.display(), line_index + 1))?;
        oracle.scan_json(path, &value, mode)?;
    }
    Ok(())
}

fn styled_frame_reference(path: &Path) -> Option<ObjectRef> {
    let directory =
        PathBuf::from(OBJECT_DIRECTORY).join(bundle::kind_directory(ObjectKind::StyledFrame));
    let relative = path.strip_prefix(directory).ok()?;
    if relative.components().count() != 1 {
        return None;
    }
    let digest = relative.file_name()?.to_str()?.strip_suffix(".json")?;
    Some(ObjectRef {
        kind: ObjectKind::StyledFrame,
        sha256: digest.to_owned(),
    })
}

fn scan_styled_frame(
    oracle: &InstalledBundleLeakOracle<'_>,
    path: &Path,
    bytes: &[u8],
) -> Result<ObjectRef, String> {
    let reference = styled_frame_reference(path)
        .ok_or_else(|| format!("{}: styled frame path is invalid", path.display()))?;
    let value = validate_object(&reference, bytes).map_err(error_message)?;
    let frame: StyledFrameSnapshot = serde_json::from_value(value).map_err(error_message)?;
    scan_frame(oracle, path, &frame)?;
    Ok(reference)
}

fn scan_frame(
    oracle: &InstalledBundleLeakOracle<'_>,
    path: &Path,
    frame: &StyledFrameSnapshot,
) -> Result<(), String> {
    for (row, text) in frame
        .readable_lines()
        .map_err(error_message)?
        .iter()
        .enumerate()
    {
        // The footer clips at the frame edge or just before its final rounded border.
        let right_content_edge = (text.as_str().cell_width() == frame.area.width).then(|| {
            let right_cell = &frame.cells[(row + 1) * usize::from(frame.area.width) - 1];
            if right_cell.symbol == "│" && text.ends_with('│') {
                text.len() - '│'.len_utf8()
            } else {
                text.len()
            }
        });
        oracle.scan_text_with_frame_edge(
            path,
            &format!("logical frame row {row}"),
            text,
            LeakScanMode::Renderer,
            right_content_edge,
        )?;
    }
    for cell in frame
        .symbols_omitted_from_readable_lines()
        .map_err(error_message)?
    {
        oracle.scan_text(
            path,
            &format!("omitted styled cell {},{}", cell.x, cell.y),
            &cell.symbol,
            LeakScanMode::Renderer,
        )?;
    }
    Ok(())
}

/// Check a complete checkpoint before the sink records any of its bytes.
pub(super) fn validate_checkpoint_leaks(
    sandbox: &SandboxMetadata,
    facts: &LeakOracleFacts,
    values: &[(&str, &Value)],
    frame: &StyledFrameSnapshot,
) -> Result<(), String> {
    let oracle = InstalledBundleLeakOracle::for_sandbox(sandbox, facts)?;
    for (channel, value) in values {
        oracle.scan_json(Path::new(channel), value, LeakScanMode::ZeroTolerance)?;
    }
    scan_frame(&oracle, Path::new("styled-frame"), frame)
}

fn masked_run_value(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    run: &RunMetadata,
) -> Result<Value, String> {
    let stored = artifact(files, Path::new(RUN_FILE))?;
    let mut value = bundle::decode_canonical_bytes(stored).map_err(error_message)?;
    let sandbox = value
        .get_mut("sandbox")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "typed run metadata has no sandbox object".to_owned())?;
    let root = sandbox
        .get("root")
        .and_then(Value::as_str)
        .ok_or_else(|| "typed run metadata has no sandbox root".to_owned())?;
    require(
        root == run.sandbox.root(),
        "run metadata sandbox root changed before leak validation",
    )?;
    sandbox.insert("root".to_owned(), Value::Null);
    let profiles = sandbox
        .get_mut("profiles")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "typed run metadata has no sandbox profiles".to_owned())?;
    require(
        profiles.len() == run.sandbox.profiles().len(),
        "run metadata sandbox profile count changed before leak validation",
    )?;
    for (profile, expected_root) in run.sandbox.profiles() {
        let stored_root = profiles
            .get(profile.as_str())
            .and_then(Value::as_str)
            .ok_or_else(|| "typed run metadata has no declared sandbox profile".to_owned())?;
        require(
            stored_root == expected_root,
            "run metadata profile root changed before leak validation",
        )?;
        profiles.insert(profile.as_str().to_owned(), Value::Null);
    }
    Ok(value)
}

/// Scan one file subset with one oracle and report the covered relative paths.
///
/// `trusted` names reviewer-authored local files. They count as scanned without a scan.
fn scan_bundle_subset(
    oracle: &InstalledBundleLeakOracle<'_>,
    files: &BTreeMap<PathBuf, Vec<u8>>,
    subset: &BTreeSet<PathBuf>,
    run: &RunMetadata,
    rows: &[TimelineRow],
    trusted: &BTreeSet<PathBuf>,
) -> Result<BTreeSet<PathBuf>, String> {
    let renderer_objects = rows
        .iter()
        .map(|row| bundle::object_path(&row.styled_frame))
        .collect::<BTreeSet<_>>();
    let mut scanned = BTreeSet::new();
    for path in subset {
        if scanned.contains(path) {
            continue;
        }
        let bytes = artifact(files, path)?;
        if trusted.contains(path) {
            // The reviewer owns this file. Its content never comes from the walker.
            scanned.insert(path.clone());
        } else if path == Path::new(RUN_FILE) {
            let value = masked_run_value(files, run)?;
            oracle.scan_json(path, &value, LeakScanMode::ZeroTolerance)?;
            scanned.insert(path.clone());
        } else if renderer_objects.contains(path) {
            let reference = scan_styled_frame(oracle, path, bytes)?;
            scanned.insert(path.clone());
            scanned.insert(bundle::object_view_path(&reference));
        } else if path.file_name().and_then(|name| name.to_str()) == Some(TIMELINE_FILE) {
            scan_ndjson_bytes(oracle, path, bytes, LeakScanMode::ZeroTolerance)?;
            scanned.insert(path.clone());
        } else if run
            .profiles
            .iter()
            .any(|profile| path == &bundle::profile_directory(profile).join(CAST_FILE))
        {
            // The core reader proved that these bytes are an exact rebuild of the styled frames.
            // Scanning the logical frame rows avoids treating terminal control bytes as text.
            scanned.insert(path.clone());
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            scan_json_bytes(oracle, path, bytes, LeakScanMode::ZeroTolerance)?;
            scanned.insert(path.clone());
        } else {
            let text = std::str::from_utf8(bytes)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            oracle.scan_text(path, "text", text, LeakScanMode::ZeroTolerance)?;
            scanned.insert(path.clone());
        }
    }
    Ok(scanned)
}

fn validate_installed_bundle_leaks(
    layout: &BundleLayout,
    files: &BTreeMap<PathBuf, Vec<u8>>,
    run: &RunMetadata,
    rows: &[TimelineRow],
    facts: &LeakOracleFacts,
) -> Result<(), String> {
    let oracle = InstalledBundleLeakOracle::new(run, facts)?;
    let scanned = scan_bundle_subset(&oracle, files, &layout.files, run, rows, &BTreeSet::new())?;
    require_exact_scan_coverage(&layout.files, &scanned)
}

fn require_exact_scan_coverage(
    expected: &BTreeSet<PathBuf>,
    scanned: &BTreeSet<PathBuf>,
) -> Result<(), String> {
    if let Some(path) = expected.difference(scanned).next() {
        return Err(format!(
            "leak oracle skipped declared bundle file: {}",
            path.display()
        ));
    }
    if let Some(path) = scanned.difference(expected).next() {
        return Err(format!(
            "leak oracle covered an undeclared bundle file: {}",
            path.display()
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct ValidatedBundle {
    digest: String,
}

/// One validated profile tree with its stored documents.
#[derive(Debug)]
pub(super) struct ProfileReadback {
    pub(super) profile: ProfileManifest,
    profile_sha256: String,
    pub(super) rows: Vec<TimelineRow>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    objects: BTreeMap<(ObjectKind, String), Value>,
}

/// Validate every stored document of one profile against the shared root documents.
///
/// The caller declares the layout; this function never inspects it.
fn validate_profile_tree(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    expected: ProfileExpectation<'_>,
    run: &RunMetadata,
    operations: &[Value],
    final_liveness: &Value,
) -> Result<ProfileReadback, String> {
    let profile_path = bundle::profile_directory(expected.id).join(PROFILE_FILE);
    let profile_bytes = artifact(files, &profile_path)?;
    let profile: ProfileManifest = decode(profile_bytes)?;
    bundle::validate_profile_manifest(&profile, expected).map_err(error_message)?;
    require(
        profile.operations_sha256 == run.operations_sha256,
        "profile operations digest does not match run metadata",
    )?;

    let timeline_path = bundle::profile_directory(expected.id).join(TIMELINE_FILE);
    let timeline_bytes = artifact(files, &timeline_path)?;
    let rows = bundle::decode_timeline_ndjson(timeline_bytes).map_err(error_message)?;
    require(
        bundle::timeline_ndjson_bytes(&rows).map_err(error_message)? == timeline_bytes,
        "typed timeline bytes do not match timeline.ndjson",
    )?;
    validate_timeline_semantics(&rows, operations, final_liveness, &profile.initial_locale)
        .map_err(error_message)?;
    validate_live_locales(&rows, &profile.initial_locale)?;
    require(
        usize::try_from(profile.row_count).map_err(error_message)? == rows.len(),
        "profile row count does not match the timeline",
    )?;
    require(
        rows.first().is_some_and(|first| {
            first.profile == profile.id && first.viewport == profile.initial_viewport
        }),
        "profile metadata does not match the first timeline row",
    )?;

    validate_chunks_and_views(files, expected.id, &profile, &rows)?;
    let objects = read_objects(files, &rows)?;
    for row in &rows {
        let frame: StyledFrameSnapshot =
            serde_json::from_value(stored_object(&objects, &row.styled_frame)?.clone())
                .map_err(error_message)?;
        require(
            frame.area == row.viewport,
            "a styled frame does not match its timeline viewport",
        )?;
    }
    validate_cast(files, expected.id, &profile, &rows, &objects)?;
    validate_reducer_replay(&rows, &objects)?;
    validate_host_state(&rows, &objects)?;
    validate_effect_chain_termination(&rows)?;
    Ok(ProfileReadback {
        profile,
        profile_sha256: sha256_hex(profile_bytes),
        rows,
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        objects,
    })
}

fn read_bundle(
    root: &Path,
    expected: ProfileExpectation<'_>,
    facts: &LeakOracleFacts,
) -> Result<ValidatedBundle, String> {
    let (actual_layout, files) = read_tree(root)?;

    let run_bytes = artifact(&files, Path::new(RUN_FILE))?;
    let run = bundle::decode_run_metadata(run_bytes).map_err(error_message)?;
    require(
        run.profiles.as_slice() == std::slice::from_ref(expected.id),
        "run metadata does not name the expected profile",
    )?;

    let operations_bytes = artifact(&files, Path::new(OPERATIONS_FILE))?;
    let operations =
        value_array(bundle::decode_canonical_bytes(operations_bytes).map_err(error_message)?)?;
    require(
        operations.len() == run.operation_count,
        "run operation count does not match operations.json",
    )?;
    require(
        sha256_hex(operations_bytes) == run.operations_sha256,
        "run operation digest does not match operations.json",
    )?;

    let final_liveness_bytes = artifact(&files, Path::new(FINAL_LIVENESS_FILE))?;
    let final_liveness =
        bundle::decode_canonical_bytes(final_liveness_bytes).map_err(error_message)?;
    require(
        sha256_hex(final_liveness_bytes) == run.final_liveness_sha256,
        "run liveness digest does not match final-liveness.json",
    )?;

    let readback = validate_profile_tree(&files, expected, &run, &operations, &final_liveness)?;
    let profile = readback.profile;
    let rows = readback.rows;

    let mut expected_layout = BundleLayout::default();
    expected_layout
        .expect_bundle(&profile, &rows)
        .map_err(error_message)?;
    expected_layout
        .compare(&actual_layout)
        .map_err(error_message)?;

    let mut profile_digests = BTreeMap::new();
    profile_digests.insert(expected.id.clone(), readback.profile_sha256);
    let digest = bundle::bundle_digest(&profile_digests, &run.source_revision, &run.sandbox)
        .map_err(error_message)?;
    let expected_name = format!("bundle-{digest}");
    let directory_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if directory_name.starts_with("bundle-") {
        require(
            directory_name == expected_name,
            "bundle directory name does not match its contents",
        )?;
    }
    validate_installed_bundle_leaks(&expected_layout, &files, &run, &rows, facts)?;
    Ok(ValidatedBundle { digest })
}

/// One four-profile review corpus proved from its stored bytes alone.
#[derive(Debug)]
pub(super) struct ValidatedReviewCorpus {
    digest: String,
    pub(super) run: RunMetadata,
    manifest: CorpusManifest,
    pub(super) operations_bytes: Vec<u8>,
    pub(super) profiles: Vec<ProfileReadback>,
    pub(super) coverage: CoverageSummary,
    pub(super) profile_coverage: Vec<(SafeProfileId, CoverageSummary)>,
    pub(super) progress: ReviewProgress,
    layout: BundleLayout,
}

/// Validate one installed review corpus from the bytes under `root` and nothing else.
pub(super) fn read_review_corpus(root: &Path) -> Result<ValidatedReviewCorpus, String> {
    let (actual_layout, files) = read_tree(root)?;

    let run_bytes = artifact(&files, Path::new(RUN_FILE))?;
    let run = bundle::decode_run_metadata(run_bytes).map_err(error_message)?;
    bundle::validate_review_corpus_run(&run).map_err(error_message)?;

    let operations_bytes = artifact(&files, Path::new(OPERATIONS_FILE))?;
    let operations =
        value_array(bundle::decode_canonical_bytes(operations_bytes).map_err(error_message)?)?;
    require(
        operations.len() == run.operation_count,
        "run operation count does not match operations.json",
    )?;
    require(
        sha256_hex(operations_bytes) == run.operations_sha256,
        "run operation digest does not match operations.json",
    )?;

    let final_liveness_bytes = artifact(&files, Path::new(FINAL_LIVENESS_FILE))?;
    let final_liveness =
        bundle::decode_canonical_bytes(final_liveness_bytes).map_err(error_message)?;
    require(
        sha256_hex(final_liveness_bytes) == run.final_liveness_sha256,
        "run liveness digest does not match final-liveness.json",
    )?;

    let manifest =
        decode_manifest(artifact(&files, Path::new(MANIFEST_FILE))?).map_err(error_message)?;

    let mut profiles = Vec::new();
    let mut profile_coverage = Vec::new();
    let mut rows_by_profile = BTreeMap::new();
    let mut profile_digests = BTreeMap::new();
    for required in skit_tui_walker_support::required_review_profiles() {
        let id = SafeProfileId::try_from(required.id).map_err(error_message)?;
        let expected = ProfileExpectation {
            id: &id,
            locale: required.locale,
            viewport: required.viewport,
            operations_sha256: &run.operations_sha256,
        };
        let readback = validate_profile_tree(&files, expected, &run, &operations, &final_liveness)?;
        profile_coverage.push((
            id.clone(),
            measured_effect_coverage_from_timeline(&readback.rows)?,
        ));
        rows_by_profile.insert(id.clone(), readback.rows.clone());
        profile_digests.insert(id, readback.profile_sha256.clone());
        profiles.push(readback);
    }
    let stored_profiles = profiles
        .iter()
        .map(|readback| readback.profile.clone())
        .collect::<Vec<_>>();
    bundle::validate_review_corpus_manifest(&manifest, &run, &stored_profiles)
        .map_err(error_message)?;

    let coverage = bundle::decode_coverage_summary(artifact(&files, Path::new(COVERAGE_FILE))?)
        .map_err(error_message)?;
    validate_stored_effect_coverage_summary(&coverage, &profile_coverage)?;

    let progress = bundle::validate_review_files(
        &manifest,
        artifact(&files, Path::new(README_FILE))?,
        artifact(&files, Path::new(REVIEW_PROGRESS_FILE))?,
        artifact(&files, Path::new(REVIEW_FILE))?,
    )
    .map_err(error_message)?;

    let mut layout = BundleLayout::default();
    layout
        .expect_review_corpus(&manifest, &rows_by_profile)
        .map_err(error_message)?;
    layout.compare(&actual_layout).map_err(error_message)?;

    let digest = bundle::bundle_digest(&profile_digests, &run.source_revision, &run.sandbox)
        .map_err(error_message)?;
    let directory_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if directory_name.starts_with(bundle::REVIEW_CORPUS_DIRECTORY_PREFIX) {
        require(
            directory_name
                == bundle::review_corpus_directory_name(&digest).map_err(error_message)?,
            "review corpus directory name does not match its contents",
        )?;
    }
    Ok(ValidatedReviewCorpus {
        digest,
        run,
        manifest,
        operations_bytes: operations_bytes.to_vec(),
        profiles,
        coverage,
        profile_coverage,
        progress,
        layout,
    })
}

/// Report the reviewer files that no longer hold the bytes the writer installed.
fn edited_reviewer_files(
    manifest: &CorpusManifest,
    files: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<BTreeSet<PathBuf>, String> {
    let mut edited = BTreeSet::new();
    if artifact(files, Path::new(REVIEW_PROGRESS_FILE))?
        != template_review_progress_bytes(manifest)?
    {
        edited.insert(PathBuf::from(REVIEW_PROGRESS_FILE));
    }
    if artifact(files, Path::new(REVIEW_FILE))? != REVIEW_TEMPLATE {
        edited.insert(PathBuf::from(REVIEW_FILE));
    }
    Ok(edited)
}

/// Scan one review corpus with the four generation-time fact sets.
fn validate_review_corpus_leaks(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    corpus: &ValidatedReviewCorpus,
    facts: &[&LeakOracleFacts],
) -> Result<(), String> {
    require(
        facts.len() == corpus.profiles.len(),
        "review corpus leak validation needs one fact set for each profile",
    )?;
    let trusted = edited_reviewer_files(&corpus.manifest, files)?;
    let root_files = corpus
        .layout
        .files
        .iter()
        .filter(|path| path.components().count() == 1)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut union = BTreeSet::new();
    for (readback, profile_facts) in corpus.profiles.iter().zip(facts) {
        let oracle = InstalledBundleLeakOracle::new(&corpus.run, profile_facts)?;
        let profile_root = bundle::profile_directory(&readback.profile.id);
        let mut subset = root_files.clone();
        for path in &corpus.layout.files {
            if path.starts_with(&profile_root) {
                subset.insert(path.clone());
            }
        }
        for row in &readback.rows {
            for reference in [
                &row.reducer,
                &row.host,
                &row.session,
                &row.styled_frame,
                &row.geometry,
            ] {
                subset.insert(bundle::object_path(reference));
                subset.insert(bundle::object_view_path(reference));
            }
        }
        let scanned = scan_bundle_subset(
            &oracle,
            files,
            &subset,
            &corpus.run,
            &readback.rows,
            &trusted,
        )?;
        union.extend(scanned);
    }
    require_exact_scan_coverage(&corpus.layout.files, &union)
}

pub(super) fn checkout_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "the skit-cli manifest is not inside the checkout".to_owned())
}

fn real_git(arguments: &[&str]) -> Result<Vec<u8>, String> {
    let checkout = checkout_root()?;
    let output = io(
        Command::new("git")
            .args(arguments)
            .current_dir(&checkout)
            .output(),
        &checkout,
    )?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(output.stdout)
}

/// Read one file of the checkout for the source fingerprint.
fn read_checkout_file(checkout: &Path, relative: &str) -> Result<Vec<u8>, String> {
    let path = checkout.join(relative);
    io(fs::read(&path), &path)
}

pub(super) fn source_identity() -> Result<String, String> {
    let checkout = checkout_root()?;
    let head = String::from_utf8(real_git(&["rev-parse", "HEAD"])?).map_err(error_message)?;
    let diff = real_git(&["diff", "--binary", "HEAD", "--", "."])?;
    let untracked = real_git(&["ls-files", "--others", "--exclude-standard", "-z"])?;
    let read = |relative: &str| read_checkout_file(&checkout, relative);
    bundle::source_revision(&head, &diff, &untracked, read).map_err(error_message)
}

fn install_bundle(
    parent: &Path,
    trace: &RecordedRealTrace,
    fingerprint: &mut impl FnMut() -> Result<String, String>,
) -> Result<PathBuf, String> {
    let revision = fingerprint()?;
    let staged = io(Builder::new().prefix(".walker-").tempdir_in(parent), parent)?;
    let writer = BundleWriter::create(staged.path(), &trace.review_profile)?;
    writer.write_trace(trace, &revision)?;
    let operations =
        canonical_json_bytes(&Value::Array(trace.operations.clone())).map_err(error_message)?;
    let operations_sha256 = sha256_hex(&operations);
    let expected = ProfileExpectation {
        id: &trace.review_profile,
        locale: &trace.locale,
        viewport: trace.viewport,
        operations_sha256: &operations_sha256,
    };
    let staged_bundle = read_bundle(staged.path(), expected, &trace.leak_oracle_facts)?;
    let (staged_layout, staged_files) = read_tree(staged.path())?;

    let completed_revision = fingerprint()?;
    require(
        completed_revision == revision,
        "the source tree changed during the walker bundle generation",
    )?;

    let destination = parent.join(format!("bundle-{}", staged_bundle.digest));

    if io(fs::rename(staged.path(), &destination), &destination).is_ok() {
        read_bundle(&destination, expected, &trace.leak_oracle_facts)?;
        let (installed_layout, installed_files) = read_tree(&destination)?;
        require(
            installed_layout == staged_layout && installed_files == staged_files,
            "the installed walker bundle differs from the validated staged bundle",
        )?;
        return Ok(destination);
    }

    let (installed_layout, installed_files) = read_tree(&destination)?;
    if staged_layout == installed_layout && staged_files == installed_files {
        read_bundle(&destination, expected, &trace.leak_oracle_facts)?;
        Ok(destination)
    } else {
        Err("the installed walker bundle differs from the staged bundle".to_owned())
    }
}

/// Create the private staging directory for one review corpus.
#[cfg(unix)]
fn stage_review_corpus_directory(parent: &Path) -> Result<tempfile::TempDir, String> {
    use std::os::unix::fs::PermissionsExt as _;

    io(
        Builder::new()
            .prefix(".walker-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(parent),
        parent,
    )
}

/// Create the private staging directory for one review corpus.
#[cfg(not(unix))]
fn stage_review_corpus_directory(parent: &Path) -> Result<tempfile::TempDir, String> {
    io(Builder::new().prefix(".walker-").tempdir_in(parent), parent)
}

/// Install or re-verify the four-profile review corpus under `parent`.
///
/// `parent` must be a gitignored location. `source_identity` hashes every untracked file.
fn install_review_corpus(
    parent: &Path,
    corpus: &RecordedRealCorpus,
    fingerprint: &mut impl FnMut() -> Result<String, String>,
) -> Result<PathBuf, String> {
    install_review_corpus_with(
        parent,
        corpus,
        fingerprint,
        |pinned, source, destination| pinned.rename_child_directory_noreplace(source, destination),
    )
}

/// Install or re-verify the four-profile review corpus with one explicit publish step.
///
/// `publish` is the filesystem boundary. It renames the staged directory to its digest name
/// without replacing any destination.
///
/// The rename is atomic, so a refusal before it leaves `parent` exactly as it was. A refusal
/// after a successful rename keeps the digest-named destination in place. The next run
/// validates or refuses it. Only tampering between the rename and the byte comparison reaches
/// that state.
///
/// A successful rename moves the validated staged tree. The installer then compares the
/// destination bytes with the staged bytes. It does not read the corpus again. The reader and
/// the leak scan are pure functions of the bytes that the staged tree already validated.
fn install_review_corpus_with(
    parent: &Path,
    corpus: &RecordedRealCorpus,
    fingerprint: &mut impl FnMut() -> Result<String, String>,
    publish: impl FnOnce(
        &PinnedDirectory,
        &ChildName,
        &ChildName,
    ) -> Result<PinnedDirectory, SandboxFsError>,
) -> Result<PathBuf, String> {
    let revision = fingerprint()?;
    let staged = stage_review_corpus_directory(parent)?;
    let writer = BundleWriter::create_common(staged.path())?;
    writer.write_review_corpus(corpus, &revision)?;

    let (staged_layout, staged_files) = read_tree(staged.path())?;
    let validated = read_review_corpus(staged.path())?;
    let facts = corpus
        .profiles
        .iter()
        .map(|recorded| &recorded.leak_oracle_facts)
        .collect::<Vec<_>>();
    validate_review_corpus_leaks(&staged_files, &validated, &facts)?;

    require(
        fingerprint()? == revision,
        "the source tree changed during the walker corpus generation",
    )?;

    let destination_name =
        bundle::review_corpus_directory_name(&validated.digest).map_err(error_message)?;
    let destination = parent.join(&destination_name);
    let source_name = staged
        .path()
        .file_name()
        .ok_or_else(|| "the staged walker corpus has no directory name".to_owned())?
        .to_owned();
    let pinned = PinnedDirectory::open_ambient_parent(parent).map_err(error_message)?;
    let source_child = ChildName::new(source_name).map_err(error_message)?;
    let destination_child = ChildName::new(destination_name).map_err(error_message)?;
    match publish(&pinned, &source_child, &destination_child) {
        Ok(installed) => {
            drop(installed);
            let (installed_layout, installed_files) = read_tree(&destination)?;
            require(
                installed_layout == staged_layout && installed_files == staged_files,
                "the installed review corpus differs from the validated staged corpus",
            )?;
            Ok(destination)
        }
        Err(error) => {
            let taken = error.destination_already_exists();
            require(
                taken,
                &format!("could not publish the walker review corpus: {error}"),
            )?;
            let existing = pinned
                .open_directory_shared(&destination_child)
                .map_err(|error| format!("could not open the installed review corpus: {error}"))?;
            drop(existing);
            let (_, installed_files) = read_tree(&destination)?;
            bundle::compare_regenerated_corpus(&staged_files, &installed_files)
                .map_err(error_message)?;
            let published = read_review_corpus(&destination)?;
            validate_review_corpus_leaks(&installed_files, &published, &facts)?;
            Ok(destination)
        }
    }
}

/// Record, install, replay, and reinstall one four-profile review corpus.
pub(super) fn generate_and_install_review_corpus(
    namespace: StableSandboxNamespace,
    parent: &Path,
    operations: &[CorpusOperation],
    fingerprint: &mut impl FnMut() -> Result<String, String>,
) -> Result<(PathBuf, StableCorpusTracePair), String> {
    generate_and_install_review_corpus_with(parent, fingerprint, |phase| {
        record_real_review_corpus_in(namespace.clone(), operations, phase)
    })
}

/// Record, install, replay, and reinstall one four-profile review corpus with one recorder.
///
/// `record` is the recording boundary. This function compares the main recording with the
/// replay before the replay installs. A different replay makes it refuse. It does not change
/// the installed corpus.
fn generate_and_install_review_corpus_with(
    parent: &Path,
    fingerprint: &mut impl FnMut() -> Result<String, String>,
    mut record: impl FnMut(StablePairPhase) -> Result<RecordedRealCorpus, String>,
) -> Result<(PathBuf, StableCorpusTracePair), String> {
    let revision = fingerprint()?;
    let mut unchanged_source = || {
        let current = fingerprint()?;
        require(
            current == revision,
            "the source tree changed during the walker corpus generation",
        )?;
        Ok(current)
    };
    let main = record(StablePairPhase::Main)?;
    let installed = install_review_corpus(parent, &main, &mut unchanged_source)?;
    let replay = record(StablePairPhase::Replay)?;
    let pair = StableCorpusTracePair::new(main, replay)?;
    let reinstalled = install_review_corpus(parent, &pair.replay, &mut unchanged_source)?;
    require(
        reinstalled == installed,
        "the replay review corpus installed a different directory",
    )?;
    Ok((installed, pair))
}

/// Validate one installed review corpus that a reviewer completed.
///
/// `expected_final` applies the final-corpus policy. `expected_revision` binds the source.
pub(super) fn validate_completed_review_corpus(
    root: &Path,
    expected_revision: Option<&str>,
    expected_final: bool,
) -> Result<ValidatedReviewCorpus, String> {
    let corpus = read_review_corpus(root)?;
    require(
        corpus.progress.complete,
        "the installed review corpus has an incomplete review",
    )?;
    if expected_final {
        validate_final_review_corpus(
            &corpus.operations_bytes,
            corpus.run.operation_count,
            &corpus.profile_coverage,
            &corpus.coverage,
        )?;
    }
    if let Some(revision) = expected_revision {
        require(
            corpus.run.source_revision == revision,
            "the installed review corpus has a different source revision",
        )?;
    }
    Ok(corpus)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn review_corpus_namespace(scratch: &Path) -> StableSandboxNamespace {
    StableSandboxNamespace::explicit(scratch.join(STABLE_SANDBOX_NAMESPACE))
        .expect("the explicit stable namespace is valid")
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn fixed_revision() -> impl FnMut() -> Result<String, String> {
    || Ok("revision".to_owned())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn record_small_review_corpus(scratch: &Path) -> RecordedRealCorpus {
    record_real_review_corpus_in(
        review_corpus_namespace(scratch),
        &review_corpus_contract_operations(),
        StablePairPhase::Main,
    )
    .unwrap()
}

pub(super) fn staged_residue(parent: &Path) -> Vec<PathBuf> {
    fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".walker-"))
        })
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn assert_reinstall_accepts(parent: &Path, installed: &Path, corpus: &RecordedRealCorpus) {
    let before = read_tree(installed).unwrap();
    let mut fingerprint = fixed_revision();
    let again = install_review_corpus(parent, corpus, &mut fingerprint).unwrap();
    assert_eq!(again.as_path(), installed);
    assert_eq!(read_tree(installed).unwrap(), before);
    assert!(staged_residue(parent).is_empty());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn assert_reinstall_refuses(
    parent: &Path,
    installed: &Path,
    corpus: &RecordedRealCorpus,
    message: &str,
) {
    let before = read_tree(installed).unwrap();
    let mut fingerprint = fixed_revision();
    let error = install_review_corpus(parent, corpus, &mut fingerprint).unwrap_err();
    assert_eq!(error, message);
    assert_eq!(read_tree(installed).unwrap(), before);
    assert!(staged_residue(parent).is_empty());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn raw_progress_bytes(progress: &ReviewProgress) -> Vec<u8> {
    let mut bytes = canonical_json_bytes(&serde_json::to_value(progress).unwrap()).unwrap();
    bytes.push(b'\n');
    bytes
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn edited_review_report(finding: &str) -> Vec<u8> {
    format!("# UI review\n\n## Findings\n\n{finding}\n\n## Review method\n\nRead every chunk.\n")
        .into_bytes()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn partial_review_progress(manifest: &CorpusManifest, report: &[u8]) -> ReviewProgress {
    let chunk = &manifest.profiles[0].chunks[0];
    ReviewProgress {
        manifest_sha256: manifest_digest(manifest).unwrap(),
        complete: false,
        review_sha256: Some(sha256_hex(report)),
        reviewed_chunks: BTreeMap::from([(chunk.id.clone(), chunk.sha256.clone())]),
        profile_verdicts: BTreeMap::new(),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn complete_review_progress(manifest: &CorpusManifest, report: &[u8]) -> ReviewProgress {
    let mut reviewed_chunks = BTreeMap::new();
    for profile in &manifest.profiles {
        for chunk in &profile.chunks {
            reviewed_chunks.insert(chunk.id.clone(), chunk.sha256.clone());
        }
    }
    ReviewProgress {
        manifest_sha256: manifest_digest(manifest).unwrap(),
        complete: true,
        review_sha256: Some(sha256_hex(report)),
        reviewed_chunks,
        profile_verdicts: manifest
            .profiles
            .iter()
            .map(|profile| (profile.id.clone(), ReviewVerdict::NoFindings))
            .collect(),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn write_reviewer_files(root: &Path, progress: &ReviewProgress, report: &[u8]) {
    fs::write(
        root.join(REVIEW_PROGRESS_FILE),
        raw_progress_bytes(progress),
    )
    .unwrap();
    fs::write(root.join(REVIEW_FILE), report).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn assert_reviewer_files_refuse(
    installed: &Path,
    progress: &ReviewProgress,
    report: &[u8],
    message: &str,
) {
    write_reviewer_files(installed, progress, report);
    let error = read_review_corpus(installed).unwrap_err();
    assert_eq!(error, message);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn install_completed_small_review_corpus(scratch: &Path, parent: &Path) -> PathBuf {
    let corpus = record_small_review_corpus(scratch);
    let mut fingerprint = fixed_revision();
    let installed = install_review_corpus(parent, &corpus, &mut fingerprint).unwrap();
    let manifest = read_review_corpus(&installed).unwrap().manifest;
    let report = edited_review_report("No finding.");
    write_reviewer_files(
        &installed,
        &complete_review_progress(&manifest, &report),
        &report,
    );
    installed
}

/// Install, replay, and roll back the small four-profile review corpus.
///
/// The recorder failure is proved on the first profile. Every profile records the same
/// operations, so no operation fails in a later profile alone. The successful recording
/// that follows proves that the failed profile released its lease.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn installs_replays_and_rolls_back_the_small_four_profile_review_corpus() {
    let scratch = tempfile::TempDir::new().unwrap();
    let namespace = review_corpus_namespace(scratch.path());
    let failure = record_real_review_corpus_in(
        namespace.clone(),
        &[invalid_corpus_operation()],
        StablePairPhase::Main,
    )
    .unwrap_err();
    assert!(failure.contains("en-80x24"), "{failure}");
    assert!(failure.contains("operation 1"), "{failure}");

    let parent = tempfile::TempDir::new().unwrap();
    let mut fingerprint = fixed_revision();
    let (installed, pair) = generate_and_install_review_corpus(
        namespace,
        parent.path(),
        &review_corpus_contract_operations(),
        &mut fingerprint,
    )
    .unwrap();
    assert!(staged_residue(parent.path()).is_empty());

    let corpus = read_review_corpus(&installed).unwrap();
    assert_eq!(
        installed.file_name().unwrap().to_str().unwrap(),
        bundle::review_corpus_directory_name(&corpus.digest).unwrap()
    );
    assert_eq!(
        corpus.run.operation_count,
        review_corpus_contract_operations().len()
    );
    assert_eq!(fs::read(installed.join(README_FILE)).unwrap(), REVIEW_GUIDE);
    assert_eq!(
        fs::read(installed.join(REVIEW_FILE)).unwrap(),
        REVIEW_TEMPLATE
    );
    assert_eq!(
        fs::read(installed.join(REVIEW_PROGRESS_FILE)).unwrap(),
        template_review_progress_bytes(&corpus.manifest).unwrap()
    );
    assert!(!corpus.progress.complete);
    assert_eq!(
        corpus
            .run
            .profiles
            .iter()
            .map(SafeProfileId::as_str)
            .collect::<Vec<_>>(),
        ["en-80x24", "pseudo-120x12", "zh-cn-120x30", "zh-tw-40x40"]
    );
    assert_eq!(
        corpus
            .manifest
            .profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<Vec<_>>(),
        ["en-80x24", "zh-cn-120x30", "zh-tw-40x40", "pseudo-120x12"]
    );

    let mut shared_objects = BTreeMap::new();
    for readback in &corpus.profiles {
        assert!(!readback.rows.iter().any(|row| matches!(
            &row.cause,
            TransitionCause::Session { handling, .. } if handling == "not_applicable"
        )));
        let coverage = measured_effect_coverage_from_timeline(&readback.rows).unwrap();
        assert!(
            !coverage
                .effects
                .served_requests
                .as_ref()
                .unwrap()
                .is_empty()
        );
        assert!(
            corpus
                .layout
                .files
                .contains(&bundle::profile_directory(&readback.profile.id).join(CAST_FILE))
        );
        for key in readback.objects.keys() {
            *shared_objects.entry(key.clone()).or_insert(0_usize) += 1;
        }
    }
    for relative in [
        OPERATIONS_FILE,
        FINAL_LIVENESS_FILE,
        RUN_FILE,
        MANIFEST_FILE,
        COVERAGE_FILE,
        REVIEW_PROGRESS_FILE,
        README_FILE,
        REVIEW_FILE,
    ] {
        assert!(corpus.layout.files.contains(Path::new(relative)));
    }
    assert!(
        shared_objects.values().any(|count| *count > 1),
        "the four profiles share no content object"
    );

    assert_eq!(pair.main.sandbox, pair.replay.sandbox);
    assert_eq!(pair.main.profiles[0].trace, pair.replay.profiles[0].trace);

    let mut diverging_profiles = pair.main.profiles.clone();
    let object = diverging_profiles[0].objects.keys().next().unwrap().clone();
    diverging_profiles[0].objects.insert(object, b"{}".to_vec());
    let diverging = RecordedRealCorpus::new(diverging_profiles).unwrap();
    let divergent_parent = tempfile::TempDir::new().unwrap();
    let mut fingerprint = fixed_revision();
    let error = generate_and_install_review_corpus_with(
        divergent_parent.path(),
        &mut fingerprint,
        |phase| {
            Ok(match phase {
                StablePairPhase::Main => pair.main.clone(),
                StablePairPhase::Replay => diverging.clone(),
            })
        },
    )
    .unwrap_err();
    assert_eq!(
        error,
        "stable corpus main and replay traces differ at en-80x24"
    );
    assert_eq!(
        fs::read_dir(divergent_parent.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        vec![installed.file_name().unwrap().to_owned()]
    );
    assert!(staged_residue(divergent_parent.path()).is_empty());

    let rollback = tempfile::TempDir::new().unwrap();
    let mut generation = 0_usize;
    let mut changing = || {
        generation += 1;
        Ok(format!("revision-{generation}"))
    };
    assert_eq!(
        install_review_corpus(rollback.path(), &pair.main, &mut changing).unwrap_err(),
        "the source tree changed during the walker corpus generation"
    );
    assert!(fs::read_dir(rollback.path()).unwrap().next().is_none());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn source_changes_during_recording_refuse_corpus_publication() {
    let scratch = tempfile::TempDir::new().unwrap();
    let corpus = record_small_review_corpus(scratch.path());
    for changed_phase in [StablePairPhase::Main, StablePairPhase::Replay] {
        let parent = tempfile::TempDir::new().unwrap();
        let revision = std::cell::Cell::new("before");
        let mut fingerprint = || Ok(revision.get().to_owned());
        let result =
            generate_and_install_review_corpus_with(parent.path(), &mut fingerprint, |phase| {
                if phase == changed_phase {
                    revision.set("after");
                }
                Ok(corpus.clone())
            });
        let error = result
            .map(drop)
            .expect_err("a changed source must refuse generation");
        assert_eq!(
            error,
            "the source tree changed during the walker corpus generation"
        );
        let installed = fs::read_dir(parent.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        if changed_phase == StablePairPhase::Main {
            assert!(installed.is_empty());
        } else {
            assert_eq!(installed.len(), 1);
            assert_eq!(
                read_review_corpus(&installed[0])
                    .unwrap()
                    .run
                    .source_revision,
                "before"
            );
        }
        assert!(staged_residue(parent.path()).is_empty());
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn review_corpus_reinstall_preserves_and_refuses_reviewer_files() {
    let scratch = tempfile::TempDir::new().unwrap();
    let corpus = record_small_review_corpus(scratch.path());
    let parent = tempfile::TempDir::new().unwrap();
    let mut fingerprint = fixed_revision();
    let installed = install_review_corpus(parent.path(), &corpus, &mut fingerprint).unwrap();
    let manifest = read_review_corpus(&installed).unwrap().manifest;
    assert_eq!(
        validate_completed_review_corpus(&installed, None, false).unwrap_err(),
        "the installed review corpus has an incomplete review"
    );

    let partial_report = edited_review_report("One partial finding.");
    write_reviewer_files(
        &installed,
        &partial_review_progress(&manifest, &partial_report),
        &partial_report,
    );
    assert_reinstall_accepts(parent.path(), &installed, &corpus);
    let preserved = read_review_corpus(&installed).unwrap();
    assert!(!preserved.progress.complete);
    assert_eq!(
        preserved.progress.review_sha256,
        Some(sha256_hex(&partial_report))
    );
    assert_eq!(
        fs::read(installed.join(REVIEW_FILE)).unwrap(),
        partial_report
    );

    let report = edited_review_report("No finding.");
    let complete = complete_review_progress(&manifest, &report);
    write_reviewer_files(&installed, &complete, &report);
    assert_reinstall_accepts(parent.path(), &installed, &corpus);
    let completed = validate_completed_review_corpus(&installed, Some("revision"), false).unwrap();
    assert!(completed.progress.complete);
    assert_eq!(completed.progress.profile_verdicts.len(), 4);
    assert_eq!(
        validate_completed_review_corpus(&installed, Some("other"), false).unwrap_err(),
        "the installed review corpus has a different source revision"
    );
    assert_eq!(
        validate_completed_review_corpus(&installed, None, true).unwrap_err(),
        "the review corpus does not have the canonical operation count"
    );

    let mut template_claim = complete.clone();
    template_claim.review_sha256 = Some(sha256_hex(REVIEW_TEMPLATE));
    write_reviewer_files(&installed, &template_claim, REVIEW_TEMPLATE);
    assert_reinstall_refuses(
        parent.path(),
        &installed,
        &corpus,
        "the UI review report is still the unedited template",
    );

    let mut false_claim = partial_review_progress(&manifest, &partial_report);
    false_claim.review_sha256 = Some(sha256_hex(b"other bytes"));
    assert_reviewer_files_refuse(
        &installed,
        &false_claim,
        &partial_report,
        "review progress claims a different report digest",
    );

    let mut stale_chunk = partial_review_progress(&manifest, &partial_report);
    stale_chunk
        .reviewed_chunks
        .values_mut()
        .for_each(|digest| *digest = "0".repeat(64));
    assert_reviewer_files_refuse(
        &installed,
        &stale_chunk,
        &partial_report,
        "progress has an unknown or stale chunk digest",
    );

    let mut unterminated = partial_report.clone();
    unterminated.pop();
    assert_reviewer_files_refuse(
        &installed,
        &partial_review_progress(&manifest, &unterminated),
        &unterminated,
        "the UI review report must have exactly one trailing newline",
    );

    write_reviewer_files(
        &installed,
        &partial_review_progress(&manifest, &partial_report),
        &partial_report,
    );
    let mut guide = REVIEW_GUIDE.to_vec();
    guide.extend_from_slice(b"reviewer note\n");
    fs::write(installed.join(README_FILE), &guide).unwrap();
    assert_reinstall_refuses(
        parent.path(),
        &installed,
        &corpus,
        "review corpus changed an immutable file: README.md",
    );
    fs::write(installed.join(README_FILE), REVIEW_GUIDE).unwrap();
    read_review_corpus(&installed).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn root_document_mutations(bytes: &[u8], trailing_newline: bool) -> Vec<Vec<u8>> {
    let body = if trailing_newline {
        &bytes[..bytes.len() - 1]
    } else {
        bytes
    };
    let value: Value = serde_json::from_slice(body).unwrap();
    let object = value.as_object().unwrap();
    let mut unknown = object.clone();
    unknown.insert("skit_unknown_field".to_owned(), json!(1));
    let mut missing = object.clone();
    missing.remove(&object.keys().next().unwrap().clone());
    let mut schema = object.clone();
    schema.insert("schema".to_owned(), json!(9));
    let mut noncanonical = body.to_vec();
    noncanonical.insert(1, b' ');
    let mut mutations = vec![
        canonical_json_bytes(&Value::Object(unknown)).unwrap(),
        canonical_json_bytes(&Value::Object(missing)).unwrap(),
        canonical_json_bytes(&Value::Object(schema)).unwrap(),
        b"[]".to_vec(),
        noncanonical,
    ];
    if trailing_newline {
        for mutation in &mut mutations {
            mutation.push(b'\n');
        }
    }
    mutations
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn inject_text(files: &mut BTreeMap<PathBuf, Vec<u8>>, relative: &Path, text: &str) {
    let mut bytes = files[relative].clone();
    bytes.extend_from_slice(format!("\n{text}\n").as_bytes());
    files.insert(relative.to_path_buf(), bytes);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn assert_read_back_refuses(installed: &Path, relative: &Path, bytes: &[u8], message: &str) {
    let path = installed.join(relative);
    let original = fs::read(&path).unwrap();
    fs::write(&path, bytes).unwrap();
    let refusal = read_review_corpus(installed);
    fs::write(&path, &original).unwrap();
    let error = refusal.err().unwrap_or_default();
    assert_eq!(error, message, "{}", path.display());
}

/// The exact serde refusal of one unknown `review-progress.json` field.
#[cfg(any(target_os = "linux", target_os = "windows"))]
const PROGRESS_UNKNOWN_FIELD: &str = concat!(
    "unknown field `skit_unknown_field`, expected one of `manifest_sha256`, ",
    "`complete`, `review_sha256`, `reviewed_chunks`, `profile_verdicts`"
);

/// The exact serde refusal of a `schema` field in `review-progress.json`.
#[cfg(any(target_os = "linux", target_os = "windows"))]
const PROGRESS_UNKNOWN_SCHEMA: &str = concat!(
    "unknown field `schema`, expected one of `manifest_sha256`, ",
    "`complete`, `review_sha256`, `reviewed_chunks`, `profile_verdicts`"
);

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn assert_root_document_shapes(
    installed: &Path,
    relative: &Path,
    trailing_newline: bool,
    messages: [&str; 5],
) {
    let original = fs::read(installed.join(relative)).unwrap();
    for (mutation, message) in root_document_mutations(&original, trailing_newline)
        .into_iter()
        .zip(messages)
    {
        assert_read_back_refuses(installed, relative, &mutation, message);
    }
    read_review_corpus(installed).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn review_corpus_refuses_every_destination_difference() {
    let scratch = tempfile::TempDir::new().unwrap();
    let corpus = record_small_review_corpus(scratch.path());
    let parent = tempfile::TempDir::new().unwrap();
    let mut fingerprint = fixed_revision();
    let installed = install_review_corpus(parent.path(), &corpus, &mut fingerprint).unwrap();
    let read = read_review_corpus(&installed).unwrap();

    let misnamed = parent
        .path()
        .join(bundle::review_corpus_directory_name(&"0".repeat(64)).unwrap());
    fs::rename(&installed, &misnamed).unwrap();
    let refusal = read_review_corpus(&misnamed);
    fs::rename(&misnamed, &installed).unwrap();
    assert_eq!(
        refusal.err().unwrap_or_default(),
        "review corpus directory name does not match its contents"
    );

    let object = bundle::object_path(&read.profiles[0].rows[0].reducer);
    let first = &read.manifest.profiles[0];
    let timeline = bundle::profile_directory(&first.id).join(TIMELINE_FILE);
    let profile_document = bundle::profile_directory(&first.id).join(PROFILE_FILE);

    let mut wider_profile = first.clone();
    wider_profile.maximum_chunk_rows = wider_profile.maximum_chunk_rows.saturating_add(1);
    assert_read_back_refuses(
        &installed,
        &profile_document,
        &canonical_json_bytes(&serde_json::to_value(&wider_profile).unwrap()).unwrap(),
        "stored profiles differ from the manifest",
    );

    let mut wider_manifest = read.manifest.clone();
    wider_manifest.profiles[0] = wider_profile;
    let progress_path = installed.join(REVIEW_PROGRESS_FILE);
    let stored_progress = fs::read(&progress_path).unwrap();
    let mut rebound = read.progress.clone();
    rebound.manifest_sha256 = manifest_digest(&wider_manifest).unwrap();
    fs::write(&progress_path, raw_progress_bytes(&rebound)).unwrap();
    assert_read_back_refuses(
        &installed,
        Path::new(MANIFEST_FILE),
        &canonical_json_bytes(&serde_json::to_value(&wider_manifest).unwrap()).unwrap(),
        "stored profiles differ from the manifest",
    );
    fs::write(&progress_path, &stored_progress).unwrap();

    assert_read_back_refuses(&installed, &timeline, b"{}\n", "missing field `schema`");

    let mut coverage = read.coverage.clone();
    *coverage
        .effects
        .served_requests
        .as_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap() += 1;
    assert_read_back_refuses(
        &installed,
        Path::new(COVERAGE_FILE),
        &bundle::coverage_summary_bytes(&coverage).unwrap(),
        "stored effect coverage does not match the profile timelines",
    );

    let mut random_run = read.run.clone();
    random_run.sandbox = SandboxMetadata::random(
        read.run.sandbox.platform(),
        read.run.sandbox.root().to_owned(),
        first.id.clone(),
    )
    .unwrap();
    random_run.profiles = vec![first.id.clone()];
    assert_read_back_refuses(
        &installed,
        Path::new(RUN_FILE),
        &bundle::run_metadata_bytes(&random_run).unwrap(),
        "review corpus needs a stable sandbox",
    );

    let mut misnamed_run = read.run.clone();
    let mut ids = read
        .manifest
        .profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<BTreeSet<_>>();
    ids.pop_last();
    ids.insert(SafeProfileId::try_from("other-profile").unwrap());
    misnamed_run.sandbox = SandboxMetadata::stable(
        read.run.sandbox.platform(),
        read.run.sandbox.root().to_owned(),
        ids.clone(),
    )
    .unwrap();
    misnamed_run.profiles = ids.into_iter().collect();
    assert_read_back_refuses(
        &installed,
        Path::new(RUN_FILE),
        &bundle::run_metadata_bytes(&misnamed_run).unwrap(),
        "review corpus run profiles are not the required profiles",
    );

    assert_root_document_shapes(
        &installed,
        Path::new(MANIFEST_FILE),
        false,
        [
            "manifest fields are not exact",
            "manifest fields are not exact",
            "manifest has an unsupported schema",
            "manifest fields are not exact",
            "JSON bytes are not canonical",
        ],
    );
    assert_root_document_shapes(
        &installed,
        Path::new(COVERAGE_FILE),
        false,
        [
            "unknown field `skit_unknown_field`, expected `schema` or `effects`",
            "coverage is missing effects",
            "coverage has an unsupported schema",
            "coverage is not an object",
            "JSON bytes are not canonical",
        ],
    );
    assert_root_document_shapes(
        &installed,
        Path::new(REVIEW_PROGRESS_FILE),
        true,
        [
            PROGRESS_UNKNOWN_FIELD,
            "missing field `complete`",
            PROGRESS_UNKNOWN_SCHEMA,
            "review progress is not an object",
            "JSON bytes are not canonical",
        ],
    );

    let object_path = installed.join(&object);
    let object_bytes = fs::read(&object_path).unwrap();
    fs::write(&object_path, b"{}").unwrap();
    assert_reinstall_refuses(
        parent.path(),
        &installed,
        &corpus,
        &format!(
            "review corpus changed an immutable file: {}",
            object.display()
        ),
    );
    fs::write(&object_path, &object_bytes).unwrap();

    let extra = installed.join("extra.json");
    fs::write(&extra, b"{}").unwrap();
    assert_reinstall_refuses(
        parent.path(),
        &installed,
        &corpus,
        "review corpus has an extra file: extra.json",
    );
    fs::remove_file(&extra).unwrap();

    let removed = installed.join(&timeline);
    let removed_bytes = fs::read(&removed).unwrap();
    fs::remove_file(&removed).unwrap();
    assert_reinstall_refuses(
        parent.path(),
        &installed,
        &corpus,
        &format!("review corpus is missing a file: {}", timeline.display()),
    );
    fs::write(&removed, &removed_bytes).unwrap();
    assert_reinstall_accepts(parent.path(), &installed, &corpus);

    let refused_publish = tempfile::TempDir::new().unwrap();
    let mut fingerprint = fixed_revision();
    let error = install_review_corpus_with(
        refused_publish.path(),
        &corpus,
        &mut fingerprint,
        |_, _, destination| {
            Err(SandboxFsError::Invalid {
                path: PathBuf::from(destination.as_os_str()),
                reason: "the contract refused the publish".to_owned(),
            })
        },
    )
    .unwrap_err();
    let name = bundle::review_corpus_directory_name(&read.digest).unwrap();
    assert_eq!(
        error,
        format!(
            "could not publish the walker review corpus: {}",
            SandboxFsError::Invalid {
                path: PathBuf::from(&name),
                reason: "the contract refused the publish".to_owned(),
            }
        )
    );
    assert!(
        fs::read_dir(refused_publish.path())
            .unwrap()
            .next()
            .is_none()
    );

    let tampered_parent = tempfile::TempDir::new().unwrap();
    let tampered_root = tampered_parent.path().to_path_buf();
    let mut fingerprint = fixed_revision();
    let error = install_review_corpus_with(
        &tampered_root,
        &corpus,
        &mut fingerprint,
        |pinned, source, destination| {
            let published = pinned.rename_child_directory_noreplace(source, destination)?;
            let guide = tampered_root
                .join(destination.as_os_str())
                .join(README_FILE);
            fs::write(&guide, b"tampered after the rename\n").unwrap();
            Ok(published)
        },
    )
    .unwrap_err();
    assert_eq!(
        error,
        "the installed review corpus differs from the validated staged corpus"
    );
    assert!(staged_residue(&tampered_root).is_empty());
    assert_eq!(
        fs::read(tampered_root.join(&name).join(README_FILE)).unwrap(),
        b"tampered after the rename\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn review_corpus_refuses_a_nonregular_entry_a_symlink_and_a_taken_name() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let scratch = tempfile::TempDir::new().unwrap();
    let corpus = record_small_review_corpus(scratch.path());
    let parent = tempfile::TempDir::new().unwrap();
    let mut fingerprint = fixed_revision();
    let installed = install_review_corpus(parent.path(), &corpus, &mut fingerprint).unwrap();

    let before = read_tree(&installed).unwrap();

    let fifo = installed.join("pipe");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .unwrap();
    let mut fingerprint = fixed_revision();
    let error = install_review_corpus(parent.path(), &corpus, &mut fingerprint).unwrap_err();
    assert_eq!(error, "walker bundle contains a non-regular entry: pipe");
    assert!(staged_residue(parent.path()).is_empty());
    fs::remove_file(&fifo).unwrap();
    assert_eq!(read_tree(&installed).unwrap(), before);

    let link = installed.join("link.json");
    symlink(installed.join(README_FILE), &link).unwrap();
    let mut fingerprint = fixed_revision();
    let error = install_review_corpus(parent.path(), &corpus, &mut fingerprint).unwrap_err();
    assert_eq!(error, "walker bundle contains a symlink: link.json");
    assert!(staged_residue(parent.path()).is_empty());
    fs::remove_file(&link).unwrap();
    assert_eq!(read_tree(&installed).unwrap(), before);
    read_review_corpus(&installed).unwrap();

    let mode = fs::metadata(&installed).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o700);
    fs::set_permissions(&installed, fs::Permissions::from_mode(0o755)).unwrap();
    assert_reinstall_accepts(parent.path(), &installed, &corpus);
    fs::set_permissions(&installed, fs::Permissions::from_mode(0o700)).unwrap();

    let taken_parent = tempfile::TempDir::new().unwrap();
    let taken = taken_parent.path().join(installed.file_name().unwrap());
    fs::write(&taken, b"not a review corpus").unwrap();
    let mut fingerprint = fixed_revision();
    let error = install_review_corpus(taken_parent.path(), &corpus, &mut fingerprint).unwrap_err();
    assert!(
        error.contains("could not open the installed review corpus"),
        "{error}"
    );
    assert!(staged_residue(taken_parent.path()).is_empty());
    assert_eq!(fs::read(&taken).unwrap(), b"not a review corpus");

    fs::remove_file(&taken).unwrap();
    symlink(&installed, &taken).unwrap();
    let mut fingerprint = fixed_revision();
    let error = install_review_corpus(taken_parent.path(), &corpus, &mut fingerprint).unwrap_err();
    assert!(
        error.contains("could not open the installed review corpus"),
        "{error}"
    );
    assert!(staged_residue(taken_parent.path()).is_empty());
    assert!(
        fs::symlink_metadata(&taken)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn review_corpus_coverage_and_leak_validation_are_exact() {
    let scratch = tempfile::TempDir::new().unwrap();
    let corpus = record_small_review_corpus(scratch.path());
    let parent = tempfile::TempDir::new().unwrap();
    let mut fingerprint = fixed_revision();
    let installed = install_review_corpus(parent.path(), &corpus, &mut fingerprint).unwrap();
    let read = read_review_corpus(&installed).unwrap();
    let facts = corpus
        .profiles
        .iter()
        .map(|recorded| &recorded.leak_oracle_facts)
        .collect::<Vec<_>>();
    let (_, files) = read_tree(&installed).unwrap();
    validate_review_corpus_leaks(&files, &read, &facts).unwrap();

    assert_eq!(
        validate_review_corpus_leaks(&files, &read, &facts[..3]).unwrap_err(),
        "review corpus leak validation needs one fact set for each profile"
    );

    let host_index = read.profiles[0]
        .rows
        .iter()
        .position(|row| matches!(row.cause, TransitionCause::Host { .. }))
        .expect("the small vector records one host request");
    let mut fewer = read.profile_coverage.clone();
    let mut rows = read.profiles[0].rows.clone();
    rows.remove(host_index);
    fewer[0].1 = measured_effect_coverage_from_timeline(&rows).unwrap();
    assert_eq!(
        validate_stored_effect_coverage_summary(&read.coverage, &fewer).unwrap_err(),
        "stored effect coverage does not match the profile timelines"
    );

    let mut renamed = read.profile_coverage.clone();
    let mut summary = measured_effect_coverage_from_timeline(&read.profiles[0].rows).unwrap();
    let counts = summary.effects.served_requests.as_mut().unwrap();
    let name = counts.keys().next().unwrap().clone();
    let count = counts.remove(&name).unwrap();
    counts.insert("reload".to_owned(), count);
    renamed[0].1 = summary;
    assert_eq!(
        validate_stored_effect_coverage_summary(&read.coverage, &renamed).unwrap_err(),
        "stored effect coverage does not match the profile timelines"
    );

    assert_eq!(
        validate_final_review_corpus(
            &read.operations_bytes,
            read.run.operation_count,
            &read.profile_coverage,
            &read.coverage,
        )
        .unwrap_err(),
        "the review corpus does not have the canonical operation count"
    );

    let first_view = bundle::chunk_view_path(
        &read.profiles[0].profile.id,
        &read.profiles[0].profile.chunks[0].id,
    );
    let second_view = bundle::chunk_view_path(
        &read.profiles[1].profile.id,
        &read.profiles[1].profile.chunks[0].id,
    );
    let first_root = read.run.sandbox.profiles()[&read.profiles[0].profile.id].clone();
    let mut shared_leak = files.clone();
    inject_text(&mut shared_leak, &second_view, &first_root);
    let error = validate_review_corpus_leaks(&shared_leak, &read, &facts).unwrap_err();
    assert!(error.contains("raw sandbox root"), "{error}");

    let first_id = read.profiles[0].profile.id.as_str().to_owned();
    let token = format!("artifact-of-{first_id}");
    let mut owned = corpus
        .profiles
        .iter()
        .map(|recorded| recorded.leak_oracle_facts.clone())
        .collect::<Vec<_>>();
    owned[0].artifacts.insert(
        format!("projected/{first_id}-artifact"),
        ArtifactLeakOracleFact {
            raw_path_spellings: BTreeSet::from([token.clone()]),
            source_identities: BTreeSet::new(),
            modified_values: BTreeSet::new(),
        },
    );
    let paired = owned.iter().collect::<Vec<_>>();
    let mut owned_leak = files.clone();
    inject_text(&mut owned_leak, &first_view, &token);
    let error = validate_review_corpus_leaks(&owned_leak, &read, &paired).unwrap_err();
    assert!(error.contains("raw artifact path"), "{error}");
    let mut other_leak = files.clone();
    inject_text(&mut other_leak, &second_view, &token);
    validate_review_corpus_leaks(&other_leak, &read, &paired).unwrap();

    let second_id = read.profiles[1].profile.id.as_str().to_owned();
    let draft_name = format!("draft-of-{second_id}.sh");
    let projected_path = format!("projected/{second_id}-draft.sh");
    owned[1].renderer_drafts.insert(
        projected_path.clone(),
        RendererDraftFact {
            raw_path: format!("/walker/{second_id}/{draft_name}"),
            projected_path,
            raw_kind_picker_basename: draft_name.clone(),
            raw_lossy_basename: draft_name.clone(),
            projected_basename: format!("{second_id}-draft.sh"),
            review_names: BTreeSet::new(),
        },
    );
    let paired = owned.iter().collect::<Vec<_>>();
    let mut owner_leak = files.clone();
    inject_text(&mut owner_leak, &second_view, &draft_name);
    let error = validate_review_corpus_leaks(&owner_leak, &read, &paired).unwrap_err();
    assert!(error.contains("raw draft basename"), "{error}");
    let mut foreign_leak = files.clone();
    inject_text(&mut foreign_leak, &first_view, &draft_name);
    validate_review_corpus_leaks(&foreign_leak, &read, &paired).unwrap();

    let leaking_report = format!(
        "# UI review\n\n## Findings\n\n{first_root}\n\n## Review method\n\nRead every chunk.\n"
    )
    .into_bytes();
    write_reviewer_files(
        &installed,
        &partial_review_progress(&read.manifest, &leaking_report),
        &leaking_report,
    );
    assert_reinstall_accepts(parent.path(), &installed, &corpus);
    let edited = read_review_corpus(&installed).unwrap();
    let (_, edited_files) = read_tree(&installed).unwrap();
    validate_review_corpus_leaks(&edited_files, &edited, &facts).unwrap();
    assert_eq!(
        edited_reviewer_files(&edited.manifest, &edited_files).unwrap(),
        BTreeSet::from([
            PathBuf::from(REVIEW_FILE),
            PathBuf::from(REVIEW_PROGRESS_FILE)
        ])
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn validates_one_completed_review_corpus_named_by_the_environment() {
    let scratch = tempfile::TempDir::new().unwrap();
    let parent = tempfile::TempDir::new().unwrap();
    let expected_final = std::env::var_os("SKIT_WALKER_REVIEW").is_some();
    let root = std::env::var_os("SKIT_WALKER_REVIEW")
        .map(PathBuf::from)
        .unwrap_or_else(|| install_completed_small_review_corpus(scratch.path(), parent.path()));
    let expected_revision = std::env::var("SKIT_WALKER_EXPECT_REVISION").ok();
    let corpus =
        validate_completed_review_corpus(&root, expected_revision.as_deref(), expected_final)
            .unwrap();
    assert!(corpus.progress.complete);
    assert_eq!(corpus.profiles.len(), 4);
}

#[test]
fn installs_real_smoke_bundle_and_matches_exact_layout() {
    let trace = record_real_smoke_main().unwrap();
    let parent = tempfile::TempDir::new().unwrap();
    let mut fingerprint = || Ok("revision".to_owned());

    let installed = install_bundle(parent.path(), &trace, &mut fingerprint).unwrap();
    assert!(
        installed
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("bundle-")
    );

    let (actual, files) = read_tree(&installed).unwrap();
    let profile_path = bundle::profile_directory(&trace.review_profile).join(PROFILE_FILE);
    let profile: ProfileManifest = serde_json::from_slice(&files[&profile_path]).unwrap();
    let mut expected = BundleLayout::default();
    expected.expect_bundle(&profile, &trace.rows).unwrap();
    assert_eq!(actual, expected);
    let run = bundle::decode_run_metadata(&files[Path::new(RUN_FILE)]).unwrap();
    assert_eq!(run.schema, 2);
    assert_eq!(run.sandbox, trace.sandbox);
    assert_eq!(run.sandbox.mode(), SandboxMode::Random);
    assert_eq!(
        run.profiles.as_slice(),
        std::slice::from_ref(&trace.review_profile)
    );
}

#[test]
fn random_main_and_replay_keep_equal_traces_with_truthful_independent_metadata() {
    let main = record_real_smoke_main().unwrap();
    let replay = record_real_smoke_replay().unwrap();

    assert_eq!(main.trace, replay.trace);
    for recorded in [&main, &replay] {
        assert_eq!(recorded.sandbox.mode(), SandboxMode::Random);
        assert_eq!(
            recorded
                .sandbox
                .profiles()
                .get(&recorded.review_profile)
                .map(String::as_str),
            Some(recorded.sandbox.root())
        );
    }
}

// Keep the stable smoke path compiled on hosts that cannot run its sandbox.
#[test]
#[cfg_attr(
    not(any(target_os = "linux", target_os = "windows")),
    ignore = "the stable sandbox supports Linux and Windows"
)]
fn stable_main_and_replay_bundles_are_byte_identical() {
    let sandbox_parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(sandbox_parent.path().join(STABLE_SANDBOX_NAMESPACE))
            .unwrap();
    let pair = record_real_smoke_stable_pair_in(namespace).unwrap();
    let main_parent = tempfile::TempDir::new().unwrap();
    let replay_parent = tempfile::TempDir::new().unwrap();
    let mut main_fingerprint = || Ok("revision".to_owned());
    let mut replay_fingerprint = || Ok("revision".to_owned());

    let main = install_bundle(main_parent.path(), &pair.main, &mut main_fingerprint).unwrap();
    let replay =
        install_bundle(replay_parent.path(), &pair.replay, &mut replay_fingerprint).unwrap();

    assert_eq!(read_tree(&main).unwrap(), read_tree(&replay).unwrap());
}

fn expectation<'a>(
    trace: &'a RecordedRealTrace,
    profile: &'a ProfileManifest,
) -> ProfileExpectation<'a> {
    ProfileExpectation {
        id: &trace.review_profile,
        locale: &trace.locale,
        viewport: trace.viewport,
        operations_sha256: &profile.operations_sha256,
    }
}

fn write_canonical(path: &Path, value: &impl Serialize) {
    let value = serde_json::to_value(value).unwrap();
    let bytes = canonical_json_bytes(&value).unwrap();
    io(fs::write(path, bytes), path).unwrap();
}

fn read_canonical<T: DeserializeOwned + Serialize>(path: &Path) -> T {
    let bytes = io(fs::read(path), path).unwrap();
    decode(&bytes).unwrap()
}

fn read_run_metadata(path: &Path) -> RunMetadata {
    let bytes = io(fs::read(path), path).unwrap();
    bundle::decode_run_metadata(&bytes).unwrap()
}

fn write_run_metadata(path: &Path, run: &RunMetadata) {
    let bytes = bundle::run_metadata_bytes(run).unwrap();
    io(fs::write(path, bytes), path).unwrap();
}

fn write_trace_at(root: &Path, trace: &RecordedRealTrace) -> ProfileManifest {
    BundleWriter::create(root, &trace.review_profile)
        .unwrap()
        .write_trace(trace, "revision")
        .unwrap()
}

fn stage_valid_at(root: &Path, trace: &RecordedRealTrace) -> ProfileManifest {
    let profile = write_trace_at(root, trace);
    read_bundle(root, expectation(trace, &profile), &trace.leak_oracle_facts).unwrap();
    profile
}

fn profile_path(root: &Path, trace: &RealTrace) -> PathBuf {
    root.join(bundle::profile_directory(&trace.review_profile))
        .join(PROFILE_FILE)
}

fn timeline_path(root: &Path, trace: &RealTrace) -> PathBuf {
    root.join(bundle::profile_directory(&trace.review_profile))
        .join(TIMELINE_FILE)
}

fn cast_path(root: &Path, trace: &RealTrace) -> PathBuf {
    root.join(bundle::profile_directory(&trace.review_profile))
        .join(CAST_FILE)
}

fn reseal_rows(rows: &mut [TimelineRow]) {
    rows[0].previous_row_sha256 = None;
    for index in 1..rows.len() {
        rows[index].previous_row_sha256 = Some(timeline_row_digest(&rows[index - 1]).unwrap());
    }
}

fn rewrite_timeline_and_chunks(
    root: &Path,
    profile_id: &SafeProfileId,
    profile: &mut ProfileManifest,
    rows: &[TimelineRow],
) {
    let path = root
        .join(bundle::profile_directory(profile_id))
        .join(TIMELINE_FILE);
    io(
        fs::write(&path, bundle::timeline_ndjson_bytes(rows).unwrap()),
        &path,
    )
    .unwrap();
    profile.row_count = u32::try_from(rows.len()).unwrap();
    profile.chunks = build_chunks(&profile.id, rows, MAXIMUM_CHUNK_ROWS).unwrap();
    let path = root
        .join(bundle::profile_directory(profile_id))
        .join(PROFILE_FILE);
    write_canonical(&path, profile);

    let mut cursor = ChunkViewCursor::new();
    for chunk in &profile.chunks {
        let start = usize::try_from(chunk.start_sequence).unwrap();
        let end = usize::try_from(chunk.end_sequence).unwrap();
        let path = root.join(bundle::chunk_json_path(profile_id, &chunk.id));
        io(
            fs::write(&path, bundle::chunk_json_bytes(&rows[start..end]).unwrap()),
            &path,
        )
        .unwrap();
        let path = root.join(bundle::chunk_view_path(profile_id, &chunk.id));
        io(
            fs::write(&path, cursor.chunk_view_bytes(rows, chunk).unwrap()),
            &path,
        )
        .unwrap();
    }
}

fn assert_dynamic_locale_corruption(
    parent: &Path,
    name: &str,
    original: &RecordedRealTrace,
    mutate: impl FnOnce(&mut [TimelineRow], usize),
) -> String {
    let root = parent.join(name);
    let mut profile = stage_valid_at(&root, original);
    let mut rows = original.rows.clone();
    let saved = rows
        .iter()
        .position(|row| {
            matches!(
                &row.cause,
                TransitionCause::Host { response, .. }
                    if response.get("preferences_saved").is_some()
            )
        })
        .unwrap();
    mutate(&mut rows, saved);
    reseal_rows(&mut rows);
    rewrite_timeline_and_chunks(&root, &original.review_profile, &mut profile, &rows);
    assert_bundle_refuses(&root, original, &profile)
}

#[test]
fn timeline_rewrite_resyncs_the_profile_row_count_only() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let root = parent.path().join("row-count");
    let mut profile = stage_valid_at(&root, &trace);
    let rows = &trace.rows[..trace.rows.len() - 1];
    let expected_row_count = u32::try_from(rows.len()).unwrap();
    let cast_sha256 = profile.cast_sha256.clone();
    let cast_byte_size = profile.cast_byte_size;

    rewrite_timeline_and_chunks(&root, &trace.review_profile, &mut profile, rows);

    assert_eq!(profile.row_count, expected_row_count);
    assert_eq!(profile.cast_sha256, cast_sha256);
    assert_eq!(profile.cast_byte_size, cast_byte_size);
    let stored: ProfileManifest = read_canonical(&profile_path(&root, &trace));
    assert_eq!(stored, profile);
}

fn insert_trace_object(trace: &mut RealTrace, kind: ObjectKind, value: Value) -> ObjectRef {
    let (reference, bytes) = build_object(kind, &value).unwrap();
    trace.objects.insert(object_key(&reference), bytes);
    reference
}

fn rebuild_trace_cast(trace: &mut RealTrace) {
    trace.cast = bundle::rebuild_presented_cast(&trace.rows, |reference| {
        let bytes = trace
            .objects
            .get(&object_key(reference))
            .ok_or_else(|| "test frame object is absent".to_owned())?;
        let value = validate_object(reference, bytes).map_err(error_message)?;
        serde_json::from_value(value).map_err(error_message)
    })
    .unwrap();
}

fn assert_bundle_refuses(
    root: &Path,
    trace: &RecordedRealTrace,
    profile: &ProfileManifest,
) -> String {
    read_bundle(root, expectation(trace, profile), &trace.leak_oracle_facts).unwrap_err()
}

#[test]
fn repeated_objects_are_written_once() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let root = parent.path().join("deduplicated");
    let profile = stage_valid_at(&root, &trace);
    let (layout, _) = read_tree(&root).unwrap();

    let references = trace
        .rows
        .iter()
        .flat_map(|row| {
            [
                &row.reducer,
                &row.host,
                &row.session,
                &row.styled_frame,
                &row.geometry,
            ]
        })
        .collect::<Vec<_>>();
    let unique = references
        .iter()
        .map(|reference| object_key(reference))
        .collect::<BTreeSet<_>>();
    assert!(unique.len() < references.len());
    assert_eq!(
        layout
            .files
            .iter()
            .filter(|path| path.starts_with(OBJECT_DIRECTORY))
            .count(),
        unique.len()
    );
    assert_eq!(profile.row_count as usize, trace.rows.len());
}

#[test]
fn object_frame_view_and_generic_collisions_refuse_different_bytes() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();

    let object_root = parent.path().join("object");
    stage_valid_at(&object_root, &trace);
    let reducer = &trace.rows[0].reducer;
    let object_path = object_root.join(bundle::object_path(reducer));
    io(fs::write(&object_path, b"different"), &object_path).unwrap();
    let writer = BundleWriter {
        root: object_root.clone(),
    };
    assert!(
        writer
            .put_object(reducer, &trace.objects[&object_key(reducer)])
            .unwrap_err()
            .contains("collision")
    );

    let frame_root = parent.path().join("frame-view");
    stage_valid_at(&frame_root, &trace);
    let frame = &trace.rows[0].styled_frame;
    let frame_path = frame_root.join(bundle::object_view_path(frame));
    io(fs::write(&frame_path, b"different"), &frame_path).unwrap();
    let writer = BundleWriter {
        root: frame_root.clone(),
    };
    assert!(
        writer
            .put_object(frame, &trace.objects[&object_key(frame)])
            .unwrap_err()
            .contains("collision")
    );

    let generic_root = parent.path().join("generic");
    stage_valid_at(&generic_root, &trace);
    let writer = BundleWriter { root: generic_root };
    let run = io(
        fs::read(writer.root.join(RUN_FILE)),
        &writer.root.join(RUN_FILE),
    )
    .unwrap();
    writer.write_same_or_new(Path::new(RUN_FILE), &run).unwrap();
    assert!(
        writer
            .write_same_or_new(Path::new(RUN_FILE), b"different")
            .unwrap_err()
            .contains("collision")
    );
}

#[test]
fn writer_refuses_a_trace_profile_not_owned_by_its_sandbox_metadata() {
    let parent = tempfile::TempDir::new().unwrap();
    let mut trace = record_real_smoke_main().unwrap();
    trace.sandbox = SandboxMetadata::random(
        trace.sandbox.platform(),
        trace.sandbox.root().to_owned(),
        SafeProfileId::try_from("other-profile").unwrap(),
    )
    .unwrap();
    let writer = BundleWriter::create(parent.path(), &trace.review_profile).unwrap();

    let error = writer.write_trace(&trace, "revision").unwrap_err();

    assert!(error.contains("trace profile does not match"));
}

#[test]
fn read_back_refuses_root_document_corruption() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();

    let root = parent.path().join("run-canonical");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(RUN_FILE);
    let mut bytes = io(fs::read(&path), &path).unwrap();
    bytes.push(b'\n');
    io(fs::write(&path, bytes), &path).unwrap();
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    let root = parent.path().join("run-schema");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(RUN_FILE);
    let run: Value = read_canonical(&path);
    for schema in [0, 1, 3] {
        let mut wrong_schema = run.clone();
        wrong_schema["schema"] = json!(schema);
        write_canonical(&path, &wrong_schema);
        assert!(assert_bundle_refuses(&root, &trace, &profile).contains("unsupported schema"));
    }

    let root = parent.path().join("operations-shape");
    let profile = stage_valid_at(&root, &trace);
    write_canonical(&root.join(OPERATIONS_FILE), &json!({"not": "an array"}));
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("operation array"));

    let root = parent.path().join("operation-count");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(RUN_FILE);
    let mut run = read_run_metadata(&path);
    run.operation_count += 1;
    write_run_metadata(&path, &run);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("operation count"));

    let root = parent.path().join("operation-digest");
    let profile = stage_valid_at(&root, &trace);
    write_canonical(
        &root.join(OPERATIONS_FILE),
        &vec![json!({"operation": "other"})],
    );
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("operation digest"));

    let root = parent.path().join("liveness-digest");
    let profile = stage_valid_at(&root, &trace);
    write_canonical(
        &root.join(FINAL_LIVENESS_FILE),
        &json!({"synthetic": "other"}),
    );
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("liveness digest"));

    let root = parent.path().join("run-unknown-field");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(RUN_FILE);
    let mut run: Value = read_canonical(&path);
    run["unknown"] = json!(true);
    write_canonical(&path, &run);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("unknown field"));

    let root = parent.path().join("operations-canonical");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(OPERATIONS_FILE);
    let mut bytes = vec![b' '];
    bytes.extend(io(fs::read(&path), &path).unwrap());
    io(fs::write(&path, bytes), &path).unwrap();
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    let root = parent.path().join("liveness-canonical");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(FINAL_LIVENESS_FILE);
    let mut bytes = vec![b' '];
    bytes.extend(io(fs::read(&path), &path).unwrap());
    io(fs::write(&path, bytes), &path).unwrap();
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());
}

#[test]
fn read_back_refuses_missing_unknown_and_mismatched_sandbox_metadata() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();

    let root = parent.path().join("missing-sandbox");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(RUN_FILE);
    let mut run: Value = read_canonical(&path);
    run.as_object_mut().unwrap().remove("sandbox");
    write_canonical(&path, &run);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("sandbox"));

    let root = parent.path().join("unknown-sandbox-field");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(RUN_FILE);
    let mut run: Value = read_canonical(&path);
    run["sandbox"]["unknown"] = json!(true);
    write_canonical(&path, &run);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("unknown field"));

    let root = parent.path().join("profile-map-mismatch");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(RUN_FILE);
    let mut run: Value = read_canonical(&path);
    run["profiles"] = json!(["other-profile"]);
    write_canonical(&path, &run);
    assert!(
        assert_bundle_refuses(&root, &trace, &profile)
            .contains("profiles do not match sandbox metadata")
    );

    let root = parent.path().join("expected-profile-mismatch");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(RUN_FILE);
    let mut run = read_run_metadata(&path);
    let other = SafeProfileId::try_from("other-profile").unwrap();
    run.sandbox = SandboxMetadata::random(
        trace.sandbox.platform(),
        trace.sandbox.root().to_owned(),
        other.clone(),
    )
    .unwrap();
    run.profiles = vec![other];
    write_run_metadata(&path, &run);
    assert!(
        assert_bundle_refuses(&root, &trace, &profile)
            .contains("does not name the expected profile")
    );
}

#[test]
fn read_back_refuses_disconnected_profile_operations_digest() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let root = parent.path().join("operations-digest-chain");
    let mut profile = stage_valid_at(&root, &trace);

    let operations_path = root.join(OPERATIONS_FILE);
    let mut operations: Vec<Value> = read_canonical(&operations_path);
    operations[0]["contract_test"] = json!(true);
    write_canonical(&operations_path, &operations);
    let operations_bytes = io(fs::read(&operations_path), &operations_path).unwrap();

    let run_path = root.join(RUN_FILE);
    let mut run = read_run_metadata(&run_path);
    run.operations_sha256 = sha256_hex(&operations_bytes);
    write_run_metadata(&run_path, &run);

    let mut rows = trace.rows.clone();
    for row in &mut rows {
        if row.operation_index == Some(0)
            && let TransitionCause::Reducer { requested, .. } = &mut row.cause
        {
            *requested = operations[0].clone();
        }
    }
    reseal_rows(&mut rows);
    rewrite_timeline_and_chunks(&root, &trace.review_profile, &mut profile, &rows);

    let error = assert_bundle_refuses(&root, &trace, &profile);
    assert!(error.contains("profile operations digest"), "{error}");
}

#[test]
fn read_back_refuses_profile_timeline_and_chunk_corruption() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();

    let root = parent.path().join("profile");
    let profile = stage_valid_at(&root, &trace);
    let path = profile_path(&root, &trace);
    let mut corrupt: ProfileManifest = read_canonical(&path);
    corrupt.cast_byte_size = 0;
    write_canonical(&path, &corrupt);
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    let root = parent.path().join("profile-canonical");
    let profile = stage_valid_at(&root, &trace);
    let path = profile_path(&root, &trace);
    let mut bytes = vec![b' '];
    bytes.extend(io(fs::read(&path), &path).unwrap());
    io(fs::write(&path, bytes), &path).unwrap();
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    let root = parent.path().join("profile-unknown-field");
    let profile = stage_valid_at(&root, &trace);
    let path = profile_path(&root, &trace);
    let mut value: Value = read_canonical(&path);
    value["unknown"] = json!(true);
    write_canonical(&path, &value);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("unknown field"));

    let root = parent.path().join("profile-row-count");
    let profile = stage_valid_at(&root, &trace);
    let path = profile_path(&root, &trace);
    let mut corrupt = profile.clone();
    corrupt.row_count += 1;
    corrupt.chunks[0].end_sequence += 1;
    corrupt.chunks[0].row_count += 1;
    write_canonical(&path, &corrupt);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("row count"));

    let root = parent.path().join("timeline-newline");
    let profile = stage_valid_at(&root, &trace);
    let path = timeline_path(&root, &trace);
    let mut bytes = io(fs::read(&path), &path).unwrap();
    bytes.pop();
    io(fs::write(&path, bytes), &path).unwrap();
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    let root = parent.path().join("timeline-omitted-null");
    let profile = stage_valid_at(&root, &trace);
    let path = timeline_path(&root, &trace);
    let bytes = io(fs::read(&path), &path).unwrap();
    let mut values = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    values[0].as_object_mut().unwrap().remove("event_chain");
    io(fs::write(&path, cast_bytes(&values)), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("timeline bytes"));

    let root = parent.path().join("timeline-semantics");
    let profile = stage_valid_at(&root, &trace);
    let path = timeline_path(&root, &trace);
    let mut rows = bundle::decode_timeline_ndjson(&io(fs::read(&path), &path).unwrap()).unwrap();
    if let TransitionCause::Reducer { requested, .. } = &mut rows[1].cause {
        *requested = json!({"operation": "other"});
    }
    reseal_rows(&mut rows);
    let bytes = bundle::timeline_ndjson_bytes(&rows).unwrap();
    io(fs::write(&path, bytes), &path).unwrap();
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    let root = parent.path().join("profile-first-row");
    let profile = stage_valid_at(&root, &trace);
    let path = timeline_path(&root, &trace);
    let mut rows = bundle::decode_timeline_ndjson(&io(fs::read(&path), &path).unwrap()).unwrap();
    rows[0].viewport.width -= 1;
    reseal_rows(&mut rows);
    let bytes = bundle::timeline_ndjson_bytes(&rows).unwrap();
    io(fs::write(&path, bytes), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("profile metadata"));

    let root = parent.path().join("frame-viewport");
    let mut profile = stage_valid_at(&root, &trace);
    let mut rows = trace.rows.clone();
    rows[1].viewport.width -= 1;
    reseal_rows(&mut rows);
    rewrite_timeline_and_chunks(&root, &trace.review_profile, &mut profile, &rows);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("timeline viewport"));

    let root = parent.path().join("chunk-canonical");
    let profile = stage_valid_at(&root, &trace);
    let chunk = &profile.chunks[0];
    let path = root.join(bundle::chunk_json_path(&trace.review_profile, &chunk.id));
    let mut bytes = vec![b' '];
    bytes.extend(io(fs::read(&path), &path).unwrap());
    io(fs::write(&path, bytes), &path).unwrap();
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    let root = parent.path().join("chunk-descriptor");
    let profile = stage_valid_at(&root, &trace);
    let path = profile_path(&root, &trace);
    let mut corrupt = profile.clone();
    corrupt.chunks[0].first_row_sha256 = "0".repeat(64);
    write_canonical(&path, &corrupt);
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    let root = parent.path().join("chunk-json");
    let profile = stage_valid_at(&root, &trace);
    let chunk = &profile.chunks[0];
    let path = root.join(bundle::chunk_json_path(&trace.review_profile, &chunk.id));
    io(fs::write(&path, b"[]"), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("chunk digest"));

    let root = parent.path().join("chunk-view");
    let profile = stage_valid_at(&root, &trace);
    let chunk = &profile.chunks[0];
    let path = root.join(bundle::chunk_view_path(&trace.review_profile, &chunk.id));
    io(fs::write(&path, b"different"), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("chunk view"));
}

fn cast_values(bytes: &[u8]) -> Vec<Value> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

fn cast_bytes(values: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend(canonical_json_bytes(value).unwrap());
        bytes.push(b'\n');
    }
    bytes
}

fn update_profile_for_cast(root: &Path, trace: &RealTrace, bytes: &[u8], update_size: bool) {
    let path = profile_path(root, trace);
    let mut profile: ProfileManifest = read_canonical(&path);
    profile.cast_sha256 = sha256_hex(bytes);
    if update_size {
        profile.cast_byte_size = bytes.len() as u64;
    }
    write_canonical(&path, &profile);
}

#[test]
fn read_back_refuses_each_cast_corruption() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();

    let root = parent.path().join("invalid");
    let profile = stage_valid_at(&root, &trace);
    let path = cast_path(&root, &trace);
    io(fs::write(&path, b"invalid"), &path).unwrap();
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    let root = parent.path().join("digest");
    let profile = stage_valid_at(&root, &trace);
    let path = cast_path(&root, &trace);
    let mut values = cast_values(&io(fs::read(&path), &path).unwrap());
    values.push(json!([0.0, "o", ""]));
    io(fs::write(&path, cast_bytes(&values)), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("cast digest"));

    let root = parent.path().join("size");
    let profile = stage_valid_at(&root, &trace);
    let path = cast_path(&root, &trace);
    let mut values = cast_values(&io(fs::read(&path), &path).unwrap());
    values.push(json!([0.0, "o", ""]));
    let bytes = cast_bytes(&values);
    io(fs::write(&path, &bytes), &path).unwrap();
    update_profile_for_cast(&root, &trace, &bytes, false);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("byte size"));

    let root = parent.path().join("canvas");
    let profile = stage_valid_at(&root, &trace);
    let path = cast_path(&root, &trace);
    let mut values = cast_values(&io(fs::read(&path), &path).unwrap());
    values[0]["term"]["cols"] = json!(61);
    let bytes = cast_bytes(&values);
    io(fs::write(&path, &bytes), &path).unwrap();
    update_profile_for_cast(&root, &trace, &bytes, true);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("rebuild"));

    let root = parent.path().join("rebuild");
    let profile = stage_valid_at(&root, &trace);
    let path = cast_path(&root, &trace);
    let mut values = cast_values(&io(fs::read(&path), &path).unwrap());
    values[1][2] = json!("different");
    let bytes = cast_bytes(&values);
    io(fs::write(&path, &bytes), &path).unwrap();
    update_profile_for_cast(&root, &trace, &bytes, true);
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("rebuild"));
}

#[test]
fn read_back_refuses_untyped_styled_frame_member() {
    let parent = tempfile::TempDir::new().unwrap();
    let mut trace = record_real_smoke_main().unwrap();
    let original = trace.rows[0].styled_frame.clone();
    let mut frame = validate_object(&original, &trace.objects[&object_key(&original)]).unwrap();
    frame["contract_test"] = json!(true);
    let replacement = insert_trace_object(&mut trace, ObjectKind::StyledFrame, frame);
    for row in &mut trace.rows {
        if row.styled_frame == original {
            row.styled_frame = replacement.clone();
        }
    }
    trace.objects.remove(&object_key(&original)).unwrap();
    reseal_rows(&mut trace.rows);

    let root = parent.path().join("typed-frame");
    let profile = write_trace_at(&root, &trace);
    let error = assert_bundle_refuses(&root, &trace, &profile);
    assert!(error.contains("typed styled frame"), "{error}");
}

#[test]
fn read_back_refuses_object_and_view_corruption() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let reference = &trace.rows[0].reducer;

    let root = parent.path().join("object-canonical");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(bundle::object_path(reference));
    let mut bytes = io(fs::read(&path), &path).unwrap();
    bytes.insert(0, b' ');
    io(fs::write(&path, bytes), &path).unwrap();
    assert!(!assert_bundle_refuses(&root, &trace, &profile).is_empty());

    for (case, field, value, expected) in [
        ("object-schema", "schema", json!(2), "schema"),
        ("object-kind", "kind", json!("host"), "kind"),
        (
            "object-digest",
            "value",
            json!({"different": true}),
            "digest",
        ),
    ] {
        let root = parent.path().join(case);
        let profile = stage_valid_at(&root, &trace);
        let path = root.join(bundle::object_path(reference));
        let mut object: Value = read_canonical(&path);
        object[field] = value;
        write_canonical(&path, &object);
        assert!(assert_bundle_refuses(&root, &trace, &profile).contains(expected));
    }

    let root = parent.path().join("object-view");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(bundle::object_view_path(reference));
    io(fs::write(&path, b"different"), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("object view"));

    let root = parent.path().join("frame-view");
    let profile = stage_valid_at(&root, &trace);
    let frame = &trace.rows[0].styled_frame;
    let path = root.join(bundle::object_view_path(frame));
    io(fs::write(&path, b"different"), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("object view"));
}

#[test]
fn read_back_refuses_reducer_host_and_store_corruption() {
    let parent = tempfile::TempDir::new().unwrap();

    let mut reducer_trace = record_real_smoke_main().unwrap();
    reducer_trace.rows[2].reducer = reducer_trace.rows[0].reducer.clone();
    reseal_rows(&mut reducer_trace.rows);
    let root = parent.path().join("reducer");
    let profile = write_trace_at(&root, &reducer_trace);
    assert!(assert_bundle_refuses(&root, &reducer_trace, &profile).contains("production Action"));

    let mut host_trace = record_real_smoke_main().unwrap();
    let host_reference = host_trace.rows[0].host.clone();
    let host_bytes = host_trace.objects[&object_key(&host_reference)].clone();
    let mut host = validate_object(&host_reference, &host_bytes).unwrap();
    host["state"] = Value::Null;
    host_trace.rows[0].host = insert_trace_object(&mut host_trace, ObjectKind::Host, host);
    reseal_rows(&mut host_trace.rows);
    let root = parent.path().join("host-binding");
    let profile = write_trace_at(&root, &host_trace);
    assert!(assert_bundle_refuses(&root, &host_trace, &profile).contains("embed"));

    let mut store_trace = record_real_smoke_main().unwrap();
    let host_reference = store_trace.rows[1].host.clone();
    let host_bytes = store_trace.objects[&object_key(&host_reference)].clone();
    let mut host = validate_object(&host_reference, &host_bytes).unwrap();
    host["surface"] = json!({"different": true});
    store_trace.rows[1].host = insert_trace_object(&mut store_trace, ObjectKind::Host, host);
    reseal_rows(&mut store_trace.rows);
    let root = parent.path().join("store-quiet");
    let profile = write_trace_at(&root, &store_trace);
    assert!(assert_bundle_refuses(&root, &store_trace, &profile).contains("store-quiet"));
}

#[test]
fn read_back_refuses_unterminated_effect_and_live_locale_corruption() {
    let parent = tempfile::TempDir::new().unwrap();

    let original = record_real_smoke_main().unwrap();
    let mut effect_trace = original.clone();
    let previous_reducer = &original.rows[2].reducer;
    let previous_value = validate_object(
        previous_reducer,
        &original.objects[&object_key(previous_reducer)],
    )
    .unwrap();
    let mut state = parse_library_state(&previous_value).unwrap();
    let emitted = state.update(skit_ui::Action::OpenRun);
    assert_ne!(emitted, skit_ui::Effect::None);
    let reducer = insert_trace_object(
        &mut effect_trace,
        ObjectKind::Reducer,
        serde_json::to_value(&state).unwrap(),
    );
    let previous_host = &original.rows[2].host;
    let mut host =
        validate_object(previous_host, &original.objects[&object_key(previous_host)]).unwrap();
    host["state"] = serde_json::to_value(&state).unwrap();
    let host = insert_trace_object(&mut effect_trace, ObjectKind::Host, host);
    let mut final_row = original.rows[1].clone();
    final_row.sequence = 3;
    final_row.phase = TimelinePhase::FinalLiveness;
    final_row.event_chain = Some(EventChainIdentity {
        phase: TimelinePhase::FinalLiveness,
        sequence: 0,
    });
    final_row.operation_index = None;
    if let TransitionCause::Reducer {
        requested,
        resolved,
        event,
        emitted: recorded_emitted,
        ..
    } = &mut final_row.cause
    {
        *requested = original.final_liveness_requested.clone();
        *recorded_emitted = serde_json::to_value(emitted).unwrap();
        if let TransitionCause::Reducer {
            resolved: final_resolved,
            event: final_event,
            ..
        } = &original.rows[3].cause
        {
            *resolved = final_resolved.clone();
            *event = final_event.clone();
        }
    }
    final_row.reducer = reducer;
    final_row.host = host;
    final_row.liveness = original.rows[3].liveness.clone();
    effect_trace.rows = vec![
        original.rows[0].clone(),
        original.rows[1].clone(),
        original.rows[2].clone(),
        final_row,
    ];
    reseal_rows(&mut effect_trace.rows);
    rebuild_trace_cast(&mut effect_trace);
    let root = parent.path().join("effect");
    let profile = write_trace_at(&root, &effect_trace);
    assert!(assert_bundle_refuses(&root, &effect_trace, &profile).contains("pending"));

    let root = parent.path().join("locale");
    let profile = stage_valid_at(&root, &original);
    let path = timeline_path(&root, &original);
    let mut rows = bundle::decode_timeline_ndjson(&io(fs::read(&path), &path).unwrap()).unwrap();
    rows[1].locale = "zh-CN".to_owned();
    reseal_rows(&mut rows);
    io(
        fs::write(&path, bundle::timeline_ndjson_bytes(&rows).unwrap()),
        &path,
    )
    .unwrap();
    assert!(!assert_bundle_refuses(&root, &original, &profile).is_empty());
}

#[test]
fn read_back_refuses_nested_locale_lookalike() {
    let parent = tempfile::TempDir::new().unwrap();
    let mut trace = record_real_smoke_main().unwrap();
    let mut cause = serde_json::to_value(&trace.rows[2].cause).unwrap();
    cause
        .pointer_mut("/response/present/run")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("locale".to_owned(), json!("zh-CN"));
    trace.rows[2].cause = serde_json::from_value(cause).unwrap();
    trace.rows[2].locale = "zh-CN".to_owned();
    trace.rows[3].locale = "zh-CN".to_owned();
    reseal_rows(&mut trace.rows);

    let root = parent.path().join("nested-locale");
    let profile = write_trace_at(&root, &trace);
    let error = assert_bundle_refuses(&root, &trace, &profile);
    assert!(error.contains("live reducer response"), "{error}");
}

fn rows_with_preferences_saved(response_locale: &str, row_locale: &str) -> Vec<TimelineRow> {
    let mut rows = record_real_smoke_main().unwrap().rows.clone();
    let mut cause = serde_json::to_value(&rows[2].cause).unwrap();
    cause["response"] = serde_json::to_value(skit_ui::Action::PreferencesSaved {
        locale: response_locale.to_owned(),
        message: "saved".to_owned(),
    })
    .unwrap();
    rows[2].cause = serde_json::from_value(cause).unwrap();
    for row in &mut rows[2..] {
        row.locale = row_locale.to_owned();
    }
    rows
}

#[test]
fn live_locale_refuses_unlabelled_preferences_response() {
    let rows = rows_with_preferences_saved("zh-CN", "en");
    assert!(
        validate_live_locales(&rows, "en")
            .unwrap_err()
            .contains("live reducer response")
    );
}

#[test]
fn live_locale_accepts_each_canonical_saved_response() {
    for locale in ["en", "zh-CN", "zh-TW", "x-pseudo"] {
        let rows = rows_with_preferences_saved(locale, locale);
        assert_eq!(validate_live_locales(&rows, "en"), Ok(()), "{locale}");
    }
}

#[test]
fn live_locale_refuses_a_noncanonical_alias_in_the_saved_response() {
    let rows = rows_with_preferences_saved("zh", "zh-CN");
    assert_eq!(rows[0].locale, "en");
    assert_eq!(rows[1].locale, "en");
    assert!(
        validate_live_locales(&rows, "en")
            .unwrap_err()
            .contains("canonical")
    );
}

#[test]
fn live_locale_refuses_a_noncanonical_row_after_a_canonical_response() {
    let rows = rows_with_preferences_saved("zh-CN", "zh");
    assert!(
        validate_live_locales(&rows, "en")
            .unwrap_err()
            .contains("live reducer response")
    );
}

#[test]
fn live_locale_refuses_an_unsupported_saved_response_tag() {
    let rows = rows_with_preferences_saved("not-a-supported-locale", "en");
    assert!(
        validate_live_locales(&rows, "en")
            .unwrap_err()
            .contains("canonical")
    );
}

#[test]
fn live_locale_refuses_a_wrong_target_and_a_later_reversion() {
    let rows = rows_with_preferences_saved("zh-CN", "zh-TW");
    assert!(
        validate_live_locales(&rows, "en")
            .unwrap_err()
            .contains("live reducer response")
    );

    let mut rows = rows_with_preferences_saved("zh-CN", "zh-CN");
    rows[3].locale = "en".to_owned();
    assert!(
        validate_live_locales(&rows, "en")
            .unwrap_err()
            .contains("live reducer response")
    );
}

#[test]
fn read_back_refuses_every_dynamic_locale_corruption() {
    let parent = tempfile::TempDir::new().unwrap();
    let original = record_real_locale_smoke(LocaleSmokeTarget::TraditionalChinese).unwrap();

    let error = assert_dynamic_locale_corruption(
        parent.path(),
        "noncanonical-response",
        &original,
        |rows, saved| {
            let mut cause = serde_json::to_value(&rows[saved].cause).unwrap();
            cause["response"]["preferences_saved"]["locale"] = json!("zh_TW");
            rows[saved].cause = serde_json::from_value(cause).unwrap();
            for row in &mut rows[saved..] {
                row.locale = "zh_TW".to_owned();
            }
        },
    );
    assert!(error.contains("canonical"), "{error}");

    for (name, mutate) in [
        (
            "missing-switch",
            (|rows: &mut [TimelineRow], saved: usize| {
                rows[saved].locale = "en".to_owned();
            }) as fn(&mut [TimelineRow], usize),
        ),
        ("wrong-target", |rows: &mut [TimelineRow], saved: usize| {
            for row in &mut rows[saved..] {
                row.locale = "zh-CN".to_owned();
            }
        }),
        (
            "later-reversion",
            |rows: &mut [TimelineRow], saved: usize| {
                rows[saved + 1].locale = "en".to_owned();
            },
        ),
    ] {
        let error = assert_dynamic_locale_corruption(parent.path(), name, &original, mutate);
        assert!(!error.is_empty(), "{name}");
    }
}

#[test]
fn allowlist_refuses_extra_and_missing_entries() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();

    let root = parent.path().join("extra-file");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join("extra");
    io(fs::write(&path, b"extra"), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("extra file"));

    let root = parent.path().join("extra-directory");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join("extra");
    io(fs::create_dir(&path), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("extra directory"));

    let root = parent.path().join("missing-file");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(FINAL_LIVENESS_FILE);
    io(fs::remove_file(&path), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("missing file"));
}

#[cfg(unix)]
#[test]
fn tree_walk_refuses_symlinks_including_one_to_a_declared_file() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let root = parent.path().join("symlink");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join(OPERATIONS_FILE);
    io(fs::remove_file(&path), &path).unwrap();
    io(symlink(FINAL_LIVENESS_FILE, &path), &path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("symlink"));

    let target = parent.path().join("root-target");
    stage_valid_at(&target, &trace);
    let root_link = parent.path().join("root-link");
    io(symlink(&target, &root_link), &root_link).unwrap();
    assert!(read_tree(&root_link).unwrap_err().contains("symlink"));
}

#[cfg(unix)]
#[test]
fn tree_walk_refuses_a_non_regular_entry() {
    use std::os::unix::net::UnixListener;

    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let root = parent.path().join("socket");
    let profile = stage_valid_at(&root, &trace);
    let path = root.join("socket");
    let _listener = UnixListener::bind(&path).unwrap();
    assert!(assert_bundle_refuses(&root, &trace, &profile).contains("non-regular"));
}

#[test]
fn changed_fingerprint_removes_every_staged_and_installed_directory() {
    let parent = tempfile::TempDir::new().unwrap();
    let marker = parent.path().join("keep");
    io(fs::write(&marker, b"keep"), &marker).unwrap();
    let trace = record_real_smoke_main().unwrap();
    let mut calls = 0;
    let mut fingerprint = || {
        calls += 1;
        Ok(format!("revision-{calls}"))
    };

    assert!(
        install_bundle(parent.path(), &trace, &mut fingerprint)
            .unwrap_err()
            .contains("source tree changed")
    );
    assert_eq!(calls, 2);
    let entries = io(fs::read_dir(parent.path()), parent.path())
        .unwrap()
        .map(|entry| io(entry, parent.path()).unwrap().file_name())
        .collect::<Vec<_>>();
    assert!(entries.iter().all(|name| {
        let name = name.to_string_lossy();
        !name.starts_with("bundle-") && !name.starts_with(".walker-")
    }));
}

#[test]
fn reinstall_accepts_identical_bytes_and_refuses_different_bytes() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let mut fingerprint = || Ok("revision".to_owned());
    let installed = install_bundle(parent.path(), &trace, &mut fingerprint).unwrap();
    let before = read_tree(&installed).unwrap();

    let reinstalled = install_bundle(parent.path(), &trace, &mut fingerprint).unwrap();
    assert_eq!(reinstalled, installed);
    assert_eq!(read_tree(&installed).unwrap(), before);

    let path = installed.join(RUN_FILE);
    io(fs::write(&path, b"different"), &path).unwrap();
    assert!(
        install_bundle(parent.path(), &trace, &mut fingerprint)
            .unwrap_err()
            .contains("differs")
    );
    assert_eq!(io(fs::read(&path), &path).unwrap(), b"different");
}

#[test]
fn git_identity_and_directory_name_checks_refuse_mismatches() {
    assert!(!source_identity().unwrap().is_empty());
    assert!(real_git(&["not-a-git-command"]).is_err());

    let checkout = tempfile::TempDir::new().unwrap();
    fs::write(checkout.path().join("untracked.txt"), b"untracked bytes").unwrap();
    assert_eq!(
        read_checkout_file(checkout.path(), "untracked.txt").unwrap(),
        b"untracked bytes"
    );
    assert!(read_checkout_file(checkout.path(), "absent.txt").is_err());

    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let root = parent.path().join("staged");
    let profile = stage_valid_at(&root, &trace);
    let wrong = parent.path().join(format!("bundle-{}", "0".repeat(64)));
    io(fs::rename(&root, &wrong), &wrong).unwrap();
    assert!(assert_bundle_refuses(&wrong, &trace, &profile).contains("directory name"));
}

#[test]
fn directory_name_recomputation_uses_only_the_stored_sandbox_metadata() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let mut fingerprint = || Ok("revision".to_owned());
    let installed = install_bundle(parent.path(), &trace, &mut fingerprint).unwrap();
    let profile_path = bundle::profile_directory(&trace.review_profile).join(PROFILE_FILE);
    let (_, files) = read_tree(&installed).unwrap();
    let profile: ProfileManifest = decode(&files[&profile_path]).unwrap();
    let run_path = installed.join(RUN_FILE);
    let mut run = read_run_metadata(&run_path);
    let changed_root = parent
        .path()
        .join("different-random-root")
        .to_string_lossy()
        .into_owned();
    run.sandbox = SandboxMetadata::random(
        trace.sandbox.platform(),
        changed_root,
        trace.review_profile.clone(),
    )
    .unwrap();
    write_run_metadata(&run_path, &run);

    let error = assert_bundle_refuses(&installed, &trace, &profile);
    assert!(error.contains("directory name"), "{error}");
}

#[test]
fn filesystem_errors_use_the_shared_io_funnel() {
    let parent = tempfile::TempDir::new().unwrap();
    let missing = parent.path().join("missing");
    let error = read_tree(&missing).unwrap_err();
    assert!(error.contains("missing"));

    let blocked = parent.path().join("blocked");
    io(fs::write(&blocked, b"file"), &blocked).unwrap();
    assert!(read_tree(&blocked).unwrap_err().contains("not a directory"));
    let profile = SafeProfileId::try_from("profile").unwrap();
    assert!(BundleWriter::create(&blocked, &profile).is_err());
}

#[test]
fn reducer_replay_covers_initial_and_session_boundaries() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let root = parent.path().join("replay-boundaries");
    stage_valid_at(&root, &trace);
    let (_, files) = read_tree(&root).unwrap();
    let objects = read_objects(&files, &trace.rows).unwrap();

    let mut repeated_initial = trace.rows.clone();
    repeated_initial[1].cause = TransitionCause::Initial;
    assert!(
        validate_reducer_replay(&repeated_initial, &objects)
            .unwrap_err()
            .contains("only the first")
    );

    let mut session = trace.rows.clone();
    session[1].cause = TransitionCause::Session {
        requested: json!({"operation": "session"}),
        resolved: Value::Null,
        event: Value::Null,
        handling: json!("consumed"),
    };
    session[1].reducer = session[0].reducer.clone();
    validate_reducer_replay(&session, &objects).unwrap();
    session[1].reducer = session[2].reducer.clone();
    assert!(
        validate_reducer_replay(&session, &objects)
            .unwrap_err()
            .contains("session checkpoint")
    );
}

#[test]
fn c3_leak_oracle_refuses_raw_root_in_every_zero_tolerance_channel() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let root = parent.path().join("c3-raw-root-channels");
    let profile = write_trace_at(&root, &trace);
    let (layout, files) = read_tree(&root).unwrap();
    let run = bundle::decode_run_metadata(&files[Path::new(RUN_FILE)]).unwrap();
    let raw_root = trace.sandbox.profiles().get(&trace.review_profile).unwrap();
    validate_installed_bundle_leaks(&layout, &files, &run, &trace.rows, &trace.leak_oracle_facts)
        .unwrap();

    let first = trace.rows.first().unwrap();
    let profile_root = bundle::profile_directory(&trace.review_profile);
    let chunk = profile.chunks.first().unwrap();
    let cases = [
        PathBuf::from(OPERATIONS_FILE),
        PathBuf::from(FINAL_LIVENESS_FILE),
        bundle::object_path(&first.reducer),
        bundle::object_view_path(&first.reducer),
        bundle::object_path(&first.host),
        bundle::object_view_path(&first.host),
        bundle::object_path(&first.session),
        bundle::object_view_path(&first.session),
        bundle::object_path(&first.geometry),
        bundle::object_view_path(&first.geometry),
        profile_root.join(PROFILE_FILE),
        profile_root.join(TIMELINE_FILE),
        bundle::chunk_json_path(&trace.review_profile, &chunk.id),
        bundle::chunk_view_path(&trace.review_profile, &chunk.id),
    ];

    for path in cases {
        let mut corrupt = files.clone();
        let bytes = if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
            raw_root.as_bytes().to_vec()
        } else if path.file_name().and_then(|name| name.to_str()) == Some(TIMELINE_FILE) {
            let mut bytes = canonical_json_bytes(&json!({
                "cause": {"session": {"requested": raw_root}}
            }))
            .unwrap();
            bytes.push(b'\n');
            bytes
        } else {
            canonical_json_bytes(&json!({"c3_raw_root": raw_root})).unwrap()
        };
        corrupt.insert(path.clone(), bytes);
        let error = validate_installed_bundle_leaks(
            &layout,
            &corrupt,
            &run,
            &trace.rows,
            &trace.leak_oracle_facts,
        )
        .unwrap_err();
        assert!(
            error.contains(&path.display().to_string()),
            "{path:?}: {error}"
        );
        assert!(error.contains("raw sandbox root"), "{path:?}: {error}");
    }
}

#[test]
fn c3_run_metadata_masks_only_its_typed_root_fields() {
    let parent = tempfile::TempDir::new().unwrap();
    let trace = record_real_smoke_main().unwrap();
    let root = parent.path().join("c3-run-mask");
    write_trace_at(&root, &trace);
    let (layout, mut files) = read_tree(&root).unwrap();
    let mut run = bundle::decode_run_metadata(&files[Path::new(RUN_FILE)]).unwrap();

    validate_installed_bundle_leaks(&layout, &files, &run, &trace.rows, &trace.leak_oracle_facts)
        .unwrap();

    run.source_revision = trace.sandbox.root().to_owned();
    files.insert(
        PathBuf::from(RUN_FILE),
        bundle::run_metadata_bytes(&run).unwrap(),
    );
    let error = validate_installed_bundle_leaks(
        &layout,
        &files,
        &run,
        &trace.rows,
        &trace.leak_oracle_facts,
    )
    .unwrap_err();
    assert!(error.contains(RUN_FILE), "{error}");
    assert!(error.contains("raw sandbox root"), "{error}");
}

#[test]
fn c3_scan_coverage_refuses_one_skipped_declared_file() {
    let expected = BTreeSet::from([PathBuf::from(RUN_FILE), PathBuf::from(OPERATIONS_FILE)]);
    let scanned = BTreeSet::from([PathBuf::from(RUN_FILE)]);

    let error = require_exact_scan_coverage(&expected, &scanned).unwrap_err();
    assert!(error.contains(OPERATIONS_FILE), "{error}");
}

fn c3_test_run() -> RunMetadata {
    let profile = SafeProfileId::try_from("c3-profile").unwrap();
    #[cfg(target_os = "windows")]
    let (platform, root) = (SandboxPlatform::Windows, r"C:\sandbox\root");
    #[cfg(target_os = "macos")]
    let (platform, root) = (SandboxPlatform::Macos, "/sandbox/root");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let (platform, root) = (SandboxPlatform::Linux, "/sandbox/root");
    RunMetadata {
        schema: 2,
        source_revision: "revision".to_owned(),
        deterministic_result: "passed".to_owned(),
        operation_count: 1,
        operations_sha256: "0".repeat(64),
        final_liveness_sha256: "1".repeat(64),
        profiles: vec![profile.clone()],
        sandbox: SandboxMetadata::random(platform, root, profile).unwrap(),
    }
}

fn c3_artifact_facts() -> (LeakOracleFacts, Value, Value, u64, u64) {
    let projected_path = "<profile:c3>/data/.drafts/<draft:0>.py";
    let raw_identity = json!({
        "platform": "unix",
        "device": 91,
        "inode": 92,
        "change_time_seconds": 93,
        "change_time_nanoseconds": 94,
    });
    let projected_identity = json!({
        "platform": "unix",
        "device": 0,
        "inode": 0,
        "change_time_seconds": 0,
        "change_time_nanoseconds": 0,
    });
    let raw_modified = 4_294_967_001;
    let projected_modified = 0;
    let artifact = ArtifactLeakOracleFact {
        raw_path_spellings: BTreeSet::from(["/random/raw/skit-new-123abc.py".to_owned()]),
        source_identities: BTreeSet::from([SourceIdentityLeakOraclePair {
            raw: canonical_json_bytes(&raw_identity).unwrap(),
            projected: canonical_json_bytes(&projected_identity).unwrap(),
        }]),
        modified_values: BTreeSet::from([ModifiedLeakOraclePair {
            raw: raw_modified,
            projected: projected_modified,
        }]),
    };
    let facts = LeakOracleFacts {
        artifacts: BTreeMap::from([(projected_path.to_owned(), artifact)]),
        renderer_drafts: BTreeMap::new(),
        ambient_paths: BTreeSet::new(),
    };
    (
        facts,
        raw_identity,
        projected_identity,
        raw_modified,
        projected_modified,
    )
}

#[test]
fn stable_frame_text_needs_no_renderer_observation_grant() {
    let mut run = c3_test_run();
    run.sandbox = SandboxMetadata::stable(
        SandboxPlatform::Linux,
        "/tmp/skit-ui-walker-v1",
        run.profiles.iter().cloned().collect(),
    )
    .unwrap();
    let facts = LeakOracleFacts::default();
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
    let frame = Path::new("objects/styled-frame/frame.json");
    for text in [
        "/tmp/skit-ui-walker-v1/sandboxes/c3/external/picked.sh",
        "skit-new-000000.prompt.md",
        "000000.prompt.md",
        "status: .skit-quarantine-000000/draft",
    ] {
        oracle
            .scan_text(frame, "frame row", text, LeakScanMode::Renderer)
            .unwrap();
    }
}

#[test]
fn ambient_paths_fail_in_every_channel_without_rejecting_a_stable_frame_parent() {
    let mut run = c3_test_run();
    run.sandbox = SandboxMetadata::stable(
        SandboxPlatform::Linux,
        "/tmp/skit-ui-walker-v1",
        run.profiles.iter().cloned().collect(),
    )
    .unwrap();
    let facts = LeakOracleFacts {
        ambient_paths: BTreeSet::from([
            "/tmp".to_owned(),
            "/home/local-user".to_owned(),
            "/checkout/skit".to_owned(),
        ]),
        ..LeakOracleFacts::default()
    };
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
    let file = Path::new("checkpoint");
    let stable = "/tmp/skit-ui-walker-v1/sandboxes/c3/external/picked.sh";
    oracle
        .scan_text(file, "row", stable, LeakScanMode::Renderer)
        .unwrap();
    for mode in [LeakScanMode::Renderer, LeakScanMode::ZeroTolerance] {
        for text in [
            "/tmp/unrelated",
            "/home/local-user/data",
            "/checkout/skit/source",
        ] {
            let error = oracle
                .scan_json(file, &json!({"nested": [text]}), mode)
                .unwrap_err();
            assert!(error.contains("ambient path at /nested/0"), "{error}");
            assert!(
                oracle
                    .scan_json(file, &json!({text: "value"}), mode)
                    .is_err()
            );
        }
    }
    let error = oracle
        .scan_text(
            file,
            "row",
            &format!("{stable} /tmp/unrelated"),
            LeakScanMode::Renderer,
        )
        .unwrap_err();
    assert!(error.contains("ambient path"), "{error}");
    assert!(
        oracle
            .scan_text(file, "value", stable, LeakScanMode::ZeroTolerance)
            .is_err()
    );
    oracle
        .scan_text(
            file,
            "value",
            "/tmp-user /home/local-user-copy",
            LeakScanMode::ZeroTolerance,
        )
        .unwrap();
}

#[test]
fn only_registered_draft_values_fail_in_non_frame_text() {
    let run = c3_test_run();
    for basename in ["skit-new-123abc.py", "skit-new-123abc.prompt.md"] {
        let facts = c3_renderer_facts(&run, basename);
        let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
        assert!(
            oracle
                .scan_text(
                    Path::new("session"),
                    "value",
                    basename,
                    LeakScanMode::ZeroTolerance
                )
                .is_err()
        );
        oracle
            .scan_text(Path::new("frame"), "row", basename, LeakScanMode::Renderer)
            .unwrap();
    }
    let facts = c3_renderer_facts(&run, "skit-new-123abc.py");
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
    let file = Path::new("session");
    for text in ["skit-new-123abc.py", "skit-new-123abc"] {
        assert!(
            oracle
                .scan_text(file, "value", text, LeakScanMode::ZeroTolerance)
                .is_err()
        );
        oracle
            .scan_text(file, "row", text, LeakScanMode::Renderer)
            .unwrap();
    }
    for text in [
        "123abc.py",
        "skit-new-123",
        "skit-new-654321.py",
        "skit-new-user-file.py",
        ".run-my-script.py",
        ".injected-local",
        ".skit-quarantine-note",
    ] {
        oracle
            .scan_text(file, "value", text, LeakScanMode::ZeroTolerance)
            .unwrap();
    }
    let random_path = &facts.renderer_drafts.first_key_value().unwrap().1.raw_path;
    assert!(
        oracle
            .scan_text(file, "row", random_path, LeakScanMode::Renderer)
            .is_err()
    );
}

fn c3_draft_artifact(path: &str, identity: Value, modified: u64) -> Value {
    json!({
        "path": path,
        "modified": modified,
        "identity": identity,
        "permissions": {"readonly": false, "unix_mode": null},
        "content_hash": null,
    })
}

fn c3_source_artifact(path: &str, identity: Value) -> Value {
    json!({
        "path": path,
        "source_record": path,
        "bytes": [112, 114, 105, 110, 116],
        "permissions": {"readonly": false, "unix_mode": null},
        "executable": false,
        "is_regular": true,
        "is_directory": false,
        "is_draft": false,
        "identity": identity,
    })
}

#[test]
fn c3_artifact_identity_and_modified_checks_use_projected_path_facts() {
    let run = c3_test_run();
    let (facts, raw_identity, projected_identity, raw_modified, projected_modified) =
        c3_artifact_facts();
    let projected_path = facts.artifacts.first_key_value().unwrap().0;
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
    let file = Path::new("objects/reducer/fact.json");

    let projected = c3_draft_artifact(
        projected_path,
        projected_identity.clone(),
        projected_modified,
    );
    oracle
        .scan_json(file, &projected, LeakScanMode::ZeroTolerance)
        .unwrap();
    oracle
        .scan_json(
            file,
            &c3_source_artifact(projected_path, projected_identity.clone()),
            LeakScanMode::ZeroTolerance,
        )
        .unwrap();

    for (case, value, expected) in [
        (
            "raw-identity",
            c3_draft_artifact(projected_path, raw_identity, projected_modified),
            "raw source identity",
        ),
        (
            "unknown-identity",
            c3_draft_artifact(
                projected_path,
                json!({
                    "platform": "unix",
                    "device": 0,
                    "inode": 99,
                    "change_time_seconds": 0,
                    "change_time_nanoseconds": 0,
                }),
                projected_modified,
            ),
            "unknown projected source identity",
        ),
        (
            "raw-modified",
            c3_draft_artifact(projected_path, projected_identity.clone(), raw_modified),
            "raw draft modified value",
        ),
        (
            "unknown-modified",
            c3_draft_artifact(projected_path, projected_identity.clone(), 17),
            "unknown projected draft modified value",
        ),
        (
            "unknown-path",
            c3_draft_artifact(
                "<profile:c3>/unknown.py",
                projected_identity,
                projected_modified,
            ),
            "unknown projected path",
        ),
    ] {
        let error = oracle
            .scan_json(file, &value, LeakScanMode::ZeroTolerance)
            .unwrap_err();
        assert!(error.contains(expected), "{case}: {error}");
    }
}

#[test]
fn c3_artifact_facts_refuse_same_pair_and_cross_pair_ambiguity() {
    let run = c3_test_run();
    let identity = |number| canonical_json_bytes(&json!({"identity": number})).unwrap();

    for (case, identities, modified) in [
        (
            "same-identity",
            BTreeSet::from([SourceIdentityLeakOraclePair {
                raw: identity(1),
                projected: identity(1),
            }]),
            BTreeSet::new(),
        ),
        (
            "cross-identity",
            BTreeSet::from([
                SourceIdentityLeakOraclePair {
                    raw: identity(1),
                    projected: identity(2),
                },
                SourceIdentityLeakOraclePair {
                    raw: identity(3),
                    projected: identity(1),
                },
            ]),
            BTreeSet::new(),
        ),
        (
            "same-modified",
            BTreeSet::new(),
            BTreeSet::from([ModifiedLeakOraclePair {
                raw: 10,
                projected: 10,
            }]),
        ),
        (
            "cross-modified",
            BTreeSet::new(),
            BTreeSet::from([
                ModifiedLeakOraclePair {
                    raw: 10,
                    projected: 0,
                },
                ModifiedLeakOraclePair {
                    raw: 20,
                    projected: 10,
                },
            ]),
        ),
    ] {
        let facts = LeakOracleFacts {
            artifacts: BTreeMap::from([(
                "<profile:c3>/ambiguous".to_owned(),
                ArtifactLeakOracleFact {
                    raw_path_spellings: BTreeSet::new(),
                    source_identities: identities,
                    modified_values: modified,
                },
            )]),
            renderer_drafts: BTreeMap::new(),
            ambient_paths: BTreeSet::new(),
        };
        let error = InstalledBundleLeakOracle::new(&run, &facts).unwrap_err();
        assert!(error.contains("overlap"), "{case}: {error}");
    }
}

#[test]
fn c3_artifact_fact_disjointness_is_scoped_per_projected_path() {
    let run = c3_test_run();
    let shared = canonical_json_bytes(&json!({"identity": 7})).unwrap();
    let facts = LeakOracleFacts {
        artifacts: BTreeMap::from([
            (
                "<profile:c3>/first".to_owned(),
                ArtifactLeakOracleFact {
                    raw_path_spellings: BTreeSet::new(),
                    source_identities: BTreeSet::from([SourceIdentityLeakOraclePair {
                        raw: shared.clone(),
                        projected: canonical_json_bytes(&json!({"identity": 0})).unwrap(),
                    }]),
                    modified_values: BTreeSet::from([ModifiedLeakOraclePair {
                        raw: 10,
                        projected: 0,
                    }]),
                },
            ),
            (
                "<profile:c3>/second".to_owned(),
                ArtifactLeakOracleFact {
                    raw_path_spellings: BTreeSet::new(),
                    source_identities: BTreeSet::from([SourceIdentityLeakOraclePair {
                        raw: canonical_json_bytes(&json!({"identity": 8})).unwrap(),
                        projected: shared,
                    }]),
                    modified_values: BTreeSet::from([ModifiedLeakOraclePair {
                        raw: 20,
                        projected: 10,
                    }]),
                },
            ),
        ]),
        renderer_drafts: BTreeMap::new(),
        ambient_paths: BTreeSet::new(),
    };

    InstalledBundleLeakOracle::new(&run, &facts).unwrap();
}

#[test]
fn c3_unregistered_source_requires_facts_only_for_a_nonnull_identity() {
    let run = c3_test_run();
    let (facts, _, projected_identity, _, _) = c3_artifact_facts();
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
    let file = Path::new("objects/reducer/source.json");

    oracle
        .scan_json(
            file,
            &c3_source_artifact("user-owned.py", Value::Null),
            LeakScanMode::ZeroTolerance,
        )
        .unwrap();

    let error = oracle
        .scan_json(
            file,
            &c3_source_artifact("user-owned.py", projected_identity),
            LeakScanMode::ZeroTolerance,
        )
        .unwrap_err();
    assert!(error.contains("unknown projected path"), "{error}");
}

#[test]
fn c3_structural_checks_preserve_identity_text_and_unrelated_numbers() {
    let run = c3_test_run();
    let (facts, raw_identity, _, raw_modified, _) = c3_artifact_facts();
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
    let value = json!({
        "library_size": raw_modified,
        "sequence": raw_modified,
        "timestamp": raw_modified,
        "user_modified": {"modified": raw_modified, "path": "user-owned"},
        "identity_text": String::from_utf8(canonical_json_bytes(&raw_identity).unwrap()).unwrap(),
    });

    oracle
        .scan_json(
            Path::new("objects/host/lookalikes.json"),
            &value,
            LeakScanMode::ZeroTolerance,
        )
        .unwrap();
}

#[test]
fn c3_structural_checks_cover_nested_source_artifacts_and_refuse_wrong_types() {
    let run = c3_test_run();
    let (facts, _, _, _, projected_modified) = c3_artifact_facts();
    let projected_path = facts.artifacts.first_key_value().unwrap().0;
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
    let file = Path::new("profiles/c3/chunks/chunk.json");
    let source = json!({
        "path": projected_path,
        "source_record": "<profile:c3>/data/.drafts/<draft:0>.py",
        "bytes": [112, 114, 105, 110, 116],
        "permissions": {"readonly": false, "unix_mode": null},
        "executable": false,
        "is_regular": true,
        "is_directory": false,
        "is_draft": true,
        "identity": null,
    });
    oracle
        .scan_json(
            file,
            &json!({"nested": [source]}),
            LeakScanMode::ZeroTolerance,
        )
        .unwrap();

    let mut wrong_path = c3_draft_artifact(projected_path, Value::Null, projected_modified);
    wrong_path["path"] = json!(7);
    let error = oracle
        .scan_json(file, &wrong_path, LeakScanMode::ZeroTolerance)
        .unwrap_err();
    assert!(error.contains("no text path"), "{error}");
    for nested in [
        Value::Array(vec![wrong_path.clone()]),
        json!({"nested": wrong_path}),
    ] {
        assert!(
            oracle
                .scan_json(file, &nested, LeakScanMode::ZeroTolerance)
                .unwrap_err()
                .contains("no text path")
        );
    }

    let mut wrong_modified = c3_draft_artifact(projected_path, Value::Null, projected_modified);
    wrong_modified["modified"] = json!("zero");
    let error = oracle
        .scan_json(file, &wrong_modified, LeakScanMode::ZeroTolerance)
        .unwrap_err();
    assert!(error.contains("modified value is not a u64"), "{error}");
}

#[test]
fn c3_fact_provenance_mismatches_refuse_before_scanning() {
    let run = c3_test_run();
    let mut mismatched_key = c3_renderer_facts(&run, "skit-new-123abc.py");
    let (_, fact) = mismatched_key.renderer_drafts.pop_first().unwrap();
    mismatched_key
        .renderer_drafts
        .insert("<different>".to_owned(), fact);
    let error = InstalledBundleLeakOracle::new(&run, &mismatched_key).unwrap_err();
    assert!(error.contains("mismatched projected path"), "{error}");

    let mut mismatched_review = c3_renderer_facts(&run, "skit-new-123abc.py");
    let draft = mismatched_review
        .renderer_drafts
        .first_key_value()
        .map(|(_, draft)| draft)
        .unwrap();
    let mut review = draft.review_names.first().unwrap().clone();
    review.raw_source_path.push_str("-different");
    let projected_path = mismatched_review
        .renderer_drafts
        .first_key_value()
        .unwrap()
        .0
        .clone();
    let draft = mismatched_review
        .renderer_drafts
        .get_mut(&projected_path)
        .unwrap();
    draft.review_names.clear();
    draft.review_names.insert(review);
    let error = InstalledBundleLeakOracle::new(&run, &mismatched_review).unwrap_err();
    assert!(error.contains("source provenance"), "{error}");
}

#[test]
fn c3_zero_tolerance_refuses_a_complete_observed_allocator_token() {
    let run = c3_test_run();
    let facts = c3_renderer_facts(&run, "skit-new-123abc.py");
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();

    let error = oracle
        .scan_text(
            Path::new(OPERATIONS_FILE),
            "complete JSON string",
            "skit-new-123abc.py",
            LeakScanMode::ZeroTolerance,
        )
        .unwrap_err();
    assert!(error.contains("raw draft basename"), "{error}");
}

#[test]
fn c3_windows_roots_use_the_declared_separator() {
    let profile = SafeProfileId::try_from("c3-profile").unwrap();
    let run = RunMetadata {
        sandbox: SandboxMetadata::random(
            SandboxPlatform::Windows,
            r"C:\sandbox\root",
            profile.clone(),
        )
        .unwrap(),
        profiles: vec![profile],
        ..c3_test_run()
    };
    let facts = c3_renderer_facts(&run, "skit-new-123abc.py");
    let draft = &facts.renderer_drafts.first_key_value().unwrap().1;
    assert_eq!(
        draft.raw_path,
        r"C:\sandbox\root\data\drafts\skit-new-123abc.py"
    );
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
    assert!(
        oracle
            .scan_json(
                Path::new("session"),
                &json!({"path": draft.raw_path}),
                LeakScanMode::ZeroTolerance
            )
            .is_err()
    );
}

#[test]
fn c3_styled_frame_path_parser_refuses_nested_objects() {
    assert!(styled_frame_reference(Path::new("objects/styled-frame/nested/frame.json")).is_none());
}

#[test]
fn c3_run_mask_refuses_root_and_profile_mismatches() {
    let run = c3_test_run();
    let baseline = serde_json::to_value(&run).unwrap();
    for (case, pointer, replacement, expected) in [
        (
            "root",
            "/sandbox/root",
            json!("/sandbox/different"),
            "sandbox root changed",
        ),
        (
            "profile-root",
            "/sandbox/profiles/c3-profile",
            json!("/sandbox/different"),
            "profile root changed",
        ),
    ] {
        let mut value = baseline.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let files = BTreeMap::from([(
            PathBuf::from(RUN_FILE),
            canonical_json_bytes(&value).unwrap(),
        )]);
        let error = masked_run_value(&files, &run).unwrap_err();
        assert!(error.contains(expected), "{case}: {error}");
    }

    let mut value = baseline;
    value
        .pointer_mut("/sandbox/profiles")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("extra".to_owned(), json!("/sandbox/extra"));
    let files = BTreeMap::from([(
        PathBuf::from(RUN_FILE),
        canonical_json_bytes(&value).unwrap(),
    )]);
    let error = masked_run_value(&files, &run).unwrap_err();
    assert!(error.contains("profile count changed"), "{error}");
}

#[test]
fn c3_scan_coverage_refuses_an_undeclared_covered_file() {
    let expected = BTreeSet::from([PathBuf::from(RUN_FILE)]);
    let scanned = BTreeSet::from([PathBuf::from(RUN_FILE), PathBuf::from("extra")]);

    let error = require_exact_scan_coverage(&expected, &scanned).unwrap_err();
    assert!(error.contains("extra"), "{error}");
}

#[test]
fn c3_new_trace_cast_path_defaults_to_zero_tolerance() {
    let run = c3_test_run();
    let path = PathBuf::from("future/trace.cast");
    let layout = BundleLayout {
        files: BTreeSet::from([path.clone()]),
        directories: BTreeSet::new(),
    };
    let files = BTreeMap::from([(path, run.sandbox.root().as_bytes().to_vec())]);

    let error =
        validate_installed_bundle_leaks(&layout, &files, &run, &[], &LeakOracleFacts::default())
            .unwrap_err();
    assert!(error.contains("raw sandbox root"), "{error}");
}

#[test]
fn c3_unreferenced_styled_frame_object_defaults_to_zero_tolerance() {
    let run = c3_test_run();
    let (reference, bytes) = build_object(
        ObjectKind::StyledFrame,
        &json!({"leak": run.sandbox.root()}),
    )
    .unwrap();
    let path = bundle::object_path(&reference);
    let layout = BundleLayout {
        files: BTreeSet::from([path.clone()]),
        directories: BTreeSet::new(),
    };
    let files = BTreeMap::from([(path, bytes)]);

    let error =
        validate_installed_bundle_leaks(&layout, &files, &run, &[], &LeakOracleFacts::default())
            .unwrap_err();
    assert!(error.contains("raw sandbox root"), "{error}");
}

#[test]
fn c3_exact_root_scan_preserves_unregistered_lookalikes() {
    let run = c3_test_run();
    let facts = LeakOracleFacts::default();
    let oracle = InstalledBundleLeakOracle::new(&run, &facts).unwrap();
    for lookalike in [
        "/sandbox/root-user",
        "/sandbox/root.py",
        "/sandbox/roots",
        "/different/sandbox/root",
    ] {
        oracle
            .scan_text(
                Path::new(OPERATIONS_FILE),
                "lookalike",
                lookalike,
                LeakScanMode::ZeroTolerance,
            )
            .unwrap();
    }
}

fn c3_renderer_facts(run: &RunMetadata, basename: &str) -> LeakOracleFacts {
    let profile_root = run.sandbox.profiles().first_key_value().unwrap().1;
    let separator = sandbox_separator(run.sandbox.platform());
    let raw_path = format!("{profile_root}{separator}data{separator}drafts{separator}{basename}");
    let stem = allocator_stem(basename).unwrap();
    let projected_path = format!(
        "<profile:c3>/data/.drafts/<draft:c3>{}",
        &basename[stem.len()..]
    );
    let review = RendererReviewNameFact {
        kind: if basename.ends_with(".prompt.md") {
            "prompt"
        } else {
            "python"
        }
        .to_owned(),
        raw_source_path: raw_path.clone(),
        projected_source_path: projected_path.clone(),
        raw_name: stem.to_owned(),
        projected_name: "<draft:c3>".to_owned(),
    };
    let renderer = RendererDraftFact {
        raw_path,
        projected_path: projected_path.clone(),
        raw_kind_picker_basename: basename.to_owned(),
        raw_lossy_basename: basename.to_owned(),
        projected_basename: format!("<draft:c3>{}", &basename[stem.len()..]),
        review_names: BTreeSet::from([review]),
    };
    LeakOracleFacts {
        artifacts: BTreeMap::new(),
        renderer_drafts: BTreeMap::from([(projected_path, renderer)]),
        ambient_paths: BTreeSet::new(),
    }
}

fn replace_first_frame(
    trace: &mut RecordedRealTrace,
    change: impl FnOnce(&mut StyledFrameSnapshot),
) {
    let original = trace.rows[0].styled_frame.clone();
    let value = validate_object(&original, &trace.objects[&object_key(&original)]).unwrap();
    let mut frame: StyledFrameSnapshot = serde_json::from_value(value).unwrap();
    change(&mut frame);
    let replacement = insert_trace_object(
        &mut trace.trace,
        ObjectKind::StyledFrame,
        serde_json::to_value(frame).unwrap(),
    );
    for row in &mut trace.rows {
        if row.styled_frame == original {
            row.styled_frame = replacement.clone();
        }
    }
    trace.objects.remove(&object_key(&original)).unwrap();
    reseal_rows(&mut trace.rows);
    rebuild_trace_cast(&mut trace.trace);
}

fn replace_first_frame_text(trace: &mut RecordedRealTrace, text: &str) {
    replace_first_frame(trace, |frame| {
        let width = usize::from(frame.area.width);
        assert!(text.chars().count() <= width);
        let mut characters = text.chars();
        for cell in &mut frame.cells[..width] {
            cell.symbol = characters.next().unwrap_or(' ').to_string();
            cell.diff = CellDiffSnapshot::None;
            cell.skip = false;
        }
    });
}

#[derive(Clone, Copy, Debug)]
enum C3OmittedSymbolKind {
    ExplicitSkip,
    LegacySkip,
    Continuation,
    OverwideForcedWidth,
}

fn replace_first_frame_omitted_text(
    trace: &mut RecordedRealTrace,
    text: &str,
    kind: C3OmittedSymbolKind,
) {
    replace_first_frame(trace, |frame| {
        let width = usize::from(frame.area.width);
        let row = &mut frame.cells[..width];
        for cell in row.iter_mut().take(2) {
            cell.symbol = " ".to_owned();
            cell.diff = CellDiffSnapshot::None;
            cell.skip = false;
        }
        match kind {
            C3OmittedSymbolKind::ExplicitSkip => {
                row[0].symbol = text.to_owned();
                row[0].diff = CellDiffSnapshot::Skip;
            }
            C3OmittedSymbolKind::LegacySkip => {
                row[0].symbol = text.to_owned();
                row[0].skip = true;
            }
            C3OmittedSymbolKind::Continuation => {
                row[0].symbol = "界".to_owned();
                row[1].symbol = text.to_owned();
            }
            C3OmittedSymbolKind::OverwideForcedWidth => {
                row[0].symbol = text.to_owned();
                row[0].diff = CellDiffSnapshot::ForcedWidth { width: 1 };
            }
        }
    });
}

#[test]
fn c3_resealed_renderer_frames_scan_every_omitted_symbol_for_ambient_paths() {
    let baseline = record_real_smoke_main().unwrap();
    for kind in [
        C3OmittedSymbolKind::ExplicitSkip,
        C3OmittedSymbolKind::LegacySkip,
        C3OmittedSymbolKind::Continuation,
        C3OmittedSymbolKind::OverwideForcedWidth,
    ] {
        let mut trace = baseline.clone();
        let ambient = "/ambient/machine";
        trace
            .leak_oracle_facts
            .ambient_paths
            .insert(ambient.to_owned());
        replace_first_frame_omitted_text(&mut trace, ambient, kind);
        let parent = tempfile::TempDir::new().unwrap();
        let root = parent.path().join("omitted-ambient");
        let profile = write_trace_at(&root, &trace);
        let error = assert_bundle_refuses(&root, &trace, &profile);
        assert!(error.contains("omitted styled cell"), "{kind:?}: {error}");
        assert!(error.contains("ambient path"), "{kind:?}: {error}");
    }
}

#[test]
fn c3_core_view_and_cast_checks_precede_the_leak_oracle() {
    let mut trace = record_real_smoke_main().unwrap();
    let run = RunMetadata {
        sandbox: trace.sandbox.clone(),
        profiles: vec![trace.review_profile.clone()],
        ..c3_test_run()
    };
    trace.leak_oracle_facts = c3_renderer_facts(&run, "skit-new-123abc.py");
    replace_first_frame_text(&mut trace, "skit-new-123abc.py");
    let parent = tempfile::TempDir::new().unwrap();

    let root = parent.path().join("frame-view-first");
    let profile = stage_valid_at(&root, &trace);
    let view = root.join(bundle::object_view_path(&trace.rows[0].styled_frame));
    io(fs::write(&view, b"skit-new-654321.py"), &view).unwrap();
    let error = assert_bundle_refuses(&root, &trace, &profile);
    assert!(error.contains("object view"), "{error}");
    assert!(!error.contains("leak oracle"), "{error}");

    let root = parent.path().join("cast-first");
    let profile = stage_valid_at(&root, &trace);
    let path = cast_path(&root, &trace);
    let mut values = cast_values(&io(fs::read(&path), &path).unwrap());
    values[1][2] = json!("skit-new-654321.py");
    let bytes = cast_bytes(&values);
    io(fs::write(&path, &bytes), &path).unwrap();
    update_profile_for_cast(&root, &trace, &bytes, true);
    let error = assert_bundle_refuses(&root, &trace, &profile);
    assert!(error.contains("rebuild"), "{error}");
    assert!(!error.contains("leak oracle"), "{error}");
}

#[test]
fn c3_facts_never_change_stored_files_or_bundle_digest_inputs() {
    let plain = record_real_smoke_main().unwrap();
    let mut with_facts = plain.clone();
    let mut run = c3_test_run();
    run.sandbox = with_facts.sandbox.clone();
    run.profiles = vec![with_facts.review_profile.clone()];
    with_facts.leak_oracle_facts = c3_renderer_facts(&run, "skit-new-123abc.py");
    let parent = tempfile::TempDir::new().unwrap();
    let plain_root = parent.path().join("plain");
    let facts_root = parent.path().join("facts");

    write_trace_at(&plain_root, &plain);
    write_trace_at(&facts_root, &with_facts);

    let plain_tree = read_tree(&plain_root).unwrap();
    let facts_tree = read_tree(&facts_root).unwrap();
    assert_eq!(plain_tree, facts_tree);
    assert!(facts_tree.1.values().all(|bytes| {
        !bytes
            .windows("leak_oracle_facts".len())
            .any(|window| window == b"leak_oracle_facts")
    }));
}

/// The cut derived review name reaches the recorded corpus as its typed sentinel.
///
/// The operations author one prompt draft and cut its derived review name with `Control+U`.
/// The name stays in the serialized input cut buffer after the paste replaces the value, so
/// the mirror must own that buffer. A raw allocator stem there refuses the whole bundle.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn the_recorded_review_corpus_projects_a_cut_derived_review_name() {
    let scratch = tempfile::TempDir::new().unwrap();
    let corpus = record_real_review_corpus_in(
        review_corpus_namespace(scratch.path()),
        &review_name_cut_corpus_operations(),
        StablePairPhase::Main,
    )
    .unwrap();

    let mut projected = 0_usize;
    for recorded in &corpus.profiles {
        let profile = recorded.review_profile.to_string();
        for ((kind, digest), bytes) in &recorded.objects {
            if *kind != ObjectKind::Session {
                continue;
            }
            let value: Value = serde_json::from_slice(bytes).unwrap();
            let Some(yank) = value.pointer("/value/add/fields/inputs/0/state/yank") else {
                continue;
            };
            let yank = yank.as_str().unwrap();
            assert!(
                !yank.contains("skit-new-"),
                "{profile} {digest} holds a raw allocator cut buffer {yank}"
            );
            if yank.starts_with("<draft:") {
                projected += 1;
            }
        }
    }
    assert!(projected > 0, "no profile recorded a projected cut buffer");
}

/// Every real allocation path reaches the recorded corpus as its typed sentinel.
///
/// The operations author one draft and delete it. The deletion quarantines the draft file, so
/// every profile records one accepted draft quarantine allocation. The leak oracle refuses a
/// raw allocator name in a transcript, so an allocator boundary without a projection cannot
/// reach an installed corpus. The quarantine directory has no renderer channel, so its name
/// must be absent from every recorded byte.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn the_recorded_review_corpus_projects_every_real_allocation_path() {
    let scratch = tempfile::TempDir::new().unwrap();
    let corpus = record_real_review_corpus_in(
        review_corpus_namespace(scratch.path()),
        &draft_quarantine_corpus_operations(),
        StablePairPhase::Main,
    )
    .unwrap();

    let mut purposes = BTreeSet::new();
    for recorded in &corpus.profiles {
        let profile = recorded.review_profile.to_string();
        let mut sentinels = BTreeSet::new();
        for ((kind, digest), bytes) in &recorded.objects {
            assert!(
                !String::from_utf8_lossy(bytes).contains(".skit-quarantine-"),
                "{profile} {kind:?} {digest} holds a raw quarantine name"
            );
            if *kind != ObjectKind::Host {
                continue;
            }
            let value: Value = serde_json::from_slice(bytes).unwrap();
            for event in value["value"]["transcript"].as_array().unwrap() {
                let Some(allocation) = event.get("allocation") else {
                    continue;
                };
                purposes.insert(allocation["purpose"].as_str().unwrap().to_owned());
                let path = allocation["path"]
                    .as_str()
                    .expect("an allocation records its path");
                for raw in ["skit-new-", ".skit-quarantine-", ".injected-", ".run-"] {
                    assert!(
                        !path.contains(raw),
                        "{profile} {digest} holds the raw allocator name {raw} at {path}"
                    );
                }
                for sentinel in ["<draft:", "<quarantine:"] {
                    if path.contains(sentinel) {
                        sentinels.insert(sentinel);
                    }
                }
            }
        }
        assert_eq!(
            sentinels,
            BTreeSet::from(["<draft:", "<quarantine:"]),
            "{profile}"
        );
    }
    assert_eq!(
        purposes,
        BTreeSet::from(["authored_draft".to_owned(), "draft_quarantine".to_owned()])
    );
}

/// A real corpus keeps its deterministic rendered path and installs without row grants.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn the_recorded_review_corpus_keeps_a_rendered_sandbox_root() {
    let scratch = tempfile::TempDir::new().unwrap();
    let corpus = record_real_review_corpus_in(
        review_corpus_namespace(scratch.path()),
        &rendered_sandbox_root_corpus_operations(),
        StablePairPhase::Main,
    )
    .unwrap();
    let painted = corpus.profiles.iter().any(|recorded| {
        recorded.objects.iter().any(|((kind, digest), bytes)| {
            if *kind != ObjectKind::StyledFrame {
                return false;
            }
            let reference = ObjectRef {
                kind: *kind,
                sha256: digest.clone(),
            };
            let value = validate_object(&reference, bytes).unwrap();
            let frame: StyledFrameSnapshot = serde_json::from_value(value).unwrap();
            frame
                .readable_lines()
                .unwrap()
                .iter()
                .any(|line| line.contains(recorded.sandbox.root()))
        })
    });
    assert!(painted, "no profile painted a source path");

    let parent = tempfile::TempDir::new().unwrap();
    let mut fingerprint = fixed_revision();
    let installed = install_review_corpus(parent.path(), &corpus, &mut fingerprint).unwrap();
    let read = read_review_corpus(&installed).unwrap();
    let (_, files) = read_tree(&installed).unwrap();
    let facts = corpus
        .profiles
        .iter()
        .map(|recorded| &recorded.leak_oracle_facts)
        .collect::<Vec<_>>();
    validate_review_corpus_leaks(&files, &read, &facts).unwrap();
}

fn ambient_edge_frame(text: &str, padding: u16) -> StyledFrameSnapshot {
    use ratatui_core::{
        buffer::Buffer,
        layout::{Position, Rect},
        style::Style,
    };

    let mut buffer = Buffer::empty(Rect::new(0, 0, text.cell_width() + padding, 1));
    buffer.set_string(0, 0, text, Style::default());
    StyledFrameSnapshot::from_buffer(&buffer, Position::new(0, 0), false)
}

#[test]
fn a_declared_namespace_prefix_can_reach_the_frame_right_edge() {
    let sandbox = SandboxMetadata::stable(
        SandboxPlatform::Linux,
        "/home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walker-v1",
        BTreeSet::from([SafeProfileId::try_from("edge-test").unwrap()]),
    )
    .unwrap();
    let facts = LeakOracleFacts {
        ambient_paths: BTreeSet::from(["/home/tim".to_owned()]),
        ..LeakOracleFacts::default()
    };
    let oracle = InstalledBundleLeakOracle::for_sandbox(&sandbox, &facts).unwrap();
    for text in [
        "│Installed the skit Agent Skill: /home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walke│",
        "Installed the skit Agent Skill: /home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walke",
        "│已安裝：/home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walke│",
        "│ /home/tim/coding/.tmp/.tmpThOJlv/ski │",
        "│Path: /home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walker-v1│",
    ] {
        let frame = ambient_edge_frame(text, 0);
        assert_eq!(frame.readable_lines().unwrap(), [text]);
        scan_frame(&oracle, Path::new("frame"), &frame).unwrap();
    }
}

#[test]
fn a_frame_edge_does_not_permit_other_ambient_text() {
    let sandbox = SandboxMetadata::stable(
        SandboxPlatform::Linux,
        "/home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walker-v1",
        BTreeSet::from([SafeProfileId::try_from("edge-test").unwrap()]),
    )
    .unwrap();
    let facts = LeakOracleFacts {
        ambient_paths: BTreeSet::from(["/home/tim".to_owned()]),
        ..LeakOracleFacts::default()
    };
    let oracle = InstalledBundleLeakOracle::for_sandbox(&sandbox, &facts).unwrap();
    for (text, padding) in [
        ("│Path: /home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walke│", 3),
        ("Path: /home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walke", 3),
        ("│Path: /home/tim/coding/.tmp/different/skit-ui-walke│", 0),
        (
            "│Path: /home/tim other /home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walke│",
            0,
        ),
        (
            "│Path: /home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walker-v1 /home/tim│",
            0,
        ),
        ("│Path: /home/tim│", 0),
        ("Path: /home/tim", 0),
    ] {
        let error = scan_frame(
            &oracle,
            Path::new("frame"),
            &ambient_edge_frame(text, padding),
        )
        .unwrap_err();
        assert!(error.contains("ambient path"), "{text}: {error}");
    }

    let prefix = "/home/tim/coding/.tmp/.tmpThOJlv/skit-ui-walke";
    assert!(
        oracle
            .scan_text(Path::new("frame"), "text", prefix, LeakScanMode::Renderer)
            .is_err()
    );
    assert!(
        oracle
            .scan_json(
                Path::new("session"),
                &json!({"path": prefix}),
                LeakScanMode::ZeroTolerance
            )
            .is_err()
    );
    let random = SandboxMetadata::random(
        SandboxPlatform::Linux,
        sandbox.root(),
        SafeProfileId::try_from("edge-test").unwrap(),
    )
    .unwrap();
    let random_oracle = InstalledBundleLeakOracle::for_sandbox(&random, &facts).unwrap();
    assert!(
        scan_frame(
            &random_oracle,
            Path::new("frame"),
            &ambient_edge_frame(prefix, 0)
        )
        .is_err()
    );
    let mut omitted = ambient_edge_frame("x", 0);
    omitted.cells[0].symbol = prefix.to_owned();
    omitted.cells[0].diff = CellDiffSnapshot::Skip;
    assert!(scan_frame(&oracle, Path::new("frame"), &omitted).is_err());
}

#[test]
fn canonical_profile_roots_are_frame_data_and_still_fail_in_json() {
    let sandbox = SandboxMetadata::stable(
        SandboxPlatform::Windows,
        r"C:\Users\RUNNER~1\Temp\skit-ui-walker-v1",
        BTreeSet::from([SafeProfileId::try_from("edge-test").unwrap()]),
    )
    .unwrap();
    let canonical = r"C:\Users\runneradmin\Temp\skit-ui-walker-v1\sandboxes\edge-test";
    let mut facts = LeakOracleFacts {
        ambient_paths: BTreeSet::from([r"C:\Users\runneradmin".to_owned()]),
        ..LeakOracleFacts::default()
    };
    facts.artifacts.insert(
        "<profile:canonical-corpus>".to_owned(),
        ArtifactLeakOracleFact {
            raw_path_spellings: BTreeSet::from([
                sandbox.profiles().values().next().unwrap().clone(),
                canonical.to_owned(),
            ]),
            ..ArtifactLeakOracleFact::default()
        },
    );
    let oracle = InstalledBundleLeakOracle::for_sandbox(&sandbox, &facts).unwrap();
    scan_frame(
        &oracle,
        Path::new("frame"),
        &ambient_edge_frame(canonical, 0),
    )
    .unwrap();
    assert!(
        oracle
            .scan_json(
                Path::new("state"),
                &json!({"path": canonical}),
                LeakScanMode::ZeroTolerance
            )
            .is_err()
    );
    let outside = r"C:\Users\runneradmin\other";
    assert!(scan_frame(&oracle, Path::new("frame"), &ambient_edge_frame(outside, 0)).is_err());
}
