//! Strong behavioral ports of Python v0.4 `tests/test_raw.py`.
//!
//! The Python tests only observed `script_override`; this Rust port additionally records the real
//! `uv --script` path and bytes. Remembered state is seeded in the raw/cleanup cases so an
//! accidental fall-through into normal injection cannot pass vacuously.

use std::{fs, path::PathBuf, process::Command as StdCommand};

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
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("PSModulePath")
            .current_dir(self.home.path());
        command
    }

    fn stored_script(&self) -> PathBuf {
        self.data.path().join("scripts/hello/script.py")
    }

    fn write_entry(&self) {
        let dir = self.data.path().join("scripts/hello");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("script.py"),
            concat!(
                "# /// script\n",
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
            ),
        )
        .unwrap();
        fs::write(
            dir.join("meta.toml"),
            concat!(
                "schema = 1\n",
                "name = \"hello\"\n",
                "kind = \"python\"\n",
                "mode = \"copy\"\n",
                "source = \"/original/hello.py\"\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-08-10T00:00:00Z\"\n",
                "id = \"4123456789abcdef0123456789abcdef\"\n",
                "workdir = \"invoke\"\n",
                "description = \"\"\n",
            ),
        )
        .unwrap();
        fs::write(self.data.path().join("registry.toml"), "[entries.hello]\n").unwrap();
    }

    fn seed_last_city(&self, value: &str) {
        let path = self.state.path().join("values/hello.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("[values]\nCITY = {value:?}\n")).unwrap();
    }

    fn compile_fake_uv(&self) -> PathBuf {
        let bin = self.home.path().join("fake-uv-bin");
        fs::create_dir_all(&bin).unwrap();
        let source = self.home.path().join("fake_uv_raw.rs");
        fs::write(
            &source,
            r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let i = args.iter().position(|arg| arg == "--script").expect("--script");
    let script = PathBuf::from(args.get(i + 1).expect("script path"));
    fs::write(env::var_os("SKIT_CAPTURE_PATH").expect("capture path"), script.to_string_lossy().as_bytes())
        .expect("write path");
    fs::copy(&script, env::var_os("SKIT_CAPTURE_BYTES").expect("capture bytes"))
        .expect("copy script bytes");
}
"#,
        )
        .unwrap();
        let executable = bin.join(if cfg!(windows) { "uv.exe" } else { "uv" });
        let status = StdCommand::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    fn run_and_capture(&self, args: &[&str]) -> (std::process::Output, PathBuf, String) {
        let uv = self.compile_fake_uv();
        let path_capture = self.home.path().join("captured-path.txt");
        let bytes_capture = self.home.path().join("captured-script.py");
        let output = self
            .command()
            .env("PATH", uv.parent().unwrap())
            .env("SKIT_CAPTURE_PATH", &path_capture)
            .env("SKIT_CAPTURE_BYTES", &bytes_capture)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let launched_path = PathBuf::from(fs::read_to_string(path_capture).unwrap());
        let launched_bytes = fs::read_to_string(bytes_capture).unwrap();
        (output, launched_path, launched_bytes)
    }
}

#[test]
fn test_raw_skips_form_and_injection() {
    let sandbox = Sandbox::new();
    sandbox.write_entry();
    sandbox.seed_last_city("Kaohsiung");

    let (_output, launched_path, launched_bytes) =
        sandbox.run_and_capture(&["run", "hello", "--raw", "--no-input"]);

    assert_eq!(launched_path, sandbox.stored_script());
    assert!(
        launched_bytes.contains("CITY = \"Taipei\""),
        "{launched_bytes}"
    );
    assert!(!launched_bytes.contains("Kaohsiung"), "{launched_bytes}");
}

#[test]
fn test_default_run_injects() {
    let sandbox = Sandbox::new();
    sandbox.write_entry();
    sandbox.seed_last_city("Kaohsiung");

    let (_output, launched_path, launched_bytes) =
        sandbox.run_and_capture(&["run", "hello", "--no-input"]);

    assert_ne!(launched_path, sandbox.stored_script());
    assert!(launched_bytes.contains("Kaohsiung"), "{launched_bytes}");
    assert!(
        !launched_path.exists(),
        "injected artifact survived launch: {launched_path:?}"
    );
}

#[test]
fn test_no_values_runs_copy_directly() {
    let sandbox = Sandbox::new();
    sandbox.write_entry();

    let (_output, launched_path, launched_bytes) =
        sandbox.run_and_capture(&["run", "hello", "--no-input"]);

    assert_eq!(launched_path, sandbox.stored_script());
    assert!(
        launched_bytes.contains("CITY = \"Taipei\""),
        "{launched_bytes}"
    );
}

#[test]
fn test_raw_does_not_leave_injected_artifact() {
    let sandbox = Sandbox::new();
    sandbox.write_entry();
    sandbox.seed_last_city("Kaohsiung");

    let (_output, launched_path, _bytes) =
        sandbox.run_and_capture(&["run", "hello", "--raw", "--no-input"]);

    assert_eq!(launched_path, sandbox.stored_script());
    let stored_script = sandbox.stored_script();
    let entry_dir = stored_script.parent().unwrap();
    assert!(
        fs::read_dir(entry_dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".injected")),
        "raw run left an injected artifact beside the stored copy"
    );
}

#[test]
fn test_normal_run_cleans_injected_artifact() {
    let sandbox = Sandbox::new();
    sandbox.write_entry();
    sandbox.seed_last_city("Kaohsiung");

    let (_output, launched_path, launched_bytes) =
        sandbox.run_and_capture(&["run", "hello", "--no-input"]);

    assert_ne!(launched_path, sandbox.stored_script());
    assert!(launched_bytes.contains("Kaohsiung"), "{launched_bytes}");
    assert!(
        !launched_path.exists(),
        "normal run failed to remove the real staged artifact: {launched_path:?}"
    );
    let stored_script = sandbox.stored_script();
    let entry_dir = stored_script.parent().unwrap();
    assert!(
        fs::read_dir(entry_dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".injected")),
        "normal run left an injected artifact beside the stored copy"
    );
}
