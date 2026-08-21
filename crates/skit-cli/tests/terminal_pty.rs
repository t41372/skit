use std::{
    collections::BTreeMap,
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _,
    SourcePermissions,
    form_state::FormStateRepository as _,
    runner_management::{EditableArgvDialect, join_editable_argv},
};
use skit_domain::{
    EntryKind, EntrySettings, Slug, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_language::write_managed_params;
use skit_store::{FileConfigStore, FileFormStateStore, FileStore, PromptRunner};
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};
use tempfile::TempDir;

fn write_command_entry(data: &Path, with_parameter: bool) {
    let directory = data.join("scripts/demo");
    fs::create_dir_all(&directory).unwrap();
    let parameter = if with_parameter {
        concat!(
            "params = [\"name\"]\n",
            "[[parameters]]\n",
            "name = \"name\"\n",
            "delivery = \"placeholder\"\n",
            "required = true\n",
        )
    } else {
        "params = []\n"
    };
    let template = if with_parameter {
        "echo {name}"
    } else {
        "echo done"
    };
    fs::write(
        directory.join("meta.toml"),
        format!(
            concat!(
                "schema = 1\n",
                "name = \"Demo\"\n",
                "kind = \"command\"\n",
                "mode = \"copy\"\n",
                "source = \"\"\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-08-08T00:00:00Z\"\n",
                "id = \"0123456789abcdef0123456789abcdef\"\n",
                "workdir = \"invoke\"\n",
                "description = \"\"\n",
                "template = {template:?}\n",
                "{parameter}",
            ),
            parameter = parameter,
            template = template,
        ),
    )
    .unwrap();
    FileStore::new(data).rebuild_registry().unwrap();
}

fn write_plain_preset_entry(data: &Path) {
    let mut plain = ParamDecl::new("PLAIN");
    plain.prompt = "Plain value".to_owned();
    plain.help = "Plain help row".to_owned();
    plain.default = Some(ParameterValue::String("alpha".to_owned()));
    plain.delivery = ParameterDelivery::Placeholder;
    let mut enabled = ParamDecl::new("ENABLED");
    enabled.parameter_type = ParameterType::Bool;
    enabled.default = Some(ParameterValue::Bool(true));
    enabled.delivery = ParameterDelivery::Placeholder;
    let mut token = ParamDecl::new("TOKEN");
    token.secret = true;
    token.required = true;
    token.env_source = "PRESET_TOKEN".to_owned();
    token.delivery = ParameterDelivery::Placeholder;
    let mut optional_secret = ParamDecl::new("OPTIONAL_SECRET");
    optional_secret.secret = true;
    optional_secret.delivery = ParameterDelivery::Placeholder;
    let mut color = ParamDecl::new("COLOR");
    color.parameter_type = ParameterType::Choice;
    color.required = true;
    color.choices = vec!["red".to_owned(), "blue".to_owned()];
    color.delivery = ParameterDelivery::Placeholder;
    let parameters = vec![plain, enabled, token, optional_secret, color];
    FileStore::new(data)
        .create(CreateEntry {
            name: "Preset matrix".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings {
                template: "printf '%s' '{PLAIN} {ENABLED} {TOKEN} {OPTIONAL_SECRET} {COLOR}'"
                    .to_owned(),
                params: ["PLAIN", "ENABLED", "TOKEN", "OPTIONAL_SECRET", "COLOR"]
                    .map(str::to_owned)
                    .to_vec(),
                parameters,
                ..EntrySettings::default()
            },
        })
        .unwrap();
}

fn write_analyzer_preset_entries(data: &Path) {
    let store = FileStore::new(data);
    let mut input = ParamDecl::new("input-1");
    input.binding = ParameterBinding::Input;
    input.delivery = ParameterDelivery::Inject;
    input.prompt = "Name: ".to_owned();
    let shell = write_managed_params(
        "shell",
        "#!/usr/bin/env bash\nread -p \"Name: \" NAME\n",
        &[input],
    )
    .unwrap();
    for (name, kind, bytes, stored_name) in [
        (
            "Degraded analyzer",
            "js",
            b"parseArgs({options:{name: someVar}});\n".to_vec(),
            "script.js",
        ),
        ("Input analyzer", "shell", shell.into_bytes(), "script.sh"),
    ] {
        store
            .create(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse(kind).unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes,
                    stored_name: Some(stored_name.to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
    }
}

fn write_secret_command_entry(data: &Path) {
    let directory = data.join("scripts/secret");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Secret\"\n",
            "kind = \"command\"\n",
            "mode = \"copy\"\n",
            "source = \"\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-08-08T00:00:00Z\"\n",
            "id = \"1123456789abcdef0123456789abcdef\"\n",
            "workdir = \"invoke\"\n",
            "description = \"\"\n",
            "template = \"echo {name}\"\n",
            "params = [\"name\", \"token\"]\n",
            "[[parameters]]\n",
            "name = \"name\"\n",
            "delivery = \"placeholder\"\n",
            "required = true\n",
            "[[parameters]]\n",
            "name = \"token\"\n",
            "delivery = \"placeholder\"\n",
            "required = true\n",
            "secret = true\n",
        ),
    )
    .unwrap();
    FileStore::new(data).rebuild_registry().unwrap();
}

fn write_pinned_prompt_entry(data: &Path) {
    let directory = data.join("scripts/pinned-prompt");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("prompt.md"), "Hello\n").unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Pinned prompt\"\n",
            "kind = \"prompt\"\n",
            "mode = \"copy\"\n",
            "source = \"prompt.md\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-08-08T00:00:00Z\"\n",
            "id = \"2123456789abcdef0123456789abcdef\"\n",
            "workdir = \"invoke\"\n",
            "description = \"\"\n",
            "runner = \"local\"\n",
            "interpolate = true\n",
        ),
    )
    .unwrap();
    FileStore::new(data).rebuild_registry().unwrap();
}

fn write_stale_default_entry(data: &Path, home: &Path) -> (PathBuf, PathBuf) {
    let mut name = ParamDecl::new("NAME");
    name.binding = ParameterBinding::Const;
    name.delivery = ParameterDelivery::Inject;
    name.parameter_type = ParameterType::Str;
    name.default = Some(ParameterValue::String("hello".to_owned()));
    let mut count = ParamDecl::new("COUNT");
    count.binding = ParameterBinding::Const;
    count.delivery = ParameterDelivery::Inject;
    count.parameter_type = ParameterType::Int;
    count.default = Some(ParameterValue::Integer(3));

    let original = "NAME = \"hello\"\nCOUNT = 3\nprint(NAME, COUNT)\n";
    let managed = write_managed_params("python", original, &[name, count]).unwrap();
    let source = home.join("greet.py");
    fs::write(&source, managed.as_bytes()).unwrap();
    let store = FileStore::new(data);
    let entry = store
        .create(CreateEntry {
            name: "greet".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Copy,
            source: source.display().to_string(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: managed.into_bytes(),
                stored_name: Some("script.py".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let payload = store.payload_path(&entry).unwrap();
    let current = fs::read_to_string(&payload).unwrap();
    assert!(current.contains("default = \"hello\""));
    fs::write(
        &payload,
        current.replace("NAME = \"hello\"", "NAME = \"bonjour\""),
    )
    .unwrap();
    let metadata = payload.parent().unwrap().join("meta.toml");
    (payload, metadata)
}

fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, snapshot: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let mut children = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            let relative = child.strip_prefix(root).unwrap().to_path_buf();
            if child.is_dir() {
                snapshot.insert(relative, None);
                visit(root, &child, snapshot);
            } else {
                snapshot.insert(relative, Some(fs::read(&child).unwrap()));
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    visit(root, root, &mut snapshot);
    snapshot
}

fn run_in_pty(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
    input: &[&[u8]],
) -> (u32, String) {
    run_pty(args, data, state, config, input, true)
}

fn run_plain_in_pty(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
    input: &[&[u8]],
) -> (u32, String) {
    run_pty(args, data, state, config, input, false)
}

fn run_pty(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
    input: &[&[u8]],
    answer_cursor_query: bool,
) -> (u32, String) {
    run_pty_configured(
        args,
        data,
        state,
        config,
        input,
        answer_cursor_query,
        |_| {},
    )
}

fn run_pty_configured(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
    input: &[&[u8]],
    answer_cursor_query: bool,
    configure: impl FnOnce(&mut CommandBuilder),
) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", data);
    command.env("SKIT_STATE_DIR", state);
    command.env("SKIT_CONFIG_DIR", config);
    configure(&mut command);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(40));
    if answer_cursor_query {
        let _ = writer.write_all(b"\x1b[1;1R");
        let _ = writer.flush();
    }
    for bytes in input {
        thread::sleep(Duration::from_millis(120));
        if writer.write_all(bytes).is_err() {
            break;
        }
        let _ = writer.flush();
    }
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    (status.exit_code(), output)
}

struct LiveTui {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn std::io::Write + Send>,
    chunks: Receiver<Vec<u8>>,
    output: Vec<u8>,
}

impl LiveTui {
    fn spawn(data: &Path, state: &Path, config: &Path, home: &Path) -> Self {
        let mut tui = Self::spawn_command(&["tui"], data, state, config, home, "en");
        tui.answer_cursor_query_after(0);
        tui
    }

    fn spawn_command(
        args: &[&str],
        data: &Path,
        state: &Path,
        config: &Path,
        home: &Path,
        locale: &str,
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(args);
        command.cwd(home);
        command.env("TERM", "xterm-256color");
        command.env("NO_COLOR", "1");
        command.env("SKIT_LANG", locale);
        command.env("SKIT_DATA_DIR", data);
        command.env("SKIT_STATE_DIR", state);
        command.env("SKIT_CONFIG_DIR", config);
        command.env("HOME", home);
        command.env("USERPROFILE", home);
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (sender, chunks) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if sender.send(buffer[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            child,
            writer,
            chunks,
            output: Vec::new(),
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    fn send_effect_key(&mut self, bytes: &[u8]) {
        let checkpoint = self.checkpoint();
        self.send(bytes);
        self.answer_cursor_query_after(checkpoint);
    }

    fn checkpoint(&mut self) -> usize {
        self.drain();
        self.output.len()
    }

    fn wait_for(&mut self, needle: &str) -> String {
        self.wait_for_after(0, needle)
    }

    fn wait_for_after(&mut self, checkpoint: usize, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            self.drain();
            let visible = self.visible_after(checkpoint);
            if visible.contains(needle) {
                return visible;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                panic!(
                    "timed out waiting for {needle:?} after checkpoint {checkpoint}; new terminal output:\n{visible}"
                );
            };
            match self
                .chunks
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    assert!(
                        self.child.try_wait().unwrap().is_none(),
                        "TUI exited while waiting for {needle:?}; new terminal output:\n{visible}"
                    );
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!(
                    "TUI output closed while waiting for {needle:?}; new terminal output:\n{visible}"
                ),
            }
        }
    }

    fn wait_for_exit_after(&mut self, checkpoint: usize) -> String {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            self.drain();
            if self.child.try_wait().unwrap().is_some() {
                return self.visible_after(checkpoint);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                panic!(
                    "timed out waiting for TUI exit; new terminal output:\n{}",
                    self.visible_after(checkpoint)
                );
            };
            match self
                .chunks
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if self.child.try_wait().unwrap().is_some() {
                        return self.visible_after(checkpoint);
                    }
                }
            }
        }
    }

    fn visible_after(&mut self, checkpoint: usize) -> String {
        self.drain();
        let checkpoint = checkpoint.min(self.output.len());
        strip_terminal_control(&String::from_utf8_lossy(&self.output[checkpoint..]))
    }

    fn drain(&mut self) {
        while let Ok(chunk) = self.chunks.try_recv() {
            self.output.extend_from_slice(&chunk);
        }
    }

    fn answer_cursor_query_after(&mut self, checkpoint: usize) {
        const QUERY: &[u8] = b"\x1b[6n";

        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            self.drain();
            if self.output[checkpoint.min(self.output.len())..]
                .windows(QUERY.len())
                .any(|window| window == QUERY)
            {
                self.send(b"\x1b[1;1R");
                return;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                panic!("timed out waiting for the terminal cursor-position query");
            };
            match self
                .chunks
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    assert!(
                        self.child.try_wait().unwrap().is_none(),
                        "TUI exited before it requested the cursor position"
                    );
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("TUI output closed before it requested the cursor position");
                }
            }
        }
    }
}

impl Drop for LiveTui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn test_resume_bash_shebang_draft_lands_as_shell() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let drafts = data.path().join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let draft = drafts.join("skit-new-ship.py");
    const SOURCE: &[u8] = b"#!/usr/bin/env bash\necho drafted\n";
    fs::write(&draft, SOURCE).unwrap();

    let mut tui = LiveTui::spawn(data.path(), state.path(), config.path(), home.path());
    tui.wait_for("Library");
    let open = tui.checkpoint();
    tui.send_effect_key(b"a");
    tui.wait_for_after(open, "Add an entry");

    // Enter the real kept-draft path in the focused Source path field. This uses the same
    // keyboard route as `skit add <path>` without a direct reducer action surrogate.
    let draft_input = draft.display().to_string();
    let typing = tui.checkpoint();
    tui.send(draft_input.as_bytes());
    tui.wait_for_after(typing, &draft_input);
    let resume = tui.checkpoint();
    tui.send(b"\r\x1b[1;1R");
    tui.wait_for_after(resume, "Add skit-new-ship.py");

    let submit = tui.checkpoint();
    tui.send(&[0x13]);
    tui.send(b"\x1b[1;1R");
    let receipt = tui.wait_for_after(submit, "Entryadded");
    assert!(receipt.contains("Library"), "{receipt}");

    let store = FileStore::new(data.path());
    let entry = store.resolve("skit-new-ship").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "shell");
    assert_eq!(entry.meta.mode, StorageMode::Copy);
    assert_eq!(
        fs::read(store.payload_path(&entry).unwrap()).unwrap(),
        SOURCE
    );
    assert!(
        !draft.exists(),
        "the exact reviewed draft was consumed after commit"
    );

    let quit = tui.checkpoint();
    tui.send(b"q");
    let _ = tui.wait_for_exit_after(quit);
}

fn strip_terminal_control(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0x1b {
            if bytes[index] != b'\r' {
                output.push(bytes[index]);
            }
            index += 1;
            continue;
        }
        index += 1;
        if index >= bytes.len() {
            break;
        }
        match bytes[index] {
            b'[' => {
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            b']' => {
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn compact_terminal_text(input: &str) -> String {
    input
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

struct PromptTuiSandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl PromptTuiSandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        };
        sandbox.config().set("after_run", "stay").unwrap();
        sandbox
    }

    fn config(&self) -> FileConfigStore {
        FileConfigStore::new(self.config.path())
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn clear_runners(&self) {
        let config = self.config();
        config.ensure_runners_seeded().unwrap();
        let names = config
            .runners()
            .unwrap()
            .into_iter()
            .map(|runner| runner.name)
            .collect::<Vec<_>>();
        for name in names {
            assert!(config.remove_runner(&name).unwrap());
        }
    }

    fn runner(&self, name: &str, child_marker: &str) {
        let marker_path = self.marker_path(name);
        let program = runner_program(self.home.path(), name, child_marker);
        self.config()
            .set_runner(
                PromptRunner {
                    name: name.to_owned(),
                    argv: runner_argv(&program, &marker_path),
                },
                false,
            )
            .unwrap();
    }

    fn runner_draft(&self, name: &str, child_marker: &str) -> (String, Vec<String>, PathBuf) {
        let marker_path = self.marker_path(name);
        let program = runner_program(self.home.path(), name, child_marker);
        let argv = runner_argv(&program, &marker_path);
        let command = join_editable_argv(&argv, EditableArgvDialect::host());
        (command, argv, marker_path)
    }

    fn missing_runner(&self, name: &str) {
        self.config()
            .set_runner(
                PromptRunner {
                    name: name.to_owned(),
                    argv: vec![name.to_owned(), "{{prompt}}".to_owned()],
                },
                false,
            )
            .unwrap();
    }

    fn marker_path(&self, name: &str) -> PathBuf {
        self.home.path().join(format!("child-{name}.ran"))
    }

    fn prompt(&self, pin: &str) {
        let path = self.home.path().join("p.prompt.md");
        fs::write(&path, "Do {{a}}\n").unwrap();
        let review = ReviewState::from_source(
            SourceSnapshot {
                path: path.clone(),
                source_record: path.display().to_string(),
                bytes: b"Do {{a}}\n".to_vec(),
                permissions: SourcePermissions::default(),
                is_regular: true,
                is_directory: false,
                is_draft: false,
                identity: None,
            },
            KnownEntryKind::Prompt,
            ReviewDefaults {
                name: Some("p".to_owned()),
                ..ReviewDefaults::default()
            },
        );
        let mut request = review.create_entry().unwrap();
        request.settings.runner = pin.to_owned();
        self.store().create(request).unwrap();
    }

    fn seed_run(&self, runner: &str, child_marker: &str) {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_skit"))
            .args([
                "run",
                "p",
                "--set",
                "a=1",
                "--runner",
                runner,
                "--no-input",
                "--plain",
            ])
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .current_dir(self.home.path())
            .output()
            .unwrap();
        let shown = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "seed run failed: {shown}");
        assert!(
            shown.lines().any(|line| line.trim() == child_marker),
            "seed run did not reach the runner child: {shown}"
        );
        assert!(
            shown.lines().any(|line| line.trim() == "Do 1"),
            "seed run did not render its stored value: {shown}"
        );
        assert!(
            self.marker_path(runner).is_file(),
            "seed run did not write its child marker"
        );
    }

    fn tui(&self) -> LiveTui {
        LiveTui::spawn(
            self.data.path(),
            self.state.path(),
            self.config.path(),
            self.home.path(),
        )
    }
}

#[cfg(unix)]
fn runner_program(home: &Path, name: &str, child_marker: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let path = home.join(format!("runner-{name}"));
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' {child_marker:?}\nprintf ran > \"$1\"\nprintf '%s\\n' \"$2\"\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
fn runner_argv(program: &Path, marker_path: &Path) -> Vec<String> {
    vec![
        program.display().to_string(),
        marker_path.display().to_string(),
        "{{prompt}}".to_owned(),
    ]
}

#[cfg(windows)]
fn runner_program(home: &Path, name: &str, child_marker: &str) -> PathBuf {
    let path = home.join(format!("runner-{name}.cmd"));
    fs::write(
        &path,
        format!("@echo off\r\necho {child_marker}\r\necho ran>\"%~1\"\r\necho %~2\r\n"),
    )
    .unwrap();
    path
}

#[cfg(windows)]
fn runner_argv(program: &Path, marker_path: &Path) -> Vec<String> {
    vec![
        "cmd.exe".to_owned(),
        "/C".to_owned(),
        program.display().to_string(),
        marker_path.display().to_string(),
        "{{prompt}}".to_owned(),
    ]
}

#[test]
fn test_rerun_unpinned_prompt_falls_back_to_the_form() {
    const LAST_RUNNER_CHILD: &str = "CHILD-UNPINNED-LAST";
    const OTHER_RUNNER_CHILD: &str = "CHILD-UNPINNED-OTHER";

    let sandbox = PromptTuiSandbox::new();
    sandbox.clear_runners();
    sandbox.runner("codex", LAST_RUNNER_CHILD);
    sandbox.runner("claude", OTHER_RUNNER_CHILD);
    sandbox.prompt("");
    sandbox.seed_run("codex", LAST_RUNNER_CHILD);
    let last_runner_marker = sandbox.marker_path("codex");
    let other_runner_marker = sandbox.marker_path("claude");
    fs::remove_file(&last_runner_marker).unwrap();

    let mut tui = sandbox.tui();
    tui.wait_for("Library");
    let rerun = tui.checkpoint();
    tui.send_effect_key(b"r");
    let opened = tui.wait_for_after(rerun, "Run p");
    assert!(
        opened.contains("Runner"),
        "an unpinned rerun did not expose the runner picker: {opened}"
    );
    let after_rerun = tui.visible_after(rerun);
    assert!(
        !after_rerun.contains(LAST_RUNNER_CHILD) && !after_rerun.contains(OTHER_RUNNER_CHILD),
        "an unpinned rerun silently launched a runner child: {after_rerun}"
    );
    assert!(
        !last_runner_marker.exists() && !other_runner_marker.exists(),
        "an unpinned rerun wrote a runner child marker"
    );

    let back = tui.checkpoint();
    tui.send(b"\x1b");
    tui.wait_for_after(back, "Library");
    let quit = tui.checkpoint();
    tui.send(b"q");
    let _ = tui.wait_for_exit_after(quit);
}

#[test]
fn test_rerun_pinned_prompt_skips_the_form_and_uses_the_pin() {
    const LAST_RUNNER_CHILD: &str = "CHILD-PINNED-LAST";
    const PINNED_RUNNER_CHILD: &str = "CHILD-PINNED-CLAUDE";

    let sandbox = PromptTuiSandbox::new();
    sandbox.clear_runners();
    sandbox.runner("codex", LAST_RUNNER_CHILD);
    sandbox.runner("claude", PINNED_RUNNER_CHILD);
    sandbox.prompt("claude");
    sandbox.seed_run("codex", LAST_RUNNER_CHILD);
    let last_runner_marker = sandbox.marker_path("codex");
    let pinned_runner_marker = sandbox.marker_path("claude");
    fs::remove_file(&last_runner_marker).unwrap();

    let mut tui = sandbox.tui();
    tui.wait_for("Library");
    let rerun = tui.checkpoint();
    tui.send_effect_key(b"r");
    let child = tui.wait_for_after(rerun, PINNED_RUNNER_CHILD);
    let rendered = tui.wait_for_after(rerun, "Do 1");
    assert!(
        !child.contains(LAST_RUNNER_CHILD),
        "the pinned rerun used the last-run runner instead of its pin: {child}"
    );
    assert!(
        !rendered.contains("Run p"),
        "the pinned rerun opened the form before it launched: {rendered}"
    );
    assert!(
        pinned_runner_marker.is_file(),
        "the pinned rerun did not write the pinned runner's child marker"
    );
    assert!(
        !last_runner_marker.exists(),
        "the pinned rerun wrote the last-run runner's child marker"
    );

    tui.wait_for_after(rerun, "Library");
    let quit = tui.checkpoint();
    tui.send(b"q");
    let _ = tui.wait_for_exit_after(quit);
}

#[test]
fn test_selected_prompt_runner_preflight_failure_returns_to_library() {
    const MISSING_RUNNER: &str = "skit-definitely-missing-selected-runner";
    const WORKING_CHILD: &str = "CHILD-WORKING-FALLBACK";

    let sandbox = PromptTuiSandbox::new();
    sandbox.clear_runners();
    sandbox.missing_runner(MISSING_RUNNER);
    sandbox.runner("working", WORKING_CHILD);
    sandbox.prompt("");
    let working_marker = sandbox.marker_path("working");
    let form_state = sandbox.state.path().join("values/p.toml");

    let mut tui = sandbox.tui();
    tui.wait_for("Library");
    let open = tui.checkpoint();
    tui.send_effect_key(b"\r");
    let form = tui.wait_for_after(open, "Run p");
    assert!(
        compact_terminal_text(&form).contains(MISSING_RUNNER),
        "the missing runner was not the form's resolved default: {form}"
    );
    tui.send(b"hello");

    assert!(!working_marker.exists());
    assert!(!form_state.exists());
    let submit = tui.checkpoint();
    tui.send_effect_key(&[0x12]);
    let refused = tui.wait_for_after(submit, "program");
    let program_error = format!("required program was not found: {MISSING_RUNNER}");
    assert!(
        compact_terminal_text(&refused).contains(&compact_terminal_text(&program_error)),
        "the selected runner refusal lost its exact typed error: {refused}"
    );
    assert!(
        !working_marker.exists(),
        "the host launched the working fallback after the selected runner failed"
    );
    assert!(
        !form_state.exists(),
        "a failed preflight wrote last-run or form state"
    );

    let library = tui.wait_for_after(submit, "Library");
    let expected = format!("Error: {program_error}");
    assert!(
        compact_terminal_text(&library).contains(&compact_terminal_text(&expected)),
        "the Library lost the exact localized refusal: {library}"
    );
    let quit = tui.checkpoint();
    tui.send(b"q");
    let _ = tui.wait_for_exit_after(quit);
}

#[test]
fn test_run_with_zero_runners_offers_the_new_agent_modal() {
    const CANCELLED_CHILD: &str = "CHILD-ZERO-RUNNER-CANCELLED";

    let sandbox = PromptTuiSandbox::new();
    sandbox.prompt("");
    sandbox.clear_runners();
    let config_path = sandbox.config.path().join("config.toml");
    let config_before = fs::read(&config_path).unwrap();
    let (_, _, child_marker) = sandbox.runner_draft("cancelled", CANCELLED_CHILD);
    let form_state = sandbox.state.path().join("values/p.toml");
    let selection_state = sandbox.state.path().join("prompt.toml");

    let mut tui = sandbox.tui();
    tui.wait_for("Library");
    let open = tui.checkpoint();
    tui.send_effect_key(b"\r");
    tui.wait_for_after(open, "New agent (runner)");

    let cancel = tui.checkpoint();
    tui.send(b"\x1b");
    let returned = tui.wait_for_after(cancel, "configured");
    let expected = "A prompt needs a configured agent to run with.";
    assert!(
        compact_terminal_text(&returned).contains(&compact_terminal_text(expected)),
        "the zero-runner cancel lost its exact status: {returned}"
    );
    assert!(
        returned.contains("Library"),
        "the zero-runner cancel did not return to the Library: {returned}"
    );
    assert_eq!(fs::read(&config_path).unwrap(), config_before);
    assert!(sandbox.config().runners().unwrap().is_empty());
    assert!(!form_state.exists());
    assert!(!selection_state.exists());
    assert!(!child_marker.exists());

    let quit = tui.checkpoint();
    tui.send(b"q");
    let _ = tui.wait_for_exit_after(quit);
}

#[test]
fn test_run_with_zero_runners_define_agent_then_run() {
    const CHILD: &str = "CHILD-ZERO-RUNNER-DEFINED";

    let sandbox = PromptTuiSandbox::new();
    sandbox.prompt("");
    sandbox.clear_runners();
    let (command, expected_argv, child_marker) = sandbox.runner_draft("mycli", CHILD);
    let form_state_path = sandbox.state.path().join("values/p.toml");

    let mut tui = sandbox.tui();
    tui.wait_for("Library");
    let open = tui.checkpoint();
    tui.send_effect_key(b"\r");
    tui.wait_for_after(open, "New agent (runner)");
    tui.send(b"mycli");
    tui.send(b"\t");
    tui.send(command.as_bytes());
    let save = tui.checkpoint();
    tui.send_effect_key(b"\r");
    let form = tui.wait_for_after(save, "Run p");
    assert!(
        compact_terminal_text(&form).contains("mycli"),
        "the saved runner was not selected in the existing run form: {form}"
    );

    let runners = sandbox.config().runners().unwrap();
    assert_eq!(runners.len(), 1);
    assert_eq!(runners[0].name, "mycli");
    assert_eq!(runners[0].argv, expected_argv);
    assert!(!child_marker.exists());
    assert!(!form_state_path.exists());

    tui.send(b"x");
    let submit = tui.checkpoint();
    tui.send_effect_key(&[0x12]);
    let child = tui.wait_for_after(submit, CHILD);
    let rendered = tui.wait_for_after(submit, "Do x");
    assert!(
        compact_terminal_text(&child).contains(CHILD),
        "the configured runner child did not execute: {child}"
    );
    assert!(
        compact_terminal_text(&rendered).contains("Dox"),
        "the runner did not receive the rendered prompt: {rendered}"
    );
    assert!(child_marker.is_file());

    let persisted = FileFormStateStore::new(sandbox.state.path()).load(&Slug::parse("p").unwrap());
    assert_eq!(persisted.values.get("a").map(String::as_str), Some("x"));
    assert_eq!(persisted.last_run.exit, Some(0));
    assert_eq!(
        persisted
            .last_run
            .values
            .as_ref()
            .and_then(|values| values.get("a"))
            .map(String::as_str),
        Some("x")
    );

    tui.wait_for_after(submit, "Library");
    let quit = tui.checkpoint();
    tui.send(b"q");
    let _ = tui.wait_for_exit_after(quit);
}

#[test]
fn zero_runner_save_failure_keeps_the_required_editor_and_config() {
    const CHILD: &str = "CHILD-ZERO-RUNNER-RACE";

    let sandbox = PromptTuiSandbox::new();
    sandbox.prompt("");
    sandbox.clear_runners();
    let (command, expected_argv, child_marker) = sandbox.runner_draft("mycli", CHILD);

    let mut tui = sandbox.tui();
    tui.wait_for("Library");
    let open = tui.checkpoint();
    tui.send_effect_key(b"\r");
    tui.wait_for_after(open, "New agent (runner)");
    tui.send(b"mycli");
    tui.send(b"\t");
    tui.send(command.as_bytes());

    sandbox
        .config()
        .set_runner(
            PromptRunner {
                name: "mycli".to_owned(),
                argv: expected_argv,
            },
            false,
        )
        .unwrap();
    let config_path = sandbox.config.path().join("config.toml");
    let config_before_save = fs::read(&config_path).unwrap();

    let save = tui.checkpoint();
    tui.send_effect_key(b"\r");
    let refused = tui.wait_for_after(save, "already exists");
    let compact_refused = compact_terminal_text(&refused);
    assert!(
        compact_refused.contains("Newagent(runner)"),
        "the failed save closed the required runner editor: {refused}"
    );
    assert!(
        compact_refused.contains("mycli"),
        "the failed save lost the entered runner name: {refused}"
    );
    assert!(
        compact_refused.contains("child-mycli.ran") && compact_refused.contains("{{prompt}}"),
        "the failed save lost the entered runner command: {refused}"
    );
    assert_eq!(fs::read(&config_path).unwrap(), config_before_save);
    assert!(!child_marker.exists());

    let cancel = tui.checkpoint();
    tui.send(b"\x1b");
    tui.wait_for_after(cancel, "Library");
    let quit = tui.checkpoint();
    tui.send(b"q");
    let _ = tui.wait_for_exit_after(quit);
}

#[test]
fn bare_agent_install_lists_existing_targets_and_writes_only_the_confirmed_choice() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".claude")).unwrap();
    fs::create_dir(project.path().join(".agents")).unwrap();

    let (code, output) = run_pty_configured(
        &["agent", "install"],
        data.path(),
        state.path(),
        config.path(),
        &[b"2\n", b"y\n"],
        false,
        |command| {
            command.env("HOME", home.path());
            command.env("USERPROFILE", home.path());
            command.cwd(project.path());
        },
    );

    assert_eq!(code, 0, "{output}");
    assert!(output.contains("1. claude (user)"), "{output}");
    assert!(output.contains("2. agents (project)"), "{output}");
    assert!(output.contains("Write the skill into"), "{output}");
    assert!(
        project
            .path()
            .join(".agents/skills/skit/SKILL.md")
            .is_file(),
        "{output}"
    );
    assert!(!home.path().join(".claude/skills").exists());
}

#[cfg(unix)]
fn run_with_null_stdin_in_pty(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new("sh");
    command.args(["-c", "exec \"$@\" < /dev/null", "sh"]);
    command.arg(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", data);
    command.env("SKIT_STATE_DIR", state);
    command.env("SKIT_CONFIG_DIR", config);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let writer = pair.master.take_writer().unwrap();
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    (status.exit_code(), output)
}

#[test]
fn terminal_detection_keeps_automation_flags_noninteractive() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), false);

    let (code, output) = run_in_pty(
        &["add", "--no-input"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 2, "{output}");

    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run", "--no-input"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 0, "{output}");

    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run", "--raw"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 2, "{output}");
}

#[test]
fn bare_add_uses_the_typed_workflow_and_returns_the_created_entry() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("typed-add.sh");
    fs::write(&source, b"#!/bin/sh\necho typed\n").unwrap();
    let path_input = format!("{}", source.display());

    let (code, output) = run_in_pty(
        &["add"],
        data.path(),
        state.path(),
        config.path(),
        &[path_input.as_bytes(), b"\r", b"\x1b[1;1R", b"\x13"],
    );

    assert_eq!(code, 0, "{output}");
    let entry = FileStore::new(data.path()).resolve("typed-add").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "shell");
    assert!(output.contains("Added: typed-add"), "{output}");
}

/// `skit add <path>` from a terminal opens the review panel instead of writing the entry.
///
/// Version 0.4's own tape calls this "the common path, since the shell"
/// (`docs/assets/demo/demo.tape:8`), and it hosts the same panel the `a` door hosts
/// (`src/skit/cli.py:2116-2126`). Replaying that tape against this build is what found the loss:
/// the entry was written and summarized before anyone could name it, choose copy or link, or edit
/// the detected dependencies. A pty is the only place the rule is real, because the rule is that
/// both standard streams are terminals.
///
/// This reads the panel and stops. Submitting from here would need `Ctrl+S`, which the pty's own
/// flow control eats as XOFF — the same thing that froze the recorded tape — so the submit path is
/// covered by the workflow's own tests instead.
#[test]
fn a_path_add_from_a_terminal_opens_the_review_panel() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("greet.py");
    fs::write(&source, b"GREETING = \"hello\"\nprint(GREETING)\n").unwrap();

    let screen = read_pty_screen(
        &["add", &source.to_string_lossy()],
        data.path(),
        state.path(),
        config.path(),
    );

    // The panel's own controls, none of which the direct-write lane ever drew.
    for expected in [
        "Package dependencies",
        "Python constraint",
        "Tick the ones the run form should ask for:",
        "GREETING",
        // The footer chips in full: the filter keeps SGR parameter text, so a bare word could
        // match escape-code residue rather than anything a person reads.
        "[Ctrl+S] Add",
        "[Esc] Cancel",
    ] {
        assert!(
            screen.contains(expected),
            "the review panel never drew {expected}: {screen}"
        );
    }
    // Nothing was written: the review is a question, not a receipt.
    assert!(
        FileStore::new(data.path()).resolve("greet").is_err(),
        "the entry was created before anyone reviewed it"
    );
}

/// Run one command on a pty, answer the cursor query, and return what it drew.
///
/// The child is stopped rather than driven: this reports the first screen, which is the claim.
fn read_pty_screen(args: &[&str], data: &Path, state: &Path, config: &Path) -> String {
    use std::io::Read as _;

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", data.to_string_lossy().as_ref());
    command.env("SKIT_STATE_DIR", state.to_string_lossy().as_ref());
    command.env("SKIT_CONFIG_DIR", config.to_string_lossy().as_ref());
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut buffer = vec![0_u8; 65_536];
        let mut total = Vec::new();
        while total.len() < 400_000 {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => total.extend_from_slice(&buffer[..read]),
            }
        }
        total
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(60));
    let _ = writer.write_all(b"\x1b[1;1R");
    let _ = writer.flush();
    thread::sleep(Duration::from_millis(1_500));
    let _ = child.kill();
    let _ = child.wait();
    drop(writer);
    let raw = drain.join().unwrap();
    String::from_utf8_lossy(&raw)
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .collect()
}

#[test]
fn bare_add_plain_menu_and_typed_cancel_keep_the_latest_main_contract() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    fs::write(config.path().join("config.toml"), "form = \"plain\"\n").unwrap();
    let source = data.path().join("plain-add.sh");
    fs::write(&source, b"#!/bin/sh\necho plain\n").unwrap();
    let path_input = format!("{}\n", source.display());

    let (code, output) = run_plain_in_pty(
        &["add"],
        data.path(),
        state.path(),
        config.path(),
        &[b"1\n", path_input.as_bytes()],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("What would you like to add?"), "{output}");
    assert!(output.contains("Which one?"), "{output}");
    FileStore::new(data.path()).resolve("plain-add").unwrap();

    fs::remove_file(config.path().join("config.toml")).unwrap();
    let (code, output) = run_in_pty(
        &["add"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 130, "{output}");
    assert!(
        output.contains("Cancelled — nothing was added."),
        "{output}"
    );
}

#[cfg(unix)]
#[test]
fn one_nonterminal_standard_stream_disables_interactive_forms() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), false);

    let (code, output) =
        run_with_null_stdin_in_pty(&["add"], data.path(), state.path(), config.path());
    assert_eq!(code, 2, "{output}");

    let (code, output) = run_with_null_stdin_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
    );
    assert_eq!(code, 0, "{output}");
}

fn write_python_entry(data: &Path) {
    let directory = data.join("scripts/bootstrap");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("script.py"), "print('ok')\n").unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Bootstrap\"\n",
            "kind = \"python\"\n",
            "mode = \"copy\"\n",
            "source = \"\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-08-08T00:00:00Z\"\n",
            "id = \"3123456789abcdef0123456789abcdef\"\n",
            "workdir = \"invoke\"\n",
            "description = \"\"\n",
            "params = []\n",
        ),
    )
    .unwrap();
    FileStore::new(data).rebuild_registry().unwrap();
}

/// The first Python run without a system uv asks before it downloads anything.
///
/// Version 0.4 asks on stderr with a default-yes question (`src/skit/uvman.py:74-82`), treats only
/// `n`/`no` as a refusal (`src/skit/uvman.py:88`), answers itself at end of input
/// (`src/skit/uvman.py:85-86`), and reports the self-install guidance when refused
/// (`src/skit/uvman.py:252-256`).
#[test]
fn a_first_python_run_asks_before_it_downloads_a_private_uv() {
    let empty_path = TempDir::new().unwrap();
    let ask = "Download uv";
    let declined = "Download declined.";

    // Every case points the uv mirror at a refused local port, so no case can reach the network
    // even if the question stops working.
    let attempt = |answer: &'static [u8]| {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        write_python_entry(data.path());
        fs::write(
            config.path().join("config.toml"),
            "[mirror]\nenabled = true\nuv_binary = \"https://127.0.0.1:9/astral-sh/uv\"\n",
        )
        .unwrap();
        // Ctrl+R submits the launch form, and the consent question follows it.
        let (code, output) = run_pty_configured(
            &["run", "bootstrap"],
            data.path(),
            state.path(),
            config.path(),
            &[b"\x12", answer],
            true,
            |command| {
                command.env("PATH", empty_path.path());
            },
        );
        assert!(!data.path().join("bin/uv").exists(), "{output}");
        (code, output)
    };

    for answer in [&b"n\n"[..], &b"no\n"[..], &b"  NO  \n"[..]] {
        let (code, output) = attempt(answer);
        assert!(output.contains(ask), "{output}");
        assert!(output.contains("This won't touch your PATH"), "{output}");
        assert!(output.contains(declined), "{output}");
        // A launch failure exits 125 (`src/skit/flows.py:868`).
        assert_eq!(code, 125, "{output}");
        assert!(!output.contains("First run — downloading uv"), "{output}");
    }

    // Anything else is consent, so the download starts and fails against the refused port.
    let (code, output) = attempt(b"\n");
    assert!(output.contains(ask), "{output}");
    assert!(!output.contains(declined), "{output}");
    assert!(output.contains("First run — downloading uv"), "{output}");
    assert_eq!(code, 125, "{output}");
}

/// The running binary must feed the Library detail pane, not only the reducer tests.
///
/// Version 0.4 shows parameters, presets, dependencies, and the last run in that pane
/// (`src/skit/tui.py:558-604`) and marks a missing target in the list (`src/skit/tui.py:414`).
/// Every one of those facts comes from the host projection, so a scan-only composition root
/// renders a pane with nothing but the name and kind.
#[test]
fn the_terminal_library_shows_host_projected_detail_facts() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), true);
    fs::create_dir_all(state.path().join("values")).unwrap();
    fs::write(
        state.path().join("values/demo.toml"),
        concat!(
            "[values]\n",
            "name = \"Ada\"\n",
            "[presets.nightly]\n",
            "name = \"Ada\"\n",
            "[last_run]\n",
            "at = \"2026-08-08T00:00:00Z\"\n",
            "exit = 0\n",
        ),
    )
    .unwrap();

    let (code, output) = run_in_pty(&["tui"], data.path(), state.path(), config.path(), &[b"q"]);
    assert_eq!(code, 0, "{output}");
    // Cursor moves sit between rendered words, so assert on tokens rather than whole phrases.
    for fact in [
        "Parameters",
        "name=Ada",
        "Presets",
        "nightly",
        "ago",
        "finished",
    ] {
        assert!(output.contains(fact), "missing {fact}: {output}");
    }
}

/// The Settings parameter summary must use the source's live default without changing any stored
/// data merely because the user inspected it.
#[test]
fn test_settings_param_row_shows_the_sources_live_default() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let (payload, metadata) = write_stale_default_entry(data.path(), home.path());
    let payload_before = fs::read(&payload).unwrap();
    let metadata_before = fs::read(&metadata).unwrap();
    let state_before = tree_snapshot(state.path());
    let config_before = tree_snapshot(config.path());

    let mut tui = LiveTui::spawn(data.path(), state.path(), config.path(), home.path());
    tui.wait_for("greet");
    let settings_checkpoint = tui.checkpoint();
    tui.send_effect_key(b"p");
    let settings = tui.wait_for_after(settings_checkpoint, "NAME");
    assert!(
        settings.contains("Entry settings · greet"),
        "Settings did not open: {settings}"
    );
    let compact = compact_terminal_text(&settings);
    assert!(
        compact.contains("NAMEstr'bonjour'"),
        "the NAME row did not show the exact live-source repr: {settings}"
    );
    assert!(
        !compact.contains("hello"),
        "the stale managed-block default leaked into Settings: {settings}"
    );

    let library_checkpoint = tui.checkpoint();
    tui.send(b"\x1b");
    tui.wait_for_after(library_checkpoint, "greet");
    let quit = tui.checkpoint();
    tui.send(b"q");
    let _ = tui.wait_for_exit_after(quit);
    drop(tui);

    assert_eq!(fs::read(&payload).unwrap(), payload_before);
    assert_eq!(fs::read(&metadata).unwrap(), metadata_before);
    assert_eq!(tree_snapshot(state.path()), state_before);
    assert_eq!(tree_snapshot(config.path()), config_before);
}

#[test]
fn terminal_browser_runs_host_success_error_and_host_quit_paths() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), false);

    let (code, output) = run_in_pty(
        &["tui"],
        data.path(),
        state.path(),
        config.path(),
        &[b"x", b"\x1b[A", b"h", b"\x1b[1;1R", b"\x1b", b"q"],
    );
    assert_eq!(code, 0, "{output}");

    let invalid_config = config.path().join("not-a-directory");
    fs::write(&invalid_config, "file").unwrap();
    // Reads follow version 0.4 and project I/O failures to defaults. The first q
    // must reach the focused Preferences input. Escape returns to the library,
    // where the final q exits.
    let (code, output) = run_in_pty(
        &["tui"],
        data.path(),
        state.path(),
        &invalid_config,
        &[b",", b"\x1b[1;1R", b"q", b"\x1b", b"q"],
    );
    assert_eq!(code, 0, "{output}");

    // Enter opens the run form; Ctrl+R is the run form's explicit run chord
    // (`src/skit/tui_form.py:555` `Binding("ctrl+r", "submit", …)`). Ctrl+S there means
    // "Save as preset" (tui_form.py:548), so it would open a modal and wait for a name.
    fs::write(config.path().join("config.toml"), "after_run = \"exit\"\n").unwrap();
    let (code, output) = run_in_pty(
        &["tui"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\r", b"\x1b[1;1R", b"\x12"],
    );
    assert_eq!(code, 0, "{output}");
}

#[test]
fn terminal_run_form_can_submit_or_cancel_without_plain_input() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), true);

    // Ctrl+R is the run form's explicit run chord (`src/skit/tui_form.py:555`).
    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Ada", b"\x12"],
    );
    assert_eq!(code, 0, "{output}");

    // Ctrl+S opens the preset name modal (`tui_form.py:548`). Enter saves the typed name and
    // returns to the form (`tui_form.py:363-366`), so the run still happens afterwards.
    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Ada", b"\x13", b"nightly", b"\r", b"\x12"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Preset \"nightly\" saved."), "{output}");
    let saved = fs::read_to_string(state.path().join("values/demo.toml")).unwrap();
    assert!(saved.contains("nightly"), "{saved}");
    assert!(saved.contains("Ada"), "{saved}");

    // Escape inside the modal dismisses only the modal (`tui_form.py:376-377`), so the form
    // survives and the following Ctrl+R still runs.
    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Ada", b"\x13", b"\x1b", b"\x12"],
    );
    assert_eq!(code, 0, "{output}");

    let (code, _) = run_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 130);
}

#[test]
fn plain_preset_collection_saves_typed_nonsecrets_in_three_locales() {
    for (locale, environment) in [
        (
            "en",
            "Enter to read it from the environment variable PRESET_TOKEN.",
        ),
        ("zh-CN", "直接按 Enter 就从环境变量 PRESET_TOKEN 读取。"),
        ("zh-TW", "直接按 Enter 就從環境變數 PRESET_TOKEN 讀取。"),
    ] {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write_plain_preset_entry(data.path());
        let mut cli = LiveTui::spawn_command(
            &["preset", "save", "preset-matrix", "night"],
            data.path(),
            state.path(),
            config.path(),
            home.path(),
            locale,
        );
        cli.wait_for("Plain value");
        cli.send(b"\r");
        cli.wait_for("ENABLED");
        cli.send(b"\r");
        cli.wait_for("TOKEN");
        cli.send(b"secret-value\r");
        cli.wait_for("OPTIONAL_SECRET");
        cli.send(b"\r");
        cli.wait_for("COLOR (red/blue)");
        let checkpoint = cli.checkpoint();
        cli.send(b"\r");
        let tail = cli.wait_for_exit_after(checkpoint);
        let output = cli.visible_after(0);
        assert!(output.contains("Plain help row"), "{output}");
        assert!(output.contains(environment), "{output}");
        assert!(output.contains("OPTIONAL_SECRET, TOKEN"), "{output}");
        assert!(tail.contains("night"), "{tail}");
        let saved = fs::read_to_string(state.path().join("values/preset-matrix.toml")).unwrap();
        assert!(saved.contains("PLAIN = \"alpha\""), "{saved}");
        assert!(saved.contains("ENABLED = \"true\""), "{saved}");
        assert!(saved.contains("COLOR = \"red\""), "{saved}");
        assert!(!saved.contains("secret-value"), "{saved}");
        assert!(!saved.contains("TOKEN"), "{saved}");
    }
}

#[test]
fn analyzer_preset_notices_use_real_sources_in_three_locales() {
    for (locale, default_notice, input_notice) in [
        (
            "en",
            "Leave empty to use the script's own default.",
            "Leave empty and the script will ask you in the terminal.",
        ),
        (
            "zh-CN",
            "留空＝用脚本自己的默认值。",
            "留空＝运行时脚本自己在终端问你。",
        ),
        (
            "zh-TW",
            "留空＝用腳本自己的預設。",
            "留空＝執行時腳本自己在終端機問你。",
        ),
    ] {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write_analyzer_preset_entries(data.path());

        let javascript = data.path().join("scripts/degraded-analyzer/script.js");
        let shell = data.path().join("scripts/input-analyzer/script.sh");
        let javascript_before = fs::read(&javascript).unwrap();
        let shell_before = fs::read(&shell).unwrap();

        let mut degraded = LiveTui::spawn_command(
            &["preset", "save", "degraded-analyzer", "degraded"],
            data.path(),
            state.path(),
            config.path(),
            home.path(),
            locale,
        );
        let output = degraded.wait_for("name");
        assert!(output.contains(default_notice), "{output}");
        let checkpoint = degraded.checkpoint();
        degraded.send(b"value\r");
        let tail = degraded.wait_for_exit_after(checkpoint);
        assert!(tail.contains("degraded"), "{tail}");

        let mut input = LiveTui::spawn_command(
            &["preset", "save", "input-analyzer", "input"],
            data.path(),
            state.path(),
            config.path(),
            home.path(),
            locale,
        );
        let output = input.wait_for("Name:");
        assert!(output.contains(input_notice), "{output}");
        let checkpoint = input.checkpoint();
        input.send(b"Ada\r");
        let tail = input.wait_for_exit_after(checkpoint);
        assert!(tail.contains("input"), "{tail}");

        assert_eq!(fs::read(&javascript).unwrap(), javascript_before);
        assert_eq!(fs::read(&shell).unwrap(), shell_before);
        let degraded_state =
            fs::read_to_string(state.path().join("values/degraded-analyzer.toml")).unwrap();
        let input_state =
            fs::read_to_string(state.path().join("values/input-analyzer.toml")).unwrap();
        assert!(
            degraded_state.contains("name = \"value\""),
            "{degraded_state}"
        );
        assert!(input_state.contains("input-1 = \"Ada\""), "{input_state}");
    }
}

#[test]
fn terminal_authoring_and_confirmation_paths_need_no_hidden_cli_knowledge() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("source.sh");
    fs::write(&source, "echo ok\n").unwrap();
    let source_input = source.display().to_string().into_bytes();

    // The source step advertises Enter Continue; the review panel that follows advertises
    // Ctrl+S Add. The panel arrives after a host round trip, which re-enters the terminal and
    // asks for the cursor position again.
    let (code, output) = run_in_pty(
        &["add"],
        data.path(),
        state.path(),
        config.path(),
        &[source_input.as_slice(), b"\r", b"\x1b[1;1R", b"\x13"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(data.path().join("scripts/source/meta.toml").is_file());

    let editor = data.path().join("editor.sh");
    fs::write(
        &editor,
        "#!/bin/sh\nsleep 0.2\nprintf '#!/usr/bin/env python3\\nprint(1)\\n' > \"$1\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        config.path().join("config.toml"),
        format!("editor = {:?}\n", editor.display().to_string()),
    )
    .unwrap();

    let (code, output) = run_plain_in_pty(
        &["add", "--prompt", "--name", "Prompt"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    let (code, output) = run_plain_in_pty(
        &["add", "--prompt", "--name", "No Body", "--no-input"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 2, "{output}");

    let (code, output) = run_plain_in_pty(
        &["edit", "Declined"],
        data.path(),
        state.path(),
        config.path(),
        &[b"n\n"],
    );
    assert_eq!(code, 0, "{output}");
    let (code, output) = run_plain_in_pty(
        &["edit", "End of Input"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x04"],
    );
    assert_eq!(code, 130, "{output}");
    let (code, output) = run_plain_in_pty(
        &["edit", "New Script"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(data.path().join("scripts/new-script/meta.toml").is_file());

    let (code, output) = run_plain_in_pty(
        &["remove", "new-script"],
        data.path(),
        state.path(),
        config.path(),
        &[b"y\n"],
    );
    assert_eq!(code, 0, "{output}");
    let (code, output) = run_plain_in_pty(
        &["remove", "prompt"],
        data.path(),
        state.path(),
        config.path(),
        &[b"n\n"],
    );
    assert_eq!(code, 130, "{output}");
    let (code, output) = run_plain_in_pty(
        &["remove", "prompt"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x04"],
    );
    assert_eq!(code, 130, "{output}");

    let (code, output) = run_plain_in_pty(
        &["runner", "remove", "codex"],
        data.path(),
        state.path(),
        config.path(),
        &[b"y\n"],
    );
    assert_eq!(code, 0, "{output}");
    let (code, output) = run_plain_in_pty(
        &["runner", "remove", "claude"],
        data.path(),
        state.path(),
        config.path(),
        &[b"n\n"],
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("operation cancelled"), "{output}");
    assert!(
        FileConfigStore::new(config.path())
            .runners()
            .unwrap()
            .iter()
            .any(|runner| runner.name == "claude"),
        "negative confirmation removed claude: {output}"
    );
    let (code, output) = run_plain_in_pty(
        &["runner", "remove", "amp"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x04"],
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("operation cancelled"), "{output}");
    assert!(
        FileConfigStore::new(config.path())
            .runners()
            .unwrap()
            .iter()
            .any(|runner| runner.name == "amp"),
        "end of input removed amp: {output}"
    );

    let values = state.path().join("values");
    fs::create_dir_all(&values).unwrap();
    fs::write(
        values.join("prompt.toml"),
        "[presets.old]\n[ presets.keep ]\n",
    )
    .unwrap();
    let (code, output) = run_plain_in_pty(
        &["preset", "delete", "prompt", "old"],
        data.path(),
        state.path(),
        config.path(),
        &[b"y\n"],
    );
    assert_eq!(code, 0, "{output}");
    let (code, output) = run_plain_in_pty(
        &["preset", "delete", "prompt", "keep"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 0, "{output}");
}

#[test]
fn interactive_preset_save_collects_current_values_instead_of_saving_the_prefill_unasked() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), true);
    let _ = run_plain_in_pty(
        &["doctor", "--rebuild"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );

    let (code, output) = run_plain_in_pty(
        &["preset", "save", "demo", "favorite"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Grace\n"],
    );
    assert_eq!(code, 0, "{output}");
    let saved = fs::read_to_string(state.path().join("values/demo.toml")).unwrap();
    assert!(saved.contains("[presets.favorite]"), "{saved}");
    assert!(saved.contains("name = \"Grace\""), "{saved}");
}

#[test]
fn add_onboarding_accepts_clean_defaults_and_leaves_demoted_candidates_unmanaged() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    // The plain lane, named outright. `form = "tui"` sends `skit add <path>` to the review
    // panel, whose own tick list is this onboarding's twin
    // (`src/skit/cli.py:2116-2126`); these tests are about the line-prompt half.
    fs::write(config.path().join("config.toml"), "form = \"plain\"\n").unwrap();
    let source = data.path().join("parameters.py");
    fs::write(
        &source,
        "KEEP = 1\nCOUNT = 0\nCOUNT += 1\nprint(KEEP, COUNT)\n",
    )
    .unwrap();
    let source = source.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &source, "--name", "Parameters"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\r"],
    );
    assert_eq!(code, 0, "{output}");
    let stored = fs::read_to_string(data.path().join("scripts/parameters/script.py")).unwrap();
    assert!(stored.contains("name = \"KEEP\""), "{stored}");
    assert!(!stored.contains("name = \"COUNT\""), "{stored}");
}

#[test]
fn add_onboarding_space_toggles_the_focused_checkbox() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    // The plain lane, named outright. `form = "tui"` sends `skit add <path>` to the review
    // panel, whose own tick list is this onboarding's twin
    // (`src/skit/cli.py:2116-2126`); these tests are about the line-prompt half.
    fs::write(config.path().join("config.toml"), "form = \"plain\"\n").unwrap();
    let source = data.path().join("toggle.py");
    fs::write(&source, "VALUE = 1\nprint(VALUE)\n").unwrap();
    let source = source.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &source, "--name", "Toggle"],
        data.path(),
        state.path(),
        config.path(),
        &[b" \r"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Space toggles"), "{output}");
    let stored = fs::read_to_string(data.path().join("scripts/toggle/script.py")).unwrap();
    assert!(!stored.contains("[tool.skit]"), "{stored}");
}

#[test]
fn an_empty_onboarding_selection_does_not_delete_existing_managed_metadata() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    // The plain lane, named outright. `form = "tui"` sends `skit add <path>` to the review
    // panel, whose own tick list is this onboarding's twin
    // (`src/skit/cli.py:2116-2126`); these tests are about the line-prompt half.
    fs::write(config.path().join("config.toml"), "form = \"plain\"\n").unwrap();
    let source = data.path().join("existing.py");
    let original = concat!(
        "# /// script\n",
        "# dependencies = []\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"VALUE\"\n",
        "# kind = \"const\"\n",
        "# type = \"int\"\n",
        "# default = 1\n",
        "# ///\n",
        "VALUE = 1\n",
        "print(VALUE)\n",
    );
    fs::write(&source, original).unwrap();
    let source = source.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &source, "--name", "Existing"],
        data.path(),
        state.path(),
        config.path(),
        &[b" \r"],
    );
    assert_eq!(code, 0, "{output}");
    let stored = fs::read_to_string(data.path().join("scripts/existing/script.py")).unwrap();
    assert_eq!(stored, original);
}

#[test]
fn add_onboarding_distinguishes_modeled_and_dynamic_cli_surfaces() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    // The plain lane, named outright. `form = "tui"` sends `skit add <path>` to the review
    // panel, whose own tick list is this onboarding's twin
    // (`src/skit/cli.py:2116-2126`); these tests are about the line-prompt half.
    fs::write(config.path().join("config.toml"), "form = \"plain\"\n").unwrap();
    let modeled = data.path().join("modeled.py");
    fs::write(
        &modeled,
        concat!(
            "VALUE = 1\n",
            "import argparse\n",
            "p = argparse.ArgumentParser()\n",
            "p.add_argument('--name')\n",
        ),
    )
    .unwrap();
    let modeled = modeled.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &modeled, "--name", "Modeled"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("skit read this script's own arguments"),
        "{output}"
    );
    let stored = fs::read_to_string(data.path().join("scripts/modeled/script.py")).unwrap();
    assert!(!stored.contains("[tool.skit]"), "{stored}");

    let dynamic = data.path().join("dynamic.py");
    fs::write(
        &dynamic,
        concat!(
            "VALUE = 1\n",
            "import argparse\n",
            "p = argparse.ArgumentParser()\n",
            "p.add_subparsers()\n",
        ),
    )
    .unwrap();
    let dynamic = dynamic.to_string_lossy().into_owned();
    let (code, output) = run_plain_in_pty(
        &["add", &dynamic, "--name", "Dynamic"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\r"],
    );
    assert_eq!(code, 0, "{output}");
    let stored = fs::read_to_string(data.path().join("scripts/dynamic/script.py")).unwrap();
    assert!(stored.contains("name = \"VALUE\""), "{stored}");
}

#[test]
fn reference_add_reports_onboarding_but_never_writes_the_original() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    // The plain lane, named outright. `form = "tui"` sends `skit add <path>` to the review
    // panel, whose own tick list is this onboarding's twin
    // (`src/skit/cli.py:2116-2126`); these tests are about the line-prompt half.
    fs::write(config.path().join("config.toml"), "form = \"plain\"\n").unwrap();
    let source = data.path().join("reference.py");
    let original = b"VALUE = 1\n";
    fs::write(&source, original).unwrap();
    let source_arg = source.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &source_arg, "--name", "Reference", "--ref"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("parameter setup was skipped"), "{output}");
    assert_eq!(fs::read(source).unwrap(), original);
}

#[test]
fn terminal_plain_launch_menu_uses_the_same_prefill_and_argument_contract() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), true);
    fs::write(config.path().join("config.toml"), "form = \"plain\"\n").unwrap();

    let (code, output) = run_plain_in_pty(
        &[
            "run",
            "demo",
            "--set",
            "name=Grace",
            "--",
            "two words",
            "single",
        ],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Grace"), "{output}");

    let (code, output) = run_plain_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("two words"), "{output}");
    assert!(output.contains("single"), "{output}");

    let (code, output) = run_plain_in_pty(
        &["run", "demo", "--dry-run", "--forget-args"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(!output.contains("two words"), "{output}");
    assert!(!output.contains("single"), "{output}");

    let (code, output) = run_plain_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x04"],
    );
    assert_eq!(code, 130, "{output}");

    write_secret_command_entry(data.path());
    let (code, output) = run_plain_in_pty(
        &["run", "secret", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Ada\n", b"private-token\n", b"\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(!output.contains("private-token"), "{output}");

    write_pinned_prompt_entry(data.path());
    fs::write(
        config.path().join("config.toml"),
        concat!(
            "form = \"plain\"\n",
            "[prompt]\n",
            "runners_seeded = true\n",
            "[[prompt.runners]]\n",
            "name = \"backup\"\n",
            "argv = [\"printf\", \"{{prompt}}\"]\n",
            "[[prompt.runners]]\n",
            "name = \"local\"\n",
            "argv = [\"sh\", \"-c\", \"printf %s\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    let (code, output) = run_plain_in_pty(
        &["run", "pinned-prompt", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("Prompt runner choices: backup, local [local]:"),
        "{output}"
    );
}
