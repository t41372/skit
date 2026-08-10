//! Closed warning-set port from Python v0.4 `test_cli_declared_warning_codes_render`.
//!
//! All seven scenarios run before the assertion fires so one hard-error substitution cannot hide
//! missing renderers for the remaining warning codes.

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

    fn add_exe(&self, name: &str) {
        let source = self.home.path().join(format!("{name}-exe"));
        fs::create_dir(&source).unwrap();
        let output = self
            .command()
            .args(["add", source.to_str().unwrap(), "--exe", "--name", name])
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    fn add_command(&self, name: &str, template: &str) {
        let output = self
            .command()
            .args(["add", "--cmd", template, "--name", name, "--no-input"])
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }
}

fn plain(output: &std::process::Output) -> String {
    let input = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
            continue;
        }
        let ch = input[index..].chars().next().unwrap();
        result.push(ch);
        index += ch.len_utf8();
    }
    result
}

fn check(label: &str, output: &std::process::Output, expected: &str, failures: &mut Vec<String>) {
    let text = plain(output);
    if !output.status.success() || !text.lines().any(|line| line == expected) {
        failures.push(format!(
            "{label}: expected soft warning {expected:?}; status={:?}; output={text:?}",
            output.status.code()
        ));
    }
}

#[test]
fn test_cli_declared_warning_codes_render() {
    let mut failures = Vec::new();

    {
        let sandbox = Sandbox::new();
        sandbox.add_exe("notdecl");
        let _ = sandbox.run(&["params", "notdecl", "--add", "a"]);
        let output = sandbox.run(&["params", "notdecl", "--rm", "x"]);
        check(
            "not-declared",
            &output,
            "x isn't a declared parameter; skipped.",
            &mut failures,
        );
    }
    {
        let sandbox = Sandbox::new();
        sandbox.add_exe("already");
        let _ = sandbox.run(&["params", "already", "--add", "x"]);
        let output = sandbox.run(&["params", "already", "--add", "x"]);
        check(
            "already-declared",
            &output,
            "x is already declared; skipped.",
            &mut failures,
        );
    }
    {
        let sandbox = Sandbox::new();
        sandbox.add_exe("delivery");
        let _ = sandbox.run(&["params", "delivery", "--add", "x"]);
        let output = sandbox.run(&["params", "delivery", "--deliver", "x=placeholder"]);
        check(
            "bad-delivery",
            &output,
            "x: that delivery isn't available for this kind; skipped.",
            &mut failures,
        );
    }
    {
        let sandbox = Sandbox::new();
        sandbox.add_command("placeholder", "echo {other}");
        let _ = sandbox.run(&[
            "params", "placeholder", "--add", "x", "--deliver", "x=env",
        ]);
        let output = sandbox.run(&[
            "params", "placeholder", "--deliver", "x=placeholder",
        ]);
        check(
            "not-a-placeholder",
            &output,
            "x isn't a template placeholder, so it can't use placeholder delivery; skipped.",
            &mut failures,
        );
    }
    {
        let sandbox = Sandbox::new();
        sandbox.add_exe("badtype");
        let _ = sandbox.run(&["params", "badtype", "--add", "x"]);
        let output = sandbox.run(&["params", "badtype", "--type", "x=integer"]);
        check(
            "bad-type",
            &output,
            "x: unknown type; skipped (use str, int, float, bool, choice, or path).",
            &mut failures,
        );
    }
    {
        let sandbox = Sandbox::new();
        sandbox.add_exe("baddefault");
        let _ = sandbox.run(&[
            "params", "baddefault", "--add", "x", "--type", "x=int", "--default", "x=3",
        ]);
        let output = sandbox.run(&[
            "params", "baddefault", "--default", "x=notanint",
        ]);
        check(
            "bad-default",
            &output,
            "x: the default doesn't fit its type; skipped.",
            &mut failures,
        );
    }
    {
        let sandbox = Sandbox::new();
        sandbox.add_exe("choice");
        let _ = sandbox.run(&[
            "params", "choice", "--add", "x", "--help-text", "x=keep",
        ]);
        let output = sandbox.run(&[
            "params", "choice", "--type", "x=choice", "--help-text", "x=changed",
        ]);
        check(
            "choice-without-choices",
            &output,
            "x: a choice parameter needs choices; set --choices x=a,b,c.",
            &mut failures,
        );
    }

    assert!(
        failures.is_empty(),
        "the declared warning set is incomplete or changed semantics:\n{}",
        failures.join("\n")
    );
}
