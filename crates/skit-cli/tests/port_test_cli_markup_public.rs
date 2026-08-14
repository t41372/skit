use std::{fs, path::PathBuf, process::Output};

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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("source parent");
        }
        fs::write(&path, body).expect("source");
        path
    }

    fn add_python(&self, name: &str) {
        let source = self.source(&format!("{name}.py"), "print(1)\n");
        assert_success(&self.run(&[
            "add",
            source.to_str().unwrap(),
            "--name",
            name,
            "--no-input",
        ]));
    }

    fn add_command(&self, name: &str, template: &str) {
        assert_success(&self.run(&["add", "--cmd", template, "--name", name]));
    }

    fn write_state(&self, slug: &str, body: &str) {
        let dir = self.state.path().join("values");
        fs::create_dir_all(&dir).expect("values dir");
        fs::write(dir.join(format!("{slug}.toml")), body).expect("state");
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_code(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", combined(output));
}

fn assert_success(output: &Output) {
    assert_code(output, 0);
}

#[test]
fn test_add_summary_escapes_markup_in_name_and_description() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[
        "add",
        "--cmd",
        "echo {x}",
        "--name",
        "[red]evil[/red]",
        "--description",
        "[b]d[/b]",
    ]);
    assert_success(&output);
    let shown = combined(&output);
    assert!(shown.contains("[red]evil[/red]"), "{shown}");
    assert!(shown.contains("[b]d[/b]"), "{shown}");
}

#[test]
fn test_add_deps_summary_escapes_markup() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--dep",
        "demo[bold]",
        "--no-input",
    ]);
    assert_success(&output);
    assert!(combined(&output).contains("demo[bold]"), "{}", combined(&output));
}

#[test]
fn test_add_not_py_file_warning_escapes_markup_in_filename() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("[red]evil[bold].txt", "hi\n");
    let output = sandbox.run(&["add", source.to_str().unwrap()]);
    assert_code(&output, 2);
    assert!(combined(&output).contains("[red]evil[bold].txt"), "{}", combined(&output));
}

#[test]
fn test_remove_escapes_markup_in_name() {
    let sandbox = Sandbox::new();
    sandbox.add_command("[blue]hi[/blue]", "echo hi");
    let output = sandbox.run(&["remove", "[blue]hi[/blue]", "--yes"]);
    assert_success(&output);
    assert!(combined(&output).contains("[blue]hi[/blue]"), "{}", combined(&output));
}

#[test]
fn test_not_found_error_escapes_markup_in_argument() {
    let output = Sandbox::new().run(&["deps", "[red]ghost[/red]"]);
    assert_code(&output, 1);
    assert!(combined(&output).contains("[red]ghost[/red]"), "{}", combined(&output));
}

#[test]
fn test_params_command_placeholder_line_escapes_markup() {
    let sandbox = Sandbox::new();
    sandbox.add_command("e", "echo {msg}");
    sandbox.write_state("e", "[values]\nmsg = \"[green]hello[/green]\"\n");
    let output = sandbox.run(&["params", "e"]);
    assert_success(&output);
    assert!(combined(&output).contains("[green]hello[/green]"), "{}", combined(&output));
}

#[test]
fn test_preset_list_escapes_markup_in_name_and_values() {
    let sandbox = Sandbox::new();
    sandbox.add_command("a", "echo {CITY}");
    sandbox.write_state(
        "a",
        "[presets.\"[blue]prod[/blue]\"]\nCITY = \"[red]Taipei[/red]\"\n",
    );
    let output = sandbox.run(&["preset", "list", "a"]);
    assert_success(&output);
    let shown = combined(&output);
    assert!(shown.contains("[blue]prod[/blue]"), "{shown}");
    assert!(shown.contains("[red]Taipei[/red]"), "{shown}");
}

#[test]
fn test_preset_delete_unknown_escapes_markup_in_preset_name() {
    let sandbox = Sandbox::new();
    sandbox.add_command("a", "echo hi");
    let output = sandbox.run(&["preset", "delete", "a", "[red]nope[/red]"]);
    assert_code(&output, 1);
    assert!(combined(&output).contains("[red]nope[/red]"), "{}", combined(&output));
}

#[test]
fn test_validate_preset_unknown_escapes_markup() {
    let sandbox = Sandbox::new();
    sandbox.add_command("a", "echo hi");
    let output = sandbox.run(&["run", "a", "--preset", "[red]nope[/red]"]);
    assert_code(&output, 2);
    assert!(combined(&output).contains("[red]nope[/red]"), "{}", combined(&output));
}

#[test]
fn test_deps_view_escapes_markup() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a");
    assert_success(&sandbox.run(&["deps", "a", "--dep", "demo[bold]"]));
    let output = sandbox.run(&["deps", "a"]);
    assert_success(&output);
    assert!(combined(&output).contains("demo[bold]"), "{}", combined(&output));
}

#[test]
fn test_deps_set_summary_escapes_markup() {
    let sandbox = Sandbox::new();
    sandbox.add_python("a");
    let output = sandbox.run(&["deps", "a", "--dep", "demo[bold]"]);
    assert_success(&output);
    assert!(combined(&output).contains("demo[bold]"), "{}", combined(&output));
}

#[test]
fn test_doctor_missing_reference_escapes_markup_in_name() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("tool", "#!/bin/sh\necho hi\n");
    assert_success(&sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--exe",
        "--ref",
        "--name",
        "[red]gone[/red]",
        "--no-input",
    ]));
    fs::remove_file(&source).expect("remove source");
    let output = sandbox.run(&["doctor"]);
    assert!(combined(&output).contains("[red]gone[/red]"), "{}", combined(&output));
}

#[test]
fn test_config_set_unknown_language_escapes_markup() {
    let output = Sandbox::new().run(&["config", "lang", "[red]xx-YY[/red]"]);
    assert_code(&output, 2);
    assert!(combined(&output).contains("[red]xx-YY[/red]"), "{}", combined(&output));
}

#[test]
fn test_config_set_unknown_mirror_escapes_markup() {
    let output = Sandbox::new().run(&["config", "mirror", "[red]nope[/red]"]);
    assert_code(&output, 2);
    assert!(combined(&output).contains("[red]nope[/red]"), "{}", combined(&output));
}

#[test]
fn test_edit_missing_reference_source_escapes_markup_in_path() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("[red]weird[bold]/job.py", "print(1)\n");
    assert_success(&sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--ref",
        "--name",
        "refjob",
        "--no-input",
    ]));
    fs::remove_file(&source).expect("remove source");
    let output = sandbox.run(&["edit", "refjob", "--no-input"]);
    assert_code(&output, 1);
    assert!(combined(&output).contains("[red]weird[bold]"), "{}", combined(&output));
}

#[test]
fn test_list_table_renders_markup_literally_end_to_end() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("[red]boom[bold]/tool", "#!/bin/sh\necho hi\n");
    assert_success(&sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--exe",
        "--ref",
        "--name",
        "mkup-path",
        "--description",
        "[blue]hi[/blue]",
        "--no-input",
    ]));
    fs::remove_file(&source).expect("remove source");
    let output = sandbox.run(&["list"]);
    assert_success(&output);
    let shown = combined(&output);
    assert!(shown.contains("[blue]hi[/blue]"), "{shown}");
    assert!(shown.contains("missing"), "{shown}");
}
