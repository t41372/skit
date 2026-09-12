//! Mechanical port of the Python oracle module `tests/test_hermeticity.py`
//! (`origin/main@206f9ef`): a regression guard that skit's directory resolution stays
//! isolated even when the `SKIT_*` override env vars are absent — the second isolation
//! layer (HOME + XDG redirect) the oracle's `conftest._isolate_skit_dirs` installs so a
//! mutant that breaks the `SKIT_*` lookup still can not escape into the developer's real
//! `~/.local/share/skit` (Linux) or `~/Library/Application Support/skit` (macOS). The one
//! `#[test]` keeps its Python `def test_*` name and its WHY comment.
//!
//! WHY skit-cli: the oracle exercises `skit.paths.data_dir()` / `state_dir()` /
//! `config_dir()`. Their Rust equivalents are the PRIVATE resolvers `resolve_data_dir` /
//! `resolve_state_dir` / `resolve_config_dir` plus the private `platform_data_dir` /
//! `platform_state_dir` / `platform_config_dir` fallbacks in `crates/skit-cli/src/cli.rs`
//! (line 7502 onward) — none `pub`, so an integration test can not call them. The one
//! public seam that resolves and REPORTS all three is `skit doctor --json`, whose JSON
//! carries `location` (= `data_dir/scripts`), `state_location`, and `config_location`
//! (`crates/skit-cli/src/cli.rs:4931`). So the port drives the composition root, the same
//! reason and pattern as `port_test_healthcheck.rs`.
//!
//! Concept mapping used throughout:
//! - Python `paths.data_dir()`  -> JSON `location`'s data root. `location` = the library
//!   path = `data_dir/scripts` (oracle `paths.scripts_dir()` = `data_dir()/"scripts"`), so
//!   `location.starts_with(root)` implies `data_dir` is inside `root` too.
//! - Python `paths.state_dir()`  -> JSON `state_location` (1:1).
//! - Python `paths.config_dir()` -> JSON `config_location` (1:1).
//! - Python `monkeypatch.delenv("SKIT_DATA_DIR"/STATE/CONFIG, raising=False)` ->
//!   `command.env_remove("SKIT_DATA_DIR")` etc. assert_cmd inherits the parent env, so the
//!   removal is what makes the fallback — not a leaked override — the thing under test.
//! - Python `conftest._isolate_skit_dirs`'s second layer (`HOME`, `USERPROFILE`,
//!   `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CONFIG_HOME` all under `tmp_path`) -> the same
//!   env set on the `skit` command, all under one TempDir `root`. The Linux fallback
//!   resolves via `XDG_*_HOME`; the macOS fallback via `HOME/Library/Application Support`;
//!   setting both keeps every resolution inside `root` on either platform, so one
//!   `starts_with(root)` assertion holds without per-platform branching.
//! - Python `assert Path.home() == tmp_path / "home"` (a sanity check that the HOME redirect
//!   took effect) -> no CLI observable; it is subsumed by setting `HOME` on the child
//!   command, which guarantees the redirect is in force for the resolution under test.
//! - Python `tmp_path in resolved.parents` -> `resolved.starts_with(root)`. Every resolved
//!   path is at least two levels below `root` (e.g. `root/xdg-data/skit/scripts`), so the
//!   inclusive `starts_with` and the strict `in parents` agree.
//! - Python `@pytest.mark.skipif(sys.platform == "win32", ...)` -> `#![cfg(unix)]` (whole
//!   file). The fallback pinning is POSIX-only: Windows resolves the user dirs via
//!   `LOCALAPPDATA`/`APPDATA` (`crates/skit-cli/src/cli.rs:7527`), which the HOME/XDG
//!   redirect can not repoint into the sandbox.
//!
//! DELIBERATE env deviation (read before flagging a hygiene bug): the general port rule is
//! that every binary invocation sets `SKIT_DATA_DIR`/`STATE`/`CONFIG` at fresh temp dirs.
//! This test's whole subject is their ABSENCE, so it must unset them instead. The sandbox
//! guarantee is preserved by the identical second isolation layer the oracle conftest uses:
//! `HOME` and the three `XDG_*_HOME` vars all point inside one TempDir `root`, so every
//! fallback resolution — and any file `skit doctor` might touch — lands in temp, never in
//! the developer's real home.
//!
//! Buckets:
//! - REAL asserting `#[test]` (API exists, reachable through the binary): the one test.
//!   No cross-crate stub, no absent-gap, no divergence.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::TempDir;

/// One TempDir root standing in for the oracle's `tmp_path`, plus the HOME + XDG
/// sub-directories the second isolation layer redirects into it.
struct Sandbox {
    root: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        // Mirror the conftest tmp_path sub-directories. Pre-creating them is harmless and
        // keeps the resolution deterministic on either POSIX platform.
        for name in ["home", "xdg-data", "xdg-state", "xdg-config"] {
            fs::create_dir_all(root.path().join(name)).unwrap();
        }
        Self { root }
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    /// Parsed `skit doctor --json`, run WITHOUT the `SKIT_*` overrides and with the HOME +
    /// XDG fallback redirected inside the sandbox root.
    fn doctor_json_without_skit_env(&self) -> Value {
        let root = self.path();
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            // Python: monkeypatch.delenv("SKIT_DATA_DIR"/STATE/CONFIG, raising=False).
            .env_remove("SKIT_DATA_DIR")
            .env_remove("SKIT_STATE_DIR")
            .env_remove("SKIT_CONFIG_DIR")
            // Second isolation layer: redirect the platformdirs-style fallback into temp.
            .env("HOME", root.join("home"))
            .env("XDG_DATA_HOME", root.join("xdg-data"))
            .env("XDG_STATE_HOME", root.join("xdg-state"))
            .env("XDG_CONFIG_HOME", root.join("xdg-config"))
            .env("SKIT_LANG", "en");
        // Do NOT assert success: a missing uv makes doctor exit 1, but it still prints the
        // JSON (with the resolved locations) to stdout first.
        let output = command.args(["doctor", "--json"]).output().unwrap();
        serde_json::from_slice(&output.stdout).expect("doctor --json emits valid JSON")
    }
}

/// The resolved directory a JSON string field names.
fn located(doctor: &Value, field: &str) -> PathBuf {
    PathBuf::from(doctor[field].as_str().expect("string path field"))
}

/// Simulate a mutant that breaks the SKIT_DATA_DIR/STATE/CONFIG lookup.
///
/// Even with those env vars entirely absent (as if paths.py's os.environ.get key were
/// corrupted by a mutant), data_dir()/state_dir()/config_dir() must still resolve inside the
/// fixture's fake HOME (set by conftest._isolate_skit_dirs) — never inside the developer's
/// real home directory.
#[test]
fn test_platformdirs_fallback_stays_isolated_when_skit_env_missing() {
    let sandbox = Sandbox::new();
    let doctor = sandbox.doctor_json_without_skit_env();
    let root = sandbox.path();

    // Python iterates (paths.data_dir, paths.state_dir, paths.config_dir) and asserts each
    // resolved path has tmp_path among its parents. Here the three resolved directories are
    // read back from `skit doctor --json` (`location` = data_dir/scripts, `state_location`,
    // `config_location`); each must land inside the sandbox root, never the real home.
    for field in ["location", "state_location", "config_location"] {
        let resolved = located(&doctor, field);
        assert!(
            resolved.starts_with(root),
            "{field} resolved to {resolved:?}, which escaped the sandbox root {root:?}"
        );
    }
}
