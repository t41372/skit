use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _,
    SourcePermissions, payload_stored_name,
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_language::{ParseOutcome, parse_document, write_managed_params};
use skit_runtime::{SystemProbe, resolve_javascript_runtime_program};
use skit_store::FileStore;
use tempfile::TempDir;

pub(crate) struct Sandbox {
    pub data: TempDir,
    pub state: TempDir,
    pub config: TempDir,
    pub home: TempDir,
    pub bin: TempDir,
}

impl Sandbox {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    pub(crate) fn command(&self) -> Command {
        self.command_for_locale("en")
    }

    pub(crate) fn command_for_locale(&self, locale: &str) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        let inherited = env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.bin.path().to_path_buf()];
        paths.extend(env::split_paths(&inherited));
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", locale)
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
    fn write_program(&self, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt as _;

        let path = self.bin.path().join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(windows)]
    fn write_program(&self, name: &str, body: &str) {
        fs::write(self.bin.path().join(format!("{name}.CMD")), body).unwrap();
    }

    pub(crate) fn install_inspector_node(&self) {
        #[cfg(unix)]
        self.write_program(
            "node",
            r#"#!/bin/sh
if [ "$1" = "--check" ]; then
  if [ -n "${SKIT_TEST_GATE_MARKER:-}" ]; then : > "$SKIT_TEST_GATE_MARKER"; fi
  if [ "${SKIT_TEST_REJECT_CHECK:-}" = "1" ]; then
    printf '%s\n' 'SyntaxError: boom' >&2
    exit 1
  fi
  exit 0
fi
if [ -n "${SKIT_TEST_LAUNCH_MARKER:-}" ]; then : > "$SKIT_TEST_LAUNCH_MARKER"; fi
printf 'FAKE_PATH=%s\n' "$1"
printf 'FAKE_MODE=%s\n' "$(LC_ALL=C ls -ld "$1" | cut -c1-10)"
printf '%s\n' 'FAKE_BODY_BEGIN'
cat "$1"
printf '%s\n' 'FAKE_BODY_END'
"#,
        );
        #[cfg(windows)]
        self.write_program(
            "node",
            concat!(
                "@echo off\r\n",
                "if \"%~1\"==\"--check\" (\r\n",
                "  if not \"%SKIT_TEST_GATE_MARKER%\"==\"\" type nul > \"%SKIT_TEST_GATE_MARKER%\"\r\n",
                "  if \"%SKIT_TEST_REJECT_CHECK%\"==\"1\" (echo SyntaxError: boom 1>&2& exit /b 1)\r\n",
                "  exit /b 0\r\n",
                ")\r\n",
                "if not \"%SKIT_TEST_LAUNCH_MARKER%\"==\"\" type nul > \"%SKIT_TEST_LAUNCH_MARKER%\"\r\n",
                "echo FAKE_PATH=%~1\r\n",
                "echo FAKE_BODY_BEGIN\r\n",
                "type \"%~1\"\r\n",
                "echo.\r\n",
                "echo FAKE_BODY_END\r\n",
            ),
        );
        #[cfg(unix)]
        self.write_program("npm", "#!/bin/sh\nmkdir -p node_modules\n");
        #[cfg(windows)]
        self.write_program("npm", "@echo off\r\nmkdir node_modules 2>nul\r\n");
    }

    pub(crate) fn install_node_order_wrapper(&self) {
        #[cfg(unix)]
        self.write_program(
            "node",
            r#"#!/bin/sh
if [ "$1" = "--check" ]; then
  if [ -e "$SKIT_TEST_ENTRY_DIR/package.json" ]; then
    printf '%s\n' 'package.json existed before gate' >&2
    exit 91
  fi
  : > "$SKIT_TEST_GATE_MARKER"
  exec "$SKIT_REAL_NODE" "$@"
fi
if [ ! -e "$SKIT_TEST_ENTRY_DIR/package.json" ]; then
  printf '%s\n' 'package.json missing before launch' >&2
  exit 92
fi
exec "$SKIT_REAL_NODE" "$@"
"#,
        );
        #[cfg(windows)]
        self.write_program(
            "node",
            concat!(
                "@echo off\r\n",
                "if \"%~1\"==\"--check\" (\r\n",
                "  if exist \"%SKIT_TEST_ENTRY_DIR%\\package.json\" (echo package.json existed before gate 1>&2& exit /b 91)\r\n",
                "  type nul > \"%SKIT_TEST_GATE_MARKER%\"\r\n",
                "  \"%SKIT_REAL_NODE%\" %*\r\n",
                "  exit /b %errorlevel%\r\n",
                ")\r\n",
                "if not exist \"%SKIT_TEST_ENTRY_DIR%\\package.json\" (echo package.json missing before launch 1>&2& exit /b 92)\r\n",
                "\"%SKIT_REAL_NODE%\" %*\r\n",
                "exit /b %errorlevel%\r\n",
            ),
        );
    }

    pub(crate) fn create_managed_entry(
        &self,
        name: &str,
        kind: &str,
        origin: &str,
        source: &str,
        interpreter: &str,
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
        self.create_entry(
            name,
            kind_value,
            origin,
            managed.into_bytes(),
            interpreter,
            dependencies,
        );
    }

    pub(crate) fn create_drifted_entry(&self, name: &str, dependencies: Vec<String>) {
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
            "node",
            dependencies,
        );
    }

    fn create_entry(
        &self,
        name: &str,
        kind: EntryKind,
        origin: &str,
        bytes: Vec<u8>,
        interpreter: &str,
        dependencies: Vec<String>,
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
                    dependencies,
                    ..EntrySettings::default()
                },
            })
            .unwrap();
    }

    pub(crate) fn run(&self, name: &str, key: &str, value: &str) -> std::process::Output {
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

    pub(crate) fn entry_dir(&self, name: &str) -> PathBuf {
        let entry = self.store().resolve(name).unwrap();
        self.store().entry_dir_path(&entry.slug)
    }

    pub(crate) fn staged_files(&self, name: &str) -> Vec<String> {
        fs::read_dir(self.entry_dir(name))
            .unwrap()
            .filter_map(Result::ok)
            .map(|item| item.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".run-") || name.starts_with(".injected-"))
            .collect()
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<String, Option<Vec<u8>>> {
        let mut snapshot = BTreeMap::new();
        for (label, root) in [
            ("data", self.data.path()),
            ("state", self.state.path()),
            ("config", self.config.path()),
        ] {
            snapshot_tree(root, Path::new(label), &mut snapshot);
        }
        snapshot
    }

    pub(crate) fn assert_no_dependency_artifacts(&self, name: &str) {
        let entry_dir = self.entry_dir(name);
        for item in [
            "package.json",
            "package-lock.json",
            "bun.lock",
            "bun.lockb",
            "deno.lock",
            ".skit-deps",
            "node_modules",
        ] {
            assert!(
                !entry_dir.join(item).exists(),
                "dependency artifact appeared before launch: {item}"
            );
        }
        assert!(
            !self
                .data
                .path()
                .join(".locks")
                .join(format!("{name}.skit-deps.lock"))
                .exists(),
            "dependency lock appeared before launch"
        );
    }
}

fn snapshot_tree(root: &Path, relative: &Path, output: &mut BTreeMap<String, Option<Vec<u8>>>) {
    if relative
        .components()
        .any(|component| component.as_os_str() == ".locks")
    {
        return;
    }
    output.insert(relative.to_string_lossy().into_owned(), None);
    let mut children = fs::read_dir(root)
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    children.sort_by_key(|item| item.file_name());
    for child in children {
        let child_relative = relative.join(child.file_name());
        let kind = child.file_type().unwrap();
        if kind.is_dir() {
            snapshot_tree(&child.path(), &child_relative, output);
        } else {
            output.insert(
                child_relative.to_string_lossy().into_owned(),
                Some(fs::read(child.path()).unwrap()),
            );
        }
    }
}

pub(crate) fn output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

pub(crate) fn tagged<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing {prefix:?} in output:\n{text}"))
}

pub(crate) fn oracle_runtime() -> Option<PathBuf> {
    resolve_javascript_runtime_program(&EntrySettings::default(), &SystemProbe)
        .ok()
        .map(|runtime| runtime.program)
}

pub(crate) fn exact_tree_keys(snapshot: &BTreeMap<String, Option<Vec<u8>>>) -> BTreeSet<&str> {
    snapshot.keys().map(String::as_str).collect()
}
