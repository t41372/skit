use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

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

    fn spawn(&self, args: &[&str]) -> Child {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        self.configure(&mut command);
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn add_prompt(&self) {
        let source = self.home.path().join("p.prompt.md");
        fs::write(&source, "{{a}}\n").unwrap();
        let output = self.run(&[
            "add",
            source.to_str().unwrap(),
            "--name",
            "p",
            "--no-input",
        ]);
        assert_success(&output);
    }

    fn show(&self) -> Value {
        let output = self.run(&["show", "p", "--json"]);
        assert_success(&output);
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn meta_path(&self) -> PathBuf {
        self.data.path().join("scripts/p/meta.toml")
    }

    fn meta_lock_path(&self) -> PathBuf {
        self.data.path().join(".locks/p.meta.lock")
    }

    fn registry_lock_path(&self) -> PathBuf {
        self.data.path().join("registry.native.lock")
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", combined(output));
}

fn lock_file(path: &Path) -> File {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
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
    file.lock().unwrap();
    file
}

fn wait_until_locked(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let probe = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap();
        match probe.try_lock() {
            Ok(()) => {
                probe.unlock().unwrap();
                assert!(Instant::now() < deadline, "writer never acquired {}", path.display());
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(error) => panic!("could not probe {}: {error}", path.display()),
        }
    }
}

fn assert_blocked(child: &mut Child, context: &str) {
    thread::sleep(Duration::from_millis(120));
    assert!(child.try_wait().unwrap().is_none(), "{context} did not wait on the entry transaction");
}

fn wait_success(child: Child) -> Output {
    let output = child.wait_with_output().unwrap();
    assert_success(&output);
    output
}

fn parsed_meta(sandbox: &Sandbox) -> toml::Value {
    toml::from_str(&fs::read_to_string(sandbox.meta_path()).unwrap()).unwrap()
}

#[test]
fn test_prompt_meta_setters_preserve_concurrent_distinct_fields() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt();

    // update_settings takes p.meta.lock and then registry.native.lock. Holding the latter lets the
    // first writer park *inside* the entry transaction, exactly like frozen Python's paused
    // _write_meta hook, while the second writer has to wait on the same persistent entry lock.
    let registry = lock_file(&sandbox.registry_lock_path());
    let mut pin = sandbox.spawn(&["params", "p", "--runner", "claude"]);
    wait_until_locked(&sandbox.meta_lock_path());
    let mut interpolate = sandbox.spawn(&["params", "p", "--no-interpolate"]);
    assert_blocked(&mut interpolate, "interpolate writer");

    registry.unlock().unwrap();
    wait_success(pin);
    wait_success(interpolate);

    let shown = sandbox.show();
    assert_eq!(shown["runner"], "claude");
    assert_eq!(shown["interpolate"], false);
}

#[test]
fn test_prompt_and_generic_meta_setters_share_one_entry_lock() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt();

    let registry = lock_file(&sandbox.registry_lock_path());
    let mut pin = sandbox.spawn(&["params", "p", "--runner", "claude"]);
    wait_until_locked(&sandbox.meta_lock_path());
    let mut needs = sandbox.spawn(&["deps", "p", "--need", "jq"]);
    assert_blocked(&mut needs, "generic needs writer");

    registry.unlock().unwrap();
    wait_success(pin);
    wait_success(needs);

    let meta = parsed_meta(&sandbox);
    assert_eq!(meta["runner"].as_str(), Some("claude"));
    assert_eq!(
        meta["needs"].as_array().unwrap().iter().map(|value| value.as_str().unwrap()).collect::<Vec<_>>(),
        ["jq"]
    );
}

#[test]
fn test_remove_waits_for_meta_writer_and_leaves_no_resurrectable_orphan() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt();

    let registry = lock_file(&sandbox.registry_lock_path());
    let pin = sandbox.spawn(&["params", "p", "--runner", "claude"]);
    wait_until_locked(&sandbox.meta_lock_path());
    let mut remove = sandbox.spawn(&["remove", "p", "--yes"]);
    assert_blocked(&mut remove, "remove");
    assert!(sandbox.data.path().join("scripts/p").is_dir(), "remove deleted the entry under an active metadata writer");

    registry.unlock().unwrap();
    wait_success(pin);
    wait_success(remove);

    assert!(!sandbox.data.path().join("scripts/p").exists(), "removed entry directory was resurrected");
    assert!(sandbox.meta_lock_path().is_file(), "persistent entry lock inode was deleted");

    let show = sandbox.run(&["show", "p", "--json"]);
    assert_eq!(show.status.code(), Some(1), "removed entry still resolves: {}", combined(&show));

    let rebuild = sandbox.run(&["doctor", "--rebuild", "--json"]);
    assert_success(&rebuild);
    let list = sandbox.run(&["list", "--json"]);
    assert_success(&list);
    let entries: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(entries, serde_json::json!([]), "doctor rebuild resurrected an orphaned prompt: {entries}");
}
