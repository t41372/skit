//! Exact behavior ports of Python `tests/test_draft_inference_and_reader_cli.py` at
//! `main@206f9ef`. Draft ownership is tested through real filesystem side effects, reader contracts
//! through parser-backed public plans or real CLI output, and every add case reads the authoritative
//! FileStore/source after the transaction. Product divergences are expected to stay red.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _,
    SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_form::{OnboardingParseState, onboarding_plan};
use skit_language::python_version_pin;
use skit_store::FileStore;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
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

    fn run(&self, args: &[&str], input: Option<&[u8]>) -> Output {
        let mut command = self.command();
        command.args(args);
        match input {
            None => command.output().unwrap(),
            Some(bytes) => {
                command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                let mut child = command.spawn().unwrap();
                child.stdin.take().unwrap().write_all(bytes).unwrap();
                child.wait_with_output().unwrap()
            }
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn draft(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let directory = self.data.path().join("drafts");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn stored_text(&self, selector: &str) -> String {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        fs::read_to_string(store.payload_path(&entry).unwrap()).unwrap()
    }

    fn kind(&self, selector: &str) -> String {
        self.store()
            .resolve(selector)
            .unwrap()
            .meta
            .kind
            .as_str()
            .to_owned()
    }

    fn seed_unmanaged(&self, name: &str, kind: &str, bytes: &[u8], reference: bool) {
        let source = self.source(&format!("{name}.src"), bytes);
        let kind_value = EntryKind::parse(kind).unwrap();
        let stored_name = match kind {
            "python" => "script.py",
            "shell" => "script.sh",
            other => panic!("fixture has no stored filename for {other}"),
        };
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: kind_value,
                mode: if reference {
                    StorageMode::Reference
                } else {
                    StorageMode::Copy
                },
                source: source.display().to_string(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: bytes.to_vec(),
                    stored_name: Some(stored_name.to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn flat(output: &Output) -> String {
    combined(output)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "{}", combined(output));
}

fn reader_fields(kind: &str, source: &str) -> usize {
    onboarding_plan(kind, source)
        .modeled_cli_fields()
        .map_or(0, |fields| fields.len())
}

#[test]
fn test_python_version_pin_rows() {
    let rows = [
        (Some("python"), ""),
        (Some("python3"), ""),
        (Some("python3.12"), ">=3.12,<3.13"),
        (Some("python3.12.1"), ">=3.12.1,<3.13"),
        (Some("python2"), ""),
        (Some("python2.7"), ""),
        (Some("bash"), ""),
        (Some(""), ""),
        (None, ""),
    ];
    for (program, expected) in rows {
        let actual = program.and_then(python_version_pin).unwrap_or_default();
        assert_eq!(actual, expected, "program={program:?}");
    }
}

#[test]
fn test_kind_for_draft_shebang_first() {
    let fixture = Fixture::new();
    let bash = fixture.draft("skit-new-a.py", b"#!/usr/bin/env bash\necho hi\n");
    let awk = fixture.draft("skit-new-b.py", b"#!/usr/bin/awk -f\nBEGIN{print 1}\n");
    let python = fixture.draft("skit-new-c.py", b"print('x')\n");

    let shell = fixture.run(
        &[
            "add",
            bash.to_str().unwrap(),
            "-n",
            "unit-shell",
            "--no-input",
        ],
        None,
    );
    assert_success(&shell);
    assert_eq!(fixture.kind("unit-shell"), "shell");
    assert!(!bash.exists());

    let unknown = fixture.run(
        &["add", awk.to_str().unwrap(), "-n", "unit-awk", "--no-input"],
        None,
    );
    assert_eq!(unknown.status.code(), Some(2), "{}", combined(&unknown));
    assert!(flat(&unknown).contains("names no interpreter skit knows"));
    assert!(awk.exists());
    assert!(fixture.store().resolve("unit-awk").is_err());

    let py = fixture.run(
        &[
            "add",
            python.to_str().unwrap(),
            "-n",
            "unit-python",
            "--no-input",
        ],
        None,
    );
    assert_success(&py);
    assert_eq!(fixture.kind("unit-python"), "python");
    assert!(!python.exists());
}

#[test]
fn test_is_draft_needs_both_dir_and_prefix() {
    let fixture = Fixture::new();
    let owned = fixture.draft("skit-new-owned.py", b"print('owned')\n");
    let parked = fixture.draft("mytool.py", b"print('parked')\n");
    let outside = fixture.source("skit-new-outside.py", b"print('outside')\n");

    for (path, name) in [
        (&owned, "owned"),
        (&parked, "parked-unit"),
        (&outside, "outside-unit"),
    ] {
        let output = fixture.run(
            &["add", path.to_str().unwrap(), "-n", name, "--no-input"],
            None,
        );
        assert_success(&output);
    }

    assert!(!owned.exists(), "owned skit- draft was not consumed");
    assert!(
        parked.exists(),
        "unowned file in drafts directory was consumed"
    );
    assert!(
        outside.exists(),
        "skit- prefix outside drafts directory was consumed"
    );
}

#[test]
fn test_reader_fields_predicate_rows() {
    let docopt = "\"\"\"Usage: x --city=<c>\"\"\"\nimport docopt\nprint(docopt.docopt(__doc__))\n";
    let modeled =
        "import argparse\np=argparse.ArgumentParser()\np.add_argument('--n')\np.parse_args()\n";
    let getopts2 = "#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n";
    let dynamic = "#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\n";

    assert_eq!(reader_fields("python", docopt), 0);
    assert_eq!(reader_fields("python", modeled), 1);
    assert_eq!(reader_fields("shell", getopts2), 2);
    assert_eq!(reader_fields("shell", dynamic), 0);
    assert_eq!(reader_fields("ruby", modeled), 0);
    assert_eq!(reader_fields("python", ""), 0);
}

#[test]
fn test_cli_add_bash_shebang_draft_lands_as_shell_and_unlinks() {
    let fixture = Fixture::new();
    let draft = fixture.draft("skit-new-ship.py", b"#!/usr/bin/env bash\necho drafted\n");
    let output = fixture.run(
        &["add", draft.to_str().unwrap(), "-n", "ship", "--no-input"],
        None,
    );
    assert_success(&output);
    assert_eq!(fixture.kind("ship"), "shell");
    assert!(!draft.exists());
}

#[test]
fn test_cli_add_awk_shebang_draft_is_unknown_kept_with_kind_escape() {
    let fixture = Fixture::new();
    let draft = fixture.draft("skit-new-awk.py", b"#!/usr/bin/awk -f\nBEGIN{print 1}\n");
    let output = fixture.run(
        &["add", draft.to_str().unwrap(), "-n", "awky", "--no-input"],
        None,
    );
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    let text = flat(&output);
    assert!(text.contains("--kind"), "{text}");
    assert!(
        text.contains("The #! in skit-new-awk.py names no interpreter skit knows"),
        "{text}"
    );
    assert!(!text.contains("isn't a script or an executable"), "{text}");
    assert!(draft.exists());
    assert!(fixture.store().resolve("awky").is_err());
}

#[test]
fn test_cli_add_no_shebang_draft_falls_back_to_python() {
    let fixture = Fixture::new();
    let draft = fixture.draft("skit-new-plain.py", b"print('resume me')\n");
    let output = fixture.run(
        &["add", draft.to_str().unwrap(), "-n", "plain", "--no-input"],
        None,
    );
    assert_success(&output);
    assert_eq!(fixture.kind("plain"), "python");
    assert!(!draft.exists());
}

#[test]
fn test_cli_add_bash_shebang_py_outside_drafts_stays_python() {
    let fixture = Fixture::new();
    let source = fixture.source("thing.py", b"#!/usr/bin/env bash\necho hi\n");
    let output = fixture.run(
        &["add", source.to_str().unwrap(), "-n", "thing", "--no-input"],
        None,
    );
    assert_success(&output);
    assert_eq!(fixture.kind("thing"), "python");
    assert!(source.exists());
}

#[test]
fn test_cli_add_parked_user_file_in_drafts_dir_is_not_unlinked() {
    let fixture = Fixture::new();
    let parked = fixture.draft("mytool.sh", b"#!/usr/bin/env bash\necho hi\n");
    let output = fixture.run(
        &[
            "add",
            parked.to_str().unwrap(),
            "-n",
            "parked",
            "--no-input",
        ],
        None,
    );
    assert_success(&output);
    assert_eq!(fixture.kind("parked"), "shell");
    assert!(parked.exists());
}

#[test]
fn test_stdin_python2_shebang_is_refused() {
    let fixture = Fixture::new();
    let output = fixture.run(
        &["add", "-", "-n", "p2"],
        Some(b"#!/usr/bin/env python2\nprint(1)\n"),
    );
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&output).contains("names no interpreter"));
    assert!(fixture.store().resolve("p2").is_err());
}

#[test]
fn test_path_add_python2_extensionless_is_refused() {
    let fixture = Fixture::new();
    let source = fixture.source("legacy", b"#!/usr/bin/env python2\nprint(1)\n");
    let output = fixture.run(
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "legacy",
            "--no-input",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&output).contains("--kind"));
    assert!(fixture.store().resolve("legacy").is_err());
}

#[test]
fn test_stdin_versioned_shebang_pins_requires_python_and_announces() {
    let fixture = Fixture::new();
    let output = fixture.run(
        &["add", "-", "-n", "v"],
        Some(b"#!/usr/bin/env python3.12\nprint(1)\n"),
    );
    assert_success(&output);
    assert!(
        flat(&output).contains("recording requires-python >=3.12,<3.13"),
        "{}",
        combined(&output)
    );
    assert!(
        fixture
            .stored_text("v")
            .contains("requires-python = \">=3.12,<3.13\"")
    );
}

#[test]
fn test_explicit_python_beats_the_shebang_pin_silently() {
    let fixture = Fixture::new();
    let output = fixture.run(
        &["add", "-", "-n", "vo", "--python", ">=3.11"],
        Some(b"#!/usr/bin/env python3.12\nprint(1)\n"),
    );
    assert_success(&output);
    assert!(!combined(&output).contains("recording requires-python"));
    let text = fixture.stored_text("vo");
    assert!(text.contains("requires-python = \">=3.11\""), "{text}");
    assert!(!text.contains(">=3.12,<3.13"), "{text}");
}

#[test]
fn test_existing_pep723_block_beats_the_shebang_pin_silently() {
    let fixture = Fixture::new();
    let body =
        b"#!/usr/bin/env python3.12\n# /// script\n# requires-python = '>=3.9'\n# ///\nprint(1)\n";
    let output = fixture.run(&["add", "-", "-n", "vb"], Some(body));
    assert_success(&output);
    assert!(!combined(&output).contains("recording requires-python"));
    let text = fixture.stored_text("vb");
    assert!(text.contains(">=3.9"), "{text}");
    assert!(!text.contains(">=3.12,<3.13"), "{text}");
}

#[test]
fn test_dep_flag_present_still_pins_from_the_shebang() {
    let fixture = Fixture::new();
    let output = fixture.run(
        &["add", "-", "-n", "vd", "--dep", "rich"],
        Some(b"#!/usr/bin/env python3.12\nprint(1)\n"),
    );
    assert_success(&output);
    assert!(flat(&output).contains("recording requires-python >=3.12,<3.13"));
    let text = fixture.stored_text("vd");
    assert!(
        text.contains("requires-python = \">=3.12,<3.13\""),
        "{text}"
    );
    assert!(text.contains("rich"), "{text}");
}

#[test]
fn test_suggested_deps_noninteractive_pins_from_the_shebang() {
    let fixture = Fixture::new();
    let output = fixture.run(
        &["add", "-", "-n", "vs"],
        Some(b"#!/usr/bin/env python3.12\nimport requests\nprint(requests)\n"),
    );
    assert_success(&output);
    assert!(flat(&output).contains("recording requires-python >=3.12,<3.13"));
    let text = fixture.stored_text("vs");
    assert!(
        text.contains("requires-python = \">=3.12,<3.13\""),
        "{text}"
    );
    assert!(text.contains("requests"), "{text}");
}

#[test]
fn test_onboard_script_params_returns_empty_for_analyzerless_kind() {
    let plan = onboarding_plan("ruby", "puts 'hi'\n");
    assert_eq!(plan.parse_state, OnboardingParseState::ParserUnavailable);
    assert!(plan.candidates.is_empty());
    assert!(plan.modeled_cli_fields().is_none());
}

const DOCOPT: &[u8] = b"\"\"\"Usage: dc --city=<c>\"\"\"\nimport docopt\nCITY = \"x\"\nprint(docopt.docopt(__doc__), CITY)\n";
const DYNAMIC_SHELL: &[u8] = b"#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho $OUTDIR\n";

#[test]
fn test_docopt_python_read_view_offers_manage() {
    let fixture = Fixture::new();
    fixture.seed_unmanaged("dc", "python", DOCOPT, false);
    let plain = fixture.run(&["params", "dc"], None);
    assert_success(&plain);
    let text = flat(&plain);
    assert!(
        text.contains("Detected but not yet managed: CITY"),
        "{text}"
    );
    assert!(text.contains("--manage"), "{text}");
    let json = fixture.run(&["params", "dc", "--json"], None);
    assert_success(&json);
    let payload: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(payload["unmanaged"], serde_json::json!(["CITY"]));
}

#[test]
fn test_docopt_python_manage_prints_no_flip_note() {
    let fixture = Fixture::new();
    fixture.seed_unmanaged("dc", "python", DOCOPT, false);
    let output = fixture.run(&["params", "dc", "--manage", "CITY"], None);
    assert_success(&output);
    assert!(!combined(&output).contains("The run form now asks"));
}

#[test]
fn test_dynamic_getopts_read_view_offers_manage() {
    let fixture = Fixture::new();
    fixture.seed_unmanaged("dyn", "shell", DYNAMIC_SHELL, false);
    let plain = fixture.run(&["params", "dyn"], None);
    assert_success(&plain);
    assert!(
        combined(&plain).contains("--manage"),
        "{}",
        combined(&plain)
    );
    let json = fixture.run(&["params", "dyn", "--json"], None);
    assert_success(&json);
    let payload: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert!(
        payload["unmanaged"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item == "OUTDIR")),
        "{}",
        String::from_utf8_lossy(&json.stdout)
    );
}

#[test]
fn test_dynamic_getopts_manage_prints_no_flip_note() {
    let fixture = Fixture::new();
    fixture.seed_unmanaged("dyn", "shell", DYNAMIC_SHELL, false);
    let output = fixture.run(&["params", "dyn", "--manage", "OUTDIR"], None);
    assert_success(&output);
    assert!(!combined(&output).contains("The run form now asks"));
}

#[test]
fn test_reference_getopts_read_view_has_no_manage_advice() {
    let fixture = Fixture::new();
    fixture.seed_unmanaged(
        "refg",
        "shell",
        b"#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n",
        true,
    );
    let output = fixture.run(&["params", "refg"], None);
    assert_success(&output);
    assert!(combined(&output).contains("has no managed parameters."));
    assert!(!combined(&output).contains("--manage"));
    let show = fixture.run(&["show", "refg", "--json"], None);
    assert_success(&show);
    let payload: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(payload["param_source"], "argparse");
}

#[test]
fn test_reference_constants_read_view_names_unmanaged_with_teaching() {
    let fixture = Fixture::new();
    fixture.seed_unmanaged(
        "refc",
        "shell",
        b"#!/usr/bin/env bash\nOUTDIR=/tmp\necho $OUTDIR\n",
        true,
    );
    let output = fixture.run(&["params", "refc"], None);
    assert_success(&output);
    let text = flat(&output);
    assert!(
        text.contains("Detected but not yet managed: OUTDIR"),
        "{text}"
    );
    assert!(!text.contains("use --manage to manage them"), "{text}");
    assert!(
        text.contains("skit never writes the original file"),
        "{text}"
    );
    let json = fixture.run(&["params", "refc", "--json"], None);
    assert_success(&json);
    let payload: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(payload["unmanaged"], serde_json::json!(["OUTDIR"]));
}

#[test]
fn test_reference_reader_add_prints_the_read_notice() {
    let fixture = Fixture::new();
    let shell = fixture.source(
        "refadd.sh",
        b"#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n",
    );
    let first = fixture.run(
        &[
            "add",
            shell.to_str().unwrap(),
            "-n",
            "refadd",
            "--ref",
            "--no-input",
        ],
        None,
    );
    assert_success(&first);
    assert!(combined(&first).contains("skit read this script's own arguments"));

    let python = fixture.source(
        "refap.py",
        b"import argparse\np=argparse.ArgumentParser()\np.add_argument('--n')\np.parse_args()\n",
    );
    let second = fixture.run(
        &[
            "add",
            python.to_str().unwrap(),
            "-n",
            "refap",
            "--ref",
            "--no-input",
        ],
        None,
    );
    assert_success(&second);
    assert!(combined(&second).contains("skit read this script's own arguments"));
}

#[test]
fn test_reference_constants_add_prints_the_skip_line() {
    let fixture = Fixture::new();
    let shell = fixture.source(
        "refcadd.sh",
        b"#!/usr/bin/env bash\nOUTDIR=/tmp\necho $OUTDIR\n",
    );
    let output = fixture.run(
        &[
            "add",
            shell.to_str().unwrap(),
            "-n",
            "refcadd",
            "--ref",
            "--no-input",
        ],
        None,
    );
    assert_success(&output);
    assert!(combined(&output).contains("parameter setup was skipped"));
    assert!(!combined(&output).contains("skit read this script's own arguments"));
}

#[test]
fn test_one_field_getopts_add_says_singular() {
    let fixture = Fixture::new();
    let shell = fixture.source(
        "one.sh",
        b"#!/usr/bin/env bash\nwhile getopts \"n:\" o; do :; done\n",
    );
    let output = fixture.run(
        &["add", shell.to_str().unwrap(), "-n", "one", "--no-input"],
        None,
    );
    assert_success(&output);
    assert!(combined(&output).contains("(1 field)"));
    assert!(!combined(&output).contains("(1 fields)"));
}

#[test]
fn test_multi_field_getopts_add_says_plural() {
    let fixture = Fixture::new();
    let shell = fixture.source(
        "many.sh",
        b"#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n",
    );
    let output = fixture.run(
        &["add", shell.to_str().unwrap(), "-n", "many", "--no-input"],
        None,
    );
    assert_success(&output);
    assert!(combined(&output).contains("(2 fields)"));
}
