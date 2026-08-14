use std::{env, fs, path::{Path, PathBuf}};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
    payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_language::{ParseOutcome, parse_document, write_managed_params};
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

    pub fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    pub fn create_managed_entry(&self, name: &str, source: &str) {
        let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
            panic!("shell test fixture must parse");
        };
        let declarations = document
            .analysis()
            .candidates
            .into_iter()
            .map(|candidate| candidate.declaration)
            .collect::<Vec<_>>();
        assert!(!declarations.is_empty(), "fixture must expose at least one shell parameter");
        let managed = write_managed_params("shell", source, &declarations).unwrap();
        let kind = EntryKind::parse("shell").unwrap();
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: kind.clone(),
                mode: StorageMode::Copy,
                source: format!("{name}.sh"),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: managed.into_bytes(),
                    stored_name: Some(payload_stored_name(&kind, Path::new(&format!("{name}.sh")))),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
    }

    pub fn run_sets(&self, name: &str, pairs: &[(&str, &str)]) -> std::process::Output {
        let mut command = self.command();
        command.args(["run", name]);
        for (key, value) in pairs {
            command.args(["--set", &format!("{key}={value}")]);
        }
        command.arg("--no-input").output().unwrap()
    }

    pub fn payload_path(&self, name: &str) -> PathBuf {
        let entry = self.store().resolve(name).unwrap();
        self.store().payload_path(&entry).unwrap()
    }

    pub fn staged_files(&self, name: &str) -> Vec<PathBuf> {
        let entry = self.store().resolve(name).unwrap();
        let dir = self.data.path().join("scripts").join(entry.slug.as_str());
        fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|item| item.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".run-"))
            })
            .collect()
    }

    #[cfg(unix)]
    pub fn install_inspector_bash(&self) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.bin.path().join("bash");
        fs::write(
            &path,
            r#"#!/bin/sh
printf 'SHELL_PATH=%s\n' "$1"
printf 'SHELL_MODE=%s\n' "${MODE-}"
printf 'SHELL_GREETING=%s\n' "${GREETING-}"
printf '%s\n' 'SHELL_BODY_BEGIN'
cat "$1"
printf '%s\n' 'SHELL_BODY_END'
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
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
    text.split_once("SHELL_BODY_BEGIN\n")
        .and_then(|(_, rest)| rest.split_once("SHELL_BODY_END"))
        .map(|(body, _)| body)
        .unwrap_or_else(|| panic!("inspector shell did not report source body:\n{text}"))
}
