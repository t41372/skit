//! Python-byte/dependency ports from `tests/test_store_fix.py` at `main@206f9ef`.
//!
//! These cross the real CLI/store boundary and assert the stored bytes plus effective uv metadata.
//! The oracle is deliberately byte-level: non-UTF-8 copies must never be replacement-decoded, and
//! CRLF/LF inputs must keep their own physical newline style while the PEP 723 block changes.

use std::{fs, path::PathBuf};

use assert_cmd::Command;
use skit_application::EntryRepository as _;
use skit_domain::EntrySettings;
use skit_language::{managed_params, read_uv_metadata};
use skit_store::FileStore;
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
            .current_dir(self.home.path());
        command
    }

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn add(&self, source: &std::path::Path, name: &str, extra: &[&str]) {
        let mut command = self.command();
        command.arg("add").arg(source).args(["--name", name]);
        command.args(extra).arg("--no-input").assert().success();
    }

    fn entry(&self, name: &str) -> skit_domain::Entry {
        FileStore::new(self.data.path()).resolve(name).unwrap()
    }

    fn stored(&self, name: &str) -> PathBuf {
        let store = FileStore::new(self.data.path());
        let entry = store.resolve(name).unwrap();
        store.payload_path(&entry).unwrap()
    }

    fn effective(&self, name: &str) -> skit_language::UvMetadata {
        let text = String::from_utf8(fs::read(self.stored(name)).unwrap()).unwrap();
        read_uv_metadata(&text).unwrap_or_default()
    }
}

#[test]
fn test_add_python_non_utf8_source_skips_injection_keeps_deps_in_meta() {
    let sandbox = Sandbox::new();
    let bytes = b"# -*- coding: latin-1 -*-\nX = 1\nS = \"caf\xe9\"\nimport requests\n";
    let source = sandbox.source("latin1.py", bytes);
    sandbox.add(&source, "latin1", &["--dep", "requests"]);

    assert_eq!(fs::read(sandbox.stored("latin1")).unwrap(), bytes);
    let settings = EntrySettings::from_meta(&sandbox.entry("latin1").meta);
    assert_eq!(settings.dependencies, ["requests"]);
}

#[test]
fn test_add_python_utf8_source_still_injects_normally() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("plain.py", b"print(1)\n");
    sandbox.add(&source, "plain", &["--dep", "httpx"]);

    let stored = fs::read_to_string(sandbox.stored("plain")).unwrap();
    assert_eq!(read_uv_metadata(&stored).unwrap().dependencies, ["httpx"]);
    let settings = EntrySettings::from_meta(&sandbox.entry("plain").meta);
    assert!(
        settings.dependencies.is_empty(),
        "UTF-8 copy must keep the dependency block as the single source of truth"
    );
}

#[test]
fn test_update_dependencies_copy_non_utf8_leaves_stored_copy_byte_identical() {
    let sandbox = Sandbox::new();
    let bytes = b"# -*- coding: latin-1 -*-\nTEXT = 'caf\xe9'\n";
    let source = sandbox.source("latin1.py", bytes);
    sandbox.add(&source, "latin1", &[]);
    let before = fs::read(sandbox.stored("latin1")).unwrap();

    sandbox
        .command()
        .args(["deps", "latin1", "--dep", "requests"])
        .assert()
        .success();

    assert_eq!(fs::read(sandbox.stored("latin1")).unwrap(), before);
    assert_eq!(
        EntrySettings::from_meta(&sandbox.entry("latin1").meta).dependencies,
        ["requests"]
    );
}

#[test]
fn test_update_dependencies_copy_utf8_syncs_block_and_stays_utf8() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("plain.py", b"print(1)\n");
    sandbox.add(&source, "plain", &[]);

    sandbox
        .command()
        .args([
            "deps",
            "plain",
            "--dep",
            "httpx",
            "--python",
            ">=3.11",
        ])
        .assert()
        .success();

    let stored = fs::read_to_string(sandbox.stored("plain")).unwrap();
    let uv = read_uv_metadata(&stored).unwrap();
    assert_eq!(uv.dependencies, ["httpx"]);
    assert_eq!(uv.requires_python, ">=3.11");
}

fn add_non_utf8_authoritative_block(sandbox: &Sandbox, name: &str, bytes: &[u8]) -> Vec<u8> {
    let source = sandbox.source(&format!("{name}.py"), bytes);
    sandbox.add(&source, name, &[]);
    let before = fs::read(sandbox.stored(name)).unwrap();
    assert_eq!(before, bytes);
    before
}

#[test]
fn test_update_dependencies_refuses_when_a_non_utf8_copy_carries_its_own_block() {
    let sandbox = Sandbox::new();
    let bytes = b"# /// script\n# requires-python = \">=3.13\"\n# dependencies = [\"requests\"]\n# ///\nTEXT = 'caf\xe9'\n";
    let before = add_non_utf8_authoritative_block(&sandbox, "latin1-block", bytes);

    let output = sandbox
        .command()
        .args(["deps", "latin1-block", "--clear"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("isn't valid UTF-8"), "{text}");
    assert!(text.contains("own dependency block"), "{text}");
    assert_eq!(fs::read(sandbox.stored("latin1-block")).unwrap(), before);

    let stored = String::from_utf8_lossy(bytes);
    assert!(stored.contains("requests"));
    assert!(stored.contains(">=3.13"));
    let settings = EntrySettings::from_meta(&sandbox.entry("latin1-block").meta);
    assert!(settings.dependencies.is_empty());
    assert!(settings.requires_python.is_empty());
}

#[test]
fn test_update_dependencies_python_unpin_is_refused_for_the_same_copy() {
    let sandbox = Sandbox::new();
    let bytes = b"# /// script\n# requires-python = \">=3.13\"\n# ///\nT = 'caf\xe9'\n";
    let before = add_non_utf8_authoritative_block(&sandbox, "latin1-unpin", bytes);

    let output = sandbox
        .command()
        .args(["deps", "latin1-unpin", "--python", "-"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(sandbox.stored("latin1-unpin")).unwrap(), before);
}

#[test]
fn test_update_dependencies_untouched_axes_never_reach_the_refusal() {
    let sandbox = Sandbox::new();
    let bytes = b"# /// script\n# dependencies = [\"requests\"]\n# ///\nT = 'caf\xe9'\n";
    let before = add_non_utf8_authoritative_block(&sandbox, "latin1-read", bytes);

    sandbox
        .command()
        .args(["deps", "latin1-read", "--json"])
        .assert()
        .success();
    assert_eq!(fs::read(sandbox.stored("latin1-read")).unwrap(), before);
}

#[test]
fn test_deps_edit_on_a_crlf_copy_keeps_one_block_and_its_params() {
    let sandbox = Sandbox::new();
    let lf = concat!(
        "# /// script\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"CITY\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# default = \"Taipei\"\n",
        "# ///\n",
        "CITY = \"Taipei\"\n",
        "print(CITY)\n",
    );
    let crlf = lf.replace('\n', "\r\n");
    let source = sandbox.source("crlf.py", crlf.as_bytes());
    sandbox.add(&source, "crlf", &[]);

    sandbox
        .command()
        .args([
            "deps",
            "crlf",
            "--dep",
            "rich>=15",
            "--python",
            ">=3.12",
        ])
        .assert()
        .success();

    let raw = fs::read(sandbox.stored("crlf")).unwrap();
    assert_eq!(raw.windows(b"/// script".len()).filter(|w| *w == b"/// script").count(), 1);
    assert!(raw.windows(2).any(|pair| pair == b"\r\n"));
    assert!(!raw
        .windows(1)
        .enumerate()
        .any(|(index, byte)| byte == b"\n" && (index == 0 || raw[index - 1] != b'\r')));
    let normalized = String::from_utf8(raw).unwrap().replace("\r\n", "\n");
    assert_eq!(
        managed_params("python", &normalized)
            .iter()
            .map(|param| param.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY"]
    );
    let uv = read_uv_metadata(&normalized).unwrap();
    assert_eq!(uv.dependencies, ["rich>=15"]);
    assert_eq!(uv.requires_python, ">=3.12");
}

#[test]
fn test_add_with_deps_does_not_double_block_a_crlf_script() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "crlf-add.py",
        b"# /// script\r\n# dependencies = [\"requests\"]\r\n# ///\r\nprint(1)\r\n",
    );
    sandbox.add(&source, "crlfadd", &["--dep", "rich"]);

    let raw = fs::read(sandbox.stored("crlfadd")).unwrap();
    assert_eq!(raw.windows(b"/// script".len()).filter(|w| *w == b"/// script").count(), 1);
    assert!(raw.windows(2).any(|pair| pair == b"\r\n"));
    let normalized = String::from_utf8(raw).unwrap().replace("\r\n", "\n");
    assert!(managed_params("python", &normalized).is_empty());
    assert_eq!(read_uv_metadata(&normalized).unwrap().dependencies, ["rich"]);
}

#[test]
fn test_add_keeps_an_lf_script_lf_when_injecting_a_block() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("lf.py", b"import rich\nprint(1)\n");
    sandbox.add(&source, "lfadd", &["--dep", "rich"]);

    let raw = fs::read(sandbox.stored("lfadd")).unwrap();
    assert!(raw.windows(b"/// script".len()).any(|w| w == b"/// script"));
    assert!(!raw.windows(2).any(|pair| pair == b"\r\n"));
    assert!(raw.contains(&b'\n'));
}
