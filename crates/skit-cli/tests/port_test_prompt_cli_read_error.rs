use std::{fs, path::PathBuf, process::{Command, Output}};

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

    fn source(&self, name: &str, body: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
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
}

fn combined(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[cfg(unix)]
#[test]
fn test_add_prompt_read_oserror_is_a_clean_store_error() {
    use std::os::unix::{fs::PermissionsExt as _, process::CommandExt as _};

    let sandbox = Sandbox::new();
    let source = sandbox.source("p.prompt.md", b"Review this\n");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o000)).unwrap();

    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u32>().ok())
        .expect("POSIX id -u");
    if uid == 0 {
        for root in [sandbox.data.path(), sandbox.state.path(), sandbox.config.path(), sandbox.home.path()] {
            fs::set_permissions(root, fs::Permissions::from_mode(0o777)).unwrap();
        }
    }

    let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
    sandbox.configure(&mut command);
    command.args(["add", source.to_str().unwrap(), "--prompt", "--no-input"]);
    if uid == 0 {
        command.uid(65_534).gid(65_534);
    }
    let output = command.output().unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("Can't read"), "{shown}");
    assert!(shown.contains(source.to_str().unwrap()), "{shown}");
    assert!(shown.to_ascii_lowercase().contains("permission denied"), "{shown}");
    assert!(!sandbox.data.path().join("scripts").exists(), "read failure created library state");
}
