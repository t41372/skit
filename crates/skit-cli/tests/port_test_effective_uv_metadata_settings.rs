//! Real-TUI Settings ports from Python `tests/test_effective_uv_metadata.py` and
//! `tests/test_uv_metadata_unpinning.py` at `main@206f9ef`.
//!
//! These are intentionally not reducer-only checks. The entry is created through the real CLI,
//! Settings is opened through an actual `skit tui` PTY, edits travel through the terminal widget,
//! and the final source is read back from the store. A block-only dependency axis therefore has to
//! survive the entire store -> host -> UI -> save path.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::PathBuf,
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_language::read_uv_metadata;
use skit_ui::{
    DEPENDENCIES_KEY, DependencyFlavor, NAME_KEY, PYTHON_KEY, SettingsInputs, SettingsView,
};
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
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
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .current_dir(self.home.path());
        command
    }

    fn add_plain(&self) {
        let source = self.home.path().join("x.py");
        fs::write(&source, "print(1)\n").unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", "x", "--no-input"])
            .assert()
            .success();
    }

    fn add_block_only(&self) {
        let source = self.home.path().join("x.py");
        fs::write(&source, "print(1)\n").unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args([
                "--name",
                "x",
                "--dep",
                "requests",
                "--python",
                ">=3.11",
                "--no-input",
            ])
            .assert()
            .success();

        let meta = fs::read_to_string(self.data.path().join("scripts/x/meta.toml")).unwrap();
        assert!(
            !meta
                .lines()
                .any(|line| line.trim_start().starts_with("dependencies =")),
            "fixture must keep dependencies block-only: {meta}"
        );
        assert!(
            !meta
                .lines()
                .any(|line| line.trim_start().starts_with("requires_python =")),
            "fixture must keep the Python constraint block-only: {meta}"
        );
        let effective = read_uv_metadata(&self.stored_source()).expect("fixture PEP 723 block");
        assert_eq!(effective.dependencies, ["requests"]);
        assert_eq!(effective.requires_python, ">=3.11");
    }

    fn add_meta_pinned(&self) {
        self.add_plain();
        // A post-add deps edit is the oracle's meta-carried branch: unlike add-time injection, the
        // stored record owns the pin as well as the synchronized PEP 723 block.
        self.command()
            .args(["deps", "x", "--dep", "requests", "--python", ">=3.11"])
            .assert()
            .success();
        let meta = fs::read_to_string(self.data.path().join("scripts/x/meta.toml")).unwrap();
        assert!(
            meta.contains("requires_python = \">=3.11\""),
            "fixture must carry the pin in meta.toml: {meta}"
        );
        let effective = read_uv_metadata(&self.stored_source()).expect("fixture PEP 723 block");
        assert_eq!(effective.dependencies, ["requests"]);
        assert_eq!(effective.requires_python, ">=3.11");
    }

    fn stored_source_path(&self) -> PathBuf {
        self.data.path().join("scripts/x/script.py")
    }

    fn stored_source(&self) -> String {
        fs::read_to_string(self.stored_source_path()).unwrap()
    }

    fn run_settings(&self, inputs: &[Vec<u8>]) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 130,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.arg("tui");
        command.env("TERM", "xterm-256color");
        command.env("SKIT_LANG", "en");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("HOME", self.home.path());
        command.env("USERPROFILE", self.home.path());
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
        thread::sleep(Duration::from_millis(220));
        writer.write_all(b"p").unwrap();
        writer.flush().unwrap();
        thread::sleep(Duration::from_millis(300));
        for input in inputs {
            let _ = writer.write_all(input);
            let _ = writer.flush();
            thread::sleep(Duration::from_millis(220));
        }
        let _ = writer.write_all(b"\x1b");
        let _ = writer.flush();
        thread::sleep(Duration::from_millis(120));
        let _ = writer.write_all(b"q");
        let _ = writer.flush();

        let status = child.wait().unwrap();
        drop(writer);
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }
}

/// Reconstruct the typed Settings surface for the deliberately-simple Python-copy fixture above.
///
/// This is used only to derive navigation from stable field keys. The behavioral assertions still
/// run against the real PTY/store path below. Keeping the Tab distance model-derived avoids a
/// brittle magic number while also pinning the exact surface the fixture is supposed to expose.
fn settings_navigation_model() -> SettingsView {
    SettingsView::from_inputs(&SettingsInputs {
        selector: "x".to_owned(),
        kind: "python".to_owned(),
        name: "x".to_owned(),
        source: "x.py".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        has_analyzer: true,
        dependency_flavor: Some(DependencyFlavor::Uv),
        effective_dependencies: vec!["requests".to_owned()],
        effective_requires_python: ">=3.11".to_owned(),
        ..SettingsInputs::default()
    })
}

fn tabs_to(target: &str) -> usize {
    let view = settings_navigation_model();
    assert_eq!(view.focused(), NAME_KEY);
    let keys = view.focusable_keys();
    let index = keys
        .iter()
        .position(|key| *key == target)
        .unwrap_or_else(|| panic!("fixture Settings surface has no {target:?} field: {keys:?}"));
    assert!(index > 0, "target field must not already own initial focus");
    index
}

fn replace_focused_line(target: &str, old_value: &str, new_value: &str) -> Vec<Vec<u8>> {
    let mut inputs = (0..tabs_to(target))
        .map(|_| b"\t".to_vec())
        .collect::<Vec<_>>();
    // xterm End. This makes deletion independent of where `tui-input` initializes the cursor.
    inputs.push(b"\x1b[F".to_vec());
    inputs.extend((0..old_value.chars().count()).map(|_| vec![0x7f]));
    if !new_value.is_empty() {
        inputs.push(new_value.as_bytes().to_vec());
    }
    inputs.push(b"\x13".to_vec()); // Ctrl+S
    inputs
}

#[test]
fn test_settings_prefills_deps_and_python_from_the_block() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only();

    let (code, output) = sandbox.run_settings(&[]);
    assert_eq!(code, 0, "{output}");
    // This is the terminal output of the real Settings screen. Both values must be present even
    // though meta.toml deliberately carries neither axis.
    assert!(
        output.contains("requests"),
        "block-only dependency was not rendered: {output}"
    );
    assert!(
        output.contains(">=3.11"),
        "block-only Python constraint was not rendered: {output}"
    );
}

#[test]
fn test_settings_deps_only_edit_preserves_the_block_pin() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only();

    let inputs = replace_focused_line(DEPENDENCIES_KEY, "requests", "requests, rich");
    let (code, output) = sandbox.run_settings(&inputs);
    assert_eq!(code, 0, "{output}");

    let source = sandbox.stored_source();
    let effective = read_uv_metadata(&source).expect("saved PEP 723 block");
    assert_eq!(
        effective.dependencies,
        ["requests", "rich"],
        "the dependency-only Settings edit did not land exactly"
    );
    assert_eq!(
        effective.requires_python, ">=3.11",
        "editing dependencies unpinned the untouched block-only Python constraint"
    );
    assert!(source.contains("requires-python = \">=3.11\""));
}

#[test]
fn test_settings_clearing_python_on_block_only_entry_unpins() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only();

    let inputs = replace_focused_line(PYTHON_KEY, ">=3.11", "");
    let (code, output) = sandbox.run_settings(&inputs);
    assert_eq!(code, 0, "{output}");

    let source = sandbox.stored_source();
    let effective = read_uv_metadata(&source).expect("dependency block must survive unpinning");
    assert_eq!(
        effective.dependencies,
        ["requests"],
        "clearing only Python constraint changed the dependency axis"
    );
    assert_eq!(effective.requires_python, "");
    assert!(
        !source.contains("requires-python"),
        "clearing the visibly-prefilled Settings field did not remove the block pin: {source}"
    );
}

#[test]
fn test_settings_untouched_save_never_touches_the_deps_axis() {
    let sandbox = Sandbox::new();
    sandbox.add_block_only();
    let before = fs::read(sandbox.stored_source_path()).unwrap();

    let (code, output) = sandbox.run_settings(&[b"\x13".to_vec()]); // Ctrl+S, no field edits.
    assert_eq!(code, 0, "{output}");

    let after = fs::read(sandbox.stored_source_path()).unwrap();
    assert_eq!(
        after, before,
        "an untouched Settings save rewrote the PEP 723 source block"
    );
    let effective = read_uv_metadata(std::str::from_utf8(&after).unwrap()).expect("PEP 723 block");
    assert_eq!(effective.dependencies, ["requests"]);
    assert_eq!(effective.requires_python, ">=3.11");
}

#[test]
fn test_settings_clearing_python_unpins_the_block() {
    let sandbox = Sandbox::new();
    sandbox.add_meta_pinned();

    let inputs = replace_focused_line(PYTHON_KEY, ">=3.11", "");
    let (code, output) = sandbox.run_settings(&inputs);
    assert_eq!(code, 0, "{output}");

    let source = sandbox.stored_source();
    let effective =
        read_uv_metadata(&source).expect("dependency block must survive Settings unpin");
    assert_eq!(effective.dependencies, ["requests"]);
    assert_eq!(effective.requires_python, "");
    assert!(
        !source.contains("requires-python"),
        "Settings cleared meta but left uv's authoritative block pinned: {source}"
    );

    let meta = fs::read_to_string(sandbox.data.path().join("scripts/x/meta.toml")).unwrap();
    assert!(
        !meta
            .lines()
            .any(|line| line.trim_start().starts_with("requires_python =")),
        "Settings unpin left the meta constraint behind: {meta}"
    );
    let view = sandbox
        .command()
        .args(["deps", "x", "--json"])
        .output()
        .unwrap();
    assert!(
        view.status.success(),
        "{}",
        String::from_utf8_lossy(&view.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&view.stdout).unwrap();
    assert_eq!(payload["requires_python"], "");
}
