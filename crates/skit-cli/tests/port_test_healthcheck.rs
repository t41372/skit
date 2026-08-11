//! Mechanical port of the Python oracle module `tests/test_healthcheck.py`
//! (`origin/main@206f9ef`): "The one health pipeline both faces consume:
//! `healthcheck.collect` / `entry_drifted`." Each `#[test]` keeps its Python `def test_*`
//! name and its WHY comment so it traces back to its origin.
//!
//! WHY skit-cli: the oracle's `healthcheck.collect`/`entry_drifted` have no public Rust
//! surface — the Rust rewrite keeps them as PRIVATE helpers on `CliHealthInspector`
//! (`crates/skit-cli/src/cli.rs`: `collect`, `doctor_entry_drifted`, `doctor_launch_block`).
//! The only public seam that drives that exact pipeline is `skit doctor --json`, whose JSON
//! is produced by `CliHealthInspector::collect`. So the port drives the real binary through
//! the composition root, the one place the whole sweep is reachable.
//!
//! Concept mapping used throughout:
//! - Python `healthcheck.collect(store.list_entries())` -> `skit doctor --json`. Its JSON
//!   maps field-for-field onto the oracle `HealthReport`:
//!   - `report.missing` (names)          -> JSON `missing`  (array of names)
//!   - `report.drifted` (names)          -> JSON `drift`    (array of names)
//!   - `report.needs_missing` (name->tools) -> JSON `needs_missing` (object)
//!   - `report.launch_blocked` (name->reason) -> JSON `launch_blocked` (object)
//!   - `report.invalid_runner_rows`      -> JSON `runner_rows_invalid` (array)
//!   - `report.needs_entries` / `report.blocked_entries` (entry objects, asserted by name)
//!     -> the KEYS of `needs_missing` / `launch_blocked`. The JSON has no separate entries
//!     list; Rust `collect` builds one `HealthIssue` per entry, so the name-sets coincide
//!     by construction (`crates/skit-cli/src/cli.rs:6410` and `:6419`).
//! - Python `healthcheck.entry_drifted(entry)` -> membership of the entry name in `skit
//!   doctor --json`'s `drift` list, read from a single-entry library. Doctor's `drift` list
//!   is populated by `doctor_entry_drifted`, the literal Rust port of `entry_drifted`
//!   (`crates/skit-cli/src/cli.rs:5001,6389`). (`skit show --json` also carries a per-entry
//!   `drift` bool, but through LibrarySurface's `form_plan`, a DIFFERENT caller — so drift is
//!   read through doctor, not show.)
//! - Python `store.add_python`/`add_script`/`add_prompt` -> `skit add <path> [--prompt]
//!   --name <name>`.
//! - Python `store.update_needs(name, [tool])` -> `skit deps <name> --need <tool>`.
//! - Python `store.write_prompt_runner(slug, "codex")` -> `skit params <name> --runner codex`.
//! - Python `store.write_prompt_interpolate(slug, False)` -> `skit params <name>
//!   --no-interpolate`.
//! - Python `config.save_config({"prompt": {"runners": [...]}})` -> writing `config.toml`
//!   directly. The validated `skit runner` path refuses the malformed "bad" row (no
//!   `{{prompt}}` slot), so a direct write is the only route to a configured-but-invalid row.
//! - Python `metawriter.write_params("CITY = 'x'\nprint(CITY)\n", [CITY, GONE])` (the
//!   module-level `_DRIFTED` fixture) -> `write_managed_params("python", ...)`, the same
//!   public writer the metawriter port drives. The body assigns only `CITY`, so the declared
//!   `GONE` has drifted.
//! - Python `monkeypatch launch._which -> None` and `shutil.which -> None` (nothing resolves
//!   on PATH) -> an EMPTY `PATH` TempDir. `find_program` reads `PATH`
//!   (`crates/skit-runtime/src/launch.rs:1097`), so an empty PATH makes every interpreter,
//!   runner binary, and declared tool unresolvable.
//! - Python `monkeypatch Path.read_text -> OSError` (unreadable body) -> `chmod 000` on the
//!   stored `prompt.md` under `#[cfg(unix)]`. The assertion is root-proof by construction:
//!   the body is left matching its params, so `entry_drifted` is `false` whether the read
//!   error is swallowed (non-root) or the read succeeds (root, body still matches).
//!
//! Buckets:
//! - REAL asserting `#[test]` (API exists): tests 1, 2, 3, 4, 5, 6. Everything is reachable
//!   through the binary; there is no cross-crate or absent-gap bucket.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::write_managed_params;
use tempfile::TempDir;

/// One isolated skit library: the three skit directories plus a scratch source directory.
struct Lib {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    src: TempDir,
}

impl Lib {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            src: TempDir::new().unwrap(),
        }
    }

    /// A fresh `skit` command with the sandbox directories and the English locale.
    fn skit(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }

    /// Write one scratch source file and return its path.
    fn write_src(&self, filename: &str, body: &str) -> PathBuf {
        let path = self.src.path().join(filename);
        fs::write(&path, body).unwrap();
        path
    }

    /// Python `store.add_python(p, name=name)`.
    fn add_python(&self, name: &str, body: &str) {
        let path = self.write_src(&format!("{name}.py"), body);
        self.skit()
            .arg("add")
            .arg(&path)
            .args(["--name", name])
            .assert()
            .success();
    }

    /// Python `store.add_script(p, kind="shell", name=name)`.
    fn add_shell(&self, name: &str) {
        let path = self.write_src(&format!("{name}.sh"), "#!/usr/bin/env bash\necho hi\n");
        self.skit()
            .arg("add")
            .arg(&path)
            .args(["--name", name])
            .assert()
            .success();
    }

    /// Python `store.add_prompt(p, name=name)`.
    fn add_prompt(&self, name: &str, text: &str) {
        let path = self.write_src(&format!("{name}.prompt.md"), text);
        self.skit()
            .arg("add")
            .arg(&path)
            .args(["--prompt", "--name", name])
            .assert()
            .success();
    }

    /// The stable slug the store assigned to a display name (e.g. `drift_pr` -> `drift-pr`).
    fn slug_of(&self, name: &str) -> String {
        let show = self.show_json(name);
        show["slug"].as_str().expect("slug field").to_owned()
    }

    /// Parsed `skit show <name> --json`.
    fn show_json(&self, name: &str) -> Value {
        let output = self.skit().args(["show", name, "--json"]).output().unwrap();
        serde_json::from_slice(&output.stdout).expect("show --json emits valid JSON")
    }

    /// The stored prompt body path for an entry.
    fn prompt_body_path(&self, name: &str) -> PathBuf {
        self.data
            .path()
            .join("scripts")
            .join(self.slug_of(name))
            .join("prompt.md")
    }

    /// Python `entry.script_path.write_text(body)` for a prompt: overwrite the stored body.
    fn overwrite_prompt_body(&self, name: &str, body: &str) {
        fs::write(self.prompt_body_path(name), body).unwrap();
    }

    /// Python `entry.script_path.unlink()` for a python entry: delete the stored payload.
    fn remove_python_payload(&self, name: &str) {
        let path = self
            .data
            .path()
            .join("scripts")
            .join(self.slug_of(name))
            .join("script.py");
        fs::remove_file(path).unwrap();
    }

    /// Python `config.save_config(...)`: replace `config.toml` with raw content.
    fn write_config(&self, toml: &str) {
        fs::write(self.config.path().join("config.toml"), toml).unwrap();
    }

    /// Python `store.update_needs(name, tools)`.
    fn set_needs(&self, name: &str, tools: &[&str]) {
        let mut command = self.skit();
        command.args(["deps", name]);
        for tool in tools {
            command.args(["--need", tool]);
        }
        command.assert().success();
    }

    /// Python `store.write_prompt_runner(slug, runner)`.
    fn pin_runner(&self, name: &str, runner: &str) {
        self.skit()
            .args(["params", name, "--runner", runner])
            .assert()
            .success();
    }

    /// Python `store.write_prompt_interpolate(slug, False)`.
    fn set_no_interpolate(&self, name: &str) {
        self.skit()
            .args(["params", name, "--no-interpolate"])
            .assert()
            .success();
    }

    /// Parsed `skit doctor --json`, optionally with a replacement `PATH`.
    fn doctor_json(&self, path_override: Option<&Path>) -> Value {
        let mut command = self.skit();
        if let Some(path) = path_override {
            command.env("PATH", path);
        }
        let output = command.args(["doctor", "--json"]).output().unwrap();
        serde_json::from_slice(&output.stdout).expect("doctor --json emits valid JSON")
    }
}

/// The names in a JSON string array, as a set.
fn name_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("a JSON array")
        .iter()
        .map(|item| item.as_str().expect("a string element").to_owned())
        .collect()
}

/// The keys of a JSON object (the reported entry names), as a set.
fn key_set(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("a JSON object")
        .keys()
        .cloned()
        .collect()
}

fn owned(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// A python source whose managed block declares CITY and GONE while the body assigns only
/// CITY — the oracle's `_DRIFTED` fixture (`tests/test_healthcheck.py:46-52`).
fn drifted_python_source() -> String {
    let mut city = ParamDecl::new("CITY");
    city.binding = ParameterBinding::Const;
    city.delivery = ParameterDelivery::Inject;
    city.parameter_type = ParameterType::Str;
    let mut gone = ParamDecl::new("GONE");
    gone.binding = ParameterBinding::Const;
    gone.delivery = ParameterDelivery::Inject;
    gone.parameter_type = ParameterType::Str;
    write_managed_params("python", "CITY = 'x'\nprint(CITY)\n", &[city, gone]).unwrap()
}

/// Write an executable stand-in for `uv` into a directory (so a python entry's interpreter
/// resolves), matching `find_program`'s `mode & 0o111` check.
fn make_fake_uv(dir: &Path) {
    let uv = dir.join("uv");
    fs::write(&uv, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&uv, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

// ---------------------------------------------------------------- entry_drifted

#[test]
fn test_entry_drifted_true_for_managed_placeholder_gone_from_prompt() {
    let lib = Lib::new();
    lib.add_prompt("pr", "Do {{a}} {{gone}}\n");
    // Precondition: the store recorded both managed placeholders (oracle `entry.meta.params
    // == ["a", "gone"]`). The `show --json` `fields` carry them in stored order.
    let keys: Vec<String> = lib.show_json("pr")["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["key"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(keys, ["a", "gone"]);
    // `gone` left the body: the managed placeholder no longer appears, so the entry drifted.
    lib.overwrite_prompt_body("pr", "Do {{a}}\n");
    let report = lib.doctor_json(None);
    assert!(name_set(&report["drift"]).contains("pr"));
}

#[test]
fn test_entry_drifted_false_when_prompt_body_unreadable() {
    // An unreadable body belongs to the target/preflight sweeps, not drift — the read guard
    // must swallow the error and report no drift. The body keeps both placeholders, so the
    // assertion holds even where `chmod 000` is not effective (root): a successful read still
    // matches the stored params, so `entry_drifted` is false either way.
    let lib = Lib::new();
    lib.add_prompt("pr", "Do {{a}} {{gone}}\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(
            lib.prompt_body_path("pr"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
    }
    let report = lib.doctor_json(None);
    assert!(!name_set(&report["drift"]).contains("pr"));
}

#[test]
fn test_entry_drifted_false_for_insertion_off_prompt() {
    // An insertion-off prompt cannot drift — nothing is filled at run time.
    let lib = Lib::new();
    lib.add_prompt("pr", "Do {{a}} {{gone}}\n");
    lib.set_no_interpolate("pr");
    lib.overwrite_prompt_body("pr", "Do {{a}}\n");
    let report = lib.doctor_json(None);
    assert!(!name_set(&report["drift"]).contains("pr"));
}

// ---------------------------------------------------------------- collect

#[test]
fn test_collect_reports_every_category_and_excludes_double_reports() {
    let lib = Lib::new();
    // (a) target-missing
    lib.add_python("gone", "print(1)\n");
    // (b) drift: a python-managed entry AND a prompt whose managed placeholder left the body
    lib.add_python("drift_py", &drifted_python_source());
    lib.add_prompt("drift_pr", "Do {{a}} {{gone}}\n");
    // (c) needs_missing: a shell entry whose declared tool is off PATH
    lib.add_shell("needs_sh");
    // (d) launch_blocked: a shell entry whose interpreter binary is absent, and a prompt
    //     whose pinned runner binary is gone
    lib.add_shell("blocked_sh");
    lib.add_prompt("blocked_pr", "Do {{a}}\n");
    lib.set_needs("needs_sh", &["ffmpeg"]);
    // A valid codex row (so the pin resolves) plus one malformed row -> (e) invalid rows.
    lib.write_config(concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "\n",
        "[[prompt.runners]]\n",
        "name = \"codex\"\n",
        "argv = [\"codex\", \"{{prompt}}\"]\n",
        "\n",
        "[[prompt.runners]]\n",
        "name = \"bad\"\n",
        "argv = [\"no-hole-here\"]\n",
    ));
    lib.pin_runner("blocked_pr", "codex");
    lib.overwrite_prompt_body("drift_pr", "Do {{a}}\n"); // gone left the body
    lib.remove_python_payload("gone");

    // No interpreter binary, no runner binary, no declared tool resolves on PATH.
    let empty_path = TempDir::new().unwrap();
    let report = lib.doctor_json(Some(empty_path.path()));

    assert_eq!(name_set(&report["missing"]), owned(&["gone"]));
    assert_eq!(name_set(&report["drift"]), owned(&["drift_py", "drift_pr"]));
    assert_eq!(
        report["needs_missing"]["needs_sh"],
        Value::from(vec!["ffmpeg"])
    );
    // needs_entries carries the ENTRY object itself, not None (the keys of needs_missing).
    assert_eq!(key_set(&report["needs_missing"]), owned(&["needs_sh"]));
    // launch_blocked names the two truly-blocked entries with a real reason...
    assert_eq!(
        key_set(&report["launch_blocked"]),
        owned(&["blocked_sh", "blocked_pr"])
    );
    assert_eq!(
        key_set(&report["launch_blocked"]),
        owned(&["blocked_sh", "blocked_pr"])
    );
    assert!(
        !report["launch_blocked"]["blocked_sh"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    // ...and NEVER double-reports an entry already missing or needs-flagged.
    assert!(report["launch_blocked"].get("gone").is_none());
    assert!(report["launch_blocked"].get("needs_sh").is_none());
    // (e) the malformed runner row is surfaced, the valid one is not.
    assert_eq!(report["runner_rows_invalid"], Value::from(vec!["bad"]));
}

#[test]
fn test_collect_double_report_exclusion_continues_not_breaks() {
    // The preflight loop skips entries already reported above (missing/needs/no-spec), just
    // that one entry, so LATER entries are still swept. A `break` mutant would abandon the
    // rest of the list at the first excluded entry. Ordering makes it observable: doctor
    // sweeps in slug order, so an excluded "aaa" precedes a blocked "zzz".
    let lib = Lib::new();
    lib.add_python("aaa_excluded", "print(1)\n");
    lib.add_shell("zzz_blocked"); // sorts AFTER; interpreter absent -> should be blocked
    lib.remove_python_payload("aaa_excluded"); // target-missing -> excluded from launch_blocked

    let empty_path = TempDir::new().unwrap();
    let report = lib.doctor_json(Some(empty_path.path()));

    assert!(name_set(&report["missing"]).contains("aaa_excluded"));
    // The later entry was still reached and reported — a `break` would have skipped it.
    assert!(key_set(&report["launch_blocked"]).contains("zzz_blocked"));
}

#[test]
fn test_collect_clean_library_reports_nothing() {
    // A stand-in `uv` on PATH so the python entry's interpreter resolves (the oracle's env
    // has a real uv). Nothing else is needed; the library is otherwise clean.
    let lib = Lib::new();
    lib.add_python("ok", "print(1)\n");
    let bin = TempDir::new().unwrap();
    make_fake_uv(bin.path());

    let report = lib.doctor_json(Some(bin.path()));

    assert!(name_set(&report["missing"]).is_empty());
    assert!(name_set(&report["drift"]).is_empty());
    assert!(report["needs_missing"].as_object().unwrap().is_empty());
    assert!(report["launch_blocked"].as_object().unwrap().is_empty());
    assert!(report["runner_rows_invalid"].as_array().unwrap().is_empty());
}
