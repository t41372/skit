//! CLI/store/staging contracts from Python v0.4 `tests/test_js_inject.py`.
//!
//! A fake `node` reports the staged path and bytes. The tests do not require a JavaScript runtime.

use std::{env, fs, path::Path};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

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
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        sandbox.install_inspector_node();
        sandbox
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        let inherited = env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.bin.path().to_path_buf()];
        paths.extend(env::split_paths(&inherited));
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
            .env("PATH", env::join_paths(paths).unwrap())
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .current_dir(self.home.path());
        #[cfg(windows)]
        command.env("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        command
    }

    #[cfg(unix)]
    fn install_inspector_node(&self) {
        let path = self.bin.path().join("node");
        fs::write(
            &path,
            r#"#!/bin/sh
if [ -n "${SKIT_TEST_MARKER:-}" ]; then : > "$SKIT_TEST_MARKER"; fi
printf 'FAKE_PATH=%s\n' "$1"
printf 'FAKE_MODE=%s\n' "$(LC_ALL=C ls -ld "$1" | cut -c1-10)"
printf '%s\n' 'FAKE_BODY_BEGIN'
cat "$1"
printf '%s\n' 'FAKE_BODY_END'
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
        let npm = self.bin.path().join("npm");
        fs::write(&npm, "#!/bin/sh\nmkdir -p node_modules\n").unwrap();
        let mut permissions = fs::metadata(&npm).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(npm, permissions).unwrap();
    }

    #[cfg(windows)]
    fn install_inspector_node(&self) {
        fs::write(
            self.bin.path().join("node.CMD"),
            concat!(
                "@echo off\r\n",
                "if not \"%SKIT_TEST_MARKER%\"==\"\" type nul > \"%SKIT_TEST_MARKER%\"\r\n",
                "echo FAKE_PATH=%~1\r\n",
                "echo FAKE_BODY_BEGIN\r\n",
                "type \"%~1\"\r\n",
                "echo.\r\n",
                "echo FAKE_BODY_END\r\n",
            ),
        )
        .unwrap();
        fs::write(
            self.bin.path().join("npm.CMD"),
            "@echo off\r\nmkdir node_modules 2>nul\r\n",
        )
        .unwrap();
    }

    fn create_managed_entry(&self, name: &str, kind: &str, origin: &str, source: &str) {
        self.create_managed_entry_with_dependencies(name, kind, origin, source, Vec::new());
    }

    fn create_managed_entry_with_dependencies(
        &self,
        name: &str,
        kind: &str,
        origin: &str,
        source: &str,
        dependencies: Vec<String>,
    ) {
        let kind_value = EntryKind::parse(kind).unwrap();
        let ParseOutcome::Parsed(document) = parse_document(kind, source) else {
            panic!("test fixture must parse as {kind}");
        };
        let declaration = document
            .analysis()
            .candidates
            .into_iter()
            .next()
            .expect("test fixture must expose one candidate")
            .declaration;
        let managed = write_managed_params(kind, source, &[declaration]).unwrap();
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: kind_value.clone(),
                mode: StorageMode::Copy,
                source: origin.to_owned(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: managed.into_bytes(),
                    stored_name: Some(payload_stored_name(&kind_value, Path::new(origin))),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    interpreter: "node".to_owned(),
                    dependencies,
                    ..EntrySettings::default()
                },
            })
            .unwrap();
    }

    fn run(&self, name: &str, key: &str, value: &str) -> std::process::Output {
        self.command()
            .args([
                "run",
                name,
                "--set",
                &format!("{key}={value}"),
                "--no-input",
            ])
            .output()
            .unwrap()
    }

    fn run_with_marker(
        &self,
        name: &str,
        key: &str,
        value: &str,
        marker: &Path,
    ) -> std::process::Output {
        self.command()
            .env("SKIT_TEST_MARKER", marker)
            .args([
                "run",
                name,
                "--set",
                &format!("{key}={value}"),
                "--no-input",
            ])
            .output()
            .unwrap()
    }

    fn staged_files(&self, name: &str) -> Vec<String> {
        let entry = self.store().resolve(name).unwrap();
        fs::read_dir(self.data.path().join("scripts").join(entry.slug.as_str()))
            .unwrap()
            .filter_map(Result::ok)
            .map(|item| item.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".run-") || name.starts_with(".injected-"))
            .collect()
    }
}

fn output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn tagged<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing {prefix:?} in output:\n{text}"))
}

#[test]
fn test_ts_temp_copy_has_ts_suffix() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry("tscopy", "ts", "plain.ts", "const N: number = 5;\n");
    let output = sandbox.run("tscopy", "N", "7");
    let text = output_text(&output);

    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(
        tagged(&text, "FAKE_PATH=").trim_end().ends_with(".ts"),
        "{text}"
    );
    assert!(
        !Path::new(tagged(&text, "FAKE_PATH=").trim_end()).starts_with(sandbox.data.path()),
        "dependency-free injected values must use OS private temp: {text}"
    );
    assert!(text.contains("const N: number = 7;"), "{text}");
    assert!(sandbox.staged_files("tscopy").is_empty());
}

#[test]
fn dependency_backed_javascript_keeps_the_injected_copy_next_to_node_modules() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry_with_dependencies(
        "withdeps",
        "js",
        "withdeps.js",
        "const NAME = 'old';\n",
        vec!["chalk".to_owned()],
    );

    let output = sandbox.run("withdeps", "NAME", "new");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let staged = Path::new(tagged(&text, "FAKE_PATH=").trim_end());
    let entry = sandbox.store().resolve("withdeps").unwrap();
    let entry_dir = sandbox.store().entry_dir_path(&entry.slug);
    assert_eq!(staged.parent(), Some(entry_dir.as_path()), "{text}");
    assert!(entry_dir.join("node_modules").is_dir());
    assert!(sandbox.staged_files("withdeps").is_empty());
}

#[test]
#[cfg(unix)]
fn test_injected_copy_is_0600() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "secret",
        "js",
        "secret.js",
        "const API_KEY = \"changeme\";\n",
    );
    let output = sandbox.run("secret", "API_KEY", "s3cr3t");
    let text = output_text(&output);

    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(
        tagged(&text, "FAKE_MODE=").trim_end(),
        "-rw-------",
        "{text}"
    );
    assert!(text.contains("const API_KEY = \"s3cr3t\";"), "{text}");
    assert!(sandbox.staged_files("secret").is_empty());
}

#[test]
fn test_execute_refuses_a_bad_value_before_launch() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry("badvalue", "js", "badvalue.js", "const WIDTH = 800;\n");
    let marker = sandbox.home.path().join("launched");
    let output = sandbox.run_with_marker("badvalue", "WIDTH", "abc", &marker);
    let text = output_text(&output);

    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(!marker.exists(), "bad JS value reached the child");
    assert!(sandbox.staged_files("badvalue").is_empty());
}
