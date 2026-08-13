use std::{collections::BTreeMap, env, fs, path::{Path, PathBuf}};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
    payload_stored_name,
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_language::{ParseOutcome, parse_document, write_managed_params};
use skit_runtime::{ProgramProbe as _, SystemProbe};
use skit_store::FileStore;
use tempfile::TempDir;

pub struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    bin: TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
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
        sandbox
    }

    pub fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    pub fn data_path(&self) -> &Path {
        self.data.path()
    }

    pub fn home_path(&self) -> &Path {
        self.home.path()
    }

    pub fn command(&self) -> Command {
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
        command
    }

    #[cfg(unix)]
    fn install_node(&self, body: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.bin.path().join("node");
        fs::write(&path, body).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    pub fn install_inspector_node(&self) {
        self.install_node(
            r#"#!/bin/sh
if [ -n "${SKIT_TEST_MARKER:-}" ]; then : > "$SKIT_TEST_MARKER"; fi
printf 'FAKE_PATH=%s\n' "$1"
printf 'FAKE_MODE=%s\n' "$(LC_ALL=C ls -ld "$1" | cut -c1-10)"
printf '%s\n' 'FAKE_BODY_BEGIN'
cat "$1"
printf '%s\n' 'FAKE_BODY_END'
"#,
        );
    }

    #[cfg(unix)]
    pub fn install_rejecting_check_node(&self) {
        self.install_node(
            r#"#!/bin/sh
if [ "$1" = "--check" ]; then
  printf '%s\n' 'SyntaxError: boom' >&2
  exit 1
fi
if [ -n "${SKIT_TEST_MARKER:-}" ]; then : > "$SKIT_TEST_MARKER"; fi
exit 0
"#,
        );
    }

    pub fn create_managed_entry(
        &self,
        name: &str,
        kind: &str,
        origin: &str,
        source: &str,
        interpreter: &str,
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
        self.create_entry(name, kind_value, origin, managed.into_bytes(), interpreter);
    }

    pub fn create_drifted_entry(&self, name: &str, interpreter: &str) {
        let mut declaration = ParamDecl::new("WIDTH");
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.parameter_type = ParameterType::Int;
        declaration.default = Some(ParameterValue::Integer(800));
        let managed = write_managed_params("js", "const TALL = 800;\n", &[declaration]).unwrap();
        self.create_entry(
            name,
            EntryKind::parse("js").unwrap(),
            &format!("{name}.js"),
            managed.into_bytes(),
            interpreter,
        );
    }

    fn create_entry(
        &self,
        name: &str,
        kind: EntryKind,
        origin: &str,
        bytes: Vec<u8>,
        interpreter: &str,
    ) {
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: kind.clone(),
                mode: StorageMode::Copy,
                source: origin.to_owned(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes,
                    stored_name: Some(payload_stored_name(&kind, Path::new(origin))),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    interpreter: interpreter.to_owned(),
                    ..EntrySettings::default()
                },
            })
            .unwrap();
    }

    pub fn run(&self, name: &str, key: &str, value: &str) -> std::process::Output {
        let set = format!("{key}={value}");
        self.command()
            .args(["run", name, "--set", &set, "--no-input"])
            .output()
            .unwrap()
    }

    pub fn run_with_marker(
        &self,
        name: &str,
        key: &str,
        value: &str,
        marker: &Path,
    ) -> std::process::Output {
        let set = format!("{key}={value}");
        self.command()
            .env("SKIT_TEST_MARKER", marker)
            .args(["run", name, "--set", &set, "--no-input"])
            .output()
            .unwrap()
    }

    pub fn staged_files(&self, name: &str) -> Vec<String> {
        let entry = self.store().resolve(name).unwrap();
        fs::read_dir(self.data.path().join("scripts").join(entry.slug.as_str()))
            .unwrap()
            .filter_map(Result::ok)
            .map(|item| item.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".run-"))
            .collect()
    }

    pub fn payload_path(&self, name: &str) -> PathBuf {
        let entry = self.store().resolve(name).unwrap();
        self.store().payload_path(&entry).unwrap()
    }
}

pub fn output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

pub fn tagged<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing {prefix:?} in output:\n{text}"))
}

pub fn body(text: &str) -> &str {
    text.split_once("FAKE_BODY_BEGIN\n")
        .and_then(|(_, rest)| rest.split_once("FAKE_BODY_END"))
        .map(|(body, _)| body)
        .unwrap_or_else(|| panic!("fake runtime did not report staged body:\n{text}"))
}

pub fn python_order_runtime() -> Option<String> {
    ["node", "deno", "bun"]
        .into_iter()
        .find(|name| SystemProbe.find_program(name).is_some())
        .map(str::to_owned)
}

pub fn real_program(name: &str) -> Option<PathBuf> {
    SystemProbe.find_program(name)
}

pub fn managed_source(kind: &str, source: &str) -> String {
    let ParseOutcome::Parsed(document) = parse_document(kind, source) else {
        panic!("test fixture must parse as {kind}");
    };
    let declarations = document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();
    write_managed_params(kind, source, &declarations).unwrap()
}

pub fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}
