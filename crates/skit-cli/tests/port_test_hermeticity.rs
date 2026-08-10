//! Mechanical port of `tests/test_hermeticity.py` from the Python oracle
//! (`main@206f9ef`). The original regression is POSIX-only, so this port has the same boundary.

#[cfg(not(windows))]
use std::{fs, path::PathBuf};

#[cfg(not(windows))]
use assert_cmd::Command;
#[cfg(not(windows))]
use tempfile::TempDir;

#[cfg(not(windows))]
fn command(root: &TempDir) -> Command {
    let home = root.path().join("home");
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env_remove("SKIT_DATA_DIR")
        .env_remove("SKIT_STATE_DIR")
        .env_remove("SKIT_CONFIG_DIR")
        .env("SKIT_LANG", "en")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_DATA_HOME", root.path().join("xdg-data"))
        .env("XDG_STATE_HOME", root.path().join("xdg-state"))
        .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
        .current_dir(home);
    command
}

#[cfg(target_os = "macos")]
fn fallback_roots(root: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
    let shared = root
        .path()
        .join("home")
        .join("Library")
        .join("Application Support")
        .join("skit");
    (shared.clone(), shared.clone(), shared)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn fallback_roots(root: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
    (
        root.path().join("xdg-data").join("skit"),
        root.path().join("xdg-state").join("skit"),
        root.path().join("xdg-config").join("skit"),
    )
}

#[cfg(not(windows))]
#[test]
fn test_platformdirs_fallback_stays_isolated_when_skit_env_missing() {
    // Simulate a mutant that breaks the SKIT_DATA_DIR/STATE/CONFIG lookup. Even with those names
    // absent, every fallback must stay inside the fixture home/XDG roots, never the real user dirs.
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("home")).unwrap();

    command(&root)
        .args(["config", "lang", "en"])
        .assert()
        .success();
    command(&root)
        .args([
            "add",
            "--cmd",
            "printf '%s' {value}",
            "--name",
            "Fallback demo",
            "--no-input",
        ])
        .assert()
        .success();
    command(&root)
        .args([
            "run",
            "fallback-demo",
            "--set",
            "value=isolated",
            "--no-input",
        ])
        .assert()
        .success();

    let (data, state, config) = fallback_roots(&root);
    for resolved in [&data, &state, &config] {
        assert!(
            resolved.starts_with(root.path()),
            "fallback escaped test root: {}",
            resolved.display()
        );
    }
    assert!(data.join("scripts/fallback-demo/meta.toml").is_file());
    assert!(data.join("registry.toml").is_file());
    assert!(state.join("values/fallback-demo.toml").is_file());
    assert!(config.join("config.toml").is_file());
}
