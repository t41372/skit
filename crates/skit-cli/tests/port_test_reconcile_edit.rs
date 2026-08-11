//! Executable higher-layer ports of the Python v0.4 `tests/test_reconcile.py` contracts that do
//! not belong in `skit-language` itself.
//!
//! These are deliberately end-to-end CLI/storage assertions.  The Python suite is the oracle:
//! a red test here is a parity finding, never a reason to weaken the assertion or patch product
//! code in this branch.

use std::{collections::BTreeSet, fs, path::PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use skit_application::LibraryService;
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
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("PSModulePath")
            .current_dir(self.home.path());
        command
    }

    fn source(&self, name: &str, text: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, text).unwrap();
        path
    }

    fn add_python(&self, name: &str, source: &str) -> PathBuf {
        let path = self.source(&format!("{name}.py"), source);
        self.command()
            .args([
                "add",
                path.to_str().unwrap(),
                "--name",
                name,
                "--no-input",
            ])
            .assert()
            .success();
        self.stored(name)
    }

    fn stored(&self, slug: &str) -> PathBuf {
        self.data.path().join("scripts").join(slug).join("script.py")
    }

    fn commit_source_edit(&self, slug: &str, edit: impl FnOnce(String) -> String) {
        let service = LibraryService::new(FileStore::new(self.data.path()));
        let entry = service.show(slug).unwrap();
        let held = service.claim_identity(&entry).unwrap();
        let current = fs::read_to_string(self.stored(slug)).unwrap();
        let updated = edit(current);
        service
            .commit_copy_edit(&held, updated.as_bytes(), &held.meta.source_hash)
            .unwrap();
    }

    fn output(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> std::process::Output {
        let output = self.output(args);
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.ok(args);
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn parameter_names(document: &Value) -> Vec<String> {
    document["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap().to_owned())
        .collect()
}

fn parameter<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("missing parameter {name} in {document}"))
}

fn manage(sandbox: &Sandbox, slug: &str, names: &[&str]) {
    let mut command = sandbox.command();
    command.args(["params", slug]);
    for name in names {
        command.args(["--manage", name]);
    }
    command.assert().success();
}

fn duplicate_managed_source(include_x_body: bool) -> String {
    let x_body = if include_x_body { "X = 1\nX = 2\n" } else { "" };
    format!(
        concat!(
            "# /// script\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"X\"\n",
            "# kind = \"const\"\n",
            "# type = \"int\"\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"X\"\n",
            "# kind = \"const\"\n",
            "# type = \"int\"\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"Y\"\n",
            "# kind = \"const\"\n",
            "# type = \"int\"\n",
            "# ///\n",
            "{}",
            "Y = 5\n",
            "print(Y)\n",
        ),
        x_body
    )
}

#[test]
fn test_drift_lines_mention_rebind() {
    let sandbox = Sandbox::new();
    sandbox.add_python(
        "myscript",
        "value = input(\"Old label: \")\nprint(value)\n",
    );
    manage(&sandbox, "myscript", &["input-1"]);
    sandbox.ok(&[
        "params",
        "myscript",
        "--prompt",
        "input-1=Old label: ",
    ]);
    let before = sandbox.json(&["params", "myscript", "--json"]);
    assert_eq!(parameter(&before, "input-1")["prompt"], "Old label: ");
    sandbox.commit_source_edit("myscript", |text| text.replace("Old label: ", "New label: "));

    let output = sandbox.ok(&["show", "myscript"]);
    let human = String::from_utf8(output.stdout).unwrap();
    let expected = "  input-1: its prompt no longer matches a unique input/read call; falling back to position (still injected — double-check this lands on the right question, especially if it's a secret)";
    assert!(human.lines().any(|line| line == expected), "{human}");
}

#[test]
fn test_resync_reanchors_rebound_input_order_and_prompt() {
    let sandbox = Sandbox::new();
    sandbox.add_python(
        "myscript",
        "value = input(\"Old label: \")\nprint(value)\n",
    );
    manage(&sandbox, "myscript", &["input-1"]);
    sandbox.ok(&[
        "params",
        "myscript",
        "--prompt",
        "input-1=Old label: ",
    ]);
    let before = sandbox.json(&["params", "myscript", "--json"]);
    assert_eq!(parameter(&before, "input-1")["prompt"], "Old label: ");
    sandbox.commit_source_edit("myscript", |text| text.replace("Old label: ", "New label: "));

    let output = sandbox.ok(&["params", "myscript", "--resync"]);
    let stderr = String::from_utf8(output.stderr).unwrap();

    let document = sandbox.json(&["params", "myscript", "--json"]);
    let row = parameter(&document, "input-1");
    assert_eq!(
        row["prompt"], "New label: ",
        "resync did not re-anchor the stored prompt: {row}"
    );
    assert_eq!(
        row["order"], 0,
        "resync did not re-anchor the stored input order: {row}"
    );

    let show = sandbox.ok(&["show", "myscript"]);
    let human = String::from_utf8(show.stdout).unwrap();
    assert!(
        !human.contains("have drifted from the script"),
        "resync left the same rebound warning active:\n{human}"
    );
    assert!(
        stderr.lines().any(|line| {
            line == "input-1: re-anchored to its current position after its prompt stopped matching uniquely; double-check the prompt/secret assignment is still correct."
        }),
        "resync changed state but omitted the Python warning:\n{stderr}"
    );
}

#[test]
fn test_drift_lines_mention_old_and_new_type() {
    let sandbox = Sandbox::new();
    let stored = sandbox.add_python(
        "myscript",
        "RETRIES = 3\nGONE = \"x\"\nprint(RETRIES, GONE)\n",
    );
    manage(&sandbox, "myscript", &["RETRIES", "GONE"]);
    let edited = fs::read_to_string(&stored)
        .unwrap()
        .replace("RETRIES = 3", "RETRIES = \"3\"")
        .replace("GONE = \"x\"\n", "");
    fs::write(&stored, edited).unwrap();

    let output = sandbox.ok(&["show", "myscript"]);
    let human = String::from_utf8(output.stdout).unwrap();
    for line in [
        "The parameter definitions for myscript have drifted from the script:",
        "  GONE: injection target no longer exists (dropped from this run's form)",
        "  RETRIES: type changed from int to str in the source (still injected — double-check the value)",
        "To refresh the definitions, run: skit params myscript --resync",
    ] {
        assert!(
            human.lines().any(|actual| actual == line),
            "missing {line:?} in:\n{human}"
        );
    }
}

fn assert_not_managed_warning(flag: &[&str], expected: &str) {
    let sandbox = Sandbox::new();
    sandbox.add_python(
        "managed",
        "CITY = \"Taipei\"\nGONE = \"unused\"\nprint(CITY, GONE)\n",
    );
    manage(&sandbox, "managed", &["CITY"]);

    let mut args = vec!["params", "managed"];
    args.extend_from_slice(flag);
    let output = sandbox.output(&args);
    assert!(
        output.status.success(),
        "Python edit_specs treats an unmatched tweak as a warning, not a failed command\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.lines().any(|line| line == expected), "{stderr}");
    let document = sandbox.json(&["params", "managed", "--json"]);
    assert_eq!(parameter_names(&document), ["CITY"]);
}

#[test]
fn test_edit_specs_not_managed_in_secret_warning() {
    assert_not_managed_warning(
        &["--secret", "GONE"],
        "GONE isn't a managed parameter; skipped.",
    );
}

#[test]
fn test_edit_specs_not_managed_in_no_secret_warning() {
    assert_not_managed_warning(
        &["--no-secret", "GONE"],
        "GONE isn't a managed parameter; skipped.",
    );
}

#[test]
fn test_edit_specs_not_managed_in_prompts_warning() {
    assert_not_managed_warning(
        &["--prompt", "GONE=Enter city:"],
        "GONE isn't a managed parameter; skipped.",
    );
}

#[test]
fn test_resync_on_unparseable_script_leaves_definitions_untouched() {
    let sandbox = Sandbox::new();
    sandbox.add_python(
        "managed",
        concat!(
            "API_KEY = \"x\"\n",
            "RETRIES = 3\n",
            "who = input(\"Who? \" )\n",
            "print(API_KEY, RETRIES, who)\n",
        ),
    );
    manage(&sandbox, "managed", &["API_KEY", "RETRIES", "input-1"]);
    sandbox
        .command()
        .args(["params", "managed", "--secret", "API_KEY"])
        .assert()
        .success();
    let before = sandbox.json(&["params", "managed", "--json"]);
    assert_eq!(parameter_names(&before), ["API_KEY", "RETRIES", "input-1"]);
    assert_eq!(parameter(&before, "API_KEY")["secret"], true);

    sandbox.commit_source_edit("managed", |text| {
        text.replace("RETRIES = 3", "RETRIES = (3")
    });

    let output = sandbox.ok(&["params", "managed", "--resync"]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    let after = sandbox.json(&["params", "managed", "--json"]);
    assert_eq!(
        parameter_names(&after),
        ["API_KEY", "RETRIES", "input-1"],
        "syntax-error resync mutated the managed definitions: {after}"
    );
    assert_eq!(parameter(&after, "API_KEY")["secret"], true);
    let warning = "Could not parse the script (syntax error); resync skipped. Parameter definitions are unchanged.";
    assert!(
        stderr.lines().any(|line| line == warning),
        "syntax-error resync kept state but omitted the Python warning:\n{stderr}"
    );
}

#[test]
fn test_resync_syntax_error_does_not_also_apply_other_edits_incorrectly() {
    let sandbox = Sandbox::new();
    sandbox.add_python("managed", "CITY = \"Taipei\"\nY = 5\nprint(CITY, Y)\n");
    manage(&sandbox, "managed", &["CITY", "Y"]);
    sandbox.commit_source_edit("managed", |text| {
        text.replace("CITY = \"Taipei\"", "def broken(:")
    });

    let output = sandbox.ok(&["params", "managed", "--resync", "--unmanage", "Y"]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    let document = sandbox.json(&["params", "managed", "--json"]);
    assert_eq!(
        parameter_names(&document),
        ["CITY"],
        "syntax-error resync must skip only resync while still applying --unmanage: {document}"
    );
    let warning = "Could not parse the script (syntax error); resync skipped. Parameter definitions are unchanged.";
    assert!(
        stderr.lines().any(|line| line == warning),
        "syntax-error resync applied the explicit edit but omitted the Python warning:\n{stderr}"
    );
}

#[test]
fn test_render_warning_resync_skipped() {
    let sandbox = Sandbox::new();
    sandbox.add_python("managed", "CITY = \"Taipei\"\nprint(CITY)\n");
    manage(&sandbox, "managed", &["CITY"]);
    sandbox.commit_source_edit("managed", |text| {
        text.replace("CITY = \"Taipei\"", "def broken(:")
    });

    let output = sandbox.ok(&["params", "managed", "--resync"]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.lines().any(|line| {
            line == "Could not parse the script (syntax error); resync skipped. Parameter definitions are unchanged."
        }),
        "{stderr}"
    );
}

#[test]
fn test_edit_specs_remove_with_duplicate_names_does_not_crash() {
    let sandbox = Sandbox::new();
    sandbox.add_python("duplicate", &duplicate_managed_source(true));

    sandbox.ok(&["params", "duplicate", "--unmanage", "X"]);
    let document = sandbox.json(&["params", "duplicate", "--json"]);
    assert_eq!(parameter_names(&document), ["Y"]);
    let source = fs::read_to_string(sandbox.stored("duplicate")).unwrap();
    assert!(!source.contains("name = \"X\""), "{source}");
    assert_eq!(source.matches("name = \"Y\"").count(), 1);
}

#[test]
fn test_edit_specs_resync_drop_with_duplicate_names_does_not_crash() {
    let sandbox = Sandbox::new();
    sandbox.add_python("duplicate", &duplicate_managed_source(false));

    let output = sandbox.ok(&["params", "duplicate", "--resync"]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    let document = sandbox.json(&["params", "duplicate", "--json"]);
    assert_eq!(
        parameter_names(&document),
        ["Y"],
        "resync must deduplicate and drop the missing duplicate-named rows: {document}"
    );
    let warning = "Dropped X: it no longer exists in the script.";
    assert_eq!(
        stderr.lines().filter(|line| *line == warning).count(),
        1,
        "the duplicate source rows must collapse to one resync warning:\n{stderr}"
    );
}

#[test]
fn test_edit_specs_dedups_duplicate_names_even_when_untouched() {
    let sandbox = Sandbox::new();
    sandbox.add_python("duplicate", &duplicate_managed_source(true));

    sandbox.ok(&["params", "duplicate", "--secret", "Y"]);
    let document = sandbox.json(&["params", "duplicate", "--json"]);
    assert_eq!(parameter_names(&document), ["X", "Y"]);
    assert_eq!(parameter(&document, "Y")["secret"], true);
    let names = parameter_names(&document).into_iter().collect::<BTreeSet<_>>();
    assert_eq!(names, BTreeSet::from(["X".to_owned(), "Y".to_owned()]));
}

#[test]
fn test_no_secret_also_clears_the_env_source() {
    let sandbox = Sandbox::new();
    let stored = sandbox.add_python("managed", "API = \"x\"\nprint(API)\n");
    manage(&sandbox, "managed", &["API"]);
    sandbox.ok(&[
        "params",
        "managed",
        "--secret",
        "API",
        "--env-source",
        "API=MY_KEY",
    ]);
    let before = sandbox.json(&["params", "managed", "--json"]);
    assert_eq!(parameter(&before, "API")["secret"], true);
    assert_eq!(parameter(&before, "API")["env_source"], "MY_KEY");

    sandbox.ok(&["params", "managed", "--no-secret", "API"]);
    let after = sandbox.json(&["params", "managed", "--json"]);
    assert!(
        !parameter(&after, "API")["secret"].as_bool().unwrap_or(false),
        "a public row must not remain secret: {}",
        parameter(&after, "API")
    );
    assert_eq!(parameter(&after, "API")["env_source"], "");
    let text = fs::read_to_string(&stored).unwrap();
    assert!(!text.contains("env_source ="), "{text}");
}
