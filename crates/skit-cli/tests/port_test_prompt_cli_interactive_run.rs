use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tempfile::TempDir;

struct Sandbox { data: TempDir, state: TempDir, config: TempDir, home: TempDir, tools: TempDir }
impl Sandbox {
    fn new() -> Self { Self { data: TempDir::new().unwrap(), state: TempDir::new().unwrap(), config: TempDir::new().unwrap(), home: TempDir::new().unwrap(), tools: TempDir::new().unwrap() } }
    fn command(&self) -> assert_cmd::Command {
        let mut c = assert_cmd::cargo::cargo_bin_cmd!("skit");
        c.env("SKIT_DATA_DIR", self.data.path()).env("SKIT_STATE_DIR", self.state.path()).env("SKIT_CONFIG_DIR", self.config.path()).env("SKIT_LANG", "en").env("HOME", self.home.path()).env("USERPROFILE", self.home.path()).current_dir(self.home.path()); c
    }
    fn source(&self, name: &str, body: &str) -> PathBuf { let p = self.home.path().join(name); fs::write(&p, body).unwrap(); p }
    fn add_prompt(&self, pin: Option<&str>) {
        let p = self.source("p.prompt.md", "Do {{a}}\n");
        let mut args = vec!["add", p.to_str().unwrap(), "--name", "p", "--no-input"];
        if let Some(pin) = pin { args.extend(["--runner", pin]); }
        self.command().args(&args).assert().success();
    }
    fn set_last(&self, runner: &str) { fs::create_dir_all(self.state.path()).unwrap(); fs::write(self.state.path().join("prompt.toml"), format!("last_runner = {runner:?}\n")).unwrap(); }
    fn last(&self) -> String { fs::read_to_string(self.state.path().join("prompt.toml")).unwrap_or_default().lines().find_map(|l| l.strip_prefix("last_runner = \"").and_then(|v| v.strip_suffix('"'))).unwrap_or_default().to_owned() }
    fn set_form(&self, form: &str) { self.command().args(["config", "form", form]).assert().success(); }
    fn configure_runner(&self, name: &str, executable: &Path) { self.command().args(["runner", "add", name, "--force", "--", executable.to_str().unwrap(), "{{prompt}}"]).assert().success(); }
}

fn compile_probe(root: &Path, name: &str) -> PathBuf {
    let src = root.join(format!("{name}.rs"));
    fs::write(&src, "fn main() {}\n").unwrap();
    let exe = root.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() });
    assert!(ProcessCommand::new("rustc").arg(src).arg("-o").arg(&exe).status().unwrap().success()); exe
}

fn run_pty(s: &Sandbox, args: &[&str], chunks: &[&[u8]], path: Option<&Path>) -> (u32, String) {
    let pair = native_pty_system().openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut c = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    c.args(args).cwd(s.home.path()).env("TERM", "xterm-256color").env("SKIT_DATA_DIR", s.data.path()).env("SKIT_STATE_DIR", s.state.path()).env("SKIT_CONFIG_DIR", s.config.path()).env("SKIT_LANG", "en").env("HOME", s.home.path()).env("USERPROFILE", s.home.path());
    if let Some(path) = path { c.env("PATH", path); }
    let mut child = pair.slave.spawn_command(c).unwrap(); drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap(); let drain = thread::spawn(move || { let mut b=Vec::new(); reader.read_to_end(&mut b).unwrap(); b });
    let mut writer = pair.master.take_writer().unwrap(); thread::sleep(Duration::from_millis(140)); let _=writer.write_all(b"\x1b[1;1R"); let _=writer.flush();
    for chunk in chunks { thread::sleep(Duration::from_millis(180)); if writer.write_all(chunk).is_err() { break; } let _=writer.flush(); }
    let status=child.wait().unwrap(); drop(writer); let out=String::from_utf8_lossy(&drain.join().unwrap()).replace("\r\n","\n").replace('\r',""); (status.exit_code(),out)
}

#[test]
fn test_run_prompt_interactive_ask_prefilled_from_last_picked() {
    let s=Sandbox::new(); s.set_form("plain"); s.add_prompt(None); s.set_last("opencode");
    let amp=compile_probe(s.tools.path(), "amp");
    let (code, out)=run_pty(&s, &["run","p","--set","a=1","--plain"], &[b"amp\r"], Some(s.tools.path()));
    assert_eq!(code,0,"{out}");
    assert!(out.contains("[opencode]"), "remembered runner was not the visible plain-form default: {out}");
    assert_eq!(s.last(),"amp");
    assert!(out.contains("amp -x runs this prompt once"),"{out}");
    assert!(amp.is_file());
}

#[test]
fn test_run_prompt_inline_stale_pin_prefills_last_configured_pick() {
    let s=Sandbox::new();
    let first=compile_probe(s.tools.path(), "first"); let remembered=compile_probe(s.tools.path(), "remembered");
    s.configure_runner("first", &first); s.configure_runner("remembered", &remembered);
    // Add while the stale pin exists, then replace the configured set without that name.
    s.configure_runner("removed", &remembered); s.add_prompt(Some("removed"));
    s.command().args(["runner","remove","removed","--yes"]).assert().success();
    s.set_last("remembered"); s.set_form("tui");
    let (code,out)=run_pty(&s, &["run","p","--set","a=1"], &[b"\x12"], Some(s.tools.path()));
    assert_eq!(code,0,"{out}");
    assert!(!out.contains("removed"), "stale pin remained the selected inline runner: {out}");
    assert_eq!(s.last(),"remembered");
}
