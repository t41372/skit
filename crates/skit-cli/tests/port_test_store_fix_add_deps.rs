//! Exact add and dependency-byte owners from Python v0.4 `tests/test_store_fix.py`.

use std::{collections::BTreeMap, fs, path::Path, process::Output};

use skit_application::EntryRepository as _;
use skit_domain::{EntrySettings, StorageMode};
use skit_language::{ParseOutcome, managed_params, parse_document, write_managed_params};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    sources: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            sources: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .current_dir(self.home.path());
        command
    }

    fn source(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = self.sources.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn source_path(&self, name: &str) -> std::path::PathBuf {
        self.sources.path().join(format!("{name}.py"))
    }

    fn add_python(
        &self,
        name: &str,
        bytes: &[u8],
        reference: bool,
        dependencies: &[&str],
    ) -> Output {
        let source = self.source(&format!("{name}.py"), bytes);
        let mut command = self.command();
        command
            .arg("add")
            .arg(source)
            .args(["--kind", "python", "--name", name, "--no-input"]);
        if reference {
            command.arg("--ref");
        }
        for dependency in dependencies {
            command.args(["--dep", dependency]);
        }
        command.output().unwrap()
    }

    fn deps(&self, arguments: &[&str]) -> Output {
        self.command().arg("deps").args(arguments).output().unwrap()
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn settings(&self, name: &str) -> EntrySettings {
        EntrySettings::from_meta(&self.store().resolve(name).unwrap().meta)
    }

    fn stored_path(&self, name: &str) -> std::path::PathBuf {
        let store = self.store();
        let entry = store.resolve(name).unwrap();
        store.payload_path(&entry).unwrap()
    }

    fn stored(&self, name: &str) -> Vec<u8> {
        fs::read(self.stored_path(name)).unwrap()
    }

    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut rows = BTreeMap::new();
        for (label, root) in [
            ("data", self.data.path()),
            ("state", self.state.path()),
            ("config", self.config.path()),
        ] {
            snapshot_tree(root, Path::new(label), &mut rows);
        }
        rows
    }
}

fn snapshot_tree(root: &Path, relative: &Path, rows: &mut BTreeMap<String, Vec<u8>>) {
    let mut children = fs::read_dir(root)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        let path = child.path();
        let relative = relative.join(child.file_name());
        if child.file_type().unwrap().is_dir() {
            snapshot_tree(&path, &relative, rows);
        } else {
            rows.insert(
                relative.to_string_lossy().into_owned(),
                fs::read(path).unwrap(),
            );
        }
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn block_count(bytes: &[u8]) -> usize {
    bytes
        .windows(b"/// script".len())
        .filter(|window| *window == b"/// script")
        .count()
}

fn has_lone_lf(bytes: &[u8]) -> bool {
    bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'\n' && index.checked_sub(1).is_none_or(|prev| bytes[prev] != b'\r')
    })
}

#[test]
fn test_add_python_copy_mode_defaults_workdir_to_invoke() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_python("copy", b"print(1)\n", false, &[]));
    let entry = sandbox.store().resolve("copy").unwrap();
    assert_eq!(entry.meta.mode, StorageMode::Copy);
    assert_eq!(entry.meta.workdir, "invoke");
}

#[test]
fn test_add_python_reference_mode_still_defaults_workdir_to_origin() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_python("reference", b"print(1)\n", true, &[]));
    let entry = sandbox.store().resolve("reference").unwrap();
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(entry.meta.workdir, "origin");
}

#[test]
fn test_add_python_non_utf8_source_skips_injection_keeps_deps_in_meta() {
    let sandbox = Sandbox::new();
    let source = b"# -*- coding: latin-1 -*-\nX = 1\nS = \"caf\xe9\"\nimport requests\n";
    assert_success(&sandbox.add_python("latin1", source, false, &["requests"]));
    assert_eq!(fs::read(sandbox.source_path("latin1")).unwrap(), source);
    assert_eq!(sandbox.stored("latin1"), source);
    assert_eq!(sandbox.settings("latin1").dependencies, ["requests"]);
}

#[test]
fn test_add_python_utf8_source_still_injects_normally() {
    let sandbox = Sandbox::new();
    let source = b"print(1)\n";
    assert_success(&sandbox.add_python("utf8", source, false, &["httpx"]));
    assert_eq!(fs::read(sandbox.source_path("utf8")).unwrap(), source);
    let stored = String::from_utf8(sandbox.stored("utf8")).unwrap();
    assert!(stored.contains("# /// script"), "{stored}");
    assert!(stored.contains("httpx"), "{stored}");
    assert!(sandbox.settings("utf8").dependencies.is_empty());
}

#[test]
fn test_update_dependencies_copy_non_utf8_leaves_stored_copy_byte_identical() {
    let sandbox = Sandbox::new();
    let source = b"# -*- coding: latin-1 -*-\nTEXT = 'caf\xe9'\n";
    assert_success(&sandbox.add_python("latin", source, false, &[]));
    let before = sandbox.stored("latin");
    assert_success(&sandbox.deps(&["latin", "--dep", "requests"]));
    assert_eq!(fs::read(sandbox.source_path("latin")).unwrap(), source);
    assert_eq!(sandbox.stored("latin"), before);
    assert_eq!(sandbox.settings("latin").dependencies, ["requests"]);
}

#[test]
fn test_update_dependencies_copy_utf8_syncs_block_and_stays_utf8() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_python("plain", b"print(1)\n", false, &[]));
    assert_success(&sandbox.deps(&["plain", "--dep", "httpx", "--python", ">=3.11"]));
    let stored = String::from_utf8(sandbox.stored("plain")).unwrap();
    assert!(stored.contains("httpx"), "{stored}");
    assert!(stored.contains("requires-python = \">=3.11\""), "{stored}");
}

#[test]
fn test_update_dependencies_refuses_when_a_non_utf8_copy_carries_its_own_block() {
    let sandbox = Sandbox::new();
    let source = concat!(
        "# /// script\n",
        "# requires-python = \">=3.13\"\n",
        "# dependencies = [\"requests\"]\n",
        "# ///\n",
    )
    .as_bytes()
    .iter()
    .copied()
    .chain(b"TEXT = 'caf\xe9'\n".iter().copied())
    .collect::<Vec<_>>();
    assert_success(&sandbox.add_python("latin1_block", &source, false, &[]));
    let before = sandbox.snapshot();
    let output = sandbox.deps(&["latin1_block", "--clear"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("what uv reads"));
    assert_eq!(sandbox.snapshot(), before);
}

#[test]
fn test_update_dependencies_python_unpin_is_refused_for_the_same_copy() {
    let sandbox = Sandbox::new();
    let mut source = b"# /// script\n# requires-python = \">=3.13\"\n# ///\nT = 'caf".to_vec();
    source.extend_from_slice(b"\xe9'\n");
    assert_success(&sandbox.add_python("latin1_block2", &source, false, &[]));
    let before = sandbox.snapshot();
    let output = sandbox.deps(&["latin1_block2", "--python", "-"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(sandbox.snapshot(), before);
}

#[test]
fn test_update_dependencies_untouched_axes_never_reach_the_refusal() {
    let sandbox = Sandbox::new();
    let mut source = b"# /// script\n# dependencies = [\"requests\"]\n# ///\nT = 'caf".to_vec();
    source.extend_from_slice(b"\xe9'\n");
    assert_success(&sandbox.add_python("latin1_block3", &source, false, &[]));
    let before = sandbox.snapshot();
    assert_success(&sandbox.deps(&["latin1_block3"]));
    assert_eq!(sandbox.snapshot(), before);
}

#[test]
fn test_deps_edit_on_a_crlf_copy_keeps_one_block_and_its_params() {
    let sandbox = Sandbox::new();
    let ParseOutcome::Parsed(document) =
        parse_document("python", "CITY = \"Taipei\"\nprint(CITY)\n")
    else {
        panic!("fixture must parse");
    };
    let declaration = document.analysis().candidates[0].declaration.clone();
    let managed =
        write_managed_params("python", "CITY = \"Taipei\"\nprint(CITY)\n", &[declaration]).unwrap();
    let crlf = managed.replace('\n', "\r\n");
    assert_success(&sandbox.add_python("crlf", crlf.as_bytes(), false, &[]));
    assert_success(&sandbox.deps(&["crlf", "--dep", "rich>=15", "--python", ">=3.12"]));
    assert_eq!(
        fs::read(sandbox.source_path("crlf")).unwrap(),
        crlf.as_bytes()
    );
    let raw = sandbox.stored("crlf");
    assert_eq!(block_count(&raw), 1);
    assert!(raw.windows(2).any(|window| window == b"\r\n"));
    assert!(!has_lone_lf(&raw));
    let normalized = String::from_utf8(raw).unwrap().replace("\r\n", "\n");
    assert_eq!(
        managed_params("python", &normalized)
            .into_iter()
            .map(|item| item.name)
            .collect::<Vec<_>>(),
        ["CITY"]
    );
    let settings = sandbox.settings("crlf");
    assert_eq!(settings.dependencies, ["rich>=15"]);
    assert_eq!(settings.requires_python, ">=3.12");
    assert!(normalized.contains("rich>=15"), "{normalized}");
    assert!(normalized.contains(">=3.12"), "{normalized}");
}

#[test]
fn test_add_with_deps_does_not_double_block_a_crlf_script() {
    let sandbox = Sandbox::new();
    let source = b"# /// script\r\n# dependencies = [\"requests\"]\r\n# ///\r\nprint(1)\r\n";
    assert_success(&sandbox.add_python("crlfadd", source, false, &["rich"]));
    assert_eq!(fs::read(sandbox.source_path("crlfadd")).unwrap(), source);
    let raw = sandbox.stored("crlfadd");
    assert_eq!(block_count(&raw), 1);
    let normalized = String::from_utf8(raw).unwrap().replace("\r\n", "\n");
    assert!(managed_params("python", &normalized).is_empty());
}

#[test]
fn test_add_keeps_an_lf_script_lf_when_injecting_a_block() {
    let sandbox = Sandbox::new();
    let source = b"import rich\nprint(1)\n";
    assert_success(&sandbox.add_python("lfadd", source, false, &["rich"]));
    assert_eq!(fs::read(sandbox.source_path("lfadd")).unwrap(), source);
    let raw = sandbox.stored("lfadd");
    assert_eq!(block_count(&raw), 1);
    assert!(!raw.windows(2).any(|window| window == b"\r\n"));
}
