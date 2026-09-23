//! End-to-end contract of the `theme` setting on the command line.
//!
//! `theme` picks the palette of the interactive interface: `terminal` follows the terminal's own
//! colors, and `skit` keeps the version 0.4 look. Each test runs the real binary against its own
//! temporary directories and leaves the resulting `config.toml` as the checked artifact.

use std::{collections::BTreeMap, fs};

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
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
        let mut command = Command::cargo_bin("skit").unwrap();
        command
            .env("SKIT_LANG", "en")
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env_remove("NO_COLOR");
        command
    }

    fn run(&self, args: &[&str]) -> Run {
        let output = self.command().args(args).output().unwrap();
        Run {
            code: output.status.code().unwrap(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }

    fn config_file(&self) -> String {
        fs::read_to_string(self.config.path().join("config.toml")).unwrap_or_default()
    }
}

fn json(text: &str) -> BTreeMap<String, String> {
    serde_json::from_str(text.trim()).unwrap()
}

#[test]
fn setting_the_theme_stores_it_and_reads_it_back() {
    let sandbox = Sandbox::new();
    let set = sandbox.run(&["config", "theme", "terminal"]);
    assert_eq!(set.code, 0, "{}", set.stderr);
    assert_eq!(set.stdout, "theme = terminal\n");
    assert!(
        sandbox.config_file().contains("theme = \"terminal\""),
        "{}",
        sandbox.config_file()
    );

    let read = sandbox.run(&["config", "theme", "--json"]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert_eq!(json(&read.stdout)["theme"], "terminal");

    let set = sandbox.run(&["config", "theme", "skit", "--json"]);
    assert_eq!(set.code, 0, "{}", set.stderr);
    assert_eq!(json(&set.stdout)["theme"], "skit");
    assert_eq!(sandbox.run(&["config", "theme"]).stdout, "skit\n");
}

#[test]
fn an_unknown_theme_is_refused_and_changes_nothing() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "theme", "skit"]);
    let before = sandbox.config_file();
    let refused = sandbox.run(&["config", "theme", "neon"]);
    assert_eq!(refused.code, 2, "{}", refused.stderr);
    assert!(
        refused.stderr.contains("neon") && refused.stderr.contains("terminal, skit"),
        "{}",
        refused.stderr
    );
    assert_eq!(sandbox.config_file(), before);
}

#[test]
fn every_listing_includes_the_theme() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "theme", "terminal"]);
    let listing = sandbox.run(&["config"]);
    assert_eq!(listing.code, 0, "{}", listing.stderr);
    assert!(
        listing
            .stdout
            .lines()
            .any(|line| line.trim_start().starts_with("theme") && line.ends_with("terminal")),
        "{}",
        listing.stdout
    );
    let listing = sandbox.run(&["config", "--json"]);
    assert_eq!(json(&listing.stdout)["theme"], "terminal");
}

#[test]
fn the_config_key_completes() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env("COMPLETE", "bash")
        .env("_CLAP_COMPLETE_INDEX", "2")
        .env("_CLAP_COMPLETE_COMP_TYPE", "9")
        .env("_CLAP_COMPLETE_SPACE", "true")
        .args(["--", "skit", "config", "th"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let candidates = String::from_utf8(output.stdout).unwrap();
    assert!(
        candidates.lines().any(|line| line == "theme"),
        "{candidates}"
    );
}
