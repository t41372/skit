use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use skit_store::{FileConfigStore, PromptRunner};
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools: TempDir::new().unwrap(),
        }
    }

    fn configure(&self, command: &mut Command) {
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
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        self.configure(&mut command);
        command.args(args).output().unwrap()
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn add_prompt(&self, body: &str, runner: &str) {
        let source = self.source("p.prompt.md", body);
        let output = self.run(&[
            "add",
            source.to_str().unwrap(),
            "--name",
            "p",
            "--runner",
            runner,
            "--no-input",
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn add_runner(&self, name: &str, argv: &[&str]) {
        let mut args = vec!["runner", "add", name, "--force", "--"];
        args.extend(argv.iter().copied());
        let output = self.run(&args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn prompt_path(&self) -> PathBuf {
        self.data.path().join("scripts/p/prompt.md")
    }

    fn lock_path(&self, suffix: &str) -> PathBuf {
        self.data.path().join(".locks").join(format!("p.{suffix}.lock"))
    }

    fn open_lock(&self, suffix: &str) -> File {
        let path = self.lock_path(suffix);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap();
        if file.metadata().unwrap().len() == 0 {
            file.set_len(1).unwrap();
        }
        file
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn compile_recorder(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let source = root.join(format!("{name}.rs"));
    let capture = root.join(format!("{name}.capture"));
    fs::write(
        &source,
        r#"
use std::{env, fs};
fn main() {
    let capture = env::var_os("SKIT_SNAPSHOT_CAPTURE").expect("capture");
    let args = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    fs::write(capture, args.join("\u{001e}")).unwrap();
}
"#,
    )
    .unwrap();
    let executable = root.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    });
    assert!(
        Command::new("rustc")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap()
            .success()
    );
    (executable, capture)
}

fn wait_for_shared_launch_lease(launch_lock: &File) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match launch_lock.try_lock() {
            Ok(()) => {
                launch_lock.unlock().unwrap();
                assert!(
                    Instant::now() < deadline,
                    "run never reached prepare_launch's shared launch lease"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(error) => panic!("could not probe launch lease: {error}"),
        }
    }
}

#[test]
fn test_real_run_spawns_the_same_prompt_snapshot_it_validated() {
    let sandbox = Sandbox::new();
    let (runner, capture) = compile_recorder(sandbox.tools.path(), "snapshot-runner");
    sandbox.add_runner(
        "snapshot-runner",
        &[runner.to_str().unwrap(), "{{prompt}}"],
    );
    sandbox.add_prompt("ONE-PREPARED-BODY", "snapshot-runner");

    let meta_lock = sandbox.open_lock("meta");
    meta_lock.lock().unwrap();
    let launch_lock = sandbox.open_lock("launch");

    let mut child_command = Command::new(env!("CARGO_BIN_EXE_skit"));
    sandbox.configure(&mut child_command);
    child_command
        .env("SKIT_SNAPSHOT_CAPTURE", &capture)
        .args(["run", "p", "--no-input"]);
    let mut child = child_command.spawn().unwrap();

    // prepare_launch takes the shared launch lease only after source_snapshot, rendering, runner
    // resolution, and the first launch-plan validation have completed. Because we hold meta.lock,
    // the child is now parked at a deterministic post-validation boundary.
    wait_for_shared_launch_lease(&launch_lock);
    fs::write(
        sandbox.prompt_path(),
        "SECOND-BODY-MUST-NOT-BE-READ",
    )
    .unwrap();
    meta_lock.unlock().unwrap();

    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let captured = fs::read_to_string(&capture).expect("runner must have spawned");
    assert!(captured.contains("ONE-PREPARED-BODY"), "{captured}");
    assert!(!captured.contains("SECOND-BODY-MUST-NOT-BE-READ"), "{captured}");
}

#[test]
fn test_real_run_transparency_and_amp_note_use_the_prepared_runner_row() {
    let sandbox = Sandbox::new();
    let (amp, capture) = compile_recorder(sandbox.tools.path(), "amp");
    // Materialize the frozen built-in seed row; the fake executable is resolved from PATH only at
    // launch and does not emulate any skit behavior.
    let seeded = sandbox.run(&["runner", "list"]);
    assert_eq!(seeded.status.code(), Some(0), "{}", combined(&seeded));
    sandbox.add_prompt("Prepared runner body", "amp");

    let meta_lock = sandbox.open_lock("meta");
    meta_lock.lock().unwrap();
    let launch_lock = sandbox.open_lock("launch");

    let mut child_command = Command::new(env!("CARGO_BIN_EXE_skit"));
    sandbox.configure(&mut child_command);
    child_command
        .env("PATH", sandbox.tools.path())
        .env("SKIT_SNAPSHOT_CAPTURE", &capture)
        .args(["run", "p", "--no-input"]);
    let mut child = child_command.spawn().unwrap();

    wait_for_shared_launch_lease(&launch_lock);
    FileConfigStore::new(sandbox.config.path())
        .set_runner(
            PromptRunner {
                name: "amp".to_owned(),
                argv: vec!["replacement-agent".to_owned(), "{{prompt}}".to_owned()],
            },
            true,
        )
        .unwrap();
    meta_lock.unlock().unwrap();

    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("amp -x runs this prompt once"), "{shown}");
    assert!(shown.contains("amp -x"), "{shown}");
    assert!(!shown.contains("replacement-agent"), "{shown}");
    let captured = fs::read_to_string(capture).expect("prepared amp row must have spawned");
    assert!(captured.starts_with("-x\u{1e}") || captured.starts_with("-x\x1e") || captured.starts_with("-x"), "{captured}");
    assert!(amp.is_file());
}
