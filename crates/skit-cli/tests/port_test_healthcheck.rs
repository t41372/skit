//! Exact public-surface ports of Python `tests/test_healthcheck.py` at `main@206f9ef`.
//!
//! Python calls the shared collector directly. Rust's composition root exposes the same typed
//! projection through `skit doctor --json`; these tests build real entries, mutate their stored
//! sources, and assert the machine contract. No assertion is weakened to "doctor failed" or a
//! source-string check.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::{Value, json};
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    empty_path: TempDir,
}

impl Sandbox {
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
            .current_dir(self.home.path());
        command
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn add_python(&self, name: &str, body: &str) {
        let source = self.source(&format!("{name}.py"), body);
        self.add_source(&source, name, &[]);
    }

    fn add_shell(&self, name: &str) {
        let source = self.source(&format!("{name}.sh"), "#!/usr/bin/env bash\necho hi\n");
        self.add_source(&source, name, &[]);
    }

    fn add_prompt(&self, name: &str, body: &str) {
        let source = self.source(&format!("{name}.prompt.md"), body);
        self.add_source(&source, name, &["--prompt"]);
    }

    fn add_source(&self, source: &Path, name: &str, extra: &[&str]) {
        let mut command = self.command();
        command.arg("add").arg(source).args(["--name", name]);
        command.args(extra).arg("--no-input");
        let output = command.output().unwrap();
        assert_success(&output);
    }

    fn run_ok(&self, args: &[&str]) -> Output {
        let output = self.command().args(args).output().unwrap();
        assert_success(&output);
        output
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn payload(&self, selector: &str) -> PathBuf {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        store.payload_path(&entry).unwrap()
    }

    fn doctor(&self, empty_path: bool) -> Value {
        let mut command = self.command();
        command.args(["doctor", "--json"]);
        if empty_path {
            command.env("PATH", self.empty_path.path());
        }
        let output = command.output().unwrap();
        // A missing uv intentionally makes doctor exit 1 while still emitting the complete JSON
        // report. The Python collector itself has no process exit code, so JSON is the oracle here.
        assert!(
            matches!(output.status.code(), Some(0 | 1)),
            "doctor failed before emitting health state: {}",
            combined(&output)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!("doctor did not emit JSON: {error}\n{}", combined(&output))
        })
    }

    fn install_runner_fixture(&self) {
        fs::create_dir_all(self.config.path()).unwrap();
        fs::write(
            self.config.path().join("config.toml"),
            concat!(
                "[prompt]\n",
                "runners_seeded = true\n",
                "\n",
                "[[prompt.runners]]\n",
                "name = \"codex\"\n",
                "argv = [\"skit-health-runner-that-does-not-exist\", \"{{prompt}}\"]\n",
                "\n",
                "[[prompt.runners]]\n",
                "name = \"bad\"\n",
                "argv = [\"no-hole-here\"]\n",
            ),
        )
        .unwrap();
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn strings(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("expected JSON array, got {value}"))
        .iter()
        .map(|item| item.as_str().unwrap().to_owned())
        .collect()
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("expected JSON object, got {value}"))
        .keys()
        .cloned()
        .collect()
}

#[test]
fn test_entry_drifted_true_for_managed_placeholder_gone_from_prompt() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("pr", "Do {{a}} {{gone}}\n");
    let path = sandbox.payload("pr");
    fs::write(&path, "Do {{a}}\n").unwrap();

    let report = sandbox.doctor(false);
    assert!(strings(&report["drift"]).contains("pr"), "{report}");
    assert!(!strings(&report["missing"]).contains("pr"), "{report}");
}

#[test]
fn test_entry_drifted_false_when_prompt_body_unreadable() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("pr", "Do {{a}} {{gone}}\n");
    let path = sandbox.payload("pr");
    fs::remove_file(&path).unwrap();

    // Rust's public health seam has no injectable read port. A missing payload exercises the same
    // `fs::read(path) -> Err => false` branch as an unreadable file, while also proving the failure
    // is owned by the target sweep rather than double-reported as drift.
    let report = sandbox.doctor(false);
    assert!(strings(&report["missing"]).contains("pr"), "{report}");
    assert!(!strings(&report["drift"]).contains("pr"), "{report}");
}

#[test]
fn test_entry_drifted_false_for_insertion_off_prompt() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("pr", "Do {{a}} {{gone}}\n");
    sandbox.run_ok(&["params", "pr", "--no-interpolate"]);
    fs::write(sandbox.payload("pr"), "Do {{a}}\n").unwrap();

    let report = sandbox.doctor(false);
    assert!(!strings(&report["drift"]).contains("pr"), "{report}");
    assert!(!strings(&report["missing"]).contains("pr"), "{report}");
}

#[test]
fn test_collect_reports_every_category_and_excludes_double_reports() {
    let sandbox = Sandbox::new();

    sandbox.add_python("gone", "print(1)\n");
    fs::remove_file(sandbox.payload("gone")).unwrap();

    sandbox.add_python("drift_py", "CITY = 'x'\nGONE = 'y'\nprint(CITY)\n");
    sandbox.run_ok(&["params", "drift_py", "--manage", "CITY", "--manage", "GONE"]);
    let drift_python = sandbox.payload("drift_py");
    let text = fs::read_to_string(&drift_python).unwrap();
    fs::write(&drift_python, text.replace("GONE = 'y'\n", "")).unwrap();

    sandbox.add_prompt("drift_pr", "Do {{a}} {{gone}}\n");
    fs::write(sandbox.payload("drift_pr"), "Do {{a}}\n").unwrap();

    sandbox.add_shell("needs_sh");
    sandbox.run_ok(&["deps", "needs_sh", "--need", "ffmpeg"]);

    sandbox.add_shell("blocked_sh");
    sandbox.run_ok(&[
        "params",
        "blocked_sh",
        "--interpreter",
        "skit-health-shell-that-does-not-exist",
    ]);

    sandbox.install_runner_fixture();
    sandbox.add_prompt("blocked_pr", "Do {{a}}\n");
    sandbox.run_ok(&["params", "blocked_pr", "--runner", "codex"]);

    let report = sandbox.doctor(true);
    assert_eq!(
        strings(&report["missing"]),
        BTreeSet::from(["gone".to_owned()])
    );
    assert_eq!(
        strings(&report["drift"]),
        BTreeSet::from(["drift_pr".to_owned(), "drift_py".to_owned()])
    );
    assert_eq!(report["needs_missing"]["needs_sh"], json!(["ffmpeg"]));
    assert_eq!(
        keys(&report["launch_blocked"]),
        BTreeSet::from(["blocked_pr".to_owned(), "blocked_sh".to_owned()])
    );
    assert!(
        report["launch_blocked"]["blocked_sh"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(report["launch_blocked"].get("gone").is_none());
    assert!(report["launch_blocked"].get("needs_sh").is_none());
    assert_eq!(report["runner_rows_invalid"], json!(["bad"]));
}

#[test]
fn test_collect_double_report_exclusion_continues_not_breaks() {
    let sandbox = Sandbox::new();
    sandbox.add_python("aaa_excluded", "print(1)\n");
    fs::remove_file(sandbox.payload("aaa_excluded")).unwrap();
    sandbox.add_shell("zzz_blocked");
    sandbox.run_ok(&[
        "params",
        "zzz_blocked",
        "--interpreter",
        "skit-health-late-runtime-that-does-not-exist",
    ]);

    let report = sandbox.doctor(true);
    assert!(
        strings(&report["missing"]).contains("aaa_excluded"),
        "{report}"
    );
    assert!(
        report["launch_blocked"]
            .as_object()
            .is_some_and(|blocked| blocked.contains_key("zzz_blocked")),
        "the health sweep stopped after the early excluded entry: {report}"
    );
}

#[test]
fn test_collect_clean_library_reports_nothing() {
    let sandbox = Sandbox::new();
    // A prompt without a pinned runner requires no external program and has no uv dependency, so
    // this fixture is deterministic even on a machine with an empty PATH.
    sandbox.add_prompt("ok", "Just a healthy prompt.\n");

    let report = sandbox.doctor(true);
    assert_eq!(report["missing"], json!([]));
    assert_eq!(report["drift"], json!([]));
    assert_eq!(report["needs_missing"], json!({}));
    assert_eq!(report["launch_blocked"], json!({}));
    assert_eq!(report["runner_rows_invalid"], json!([]));
}
