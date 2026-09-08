//! Pure byte, path, metadata, and layout contracts for walker bundles.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fmt::Write as _,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::asciicast::{AsciicastRecorder, FRAME_INTERVAL};
use crate::sandbox::{SafeProfileId, SandboxMetadata, SandboxMode, validate_sandbox_metadata};
use crate::{
    ArtifactError, ChunkDescriptor, CorpusManifest, ObjectKind, ObjectRef, Presentation,
    ProfileManifest, RectSnapshot, ReviewProgress, StyledCellSnapshot, StyledFrameSnapshot,
    TimelineRow, TransitionCause, canonical_json_bytes, decode_review_progress, require,
    required_review_profiles, sha256_hex, validate_chunk_ranges, validate_completed_review_claim,
    validate_digest, validate_manifest, validate_styled_frame,
};

/// Directory for canonical content objects.
pub const OBJECT_DIRECTORY: &str = "objects";
/// Directory for readable object views.
pub const VIEW_DIRECTORY: &str = "views";
/// Readable-view directory for styled frames.
pub const FRAME_VIEW_DIRECTORY: &str = "frames";
/// Directory for profile artifacts.
pub const PROFILE_DIRECTORY: &str = "profiles";
/// Directory for one profile's review chunks.
pub const CHUNK_DIRECTORY: &str = "chunks";
/// File name for one profile manifest.
pub const PROFILE_FILE: &str = "profile.json";
/// File name for one profile timeline.
pub const TIMELINE_FILE: &str = "timeline.ndjson";
/// File name for one profile asciicast.
pub const CAST_FILE: &str = "trace.cast";
/// File name for the shared operation vector.
pub const OPERATIONS_FILE: &str = "operations.json";
/// File name for the final liveness request.
pub const FINAL_LIVENESS_FILE: &str = "final-liveness.json";
/// File name for the bundle root index.
pub const RUN_FILE: &str = "run.json";
/// File name for the aggregate corpus manifest.
pub const MANIFEST_FILE: &str = "manifest.json";
/// File name for measured effect coverage.
pub const COVERAGE_FILE: &str = "coverage.json";
/// File name for the reviewer's local progress.
pub const REVIEW_PROGRESS_FILE: &str = "review-progress.json";
/// File name for the review guide.
pub const README_FILE: &str = "README.md";
/// File name for the reviewer's report.
pub const REVIEW_FILE: &str = "review.md";

/// Exact instructions installed with each review corpus.
pub const REVIEW_GUIDE: &[u8] = b"# UI walker review corpus\n\nRead `manifest.json`, then review every chunk in profile order. Read each checkpoint row. Open every object when its digest first appears. Use `views/` for readable state and frame projections. Every checkpoint keeps its complete objects. A row with `presentation=not_presented` is causal evidence from an in-flight reducer or host state. Read its complete state, but do not treat its frame as visible terminal output. You must not report that frame as flicker, a blank UI, stale UI, or another visible defect unless later presented evidence shows the defect. Write findings and the review method in `review.md`. Record each reviewed chunk digest, each profile verdict, and the SHA-256 digest of the completed report in `review-progress.json`. A completed progress file is the local reviewer's attestation that it checked every declared chunk. The validator binds that attestation, the per-profile verdicts, and the report bytes to this corpus.\n";
/// Exact initial report installed with each review corpus.
pub const REVIEW_TEMPLATE: &[u8] = b"# UI review\n\n## Findings\n\n## Review method\n";

/// Files a reviewer may edit inside an installed review corpus.
pub const REVIEWER_MUTABLE_FILES: [&str; 2] = [REVIEW_FILE, REVIEW_PROGRESS_FILE];
/// The installed review corpus directory prefix.
pub const REVIEW_CORPUS_DIRECTORY_PREFIX: &str = "corpus-";

/// Return true for the exact root-level relative path of one reviewer file.
///
/// A chunk view such as `profiles/en-80x24/chunks/review.md` is corpus evidence, so it
/// keeps its immutable bytes.
#[must_use]
pub fn is_reviewer_mutable(relative: &Path) -> bool {
    REVIEWER_MUTABLE_FILES
        .iter()
        .any(|name| relative.as_os_str() == OsStr::new(name))
}

/// Return the installed directory name for one review corpus digest.
pub fn review_corpus_directory_name(digest: &str) -> Result<String, ArtifactError> {
    validate_digest(digest)?;
    Ok(format!("{REVIEW_CORPUS_DIRECTORY_PREFIX}{digest}"))
}

/// Require one regenerated corpus and one installed corpus to share every immutable byte.
///
/// Both maps hold relative paths and file bytes. The two path sets must be equal. The
/// reviewer files may hold different bytes. Every other file must hold the same bytes.
pub fn compare_regenerated_corpus(
    regenerated: &BTreeMap<PathBuf, Vec<u8>>,
    installed: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), ArtifactError> {
    for path in regenerated.keys() {
        if !installed.contains_key(path) {
            return Err(ArtifactError::new(format!(
                "review corpus is missing a file: {}",
                path.display()
            )));
        }
    }
    for path in installed.keys() {
        if !regenerated.contains_key(path) {
            return Err(ArtifactError::new(format!(
                "review corpus has an extra file: {}",
                path.display()
            )));
        }
    }
    for (path, bytes) in regenerated {
        if !is_reviewer_mutable(path) && installed.get(path) != Some(bytes) {
            return Err(ArtifactError::new(format!(
                "review corpus changed an immutable file: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Return the stable directory name for one object kind.
#[must_use]
pub const fn kind_directory(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Reducer => "reducer",
        ObjectKind::Host => "host",
        ObjectKind::Session => "session",
        ObjectKind::StyledFrame => "styled-frame",
        ObjectKind::Geometry => "geometry",
    }
}

/// Return the relative path for one canonical content object.
#[must_use]
pub fn object_path(reference: &ObjectRef) -> PathBuf {
    PathBuf::from(OBJECT_DIRECTORY)
        .join(kind_directory(reference.kind))
        .join(format!("{}.json", reference.sha256))
}

/// Return the relative readable-view path for one content object.
///
/// This function is total over every kind. A styled frame maps to
/// `views/frames/<digest>.txt`.
#[must_use]
pub fn object_view_path(reference: &ObjectRef) -> PathBuf {
    if reference.kind == ObjectKind::StyledFrame {
        PathBuf::from(VIEW_DIRECTORY)
            .join(FRAME_VIEW_DIRECTORY)
            .join(format!("{}.txt", reference.sha256))
    } else {
        PathBuf::from(VIEW_DIRECTORY)
            .join(kind_directory(reference.kind))
            .join(format!("{}.json", reference.sha256))
    }
}

/// Return the relative directory for one profile.
#[must_use]
pub fn profile_directory(profile: &SafeProfileId) -> PathBuf {
    PathBuf::from(PROFILE_DIRECTORY).join(profile.as_str())
}

/// Return the relative canonical JSON path for one chunk.
#[must_use]
pub fn chunk_json_path(profile: &SafeProfileId, chunk: &str) -> PathBuf {
    profile_directory(profile)
        .join(CHUNK_DIRECTORY)
        .join(format!("{chunk}.json"))
}

/// Return the relative readable-view path for one chunk.
#[must_use]
pub fn chunk_view_path(profile: &SafeProfileId, chunk: &str) -> PathBuf {
    profile_directory(profile)
        .join(CHUNK_DIRECTORY)
        .join(format!("{chunk}.md"))
}

/// Build a pretty JSON object view with exactly one trailing newline.
pub fn object_view_bytes(value: &Value) -> Result<Vec<u8>, ArtifactError> {
    let canonical = canonical_json_bytes(value)?;
    let decoded: Value = serde_json::from_slice(&canonical)?;
    let mut view = serde_json::to_vec_pretty(&decoded)?;
    view.push(b'\n');
    Ok(view)
}

fn cell_style_value(cell: &StyledCellSnapshot) -> Value {
    serde_json::json!({
        "foreground": cell.foreground,
        "background": cell.background,
        "underline": cell.underline,
        "modifiers": cell.modifiers,
        "diff": cell.diff,
        "skip": cell.skip,
    })
}

/// Build the legacy readable view for one complete styled frame.
pub fn frame_view_bytes(frame: &StyledFrameSnapshot) -> Result<Vec<u8>, ArtifactError> {
    validate_styled_frame(frame)?;
    let cursor = if frame.cursor_visible {
        format!(
            "visible at column {}, row {}",
            frame.cursor_position.x, frame.cursor_position.y
        )
    } else {
        "hidden".to_owned()
    };
    let mut view = format!(
        "size: {}x{}\ncursor: {cursor}\n\n",
        frame.area.width, frame.area.height
    );
    let lines = frame.readable_lines()?;
    for (row, line) in lines.iter().enumerate() {
        let quoted = serde_json::to_string(line)?;
        writeln!(view, "{row:03}: {quoted}").expect("writing to a String cannot fail");
    }
    view.push_str("\nstyles:\n");
    let default_style = serde_json::json!({
        "foreground": {"kind": "reset"},
        "background": {"kind": "reset"},
        "underline": {"kind": "reset"},
        "modifiers": [],
        "diff": {"kind": "none"},
        "skip": false,
    });
    let width = usize::from(frame.area.width);
    for row in frame.cells.chunks(width) {
        let mut start = 0;
        while start < row.len() {
            let style = cell_style_value(&row[start]);
            let run_length = row[start..]
                .iter()
                .take_while(|cell| cell_style_value(cell) == style)
                .count();
            debug_assert!(run_length > 0);
            let end = start.saturating_add(run_length);
            if style != default_style {
                writeln!(
                    view,
                    "- row {}, columns {}..{}: {}",
                    row[start].y,
                    row[start].x,
                    row[end - 1].x.saturating_add(1),
                    serde_json::to_string(&style)?
                )
                .expect("writing to a String cannot fail");
            }
            start = end;
        }
    }
    Ok(view.into_bytes())
}

/// Encode canonical timeline rows with exactly one trailing newline.
pub fn timeline_ndjson_bytes(rows: &[TimelineRow]) -> Result<Vec<u8>, ArtifactError> {
    let mut bytes = Vec::new();
    for row in rows {
        bytes.extend(canonical_json_bytes(&serde_json::to_value(row)?)?);
        bytes.push(b'\n');
    }
    Ok(bytes)
}

/// Decode canonical timeline rows with exactly one trailing newline.
pub fn decode_timeline_ndjson(bytes: &[u8]) -> Result<Vec<TimelineRow>, ArtifactError> {
    if !bytes.ends_with(b"\n") || bytes.ends_with(b"\n\n") {
        return Err(ArtifactError::new(
            "timeline must have exactly one trailing newline",
        ));
    }
    let mut rows = Vec::new();
    for line in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if line.is_empty() {
            return Err(ArtifactError::new("timeline contains an empty row"));
        }
        let value: Value = serde_json::from_slice(line)?;
        if canonical_json_bytes(&value)? != line {
            return Err(ArtifactError::new("timeline row is not canonical JSON"));
        }
        rows.push(serde_json::from_value(value)?);
    }
    Ok(rows)
}

/// Encode one canonical JSON array of timeline rows without a trailing newline.
pub fn chunk_json_bytes(rows: &[TimelineRow]) -> Result<Vec<u8>, ArtifactError> {
    canonical_json_bytes(&serde_json::to_value(rows)?)
}

/// Decode one JSON value and require its exact canonical bytes.
pub fn decode_canonical_bytes(bytes: &[u8]) -> Result<Value, ArtifactError> {
    let value: Value = serde_json::from_slice(bytes)?;
    if canonical_json_bytes(&value)? != bytes {
        return Err(ArtifactError::new("JSON bytes are not canonical"));
    }
    Ok(value)
}

/// Coverage evidence for all host-served effects in one walker corpus.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectCoverage {
    /// Whether the count maps came from recorded host requests.
    pub measured: bool,
    /// Served host request rows by outer effect kind.
    pub served_requests: Option<BTreeMap<String, u64>>,
    /// Nested Add and Preferences effect occurrences.
    pub nested_occurrences: Option<BTreeMap<String, u64>>,
    /// Effect kinds that the walker cannot serve by construction.
    pub statically_unserved: Vec<String>,
}

/// Typed effect coverage for one walker corpus.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageSummary {
    /// The coverage document schema version.
    pub schema: u16,
    /// Coverage evidence for product effects.
    pub effects: EffectCoverage,
}

impl CoverageSummary {
    /// Build an honest summary before a corpus has measured its host requests.
    pub fn unmeasured_effects(statically_unserved: Vec<String>) -> Result<Self, ArtifactError> {
        let summary = Self {
            schema: 1,
            effects: EffectCoverage {
                measured: false,
                served_requests: None,
                nested_occurrences: None,
                statically_unserved,
            },
        };
        validate_coverage_summary(&summary)?;
        Ok(summary)
    }
}

/// Validate one typed effect coverage summary.
pub fn validate_coverage_summary(summary: &CoverageSummary) -> Result<(), ArtifactError> {
    require(summary.schema == 1, "coverage has an unsupported schema")?;
    let effects = &summary.effects;
    require(
        !effects.statically_unserved.is_empty(),
        "coverage statically unserved list is empty",
    )?;
    for name in &effects.statically_unserved {
        validate_effect_coverage_name(name)?;
    }
    require(
        effects
            .statically_unserved
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "coverage statically unserved list is not sorted and unique",
    )?;

    let (served_requests, nested_occurrences) = match (
        effects.measured,
        &effects.served_requests,
        &effects.nested_occurrences,
    ) {
        (true, Some(served), Some(nested)) => (Some(served), Some(nested)),
        (true, _, _) => {
            return Err(ArtifactError::new(
                "measured coverage requires both count maps",
            ));
        }
        (false, None, None) => (None, None),
        (false, _, _) => {
            return Err(ArtifactError::new(
                "unmeasured coverage must not contain count maps",
            ));
        }
    };

    let static_names = effects
        .statically_unserved
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for counts in [served_requests, nested_occurrences].into_iter().flatten() {
        for (name, count) in counts {
            validate_effect_coverage_name(name)?;
            require(*count > 0, "coverage counts must be positive")?;
            require(
                !static_names.contains(name.as_str()),
                "a counted effect is also statically unserved",
            )?;
        }
    }
    Ok(())
}

fn validate_effect_coverage_name(name: &str) -> Result<(), ArtifactError> {
    let valid = !name.is_empty()
        && name.split('.').all(|segment| {
            let mut bytes = segment.bytes();
            bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
                && bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
                && !segment.ends_with('_')
                && !segment.contains("__")
        });
    require(valid, "coverage contains a noncanonical effect name")
}

/// Encode one validated schema-1 coverage summary as canonical JSON.
pub fn coverage_summary_bytes(summary: &CoverageSummary) -> Result<Vec<u8>, ArtifactError> {
    validate_coverage_summary(summary)?;
    canonical_json_bytes(&serde_json::to_value(summary)?)
}

/// Decode exact canonical schema-1 coverage summary bytes.
pub fn decode_coverage_summary(bytes: &[u8]) -> Result<CoverageSummary, ArtifactError> {
    let value = decode_canonical_bytes(bytes)?;
    let object = value
        .as_object()
        .ok_or_else(|| ArtifactError::new("coverage is not an object"))?;
    require(object.contains_key("schema"), "coverage is missing schema")?;
    require(
        object.contains_key("effects"),
        "coverage is missing effects",
    )?;
    if let Some(schema) = object.get("schema").and_then(Value::as_u64) {
        require(schema == 1, "coverage has an unsupported schema")?;
    }
    let effect_object = object
        .get("effects")
        .and_then(Value::as_object)
        .ok_or_else(|| ArtifactError::new("coverage effects is not an object"))?;
    for field in [
        "measured",
        "served_requests",
        "nested_occurrences",
        "statically_unserved",
    ] {
        require(
            effect_object.contains_key(field),
            "coverage effects is missing a field",
        )?;
    }
    let summary: CoverageSummary = serde_json::from_value(value)?;
    validate_coverage_summary(&summary)?;
    Ok(summary)
}

/// One profile's first-appearance cursor.
///
/// Advance one cursor over that profile's chunks in ordinal order.
#[derive(Clone, Debug, Default)]
pub struct ChunkViewCursor {
    seen_objects: BTreeSet<(ObjectKind, String)>,
}

impl ChunkViewCursor {
    /// Create an empty first-appearance cursor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the legacy markdown view for one chunk and advance the cursor.
    pub fn chunk_view_bytes(
        &mut self,
        rows: &[TimelineRow],
        chunk: &ChunkDescriptor,
    ) -> Result<Vec<u8>, ArtifactError> {
        let start = chunk.start_sequence as usize;
        let end = chunk.end_sequence as usize;
        let chunk_rows = rows
            .get(start..end)
            .ok_or_else(|| ArtifactError::new("chunk range is outside the timeline"))?;
        let mut view = format!(
            "# {}\n\ncheckpoints: {}..{}\nchain: first={:?} last={:?} continues_previous={} continues_next={}\n\n",
            chunk.id,
            chunk.start_sequence,
            chunk.end_sequence,
            chunk.first_chain,
            chunk.last_chain,
            chunk.continues_previous_chain,
            chunk.continues_next_chain,
        );
        for row in chunk_rows {
            let previous = row
                .sequence
                .checked_sub(1)
                .and_then(|sequence| rows.get(sequence as usize));
            let mut changes = Vec::new();
            for (name, changed) in [
                (
                    "reducer",
                    previous.is_none_or(|item| item.reducer != row.reducer),
                ),
                ("host", previous.is_none_or(|item| item.host != row.host)),
                (
                    "session",
                    previous.is_none_or(|item| item.session != row.session),
                ),
                (
                    "frame",
                    previous.is_none_or(|item| item.styled_frame != row.styled_frame),
                ),
                (
                    "geometry",
                    previous.is_none_or(|item| item.geometry != row.geometry),
                ),
            ] {
                if changed {
                    changes.push(name);
                }
            }
            writeln!(
                view,
                "- checkpoint {}: phase={:?} boundary={:?} presentation={} operation={:?} changed={}",
                row.sequence,
                row.phase,
                row.boundary,
                presentation_label(row.presentation),
                row.operation_index,
                changes.join(",")
            )
            .expect("writing to a String cannot fail");
            writeln!(view, "  - cause: {}", cause_summary(&row.cause))
                .expect("writing to a String cannot fail");
            for reference in [
                &row.reducer,
                &row.host,
                &row.session,
                &row.styled_frame,
                &row.geometry,
            ] {
                if self
                    .seen_objects
                    .insert((reference.kind, reference.sha256.clone()))
                {
                    writeln!(
                        view,
                        "  - first object: objects/{}/{}.json",
                        kind_directory(reference.kind),
                        reference.sha256
                    )
                    .expect("writing to a String cannot fail");
                    let readable = if reference.kind == ObjectKind::StyledFrame {
                        format!("views/frames/{}.txt", reference.sha256)
                    } else {
                        format!(
                            "views/{}/{}.json",
                            kind_directory(reference.kind),
                            reference.sha256
                        )
                    };
                    writeln!(view, "    readable: {readable}")
                        .expect("writing to a String cannot fail");
                }
            }
        }
        Ok(view.into_bytes())
    }
}

const fn presentation_label(presentation: Presentation) -> &'static str {
    match presentation {
        Presentation::Presented => "presented",
        Presentation::NotPresented => "not_presented",
    }
}

fn cause_summary(cause: &TransitionCause) -> String {
    let operation = |requested: &Value| {
        requested
            .get("operation")
            .and_then(Value::as_str)
            .or_else(|| requested.get("synthetic").and_then(Value::as_str))
            .unwrap_or("unknown")
            .to_owned()
    };
    let resolved = |value: &Value| {
        value
            .pointer("/semantic_target/command")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/input/target").and_then(Value::as_str))
            .or_else(|| value.pointer("/input/event/type").and_then(Value::as_str))
            .unwrap_or("not_applicable")
            .to_owned()
    };
    match cause {
        TransitionCause::Initial => "initial".to_owned(),
        TransitionCause::Session {
            requested,
            resolved: resolution,
            event,
            handling,
        } => format!(
            "session operation={} event={} resolved={} handling={}",
            operation(requested),
            value_variant(event),
            resolved(resolution),
            handling.as_str().unwrap_or("typed")
        ),
        TransitionCause::Reducer {
            requested,
            resolved: resolution,
            event,
            action,
            emitted,
        } => format!(
            "reducer operation={} event={} resolved={} action={} effect={}",
            operation(requested),
            value_variant(event),
            resolved(resolution),
            value_variant(action),
            value_variant(emitted)
        ),
        TransitionCause::Host {
            round,
            request,
            response,
            emitted,
            ..
        } => format!(
            "host round={round} request={} response={} effect={}",
            value_variant(request),
            value_variant(response),
            value_variant(emitted)
        ),
    }
}

fn value_variant(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            value
                .as_object()
                .and_then(|value| value.keys().next().cloned())
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Return the component-wise maximum of two canvas sizes.
pub fn maximum_canvas(left: (u16, u16), right: (u16, u16)) -> (u16, u16) {
    (left.0.max(right.0), left.1.max(right.1))
}
/// Return the maximum viewport width and height across all rows.
pub fn cast_canvas(rows: &[TimelineRow]) -> Result<(u16, u16), ArtifactError> {
    let first = rows
        .first()
        .ok_or_else(|| ArtifactError::new("cannot build a cast for an empty timeline"))?;
    Ok(rows.iter().skip(1).fold(
        (first.viewport.width, first.viewport.height),
        |canvas, row| maximum_canvas(canvas, (row.viewport.width, row.viewport.height)),
    ))
}

/// Rebuild the asciicast from presented rows and stored frame snapshots.
///
/// Loader errors are plain strings because the loader is the caller's I/O boundary.
pub fn rebuild_presented_cast(
    rows: &[TimelineRow],
    mut load_frame: impl FnMut(&ObjectRef) -> Result<StyledFrameSnapshot, String>,
) -> Result<Vec<u8>, ArtifactError> {
    let (width, height) = cast_canvas(rows)?;
    let mut recorder = AsciicastRecorder::new(width, height)
        .map_err(|error| ArtifactError::new(error.to_string()))?;
    for row in rows {
        match row.presentation {
            Presentation::Presented => {
                let frame = load_frame(&row.styled_frame).map_err(ArtifactError::new)?;
                recorder
                    .record_snapshot(FRAME_INTERVAL, &frame)
                    .map_err(|error| ArtifactError::new(error.to_string()))?;
            }
            Presentation::NotPresented => recorder.skip(FRAME_INTERVAL),
        }
    }
    Ok(recorder.as_bytes().to_vec())
}

/// The expected profile identity supplied by the artifact owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileExpectation<'a> {
    /// The stable profile identifier.
    pub id: &'a SafeProfileId,
    /// The locale at the first checkpoint.
    pub locale: &'a str,
    /// The viewport at the first checkpoint.
    pub viewport: RectSnapshot,
    /// The shared operation-vector digest.
    pub operations_sha256: &'a str,
}

/// Validate one profile manifest against its expected identity.
pub fn validate_profile_manifest(
    profile: &ProfileManifest,
    expected: ProfileExpectation<'_>,
) -> Result<(), ArtifactError> {
    require(&profile.id == expected.id, "manifest profile id is invalid")?;
    require(
        profile.initial_locale == expected.locale,
        "manifest profile initial locale is invalid",
    )?;
    require(
        profile.initial_viewport == expected.viewport,
        "manifest profile initial viewport is invalid",
    )?;
    require(
        profile.operations_sha256 == expected.operations_sha256,
        "manifest profile operation digest is invalid",
    )?;
    validate_digest(&profile.cast_sha256)?;
    require(profile.cast_byte_size > 0, "manifest profile cast is empty")?;
    require(profile.row_count > 0, "manifest profile has no rows")?;
    require(!profile.chunks.is_empty(), "manifest profile has no chunks")?;
    validate_chunk_ranges(profile)
}

/// Root metadata for one immutable walker bundle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunMetadata {
    /// The run metadata schema version.
    pub schema: u16,
    /// The stable source revision or worktree fingerprint.
    pub source_revision: String,
    /// The deterministic run result.
    pub deterministic_result: String,
    /// The number of explicit operations.
    pub operation_count: usize,
    /// The digest of the canonical operation vector.
    pub operations_sha256: String,
    /// The digest of the canonical final liveness request.
    pub final_liveness_sha256: String,
    /// The sorted unique profile identifiers.
    pub profiles: Vec<SafeProfileId>,
    /// Sandbox metadata bound into this run.
    pub sandbox: SandboxMetadata,
}

/// Validate one bundle root index.
pub fn validate_run_metadata(run: &RunMetadata) -> Result<(), ArtifactError> {
    require(run.schema == 2, "run metadata has an unsupported schema")?;
    require(
        run.deterministic_result == "passed",
        "run metadata result did not pass",
    )?;
    validate_nonempty_literal(
        &run.source_revision,
        "run metadata source revision is empty",
    )?;
    require(
        run.operation_count > 0,
        "run metadata operation count is empty",
    )?;
    validate_digest(&run.operations_sha256)?;
    validate_digest(&run.final_liveness_sha256)?;
    require(!run.profiles.is_empty(), "run metadata has no profiles")?;
    validate_sandbox_metadata(&run.sandbox)?;
    let sandbox_profiles = run.sandbox.profiles().keys().cloned().collect::<Vec<_>>();
    let profiles_match = run.profiles == sandbox_profiles;
    require(
        profiles_match,
        "run metadata profiles do not match sandbox metadata",
    )
}

/// Encode one validated schema-2 run metadata object as canonical JSON.
pub fn run_metadata_bytes(run: &RunMetadata) -> Result<Vec<u8>, ArtifactError> {
    validate_run_metadata(run)?;
    canonical_json_bytes(&serde_json::to_value(run)?)
}

/// Decode exact canonical schema-2 run metadata bytes.
pub fn decode_run_metadata(bytes: &[u8]) -> Result<RunMetadata, ArtifactError> {
    let value = decode_canonical_bytes(bytes)?;
    let object = value
        .as_object()
        .ok_or_else(|| ArtifactError::new("run metadata is not an object"))?;
    if let Some(schema) = object.get("schema").and_then(Value::as_u64) {
        require(schema == 2, "run metadata has an unsupported schema")?;
    }
    let run: RunMetadata = serde_json::from_value(value)?;
    validate_run_metadata(&run)?;
    Ok(run)
}

/// Validate one bundle root index for the four-profile review corpus.
///
/// The corpus needs the stable sandbox and the exact required profile set.
/// `validate_run_metadata` guarantees sorted profiles, so only the required ids need a
/// sort.
pub fn validate_review_corpus_run(run: &RunMetadata) -> Result<(), ArtifactError> {
    validate_run_metadata(run)?;
    require(
        run.sandbox.mode() == SandboxMode::Stable,
        "review corpus needs a stable sandbox",
    )?;
    let actual = run
        .profiles
        .iter()
        .map(SafeProfileId::as_str)
        .collect::<Vec<_>>();
    let mut required = required_review_profiles()
        .iter()
        .map(|profile| profile.id)
        .collect::<Vec<_>>();
    required.sort_unstable();
    require(
        actual == required,
        "review corpus run profiles are not the required profiles",
    )
}

/// Bind one corpus manifest to its run index and its stored profile documents.
///
/// `stored_profiles` holds the profile document that each profile tree stores. It must
/// equal the manifest profile list in manifest order.
pub fn validate_review_corpus_manifest(
    manifest: &CorpusManifest,
    run: &RunMetadata,
    stored_profiles: &[ProfileManifest],
) -> Result<(), ArtifactError> {
    validate_manifest(manifest)?;
    require(
        manifest.operations_sha256 == run.operations_sha256,
        "manifest operation digest differs from the run",
    )?;
    require(
        stored_profiles == manifest.profiles,
        "stored profiles differ from the manifest",
    )?;
    let manifest_profiles = manifest
        .profiles
        .iter()
        .map(|profile| &profile.id)
        .collect::<BTreeSet<_>>();
    let run_profiles = run.profiles.iter().collect::<BTreeSet<_>>();
    require(
        run_profiles == manifest_profiles,
        "run profiles differ from the manifest profiles",
    )
}

/// Validate the byte format of one review report.
///
/// The report must be UTF-8 text, must not be empty, and must end with exactly one
/// newline.
pub fn validate_review_report(report: &[u8]) -> Result<(), ArtifactError> {
    require(
        std::str::from_utf8(report).is_ok(),
        "the UI review report is not UTF-8",
    )?;
    require(!report.is_empty(), "the UI review report is empty")?;
    require(
        report.ends_with(b"\n") && !report.ends_with(b"\n\n"),
        "the UI review report must have exactly one trailing newline",
    )
}

/// Validate the three reviewer files of one installed review corpus.
///
/// `readme` must equal `REVIEW_GUIDE`. `report` must satisfy `validate_review_report`. A
/// report digest in the progress file must match the report, even while the review is
/// incomplete. A complete progress file must satisfy `validate_completed_review_claim`.
/// Returns the decoded progress.
pub fn validate_review_files(
    manifest: &CorpusManifest,
    readme: &[u8],
    progress: &[u8],
    report: &[u8],
) -> Result<ReviewProgress, ArtifactError> {
    require(readme == REVIEW_GUIDE, "README.md is not the review guide")?;
    validate_review_report(report)?;
    let progress = decode_review_progress(manifest, progress)?;
    if let Some(review_sha256) = &progress.review_sha256 {
        require(
            *review_sha256 == sha256_hex(report),
            "review progress claims a different report digest",
        )?;
    }
    if progress.complete {
        validate_completed_review_claim(manifest, &progress, report)?;
    }
    Ok(progress)
}

/// Bind every profile document digest and the source revision into one immutable name.
pub fn bundle_digest(
    profile_digests: &BTreeMap<SafeProfileId, String>,
    source_revision: &str,
    sandbox: &SandboxMetadata,
) -> Result<String, ArtifactError> {
    validate_nonempty_literal(source_revision, "bundle source revision is empty")?;
    validate_sandbox_metadata(sandbox)?;
    require(!profile_digests.is_empty(), "bundle digest has no profiles")?;
    for digest in profile_digests.values() {
        validate_digest(digest)?;
    }
    let profiles_match = profile_digests.keys().eq(sandbox.profiles().keys());
    require(
        profiles_match,
        "bundle profile digests do not match sandbox metadata",
    )?;
    let value = serde_json::json!({
        "profiles": profile_digests,
        "sandbox": sandbox,
        "source_revision": source_revision,
    });
    Ok(sha256_hex(&canonical_json_bytes(&value)?))
}

fn validate_nonempty_literal(value: &str, message: &'static str) -> Result<(), ArtifactError> {
    require(!value.is_empty(), message)?;
    require(value.trim() == value, message)
}

/// Combine a source head and worktree state into one stable revision string.
///
/// `read_untracked` receives each NUL-separated relative path from the untracked file list.
pub fn source_revision(
    head: &str,
    diff: &[u8],
    untracked: &[u8],
    mut read_untracked: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<String, ArtifactError> {
    if diff.is_empty() && untracked.is_empty() {
        return Ok(head.trim().to_owned());
    }
    let mut worktree = diff.to_vec();
    for relative in untracked
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let relative_path =
            std::str::from_utf8(relative).map_err(|error| ArtifactError::new(error.to_string()))?;
        let bytes = read_untracked(relative_path).map_err(ArtifactError::new)?;
        worktree.extend_from_slice(&(relative.len() as u64).to_be_bytes());
        worktree.extend_from_slice(relative);
        worktree.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        worktree.extend_from_slice(&bytes);
    }
    Ok(format!(
        "{}+worktree:{}",
        head.trim(),
        sha256_hex(&worktree)
    ))
}

/// The exact relative file and directory sets for one bundle.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BundleLayout {
    /// Every expected relative file path.
    pub files: BTreeSet<PathBuf>,
    /// Every expected relative directory path.
    pub directories: BTreeSet<PathBuf>,
}

impl BundleLayout {
    /// Declare root files, row-derived object trees, readable views, and one profile.
    pub fn expect_bundle(
        &mut self,
        profile: &ProfileManifest,
        rows: &[TimelineRow],
    ) -> Result<(), ArtifactError> {
        for file in [RUN_FILE, OPERATIONS_FILE, FINAL_LIVENESS_FILE] {
            self.insert_file(file);
        }
        for directory in [OBJECT_DIRECTORY, VIEW_DIRECTORY, PROFILE_DIRECTORY] {
            self.insert_directory(directory);
        }
        for kind in [
            ObjectKind::Reducer,
            ObjectKind::Host,
            ObjectKind::Session,
            ObjectKind::StyledFrame,
            ObjectKind::Geometry,
        ] {
            self.insert_directory(PathBuf::from(OBJECT_DIRECTORY).join(kind_directory(kind)));
            if kind != ObjectKind::StyledFrame {
                self.insert_directory(PathBuf::from(VIEW_DIRECTORY).join(kind_directory(kind)));
            }
        }
        self.insert_directory(PathBuf::from(VIEW_DIRECTORY).join(FRAME_VIEW_DIRECTORY));

        let profile_root = profile_directory(&profile.id);
        self.insert_directory(profile_root.clone());
        self.insert_directory(profile_root.join(CHUNK_DIRECTORY));
        self.insert_file(profile_directory(&profile.id).join(PROFILE_FILE));
        self.insert_file(profile_directory(&profile.id).join(TIMELINE_FILE));
        self.insert_file(profile_directory(&profile.id).join(CAST_FILE));
        for chunk in &profile.chunks {
            self.insert_file(chunk_json_path(&profile.id, &chunk.id));
            self.insert_file(chunk_view_path(&profile.id, &chunk.id));
        }
        for row in rows {
            for reference in [
                &row.reducer,
                &row.host,
                &row.session,
                &row.styled_frame,
                &row.geometry,
            ] {
                validate_digest(&reference.sha256)?;
                self.insert_file(object_path(reference));
                self.insert_file(object_view_path(reference));
            }
        }
        Ok(())
    }

    /// Declare the exact aggregate layout for all required review profiles.
    pub fn expect_review_corpus(
        &mut self,
        manifest: &CorpusManifest,
        rows_by_profile: &BTreeMap<SafeProfileId, Vec<TimelineRow>>,
    ) -> Result<(), ArtifactError> {
        validate_manifest(manifest)?;
        let manifest_profiles = manifest
            .profiles
            .iter()
            .map(|profile| &profile.id)
            .collect::<BTreeSet<_>>();
        let row_profiles = rows_by_profile.keys().collect::<BTreeSet<_>>();
        let profile_sets_match = row_profiles == manifest_profiles;
        require(profile_sets_match, "aggregate profile rows differ")?;

        let mut staged = self.clone();
        for profile in &manifest.profiles {
            let rows = &rows_by_profile[&profile.id];
            let count_matches = usize::try_from(profile.row_count).ok() == Some(rows.len());
            require(count_matches, "aggregate row count differs")?;
            let profiles_match = rows.iter().all(|row| row.profile == profile.id);
            require(profiles_match, "aggregate rows contain the wrong profile")?;
            staged.expect_bundle(profile, rows)?;
        }
        for file in [
            MANIFEST_FILE,
            COVERAGE_FILE,
            REVIEW_PROGRESS_FILE,
            README_FILE,
            REVIEW_FILE,
        ] {
            staged.insert_file(file);
        }
        *self = staged;
        Ok(())
    }

    /// Add one expected relative file path.
    pub fn insert_file(&mut self, relative: impl Into<PathBuf>) {
        self.files.insert(relative.into());
    }

    /// Add one expected relative directory path.
    pub fn insert_directory(&mut self, relative: impl Into<PathBuf>) {
        self.directories.insert(relative.into());
    }

    /// Require exact file and directory set equality in both directions.
    pub fn compare(&self, actual: &Self) -> Result<(), ArtifactError> {
        if let Some(path) = self.files.difference(&actual.files).next() {
            return Err(ArtifactError::new(format!(
                "bundle is missing file: {}",
                path.display()
            )));
        }
        if let Some(path) = actual.files.difference(&self.files).next() {
            return Err(ArtifactError::new(format!(
                "bundle has an extra file: {}",
                path.display()
            )));
        }
        if let Some(path) = self.directories.difference(&actual.directories).next() {
            return Err(ArtifactError::new(format!(
                "bundle is missing directory: {}",
                path.display()
            )));
        }
        if let Some(path) = actual.directories.difference(&self.directories).next() {
            return Err(ArtifactError::new(format!(
                "bundle has an extra directory: {}",
                path.display()
            )));
        }
        Ok(())
    }
}
