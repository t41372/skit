#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, SourcePermissions,
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery},
};
use skit_language::write_managed_params;
use skit_store::FileStore;
use tempfile::TempDir;

fn delayed_interpreter(directory: &std::path::Path) -> std::path::PathBuf {
    let interpreter = directory.join("delayed-shell");
    fs::write(
        &interpreter,
        concat!(
            "#!/bin/sh\n",
            ": > \"$SKIT_TEST_READY\"\n",
            "while [ ! -f \"$SKIT_TEST_RELEASE\" ]; do sleep 0.01; done\n",
            "exec /bin/sh \"$1\"\n",
        ),
    )
    .unwrap();
    fs::set_permissions(&interpreter, fs::Permissions::from_mode(0o700)).unwrap();
    interpreter
}

fn create_shell(
    store: &FileStore,
    interpreter: &std::path::Path,
    bytes: &[u8],
) -> skit_domain::Entry {
    let settings = EntrySettings {
        interpreter: interpreter.display().to_string(),
        ..EntrySettings::default()
    };
    store
        .create(CreateEntry {
            name: "Demo".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: "/original/demo.sh".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: bytes.to_vec(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings,
        })
        .unwrap()
}

fn run_command(
    data: &std::path::Path,
    state: &std::path::Path,
    config: &std::path::Path,
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
    command
        .env("SKIT_DATA_DIR", data)
        .env("SKIT_STATE_DIR", state)
        .env("SKIT_CONFIG_DIR", config);
    command
}

fn wait_until_ready(ready: &std::path::Path) -> bool {
    (0..500).any(|_| {
        if ready.is_file() {
            true
        } else {
            thread::sleep(Duration::from_millis(10));
            false
        }
    })
}

#[test]
fn a_delayed_interpreter_reads_the_identity_checked_snapshot() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let signals = TempDir::new().unwrap();
    let ready = signals.path().join("ready");
    let release = signals.path().join("release");
    let interpreter = delayed_interpreter(signals.path());

    let store = FileStore::new(data.path());
    let entry = create_shell(&store, &interpreter, b"printf OLD");

    let child = run_command(data.path(), state.path(), config.path())
        .args(["run", "demo", "--no-input"])
        .env("SKIT_TEST_READY", &ready)
        .env("SKIT_TEST_RELEASE", &release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let became_ready = wait_until_ready(&ready);
    if !became_ready {
        fs::write(&release, []).unwrap();
        let output = child.wait_with_output().unwrap();
        panic!(
            "delayed interpreter did not start: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    store
        .commit_copy_edit(&entry, b"printf NEW", &entry.meta.source_hash)
        .unwrap();
    fs::write(&release, []).unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success(), "{output:?}");
    // A run prints one transparency line on stdout before the child writes anything
    // (`src/skit/flows.py:931` appends `"→ " + env_prefix + described`, and
    // `src/skit/cli.py:3218` emits it with the stdout console). The line names the private
    // staged snapshot, and everything after it is the child's own output.
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (transparency, child_output) = stdout
        .split_once('\n')
        .unwrap_or_else(|| panic!("no transparency line in {stdout:?}"));
    assert!(transparency.starts_with("→ "), "{stdout:?}");
    assert!(transparency.contains("/.run-"), "{stdout:?}");
    assert_eq!(child_output, "OLD", "{stdout:?}");
    assert_eq!(
        fs::read(store.payload_path(&store.resolve("demo").unwrap()).unwrap()).unwrap(),
        b"printf NEW"
    );
}

#[test]
fn remove_and_readd_wait_until_post_run_state_is_committed_and_forgotten() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let signals = TempDir::new().unwrap();
    let ready = signals.path().join("ready");
    let release = signals.path().join("release");
    let interpreter = delayed_interpreter(signals.path());
    let store = FileStore::new(data.path());
    create_shell(&store, &interpreter, b"exit 0");

    let child = run_command(data.path(), state.path(), config.path())
        .args(["run", "demo", "--no-input"])
        .env("SKIT_TEST_READY", &ready)
        .env("SKIT_TEST_RELEASE", &release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert!(
        wait_until_ready(&ready),
        "delayed interpreter did not start"
    );

    let data_path = data.path().to_path_buf();
    let state_path = state.path().to_path_buf();
    let config_path = config.path().to_path_buf();
    let replacement_interpreter = interpreter.clone();
    let (readded, readded_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let output = run_command(&data_path, &state_path, &config_path)
            .args(["remove", "demo", "--yes"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        create_shell(
            &FileStore::new(&data_path),
            &replacement_interpreter,
            b"printf REPLACEMENT",
        );
        readded.send(()).unwrap();
    });

    assert!(
        readded_rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "remove/re-add completed while the original launch was still active"
    );
    fs::write(&release, []).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    readded_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    worker.join().unwrap();

    assert!(store.resolve("demo").is_ok());
    assert!(
        !state.path().join("values/demo.toml").exists(),
        "the old run state was grafted onto the replacement entry"
    );
}

#[test]
fn a_run_that_started_public_cannot_restore_plaintext_after_the_field_becomes_secret() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let signals = TempDir::new().unwrap();
    let ready = signals.path().join("ready");
    let release = signals.path().join("release");
    let interpreter = delayed_interpreter(signals.path());
    let store = FileStore::new(data.path());
    let mut token = ParamDecl::new("TOKEN");
    token.binding = ParameterBinding::Const;
    token.delivery = ParameterDelivery::Inject;
    let source =
        write_managed_params("shell", "TOKEN=default\nprintf '%s' \"$TOKEN\"\n", &[token]).unwrap();
    create_shell(&store, &interpreter, source.as_bytes());

    let child = run_command(data.path(), state.path(), config.path())
        .args([
            "run",
            "demo",
            "--no-input",
            "--set",
            "TOKEN=plaintext-race-value",
        ])
        .env("SKIT_TEST_READY", &ready)
        .env("SKIT_TEST_RELEASE", &release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert!(
        wait_until_ready(&ready),
        "delayed interpreter did not start"
    );

    let edit = run_command(data.path(), state.path(), config.path())
        .args(["params", "demo", "--secret", "TOKEN"])
        .output()
        .unwrap();
    assert!(edit.status.success(), "{edit:?}");
    fs::write(&release, []).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");

    let state_path = state.path().join("values/demo.toml");
    let state = fs::read_to_string(&state_path).unwrap();
    assert!(!state.contains("TOKEN"), "{state}");
    assert!(!state.contains("plaintext-race-value"), "{state}");
}
