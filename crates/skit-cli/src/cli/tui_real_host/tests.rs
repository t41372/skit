//! Shared fixtures and contracts of the real walker host.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fs,
    fs::OpenOptions,
    io,
    path::{Path, PathBuf},
    rc::Rc,
};

use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::Event;
use serde_json::{Value, json};
use skit_application::{
    CreateEntry, EntryPayload, SourcePermissions, form_state::FormStateService,
    preferences::PreferencesChangeSet,
};
use skit_domain::{
    EntryKind, EntrySettings, Slug, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterValue},
};
use skit_i18n::{Locale, Localize, Message, format_text};
use skit_language::write_managed_params;
use skit_runtime::{
    DependencyCommandOutput, InjectedCommandOutput, InjectedCommandUnavailable,
    JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable,
};
use skit_store::{AgentSkillInstallPoint, FileConfigStore, FileFormStateStore, PromptRunner};
use skit_tui::{
    AddControlId, EventHandling, LocalActionTarget, TuiSession, ViewGeometry, render_with_session,
};
use skit_tui_walker_support::engine::CheckpointCauseProjection;
use skit_tui_walker_support::sandbox::{
    STABLE_SANDBOX_NAMESPACE, SafeProfileId, SandboxEvidenceState, SandboxMode, SandboxPlatform,
    profile_lease_path,
};

// The stable sandbox supports Linux and Windows only. Its tests use these items.
#[cfg(any(target_os = "linux", target_os = "windows"))]
use skit_tui_walker_support::sandbox::{
    NAMESPACE_MARKER_FILE, NamespaceMarker, ParentInitLockMarker, ProfileLeaseMarker,
    SANDBOX_MARKER_FILE, SandboxMarker, SandboxRoots, profile_cleanup_path, profile_sandbox_path,
};

use skit_ui::{
    Action, AddAction, AddWorkflowState, DraftKind, Effect, FieldValue, FormPurpose, HostRequest,
    LibraryState, PreferencesAction, PreferencesEffect, ReviewDefaults, SourceSnapshot,
};

// Only the Linux non-UTF-8 contract picks a kind through the typed serde boundary.
#[cfg(target_os = "linux")]
use skit_ui::KnownEntryKind;

use crate::cli::tui_host::{
    FileAllocator as _, PrivateDirectoryPurpose, SYSTEM_FILE_ALLOCATOR, TempLocation,
    TemporaryFilePurpose,
};
use crate::run::{StageWriteFaultGuard, new_injected_file_with_allocator};

#[cfg(unix)]
use super::host::SeededTuiHost;
#[cfg(windows)]
use super::observation::escaped_os;
use super::{
    adapters::{
        AllocationFailure, AllocationLocation, AllocationMaximums, AllocationOutcome,
        AllocationPurpose, DependencyScript, EditorScript, InjectedScript, JavaScriptGateScript,
        PortEvent, PreferenceFailure, PrivateModeFailure, PrivateModeTarget, ProbeResult,
        RecordingAdapters, UvFetchScript,
    },
    host::RealWalkerHost,
    observation::{
        ByteView, HostObservation, ObservationModeProvenance, ObservationNodeKind, encode_hex,
        escaped_wide_units, first_system_temp_residual, portable_mode, sorted_tui_drafts,
    },
    projection::{
        AddProjectionCause, AddProjectionProvenance, PathMap, ReviewNameProjection,
        StatusProjectionCause, ambient_path_spellings,
    },
    projection_cause::{
        add_projection_cause, deserialize_canonical, external_payload_mut, external_tag,
        require_external_unit, require_internal_tag, review_default_name, screen_tag,
    },
    seed::{
        DirectorySeedIo, WalkerDirectoryRoot, WalkerDirectorySeed, WalkerExternalReferenceSeed,
        WalkerExternalSeed, WalkerFormSeed, WalkerLastRunSeed, WalkerSeedSpec, set_unix_mode,
        validate_external_spec,
    },
    stable_namespace::{
        SandboxError, SandboxFaultPoint, StableHostError, StableHostPrimaryError,
        StableSandboxNamespace, combine_sandbox_release, current_sandbox_platform,
        map_contract_result, stable_namespace_root_for,
    },
    stable_sandbox::{
        Evidence, SandboxOwner, names_the_same_path, ordinary_path, ordinary_windows_path,
    },
};
#[cfg(target_os = "linux")]
use super::{
    seed::SystemDirectorySeedIo,
    stable_namespace::{
        acquire_initialization_guard, acquire_initialization_guard_with_hooks,
        acquire_profile_lease_retained, acquire_profile_lease_retained_with_hooks,
        initialize_namespace_retained_with_hook, initialize_namespace_retained_with_hooks,
        no_after_namespace_release_hook, no_before_namespace_release_hook,
        no_namespace_marker_publish_hook, no_profile_lease_hook, publish_profile_lease_marker,
        publish_relative_marker, release_initialization_guard, release_profile_lease,
        require_ticket, validate_namespace_opened_with_hook, validate_relative_marker,
        validate_relative_marker_with_hook,
    },
    stable_sandbox::{
        CleanupHooks, classify_retained_sandbox, create_fresh_sandbox_retained_with_hooks,
        no_acquisition_after_prepare_hook, no_candidate_root_hook, no_candidate_tickets_hook,
        no_cleanup_marker_hook, no_cleanup_plan_hook, no_cleanup_root_hook,
        no_fresh_sandbox_after_marker_hook, no_fresh_sandbox_before_marker_hook,
        open_candidate_parts_with_hooks, prepare_fresh_sandbox_retained,
        quarantine_original_sandbox_with_hook, recover_cleanup_sandbox_with_hook,
        remove_cleanup_sandbox_with_hooks, validate_cleanup_candidate, validate_original_candidate,
        validate_same_cleanup_with_hook, validate_same_original_with_hook,
    },
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use super::{
    stable_namespace::{
        SandboxFaults, StableSandboxPaths, default_stable_namespace_root,
        initialize_namespace_retained, map_profile_lock_result,
    },
    stable_sandbox::StableSandbox,
};

mod allocator;
mod checkpoints;
mod drafts;
mod external_seeds;
mod leak_facts;
mod observation;
mod observation_modes;
mod paths;
mod ports;
mod projection_drafts;
mod projection_provenance;
mod projection_schema;
mod projection_session;
mod seeding;
mod stable_host;
mod stable_namespace;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod stable_sandbox;

/// Name one test of `module` for the `--exact` filter of the test harness.
///
/// `module_path!()` starts with the crate name. The harness omits that segment.
fn harness_test_name(module: &str, test: &str) -> String {
    let module = module.split_once("::").map_or(module, |(_, rest)| rest);
    format!("{module}::{test}")
}

#[cfg(unix)]
fn restrictive_umask_child_profile() -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;

    let template = std::env::var_os("LLVM_PROFILE_FILE")?;
    let parent = Path::new(&template)
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let template_prefix = Path::new(&template)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let profile_prefix = format!(
        "{}umask-walker-",
        template_prefix.split('%').next().unwrap_or_default()
    );
    let profile = tempfile::Builder::new()
        .prefix(&profile_prefix)
        .suffix(".profraw")
        .tempfile_in(parent)
        .unwrap();
    fs::set_permissions(profile.path(), fs::Permissions::from_mode(0o600)).unwrap();
    let (file, path) = profile.keep().unwrap();
    drop(file);
    Some(path)
}

#[cfg(not(unix))]
fn restrictive_umask_child_profile() -> Option<PathBuf> {
    None
}

fn expect_authored_draft_source(action: Action) -> SourceSnapshot {
    match action {
        Action::Add(AddAction::DraftEdited {
            result: Ok(Some(source)),
            ..
        }) => source,
        _ => panic!("the writing editor must retain the authored draft"),
    }
}

fn expect_add_action(action: Action) -> AddAction {
    match action {
        Action::Add(action) => action,
        _ => panic!("the response must be an Add action"),
    }
}

#[test]
#[should_panic(expected = "the writing editor must retain the authored draft")]
fn authored_draft_source_extraction_refuses_other_actions() {
    let _ = expect_authored_draft_source(Action::SetStatus("not a draft".to_owned()));
}

#[test]
#[should_panic(expected = "the response must be an Add action")]
fn c2_add_action_extraction_refuses_other_actions() {
    let _ = expect_add_action(Action::SetStatus("not Add".to_owned()));
}

fn safe_profile(value: &str) -> SafeProfileId {
    SafeProfileId::try_from(value).unwrap()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn stable_paths(
    namespace_root: PathBuf,
    profile: SafeProfileId,
) -> Result<StableSandboxPaths, SandboxError> {
    StableSandboxNamespace::explicit(namespace_root)
        .and_then(|namespace| StableSandboxPaths::new(namespace, profile))
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn create_private_test_directory(path: &Path) {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn write_private_test_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn create_valid_sandbox_evidence(path: &Path, paths: &StableSandboxPaths) {
    create_private_test_directory(path);
    for child in [
        "data",
        "state",
        "config",
        "home",
        "cwd",
        "external",
        "system-temp",
    ] {
        create_private_test_directory(&path.join(child));
    }
    write_private_test_file(
        &path.join(SANDBOX_MARKER_FILE),
        paths.sandbox_marker_bytes(),
    );
}

fn assert_system_temp_is_empty(host: &RealWalkerHost) {
    assert!(
        fs::read_dir(&host.adapters.system_temp)
            .unwrap()
            .next()
            .is_none()
    );
}

fn command(name: &str) -> CreateEntry {
    let mut parameter = ParamDecl::new("value");
    parameter.default = Some(ParameterValue::String("default".to_owned()));
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Copy,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: "walker command".to_owned(),
        payload: None,
        settings: EntrySettings {
            template: "printf {value}".to_owned(),
            params: vec!["value".to_owned()],
            parameters: vec![parameter],
            ..EntrySettings::default()
        },
    }
}

fn injected_script(kind: &str, name: &str, source: &str, stored_name: &str) -> CreateEntry {
    let mut declaration = ParamDecl::new("TOKEN");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.default = Some(ParameterValue::String("before".to_owned()));
    let source = write_managed_params(kind, source, std::slice::from_ref(&declaration)).unwrap();
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode: StorageMode::Copy,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: source.into_bytes(),
            stored_name: Some(stored_name.to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings {
            params: vec!["TOKEN".to_owned()],
            parameters: vec![declaration],
            ..EntrySettings::default()
        },
    }
}

fn profile() -> WalkerSeedSpec {
    WalkerSeedSpec {
        profile: "fixture".to_owned(),
        entries: vec![
            command("Command"),
            CreateEntry {
                name: "Binary bytes".to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "store".to_owned(),
                description: "non-UTF-8 source bytes".to_owned(),
                payload: Some(EntryPayload {
                    bytes: b"print('\xff')\n\xff".to_vec(),
                    stored_name: Some("script.py".to_owned()),
                    permissions: SourcePermissions {
                        readonly: false,
                        unix_mode: Some(0o4751),
                    },
                }),
                settings: EntrySettings::default(),
            },
            CreateEntry {
                name: "JavaScript".to_owned(),
                kind: EntryKind::parse("js").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "store".to_owned(),
                description: "dependency preflight".to_owned(),
                payload: Some(EntryPayload {
                    bytes: b"console.log('ok');\n".to_vec(),
                    stored_name: Some("script.js".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    dependencies: vec!["left-pad".to_owned()],
                    interpreter: "node".to_owned(),
                    ..EntrySettings::default()
                },
            },
        ],
        settings: BTreeMap::from([
            ("after_run".to_owned(), "stay".to_owned()),
            ("lang".to_owned(), "en".to_owned()),
        ]),
        runners: vec![PromptRunner {
            name: "walker".to_owned(),
            argv: vec!["agent".to_owned(), "{{prompt}}".to_owned()],
        }],
        forms: vec![WalkerFormSeed {
            selector: "Command".to_owned(),
            values: BTreeMap::from([("value".to_owned(), "remembered".to_owned())]),
            extra_args: vec!["tail".to_owned()],
            extra_args_raw: false,
            preset: Some("favorite".to_owned()),
            last_run: Some(WalkerLastRunSeed {
                exit: 7,
                at: "2026-08-28T11:00:00+00:00".to_owned(),
                values: Some(BTreeMap::from([(
                    "value".to_owned(),
                    "remembered".to_owned(),
                )])),
            }),
        }],
        prompt_runner: "walker".to_owned(),
        external: vec![WalkerExternalSeed::File {
            path: PathBuf::from("outside.sh"),
            bytes: b"printf outside\n".to_vec(),
            readonly: false,
            unix_mode: 0o640,
        }],
        external_references: vec![WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: "Reference".to_owned(),
                kind: EntryKind::parse("shell").unwrap(),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: "outside reference".to_owned(),
                payload: Some(EntryPayload {
                    bytes: b"printf outside\n".to_vec(),
                    stored_name: Some("outside.sh".to_owned()),
                    permissions: SourcePermissions {
                        readonly: false,
                        unix_mode: Some(0o640),
                    },
                }),
                settings: EntrySettings::default(),
            },
            source: PathBuf::from("outside.sh"),
        }],
        directories: Vec::new(),
        editor_writes: Vec::new(),
    }
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
struct OutsideRecord {
    path: String,
    kind: String,
    content: Option<Vec<u8>>,
    mode: Option<u32>,
    readonly: bool,
    target: Option<String>,
}

fn outside_snapshot_portable(root: &std::path::Path) -> Vec<OutsideRecord> {
    fn visit(root: &std::path::Path, path: &std::path::Path, output: &mut Vec<OutsideRecord>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let relative = path.strip_prefix(root).unwrap().display().to_string();
        let mode = portable_mode(&metadata);
        let readonly = metadata.permissions().readonly();
        if metadata.file_type().is_symlink() {
            output.push(OutsideRecord {
                path: relative,
                kind: "symlink".to_owned(),
                content: None,
                mode,
                readonly,
                target: Some(fs::read_link(path).unwrap().display().to_string()),
            });
        } else if metadata.is_file() {
            output.push(OutsideRecord {
                path: relative,
                kind: "file".to_owned(),
                content: Some(fs::read(path).unwrap()),
                mode,
                readonly,
                target: None,
            });
        } else {
            output.push(OutsideRecord {
                path: relative,
                kind: "directory".to_owned(),
                content: None,
                mode,
                readonly,
                target: None,
            });
            let mut children = fs::read_dir(path)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            children.sort_by_key(fs::DirEntry::file_name);
            for child in children {
                visit(root, &child.path(), output);
            }
        }
    }

    let mut output = Vec::new();
    visit(root, root, &mut output);
    output.sort();
    output
}

fn events_with_keys(transcript: &[Value], keys: &[&str]) -> Vec<Value> {
    transcript
        .iter()
        .filter(|event| keys.iter().any(|key| event.get(*key).is_some()))
        .cloned()
        .collect()
}

#[cfg(unix)]
fn assert_identity_sentinel(value: &Value, identity: u64, change: u64) {
    assert_eq!(
        value,
        &json!({
            "platform": "unix",
            "device": 0,
            "inode": identity,
            "change_time_seconds": 0,
            "change_time_nanoseconds": change,
        })
    );
}

#[cfg(windows)]
fn assert_identity_sentinel(value: &Value, identity: u64, change: u64) {
    assert_eq!(
        value,
        &json!({
            "platform": "windows",
            "volume_serial_number": 0,
            "file_index": identity.to_string(),
            "creation_time": change,
        })
    );
}

#[cfg(not(any(unix, windows)))]
fn assert_identity_sentinel(value: &Value, _identity: u64, _change: u64) {
    assert_eq!(value, &Value::Null);
}

fn write_draft_at(
    host: &RealWalkerHost,
    name: &str,
    bytes: &[u8],
    modified_seconds: u64,
) -> PathBuf {
    let drafts = crate::cli::create_owned_drafts_dir(&host.roots().data).unwrap();
    let path = drafts.join(name);
    fs::write(&path, bytes).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(modified_seconds),
        ))
        .unwrap();
    path
}

// APFS refuses a non-UTF-8 file name, so only Linux can write this fixture.
#[cfg(target_os = "linux")]
fn write_non_utf_draft_at(host: &RealWalkerHost) -> PathBuf {
    use std::os::unix::ffi::OsStringExt as _;

    let drafts = crate::cli::create_owned_drafts_dir(&host.roots().data).unwrap();
    let path = drafts.join(std::ffi::OsString::from_vec(
        b"skit-non-utf-\xff.unknown".to_vec(),
    ));
    fs::write(&path, b"opaque draft\n").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            fs::FileTimes::new().set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10),
            ),
        )
        .unwrap();
    path
}

fn write_unicode_draft_at(host: &RealWalkerHost, name: &str, bytes: &[u8]) -> PathBuf {
    write_draft_at(host, name, bytes, 10)
}

fn set_review_name_projection(
    host: &mut RealWalkerHost,
    path: &Path,
    raw_name: &str,
    stable_name: &str,
) {
    host.path_map.add_provenance.review_name = Some(ReviewNameProjection {
        source_path: path.to_path_buf(),
        kind: "python".to_owned(),
        raw_name: raw_name.to_owned(),
        stable_name: stable_name.to_owned(),
    });
}

fn c2_memory_picker(root: &Path, draft: Option<&Path>) -> Value {
    let mut files = vec![json!(root.join("plain.txt"))];
    let mut entries = vec![json!({
        "name": "plain.txt",
        "path": root.join("plain.txt"),
        "entry_type": {"file": {"extension": "txt", "size": 0}},
    })];
    if let Some(draft) = draft {
        files.push(json!(draft));
        entries.push(json!({
            "name": draft.file_name().unwrap().to_string_lossy(),
            "path": draft,
            "entry_type": {"file": {"extension": "py", "size": 12}},
        }));
    }
    entries.push(json!({
        "name": "linked.txt",
        "path": root.join("linked.txt"),
        "entry_type": {"symlink": {"target": root.join("target.txt")}},
    }));
    json!({
        "kind": "file_picker",
        "fields": {
            "contract": {
                "purpose": "source",
                "start_dir": root,
                "selection": "file",
                "allow_multiple": true,
                "show_hidden": false,
                "query": format!("{}-query", root.display()),
                "output_policy": {"relative_to": root},
            },
            "explorer": {
                "current_dir": root,
                "entries": entries,
                "cursor_index": 0,
                "scroll": 0,
                "selected_files": files,
                "show_hidden": false,
                "mode": "browse",
                "search_query": format!("{}-search", root.display()),
                "filtered_indices": null,
            },
            "query": {
                "value": format!("{}-widget", root.display()),
                "cursor": root.display().to_string().chars().count() + 7,
                "yank": "",
                "last_was_cut": false,
            },
            "current_directory_focused": false,
            "visible_height": 1,
            "io_error": format!("Could not read {}.", root.display()),
            "footer_scroll": {"content_length": 0, "scroll_offset": 0},
            "footer_viewport": {"x": 0, "y": 0, "width": 0, "height": 0},
            "footer_visible_height": 0,
            "click": null,
            "query_editable": null,
            "memory_source": {
                "kind": "memory_file_picker_source",
                "fields": {
                    "root": root,
                    "directories": [root, root.join("directory")],
                    "files": files,
                },
            },
        },
    })
}

fn c2_session(root: &Path, draft: Option<&Path>, inputs: Vec<Value>) -> Value {
    let picker = c2_memory_picker(root, draft);
    let stage = if inputs.iter().any(|input| input["id"] == "ReviewName") {
        "review"
    } else {
        "source"
    };
    json!({
        "schema_version": 1,
        "preferences": {
            "kind": "preferences",
            "fields": {
                "agent_signature": [{"name": "codex", "scope": "user", "base": root}],
            },
        },
        "add": {
            "kind": "add",
            "fields": {
                "signature": {
                    "stage": stage,
                    "kind": null,
                    "storage": null,
                    "dependency_surface": null,
                    "interpolate": null,
                    "drafts": 0,
                    "candidates": [],
                    "prompt_candidates": [],
                    "runners": [],
                },
                "picker_root": root,
                "advertised": [{
                    "event": {"open_path_picker": {
                        "purpose": "source",
                        "start_dir": root,
                        "selection": "file",
                        "allow_multiple": false,
                        "show_hidden": false,
                        "query": format!("{}-advertised", root.display()),
                        "output_policy": {"relative_to": root},
                    }},
                }],
                "inputs": inputs,
            },
        },
        "path_suggestions": {
            "kind": "path_suggestions",
            "fields": {
                "expected": {"request": {
                    "value": format!("{}-request", root.display()),
                    "context": {
                        "workdir": root,
                        "tokens": {
                            "cwd": root,
                            "home": root,
                            "env": {"ROOT": root},
                        },
                    },
                }},
                "visible": {
                    "value": format!("{}-visible-value", root.display()),
                    "suggestion": root.join("suggestion"),
                },
            },
        },
        "run_modal": {
            "kind": "run_modal",
            "fields": {
                "signature": {"file": {
                    "context": {"workdir": root, "invoke_cwd": root},
                }},
                "file": picker.clone(),
                "file_picker_source": picker["fields"]["memory_source"].clone(),
            },
        },
        "add_overlay": {
            "kind": "add_file_overlay",
            "fields": {"session": picker.clone(), "geometry": null},
        },
        "file_picker_source": picker["fields"]["memory_source"].clone(),
    })
}

fn directory_seed(root: WalkerDirectoryRoot, path: &str) -> WalkerDirectorySeed {
    WalkerDirectorySeed {
        root,
        path: PathBuf::from(path),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum G2DirectorySeedFault {
    FileConflict,
    #[cfg(unix)]
    SymlinkConflict,
    Create,
    Inspect,
    PrivateMode,
}

#[derive(Debug)]
struct G2DirectorySeedIo {
    fault: Cell<Option<G2DirectorySeedFault>>,
    attempts: RefCell<Vec<PathBuf>>,
    symlink_target: PathBuf,
}

impl G2DirectorySeedIo {
    fn new(fault: G2DirectorySeedFault, symlink_target: PathBuf) -> Self {
        Self {
            fault: Cell::new(Some(fault)),
            attempts: RefCell::new(Vec::new()),
            symlink_target,
        }
    }

    fn take(&self, fault: G2DirectorySeedFault) -> bool {
        if self.fault.get() != Some(fault) {
            return false;
        }
        self.fault.set(None);
        true
    }
}

impl DirectorySeedIo for G2DirectorySeedIo {
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        self.attempts.borrow_mut().push(path.to_path_buf());
        if self.take(G2DirectorySeedFault::FileConflict) {
            fs::write(path, b"seed conflict")?;
        }
        #[cfg(unix)]
        if self.take(G2DirectorySeedFault::SymlinkConflict) {
            std::os::unix::fs::symlink(&self.symlink_target, path)?;
        }
        if self.take(G2DirectorySeedFault::Create) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected directory seed create failure",
            ));
        }
        if self.fault.get() == Some(G2DirectorySeedFault::Inspect) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        fs::create_dir(path)
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
        if self.take(G2DirectorySeedFault::Inspect) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "injected directory seed inspect failure",
            ));
        }
        fs::symlink_metadata(path)
    }

    fn set_private_directory_mode(&self, path: &Path) -> io::Result<()> {
        assert!(self.take(G2DirectorySeedFault::PrivateMode));
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "injected directory seed private-mode failure",
        ))
    }
}
