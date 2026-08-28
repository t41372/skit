use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use skit_application::{
    AgentScope, AgentTarget, LibraryScan, SourceIdentity, SourcePermissions,
    health::{HealthIssue, HealthIssueKind, HealthSnapshot, MirrorHealth, UvHealth},
    preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
        PreferencesSnapshot,
    },
    tokens::TokenContext,
};
use skit_domain::{
    EntryKind, EntrySummary, Slug, StorageMode,
    parameters::{ParamDecl, ParameterDelivery, ParameterValue},
};
use skit_ui::{
    DependencyFlavor, DraftSummary, LibraryEntryDetail, LibraryParameterDetail,
    LibraryPromptRunner, LibrarySurface, RunFormContext, RunPathContext, RunnerRow,
    RunnerRowIdentity, SettingsInputs, SourceSnapshot,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EntryFixture {
    pub(super) summary: EntrySummary,
    pub(super) detail: LibraryEntryDetail,
    pub(super) declarations: Vec<ParamDecl>,
    pub(super) presets: BTreeMap<String, BTreeMap<String, String>>,
    pub(super) settings: SettingsInputs,
    pub(super) normalized: BTreeSet<String>,
    pub(super) resync_count: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FixtureSet {
    pub(super) entries: Vec<EntryFixture>,
    pub(super) kept_drafts: Vec<DraftSummary>,
    pub(super) sources: BTreeMap<PathBuf, SourceSnapshot>,
    pub(super) runners: Vec<RunnerRow>,
    pub(super) health: HealthSnapshot,
    pub(super) preferences: PreferencesSnapshot,
    pub(super) agent_targets: Vec<AgentTarget>,
    pub(super) virtual_directories: BTreeSet<PathBuf>,
    pub(super) virtual_paths: BTreeSet<PathBuf>,
    pub(super) virtual_files: BTreeSet<PathBuf>,
}

pub(super) fn fixture_set() -> FixtureSet {
    let python = python_entry();
    let prompt = prompt_entry();
    let command = command_entry();
    let python_source = source_snapshot(
        "/fixtures/python_tool.py",
        b"#!/usr/bin/env python3\nNAME = \"World\"\nTOKEN = \"\"\nOLD = \"\"\nprint(NAME)\n",
        false,
        8,
    );
    let mut prompt_source = source_snapshot(
        "/fixtures/prompt.md",
        b"Write about {{TOPIC}} for {{AUDIENCE}}.\n",
        false,
        9,
    );
    prompt_source.permissions.unix_mode = Some(0o600);
    prompt_source.executable = Some(false);
    let new_source = source_snapshot(
        "/fixtures/new.py",
        b"#!/usr/bin/env python3\nprint('new')\n",
        false,
        10,
    );
    let draft_source = source_snapshot(
        "/fixtures/drafts/skit-new-kept.py",
        b"#!/usr/bin/env python3\nprint('draft')\n",
        true,
        11,
    );
    let kept_draft = draft_summary(&draft_source);
    let runners = runner_rows();
    let entries = vec![python, prompt, command];
    FixtureSet {
        health: health_snapshot(&entries, &runners),
        preferences: preferences_snapshot(&runners),
        entries,
        kept_drafts: vec![kept_draft],
        sources: BTreeMap::from([
            (python_source.path.clone(), python_source),
            (prompt_source.path.clone(), prompt_source),
            (new_source.path.clone(), new_source),
            (draft_source.path.clone(), draft_source),
        ]),
        runners,
        agent_targets: vec![AgentTarget {
            name: "codex".to_owned(),
            scope: AgentScope::Project,
            base: PathBuf::from("/fixtures/project/.codex"),
        }],
        virtual_directories: picker_directories(),
        virtual_paths: virtual_paths(),
        virtual_files: BTreeSet::from([
            PathBuf::from("/fixtures/bin/bash"),
            PathBuf::from("/fixtures/bin/bash.exe"),
            PathBuf::from("/fixtures/bin/uv"),
        ]),
    }
}

pub(super) fn surface(entries: impl Iterator<Item = EntryFixture>) -> LibrarySurface {
    let entries = entries.collect::<Vec<_>>();
    LibrarySurface {
        scan: LibraryScan {
            entries: entries.iter().map(|entry| entry.summary.clone()).collect(),
            diagnostics: Vec::new(),
        },
        details: entries
            .into_iter()
            .map(|entry| (entry.summary.slug, entry.detail))
            .collect(),
    }
}

pub(super) fn run_context(kind: &str) -> RunFormContext {
    let workdir = "/fixtures/work".to_owned();
    let invoke_cwd = "/fixtures/invoke".to_owned();
    RunFormContext {
        entry_kind: kind.to_owned(),
        path: Some(RunPathContext {
            workdir,
            invoke_cwd: invoke_cwd.clone(),
        }),
        tokens: TokenContext {
            cwd: invoke_cwd,
            home: Some("/fixtures/home".to_owned()),
            env: BTreeMap::from([
                ("HOME".to_owned(), "/fixtures/home".to_owned()),
                ("TOKEN".to_owned(), "fixture-secret".to_owned()),
            ]),
            today: "2026-08-27".to_owned(),
            now: "12-34-56".to_owned(),
        },
    }
}

fn python_entry() -> EntryFixture {
    let mut name = ParamDecl::new("NAME");
    name.default = Some(ParameterValue::String("World".to_owned()));
    name.prompt = "Name".to_owned();
    let mut token = ParamDecl::new("TOKEN");
    token.delivery = ParameterDelivery::Env;
    token.secret = true;
    token.env_source = "TOKEN".to_owned();
    let old = ParamDecl::new("OLD");
    let declarations = vec![name, token, old];
    let presets = BTreeMap::from([(
        "friendly".to_owned(),
        BTreeMap::from([("NAME".to_owned(), "Ada".to_owned())]),
    )]);
    let summary = summary(
        "python-tool",
        "Python tool",
        "python",
        StorageMode::Copy,
        "A parameter fixture",
    );
    let detail = LibraryEntryDetail {
        added_at: "2026-08-20T12:00:00Z".to_owned(),
        parameters: vec![
            LibraryParameterDetail {
                key: "NAME".to_owned(),
                value: "World".to_owned(),
                secret: false,
            },
            LibraryParameterDetail {
                key: "TOKEN".to_owned(),
                value: String::new(),
                secret: true,
            },
            LibraryParameterDetail {
                key: "OLD".to_owned(),
                value: String::new(),
                secret: false,
            },
        ],
        presets: vec!["friendly".to_owned()],
        dependencies: vec!["requests>=2".to_owned()],
        last_run: Some(skit_ui::LibraryLastRun {
            at: "2026-08-27T12:00:00Z".to_owned(),
            age: skit_ui::LibraryRunAge::JustNow,
            exit: Some(0),
        }),
        original_file_preserved: true,
        ..LibraryEntryDetail::default()
    };
    EntryFixture {
        settings: SettingsInputs {
            selector: summary.slug.as_str().to_owned(),
            kind: "python".to_owned(),
            name: summary.name.clone(),
            description: summary.description.clone(),
            source: "/fixtures/python_tool.py".to_owned(),
            workdir: "/fixtures/work".to_owned(),
            supports_modes: true,
            has_original_file: true,
            has_stored_name: true,
            pinnable_interpreter: false,
            has_analyzer: true,
            managed: declarations.clone(),
            candidates: vec!["SOURCE_CONST".to_owned()],
            dependency_flavor: Some(DependencyFlavor::Uv),
            effective_dependencies: vec!["requests>=2".to_owned()],
            configured_runners: vec!["codex".to_owned()],
            presets: presets.clone(),
            ..SettingsInputs::default()
        },
        summary,
        detail,
        declarations,
        presets,
        normalized: BTreeSet::new(),
        resync_count: 0,
    }
}

fn prompt_entry() -> EntryFixture {
    let mut topic = ParamDecl::new("TOPIC");
    topic.delivery = ParameterDelivery::Placeholder;
    topic.default = Some(ParameterValue::String("Rust".to_owned()));
    let declarations = vec![topic];
    let summary = summary(
        "prompt-tool",
        "Prompt tool",
        "prompt",
        StorageMode::Copy,
        "A prompt fixture",
    );
    let detail = LibraryEntryDetail {
        added_at: "2026-08-19T12:00:00Z".to_owned(),
        prompt_runner: Some(LibraryPromptRunner::Configured("codex".to_owned())),
        parameters: vec![LibraryParameterDetail {
            key: "TOPIC".to_owned(),
            value: "Rust".to_owned(),
            secret: false,
        }],
        original_file_preserved: true,
        ..LibraryEntryDetail::default()
    };
    EntryFixture {
        settings: SettingsInputs {
            selector: summary.slug.as_str().to_owned(),
            kind: "prompt".to_owned(),
            name: summary.name.clone(),
            description: summary.description.clone(),
            source: "/fixtures/prompt.md".to_owned(),
            workdir: "/fixtures/work".to_owned(),
            runner: "codex".to_owned(),
            supports_modes: true,
            has_original_file: true,
            has_stored_name: true,
            declared_schema: true,
            managed: declarations.clone(),
            candidates: vec!["AUDIENCE".to_owned()],
            interpolate: true,
            configured_runners: vec!["codex".to_owned()],
            ..SettingsInputs::default()
        },
        summary,
        detail,
        declarations,
        presets: BTreeMap::new(),
        normalized: BTreeSet::new(),
        resync_count: 0,
    }
}

fn command_entry() -> EntryFixture {
    let mut target = ParamDecl::new("TARGET");
    target.delivery = ParameterDelivery::Placeholder;
    target.default = Some(ParameterValue::String("A".to_owned()));
    let declarations = vec![target];
    let summary = summary(
        "command-tool",
        "Command tool",
        "command",
        StorageMode::Copy,
        "A command fixture",
    );
    let detail = LibraryEntryDetail {
        added_at: "2026-08-18T12:00:00Z".to_owned(),
        template: Some("printf '%s\\n' {TARGET}".to_owned()),
        parameters: vec![LibraryParameterDetail {
            key: "TARGET".to_owned(),
            value: "A".to_owned(),
            secret: false,
        }],
        ..LibraryEntryDetail::default()
    };
    EntryFixture {
        settings: SettingsInputs {
            selector: summary.slug.as_str().to_owned(),
            kind: "command".to_owned(),
            name: summary.name.clone(),
            description: summary.description.clone(),
            workdir: "/fixtures/work".to_owned(),
            template: "printf '%s\\n' {TARGET}".to_owned(),
            declared_schema: true,
            managed: declarations.clone(),
            needs: vec!["fixture-command".to_owned()],
            ..SettingsInputs::default()
        },
        summary,
        detail,
        declarations,
        presets: BTreeMap::new(),
        normalized: BTreeSet::new(),
        resync_count: 0,
    }
}

fn summary(
    slug: &str,
    name: &str,
    kind: &str,
    mode: StorageMode,
    description: &str,
) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).expect("fixture slug is valid"),
        name: name.to_owned(),
        kind: EntryKind::parse(kind).expect("fixture kind is valid"),
        mode,
        description: description.to_owned(),
        target: None,
    }
}

pub(super) fn draft_summary(source: &SourceSnapshot) -> DraftSummary {
    DraftSummary {
        path: source.path.clone(),
        modified: content_stamp(&source.bytes),
        identity: source.identity.clone(),
        permissions: source.permissions,
        content_hash: Some(format!("fixture:{:016x}", content_stamp(&source.bytes))),
    }
}

fn source_snapshot(path: &str, bytes: &[u8], is_draft: bool, inode: u64) -> SourceSnapshot {
    SourceSnapshot {
        path: PathBuf::from(path),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions {
            readonly: false,
            unix_mode: Some(0o700),
        },
        executable: Some(true),
        is_regular: true,
        is_directory: false,
        is_draft,
        identity: Some(SourceIdentity::unix(7, inode, 1_776_981_600, 0)),
    }
}

fn content_stamp(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        hash.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(*byte)
    })
}

fn virtual_paths() -> BTreeSet<PathBuf> {
    let root = PathBuf::from("/fixtures/invoke");
    [
        "alpha.py",
        "beta.py",
        "notes.txt",
        "input.json",
        ".hidden.py",
        "nested",
        "nested/gamma.py",
        "nested/.secret.py",
        "unicodé-a.rs",
    ]
    .into_iter()
    .map(|relative| root.join(relative))
    .collect()
}

fn picker_directories() -> BTreeSet<PathBuf> {
    [
        "/fixtures",
        "/fixtures/bin",
        "/fixtures/drafts",
        "/fixtures/home",
        "/fixtures/invoke",
        "/fixtures/invoke/nested",
        "/fixtures/library",
        "/fixtures/library/command-tool",
        "/fixtures/library/prompt-tool",
        "/fixtures/library/python-tool",
        "/fixtures/project",
        "/fixtures/work",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

fn runner_rows() -> Vec<RunnerRow> {
    let valid = RunnerRowIdentity {
        index: Some(0),
        snapshot_token: concat!(
            "row:0:name=Some(\"codex\"):argv=Some([\"codex\", \"exec\", ",
            "\"{{prompt}}\"]):reason=None:descriptor=\"codex\""
        )
        .to_owned(),
    };
    vec![
        RunnerRow {
            identity: valid.clone(),
            name: Some("codex".to_owned()),
            argv: Some(vec![
                "codex".to_owned(),
                "exec".to_owned(),
                "{{prompt}}".to_owned(),
            ]),
            reason: None,
            descriptor: "codex".to_owned(),
            key_identities: vec![valid],
            pinned_count: 1,
        },
        RunnerRow {
            identity: RunnerRowIdentity {
                index: Some(1),
                snapshot_token: concat!(
                    "row:1:name=None:argv=Some([\"broken\", \"{{prompt}}\"]):",
                    "reason=Some(\"name_missing\"):descriptor=\"malformed row 1\""
                )
                .to_owned(),
            },
            name: None,
            argv: Some(vec!["broken".to_owned(), "{{prompt}}".to_owned()]),
            reason: Some("name_missing".to_owned()),
            descriptor: "malformed row 1".to_owned(),
            key_identities: Vec::new(),
            pinned_count: 0,
        },
    ]
}

fn preferences_snapshot(runners: &[RunnerRow]) -> PreferencesSnapshot {
    PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
        effective_language: "en".to_owned(),
        editor: String::new(),
        editor_fallback: Some("vi".to_owned()),
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Stay,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runner_names: runners.iter().filter_map(|row| row.name.clone()).collect(),
        mirror: MirrorConfiguration::default(),
    }
}

fn health_snapshot(entries: &[EntryFixture], runners: &[RunnerRow]) -> HealthSnapshot {
    HealthSnapshot {
        uv: UvHealth::Found("/fixtures/bin/uv".to_owned()),
        entry_count: entries.len(),
        issues: vec![HealthIssue {
            slug: "command-tool".to_owned(),
            name: "Command tool".to_owned(),
            kind: HealthIssueKind::MissingNeeds {
                tools: vec!["fixture-command".to_owned()],
            },
        }],
        invalid_runner_rows: runners
            .iter()
            .filter_map(|row| row.reason.clone())
            .collect(),
        mirror: MirrorHealth::Off,
        library_path: "/fixtures/library".to_owned(),
        library_size: "12.0 KB".to_owned(),
        diagnostics: Vec::new(),
    }
}
