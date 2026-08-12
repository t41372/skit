//! Missing-uv bootstrap failure ports from Python `tests/test_launcher.py` at `main@206f9ef`.
//!
//! These use a loopback TLS endpoint that accepts the connection and then closes it. No external
//! network is involved, but the real uv downloader must still attempt the configured mirror. With
//! PATH emptied and no managed private uv present, reaching that endpoint proves lookup exhausted
//! both locations; the resulting typed download failure must surface through `skit run` as the
//! Python launch error contract requires.

use std::{
    collections::BTreeMap,
    fs,
    io::Read as _,
    net::TcpListener,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use skit_store::{FileConfigStore, managed_uv_path};
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    empty_path: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            empty_path: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
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
            .env("PATH", self.empty_path.path())
            .current_dir(self.home.path());
        command
    }

    fn add_python(&self) {
        let source = self.home.path().join("missing-uv.py");
        fs::write(&source, "print('needs uv')\n").unwrap();
        let output = self
            .command()
            .arg("add")
            .arg(&source)
            .args(["--name", "missing-uv", "--no-input"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture add failed: {}",
            combined(&output)
        );
    }

    fn configure_mirror(&self, base: &str) -> String {
        let config = FileConfigStore::new(self.config.path());
        config
            .set_many(&BTreeMap::from([
                ("mirror.github".to_owned(), base.to_owned()),
                ("mirror".to_owned(), "on".to_owned()),
            ]))
            .unwrap();
        let mirror = config.mirror().unwrap();
        assert!(mirror.enabled);
        assert!(
            mirror.uv_binary.starts_with(base),
            "custom GitHub mirror did not feed the uv-binary axis: {mirror:?}"
        );
        mirror.uv_binary
    }

    fn run(&self) -> Output {
        self.command()
            .args(["run", "missing-uv", "--no-input"])
            .output()
            .unwrap()
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Hold a local port open, accept one TLS ClientHello, then close without speaking TLS/HTTP.
fn broken_https_endpoint() -> (String, thread::JoinHandle<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                    let mut bytes = [0_u8; 256];
                    let read = stream.read(&mut bytes).unwrap_or(0);
                    return read > 0;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("loopback mirror accept failed: {error}"),
            }
        }
        false
    });
    (format!("https://{address}"), handle)
}

fn run_against_broken_mirror() -> (Fixture, Output, bool, String) {
    let fixture = Fixture::new();
    fixture.add_python();
    assert!(
        !managed_uv_path(fixture.data.path()).exists(),
        "fixture accidentally starts with a private uv"
    );
    let (base, server) = broken_https_endpoint();
    let uv_mirror = fixture.configure_mirror(&base);
    let output = fixture.run();
    let contacted = server.join().unwrap();
    (fixture, output, contacted, uv_mirror)
}

#[test]
fn test_find_uv_returns_none_when_absent() {
    let (fixture, output, contacted, uv_mirror) = run_against_broken_mirror();

    assert!(
        contacted,
        "skit never reached bootstrap, so PATH/private-uv absence was not actually exercised: {}",
        combined(&output)
    );
    assert!(
        !managed_uv_path(fixture.data.path()).exists(),
        "a failed lookup/bootstrap fabricated a private uv"
    );
    assert!(
        combined(&output).contains(&uv_mirror),
        "the configured private-uv fallback path was not the one attempted: {}",
        combined(&output)
    );
}

#[test]
fn test_python_uv_download_failure_raises() {
    let (_fixture, output, contacted, uv_mirror) = run_against_broken_mirror();
    let text = combined(&output);

    assert!(contacted, "the deterministic failing downloader was never called: {text}");
    assert_eq!(
        output.status.code(),
        Some(125),
        "Python v0.4 wraps a uv bootstrap failure as a launch/skit failure: {text}"
    );
    assert!(text.contains("could not download uv from"), "{text}");
    assert!(text.contains(&uv_mirror), "{text}");
}
