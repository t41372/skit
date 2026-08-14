use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    thread,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_store::FileStore;
use tempfile::TempDir;

pub struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        };
        sandbox.set_form("plain");
        sandbox
    }

    pub fn set_form(&self, form: &str) {
        fs::write(
            self.config.path().join("config.toml"),
            format!("form = {form:?}\n"),
        )
        .unwrap();
    }

    pub fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    pub fn home(&self) -> &Path {
        self.home.path()
    }

    pub fn data(&self) -> &Path {
        self.data.path()
    }

    pub fn command(&self) -> Command {
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
            .env("TERM", "dumb")
            .env("COLUMNS", "200")
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .current_dir(self.home.path());
        command
    }

    pub fn run(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }

    pub fn run_pty(&self, args: &[&str], input: &str) -> (u32, String) {
        self.run_pty_with_term(args, input, "dumb")
    }

    pub fn run_pty_with_term(&self, args: &[&str], input: &str, term: &str) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 200,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(args);
        command.cwd(self.home.path());
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("SKIT_LANG", "en");
        command.env("HOME", self.home.path());
        command.env("USERPROFILE", self.home.path());
        command.env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"));
        command.env("XDG_DATA_HOME", self.home.path().join("xdg-data"));
        command.env("XDG_STATE_HOME", self.home.path().join("xdg-state"));
        command.env("TERM", term);
        command.env("COLUMNS", "200");
        command.env_remove("FORCE_COLOR");
        command.env_remove("NO_COLOR");

        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut writer = pair.master.take_writer().unwrap();
        if !input.is_empty() {
            writer.write_all(input.as_bytes()).unwrap();
            writer.flush().unwrap();
        }
        let status = child.wait().unwrap();
        drop(writer);
        let output = String::from_utf8_lossy(&drain.join().unwrap())
            .replace("\r\n", "\n")
            .replace('\r', "");
        (status.exit_code(), output)
    }

    pub fn source(&self, name: &str, body: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    pub fn draft(&self, name: &str, body: &[u8]) -> PathBuf {
        let root = self.data.path().join("drafts");
        fs::create_dir_all(&root).unwrap();
        let path = root.join(name);
        fs::write(&path, body).unwrap();
        path
    }
}

pub fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
