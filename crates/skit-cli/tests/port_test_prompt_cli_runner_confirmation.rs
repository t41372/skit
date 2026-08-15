use std::{
    fs,
    io::{Read as _, Write as _},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_store::{FileConfigStore, PromptRunner};
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

    fn command(&self) -> assert_cmd::Command {
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

    fn config_store(&self) -> FileConfigStore {
        FileConfigStore::new(self.config.path())
    }

    fn runner_exists(&self, name: &str) -> bool {
        self.config_store()
            .runners()
            .unwrap()
            .iter()
            .any(|runner| runner.name == name)
    }

    fn config_path(&self) -> PathBuf {
        self.config.path().join("config.toml")
    }

    fn write_config(&self, text: &str) {
        fs::create_dir_all(self.config.path()).unwrap();
        fs::write(self.config_path(), text).unwrap();
    }
}

fn wait_until_output(shared: &Arc<Mutex<Vec<u8>>>, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = {
            let bytes = shared.lock().unwrap();
            String::from_utf8_lossy(&bytes).into_owned()
        };
        if text.contains(needle) {
            return;
        }
        assert!(Instant::now() < deadline, "PTY never printed {needle:?}; current output: {text}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn run_confirm(
    sandbox: &Sandbox,
    args: &[&str],
    prompt_needle: &str,
    before_answer: impl FnOnce(),
    answer: &[u8],
) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.cwd(sandbox.home.path());
    command.env("TERM", "xterm-256color");
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("SKIT_LANG", "en");
    command.env("HOME", sandbox.home.path());
    command.env("USERPROFILE", sandbox.home.path());

    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let shared = Arc::new(Mutex::new(Vec::new()));
    let writer_shared = Arc::clone(&shared);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut chunk = [0_u8; 512];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => writer_shared.lock().unwrap().extend_from_slice(&chunk[..count]),
                Err(_) => break,
            }
        }
    });
    let mut writer = pair.master.take_writer().unwrap();
    wait_until_output(&shared, prompt_needle);
    before_answer();
    writer.write_all(answer).unwrap();
    writer.flush().unwrap();
    let status = child.wait().unwrap();
    drop(writer);
    drain.join().unwrap();
    let output = String::from_utf8_lossy(&shared.lock().unwrap())
        .replace("\r\n", "\n")
        .replace('\r', "");
    (status.exit_code(), output)
}

#[test]
fn test_runner_remove_confirms_unless_yes() {
    let sandbox = Sandbox::new();
    let (code, output) = run_confirm(
        &sandbox,
        &["runner", "remove", "amp"],
        "Remove the agent",
        || {},
        b"y\n",
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Remove the agent \"amp\"?"), "{output}");
    assert!(!sandbox.runner_exists("amp"));
    assert!(output.contains("Runner amp removed."), "{output}");
}

#[test]
fn test_runner_remove_abort_keeps_the_runner() {
    let sandbox = Sandbox::new();
    let (code, output) = run_confirm(
        &sandbox,
        &["runner", "remove", "amp"],
        "Remove the agent",
        || {},
        b"n\n",
    );
    assert_eq!(code, 1, "{output}");
    assert!(sandbox.runner_exists("amp"), "negative confirmation still removed amp: {output}");
    assert!(!output.contains("Runner amp removed."), "{output}");
}

#[test]
fn test_runner_remove_raw_row_refuses_if_index_shifted_during_confirmation() {
    let sandbox = Sandbox::new();
    let original = concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [",
        "{ name = \"good\", argv = [\"good\", \"{{prompt}}\"] }, ",
        "{ name = \"target\", argv = [\"target\"] }, ",
        "{ name = \"other\", argv = [\"other\", \"{{prompt}}\"] }",
        "]\n",
    );
    sandbox.write_config(original);
    let shifted = concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [",
        "{ name = \"inserted\", argv = [\"inserted\", \"{{prompt}}\"] }, ",
        "{ name = \"good\", argv = [\"good\", \"{{prompt}}\"] }, ",
        "{ name = \"target\", argv = [\"target\"] }, ",
        "{ name = \"other\", argv = [\"other\", \"{{prompt}}\"] }",
        "]\n",
    );
    let (code, output) = run_confirm(
        &sandbox,
        &["runner", "remove", "--row", "1"],
        "Remove runner row 1",
        || sandbox.write_config(shifted),
        b"y\n",
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("changed before it could be removed"), "{output}");
    assert_eq!(fs::read_to_string(sandbox.config_path()).unwrap(), shifted);
}

#[test]
fn test_runner_remove_name_refuses_if_key_is_replaced_during_confirmation() {
    let sandbox = Sandbox::new();
    sandbox.config_store().set_runner(
        PromptRunner {
            name: "victim".to_owned(),
            argv: vec!["old".to_owned(), "{{prompt}}".to_owned()],
        },
        false,
    ).unwrap();
    let replacement = PromptRunner {
        name: "victim".to_owned(),
        argv: vec!["new".to_owned(), "--important".to_owned(), "{{prompt}}".to_owned()],
    };
    let (code, output) = run_confirm(
        &sandbox,
        &["runner", "remove", "victim"],
        "Remove the agent",
        || {
            sandbox.config_store().set_runner(replacement.clone(), true).unwrap();
        },
        b"y\n",
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("changed before it could be removed"), "{output}");
    assert_eq!(
        sandbox.config_store().runners().unwrap(),
        [replacement],
        "replacement runner was incorrectly deleted"
    );
}
