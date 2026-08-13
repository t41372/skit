//! CLI/store ports from Python v0.4 `tests/test_js_analyzer.py` at `main@206f9ef`.

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
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        };
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        sandbox
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

    fn add(&self, filename: &str, name: &str, text: &str) {
        let source = self.home.path().join(filename);
        fs::write(&source, text).unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", name, "--no-input"])
            .assert()
            .success();
    }
}

#[test]
fn test_params_manage_writes_block_into_js_copy() {
    let sandbox = Sandbox::new();
    sandbox.add(
        "deploy.js",
        "jsp1",
        "#!/usr/bin/env node\nconst CITY = 800;\nconsole.log(CITY);\n",
    );
    sandbox
        .command()
        .args(["params", "jsp1", "--manage", "CITY"])
        .assert()
        .success();

    let copy = sandbox.data.path().join("scripts/jsp1/script.js");
    let text = fs::read_to_string(copy).unwrap();
    assert!(text.contains("// [tool.skit]"), "{text}");
    assert!(text.contains("name = \"CITY\""), "{text}");
    assert!(text.starts_with("#!/usr/bin/env node\n"), "{text}");
    assert!(text.find("#!") < text.find("// /// script"));
}

#[test]
fn test_params_show_lists_ts_const() {
    let sandbox = Sandbox::new();
    sandbox.add("show.ts", "tsp1", "const CITY: string = \"Taipei\";\n");
    let output = sandbox.command().args(["params", "tsp1"]).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("CITY"), "{text}");
}
