//! Real-child launcher ports from Python `tests/test_launcher.py` at `main@206f9ef`.
//!
//! A compiled fake `uv` is used as the external runtime, not as a fake skit service. The real
//! `skit run` composition root resolves the entry, prepares the launch, builds argv/env, spawns the
//! child, waits for it, and records the result. The probe reads the launched script while the launch
//! lease is live and captures the exact environment it received.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use skit_application::LibraryService;
use skit_domain::EntrySettings;
use skit_store::{FileConfigStore, FileStore};
use tempfile::TempDir;

struct RunFixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
    capture: PathBuf,
    uv: PathBuf,
}

impl RunFixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let capture = tools.path().join("launch-capture.txt");
        let uv = tools.path().join(uv_executable_name());
        compile_uv_probe(tools.path(), &uv);
        Self {
            data,
            state,
            config,
            home,
            tools,
            capture,
            uv,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
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
            .env("SKIT_LAUNCH_CAPTURE", &self.capture)
            .current_dir(self.home.path());
        command
    }

    fn add_python_at(&self, source: &Path, name: &str) {
        let output = self
            .command()
            .arg("add")
            .arg(source)
            .args(["--name", name, "--no-input"])
            .output()
            .unwrap();
        assert_success(&output);
    }

    fn add_python(&self, name: &str, body: &str) -> PathBuf {
        let source = self.home.path().join(format!("{name}.py"));
        fs::write(&source, body).unwrap();
        self.add_python_at(&source, name);
        source
    }

    fn pin_uv_and_workdir(&self, selector: &str, uv: &Path, workdir: Option<&str>) {
        let store = FileStore::new(self.data.path());
        let service = LibraryService::new(store);
        let entry = service.show(selector).unwrap();
        let mut settings = EntrySettings::from_meta(&entry.meta);
        settings.interpreter = uv.display().to_string();
        service
            .update_settings(
                &entry,
                &settings,
                workdir.unwrap_or(entry.meta.workdir.as_str()),
            )
            .unwrap();
    }

    fn pin_uv(&self, selector: &str) {
        self.pin_uv_and_workdir(selector, &self.uv, None);
    }

    fn run(&self, selector: &str, configure: impl FnOnce(&mut Command)) -> Output {
        let mut command = self.command();
        command.args(["run", selector, "--no-input"]);
        configure(&mut command);
        command.output().unwrap()
    }

    fn captured(&self) -> String {
        fs::read_to_string(&self.capture).unwrap()
    }

    fn enable_full_mirror(&self) {
        let config = FileConfigStore::new(self.config.path());
        config
            .set_many(&BTreeMap::from([
                ("mirror.pypi".to_owned(), "tsinghua".to_owned()),
                ("mirror.github".to_owned(), "nju".to_owned()),
                ("mirror.npm".to_owned(), "npmmirror".to_owned()),
                ("mirror".to_owned(), "on".to_owned()),
            ]))
            .unwrap();
        let mirror = config.mirror().unwrap();
        assert!(mirror.enabled);
        assert_eq!(mirror.pypi, "https://pypi.tuna.tsinghua.edu.cn/simple");
        assert_eq!(
            mirror.python_install,
            "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/"
        );
    }

    fn install_private_uv(&self) -> PathBuf {
        let output = self
            .data
            .path()
            .join("bin")
            .join(if cfg!(windows) { "uv.exe" } else { "uv" });
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        compile_uv_probe(self.tools.path(), &output);
        output
    }
}

fn compile_uv_probe(root: &Path, output: &Path) {
    let source = root.join("uv_probe.rs");
    if !source.exists() {
        fs::write(
            &source,
            r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let args = env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect::<Vec<_>>();
    let script = args.windows(2).find_map(|pair| (pair[0] == "--script").then(|| PathBuf::from(&pair[1])));
    let script_text = script.as_ref().map(|path| fs::read_to_string(path).expect("read launched script")).unwrap_or_default();
    let capture = env::var_os("SKIT_LAUNCH_CAPTURE").expect("capture path");
    let default_index = env::var("UV_DEFAULT_INDEX").unwrap_or_else(|_| "<missing>".to_owned());
    let python_mirror = env::var("UV_PYTHON_INSTALL_MIRROR").unwrap_or_else(|_| "<missing>".to_owned());
    fs::write(
        capture,
        format!(
            "ARGS={}\nDEFAULT={}\nPYTHON={}\nSCRIPT={}\nCONTENT={}\n",
            args.join("\u{1f}"),
            default_index,
            python_mirror,
            script.as_ref().map(|path| path.display().to_string()).unwrap_or_default(),
            script_text.replace('\n', "\\n"),
        ),
    ).expect("write launch capture");
}
"#,
        )
        .unwrap();
    }
    let status = Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(output)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile fake uv probe");
}

#[cfg(windows)]
fn uv_executable_name() -> &'static str {
    "uv-probe.exe"
}

#[cfg(not(windows))]
fn uv_executable_name() -> &'static str {
    "uv-probe"
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn test_run_entry_real_execution() {
    let fixture = RunFixture::new();
    fixture.add_python("real", "print('ok')\n");
    fixture.pin_uv("real");

    let output = fixture.run("real", |_| {});
    assert_success(&output);
    let capture = fixture.captured();
    assert!(capture.contains("ARGS=run\u{1f}--no-project\u{1f}--script\u{1f}"), "{capture}");
    assert!(capture.contains("CONTENT=print('ok')\\n"), "{capture}");
}

#[test]
fn test_find_uv_private_bin_fallback() {
    let fixture = RunFixture::new();
    fixture.add_python("private", "print('private')\n");
    let private = fixture.install_private_uv();
    let empty_path = fixture.tools.path().join("empty-path");
    fs::create_dir(&empty_path).unwrap();

    let output = fixture.run("private", |command| {
        command.env("PATH", &empty_path);
    });
    assert_success(&output);
    let capture = fixture.captured();
    assert!(capture.contains("CONTENT=print('private')\\n"), "{capture}");
    assert!(private.is_file(), "managed private uv disappeared during launch");
}

#[test]
fn test_run_entry_command_entry() {
    let fixture = RunFixture::new();
    let output = fixture
        .command()
        .args([
            "add",
            "--cmd",
            "echo hello",
            "--name",
            "greet",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_success(&output);

    let output = fixture.run("greet", |_| {});
    assert_success(&output);
}

#[test]
fn test_run_entry_injects_mirror_env() {
    let fixture = RunFixture::new();
    fixture.add_python("mirror", "print('mirror')\n");
    fixture.pin_uv("mirror");
    fixture.enable_full_mirror();

    let output = fixture.run("mirror", |command| {
        command
            .env_remove("UV_DEFAULT_INDEX")
            .env_remove("UV_INDEX_URL")
            .env_remove("UV_PYTHON_INSTALL_MIRROR");
    });
    assert_success(&output);
    let capture = fixture.captured();
    assert!(
        capture.contains("DEFAULT=https://pypi.tuna.tsinghua.edu.cn/simple"),
        "{capture}"
    );
    assert!(
        capture.contains(
            "PYTHON=https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/"
        ),
        "{capture}"
    );
}

#[test]
fn test_run_entry_no_mirror_env_when_disabled() {
    let fixture = RunFixture::new();
    fixture.add_python("plain", "print('plain')\n");
    fixture.pin_uv("plain");

    let output = fixture.run("plain", |command| {
        command
            .env_remove("UV_DEFAULT_INDEX")
            .env_remove("UV_INDEX_URL")
            .env_remove("UV_PYTHON_INSTALL_MIRROR");
    });
    assert_success(&output);
    let capture = fixture.captured();
    assert!(capture.contains("DEFAULT=<missing>"), "{capture}");
    assert!(capture.contains("PYTHON=<missing>"), "{capture}");
}

#[test]
fn test_run_entry_keeps_user_index_when_mirror_enabled() {
    let fixture = RunFixture::new();
    fixture.add_python("user-index", "print('user')\n");
    fixture.pin_uv("user-index");
    fixture.enable_full_mirror();

    let output = fixture.run("user-index", |command| {
        command
            .env("UV_DEFAULT_INDEX", "https://user/own/simple")
            .env_remove("UV_INDEX_URL")
            .env_remove("UV_PYTHON_INSTALL_MIRROR");
    });
    assert_success(&output);
    let capture = fixture.captured();
    assert!(capture.contains("DEFAULT=https://user/own/simple"), "{capture}");
    assert!(
        capture.contains(
            "PYTHON=https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/"
        ),
        "{capture}"
    );
}

#[test]
fn test_run_entry_succeeds_for_copy_mode_entry_with_deleted_origin() {
    let fixture = RunFixture::new();
    let origin = fixture.home.path().join("origin");
    fs::create_dir(&origin).unwrap();
    let source = origin.join("legacy.py");
    fs::write(&source, "print('stored survives')\n").unwrap();
    fixture.add_python_at(&source, "legacy");
    fixture.pin_uv_and_workdir("legacy", &fixture.uv, Some("origin"));
    fs::remove_dir_all(&origin).unwrap();

    let output = fixture.run("legacy", |_| {});
    assert_success(&output);
    let capture = fixture.captured();
    assert!(capture.contains("CONTENT=print('stored survives')\\n"), "{capture}");
}
