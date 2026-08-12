//! Exact behavioral port of Python `tests/test_hermeticity.py` from `main@206f9ef`.
//!
//! The Python regression came from mutation tests escaping `SKIT_*` overrides and touching the
//! developer's real library. This port deliberately crosses the public process boundary: remove
//! every `SKIT_*_DIR` override, redirect HOME/XDG into one temporary tree, then prove the data,
//! state, and config fallbacks all resolve there. A source-string check would not protect users.

#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;

struct FallbackRoots {
    home: PathBuf,
    data: PathBuf,
    state: PathBuf,
    config: PathBuf,
    xdg_data: PathBuf,
    xdg_state: PathBuf,
    xdg_config: PathBuf,
}

impl FallbackRoots {
    fn under(root: &Path) -> Self {
        let home = root.join("home");
        let xdg_data = root.join("xdg-data");
        let xdg_state = root.join("xdg-state");
        let xdg_config = root.join("xdg-config");

        #[cfg(target_os = "macos")]
        let (data, state, config) = {
            let application_support = home
                .join("Library")
                .join("Application Support")
                .join("skit");
            (
                application_support.clone(),
                application_support.clone(),
                application_support,
            )
        };

        #[cfg(not(target_os = "macos"))]
        let (data, state, config) = (
            xdg_data.join("skit"),
            xdg_state.join("skit"),
            xdg_config.join("skit"),
        );

        Self {
            home,
            data,
            state,
            config,
            xdg_data,
            xdg_state,
            xdg_config,
        }
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env_remove("SKIT_DATA_DIR")
            .env_remove("SKIT_STATE_DIR")
            .env_remove("SKIT_CONFIG_DIR")
            .env("SKIT_LANG", "en")
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_DATA_HOME", &self.xdg_data)
            .env("XDG_STATE_HOME", &self.xdg_state)
            .env("XDG_CONFIG_HOME", &self.xdg_config)
            .current_dir(&self.home);
        command
    }
}

#[test]
fn test_platformdirs_fallback_stays_isolated_when_skit_env_missing() {
    let temporary = TempDir::new().unwrap();
    let roots = FallbackRoots::under(temporary.path());
    fs::create_dir_all(&roots.home).unwrap();

    // Data resolver: create a real entry without a --data-dir or SKIT_DATA_DIR escape hatch.
    roots
        .command()
        .args([
            "add",
            "--cmd",
            "printf hermetic",
            "--name",
            "Hermetic",
            "--no-input",
        ])
        .assert()
        .success();
    let meta = roots.data.join("scripts/hermetic/meta.toml");
    assert!(meta.is_file(), "data fallback escaped temporary HOME/XDG: {meta:?}");

    // State resolver: seed an unmistakable last-run record at the platform fallback and require
    // the public list command to read it. Merely checking that the directory exists is too weak.
    let state_file = roots.state.join("values/hermetic.toml");
    fs::create_dir_all(state_file.parent().unwrap()).unwrap();
    fs::write(
        &state_file,
        "[last_run]\nat = \"2099-01-02T03:04:05Z\"\nexit = 73\n",
    )
    .unwrap();
    roots
        .command()
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "\"last_run_at\":\"2099-01-02T03:04:05Z\"",
        ))
        .stdout(predicates::str::contains("\"last_exit\":73"));

    // Config resolver: perform a real config mutation and require the write at the fallback root.
    roots
        .command()
        .args(["config", "lang", "en"])
        .assert()
        .success();
    let config_file = roots.config.join("config.toml");
    let config_text = fs::read_to_string(&config_file)
        .unwrap_or_else(|error| panic!("config fallback escaped temporary HOME/XDG: {error}"));
    assert!(
        config_text.contains("lang = \"en\""),
        "unexpected config at {config_file:?}: {config_text}"
    );

    for resolved in [&roots.data, &roots.state, &roots.config] {
        assert!(
            resolved.starts_with(temporary.path()),
            "fallback escaped the test sandbox: {resolved:?}"
        );
    }
}
