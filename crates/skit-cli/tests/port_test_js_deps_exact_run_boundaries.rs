//! Exact process/filesystem ports from Python v0.4 `tests/test_js_deps.py`.
#![cfg(unix)]

use std::{
    fs,
    fs::FileTimes,
    path::{Path, PathBuf},
    time::SystemTime,
};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _,
    SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_language::{ParseOutcome, parse_document, write_managed_params};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    bin: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            bin: TempDir::new().unwrap(),
        };
        sandbox.install_program("node", "#!/bin/sh\nif [ -n \"${SKIT_TEST_NODE_PATH:-}\" ]; then printf '%s' \"$1\" > \"$SKIT_TEST_NODE_PATH\"; fi\nexit 0\n");
        sandbox.install_program("npm", "#!/bin/sh\nmkdir -p node_modules\nexit 0\n");
        sandbox
    }

    fn install_program(&self, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.bin.path().join(name);
        fs::write(&path, body).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
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
            .env("PATH", self.bin.path())
            .current_dir(self.home.path());
        command
    }

    fn create_js(&self, name: &str, source_name: &str, body: &str, dependencies: &[&str]) {
        let kind = EntryKind::parse("js").unwrap();
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: kind.clone(),
                mode: StorageMode::Copy,
                source: source_name.to_owned(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: body.as_bytes().to_vec(),
                    stored_name: Some(payload_stored_name(&kind, Path::new(source_name))),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    dependencies: dependencies.iter().map(|item| (*item).to_owned()).collect(),
                    interpreter: "node".to_owned(),
                    ..EntrySettings::default()
                },
            })
            .unwrap();
    }

    fn managed_source() -> String {
        let source = "const M = \"x\";\nconsole.log(M);\n";
        let ParseOutcome::Parsed(document) = parse_document("js", source) else {
            panic!("managed JS fixture must parse");
        };
        let declarations = document
            .analysis()
            .candidates
            .into_iter()
            .map(|candidate| candidate.declaration)
            .collect::<Vec<_>>();
        assert_eq!(declarations.len(), 1);
        write_managed_params("js", source, &declarations).unwrap()
    }

    fn entry_dir(&self, name: &str) -> PathBuf {
        let store = self.store();
        let entry = store.resolve(name).unwrap();
        store.entry_dir_path(&entry.slug)
    }

    fn run_capture_path(&self, name: &str, sets: &[&str]) -> (std::process::Output, PathBuf) {
        let marker = self.home.path().join(format!("{name}.launched"));
        let mut command = self.command();
        command.env("SKIT_TEST_NODE_PATH", &marker).args(["run", name]);
        for set in sets {
            command.args(["--set", set]);
        }
        command.arg("--no-input");
        let output = command.output().unwrap();
        let path = fs::read_to_string(&marker)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::new());
        (output, path)
    }
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_build_sweeps_aged_injected_leftovers_but_not_fresh_ones() {
    let sandbox = Sandbox::new();
    sandbox.create_js("t", "t.js", "console.log(1);\n", &[]);
    let entry_dir = sandbox.entry_dir("t");
    let aged = entry_dir.join(".injected-dead.js");
    let fresh = entry_dir.join(".injected-live.js");
    fs::write(&aged, "x").unwrap();
    fs::write(&fresh, "x").unwrap();
    let file = fs::File::options().write(true).open(&aged).unwrap();
    file.set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
        .unwrap();

    let (output, _) = sandbox.run_capture_path("t", &[]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(!aged.exists(), "aged injected crash leftover survived build: {text}");
    assert!(fresh.exists(), "fresh concurrent injected copy was swept: {text}");
}

#[test]
fn test_write_injected_prefers_entry_dir_when_asked() {
    let sandbox = Sandbox::new();
    sandbox.create_js("t", "t.js", &Sandbox::managed_source(), &["chalk"]);
    let entry_dir = sandbox.entry_dir("t");
    let (output, launched) = sandbox.run_capture_path("t", &["M=y"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(launched.parent(), Some(entry_dir.as_path()), "{text}");
}

#[test]
fn test_js_injector_honors_prefer_entry_dir() {
    let sandbox = Sandbox::new();
    sandbox.create_js("t", "t.js", &Sandbox::managed_source(), &["chalk"]);
    let entry_dir = sandbox.entry_dir("t");
    let (output, launched) = sandbox.run_capture_path("t", &["M=y"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(launched.parent(), Some(entry_dir.as_path()), "{text}");
    assert!(!launched.exists(), "injected launch copy survived cleanup: {}", launched.display());
}

#[test]
fn test_flows_marks_prefer_entry_dir_only_for_deps_managed_npm_copies() {
    for (dependencies, expect_entry_dir) in [(&["chalk"][..], true), (&[][..], false)] {
        let sandbox = Sandbox::new();
        sandbox.create_js("t", "t.js", &Sandbox::managed_source(), dependencies);
        let entry_dir = sandbox.entry_dir("t");
        let (output, launched) = sandbox.run_capture_path("t", &["M=y"]);
        let text = combined(&output);
        assert_eq!(output.status.code(), Some(0), "dependencies={dependencies:?}\n{text}");
        assert_eq!(
            launched.parent() == Some(entry_dir.as_path()),
            expect_entry_dir,
            "dependencies={dependencies:?}; launched={}; {text}",
            launched.display()
        );
    }
}

#[test]
fn test_build_passes_the_original_extensions_module_type() {
    for (extension, expected_type) in [
        ("mjs", "module"),
        ("mts", "module"),
        ("cjs", "commonjs"),
        ("cts", "commonjs"),
    ] {
        let sandbox = Sandbox::new();
        sandbox.create_js("t", &format!("t.{extension}"), "console.log(1);\n", &["chalk"]);
        let (output, _) = sandbox.run_capture_path("t", &[]);
        let text = combined(&output);
        assert_eq!(output.status.code(), Some(0), "extension={extension}\n{text}");
        let manifest = fs::read_to_string(sandbox.entry_dir("t").join("package.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(parsed["type"], expected_type, "extension={extension}; manifest={manifest}");
    }
}

#[test]
fn test_build_writes_a_module_manifest_for_a_deps_free_module_typed_entry() {
    let sandbox = Sandbox::new();
    sandbox.create_js("t", "t.mjs", "console.log(1);\n", &[]);
    let (output, _) = sandbox.run_capture_path("t", &[]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let parsed: serde_json::Value = serde_json::from_slice(
        &fs::read(sandbox.entry_dir("t").join("package.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(parsed, serde_json::json!({"private": true, "type": "module"}));
}

#[test]
fn test_build_writes_no_manifest_for_a_flavorless_deps_free_entry() {
    let sandbox = Sandbox::new();
    sandbox.create_js("t", "t.js", "console.log(1);\n", &[]);
    let (output, _) = sandbox.run_capture_path("t", &[]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(!sandbox.entry_dir("t").join("package.json").exists(), "{text}");
}