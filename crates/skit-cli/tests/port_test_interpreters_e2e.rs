#![cfg(unix)]

//! POSIX child-process ports from Python `tests/test_interpreters.py`.
//!
//! The fake `bash` is a real executable that delegates to `/bin/sh`. This keeps PATH deterministic
//! while preserving the Python contract that the overlay reaches an actual child process.

use std::{fs, os::unix::fs::PermissionsExt as _};

use assert_cmd::Command;
use skit_application::{EntryMutationRepository as _, EntryRepository as _};
use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterDelivery},
};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    bin: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            bin: TempDir::new().unwrap(),
        };
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        let bash = sandbox.bin.path().join("bash");
        fs::write(&bash, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
        let mut permissions = fs::metadata(&bash).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&bash, permissions).unwrap();
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
            .env("PATH", self.bin.path())
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .current_dir(self.home.path());
        command
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn add_shell(&self, name: &str, body: &str, reference: bool) {
        let source = self.home.path().join(format!("{name}.sh"));
        fs::write(&source, body).unwrap();
        let mut command = self.command();
        command
            .arg("add")
            .arg(&source)
            .args(["--kind", "shell", "--name", name, "--no-input"]);
        if reference {
            command.arg("--ref");
        }
        command.assert().success();
    }
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_e2e_run_shell_script() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("hi", "#!/bin/bash\necho \"shell-ran-ok\"\n", false);
    let output = sandbox.command().args(["run", "hi"]).output().unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("shell-ran-ok"), "real child output was lost: {text}");
}

#[test]
fn test_e2e_run_shell_env_param_reaches_child() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("w", "#!/bin/bash\necho \"w=$WIDTH\"\n", false);
    let store = sandbox.store();
    let entry = store.resolve("w").unwrap();
    let mut settings = EntrySettings::from_meta(&entry.meta);
    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Env;
    settings.parameters = vec![width];
    store
        .update_settings(&entry, &settings, &entry.meta.workdir)
        .unwrap();

    let output = sandbox
        .command()
        .args(["run", "w", "--set", "WIDTH=800"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("w=800"), "environment overlay missed the real child: {text}");
}

#[test]
fn test_e2e_dry_run_shows_interpreter_and_script() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("d", "#!/bin/bash\necho hi\n", false);
    let output = sandbox
        .command()
        .args(["run", "d", "--dry-run"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let flat = String::from_utf8_lossy(&output.stdout).replace('\n', "");
    assert!(flat.contains("bash"), "{flat}");
    assert!(flat.contains("script.sh"), "{flat}");
}

#[test]
fn test_e2e_run_reference_mode_shell() {
    let sandbox = Sandbox::new();
    sandbox.add_shell("ref", "#!/bin/bash\necho \"ref-ran\"\n", true);
    let output = sandbox.command().args(["run", "ref"]).output().unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("ref-ran"), "reference-mode child did not run: {text}");
}
