//! CLI/PTY ports for the Python `tests/test_uvman.py` contracts that cross configuration or the
//! terminal-consent boundary. Every download attempt is redirected to a closed localhost endpoint
//! (or through a closed localhost HTTPS proxy), so these tests are hermetic and never fetch uv.

use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpListener,
    path::PathBuf,
    thread,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tempfile::TempDir;

const OFFICIAL_BASE: &str = "https://github.com/astral-sh/uv/releases/download";

struct Sandbox {
    _root: TempDir,
    data: PathBuf,
    state: PathBuf,
    config: PathBuf,
    home: PathBuf,
    empty_path: PathBuf,
    source: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let data = root.path().join("data");
        let state = root.path().join("state");
        let config = root.path().join("config");
        let home = root.path().join("home");
        let empty_path = root.path().join("empty-path");
        for path in [&data, &state, &config, &home, &empty_path] {
            fs::create_dir_all(path).unwrap();
        }
        let source = home.join("demo.py");
        fs::write(&source, "print('demo')\n").unwrap();
        let sandbox = Self {
            _root: root,
            data,
            state,
            config,
            home,
            empty_path,
            source,
        };
        sandbox.add_python();
        sandbox
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        self.apply_env(&mut command);
        command
    }

    fn apply_env(&self, command: &mut Command) {
        command
            .env("SKIT_DATA_DIR", &self.data)
            .env("SKIT_STATE_DIR", &self.state)
            .env("SKIT_CONFIG_DIR", &self.config)
            .env("SKIT_LANG", "en")
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.join("xdg-state"))
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .current_dir(&self.home);
    }

    fn apply_pty_env(&self, command: &mut CommandBuilder) {
        command.cwd(&self.home);
        command.env("TERM", "xterm-256color");
        command.env("SKIT_DATA_DIR", &self.data);
        command.env("SKIT_STATE_DIR", &self.state);
        command.env("SKIT_CONFIG_DIR", &self.config);
        command.env("SKIT_LANG", "en");
        command.env("HOME", &self.home);
        command.env("USERPROFILE", &self.home);
        command.env("XDG_CONFIG_HOME", self.home.join("xdg-config"));
        command.env("XDG_DATA_HOME", self.home.join("xdg-data"));
        command.env("XDG_STATE_HOME", self.home.join("xdg-state"));
        command.env("PATH", &self.empty_path);
        command.env("NO_PROXY", "127.0.0.1,localhost");
        command.env("no_proxy", "127.0.0.1,localhost");
    }

    fn add_python(&self) {
        let output = self
            .command()
            .arg("add")
            .arg(&self.source)
            .args(["--name", "demo", "--no-input"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "could not create uvman fixture: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_mirror(&self, enabled: bool, uv_binary: Option<&str>, pypi: Option<&str>) {
        let mut text = format!("[mirror]\nenabled = {enabled}\n");
        if let Some(pypi) = pypi {
            text.push_str(&format!("pypi = {pypi:?}\n"));
        }
        if let Some(uv_binary) = uv_binary {
            text.push_str(&format!("uv_binary = {uv_binary:?}\n"));
        }
        fs::write(self.config.join("config.toml"), text).unwrap();
    }

    fn run_non_tty(&self) -> std::process::Output {
        self.command()
            .env("PATH", &self.empty_path)
            .args(["run", "demo", "--no-input"])
            .output()
            .unwrap()
    }

    fn run_non_tty_through_dead_proxy(&self, proxy: &str) -> std::process::Output {
        self.command()
            .env("PATH", &self.empty_path)
            .env("HTTPS_PROXY", proxy)
            .env("https_proxy", proxy)
            .env("ALL_PROXY", proxy)
            .env("all_proxy", proxy)
            .env("NO_PROXY", "")
            .env("no_proxy", "")
            .args(["run", "demo", "--no-input"])
            .output()
            .unwrap()
    }

    fn run_pty(&self, input: &[u8]) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.arg("run");
        command.arg("demo");
        command.arg("--no-input");
        self.apply_pty_env(&mut command);
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut writer = pair.master.take_writer().unwrap();
        writer.write_all(input).unwrap();
        writer.flush().unwrap();
        let status = child.wait().unwrap();
        drop(writer);
        let transcript = drain.join().unwrap();
        (status.exit_code(), String::from_utf8_lossy(&transcript).into_owned())
    }
}

fn dead_base() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

fn run_consent_case(answer: &str, expected_consent: bool) {
    let sandbox = Sandbox::new();
    let mirror = dead_base();
    sandbox.write_mirror(true, Some(&mirror), None);
    let input = format!("{answer}\r");
    let (code, transcript) = sandbox.run_pty(input.as_bytes());

    assert_eq!(code, 125, "{transcript}");
    assert!(transcript.contains("[Y/n]"), "uv consent question was not shown: {transcript}");
    if expected_consent {
        assert!(
            transcript.contains("First run — downloading uv"),
            "consenting answer did not reach the download boundary: {transcript}"
        );
        assert!(
            transcript.contains("could not download uv from"),
            "the local refusal was not reported: {transcript}"
        );
        assert!(!transcript.contains("Download declined."), "{transcript}");
    } else {
        assert!(transcript.contains("Download declined."), "{transcript}");
        assert!(
            transcript.contains("Install uv yourself"),
            "decline guidance disappeared: {transcript}"
        );
        assert!(
            !transcript.contains("First run — downloading uv"),
            "a declined answer crossed the download boundary: {transcript}"
        );
    }
}

#[test]
fn test_consent_non_interactive_auto_yes() {
    let sandbox = Sandbox::new();
    let mirror = dead_base();
    sandbox.write_mirror(true, Some(&mirror), None);
    let output = sandbox.run_non_tty();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert_eq!(output.status.code(), Some(125), "{stderr}");
    assert!(stderr.contains("First run — downloading uv"), "{stderr}");
    assert!(stderr.contains(&mirror), "configured mirror was not attempted: {stderr}");
    assert!(!stderr.contains("[Y/n]"), "non-TTY run unexpectedly asked for input: {stderr}");
    assert!(!stderr.contains("Download declined."), "{stderr}");
}

#[test]
fn test_consent_interactive_answers() {
    for (answer, expected) in [
        ("", true),
        ("y", true),
        ("Y", true),
        ("yes", true),
        ("n", false),
        ("N", false),
        ("no", false),
        ("  n  ", false),
    ] {
        run_consent_case(answer, expected);
    }
}

#[test]
fn rust_additive_consent_empty_answer_is_yes() {
    run_consent_case("", true);
}

#[test]
fn rust_additive_consent_lower_y_is_yes() {
    run_consent_case("y", true);
}

#[test]
fn rust_additive_consent_upper_y_is_yes() {
    run_consent_case("Y", true);
}

#[test]
fn rust_additive_consent_yes_is_yes() {
    run_consent_case("yes", true);
}

#[test]
fn rust_additive_consent_lower_n_is_no() {
    run_consent_case("n", false);
}

#[test]
fn rust_additive_consent_upper_n_is_no() {
    run_consent_case("N", false);
}

#[test]
fn rust_additive_consent_no_is_no() {
    run_consent_case("no", false);
}

#[test]
fn rust_additive_consent_whitespace_n_is_no() {
    run_consent_case("  n  ", false);
}

#[test]
fn test_consent_eof_is_yes() {
    let sandbox = Sandbox::new();
    let mirror = dead_base();
    sandbox.write_mirror(true, Some(&mirror), None);
    // Ctrl+D on an empty canonical PTY line is the terminal EOF that Python's input() raises as
    // EOFError. Version 0.4 treats it as consent rather than hanging or declining.
    let (code, transcript) = sandbox.run_pty(b"\x04");

    assert_eq!(code, 125, "{transcript}");
    assert!(transcript.contains("[Y/n]"), "{transcript}");
    assert!(transcript.contains("First run — downloading uv"), "{transcript}");
    assert!(!transcript.contains("Download declined."), "{transcript}");
}

#[test]
fn test_declined_raises_with_guidance() {
    let sandbox = Sandbox::new();
    let mirror = dead_base();
    sandbox.write_mirror(true, Some(&mirror), None);
    let (code, transcript) = sandbox.run_pty(b"n\r");

    assert_eq!(code, 125, "{transcript}");
    assert!(transcript.contains("Download declined."), "{transcript}");
    assert!(transcript.contains("Install uv yourself"), "{transcript}");
    assert!(transcript.contains("skit will pick it up automatically"), "{transcript}");
    assert!(!transcript.contains("First run — downloading uv"), "{transcript}");
}

#[test]
fn test_download_url_uses_configured_mirror() {
    let sandbox = Sandbox::new();
    let mirror = dead_base();
    sandbox.write_mirror(true, Some(&mirror), None);
    let output = sandbox.run_non_tty();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert_eq!(output.status.code(), Some(125), "{stderr}");
    assert!(
        stderr.contains(&format!("{mirror}/{}/uv-", skit_runtime::UV_VERSION)),
        "configured uv mirror did not own the attempted asset URL: {stderr}"
    );
}

#[test]
fn test_download_url_defaults_to_github_without_mirror() {
    let sandbox = Sandbox::new();
    sandbox.write_mirror(false, None, None);
    let proxy = dead_base();
    let output = sandbox.run_non_tty_through_dead_proxy(&proxy);
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert_eq!(output.status.code(), Some(125), "{stderr}");
    assert!(
        stderr.contains(OFFICIAL_BASE),
        "an unconfigured mirror did not fall back to the official uv release URL: {stderr}"
    );
}

#[test]
fn test_download_url_github_when_uv_binary_blank() {
    let sandbox = Sandbox::new();
    sandbox.write_mirror(true, Some(""), Some("https://x.invalid/simple"));
    let proxy = dead_base();
    let output = sandbox.run_non_tty_through_dead_proxy(&proxy);
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert_eq!(output.status.code(), Some(125), "{stderr}");
    assert!(
        stderr.contains(OFFICIAL_BASE),
        "an enabled mirror with blank uv_binary did not fall back to GitHub: {stderr}"
    );
    assert!(!stderr.contains("https://x.invalid/simple"), "PyPI mirror leaked into uv URL: {stderr}");
}
