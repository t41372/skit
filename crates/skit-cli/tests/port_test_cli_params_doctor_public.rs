use std::{fs, path::{Path, PathBuf}, process::Output};

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

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).expect("source");
        path
    }

    fn add_python(&self, name: &str, body: &str, reference: bool) -> PathBuf {
        let source = self.source(&format!("{name}.py"), body);
        let mut args = vec![
            "add",
            source.to_str().expect("utf8 source"),
            "--name",
            name,
            "--no-input",
        ];
        if reference {
            args.push("--ref");
        }
        let output = self.run(&args);
        assert_success(&output);
        source
    }

    fn write_state(&self, slug: &str, body: &str) -> PathBuf {
        let root = self.state.path().join("values");
        fs::create_dir_all(&root).expect("values dir");
        let path = root.join(format!("{slug}.toml"));
        fs::write(&path, body).expect("state");
        path
    }

    fn fake_uv_dir(&self) -> PathBuf {
        let bin = self.home.path().join("fake-bin");
        fs::create_dir_all(&bin).expect("fake-bin");
        let name = if cfg!(windows) { "uv.exe" } else { "uv" };
        let uv = bin.join(name);
        fs::copy(env!("CARGO_BIN_EXE_skit"), &uv).expect("copy executable as uv");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut permissions = fs::metadata(&uv).expect("uv metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&uv, permissions).expect("uv mode");
        }
        bin
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn flat(output: &Output) -> String {
    combined(output).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_code(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", combined(output));
}

fn assert_success(output: &Output) {
    assert_code(output, 0);
}

fn managed_python(rows: &[(&str, bool)]) -> String {
    let mut body = String::from("# /// script\n# dependencies = []\n#\n# [tool.skit]\n# schema = 1\n#\n");
    for (name, secret) in rows {
        body.push_str("# [[tool.skit.params]]\n");
        body.push_str(&format!("# name = {name:?}\n"));
        body.push_str("# kind = \"const\"\n# type = \"str\"\n");
        if *secret {
            body.push_str("# secret = true\n");
        }
        body.push_str("#\n");
    }
    body.push_str("# ///\n");
    for (name, _) in rows {
        body.push_str(&format!("{name} = \"x\"\n"));
    }
    body
}

#[test]
fn test_params_python_table_with_secret() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a", &managed_python(&[("API", true)]), false);
    sandbox.write_state("a", "[values]\nAPI = \"shown\"\n");
    let output = sandbox.run(&["params", "a"]);
    assert_success(&output);
    let shown = combined(&output);
    assert!(shown.contains("API"), "{shown}");
    assert!(!shown.contains("shown"), "a secret value must not be rendered in plaintext: {shown}");
}

#[test]
fn test_params_secret_purges_stored_last_value_and_presets() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a", &managed_python(&[("API_KEY", false)]), false);
    let state = sandbox.write_state(
        "a",
        "[values]\nAPI_KEY = \"plaintext-secret-123\"\n[presets.prod]\nAPI_KEY = \"plaintext-secret-123\"\n",
    );
    let output = sandbox.run(&["params", "a", "--secret", "API_KEY"]);
    assert_success(&output);
    let shown = flat(&output);
    assert!(!shown.contains("plaintext-secret-123"), "{shown}");
    assert!(
        shown.contains("Removed previously stored plaintext value(s) for now-secret parameter(s): API_KEY"),
        "{shown}"
    );
    let raw = fs::read_to_string(&state).expect("state");
    assert!(!raw.contains("plaintext-secret-123"), "plaintext remained on disk: {raw}");
    assert!(!raw.contains("prod"), "an emptied preset must be dropped entirely: {raw}");
}

#[test]
fn test_params_secret_does_not_purge_other_still_public_params() {
    let sandbox = Sandbox::new();
    sandbox.add_python(
        "a",
        &managed_python(&[("API_KEY", false), ("CITY", false)]),
        false,
    );
    let state = sandbox.write_state(
        "a",
        "[values]\nAPI_KEY = \"secretval\"\nCITY = \"Taipei\"\n",
    );
    let output = sandbox.run(&["params", "a", "--secret", "API_KEY"]);
    assert_success(&output);
    let raw = fs::read_to_string(state).expect("state");
    assert!(!raw.contains("secretval"), "secret value survived: {raw}");
    assert!(raw.contains("CITY = \"Taipei\""), "public sibling was purged: {raw}");
}

#[test]
fn test_params_edit_without_stored_value_prints_no_purge_message() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a", &managed_python(&[("CITY", false)]), false);
    let output = sandbox.run(&["params", "a", "--secret", "CITY"]);
    assert_success(&output);
    assert!(
        !combined(&output).contains("Removed previously stored plaintext"),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_doctor_uv_found() {
    let sandbox = Sandbox::new();
    let uv = sandbox.fake_uv_dir();
    let output = sandbox.command().env("PATH", &uv).arg("doctor").output().expect("doctor");
    assert_success(&output);
}

#[test]
fn test_doctor_uv_missing() {
    let sandbox = Sandbox::new();
    let empty = sandbox.home.path().join("empty-bin");
    fs::create_dir(&empty).expect("empty path");
    let output = sandbox.command().env("PATH", &empty).arg("doctor").output().expect("doctor");
    assert_code(&output, 1);
}

#[test]
fn test_doctor_rebuild() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "print(1)\n", false);
    let uv = sandbox.fake_uv_dir();
    let output = sandbox
        .command()
        .env("PATH", &uv)
        .args(["doctor", "--rebuild"])
        .output()
        .expect("doctor rebuild");
    assert_success(&output);
}

#[test]
fn test_doctor_reports_missing_reference() {
    let sandbox = Sandbox::new();
    let source = sandbox.add_python("ref", "print(1)\n", true);
    fs::remove_file(&source).expect("remove reference source");
    let uv = sandbox.fake_uv_dir();
    let output = sandbox.command().env("PATH", &uv).arg("doctor").output().expect("doctor");
    assert_success(&output);
    assert!(combined(&output).contains("ref"), "{}", combined(&output));
}
