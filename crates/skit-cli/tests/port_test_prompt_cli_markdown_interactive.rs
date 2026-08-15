use std::{fs, io::{Read as _, Write as _}, path::PathBuf, thread, time::Duration};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;
use tempfile::TempDir;

struct Sandbox { data: TempDir, state: TempDir, config: TempDir, home: TempDir }
impl Sandbox {
    fn new() -> Self { Self { data: TempDir::new().unwrap(), state: TempDir::new().unwrap(), config: TempDir::new().unwrap(), home: TempDir::new().unwrap() } }
    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command.env("SKIT_DATA_DIR", self.data.path()).env("SKIT_STATE_DIR", self.state.path()).env("SKIT_CONFIG_DIR", self.config.path()).env("SKIT_LANG", "en").env("HOME", self.home.path()).env("USERPROFILE", self.home.path()).current_dir(self.home.path());
        command
    }
    fn set_plain(&self) { self.command().args(["config", "form", "plain"]).assert().success(); }
    fn source(&self, name: &str, body: &str) -> PathBuf { let path = self.home.path().join(name); fs::write(&path, body).unwrap(); path }
    fn show(&self, name: &str) -> Value { let out = self.command().args(["show", name, "--json"]).output().unwrap(); assert_eq!(out.status.code(), Some(0)); serde_json::from_slice(&out.stdout).unwrap() }
}

fn run_pty(s: &Sandbox, args: &[&str], chunks: &[&[u8]]) -> (u32, String) {
    let pair = native_pty_system().openpty(PtySize { rows: 30, cols: 140, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args).cwd(s.home.path()).env("TERM", "xterm-256color").env("SKIT_DATA_DIR", s.data.path()).env("SKIT_STATE_DIR", s.state.path()).env("SKIT_CONFIG_DIR", s.config.path()).env("SKIT_LANG", "en").env("HOME", s.home.path()).env("USERPROFILE", s.home.path());
    let mut child = pair.slave.spawn_command(command).unwrap(); drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap(); let drain = thread::spawn(move || { let mut bytes = Vec::new(); reader.read_to_end(&mut bytes).unwrap(); bytes });
    let mut writer = pair.master.take_writer().unwrap(); thread::sleep(Duration::from_millis(120)); let _ = writer.write_all(b"\x1b[1;1R"); let _ = writer.flush();
    for chunk in chunks { thread::sleep(Duration::from_millis(180)); if writer.write_all(chunk).is_err() { break; } let _ = writer.flush(); }
    let status = child.wait().unwrap(); drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).replace("\r\n", "\n").replace('\r', ""); (status.exit_code(), output)
}

#[test]
fn test_add_bare_md_interactive_ask_yes_and_no() {
    let yes = Sandbox::new(); yes.set_plain();
    let source = yes.source("notes.md", "hello {{x}}\n");
    let (code, output) = run_pty(&yes, &["add", source.to_str().unwrap()], &[b"y\r", b"\r", b"\r", b"all\r", b"-\r"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(yes.show("notes")["kind"], "prompt");

    let no = Sandbox::new(); no.set_plain();
    let other = no.source("other.md", "x\n");
    let (code, output) = run_pty(&no, &["add", other.to_str().unwrap()], &[b"n\r", b"-\r"]);
    assert_eq!(code, 130, "{output}");
    assert!(output.to_ascii_lowercase().contains("nothing was added"), "{output}");
    assert!(!no.data.path().join("scripts/other").exists());
}

#[test]
fn test_add_bare_md_confirm_no_falls_through_to_kind_ask_and_honors_pick() {
    let s = Sandbox::new(); s.set_plain();
    let source = s.source("script.md", "echo hi\n");
    // Frozen v0.4's generic interpreted-kind menu is sorted alphabetically; shell is item 9.
    let (code, output) = run_pty(&s, &["add", source.to_str().unwrap()], &[b"n\r", b"9\r", b"\r", b"\r"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(s.show("script")["kind"], "shell");
}
