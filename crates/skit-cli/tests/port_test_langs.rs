//! Mechanical port of the Python oracle module `tests/test_langs.py`
//! (`origin/main@206f9ef`): "Language registry (langs/): completeness gate + unknown-kind
//! degradation contract." Each `#[test]` keeps its Python `def test_*` name and its WHY
//! comment so it traces back to its origin.
//!
//! WHY skit-cli: the oracle module is a cross-cutting registry test. Python keeps one
//! `LangSpec` table (`skit.langs.registry`) that resolves glyph, family, launch strategy,
//! stored name, and the parser-backed capabilities in one object. The Rust rewrite
//! deliberately DISPERSES that table — stored name and storage modes in `skit-application`,
//! the parser capabilities as flat kind-keyed functions in `skit-language`, the launcher in
//! `skit-runtime`, the missing-target projection in `skit-store`, doctor/params in
//! `skit-cli`, and the kind glyph in a private `skit-tui` helper. Only the composition-root
//! crate (`skit-cli`) can reach every one of these without a forbidden dependency edit, so
//! the port lives here and drives each real public surface.
//!
//! Concept mapping used throughout:
//! - Python `registry.KNOWN_KINDS` -> the 13-kind `KNOWN_KINDS` list below (no Rust enum).
//! - Python `registry.spec_for(kind) is None` for an unknown kind -> the observable
//!   resolution surface: `build_launch_plan` answers `LaunchError::UnknownKind`,
//!   `supports_storage_modes` is false, `canonical_stored_filename` returns the "payload"
//!   fallback.
//! - Python `spec.stored_name` -> `skit_application::canonical_stored_filename` (`""` <-> `None`).
//! - Python `spec.supports_modes` -> `skit_application::supports_storage_modes`.
//! - Python `spec.analyzer`  -> `skit_language::detect_candidates` (present == detects candidates).
//! - Python `spec.cli_reader` -> `skit_language::cli_params` (present == reads a CLI surface).
//! - Python `spec.params_io`  -> `skit_language::{managed_params, write_managed_params}`.
//! - Python `spec.editable`   -> `canonical_stored_filename(kind).is_some()`.
//! - Python `launcher.build_command` -> `skit_runtime::build_launch_plan`.
//! - Python `launcher.describe_command` -> `skit_runtime::build_launch_preview` (`.display`).
//! - Python `launcher.target_missing`/`missing_marker` -> `skit_cli::library_surface`
//!   detail `missing_target`.
//! - Python `entry.script_path` -> `FileStore::entry_dir_path(slug).join(stored name)`.
//! - Python `store.add_exe` / CLI `params` / `doctor` -> the real `skit` binary via assert_cmd.
//!
//! Buckets:
//! - REAL asserting `#[test]` (API exists): 1, 2, 3, 8, 9, 10, 11, 12, 13, 14, 15, 18, 19, 20.
//!   Where a Python def mixes clauses whose CONCEPT has no public Rust surface (glyph, family,
//!   has_original_file, takes_argv, deps_flavor), the mappable clauses assert for real and
//!   the unmappable clauses are named in the WHY comment and the port ledger (NOT weakened
//!   into a match against Rust output).
//! - DIVERGENCE (full asserting body, `#[ignore]`d): 17 — the assertion is
//!   faithful to the oracle and compiles; it fails because Rust diverges. Fixing the impl
//!   and deleting the `#[ignore]` line turns it green.
//! - FRAMEWORK-INJECTION CLOSURE (compiling `#[ignore]` stub): 4, 5, 6, 7, 16, 21 — Python
//!   runtime mechanisms (lazy grammar import, `LazyCapabilities`, `without()`, dataclass
//!   `compare=False`, module-namespace monkeypatch) with no Rust equivalent by design.

use std::path::{Path, PathBuf};

use predicates::prelude::*;
use tempfile::TempDir;

use skit_application::delivery::Assembly;
use skit_application::{canonical_stored_filename, payload_stored_name, supports_storage_modes};
use skit_cli::library_surface;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_language::{cli_params, detect_candidates, managed_params, write_managed_params};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, build_launch_plan, build_launch_preview,
};
use skit_store::FileStore;

/// The oracle's `registry.KNOWN_KINDS` frozenset, in oracle order.
const KNOWN_KINDS: [&str; 13] = [
    "python",
    "shell",
    "fish",
    "js",
    "ts",
    "powershell",
    "ruby",
    "perl",
    "lua",
    "r",
    "exe",
    "command",
    "prompt",
];

// --- Launcher fixtures, self-contained (cribbed from crates/skit-runtime/tests/launch_plan.rs
// so this file edits no shared helper). ---

#[derive(Debug, Default)]
struct FakeProbe {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    executable: Vec<PathBuf>,
}

impl ProgramProbe for FakeProbe {
    fn find_program(&self, _name: &str) -> Option<PathBuf> {
        None
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|item| item == path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|item| item == path)
    }

    fn is_executable(&self, path: &Path) -> bool {
        self.executable.iter().any(|item| item == path)
    }
}

fn entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    }
}

fn paths(script: &str) -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from(script),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

fn probe_for(script: &str) -> FakeProbe {
    FakeProbe {
        files: vec![PathBuf::from(script)],
        dirs: vec![
            PathBuf::from("/invoke"),
            PathBuf::from("/data/scripts/demo"),
        ],
        executable: vec![PathBuf::from(script)],
    }
}

// ---- registry completeness -------------------------------------------------------------------

#[test]
fn test_every_known_kind_resolves_to_a_complete_spec() {
    // Oracle: KNOWN_KINDS is exactly these 13 keys, and each resolves to a fully populated
    // LangSpec (glyph, family, and a build/describe/target/preflight launch strategy).
    // Mappable in Rust: the storage-mode axis (supports_storage_modes), the stored-name
    // axis (canonical_stored_filename is total over every known kind), and the launch axis
    // (the launcher never answers UnknownKind for a known kind — it may still refuse for a
    // missing program/target/body). UNMAPPABLE clauses (named, not weakened): the kind glyph
    // (a private `kind_glyph` in skit-tui), the "family" axis (no public Rust surface), and
    // the set-CLOSURE direction of `== KNOWN_KINDS` (Rust has no kind enumeration API).
    for kind in KNOWN_KINDS {
        // canonical_stored_filename is total over every known kind (never panics): Some for
        // a copyable kind, None for exe/command.
        let stored = canonical_stored_filename(kind);
        assert_eq!(
            stored.is_none(),
            matches!(kind, "exe" | "command"),
            "{kind}"
        );

        let expects_modes = !matches!(kind, "exe" | "command");
        assert_eq!(
            supports_storage_modes(&EntryKind::parse(kind).unwrap()),
            expects_modes,
            "supports_storage_modes({kind})"
        );

        // The launch strategy exists: dispatch reaches a real strategy, never UnknownKind.
        if let Err(error) = build_launch_plan(
            &entry(kind),
            &paths("/copy/script"),
            &Assembly::default(),
            None,
            None,
            &probe_for("/copy/script"),
        ) {
            assert!(
                !matches!(error, LaunchError::UnknownKind { .. }),
                "kind={kind}: {error:?}"
            );
        }
    }
}

#[test]
fn test_python_spec_capabilities_and_pinned_store_name() {
    // stored_name is PINNED: existing stores carry script.py on disk (compat trap #2 in
    // docs/design/multilang.md) — renaming it would orphan every installed library.
    assert_eq!(canonical_stored_filename("python"), Some("script.py"));
    // editable: python has a stored text source to open in an editor.
    assert!(canonical_stored_filename("python").is_some());
    // supports_modes: copy/reference choice offered at add time.
    assert!(supports_storage_modes(&EntryKind::parse("python").unwrap()));

    // analyzer present: it detects source-bound candidates.
    assert!(!detect_candidates("python", "WIDTH = 800\n").is_empty());
    // cli_reader present: it reads the script's own argparse surface.
    let argparse =
        "import argparse\np = argparse.ArgumentParser()\np.add_argument(\"--count\", type=int)\n";
    assert!(!cli_params("python", argparse).is_empty());
    // params_io present: the [tool.skit] block round-trips (read + write).
    let mut declaration = ParamDecl::new("FOO");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = Some(ParameterValue::String("bar".to_owned()));
    let out = write_managed_params("python", "x = 1\n", &[declaration]).unwrap();
    assert!(!managed_params("python", &out).is_empty());

    // UNMAPPABLE clauses (named, not weakened): spec.extensions == (".py",), the "#" comment
    // prefix, spec.supports_deps, spec.takes_argv, and spec.has_original_file are LangSpec
    // traits with no public Rust surface (docs/design/python-test-port ledger).
}

#[test]
fn test_exe_and_command_specs_have_no_analysis_capabilities() {
    for kind in ["exe", "command"] {
        // No analyzer: no source-bound candidates.
        assert!(
            detect_candidates(kind, "COLOR = \"blue\"\n").is_empty(),
            "{kind}"
        );
        // No cli_reader: no static CLI surface.
        assert!(cli_params(kind, "import argparse\n").is_empty(), "{kind}");
        // No params_io: the '#'/'//' block engine is not offered for these kinds.
        assert!(
            managed_params(kind, "# /// script\n# [tool.skit]\n# ///\n").is_empty(),
            "{kind}"
        );
        // Not editable: no stored text source to open in an editor.
        assert!(
            !supports_storage_modes(&EntryKind::parse(kind).unwrap()),
            "{kind}"
        );
    }
    // exe keeps its original file; command has no stored copy at all.
    assert_eq!(canonical_stored_filename("exe"), None);
    assert_eq!(canonical_stored_filename("command"), None);

    // UNMAPPABLE clauses (named, not weakened): exe.family == "binary" /
    // command.family == "template", exe.has_original_file / not command.has_original_file, and
    // exe.takes_argv / not command.takes_argv — LangSpec traits with no public Rust surface.
}

#[test]
#[ignore = "UNMAPPABLE: Python import-laziness probe — spec_for() draws a badge without importing tree_sitter, checked by inspecting sys.modules (tests/test_langs.py:111). Rust links its grammars statically, so 'resolving a spec imports no parser' has no runtime analogue. Not a MUST-FIX feature."]
fn test_resolving_a_spec_does_not_import_a_language_parser() {
    // Python spawns a subprocess and asserts no `tree_sitter*` module loaded after
    // spec_for(...) for python/shell/fish/js/ts/powershell. No Rust equivalent by design.
}

#[test]
#[ignore = "UNMAPPABLE: LazyCapabilities builds a kind's parser capabilities on first use and at most once (tests/test_langs.py:131). Rust has no per-kind lazy capability builder (no LangSpec object). Not a MUST-FIX feature."]
fn test_asking_for_a_capability_resolves_it_once() {
    // Python asserts LazyCapabilities(build) runs `build` exactly once, lazily. No Rust seam.
}

#[test]
#[ignore = "UNMAPPABLE: LangSpec.without(names) models the shape a MISSING tree-sitter grammar wheel produces (tests/test_langs.py:153). Rust links grammars statically, so a capability can never be dropped at runtime. Not a MUST-FIX feature."]
fn test_without_drops_exactly_the_named_capabilities() {
    // Python asserts without("analyzer","injector") nulls exactly those, leaves the rest, and
    // does not mutate the original. No Rust capability-stripping surface exists.
}

#[test]
#[ignore = "UNMAPPABLE: dataclass compare=False on a LangSpec's capabilities — two specs stay equal even after one resolves its grammar (tests/test_langs.py:168). Rust has no LangSpec value whose equality could be observed. Not a MUST-FIX feature."]
fn test_capabilities_do_not_decide_spec_identity() {
    // Python asserts dataclasses.replace(spec, capabilities=...) == spec. No Rust equivalent.
}

#[test]
fn test_spec_for_unknown_kind_is_none_and_cached() {
    // Oracle: spec_for("martian") is None (twice — the cached path returns the same answer).
    // The observable Rust resolution surface for an unknown kind: the launcher answers
    // UnknownKind, storage modes are unpromised, and the stored name falls back to "payload".
    let martian = EntryKind::parse("martian").unwrap();
    assert!(!supports_storage_modes(&martian));
    assert_eq!(canonical_stored_filename("martian"), Some("payload"));
    let error = build_launch_plan(
        &entry("martian"),
        &paths("/copy/script"),
        &Assembly::default(),
        None,
        None,
        &probe_for("/copy/script"),
    )
    .unwrap_err();
    assert!(
        matches!(error, LaunchError::UnknownKind { .. }),
        "{error:?}"
    );
    // NOTE: the oracle's "cached, same answer" clause is meaningless for a stateless `match`
    // dispatch — there is no memo to observe — so it is not separately asserted.
}

#[test]
fn test_stored_name_unknown_kind_falls_back_to_payload() {
    // A newer store's copy-mode entry must still resolve to *some* path (never crash).
    // Rust encodes the oracle's `""` (this kind is never copied) as `None`.
    assert_eq!(canonical_stored_filename("martian"), Some("payload"));
    assert_eq!(canonical_stored_filename("python"), Some("script.py"));
    assert_eq!(canonical_stored_filename("exe"), None); // oracle: ""
    assert_eq!(canonical_stored_filename("command"), None); // oracle: ""
}

// ---- unknown-kind degradation at every launcher consumer --------------------------------------

#[test]
fn test_unknown_kind_build_command_raises_clean_launch_error() {
    // build_command raises a clean LaunchError naming the unknown kind.
    let error = build_launch_plan(
        &entry("martian"),
        &paths("/copy/script"),
        &Assembly::default(),
        None,
        None,
        &probe_for("/copy/script"),
    )
    .unwrap_err();
    assert!(
        matches!(error, LaunchError::UnknownKind { .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("martian"), "{error}");
}

#[test]
fn test_unknown_kind_run_entry_raises_before_spawning() {
    // run_entry raises before any spawn: in Rust a spawn (execute_launch) consumes a
    // LaunchPlan, and one can never be built for an unknown kind, so the process is
    // structurally unreachable.
    let outcome = build_launch_plan(
        &entry("martian"),
        &paths("/copy/script"),
        &Assembly::default(),
        None,
        None,
        &probe_for("/copy/script"),
    );
    assert!(matches!(outcome, Err(LaunchError::UnknownKind { .. })));
}

#[test]
fn test_unknown_kind_describe_returns_template_and_never_raises() {
    // describe_command is contracted side-effect-free and total: for a kind this skit
    // version doesn't know, the template is the only launch material meta carries.
    let mut with_template = entry("martian");
    with_template.meta.workdir = "invoke".to_owned();
    EntrySettings {
        template: "frob --it".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut with_template.meta);
    let plan = build_launch_preview(
        &with_template,
        &paths("/copy/script"),
        &Assembly::default(),
        None,
        None,
        None,
        &probe_for("/copy/script"),
    )
    .expect("describe must be total for an unknown kind");
    assert_eq!(plan.display, "frob --it");

    let mut without_template = entry("martian");
    without_template.meta.workdir = "invoke".to_owned();
    let empty = build_launch_preview(
        &without_template,
        &paths("/copy/script"),
        &Assembly::default(),
        None,
        None,
        None,
        &probe_for("/copy/script"),
    )
    .expect("describe must be total for an unknown kind");
    assert_eq!(empty.display, "");
}

#[test]
fn test_unknown_kind_never_reports_missing() {
    // Nothing this version can check — a missing-marker would be a false alarm. In Rust the
    // total projection is the Library surface: for an unknown kind launch_target yields None,
    // so detail.missing_target is None (Python: target_missing False, missing_marker None).
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    // A martian entry written by a newer skit: hand-write its meta.toml. The v0.4
    // compatibility rules guarantee the store keeps an open, unknown kind readable.
    let entry_dir = data.path().join("scripts").join("thing");
    std::fs::create_dir_all(&entry_dir).unwrap();
    std::fs::write(
        entry_dir.join("meta.toml"),
        "name = \"thing\"\nkind = \"martian\"\nsource = \"/gone/original\"\n",
    )
    .unwrap();

    let store = FileStore::new(data.path());
    let surface = library_surface(&store, state.path(), config.path()).unwrap();
    let detail = surface
        .details
        .get(&Slug::parse("thing").unwrap())
        .expect("the martian entry is projected, not hidden");
    assert_eq!(detail.missing_target, None);
}

#[test]
fn test_unknown_kind_preflight_still_checks_workdir() {
    // Rust fuses preflight into build_launch_plan, which resolves and validates the workdir
    // BEFORE the kind dispatch. So for an unknown kind a good workdir passes (leaving only the
    // UnknownKind refusal) and a missing workdir is still caught first — exactly the oracle's
    // "no strategy checks, workdir fine" / "raises on missing workdir".
    let mut ok = entry("martian");
    ok.meta.workdir = "/workdir/ok".to_owned();
    let mut probe = probe_for("/copy/script");
    probe.dirs.push(PathBuf::from("/workdir/ok"));
    let error = build_launch_plan(
        &ok,
        &paths("/copy/script"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    // The workdir passed, so the only remaining refusal is the unknown kind.
    assert!(
        matches!(error, LaunchError::UnknownKind { .. }),
        "{error:?}"
    );

    let mut gone = entry("martian");
    gone.meta.workdir = "/workdir/gone".to_owned();
    let error = build_launch_plan(
        &gone,
        &paths("/copy/script"),
        &Assembly::default(),
        None,
        None,
        &probe_for("/copy/script"),
    )
    .unwrap_err();
    // The workdir IS validated even for an unknown kind (checked before kind dispatch).
    assert!(
        matches!(error, LaunchError::WorkdirMissing { .. }),
        "{error:?}"
    );
}

#[test]
fn test_unknown_kind_script_path_uses_payload_fallback() {
    // entry.script_path == entry.dir / "payload". In Rust the entry's stored path is
    // entry_dir_path(slug) joined with the copy filename, which for an unknown kind is the
    // historical "payload" fallback.
    let store = FileStore::new(PathBuf::from("/data"));
    let slug = Slug::parse("thing").unwrap();
    let kind = EntryKind::parse("martian").unwrap();
    assert_eq!(canonical_stored_filename("martian"), Some("payload"));
    let filename = payload_stored_name(&kind, Path::new("/gone/original"));
    assert_eq!(filename, "payload");
    assert_eq!(
        store.entry_dir_path(&slug).join(filename),
        PathBuf::from("/data/scripts/thing/payload"),
    );
}

// ---- launcher's dynamic uv delegates -----------------------------------------------------------

#[test]
#[ignore = "UNMAPPABLE: monkeypatching skit.langs.launch.find_uv/ensure_uv must reach consumers that call launcher.find_uv/ensure_uv, or the two module namespaces split-brain (tests/test_langs.py:240). Rust resolves uv through a ProgramProbe, not a patchable module delegate. Not a MUST-FIX feature."]
fn test_launcher_uv_delegates_follow_patches_on_the_canonical_module() {
    // Python asserts launcher.find_uv()/ensure_uv() follow a monkeypatch of the canonical
    // skit.langs.launch namespace. No Rust equivalent by design.
}

// ---- audited fixes: capability-honest CLI behavior --------------------------------------------

fn write_exe(dir: &Path) -> PathBuf {
    let program = dir.join("tool");
    std::fs::write(&program, "#!/bin/sh\necho hi\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    program
}

#[test]
fn test_params_exe_prints_plain_message_without_manage_dead_end() {
    // `--manage` hard-errors for kinds without an analyzer, so the empty-params message
    // must not send exe users down that dead end (it used to suggest --manage).
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = TempDir::new().unwrap();
    let exe = write_exe(source.path());

    let skit = || {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", data.path())
            .env("SKIT_STATE_DIR", state.path())
            .env("SKIT_CONFIG_DIR", config.path())
            .env("SKIT_LANG", "en");
        command
    };
    skit()
        .arg("add")
        .arg(&exe)
        .args(["--exe", "--name", "prog"])
        .assert()
        .success();
    skit()
        .args(["params", "prog"])
        .assert()
        .success()
        .stdout(predicate::str::contains("has no managed parameters"))
        .stdout(predicate::str::contains("--manage").not());
}

#[test]
fn test_doctor_missing_uv_pure_exe_library_exits_zero() {
    // A library with no python entries runs fine without uv — exit 1 there sent
    // automation chasing a phantom problem. The red uv line still prints.
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = TempDir::new().unwrap();
    let empty_path = TempDir::new().unwrap();
    let exe = write_exe(source.path());

    let mut add = assert_cmd::cargo::cargo_bin_cmd!("skit");
    add.env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .arg("add")
        .arg(&exe)
        .args(["--exe", "--name", "prog"])
        .assert()
        .success();

    // An empty PATH makes uv unresolvable (the managed private uv under a fresh data dir
    // also does not exist).
    let mut doctor = assert_cmd::cargo::cargo_bin_cmd!("skit");
    doctor
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .env("PATH", empty_path.path())
        .arg("doctor")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("uv"));
}

#[test]
fn test_doctor_missing_uv_with_python_entry_exits_one() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = TempDir::new().unwrap();
    let empty_path = TempDir::new().unwrap();
    let script = source.path().join("a.py");
    std::fs::write(&script, "print(1)\n").unwrap();

    let mut add = assert_cmd::cargo::cargo_bin_cmd!("skit");
    add.env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .arg("add")
        .arg(&script)
        .args(["--name", "a"])
        .assert()
        .success();

    let mut doctor = assert_cmd::cargo::cargo_bin_cmd!("skit");
    doctor
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .env("PATH", empty_path.path())
        .arg("doctor")
        .assert()
        .code(1);
}

#[test]
fn test_doctor_json_missing_uv_pure_exe_library_exits_zero() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = TempDir::new().unwrap();
    let empty_path = TempDir::new().unwrap();
    let exe = write_exe(source.path());

    let mut add = assert_cmd::cargo::cargo_bin_cmd!("skit");
    add.env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .arg("add")
        .arg(&exe)
        .args(["--exe", "--name", "prog"])
        .assert()
        .success();

    let mut doctor = assert_cmd::cargo::cargo_bin_cmd!("skit");
    doctor
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .env("PATH", empty_path.path())
        .args(["doctor", "--json"])
        .assert()
        .code(0);
}

// ---- plan_for_entry: capability degradation ----------------------------------------------------

#[test]
#[ignore = "FRAMEWORK-INJECTION CLOSURE: the oracle strips a spec's cli_reader (spec.without(\"cli_reader\")) and monkeypatches flows.spec_for to prove plan_for_entry falls through to source \"none\" instead of crashing (tests/test_langs.py:307). Rust cannot construct a python spec without its statically linked CLI reader — the LazyCapabilities/without() injection seam does not exist — so the degradation path is unreachable. This tests a Python monkeypatch mechanism, not a missing product capability."]
fn test_plan_without_cli_reader_degrades_to_none_plan() {
    // Python: a future kind can carry params_io+analyzer but no static CLI reader; the plan
    // must fall through to "none", not crash. No Rust capability-stripping seam exists.
}
