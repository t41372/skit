//! Public CLI/filesystem ports from Python `tests/test_add_feedback_contracts.py` at `main@206f9ef`.

use std::{fs, path::PathBuf, process::Output};

use assert_cmd::Command;
use skit_application::EntryRepository as _;
use skit_domain::StorageMode;
use skit_language::read_uv_metadata;
use skit_store::FileStore;
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
            .current_dir(self.home.path());
        command
    }

    fn run_path(&self, path: &std::path::Path, args: &[&str]) -> Output {
        let mut command = self.command();
        command.arg("add").arg(path).args(args);
        command.output().unwrap()
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn draft(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let drafts = self.data.path().join("drafts");
        fs::create_dir_all(&drafts).unwrap();
        let path = drafts.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn flat(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn test_ref_on_kept_draft_is_refused_and_keeps_it() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-linkme.py", b"print('link me')\n");

    let output = sandbox.run_path(&draft, &["--name", "linky", "--ref", "--no-input"]);
    let shown = flat(&text(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("one of skit's own kept drafts"), "{shown}");
    assert!(shown.contains("Drop --ref"), "{shown}");
    assert!(draft.exists(), "refused add consumed the kept draft");
    assert!(sandbox.store().resolve("linky").is_err(), "refused add created an entry");
}

#[test]
fn test_ref_on_a_normal_file_still_works() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("mine.py", b"print('mine')\n");

    let output = sandbox.run_path(&source, &["--name", "mine", "--ref", "--no-input"]);

    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(sandbox.store().resolve("mine").unwrap().meta.mode, StorageMode::Reference);
    assert!(source.exists());
}

#[test]
fn test_prompt_draft_with_shebang_body_resumes_as_prompt() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft(
        "skit-new-summ.prompt.md",
        b"#!/usr/bin/env bash\nSummarize {{text}}.\n",
    );

    let output = sandbox.run_path(&draft, &["--name", "summ", "--no-input"]);

    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(sandbox.store().resolve("summ").unwrap().meta.kind.as_str(), "prompt");
    assert!(!draft.exists(), "successful prompt draft was not consumed");
}

#[test]
fn test_py_draft_with_shebang_body_still_resumes_as_shell() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft(
        "skit-new-shellish.py",
        b"#!/usr/bin/env bash\necho drafted\n",
    );

    let output = sandbox.run_path(&draft, &["--name", "shellish", "--no-input"]);

    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(sandbox.store().resolve("shellish").unwrap().meta.kind.as_str(), "shell");
    assert!(!draft.exists(), "successful script draft was not consumed");
}

#[test]
fn test_micro_versioned_shebang_lands_in_stored_pep723() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "mv.py",
        b"#!/usr/bin/env python3.12.1\nprint(1)\n",
    );

    let output = sandbox.run_path(&source, &["--name", "mv", "--no-input"]);
    let shown = flat(&text(&output));

    assert_eq!(output.status.code(), Some(0), "{shown}");
    assert!(shown.contains("requires-python >=3.12.1,<3.13"), "{shown}");
    let store = sandbox.store();
    let entry = store.resolve("mv").unwrap();
    let stored = fs::read_to_string(store.payload_path(&entry).unwrap()).unwrap();
    assert!(
        stored.contains("requires-python = \">=3.12.1,<3.13\""),
        "{stored}"
    );
}

#[test]
fn test_shebangless_unknown_uses_the_isnt_a_script_voice() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("mystery", b"just some text, no shebang\n");

    let output = sandbox.run_path(&source, &["--name", "mys", "--no-input"]);
    let shown = flat(&text(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("isn't a script or an executable"), "{shown}");
    assert!(!shown.contains("names no interpreter"), "{shown}");
}

#[test]
fn test_shebang_unknown_uses_the_names_no_interpreter_voice() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "report.tricky",
        b"#!/usr/bin/awk -f\nBEGIN{print 1}\n",
    );

    let output = sandbox.run_path(&source, &["--name", "rep", "--no-input"]);
    let shown = flat(&text(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(
        shown.contains("The #! in report.tricky names no interpreter skit knows"),
        "{shown}"
    );
    assert!(shown.contains("--kind"), "{shown}");
    assert!(!shown.contains("isn't a script or an executable"), "{shown}");
}

#[test]
fn test_dynamic_optstring_with_argv_names_extra_arguments_once() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "dyn.sh",
        b"#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho \"$@\"\n",
    );

    let output = sandbox.run_path(&source, &["--name", "dyn", "--no-input"]);
    let shown = flat(&text(&output));

    assert_eq!(output.status.code(), Some(0), "{shown}");
    assert_eq!(shown.matches("extra-arguments field").count(), 1, "{shown}");
}

#[test]
fn test_add_records_only_third_party_deps_not_sibling_modules() {
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.home.path().join("helpers.py"),
        b"def go():\n    return 1\n",
    )
    .unwrap();
    let source = sandbox.source(
        "job.py",
        b"import helpers\nimport requests\nprint(helpers.go(), requests)\n",
    );

    let output = sandbox.run_path(&source, &["--name", "job", "--no-input"]);

    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    let store = sandbox.store();
    let entry = store.resolve("job").unwrap();
    let stored = fs::read_to_string(store.payload_path(&entry).unwrap()).unwrap();
    let metadata = read_uv_metadata(&stored).expect("third-party dependency produced PEP 723");
    assert_eq!(metadata.dependencies, ["requests"]);
}
