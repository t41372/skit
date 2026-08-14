use std::{fs, path::PathBuf, process::Output};

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
            data: TempDir::new().expect("data"),
            state: TempDir::new().expect("state"),
            config: TempDir::new().expect("config"),
            home: TempDir::new().expect("home"),
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

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("run skit")
    }

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, bytes).expect("source");
        path
    }

    fn write_state(&self, slug: &str, body: &str) {
        let root = self.state.path().join("values");
        fs::create_dir_all(&root).expect("values dir");
        fs::write(root.join(format!("{slug}.toml")), body).expect("state");
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_code(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", combined(output));
}

#[test]
fn test_run_shim_error() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "j.py",
        concat!(
            "# /// script\n",
            "# dependencies = []\n",
            "#\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"CITY\"\n",
            "# kind = \"const\"\n",
            "# type = \"str\"\n",
            "# ///\n",
            "CITY = \"Taipei\"\n",
            "print(CITY)\n",
        )
        .as_bytes(),
    );
    let added = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "j",
        "--no-input",
    ]);
    assert_code(&added, 0);
    sandbox.write_state("j", "[values]\nCITY = \"Kaohsiung\"\n");

    // Corrupt only the stored source after registration so the run reaches the same managed-value
    // injection/materialization boundary that Python's ShimError monkeypatch targeted.
    fs::write(
        sandbox.data.path().join("scripts/j/script.py"),
        b"# /// script\n# [tool.skit\nCITY = \"Taipei\"\n",
    )
    .expect("corrupt managed source");
    let output = sandbox.run(&["run", "j", "--no-input"]);
    assert_code(&output, 125);
}

#[test]
fn test_run_launch_error() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("j.py", b"print(1)\n");
    let added = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "j",
        "--no-input",
    ]);
    assert_code(&added, 0);

    let empty = sandbox.home.path().join("no-runners");
    fs::create_dir(&empty).expect("empty PATH");
    let output = sandbox
        .command()
        .env("PATH", &empty)
        .args(["run", "j", "--no-input"])
        .output()
        .expect("run without uv");
    assert_code(&output, 125);
}
