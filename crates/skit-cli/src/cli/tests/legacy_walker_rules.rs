//! Host and reducer rules the retired `FakeHost` walker was the only owner of.
//!
//! Every case here drives the production host over a real `TempDir` library, so the rule is proved
//! by the same code the composition root runs.
//!
//! Two halves of the legacy rows already have owners and stay there: the paused-mirror health
//! projection is `tests.rs shared_health_inspector_reports_typed_entry_runner_and_rebuild_facts`,
//! and the refusal of a preset save for an entry with no form fields is
//! `tests.rs typed_run_feedback_and_preset_effects_use_application_and_store_ports`.

use std::path::PathBuf;

use super::*;

/// One real library: its product roots and the adapters the composition root builds over them.
struct RealLibrary {
    root: TempDir,
    state_dir: PathBuf,
    config_dir: PathBuf,
    store: FileStore,
    service: LibraryService<FileStore>,
}

impl RealLibrary {
    /// Lay out `<root>/data`, `<root>/state` and `<root>/config` the way the composition root does.
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let data_dir = root.path().join("data");
        let state_dir = root.path().join("state");
        let config_dir = root.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();
        let store = FileStore::new(&data_dir);
        let service = LibraryService::new(store.clone());
        Self {
            root,
            state_dir,
            config_dir,
            store,
            service,
        }
    }
}

/// Every kind the library launches, with the three axes the settings screen reads:
/// `(kind, has_analyzer, has_original_file, declared_schema)`.
///
/// Only a kind that keeps its parameter schema inside the user's own file has an analyzer. Every
/// other kind declares its schema in the entry metadata. A command template is the one kind with
/// no file of its own, so it also has no original file to preserve.
const ANALYZER_KINDS: [(&str, bool, bool, bool); 13] = [
    ("python", true, true, false),
    ("shell", true, true, false),
    ("fish", true, true, false),
    ("js", true, true, false),
    ("ts", true, true, false),
    ("powershell", false, true, true),
    ("ruby", false, true, true),
    ("perl", false, true, true),
    ("lua", false, true, true),
    ("r", false, true, true),
    ("exe", false, true, true),
    ("prompt", false, true, true),
    ("command", false, false, true),
];

#[test]
fn every_entry_kind_pins_its_analyzer_schema_and_original_file_axes() {
    let library = RealLibrary::new();
    for (kind, _, _, _) in ANALYZER_KINDS {
        let original = library.root.path().join(format!("original-{kind}"));
        fs::write(&original, b"# kind contract\n").unwrap();
        let template = if kind == "command" { "echo ok" } else { "" };
        let entry_kind = EntryKind::parse(kind).unwrap();
        let payload = (kind != "command").then(|| EntryPayload {
            bytes: b"# kind contract\n".to_vec(),
            stored_name: Some(payload_stored_name(&entry_kind, &original)),
            permissions: SourcePermissions::default(),
        });
        let source = if kind == "command" {
            String::new()
        } else {
            original.display().to_string()
        };
        library
            .service
            .add(CreateEntry {
                name: format!("Kind {kind}"),
                kind: entry_kind,
                mode: if kind == "command" {
                    StorageMode::Reference
                } else {
                    StorageMode::Copy
                },
                source,
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload,
                settings: EntrySettings {
                    template: template.to_owned(),
                    ..EntrySettings::default()
                },
            })
            .unwrap();
    }

    let surface =
        crate::library_surface(&library.store, &library.state_dir, &library.config_dir).unwrap();
    for (kind, has_analyzer, has_original_file, declared_schema) in ANALYZER_KINDS {
        let entry = library.service.show(&format!("Kind {kind}")).unwrap();
        let inputs = settings_inputs(
            &library.service,
            &library.store,
            &library.config_dir,
            &library.state_dir,
            &entry,
            None,
        )
        .unwrap();
        let detail = surface.details.get(&entry.slug).expect("no detail row");
        assert_eq!(inputs.has_analyzer, has_analyzer, "{kind}");
        assert_eq!(inputs.has_original_file, has_original_file, "{kind}");
        assert_eq!(inputs.declared_schema, declared_schema, "{kind}");
        // A kind that has an original file preserves it.
        assert_eq!(detail.original_file_preserved, has_original_file, "{kind}");
    }
}

/// How many host effects one user action may need.
///
/// This mirrors the private `EFFECT_LIMIT` of `tui_real_walker/corpus.rs`, which that file
/// compiles only on Linux and Windows. No public seam carries the value, so the two constants stay equal
/// by this comment.
const LEGACY_EFFECT_LIMIT: usize = 16;

/// One real library and the frontend state a session holds over it.
struct RealSession {
    library: RealLibrary,
    state: LibraryState,
}

impl RealSession {
    /// Take the state the composition root starts a session with.
    fn start(library: RealLibrary) -> Self {
        let state = TuiHost::system(&library.service, &library.state_dir, &library.config_dir)
            .initial_state()
            .unwrap();
        Self { library, state }
    }

    /// Serve one effect through the production host and give the reducer its answer.
    ///
    /// Each call builds its own host, the way the composition root builds one per session. A host
    /// caches no declarations and reads every projection from the store, and its only mutable
    /// state is the locale cell a preferences save sets on that one instance.
    fn round_trip(&mut self, effect: UiEffect) -> UiEffect {
        let action = TuiHost::system(
            &self.library.service,
            &self.library.state_dir,
            &self.library.config_dir,
        )
        .serve(effect)
        .expect("the host serves the effect");
        self.state.update(action)
    }

    /// Serve every follow-up effect until the host has nothing left to answer.
    ///
    /// One user action reaches the host at most `LEGACY_EFFECT_LIMIT` times. The engine runs that
    /// many rounds and then reads the last answer, so an action that settles on its final round
    /// is a settle, not a loop (`skit-tui-walker-support/src/engine.rs:515-555`).
    fn settle(&mut self, effect: UiEffect) {
        let mut effect = effect;
        for _ in 0..LEGACY_EFFECT_LIMIT {
            if effect == UiEffect::None {
                return;
            }
            effect = self.round_trip(effect);
        }
        assert_eq!(
            effect,
            UiEffect::None,
            "one action asked the host for more than {LEGACY_EFFECT_LIMIT} effects"
        );
    }

    /// Put the library cursor on one entry.
    fn select(&mut self, selector: &str) {
        let index = self
            .state
            .visible_entries()
            .position(|entry| entry.slug.as_str() == selector)
            .expect("the library shows the entry");
        assert_eq!(
            self.state.update(UiAction::SelectVisible(index)),
            UiEffect::None
        );
    }

    /// Open one entry's run form the way the Library screen does.
    fn open_run(&mut self, selector: &str) {
        self.select(selector);
        let open = self.state.update(UiAction::OpenRun);
        self.settle(open);
    }

    /// Open one entry's settings screen the way the Library screen does.
    fn open_settings(&mut self, selector: &str) {
        self.select(selector);
        let open = self.state.update(UiAction::OpenSettings);
        self.settle(open);
    }

    /// Find one run-form field by key.
    fn run_field(&self, key: &str) -> usize {
        self.state
            .run_form()
            .expect("the run form is on screen")
            .fields()
            .iter()
            .position(|field| field.key == key)
            .unwrap_or_else(|| panic!("the run form has no {key} field"))
    }

    /// Read one run-form field's current value.
    fn run_value(&self, key: &str) -> String {
        self.state
            .run_form()
            .expect("the run form is on screen")
            .fields()[self.run_field(key)]
        .control
        .value()
    }

    /// Move one settings control and save.
    fn save_settings(&mut self, selector: &str, edits: &[(&str, FieldValue)]) {
        self.open_settings(selector);
        for (key, value) in edits {
            assert_eq!(
                self.state
                    .update(UiAction::Settings(skit_ui::SettingsAction::SetField {
                        key: (*key).to_owned(),
                        value: value.clone(),
                    })),
                UiEffect::None
            );
        }
        let save = self
            .state
            .update(UiAction::Settings(skit_ui::SettingsAction::Save));
        self.settle(save);
    }
}

#[test]
fn one_entry_reports_its_missing_needs_instead_of_its_launch_block() {
    let library = RealLibrary::new();
    // A Ruby entry whose interpreter is not installed cannot launch.
    let blocked = EntrySettings {
        interpreter: "skit-interpreter-that-does-not-exist".to_owned(),
        ..EntrySettings::default()
    };
    let needy = EntrySettings {
        needs: vec!["skit-tool-that-does-not-exist".to_owned()],
        ..blocked.clone()
    };
    for (name, settings) in [("Blocked", blocked), ("Blocked and needy", needy)] {
        library
            .service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("ruby").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "store".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"puts 1\n".to_vec(),
                    stored_name: Some("script.rb".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings,
            })
            .unwrap();
    }

    let snapshot = CliHealthInspector::new(
        &library.service,
        &library.store,
        &library.config_dir,
        Locale::En,
        &SystemProbe,
    )
    .inspect()
    .unwrap();

    let issues = |name: &str| {
        snapshot
            .issues
            .iter()
            .filter(|issue| issue.name == name)
            .map(|issue| issue.kind.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        issues("Blocked"),
        [HealthIssueKind::LaunchBlocked {
            reason: "required program was not found: skit-interpreter-that-does-not-exist"
                .to_owned(),
        }]
    );
    // A missing need outranks the launch block, so the same entry reports one issue, not two.
    assert_eq!(
        issues("Blocked and needy"),
        [HealthIssueKind::MissingNeeds {
            tools: vec!["skit-tool-that-does-not-exist".to_owned()],
        }]
    );
}

#[test]
fn an_ordered_add_effect_list_returns_at_its_first_terminal_effect() {
    let library = RealLibrary::new();
    let source = library.root.path().join("terminal.py");
    fs::write(&source, "print(1)\n").unwrap();
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(source.display().to_string()));
    let inspect = workflow.reduce(AddAction::Continue);
    let UiAction::Add(AddAction::SourceInspected { request, result }) = tui_add_effect(
        &library.service,
        &library.store,
        &library.state_dir,
        &library.config_dir,
        inspect,
    )
    .unwrap() else {
        panic!("source inspection must return through the typed reducer");
    };
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(result.unwrap()),
    });
    let _ = workflow.reduce(AddAction::SetReviewName("Added by host".to_owned()));
    let mut effects = workflow.reduce(AddAction::Save);
    let commit = effects.pop().expect("the save must emit a commit");
    assert!(effects.is_empty(), "the reviewed save emits only a commit");
    let AddEffect::Commit { request, .. } = &commit else {
        panic!("a reviewed file save must commit: {commit:?}");
    };
    let request = *request;

    let action = tui_add_effect(
        &library.service,
        &library.store,
        &library.state_dir,
        &library.config_dir,
        vec![
            AddEffect::RememberRunner("codex".to_owned()),
            commit,
            AddEffect::Complete("must-not-run".to_owned()),
        ],
    )
    .unwrap();

    // The commit is terminal: its own answer returns and the trailing completion never runs.
    let UiAction::Add(AddAction::CommitFinished {
        request: answered,
        result,
    }) = action
    else {
        panic!("the terminal effect must answer the list: {action:?}");
    };
    assert_eq!(answered, request);
    assert_eq!(result.unwrap(), "added-by-host");
    // The prefix still applied before the terminal effect returned.
    assert_eq!(
        PromptSelectionService::new(FilePromptSelectionStore::new(&library.state_dir))
            .last_runner(),
        "codex"
    );
    // The list stops at the commit. `Complete` would have answered `AddCompleted`, and the
    // let-else above refuses that shape.
    assert_eq!(
        library
            .service
            .list()
            .unwrap()
            .entries
            .iter()
            .map(|summary| summary.slug.as_str().to_owned())
            .collect::<Vec<_>>(),
        ["added-by-host"]
    );
}

#[test]
fn a_refused_python_settings_save_rolls_back_its_staged_secret_scrub() {
    let library = RealLibrary::new();
    let mut city = ParamDecl::new("CITY");
    city.binding = ParameterBinding::Const;
    city.delivery = ParameterDelivery::Inject;
    let source =
        write_managed_params("python", "CITY = \"Taipei\"\nprint(CITY)\n", &[city]).unwrap();
    let entry = library
        .service
        .add(CreateEntry {
            name: "Rollback".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: source.into_bytes(),
                stored_name: Some("script.py".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let state_path = library
        .state_dir
        .join("values")
        .join(format!("{}.toml", entry.slug.as_str()));
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    fs::write(&state_path, "[values]\nCITY = \"plaintext\"\n").unwrap();
    let state_before = fs::read(&state_path).unwrap();
    let meta_before = fs::read(
        library
            .store
            .data_dir()
            .join("scripts")
            .join(entry.slug.as_str())
            .join("meta.toml"),
    )
    .unwrap();

    // A copy-mode Python entry runs under uv, so the screen offers no interpreter pin at all.
    let view = settings_view(
        &library.service,
        &library.store,
        &library.state_dir,
        entry.slug.as_str(),
    );
    assert!(view.field(skit_ui::INTERPRETER_KEY).is_none());

    // The refusal names the value the person typed and the form the axis wanted.
    for (key, invalid, refusal) in [
        (
            skit_ui::DEPENDENCIES_KEY,
            "@@@",
            "@@@ isn't a package requirement (e.g. \"requests\" or \"rich>=13,<16\").",
        ),
        (
            skit_ui::PYTHON_KEY,
            "not-a-version",
            "not-a-version isn't a Python version constraint (e.g. \">=3.11\" or \">=3.12,<3.13\").",
        ),
    ] {
        let mut values = settings_edits(
            &library.service,
            &library.store,
            &library.state_dir,
            entry.slug.as_str(),
            &[("parameter:CITY:secret", "true")],
        );
        set(&mut values, key, invalid);
        let tree_before = test_tree_snapshot(library.root.path());

        let error = tui_submit_settings(
            &library.service,
            &library.store,
            &library.state_dir,
            entry.slug.as_str(),
            &values,
        )
        .unwrap_err();

        assert!(matches!(error, CliError::Usage(_)), "{key}: {error:?}");
        assert_eq!(error.message().localize(Locale::En), refusal, "{key}");
        // The scrub is staged before the source axis validates, so the refusal must return it.
        assert_eq!(fs::read(&state_path).unwrap(), state_before, "{key}");
        assert_eq!(
            fs::read(
                library
                    .store
                    .data_dir()
                    .join("scripts")
                    .join(entry.slug.as_str())
                    .join("meta.toml")
            )
            .unwrap(),
            meta_before,
            "{key}"
        );
        // The rollback covers the whole product tree, not only the scrubbed value file: every
        // file under the data, state and config roots keeps its bytes.
        assert_eq!(
            test_tree_snapshot(library.root.path()),
            tree_before,
            "{key}"
        );
    }
}

#[test]
fn a_command_template_declares_its_placeholders_in_body_order_before_an_added_rider() {
    let library = RealLibrary::new();
    let entry = add_command(&library.service, "Ordered", "echo {TARGET}");
    let values = settings_edits(
        &library.service,
        &library.store,
        &library.state_dir,
        entry.slug.as_str(),
        &[
            ("template", "echo {SECOND} {TARGET}"),
            ("parameter:add", "EXTRA"),
        ],
    );

    tui_submit_settings(
        &library.service,
        &library.store,
        &library.state_dir,
        entry.slug.as_str(),
        &values,
    )
    .unwrap();

    let saved = library.service.show(entry.slug.as_str()).unwrap();
    assert_eq!(
        entry_parameters(&library.store, &saved)
            .iter()
            .map(|parameter| (parameter.name.clone(), parameter.delivery))
            .collect::<Vec<_>>(),
        [
            ("SECOND".to_owned(), ParameterDelivery::Placeholder),
            ("TARGET".to_owned(), ParameterDelivery::Placeholder),
            ("EXTRA".to_owned(), ParameterDelivery::Env),
        ]
    );
    let surface =
        crate::library_surface(&library.store, &library.state_dir, &library.config_dir).unwrap();
    assert_eq!(
        surface.details[&saved.slug]
            .parameters
            .iter()
            .map(|parameter| parameter.key.clone())
            .collect::<Vec<_>>(),
        ["SECOND", "TARGET", "EXTRA"]
    );
}

/// Configure one prompt runner that launches without leaving the temporary tree.
///
/// The program must exist on every platform the test matrix runs, and it must take the prompt as
/// plain text. `printf` would read the prompt as a format string. Windows has no `echo` program,
/// so the runner goes through the command interpreter there.
fn seed_runner(library: &RealLibrary, name: &str) {
    let argv = if cfg!(windows) {
        vec![
            "cmd.exe".to_owned(),
            "/C".to_owned(),
            "echo".to_owned(),
            "{{prompt}}".to_owned(),
        ]
    } else {
        vec!["echo".to_owned(), "{{prompt}}".to_owned()]
    };
    FileConfigStore::new(&library.config_dir)
        .set_runner(
            skit_store::PromptRunner {
                name: name.to_owned(),
                argv,
            },
            true,
        )
        .unwrap();
}

/// Add one prompt entry whose body carries the given text.
fn seed_prompt(library: &RealLibrary, name: &str, body: &str, settings: EntrySettings) -> Entry {
    library
        .service
        .add(CreateEntry {
            name: name.to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: body.as_bytes().to_vec(),
                stored_name: Some("prompt.md".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings,
        })
        .unwrap()
}

#[test]
fn a_preset_save_uses_the_declaration_schema_as_of_the_save() {
    let library = RealLibrary::new();
    let entry = add_command(&library.service, "Schema race", "echo {NAME}{TOKEN}{OLD}");
    let values = settings_edits(
        &library.service,
        &library.store,
        &library.state_dir,
        entry.slug.as_str(),
        &[("parameter:TOKEN:secret", "true")],
    );
    tui_submit_settings(
        &library.service,
        &library.store,
        &library.state_dir,
        entry.slug.as_str(),
        &values,
    )
    .unwrap();

    let mut session = RealSession::start(library);
    session.open_run(entry.slug.as_str());
    for (key, value) in [("value:NAME", "Ada"), ("value:TOKEN", "secret")] {
        let field = session.run_field(key);
        assert_eq!(
            session.state.update(UiAction::SetFieldValue {
                field,
                value: value.to_owned(),
            }),
            UiEffect::None
        );
    }
    assert_eq!(
        session.state.update(UiAction::OpenRunPresetSave),
        UiEffect::None
    );
    assert_eq!(
        session
            .state
            .update(UiAction::SetModalInput("schema-race".to_owned())),
        UiEffect::None
    );
    let save = session.state.update(UiAction::Submit);
    let UiEffect::SaveRunPreset {
        selector: ref saved_selector,
        name: ref preset,
        ref values,
        ref secret_names,
    } = save
    else {
        panic!("a preset save must use the host form contract: {save:?}");
    };
    assert_eq!(saved_selector.as_str(), entry.slug.as_str());
    assert_eq!(preset, "schema-race");
    // The schema at the open marks TOKEN secret, so the form withholds the value the person typed.
    assert_eq!(
        values,
        &BTreeMap::from([
            ("NAME".to_owned(), "Ada".to_owned()),
            ("OLD".to_owned(), String::new()),
        ])
    );
    assert_eq!(
        secret_names,
        &std::collections::BTreeSet::from(["TOKEN".to_owned()])
    );

    // The schema moves between the open and the save.
    let flip = settings_edits(
        &session.library.service,
        &session.library.store,
        &session.library.state_dir,
        entry.slug.as_str(),
        &[
            ("parameter:NAME:secret", "true"),
            ("parameter:TOKEN:secret", "false"),
        ],
    );
    tui_submit_settings(
        &session.library.service,
        &session.library.store,
        &session.library.state_dir,
        entry.slug.as_str(),
        &flip,
    )
    .unwrap();

    session.settle(save);

    // The save reads the current schema, so the newly secret name is dropped and only the public
    // parameter that was public in both schemas is written.
    assert_eq!(
        FormStateService::new(FileFormStateStore::new(&session.library.state_dir))
            .load(&entry.slug)
            .presets["schema-race"],
        BTreeMap::from([("OLD".to_owned(), String::new())])
    );
}

/// Help text the prompt body does not carry.
///
/// This is the discriminator of the interpolation rule. A save that keeps the stored schema keeps
/// this line. A save that re-plans the body and writes the plan loses it.
const PROMPT_HELP: &str = "Sentinel help the body does not carry";

/// The parameter schema of the interpolation fixture, exactly as `meta.toml` holds it.
///
/// `TOPIC` is declared and carries help text. `AUDIENCE` is in the body list but has no
/// declaration table, so a table for it is the same regression as a lost help line.
const STORED_PROMPT_SCHEMA: [&str; 10] = [
    "params = [",
    "    \"TOPIC\",",
    "    \"AUDIENCE\",",
    "]",
    "[[parameters]]",
    "delivery = \"placeholder\"",
    "help = \"Sentinel help the body does not carry\"",
    "name = \"TOPIC\"",
    "required = true",
    "type = \"str\"",
];

/// Read the stored parameter schema out of the raw `meta.toml` bytes.
///
/// The schema is the `params` list, which the writer can spread over several lines, and every
/// `[[parameters]]` table. A table ends at the next table header. The test reads the bytes on
/// disk. A save that keeps the schema in memory and drops it from the file is the data loss this
/// rule guards.
fn stored_parameter_lines(meta: &[u8]) -> Vec<String> {
    let text = std::str::from_utf8(meta).expect("meta.toml holds UTF-8");
    let mut lines = Vec::new();
    let mut inside_table = false;
    let mut inside_list = false;
    for line in text.lines().filter(|line| !line.is_empty()) {
        if inside_list {
            lines.push(line.to_owned());
            inside_list = line != "]";
        } else if let Some(list) = line.strip_prefix("params = ") {
            inside_list = !list.ends_with(']');
            lines.push(line.to_owned());
        } else {
            if line.starts_with('[') {
                inside_table = line == "[[parameters]]";
            }
            if inside_table {
                lines.push(line.to_owned());
            }
        }
    }
    lines
}

/// List the parameter fields the open run form shows, in screen order.
fn run_value_keys(session: &RealSession) -> Vec<String> {
    session
        .state
        .run_form()
        .expect("the run form is on screen")
        .fields()
        .iter()
        .map(|field| field.key.clone())
        .filter(|key| key.starts_with("value:"))
        .collect()
}

/// Find the `meta.toml` of one entry in the real store.
fn meta_path(library: &RealLibrary, selector: &str) -> PathBuf {
    library
        .store
        .data_dir()
        .join("scripts")
        .join(selector)
        .join("meta.toml")
}

#[test]
fn prompt_interpolation_controls_the_run_form_the_detail_and_the_preset_modal() {
    let library = RealLibrary::new();
    seed_runner(&library, "codex");
    // The entry comes from the product add lane, so the stored schema is the one a person makes.
    let source = library.root.path().join("interpolated.md");
    fs::write(&source, "Write about {{TOPIC}} for {{AUDIENCE}}.\n").unwrap();
    let mut workflow = review_add(&library, &source, KnownEntryKind::Prompt);
    let _ = workflow.reduce(AddAction::SetReviewName("Interpolated".to_owned()));
    let _ = workflow.reduce(AddAction::SetPromptCandidate {
        name: "TOPIC".to_owned(),
        selected: true,
    });
    let _ = workflow.reduce(AddAction::SetPromptRunner {
        name: "codex".to_owned(),
        picked: true,
    });
    let slug = commit_reviewed_add(&library, &mut workflow);
    let selector = slug.as_str().to_owned();
    let meta = meta_path(&library, &selector);

    // A settings save gives TOPIC help text. Only the stored schema carries it.
    let help = settings_edits(
        &library.service,
        &library.store,
        &library.state_dir,
        &selector,
        &[("parameter:TOPIC:help", PROMPT_HELP)],
    );
    tui_submit_settings(
        &library.service,
        &library.store,
        &library.state_dir,
        &selector,
        &help,
    )
    .unwrap();
    assert_eq!(
        stored_parameter_lines(&fs::read(&meta).unwrap()),
        STORED_PROMPT_SCHEMA
    );
    let mut session = RealSession::start(library);

    session.save_settings(
        &selector,
        &[(skit_ui::INTERPOLATE_KEY, FieldValue::boolean(false))],
    );
    // The prompt keeps its stored schema on disk. The run form stops showing it.
    assert_eq!(
        stored_parameter_lines(&fs::read(&meta).unwrap()),
        STORED_PROMPT_SCHEMA
    );
    let stored = session.library.service.show(&selector).unwrap();
    let stored_settings = EntrySettings::from_meta(&stored.meta);
    assert!(!stored_settings.interpolate);
    assert_eq!(stored_settings.params, ["TOPIC", "AUDIENCE"]);
    assert_eq!(
        stored_settings
            .parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect::<Vec<_>>(),
        ["TOPIC"]
    );
    assert!(entry_parameters(&session.library.store, &stored).is_empty());
    assert_eq!(session.state.entry_detail(&slug).unwrap().parameters, []);

    session.open_run(&selector);
    assert_eq!(run_value_keys(&session), [] as [String; 0]);
    // Without effective declarations there is nothing to remember, so the modal never opens.
    assert_eq!(
        session.state.update(UiAction::OpenRunPresetSave),
        UiEffect::None
    );
    assert!(session.state.modal().is_none());
    assert_eq!(session.state.update(UiAction::Back), UiEffect::None);

    // Turning interpolation on again shows the stored schema, with no candidate to pick: the
    // stored prompt already names both placeholders.
    session.save_settings(
        &selector,
        &[(skit_ui::INTERPOLATE_KEY, FieldValue::boolean(true))],
    );
    assert_eq!(
        stored_parameter_lines(&fs::read(&meta).unwrap()),
        STORED_PROMPT_SCHEMA
    );
    let restored = session.library.service.show(&selector).unwrap();
    let inputs = settings_inputs(
        &session.library.service,
        &session.library.store,
        &session.library.config_dir,
        &session.library.state_dir,
        &restored,
        None,
    )
    .unwrap();
    assert_eq!(inputs.candidates, [] as [String; 0]);
    // The screen shows the help text the earlier save stored, so the schema came back whole.
    assert_eq!(
        inputs
            .managed
            .iter()
            .find(|row| row.name == "TOPIC")
            .expect("the settings screen manages TOPIC")
            .help,
        PROMPT_HELP
    );
    assert_eq!(
        session
            .state
            .entry_detail(&slug)
            .unwrap()
            .parameters
            .iter()
            .map(|parameter| parameter.key.clone())
            .collect::<Vec<_>>(),
        ["TOPIC", "AUDIENCE"]
    );
    session.open_run(&selector);
    assert_eq!(run_value_keys(&session), ["value:TOPIC", "value:AUDIENCE"]);
}

/// The four mirror URLs the preset choices install, in `mirror_urls` order.
const CHOSEN_MIRRORS: [&str; 4] = [
    "https://pypi.tuna.tsinghua.edu.cn/simple",
    "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/",
    "https://mirror.nju.edu.cn/github-release/astral-sh/uv",
    "https://registry.npmmirror.com",
];

/// Read every mirror URL the configuration holds.
fn mirror_urls(mirror: &skit_store::MirrorSettings) -> [String; 4] {
    [
        mirror.pypi.clone(),
        mirror.python_install.clone(),
        mirror.uv_binary.clone(),
        mirror.npm.clone(),
    ]
}

#[test]
fn turning_the_mirror_master_off_keeps_every_configured_url() {
    let library = RealLibrary::new();
    let mut session = RealSession::start(library);
    let open = session.state.update(UiAction::OpenPreferences);
    session.settle(open);
    for action in [
        PreferencesAction::SetMirrorMaster(true),
        PreferencesAction::ChooseMirror {
            field: skit_application::preferences::PreferencesField::PypiMirror,
            choice: skit_application::preferences::MirrorChoice::Preset("tsinghua".to_owned()),
        },
        PreferencesAction::ChooseMirror {
            field: skit_application::preferences::PreferencesField::GithubMirror,
            choice: skit_application::preferences::MirrorChoice::Preset("nju".to_owned()),
        },
        PreferencesAction::ChooseMirror {
            field: skit_application::preferences::PreferencesField::NpmMirror,
            choice: skit_application::preferences::MirrorChoice::Preset("npmmirror".to_owned()),
        },
    ] {
        assert_eq!(
            session.state.update(UiAction::Preferences(action)),
            UiEffect::None
        );
    }
    let save = session
        .state
        .update(UiAction::Preferences(PreferencesAction::Save));
    session.settle(save);
    let config = FileConfigStore::new(&session.library.config_dir);
    let enabled = config.mirror().unwrap();
    assert!(enabled.enabled);
    assert_eq!(mirror_urls(&enabled), CHOSEN_MIRRORS);

    let open = session.state.update(UiAction::OpenPreferences);
    session.settle(open);
    assert_eq!(
        session
            .state
            .update(UiAction::Preferences(PreferencesAction::SetMirrorMaster(
                false
            ))),
        UiEffect::None
    );
    let save = session
        .state
        .update(UiAction::Preferences(PreferencesAction::Save));
    session.settle(save);

    // The master switch pauses the mirrors. It keeps every URL the user chose.
    let paused = config.mirror().unwrap();
    assert!(!paused.enabled);
    assert_eq!(mirror_urls(&paused), CHOSEN_MIRRORS);
}

#[test]
fn a_submitted_value_equal_to_the_current_default_is_not_remembered() {
    let library = RealLibrary::new();
    let entry = add_command(&library.service, "Default drift", "echo {TARGET}");
    let selector = entry.slug.as_str().to_owned();
    let values = settings_edits(
        &library.service,
        &library.store,
        &library.state_dir,
        &selector,
        &[("parameter:TARGET:default", "A")],
    );
    tui_submit_settings(
        &library.service,
        &library.store,
        &library.state_dir,
        &selector,
        &values,
    )
    .unwrap();
    FileConfigStore::new(&library.config_dir)
        .set("after_run", "stay")
        .unwrap();

    let mut session = RealSession::start(library);
    session.open_run(&selector);
    assert_eq!(session.run_value("value:TARGET"), "A");
    let run = session.state.update(UiAction::Submit);
    session.settle(run);
    let stored = FormStateService::new(FileFormStateStore::new(&session.library.state_dir))
        .load(&entry.slug);
    assert_eq!(stored.last_run.values.unwrap()["TARGET"], "A");
    assert!(stored.values.is_empty(), "{:?}", stored.values);

    session.save_settings(
        &selector,
        &[("parameter:TARGET:default", FieldValue::text("B"))],
    );
    assert_eq!(
        session
            .state
            .entry_detail(&entry.slug)
            .unwrap()
            .parameters
            .iter()
            .find(|parameter| parameter.key == "TARGET")
            .unwrap()
            .value,
        "B"
    );

    // The run recorded the value. It did not remember it, so the new default fills the field.
    session.open_run(&selector);
    assert_eq!(session.run_value("value:TARGET"), "B");
}

#[test]
fn protocol_only_effects_stay_inert_and_a_rerun_honors_after_run_exit() {
    let library = RealLibrary::new();
    let entry = add_command(&library.service, "Inert", "echo ok");
    let before = test_tree_snapshot(library.root.path());
    for effect in [
        UiEffect::None,
        UiEffect::Quit,
        UiEffect::Preferences(PreferencesEffect::None),
    ] {
        assert_eq!(
            tui_effect(
                &library.service,
                &library.store,
                &library.state_dir,
                &library.config_dir,
                effect,
            )
            .unwrap(),
            UiAction::ClearStatus
        );
        assert_eq!(test_tree_snapshot(library.root.path()), before);
    }

    FileConfigStore::new(&library.config_dir)
        .set("after_run", "stay")
        .unwrap();
    // The service seam fixes the locale. `tui_submit_run` reads the process locale, which this
    // test does not own.
    let ran = tui_submit_run_with_services(
        &library.service,
        &library.store,
        &library.state_dir,
        &library.config_dir,
        entry.slug.as_str(),
        &BTreeMap::new(),
        Locale::En,
        &crate::run::RunServices::system(),
    )
    .unwrap();
    let UiAction::Complete { ref message, .. } = ran else {
        panic!("a run that stays must report its result: {ran:?}");
    };
    assert_eq!(
        message,
        &format_text(Locale::En, "Run finished with exit status {}", &[&0])
    );
    FileConfigStore::new(&library.config_dir)
        .set("after_run", "exit")
        .unwrap();

    // The preference decides how a run leaves. The same submit now quits instead of reporting.
    assert_eq!(
        tui_submit_run(
            &library.service,
            &library.store,
            &library.state_dir,
            &library.config_dir,
            entry.slug.as_str(),
            &BTreeMap::new(),
        )
        .unwrap(),
        UiAction::Quit
    );

    // Stamp a record the rerun cannot leave, so a rerun that writes nothing cannot pass on the
    // record an earlier submit left.
    let state = FormStateService::new(FileFormStateStore::new(&library.state_dir));
    const STALE_RUN: &str = "2000-01-01T00:00:00+00:00";
    state
        .record_run(
            &entry.slug,
            7,
            STALE_RUN,
            &entry_parameters(&library.store, &entry),
            Some(&BTreeMap::new()),
        )
        .unwrap();
    assert_eq!(
        state.load(&entry.slug).last_run.at.as_deref(),
        Some(STALE_RUN)
    );

    // A rerun leaves the same way a run does when the preference says to exit.
    assert_eq!(
        tui_effect(
            &library.service,
            &library.store,
            &library.state_dir,
            &library.config_dir,
            UiEffect::Rerun {
                selector: entry.slug.as_str().to_owned(),
            },
        )
        .unwrap(),
        UiAction::Quit
    );
    let recorded = state.load(&entry.slug).last_run;
    assert_eq!(recorded.values, Some(BTreeMap::new()));
    assert_eq!(recorded.exit, Some(0));
    // Only the instant cannot be an exact value. It is the rerun's own, not the stamped one.
    assert_ne!(recorded.at.as_deref(), Some(STALE_RUN));
    assert!(recorded.at.is_some());

    // Pins product rule 1 for the launch menu run summary.
    FileConfigStore::new(&library.config_dir)
        .set("after_run", "stay")
        .unwrap();
    let localized = tui_submit_run_with_services(
        &library.service,
        &library.store,
        &library.state_dir,
        &library.config_dir,
        entry.slug.as_str(),
        &BTreeMap::new(),
        Locale::ZhTw,
        &crate::run::RunServices::system(),
    )
    .unwrap();
    let UiAction::Complete {
        message: ref localized_message,
        ..
    } = localized
    else {
        panic!("a run that stays must report its result: {localized:?}");
    };
    assert_eq!(
        localized_message,
        &format_text(Locale::ZhTw, "Run finished with exit status {}", &[&0])
    );
}

#[test]
fn a_run_submission_carries_the_hidden_keys_each_entry_surface_needs() {
    let library = RealLibrary::new();
    seed_runner(&library, "codex");
    let parameterized = add_command(&library.service, "Parameterized", "echo {NAME}");
    FormStateService::new(FileFormStateStore::new(&library.state_dir))
        .save_preset(
            &parameterized.slug,
            "friendly",
            &entry_parameters(&library.store, &parameterized),
            &BTreeMap::from([("NAME".to_owned(), "Ada".to_owned())]),
        )
        .unwrap();
    let prompt = seed_prompt(
        &library,
        "Hidden prompt",
        "Say hello.\n",
        EntrySettings {
            runner: "codex".to_owned(),
            ..EntrySettings::default()
        },
    );
    let plain = add_command(&library.service, "Plain", "echo ok");
    FileConfigStore::new(&library.config_dir)
        .set("after_run", "stay")
        .unwrap();

    // Every `_skit_` key one surface submits, with the value it carries. `_skit_runner_picked`
    // joins the prompt surface only after a person picks a runner, which item 27 owns.
    let surfaces: [(&Entry, &[(&str, &str)]); 3] = [
        (
            &parameterized,
            &[
                ("_skit_args", ""),
                ("_skit_dry_run", "false"),
                ("_skit_preset", "friendly"),
                ("_skit_save_preset", ""),
            ],
        ),
        (
            &prompt,
            &[
                ("_skit_args", ""),
                ("_skit_dry_run", "false"),
                ("_skit_runner", "codex"),
                ("_skit_save_preset", ""),
            ],
        ),
        (
            &plain,
            &[
                ("_skit_args", ""),
                ("_skit_dry_run", "false"),
                ("_skit_save_preset", ""),
            ],
        ),
    ];

    let mut session = RealSession::start(library);
    for (entry, hidden) in surfaces {
        let selector = entry.slug.as_str().to_owned();
        session.open_run(&selector);
        if selector == parameterized.slug.as_str() {
            let field = session.run_field("_skit_preset");
            assert_eq!(
                session.state.update(UiAction::SelectFieldOption {
                    field,
                    value: "friendly".to_owned(),
                }),
                UiEffect::None
            );
        }
        let submit = session.state.update(UiAction::Submit);
        let UiEffect::Submit { ref values, .. } = submit else {
            panic!("a run must submit through the host form contract: {submit:?}");
        };
        assert_eq!(
            values
                .iter()
                .filter(|(key, _)| key.starts_with("_skit_"))
                .map(|(key, value)| (key.as_str(), value.as_text()))
                .collect::<Vec<_>>(),
            hidden
                .iter()
                .map(|(key, value)| (*key, (*value).to_owned()))
                .collect::<Vec<_>>(),
            "{selector} hidden keys"
        );
        session.settle(submit);
        session.select(&selector);
        assert!(
            session.state.command_enabled(skit_ui::UiCommand::Rerun),
            "{selector} is not rerunnable after its run"
        );
    }
}

#[test]
fn only_a_shell_splittable_pattern_asks_the_host_to_count_matches() {
    let library = RealLibrary::new();
    let globs = library.root.path().join("globs");
    fs::create_dir_all(globs.join("nested")).unwrap();
    // Every pattern that is meant to match has two matches. A count of one is what a pattern
    // that matches nothing returns, so one match could not tell the two apart.
    for name in [
        "alpha.py",
        "blpha.py",
        "beta.py",
        "gamma.rs",
        ".hidden.py",
        ".hidden-two.py",
        "unicodé-1.rs",
        "unicodé-2.rs",
    ] {
        fs::write(globs.join(name), b"").unwrap();
    }
    for name in ["inner.py", "inner-two.py", ".secret.py", ".secret-two.py"] {
        fs::write(globs.join("nested").join(name), b"").unwrap();
    }
    let entry = add_command(&library.service, "Glob counts", "echo {NAME}");
    let invoke_cwd = tui_run_context(&library.store, &entry)
        .unwrap()
        .path
        .unwrap()
        .invoke_cwd;

    let mut session = RealSession::start(library);
    session.open_run(entry.slug.as_str());
    let field = session.run_field("value:NAME");
    for value in ["literal.py", "'unfinished*"] {
        // A literal names one file and an unbalanced quote names nothing, so neither asks.
        assert_eq!(
            session.state.update(UiAction::SetFieldValue {
                field,
                value: value.to_owned(),
            }),
            UiEffect::None,
            "{value}"
        );
    }

    // A pattern that matches nothing counts as the one name the user typed. Exactly three rows
    // do that: `***.py` and `none-*.zzz` find no file, and `[broken` does not compile.
    const NO_MATCH: [&str; 3] = ["***.py", "none-*.zzz", "[broken"];
    for (pattern, expected) in [
        ("*", 7),
        ("*.py", 3),
        ("***.py", 1),
        ("none-*.zzz", 1),
        ("[ab]lpha.py", 2),
        ("[broken", 1),
        ("**/*.py", 5),
        (".hidden*.py", 2),
        ("nested/*.py", 2),
        ("nested/.*.py", 2),
        ("unicodé-?.rs", 2),
    ] {
        // Every other row must count more than one file, so a broken class, a broken `?` or a
        // broken leading-dot rule cannot pass as a match.
        assert_eq!(expected == 1, NO_MATCH.contains(&pattern), "{pattern}");
        let value = globs.join(pattern).display().to_string();
        let effect = session.state.update(UiAction::SetFieldValue {
            field,
            value: shlex::try_quote(&value).unwrap().into_owned(),
        });
        let UiEffect::CountRunGlob { ref request, .. } = effect else {
            panic!("{pattern} must ask the host to count: {effect:?}");
        };
        assert_eq!(request.cwd, invoke_cwd, "{pattern}");
        assert_eq!(request.pieces, [value], "{pattern}");
        session.settle(effect);
        assert_eq!(
            session.state.run_form().unwrap().fields()[field]
                .feedback
                .glob_count,
            Some(expected),
            "{pattern}"
        );
    }
}

#[test]
fn a_picked_prompt_runner_and_its_extra_arguments_survive_the_run() {
    let library = RealLibrary::new();
    seed_runner(&library, "codex");
    seed_runner(&library, "other");
    let entry = seed_prompt(
        &library,
        "Picked runner",
        "Say hello.\n",
        EntrySettings::default(),
    );
    let selector = entry.slug.as_str().to_owned();
    FileConfigStore::new(&library.config_dir)
        .set("after_run", "stay")
        .unwrap();
    let expected_args = vec!["--flag".to_owned(), "two words".to_owned()];
    let editable_args = skit_application::runner_management::join_editable_argv(
        &expected_args,
        EditableArgvDialect::host(),
    );

    let mut session = RealSession::start(library);
    session.open_run(&selector);
    let runner = session.run_field("_skit_runner");
    assert_eq!(
        session.state.update(UiAction::SelectFieldOption {
            field: runner,
            value: "other".to_owned(),
        }),
        UiEffect::None
    );
    let args = session.run_field("_skit_args");
    assert_eq!(
        session.state.update(UiAction::SetFieldValue {
            field: args,
            value: editable_args.clone(),
        }),
        UiEffect::None
    );
    let run = session.state.update(UiAction::Submit);
    let UiEffect::Submit { ref values, .. } = run else {
        panic!("a run must submit through the host form contract: {run:?}");
    };
    // The form hands the host the picked runner and the arguments as one editable line.
    assert_eq!(
        values.get("_skit_runner").map(FieldValue::as_text),
        Some("other".to_owned())
    );
    assert_eq!(
        values.get("_skit_args").map(FieldValue::as_text),
        Some(editable_args.clone())
    );
    assert_eq!(
        values.get("_skit_runner_picked").map(FieldValue::as_text),
        Some("true".to_owned())
    );
    session.settle(run);

    assert_eq!(
        PromptSelectionService::new(FilePromptSelectionStore::new(&session.library.state_dir))
            .last_runner(),
        "other"
    );
    // The host splits that line into the argv it launches, and keeps the exact list.
    let stored = FormStateService::new(FileFormStateStore::new(&session.library.state_dir))
        .load(&entry.slug);
    assert_eq!(stored.extra_args, expected_args);
    assert!(!stored.extra_args_raw);
    session.open_run(&selector);
    assert_eq!(session.run_value("_skit_args"), editable_args);
    assert_eq!(session.state.update(UiAction::Back), UiEffect::None);

    // The picked runner also seeds the next add review, so the next prompt starts with it.
    let Screen::Add(workflow) = tui_open(
        &session.library.service,
        &session.library.store,
        &session.library.state_dir,
        &session.library.config_dir,
        HostRequest::Add,
        None,
    )
    .unwrap() else {
        panic!("add must use its typed workflow");
    };
    assert_eq!(
        workflow.review_defaults().last_runner.as_deref(),
        Some("other")
    );

    let before = test_tree_snapshot(session.library.root.path());
    // A runner nobody configured refuses for its own reason.
    let unknown = tui_submit_run(
        &session.library.service,
        &session.library.store,
        &session.library.state_dir,
        &session.library.config_dir,
        &selector,
        &BTreeMap::from([("_skit_runner".to_owned(), FieldValue::text("missing"))]),
    )
    .unwrap_err();
    assert!(
        matches!(
            unknown,
            CliError::Run(crate::run::RunError::RunnerNotFound { ref name, .. })
                if name == "missing"
        ),
        "{unknown:?}"
    );
    // The roster between the brackets belongs to
    // `surface_edges.rs adding_a_prompt_refuses_a_runner_that_is_not_configured` and
    // `cli/tests.rs adapter_only_error_paths_do_not_require_process_global_configuration`, so
    // this rule owns only the name it refuses and the guidance it gives.
    let message = unknown.message().localize(Locale::En);
    let (opening, roster) = message
        .split_once("(known: ")
        .unwrap_or_else(|| panic!("the refusal must name the runners it knows: {message}"));
    let (_, guidance) = roster
        .split_once("). ")
        .unwrap_or_else(|| panic!("the refusal must close its roster: {message}"));
    assert_eq!(opening, "The runner missing isn't configured ");
    assert_eq!(guidance, "Manage runners with: skit runner list");
    assert_eq!(test_tree_snapshot(session.library.root.path()), before);

    // Invalid quoting refuses first, whether or not the runner is one the library knows.
    for runner in ["missing", "codex"] {
        let error = tui_submit_run(
            &session.library.service,
            &session.library.store,
            &session.library.state_dir,
            &session.library.config_dir,
            &selector,
            &BTreeMap::from([
                ("_skit_runner".to_owned(), FieldValue::text(runner)),
                ("_skit_args".to_owned(), FieldValue::text("\"unfinished")),
            ]),
        )
        .unwrap_err();
        assert!(matches!(error, CliError::Usage(_)), "{runner}: {error:?}");
        assert_eq!(
            error.message().localize(Locale::En),
            "extra arguments have invalid quoting",
            "{runner}"
        );
        assert_eq!(
            test_tree_snapshot(session.library.root.path()),
            before,
            "{runner}"
        );
    }
}

/// Drive one reviewed add through the production add seam and return the new selector.
fn commit_reviewed_add(library: &RealLibrary, workflow: &mut AddWorkflowState) -> Slug {
    let effects = workflow.reduce(AddAction::Save);
    assert!(!effects.is_empty(), "the review must emit a host effect");
    let committed = tui_add_effect(
        &library.service,
        &library.store,
        &library.state_dir,
        &library.config_dir,
        effects,
    )
    .unwrap();
    let UiAction::Add(AddAction::CommitFinished { request, result }) = committed else {
        panic!("a reviewed save must commit: {committed:?}");
    };
    let effects = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Ok(result.unwrap()),
    });
    let completed = tui_add_effect(
        &library.service,
        &library.store,
        &library.state_dir,
        &library.config_dir,
        effects,
    )
    .unwrap();
    let UiAction::AddCompleted { slug, .. } = completed else {
        panic!("the final add effect must complete: {completed:?}");
    };
    slug
}

/// Take the review stage of one add over an existing file.
fn review_add(library: &RealLibrary, source: &Path, kind: KnownEntryKind) -> AddWorkflowState {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(source.display().to_string()));
    let effects = workflow.reduce(AddAction::Continue);
    let inspected = tui_add_effect(
        &library.service,
        &library.store,
        &library.state_dir,
        &library.config_dir,
        effects,
    )
    .unwrap();
    let UiAction::Add(AddAction::SourceInspected { request, result }) = inspected else {
        panic!("source inspection must return through the typed reducer: {inspected:?}");
    };
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(result.unwrap()),
    });
    let _ = workflow.reduce(AddAction::PickKind(Some(kind)));
    workflow
}

/// Read the candidates the settings screen offers for one entry.
fn settings_candidates(library: &RealLibrary, slug: &Slug) -> Vec<String> {
    let entry = library.service.show(slug.as_str()).unwrap();
    settings_inputs(
        &library.service,
        &library.store,
        &library.config_dir,
        &library.state_dir,
        &entry,
        None,
    )
    .unwrap()
    .candidates
}

#[test]
fn an_added_prompt_keeps_only_its_unmanaged_placeholders_as_candidates() {
    for (selected, interpolate, expected) in [
        (false, true, vec!["TOPIC"]),
        (true, true, Vec::new()),
        (false, false, vec!["TOPIC", "AUDIENCE"]),
    ] {
        let library = RealLibrary::new();
        seed_runner(&library, "codex");
        let source = library.root.path().join("prompt.md");
        fs::write(&source, "Write a {{TOPIC}} for {{AUDIENCE}}.\n").unwrap();
        let mut workflow = review_add(&library, &source, KnownEntryKind::Prompt);
        let _ = workflow.reduce(AddAction::SetReviewName("Prompt copy".to_owned()));
        let _ = workflow.reduce(AddAction::SetPromptCandidate {
            name: "TOPIC".to_owned(),
            selected,
        });
        let _ = workflow.reduce(AddAction::SetPromptRunner {
            name: "codex".to_owned(),
            picked: true,
        });
        let _ = workflow.reduce(AddAction::SetPromptInterpolation(interpolate));

        let slug = commit_reviewed_add(&library, &mut workflow);

        assert_eq!(
            settings_candidates(&library, &slug),
            expected,
            "selected {selected}, interpolate {interpolate}"
        );
    }
}

#[test]
fn deselecting_an_added_python_candidate_leaves_it_detectable_again() {
    let library = RealLibrary::new();
    let source = library.root.path().join("tool.py");
    fs::write(&source, "NAME = \"Ada\"\nprint(NAME)\n").unwrap();
    let mut workflow = review_add(&library, &source, KnownEntryKind::Python);
    let _ = workflow.reduce(AddAction::SetReviewName("Python copy".to_owned()));
    let _ = workflow.reduce(AddAction::SetReviewCandidate {
        name: "NAME".to_owned(),
        selected: false,
    });

    let slug = commit_reviewed_add(&library, &mut workflow);

    // The constant stays an unmanaged binding, so the analyzer detects it again.
    assert_eq!(settings_candidates(&library, &slug), ["NAME"]);
}
