use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
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

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
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

    fn set_form_plain(&self) {
        self.command().args(["config", "form", "plain"]).assert().success();
    }

    fn show_json(&self, selector: &str) -> Value {
        let output = self.command().args(["show", selector, "--json"]).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn entry_exists(&self, slug: &str) -> bool {
        self.data.path().join("scripts").join(slug).is_dir()
    }
}

fn run_pty(sandbox: &Sandbox, args: &[&str], term: &str, chunks: &[&[u8]]) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.cwd(sandbox.home.path());
    command.env("TERM", term);
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("SKIT_LANG", "en");
    command.env("HOME", sandbox.home.path());
    command.env("USERPROFILE", sandbox.home.path());
    command.env("XDG_CONFIG_HOME", sandbox.home.path().join("xdg-config"));
    command.env("XDG_DATA_HOME", sandbox.home.path().join("xdg-data"));
    command.env("XDG_STATE_HOME", sandbox.home.path().join("xdg-state"));

    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(120));
    let _ = writer.write_all(b"\x1b[1;1R");
    let _ = writer.flush();
    for chunk in chunks {
        thread::sleep(Duration::from_millis(180));
        if writer.write_all(chunk).is_err() {
            break;
        }
        let _ = writer.flush();
    }
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap())
        .replace("\r\n", "\n")
        .replace('\r', "");
    (status.exit_code(), output)
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn last_runner(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .find_map(|line| line.strip_prefix("last_runner = \"").and_then(|value| value.strip_suffix('"')))
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn test_add_prompt_interactive_runner_pick_pins_and_remembers() {
    let sandbox = Sandbox::new();
    sandbox.set_form_plain();
    let source = sandbox.source("p.prompt.md", "{{a}}\n");
    let (code, output) = run_pty(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "pinned"],
        "xterm-256color",
        &[b"\r", b"all\r", b"codex\r"],
    );
    assert_eq!(code, 0, "{output}");
    assert_eq!(sandbox.show_json("pinned")["runner"], "codex");
    assert_eq!(
        last_runner(&sandbox.state.path().join("prompt.toml")),
        "codex",
        "an interactive runner pick must become the next picker default"
    );
    assert!(flat(&output).contains("Runs with codex"), "{output}");
}

#[test]
fn test_add_prompt_interactive_panel_cancel_exits_130() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("cancel.prompt.md", "Do {{a}}\n");
    let (code, output) = run_pty(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "cancelled"],
        "xterm-256color",
        &[b"\x1b"],
    );
    assert_eq!(code, 130, "{output}");
    assert!(!sandbox.entry_exists("cancelled"), "Esc committed a prompt entry: {output}");
    assert!(flat(&output).contains("Cancelled"), "{output}");
}

#[test]
fn test_add_prompt_unknown_runner_refused_before_the_panel() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("x.prompt.md", "Do {{a}}\n");
    let (code, output) = run_pty(
        &sandbox,
        &["add", source.to_str().unwrap(), "--runner", "ghost"],
        "xterm-256color",
        &[],
    );
    assert_eq!(code, 2, "{output}");
    assert!(flat(&output).contains("Unknown runner"), "{output}");
    assert!(!sandbox.entry_exists("x"), "unknown runner reached commit/panel: {output}");
    assert!(!output.contains("Package dependencies") && !output.contains("Prompt variables"), "review panel opened before runner validation: {output}");
}

#[test]
fn test_add_prompt_term_dumb_keeps_line_prompts() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("dumb.prompt.md", "Do {{a}}\n");
    let (code, output) = run_pty(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "dumbly"],
        "dumb",
        &[b"\r", b"all\r", b"-\r"],
    );
    assert_eq!(code, 0, "{output}");
    let shown = flat(&output);
    assert!(shown.contains("Manage") || shown.contains("Description (optional)"), "TERM=dumb skipped line prompt flow: {shown}");
    assert_eq!(
        sandbox.show_json("dumbly")["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a"]
    );
}

#[test]
fn test_add_flood_cap_manages_nothing_and_says_so() {
    let sandbox = Sandbox::new();
    let body = (0..36)
        .map(|index| format!("{{{{h{index}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let source = sandbox.source("big.prompt.md", &(body + "\n"));
    let output = sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--no-input"])
        .output()
        .unwrap();
    let shown = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.status.code(), Some(0), "{shown}");
    assert!(shown.contains("too many to manage automatically"), "{shown}");
    assert_eq!(sandbox.show_json("big")["fields"], serde_json::json!([]));
}
