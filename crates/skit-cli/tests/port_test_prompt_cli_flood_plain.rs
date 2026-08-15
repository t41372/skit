use std::{
    fs,
    io::{Read as _, Write as _},
    path::PathBuf,
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;
use tempfile::TempDir;

const AUTO: usize = 30;
const PREVIEW: usize = 20;

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
    fn fields(&self, name: &str) -> Vec<String> {
        let output = self.command().args(["show", name, "--json"]).output().unwrap();
        assert_eq!(output.status.code(), Some(0));
        let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
        payload["fields"].as_array().unwrap().iter().map(|field| field["key"].as_str().unwrap().to_owned()).collect()
    }
}

fn run_pty(s: &Sandbox, args: &[&str], chunks: &[&[u8]]) -> (u32, String) {
    let pair = native_pty_system().openpty(PtySize { rows: 35, cols: 140, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args).cwd(s.home.path()).env("TERM", "xterm-256color").env("SKIT_DATA_DIR", s.data.path()).env("SKIT_STATE_DIR", s.state.path()).env("SKIT_CONFIG_DIR", s.config.path()).env("SKIT_LANG", "en").env("HOME", s.home.path()).env("USERPROFILE", s.home.path());
    let mut child = pair.slave.spawn_command(command).unwrap(); drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || { let mut bytes = Vec::new(); reader.read_to_end(&mut bytes).unwrap(); bytes });
    let mut writer = pair.master.take_writer().unwrap(); thread::sleep(Duration::from_millis(120));
    let _ = writer.write_all(b"\x1b[1;1R"); let _ = writer.flush();
    for chunk in chunks { thread::sleep(Duration::from_millis(180)); if writer.write_all(chunk).is_err() { break; } let _ = writer.flush(); }
    let status = child.wait().unwrap(); drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).replace("\r\n", "\n").replace('\r', "");
    (status.exit_code(), output)
}

fn many(count: usize) -> String { (0..count).map(|i| format!("{{{{h{i}}}}}")).collect::<Vec<_>>().join(" ") + "\n" }

#[test]
fn test_add_interactive_flood_defaults_to_none_and_caps_the_listing() {
    let s = Sandbox::new(); s.set_plain();
    let source = s.source("big.prompt.md", &many(AUTO + 5));
    let (code, output) = run_pty(&s, &["add", source.to_str().unwrap(), "-n", "big"], &[b"\r", b"\r", b"-\r"]);
    assert_eq!(code, 0, "{output}");
    assert!(output.contains(&format!("…and {} more", AUTO + 5 - PREVIEW)), "{output}");
    assert!(output.contains("[none]") || output.contains("none"), "flooded picker did not advertise/use the frozen none default: {output}");
    assert!(s.fields("big").is_empty(), "plain Enter on the flooded picker managed hidden values");
}

#[test]
fn test_add_interactive_flooded_numbers_address_the_previewed_names_only() {
    let s = Sandbox::new(); s.set_plain();
    let source = s.source("blind.prompt.md", &many(AUTO + 5));
    let beyond = (PREVIEW + 3).to_string();
    let selection = format!("3,{beyond}\r");
    let (code, output) = run_pty(&s, &["add", source.to_str().unwrap(), "-n", "blind"], &[b"\r", selection.as_bytes(), b"-\r"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(s.fields("blind"), ["h2"], "an index whose name was never previewed was guessed/accepted");
}
