//! Mechanical port of the Python oracle module `tests/test_edit.py`
//! (`origin/main@206f9ef`): "`skit edit`: TOML-free parameter definition editing, and
//! reconcile.edit_specs pure logic." Each `#[test]` keeps its Python `def test_*` name
//! and its WHY comment, so it traces back to its origin.
//!
//! The oracle module has two halves:
//!
//! - **`reconcile.edit_specs` pure logic** (Python `src/skit/analysis.py:229-402`): a pure
//!   function that applies resync/remove/add/secret/no_secret/prompt operations to a stored
//!   `[tool.skit]` definition list and returns `EditResult{specs, warnings}`. The apply order
//!   is fixed (resync -> remove -> add -> tweaks), unmatched names become closed-set warning
//!   codes (`resync-dropped:`, `already-managed:`, `not-a-candidate:`, `not-managed:`), and the
//!   input list is never mutated.
//!
//!   These owners call the frontend-neutral source edit operation. Public-process tests below prove
//!   that the CLI persists the same result without changing reference sources or read-only views.
//!
//! - **CLI end-to-end** (Python `CliRunner`): these drive the real `skit` binary via `assert_cmd`
//!   inside a fresh three-directory sandbox (`SKIT_DATA_DIR`/`SKIT_STATE_DIR`/`SKIT_CONFIG_DIR`).
//!   Three pass as written; three are FAILING CONTRACT divergences kept intact behind `#[ignore]`
//!   because Rust classifies the refusal as `CliError::Usage` (exit 2) where the oracle uses a
//!   plain failure (exit 1) or warns and exits 0. Each exit-code divergence was verified against
//!   the built binary before this file was written.
//!
//! Concept mapping:
//! - Python `store.add_python(script, mode="copy"/"reference")` -> `skit add <path> [--ref]
//!   --name job --no-input`. Under a non-terminal (`assert_cmd`) `onboard_add_source` skips the
//!   candidate picker and copies the pre-injected `[tool.skit]` block through unchanged
//!   (`crates/skit-cli/src/cli.rs:2617-2623`), so the fixture's managed set survives the add.
//! - Python `metawriter.write_params(SCRIPT, specs)` (fixture builder) ->
//!   `write_managed_params("python", SCRIPT, &decls)`.
//! - Python `metawriter.read_params(script.py)` (`_read_back`) -> `managed_params("python", text)`
//!   over the stored `scripts/job/script.py`.
//! - Python `runner.invoke(cli.app, ["params", name, ...])` -> `skit params job ...`.
//! - Python `runner.invoke(cli.app, ["edit", name])` -> `skit edit ...`.

use std::fs;
#[cfg(unix)]
use std::io::Read as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::PathBuf;

use skit_domain::parameters::{
    NamedEdit, ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, SourceEditRequest,
    SourceEditWarning,
};
use skit_language::{edit_source_declarations, managed_params, write_managed_params};
use tempfile::TempDir;

/// The oracle's module-level SCRIPT fixture (`tests/test_edit.py:13`): two managed candidates —
/// CITY (const str) and input-1 (order 0) — plus RETRIES (const int).
const SCRIPT: &str =
    "CITY = \"Taipei\"\nRETRIES = 3\nwho = input(\"Name: \")\nprint(CITY, RETRIES, who)\n";

/// Python `spec(name, binding="const", type="str", …)` for a plain managed const. The oracle
/// `spec()` sets no default, so neither does this — the fixture block carries name/kind/type only.
fn const_decl(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

/// The oracle `entry` fixture (`tests/test_edit.py:96-104`): the SCRIPT with CITY (const str),
/// RETRIES (const int) and GONE (const str, a drift item defined but absent from SCRIPT) written
/// into its `[tool.skit]` block.
fn fixture_source() -> String {
    write_managed_params(
        "python",
        SCRIPT,
        &[
            const_decl("CITY", ParameterType::Str),
            const_decl("RETRIES", ParameterType::Int),
            const_decl("GONE", ParameterType::Str),
        ],
    )
    .expect("python supports a managed block")
}

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    #[cfg(unix)]
    fn run_pty(&self, args: &[&str]) -> (u32, String) {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 200,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(args);
        command.env("TERM", "xterm-256color");
        command.env("SKIT_LANG", "en");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = reader.read_to_end(&mut bytes);
            bytes
        });
        let status = child.wait().unwrap();
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }

    /// Build the fixture entry: write the block-carrying SCRIPT and add it as a copy named `job`.
    /// The oracle's `store.add_python(..., mode="copy")` — done here through the real add lane.
    fn add_job(&self) -> std::path::PathBuf {
        self.add_job_source(&fixture_source())
    }

    fn add_job_source(&self, source: &str) -> std::path::PathBuf {
        let script = self.data.path().join("job.py");
        fs::write(&script, source).unwrap();
        self.command()
            .args([
                "add",
                script.to_str().unwrap(),
                "--name",
                "job",
                "--no-input",
            ])
            .assert()
            .success();
        script
    }

    /// Python `_read_back(entry)` (`tests/test_edit.py:107-108`): the managed definitions read
    /// back out of the stored copy, in stored order.
    fn read_back(&self) -> Vec<ParamDecl> {
        let stored = self.data.path().join("scripts/job/script.py");
        managed_params("python", &fs::read_to_string(stored).unwrap())
    }
}

fn combine(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

// ---------- reconcile.edit_specs pure logic ----------
//
// Rust keeps the request/result types in the domain and the parser-backed operation in
// `skit-language`. This preserves the oracle's pure boundary without exposing parser types to the
// application or domain crates.

#[test]
fn test_resync_drops_missing_and_keeps_matching() {
    // A resync prunes a stored spec whose target vanished (GONE), keeps a matching one (CITY), and
    // records a `resync-dropped:GONE` warning. The Rust resync DOES prune the missing name
    // (cli.rs:3580-3596) but emits no warning code — the drop is silent.
    //   specs = [spec("CITY"), spec("GONE")]
    //   res = reconcile.edit_specs(SCRIPT, specs, resync=True)
    //   assert [s.name for s in res.specs] == ["CITY"]
    let result = edit_source_declarations(
        "python",
        SCRIPT,
        &[
            const_decl("CITY", ParameterType::Str),
            const_decl("GONE", ParameterType::Str),
        ],
        &SourceEditRequest {
            resync: true,
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        result
            .declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY"]
    );
    assert_eq!(
        result.warnings,
        [SourceEditWarning::ResyncDropped {
            name: "GONE".to_owned()
        }]
    );
}

#[test]
fn test_resync_updates_changed_type_preserving_customization() {
    // RETRIES is int in the script but was mis-annotated as str; the user added secret/prompt.
    // Resync corrects the type to int while preserving the user's secret/prompt customization. The
    // Rust resync does take the candidate type and preserves secret/env_source/prompt inline
    // (cli.rs:3586-3592), but there is no pure surface to observe it on.
    //   specs = [spec("RETRIES", type="str", secret=True, prompt="How many? ")]
    //   res = reconcile.edit_specs(SCRIPT, specs, resync=True)
    //   s = res.specs[0]
    //   assert s.type == "int"        # type corrected to match the script
    //   assert s.secret is True       # user customisation preserved
    let mut retries = const_decl("RETRIES", ParameterType::Str);
    retries.secret = true;
    retries.prompt = "How many? ".to_owned();
    let result = edit_source_declarations(
        "python",
        SCRIPT,
        &[retries],
        &SourceEditRequest {
            resync: true,
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    let declaration = &result.declarations[0];
    assert_eq!(declaration.parameter_type, ParameterType::Int);
    assert!(declaration.secret);
    assert_eq!(declaration.prompt, "How many? ");
    assert_eq!(
        declaration.default, None,
        "a secret resync must not cache the source literal"
    );
}

#[test]
fn test_add_brings_candidate_under_management() {
    // Adding a currently detected candidate appends it at the end with its detected type.
    //   res = reconcile.edit_specs(SCRIPT, [spec("CITY")], add=["RETRIES"])
    //   assert [s.name for s in res.specs] == ["CITY", "RETRIES"]  # newly added appended last
    let result = edit_source_declarations(
        "python",
        SCRIPT,
        &[const_decl("CITY", ParameterType::Str)],
        &SourceEditRequest {
            add: vec!["RETRIES".to_owned()],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        result
            .declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY", "RETRIES"]
    );
    assert_eq!(result.declarations[1].parameter_type, ParameterType::Int);
}

#[test]
fn test_add_input_candidate_by_display_name() {
    // An input candidate is addressable by its display name (input-1); the added spec binds as an
    // input at call order 0.
    //   res = reconcile.edit_specs(SCRIPT, [], add=["input-1"])
    //   assert res.specs[0].binding == "input"
    let result = edit_source_declarations(
        "python",
        SCRIPT,
        &[],
        &SourceEditRequest {
            add: vec!["input-1".to_owned()],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    assert_eq!(result.declarations[0].binding, ParameterBinding::Input);
    assert_eq!(result.declarations[0].order, 0);
}

#[test]
fn test_add_already_managed_and_not_candidate_warn() {
    // Adding a name already managed, or a name that is not a current candidate, is not fatal. The
    // valid input candidate between them still commits in the same source-CAS operation.
    let sandbox = Sandbox::new();
    sandbox.add_job();
    let payload = sandbox.data.path().join("scripts/job/script.py");
    let meta = sandbox.data.path().join("scripts/job/meta.toml");
    let meta_before = fs::read(&meta).unwrap();
    let state_path = sandbox.state.path().join("values/job.toml");
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    fs::write(
        &state_path,
        "[values]\ninput-1 = \"plaintext\"\nCITY = \"public\"\n\n[presets.saved]\ninput-1 = \"plaintext\"\nCITY = \"public\"\n",
    )
    .unwrap();
    let output = sandbox
        .command()
        .args([
            "params",
            "job",
            "--manage",
            "CITY",
            "--manage",
            "input-1",
            "--manage",
            "NOPE",
            "--secret",
            "input-1",
            "--prompt",
            "no-equals-sign",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", combine(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let malformed = stderr
        .find("Ignored a malformed value: --prompt: no-equals-sign (expected NAME=text).")
        .unwrap_or_else(|| panic!("missing malformed warning: {stderr}"));
    let already = stderr
        .find("CITY is already managed; skipped.")
        .unwrap_or_else(|| panic!("missing already-managed warning: {stderr}"));
    let unknown = stderr
        .find("NOPE isn't a detectable parameter in the current script; skipped.")
        .unwrap_or_else(|| panic!("missing not-a-candidate warning: {stderr}"));
    assert!(malformed < already && already < unknown, "{stderr}");
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Updated job. Managed parameters: CITY, RETRIES, GONE, input-1")
    );
    let managed = sandbox.read_back();
    let input = managed
        .iter()
        .find(|row| row.name == "input-1")
        .expect("the valid candidate must commit");
    assert_eq!(input.binding, ParameterBinding::Input);
    assert_eq!(input.order, 0);
    assert!(input.secret);
    let mut meta_before: toml::Value =
        toml::from_str(std::str::from_utf8(&meta_before).unwrap()).unwrap();
    let meta_after_bytes = fs::read(&meta).unwrap();
    let mut meta_after: toml::Value =
        toml::from_str(std::str::from_utf8(&meta_after_bytes).unwrap()).unwrap();
    let before_hash = meta_before.as_table_mut().unwrap().remove("source_hash");
    let after_hash = meta_after.as_table_mut().unwrap().remove("source_hash");
    assert_ne!(after_hash, before_hash);
    assert_eq!(meta_after, meta_before);
    let state = fs::read_to_string(&state_path).unwrap();
    assert!(!state.contains("input-1"), "{state}");
    assert!(!state.contains("plaintext"), "{state}");
    assert!(state.contains("CITY = \"public\""), "{state}");

    // An all-invalid batch warns and performs no source, metadata, or state write.
    let payload_before = fs::read(&payload).unwrap();
    let meta_before = fs::read(&meta).unwrap();
    let state_before = fs::read(&state_path).unwrap();
    let output = sandbox
        .command()
        .args(["params", "job", "--manage", "NOPE"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combine(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("NOPE isn't a detectable parameter in the current script; skipped.")
    );
    assert_eq!(fs::read(&payload).unwrap(), payload_before);
    assert_eq!(fs::read(&meta).unwrap(), meta_before);
    assert_eq!(fs::read(&state_path).unwrap(), state_before);

    // JSON keeps the machine document on stdout. Recoverable diagnostics stay on stderr.
    let json = Sandbox::new();
    json.add_job();
    let output = json
        .command()
        .args([
            "params", "job", "--manage", "CITY", "--manage", "input-1", "--manage", "NOPE",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combine(&output));
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["parameters"].as_array().unwrap().len(), 4);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("skipped"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CITY is already managed; skipped."),
        "{stderr}"
    );
    assert!(
        stderr.contains("NOPE isn't a detectable parameter in the current script; skipped."),
        "{stderr}"
    );

    for (locale, already, unknown) in [
        (
            "en",
            "CITY is already managed; skipped.",
            "NOPE isn't a detectable parameter in the current script; skipped.",
        ),
        (
            "zh-CN",
            "CITY 已在管理中;已跳过。",
            "NOPE 在当前脚本中检测不到;已跳过。",
        ),
        (
            "zh-TW",
            "CITY 已在管理中;已略過。",
            "NOPE 在當前腳本中偵測不到;已略過。",
        ),
    ] {
        let localized = Sandbox::new();
        localized.add_job();
        let output = localized
            .command()
            .env("SKIT_LANG", locale)
            .args(["params", "job", "--manage", "CITY", "--manage", "NOPE"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", combine(&output));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(already), "{locale}: {stderr}");
        assert!(stderr.contains(unknown), "{locale}: {stderr}");
    }
}

#[test]
fn test_remove_and_secret_toggles() {
    // remove drops a managed spec; --secret and a prompt map both apply in the same pass.
    //   specs = [spec("CITY"), spec("RETRIES", type="int")]
    //   res = reconcile.edit_specs(
    //       SCRIPT, specs, remove=["CITY"], secret=["RETRIES"], prompts={"RETRIES": "N: "})
    //   assert [s.name for s in res.specs] == ["RETRIES"]
    //   assert res.specs[0].secret is True
    let result = edit_source_declarations(
        "python",
        SCRIPT,
        &[
            const_decl("CITY", ParameterType::Str),
            const_decl("RETRIES", ParameterType::Int),
        ],
        &SourceEditRequest {
            remove: vec!["CITY".to_owned()],
            secret: vec!["RETRIES".to_owned()],
            prompts: vec![NamedEdit::new("RETRIES", "N: ")],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    assert_eq!(result.declarations.len(), 1);
    assert_eq!(result.declarations[0].name, "RETRIES");
    assert!(result.declarations[0].secret);
    assert_eq!(result.declarations[0].prompt, "N: ");
}

#[test]
fn test_no_secret_and_missing_name_warns() {
    // --no-secret clears the secret mark on a managed spec; an unknown name becomes a
    // `not-managed:GHOST` warning rather than a failure.
    //   res = reconcile.edit_specs(SCRIPT, [spec("CITY", secret=True)], no_secret=["CITY", "GHOST"])
    //   assert res.specs[0].secret is False
    let mut city = const_decl("CITY", ParameterType::Str);
    city.secret = true;
    city.env_source = "CITY_TOKEN".to_owned();
    let result = edit_source_declarations(
        "python",
        SCRIPT,
        &[city],
        &SourceEditRequest {
            no_secret: vec!["CITY".to_owned(), "GHOST".to_owned()],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    assert!(!result.declarations[0].secret);
    assert!(result.declarations[0].env_source.is_empty());
    assert_eq!(
        result.warnings,
        [SourceEditWarning::NotManaged {
            name: "GHOST".to_owned()
        }]
    );
}

#[test]
fn test_edit_specs_is_pure_no_mutation_of_input_list() {
    // edit_specs is pure: it never mutates the caller's spec objects or list.
    //   original = [spec("CITY")]
    //   reconcile.edit_specs(SCRIPT, original, remove=["CITY"])
    let original = vec![const_decl("CITY", ParameterType::Str)];
    let snapshot = original.clone();
    let result = edit_source_declarations(
        "python",
        SCRIPT,
        &original,
        &SourceEditRequest {
            remove: vec!["CITY".to_owned()],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    assert!(result.declarations.is_empty());
    assert_eq!(original, snapshot);
}

#[test]
fn syntax_error_resync_warns_and_writes_no_source_metadata_or_state() {
    let sandbox = Sandbox::new();
    let broken = write_managed_params(
        "python",
        "CITY = \"Taipei\"\nRETRIES = (3\n",
        &[
            const_decl("CITY", ParameterType::Str),
            const_decl("RETRIES", ParameterType::Int),
        ],
    )
    .unwrap();
    sandbox.add_job_source(&broken);
    let payload = sandbox.data.path().join("scripts/job/script.py");
    let meta = sandbox.data.path().join("scripts/job/meta.toml");
    let payload_before = fs::read(&payload).unwrap();
    let meta_before = fs::read(&meta).unwrap();

    let output = sandbox
        .command()
        .args(["params", "job", "--resync"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", combine(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "Could not parse the script (syntax error); resync skipped. Parameter definitions are unchanged."
        ),
        "{}",
        combine(&output)
    );
    assert_eq!(fs::read(payload).unwrap(), payload_before);
    assert_eq!(fs::read(meta).unwrap(), meta_before);
    assert!(!sandbox.state.path().join("values/job.toml").exists());
}

#[test]
fn unknown_source_tweaks_warn_keep_valid_siblings_and_keep_json_on_stdout() {
    let sandbox = Sandbox::new();
    let mut city = const_decl("CITY", ParameterType::Str);
    city.secret = true;
    city.env_source = "CITY_TOKEN".to_owned();
    let source = write_managed_params("python", SCRIPT, &[city]).unwrap();
    sandbox.add_job_source(&source);

    let output = sandbox
        .command()
        .args([
            "params",
            "job",
            "--no-secret",
            "CITY",
            "--no-secret",
            "GHOST",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", combine(&output));
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(document.is_object());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("skipped"));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("GHOST isn't a managed parameter; skipped.")
    );
    let city = sandbox
        .read_back()
        .into_iter()
        .find(|declaration| declaration.name == "CITY")
        .unwrap();
    assert!(!city.secret);
    assert!(city.env_source.is_empty());

    let payload = sandbox.data.path().join("scripts/job/script.py");
    let meta = sandbox.data.path().join("scripts/job/meta.toml");
    let payload_before = fs::read(&payload).unwrap();
    let meta_before = fs::read(&meta).unwrap();
    let output = sandbox
        .command()
        .args(["params", "job", "--no-secret", "GHOST"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combine(&output));
    assert_eq!(fs::read(payload).unwrap(), payload_before);
    assert_eq!(fs::read(meta).unwrap(), meta_before);
    assert!(!sandbox.state.path().join("values/job.toml").exists());
}

#[test]
fn source_edit_order_is_resync_then_unmanage_manage_and_tweak() {
    let result = edit_source_declarations(
        "python",
        SCRIPT,
        &[const_decl("CITY", ParameterType::Str)],
        &SourceEditRequest {
            resync: true,
            remove: vec!["CITY".to_owned()],
            add: vec!["CITY".to_owned()],
            secret: vec!["CITY".to_owned()],
            prompts: vec![NamedEdit::new("CITY", "City: ")],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    assert!(result.warnings.is_empty());
    assert_eq!(result.declarations.len(), 1);
    assert_eq!(result.declarations[0].name, "CITY");
    assert!(result.declarations[0].secret);
    assert_eq!(result.declarations[0].prompt, "City: ");
    assert!(result.declarations[0].default.is_none());
}

// ---------- CLI end-to-end ----------

#[test]
fn test_cli_resync_prunes_and_persists() {
    let sandbox = Sandbox::new();
    sandbox.add_job();
    sandbox
        .command()
        .args(["params", "job", "--resync"])
        .assert()
        .success();
    let mut names = sandbox
        .read_back()
        .into_iter()
        .map(|declaration| declaration.name)
        .collect::<Vec<_>>();
    names.sort();
    // GONE (the drift item) is pruned; CITY and RETRIES persist (set equality in the oracle).
    assert!(!names.contains(&"GONE".to_owned()), "{names:?}");
    assert_eq!(names, ["CITY", "RETRIES"]);
}

#[test]
fn test_cli_secret_and_prompt_persist() {
    let sandbox = Sandbox::new();
    sandbox.add_job();
    // `--prompt CITY=Where? ` carries a trailing space; the value after the first `=` is the prompt.
    sandbox
        .command()
        .args([
            "params",
            "job",
            "--secret",
            "CITY",
            "--prompt",
            "CITY=Where? ",
        ])
        .assert()
        .success();
    let back = sandbox.read_back();
    let city = back
        .iter()
        .find(|declaration| declaration.name == "CITY")
        .expect("CITY is still managed");
    assert!(city.secret);
    assert_eq!(city.prompt, "Where? ");
}

#[test]
fn test_cli_params_view_no_ops() {
    let sandbox = Sandbox::new();
    sandbox.add_job();
    let output = sandbox.command().args(["params", "job"]).output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("CITY"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    // The read view must not modify any definitions: all three (CITY, RETRIES, GONE) survive.
    assert_eq!(sandbox.read_back().len(), 3);
}

#[test]
fn test_cli_bad_prompt_is_warned_not_fatal() {
    // A malformed --prompt (no `=`) is warned, not fatal — the pass still exits 0.
    let sandbox = Sandbox::new();
    sandbox.add_job();
    let payload = sandbox.data.path().join("scripts/job/script.py");
    let meta = sandbox.data.path().join("scripts/job/meta.toml");
    let payload_before = fs::read(&payload).unwrap();
    let meta_before = fs::read(&meta).unwrap();
    for malformed in ["no-equals-sign", "=empty-name"] {
        let output = sandbox
            .command()
            .args(["params", "job", "--prompt", malformed])
            .output()
            .unwrap();
        let shown = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(0), "{malformed}: {shown}");
        assert!(
            shown.contains(&format!(
                "Ignored a malformed value: --prompt: {malformed} (expected NAME=text)."
            )),
            "{malformed}: {shown}"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("Updated job. Managed parameters: CITY, RETRIES, GONE"),
            "{malformed}: {shown}"
        );
    }
    #[cfg(unix)]
    {
        let (code, shown) = sandbox.run_pty(&["params", "job", "--prompt", "no-equals-sign"]);
        assert_eq!(code, 0, "{shown}");
        let warning = shown.find("Ignored a malformed value").unwrap();
        let receipt = shown
            .find("Updated job. Managed parameters: CITY, RETRIES, GONE")
            .unwrap_or_else(|| panic!("missing receipt: {shown}"));
        assert!(warning < receipt, "{shown}");
    }
    assert_eq!(fs::read(payload).unwrap(), payload_before);
    assert_eq!(fs::read(meta).unwrap(), meta_before);
    assert!(!sandbox.state.path().join("values/job.toml").exists());
}

#[test]
fn test_cli_params_edit_reference_refused() {
    let sandbox = Sandbox::new();
    let script = sandbox.data.path().join("ref.py");
    fs::write(&script, SCRIPT).unwrap();
    sandbox
        .command()
        .args([
            "add",
            script.to_str().unwrap(),
            "--name",
            "refent",
            "--ref",
            "--no-input",
        ])
        .assert()
        .success();
    sandbox
        .command()
        .args(["params", "refent", "--resync"])
        .assert()
        .code(1);
    // The original file must never be modified.
    assert_eq!(fs::read_to_string(&script).unwrap(), SCRIPT);
}

#[test]
fn test_cli_edit_command_entry_has_no_source() {
    // `skit edit` on a non-editable (command) entry must refuse before ever launching an editor.
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--cmd", "echo {x}", "--name", "ec"])
        .assert()
        .success();
    // Sentinel editor: touches a marker if launched. The Python monkeypatch's "editor must not be
    // launched" invariant, translated. VISUAL is checked before EDITOR in the Rust editor lookup,
    // so both point at the sentinel.
    let marker = sandbox.data.path().join("editor-ran");
    let editor = sandbox.data.path().join("sentinel-editor.sh");
    fs::write(
        &editor,
        format!("#!/bin/sh\ntouch \"{}\"\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["edit", "ec"])
        .assert()
        .code(1);
    assert!(!marker.exists(), "editor must not be launched");
}
