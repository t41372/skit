//! Ambient-state half of the mechanical port of `tests/test_tokens.py`
//! (`main@206f9ef`). The deterministic scanner tests live in `skit-application`; this black-box
//! test pins the CLI adapter that supplies the process environment and local clock.

use predicates::prelude::*;
use tempfile::TempDir;

fn command(data: &TempDir, state: &TempDir, config: &TempDir) -> assert_cmd::Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en");
    command
}

#[test]
fn test_default_env_and_now_paths() {
    // Python omits `env` and `now` here. Rust makes ambient state an explicit CLI-boundary adapter,
    // so exercise the real adapter through a run preview instead of constructing a TokenContext.
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();

    command(&data, &state, &config)
        .args([
            "add",
            "--cmd",
            "printf '%s' {value}",
            "--name",
            "Token default",
            "--no-input",
        ])
        .assert()
        .success();

    command(&data, &state, &config)
        .env("SKIT_TOKEN_TEST", "v")
        .args([
            "run",
            "token-default",
            "--set",
            "value={env:SKIT_TOKEN_TEST}",
            "--dry-run",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("v"));

    command(&data, &state, &config)
        .args([
            "run",
            "token-default",
            "--set",
            "value={today}",
            "--dry-run",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"\d{4}-\d{2}-\d{2}").unwrap());
}
