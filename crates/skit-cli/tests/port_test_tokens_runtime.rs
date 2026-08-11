//! Runtime placement of Python v0.4 `tests/test_tokens.py::test_default_env_and_now_paths`.
//!
//! Rust deliberately makes `skit_application::tokens::TokenContext` explicit, so faking a context
//! there would not test the Python contract: omitted env/time come from the ambient run. This test
//! crosses the real CLI composition boundary and a real command child instead. The stored intent is
//! still the token string; the child must receive the freshly expanded process environment/date.
//! Frozen oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.

use std::fs;

use assert_cmd::Command;
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
            .env("COLUMNS", "200")
            .env("TERM", "xterm-256color")
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("PSModulePath")
            .current_dir(self.home.path());
        command
    }

    fn combined(output: &std::process::Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }

    fn assert_success(args: &[&str], output: &std::process::Output) {
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn state_text(&self) -> String {
        let path = self.state.path().join("values/ambient.toml");
        fs::read_to_string(path).unwrap_or_default()
    }

    fn state_doc(&self) -> toml::Value {
        toml::from_str(&self.state_text()).expect("ambient state must be valid TOML")
    }
}

fn is_yyyy_mm_dd(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

#[test]
fn test_default_env_and_now_paths() {
    let sandbox = Sandbox::new();
    let added = sandbox
        .command()
        .args([
            "add",
            "--cmd",
            "echo VALUE:{value}",
            "--name",
            "ambient",
            "--no-input",
        ])
        .output()
        .unwrap();
    Sandbox::assert_success(&["add", "ambient"], &added);

    let env_value = "ambient-process-env-value-7e0a";
    let env_run = sandbox
        .command()
        .env("SKIT_TOKEN_TEST", env_value)
        .args([
            "run",
            "ambient",
            "--set",
            "value={env:SKIT_TOKEN_TEST}",
            "--no-input",
        ])
        .output()
        .unwrap();
    Sandbox::assert_success(&["run", "ambient", "env"], &env_run);
    let env_text = Sandbox::combined(&env_run);
    assert!(
        env_text
            .lines()
            .any(|line| line.trim() == format!("VALUE:{env_value}")),
        "the real child did not receive the ambient environment expansion:\n{env_text}"
    );
    assert!(
        !env_text.contains("{env:SKIT_TOKEN_TEST}"),
        "unexpanded environment token reached output:\n{env_text}"
    );
    let env_state = sandbox.state_doc();
    assert_eq!(
        env_state["values"]["value"].as_str(),
        Some("{env:SKIT_TOKEN_TEST}")
    );
    assert_eq!(
        env_state["last_run"]["values"]["value"].as_str(),
        Some("{env:SKIT_TOKEN_TEST}")
    );
    assert!(
        !sandbox.state_text().contains(env_value),
        "ambient expansion was incorrectly frozen into persisted intent"
    );

    let today_run = sandbox
        .command()
        .args([
            "run",
            "ambient",
            "--set",
            "value={today}",
            "--no-input",
        ])
        .output()
        .unwrap();
    Sandbox::assert_success(&["run", "ambient", "today"], &today_run);
    let today_text = Sandbox::combined(&today_run);
    let delivered_date = today_text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("VALUE:")
            .filter(|value| is_yyyy_mm_dd(value))
    });
    assert!(
        delivered_date.is_some(),
        "the real child did not receive a runtime YYYY-MM-DD expansion:\n{today_text}"
    );
    assert!(
        !today_text.contains("{today}"),
        "unexpanded date token reached output:\n{today_text}"
    );
    let today_state = sandbox.state_doc();
    assert_eq!(today_state["values"]["value"].as_str(), Some("{today}"));
    assert_eq!(
        today_state["last_run"]["values"]["value"].as_str(),
        Some("{today}")
    );
}
