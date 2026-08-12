//! Source-of-truth ports for the CLI/Settings tail of Python `tests/test_reset_default_ui.py`.
//! The stored managed block deliberately says `hello` while the live Python assignment says
//! `bonjour`; all three public surfaces must agree with the source, never the stale block cache.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::PathBuf,
    process::{Command, Output},
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::{CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_language::write_managed_params;
use skit_store::FileStore;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .current_dir(self.home.path());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn seed_stale_default(&self) -> PathBuf {
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
        let source = self.home.path().join("greet.py");
        fs::write(&source, managed.as_bytes()).unwrap();
        let store = self.store();
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
        assert!(current.contains("hello"));
        fs::write(
            &payload,
            current.replace("NAME = \"hello\"", "NAME = \"bonjour\""),
        )
        .unwrap();
        payload
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_params_default_column_shows_the_sources_live_value() {
    let fixture = Fixture::new();
    fixture.seed_stale_default();

    let output = fixture.run(&["params", "greet"]);

    assert!(output.status.success(), "{}", combined(&output));
    let text = combined(&output);
    assert!(text.contains("bonjour"), "live source default missing: {text}");
    assert!(!text.contains("hello"), "stale managed-block default leaked: {text}");
}

#[test]
fn test_show_json_delivers_empty_true_for_str_const_false_for_int() {
    let fixture = Fixture::new();
    fixture.seed_stale_default();

    let output = fixture.run(&["show", "greet", "--json"]);

    assert!(output.status.success(), "{}", combined(&output));
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let fields = payload["fields"].as_array().expect("show --json fields array");
    let by_key = |key: &str| {
        fields
            .iter()
            .find(|field| field["key"] == key)
            .unwrap_or_else(|| panic!("missing field {key}: {payload}"))
    };
    assert_eq!(by_key("NAME")["delivers_empty"], true);
    assert_eq!(by_key("COUNT")["delivers_empty"], false);
    assert_eq!(by_key("NAME")["default"], "bonjour");
}

#[test]
fn test_settings_param_row_shows_the_sources_live_default() {
    let fixture = Fixture::new();
    let payload = fixture.seed_stale_default();
    let before = fs::read(&payload).unwrap();

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 36,
            cols: 132,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.arg("tui");
    command.cwd(fixture.home.path());
    command.env("TERM", "xterm-256color");
    command.env("SKIT_DATA_DIR", fixture.data.path());
    command.env("SKIT_STATE_DIR", fixture.state.path());
    command.env("SKIT_CONFIG_DIR", fixture.config.path());
    command.env("SKIT_LANG", "en");
    command.env("HOME", fixture.home.path());
    command.env("USERPROFILE", fixture.home.path());
    command.env("XDG_CONFIG_HOME", fixture.home.path().join("xdg-config"));
    command.env("XDG_DATA_HOME", fixture.home.path().join("xdg-data"));
    command.env("XDG_STATE_HOME", fixture.home.path().join("xdg-state"));

    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(60));
    writer.write_all(b"\x1b[1;1R").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(180));
    writer.write_all(b"s").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(280));
    writer.write_all(b"\x1b").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(80));
    writer.write_all(b"q").unwrap();
    writer.flush().unwrap();

    let status = child.wait().unwrap();
    drop(writer);
    let text = String::from_utf8_lossy(&drain.join().unwrap())
        .replace("\r\n", "\n")
        .replace('\r', "");

    assert_eq!(status.exit_code(), 0, "{text}");
    assert!(text.contains("bonjour"), "Settings did not show the live source default: {text}");
    assert!(!text.contains("'hello'"), "Settings showed the stale managed-block default: {text}");
    assert_eq!(fs::read(&payload).unwrap(), before, "opening Settings mutated source bytes");
}
