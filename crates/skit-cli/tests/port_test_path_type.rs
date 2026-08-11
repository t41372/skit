//! Exact CLI-boundary ports of Python v0.4 `tests/test_path_type.py` edit/resync contracts.
//!
//! The Python helper returned declarations plus warning codes. Rust's public edit boundary is
//! `skit params`, so these tests require both the persisted final row and the absence/presence of
//! user-visible edit consequences. No production seam is added for the port.

use std::fs;

use assert_cmd::Command;
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
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("PSModulePath")
            .current_dir(self.home.path());
        command
    }

    fn output(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> std::process::Output {
        let output = self.output(args);
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn params(&self, slug: &str) -> Value {
        let output = self.ok(&["params", slug, "--json"]);
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn add_exe(&self, name: &str) {
        let source = self.home.path().join(format!("{name}-program"));
        fs::create_dir(&source).unwrap();
        self.ok(&[
            "add",
            source.to_str().unwrap(),
            "--exe",
            "--name",
            name,
            "--no-input",
        ]);
    }

    fn add_python(&self, name: &str, source: &str) {
        let path = self.home.path().join(format!("{name}.py"));
        fs::write(&path, source).unwrap();
        self.ok(&["add", path.to_str().unwrap(), "--name", name, "--no-input"]);
    }
}

fn parameter<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("missing parameter {name}: {document}"))
}

const SCRIPT: &str = "SRC = \"./data.csv\"\nRETRIES = 3\nprint(SRC, RETRIES)\n";

#[test]
fn test_edit_declared_accepts_path_type() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    let edited = sandbox.output(&["params", "binary", "--add", "src", "--type", "src=path"]);
    assert!(
        edited.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&edited.stdout),
        String::from_utf8_lossy(&edited.stderr)
    );
    assert!(
        edited.stderr.is_empty(),
        "accepted path edit produced warnings: {}",
        String::from_utf8_lossy(&edited.stderr)
    );
    assert_eq!(parameter(&sandbox.params("binary"), "src")["type"], "path");
}

#[test]
fn test_resync_preserves_declared_path() {
    let sandbox = Sandbox::new();
    sandbox.add_python("paths", SCRIPT);
    sandbox.ok(&["params", "paths", "--manage", "SRC"]);
    sandbox.ok(&[
        "params",
        "paths",
        "--type",
        "SRC=path",
        "--prompt",
        "SRC=Which file? ",
    ]);
    let before = sandbox.params("paths");
    assert_eq!(parameter(&before, "SRC")["type"], "path");
    assert_eq!(parameter(&before, "SRC")["prompt"], "Which file? ");

    let resync = sandbox.output(&["params", "paths", "--resync"]);
    assert!(
        resync.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&resync.stdout),
        String::from_utf8_lossy(&resync.stderr)
    );
    let after = sandbox.params("paths");
    let src = parameter(&after, "SRC");
    assert_eq!(
        src["type"], "path",
        "resync lost the str->path refinement: {src}"
    );
    assert_eq!(
        src["prompt"], "Which file? ",
        "resync discarded the user-owned prompt: {src}"
    );
    assert!(
        !String::from_utf8_lossy(&resync.stderr)
            .to_ascii_lowercase()
            .contains("dropped"),
        "a surviving SRC refinement was reported as dropped: {}",
        String::from_utf8_lossy(&resync.stderr)
    );
}

#[test]
fn test_resync_still_corrects_real_type_drift() {
    let sandbox = Sandbox::new();
    sandbox.add_python("paths", SCRIPT);
    sandbox.ok(&["params", "paths", "--manage", "RETRIES"]);
    sandbox.ok(&["params", "paths", "--type", "RETRIES=path"]);
    assert_eq!(
        parameter(&sandbox.params("paths"), "RETRIES")["type"],
        "path"
    );

    let resync = sandbox.output(&["params", "paths", "--resync"]);
    assert!(
        resync.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&resync.stdout),
        String::from_utf8_lossy(&resync.stderr)
    );
    assert_eq!(
        parameter(&sandbox.params("paths"), "RETRIES")["type"],
        "int",
        "real path-over-int drift must re-anchor to source truth"
    );
}
