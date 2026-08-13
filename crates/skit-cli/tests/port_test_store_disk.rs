//! Disk-usage ports from Python v0.4 `tests/test_store.py`.
//!
//! Rust moved `dir_size` into the health adapter. These exact-name tests drive the real `doctor
//! --json` consumer and assert its raw `size_bytes`, so they exercise the same recursive size
//! contract without recreating a test-only helper.

use std::fs;

use serde_json::Value;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        };
        fs::write(sandbox.config.path().join("config.toml"), "[mirror]\nenabled = false\n").unwrap();
        sandbox
    }

    fn size_bytes(&self) -> u64 {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        let output = command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .current_dir(self.home.path())
            .args(["doctor", "--json"])
            .output()
            .unwrap();
        // Doctor can return 1 when uv is absent. The JSON report is still the authoritative health
        // payload, and this contract is independent of uv availability.
        assert!(matches!(output.status.code(), Some(0 | 1)), "stderr={}", String::from_utf8_lossy(&output.stderr));
        let document: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!("doctor did not emit JSON: {error}; stdout={} stderr={}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
        });
        document["size_bytes"].as_u64().expect("doctor JSON size_bytes must be an unsigned integer")
    }
}

#[test]
fn test_dir_size_sums_only_files_recursively() {
    let sandbox = Sandbox::new();
    let scripts = sandbox.data.path().join("scripts");
    fs::create_dir_all(scripts.join("a")).unwrap();
    fs::write(scripts.join("a/one.txt"), vec![b'x'; 100]).unwrap();
    fs::write(scripts.join("two.txt"), vec![b'y'; 50]).unwrap();
    fs::create_dir(scripts.join("empty-dir")).unwrap();
    assert_eq!(sandbox.size_bytes(), 150);
}

#[test]
fn test_dir_size_missing_dir_is_zero() {
    let sandbox = Sandbox::new();
    assert!(!sandbox.data.path().join("scripts").exists());
    assert_eq!(sandbox.size_bytes(), 0);
}

#[test]
fn test_dir_size_on_a_file_is_zero() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.data.path().join("scripts"), b"data").unwrap();
    assert_eq!(sandbox.size_bytes(), 0);
}
