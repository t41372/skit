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
//!   THE RUST SURFACE HAS NO PUBLIC EQUIVALENT. The rewrite inlines this logic, privately and
//!   partially, inside `skit-cli`'s `prepare_source_management` (resync/manage/unmanage,
//!   `crates/skit-cli/src/cli.rs:~3559`) and `params` (`~3635`). There is no pure function to
//!   call and no `EditResult`/warning contract anywhere public (verified: nothing in
//!   skit-application, skit-form, or skit-language). So each pure-logic `def` becomes a compiling
//!   `#[ignore]` stub whose body records the exact Python behavior + a MUST-FIX trailhead
//!   (`kind="absent"`). Where the Rust inline code DOES already implement the behavior, the stub
//!   says so rather than over-claiming a divergence; where it diverges observably (an unknown
//!   `--manage`/malformed `--prompt` hard-errors instead of warning), the stub notes that too and
//!   the CLI half below carries the one observable divergence assertion.
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
use std::os::unix::fs::PermissionsExt;

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{managed_params, write_managed_params};
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
        let script = self.data.path().join("job.py");
        fs::write(&script, fixture_source()).unwrap();
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

// ---------- reconcile.edit_specs pure logic ----------
//
// ABSENT (kind="absent"): the pure `reconcile.edit_specs` function and its `EditResult{specs,
// warnings}` contract do not exist on the Rust public surface. The behavior is inlined privately in
// `crates/skit-cli/src/cli.rs` (`prepare_source_management` ~:3559 for resync/manage/unmanage; the
// `params` body ~:3635 for the tweaks). MUST-FIX to make these assertable: expose a pure
// reconcile-apply that returns the warning-collecting `EditResult` (Python `src/skit/analysis.py`:
// `edit_specs` :229, `_apply_resync` :288, `_apply_add` :352, `_apply_tweaks` :372). The call
// cannot compile today, so each stub keeps the Python body as a comment.

#[test]
#[ignore = "ABSENT (kind=absent): no public reconcile.edit_specs on the Rust surface; behavior inlined privately in cli.rs::prepare_source_management (~:3559). MUST-FIX: src/skit/analysis.py:288 (_apply_resync)."]
fn test_resync_drops_missing_and_keeps_matching() {
    // A resync prunes a stored spec whose target vanished (GONE), keeps a matching one (CITY), and
    // records a `resync-dropped:GONE` warning. The Rust resync DOES prune the missing name
    // (cli.rs:3580-3596) but emits no warning code — the drop is silent.
    //   specs = [spec("CITY"), spec("GONE")]
    //   res = reconcile.edit_specs(SCRIPT, specs, resync=True)
    //   assert [s.name for s in res.specs] == ["CITY"]
    //   assert "resync-dropped:GONE" in res.warnings
}

#[test]
#[ignore = "ABSENT (kind=absent): no public reconcile.edit_specs on the Rust surface. MUST-FIX: src/skit/analysis.py:316-335 (resync retype + customization preserve)."]
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
    //   assert s.prompt == "How many? "
}

#[test]
#[ignore = "ABSENT (kind=absent): no public reconcile.edit_specs on the Rust surface. MUST-FIX: src/skit/analysis.py:352-369 (_apply_add appends candidate)."]
fn test_add_brings_candidate_under_management() {
    // Adding a currently detected candidate appends it at the end with its detected type.
    //   res = reconcile.edit_specs(SCRIPT, [spec("CITY")], add=["RETRIES"])
    //   assert [s.name for s in res.specs] == ["CITY", "RETRIES"]  # newly added appended last
    //   assert res.specs[1].type == "int"
}

#[test]
#[ignore = "ABSENT (kind=absent): no public reconcile.edit_specs on the Rust surface. MUST-FIX: src/skit/analysis.py:361-366 (add an input candidate by display name)."]
fn test_add_input_candidate_by_display_name() {
    // An input candidate is addressable by its display name (input-1); the added spec binds as an
    // input at call order 0.
    //   res = reconcile.edit_specs(SCRIPT, [], add=["input-1"])
    //   assert res.specs[0].binding == "input"
    //   assert res.specs[0].order == 0
}

#[test]
#[ignore = "ABSENT (kind=absent): no public reconcile.edit_specs on the Rust surface, and the Rust CLI DIVERGES here (an unknown --manage hard-errors, cli.rs:3606-3608, instead of warning). MUST-FIX: src/skit/analysis.py:362-369 (already-managed / not-a-candidate warnings)."]
fn test_add_already_managed_and_not_candidate_warn() {
    // Adding a name already managed, or a name that is not a current candidate, is not fatal: each
    // becomes a warning and the pass continues. Rust's inline `--manage` returns CliError::Usage on
    // the first non-candidate name and aborts the whole call — a hard error, not a per-name warning.
    //   res = reconcile.edit_specs(SCRIPT, [spec("CITY")], add=["CITY", "NOPE"])
    //   assert "already-managed:CITY" in res.warnings
    //   assert "not-a-candidate:NOPE" in res.warnings
}

#[test]
#[ignore = "ABSENT (kind=absent): no public reconcile.edit_specs on the Rust surface. MUST-FIX: src/skit/analysis.py:271-276 (remove) + 380-399 (secret/prompt tweaks)."]
fn test_remove_and_secret_toggles() {
    // remove drops a managed spec; --secret and a prompt map both apply in the same pass.
    //   specs = [spec("CITY"), spec("RETRIES", type="int")]
    //   res = reconcile.edit_specs(
    //       SCRIPT, specs, remove=["CITY"], secret=["RETRIES"], prompts={"RETRIES": "N: "})
    //   assert [s.name for s in res.specs] == ["RETRIES"]
    //   assert res.specs[0].secret is True
    //   assert res.specs[0].prompt == "N: "
}

#[test]
#[ignore = "ABSENT (kind=absent): no public reconcile.edit_specs on the Rust surface. MUST-FIX: src/skit/analysis.py:385-390 (no_secret clears the mark; not-managed warning)."]
fn test_no_secret_and_missing_name_warns() {
    // --no-secret clears the secret mark on a managed spec; an unknown name becomes a
    // `not-managed:GHOST` warning rather than a failure.
    //   res = reconcile.edit_specs(SCRIPT, [spec("CITY", secret=True)], no_secret=["CITY", "GHOST"])
    //   assert res.specs[0].secret is False
    //   assert "not-managed:GHOST" in res.warnings
}

#[test]
#[ignore = "ABSENT (kind=absent): no public reconcile.edit_specs on the Rust surface. MUST-FIX: src/skit/analysis.py:254-256 (per-spec shallow copy — purity)."]
fn test_edit_specs_is_pure_no_mutation_of_input_list() {
    // edit_specs is pure: it never mutates the caller's spec objects or list.
    //   original = [spec("CITY")]
    //   reconcile.edit_specs(SCRIPT, original, remove=["CITY"])
    //   assert [s.name for s in original] == ["CITY"]  # input list must not be mutated
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
#[ignore = "FAILING CONTRACT (divergence): oracle warns and exits 0 (src/skit/cli.py:4018-4029 _parse_kv_opts + :4521-4524 malformed warning); Rust `assignment` returns CliError::Usage -> exit 2 (verified). Ties to pending task #16 (params batch fault tolerance)."]
fn test_cli_bad_prompt_is_warned_not_fatal() {
    // A malformed --prompt (no `=`) is warned, not fatal — the pass still exits 0.
    let sandbox = Sandbox::new();
    sandbox.add_job();
    sandbox
        .command()
        .args(["params", "job", "--prompt", "no-equals-sign"])
        .assert()
        .success();
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle _edit_params refuses reference mode with exit 1 (src/skit/cli.py:4499-4507); Rust prepare_source_management returns CliError::Usage -> exit 2 (verified). Sibling declared-schema refusal was restored to exit 1 (completed task #13) as precedent."]
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
