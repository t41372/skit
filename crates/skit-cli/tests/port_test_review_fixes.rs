//! Mechanical port of the Python oracle module `tests/test_review_fixes.py`
//! (`origin/main@206f9ef`, version 0.4.1.dev0): "Regression tests for launch, rewrite,
//! metadata, i18n, and environment edge cases." Each `#[test]` keeps its Python `def
//! test_*` name and its Python "WHY" comment so it traces back to its origin.
//!
//! This oracle module is a cross-cutting grab bag: one Python test file that reaches into
//! `launcher`, `langs.launch`, `langs.python.shim`, `rewrite`, `langs.python.metawriter`,
//! `langs.python.reconcile`, `pep723`, `i18n`, `models`, `atomic`, `argstate`, `store`, and
//! `uvman`. No single lower crate reaches all of those, so the port lives in `skit-cli-rs`,
//! the composition root — the only crate whose dependency graph spans `skit-language`,
//! `skit-runtime`, `skit-store`, `skit-application`, `skit-domain`, and `skit-i18n` at once.
//!
//! Oracle fixture mapping: the module's autouse fixture points `SKIT_DATA_DIR` / `SKIT_STATE_DIR`
//! at `tmp_path`. The Rust adapters take their roots through constructors, so every test injects
//! `tempfile::TempDir` paths into `FileStore::new` / `FileConfigStore::new` /
//! `FileFormStateStore::new` instead. No test sets a process environment variable — parallel
//! `cargo test` threads share one environment, so `std::env::set_var` would race.
//!
//! Concept mapping (Python -> Rust):
//! - `launcher.build_command(command_entry, values)` (POSIX render) ->
//!   `render_command_template(template, &values)` (skit-runtime), the exact `_substitute_posix`
//!   twin. Extra-arg appending is only reachable through the full `build_launch_plan` (command
//!   entry lowered to `sh -c <text>`), so that one test drives the plan.
//! - `shim.inject(text, specs, values)` -> `inject_values("python", text, &specs, &values)`
//!   (skit-language). A non-finite float raises `shim.ShimValueError` (a `ShimError` subclass);
//!   Rust returns `LanguageError::InvalidValue`.
//! - `metawriter.write_params` / `read_params` -> `write_managed_params` / `managed_params`.
//! - `pep723.set_dependencies` / `parse_block` -> `write_uv_metadata` / `read_uv_metadata`.
//! - `store.extract_placeholders` (via `add_command`'s `meta.params`) -> `placeholder_params`.
//! - `models.slugify` -> `Slug::from_display_name` (skit-domain).
//! - `store.add_python(reference)` / `add_exe` / `add_command` -> `FileStore::create(CreateEntry)`;
//!   `store.update_dependencies` (reference / exe: meta only, no PEP 723 sync) ->
//!   `FileStore::update_settings(entry, settings, workdir)`.
//! - `store._unique_slug` (private) -> observed behaviorally through three `create` calls.
//! - `argstate.load_state(slug)["values"]` -> `FileFormStateStore::load(slug).values`.
//! - `i18n._config_language` / `set_language` / `is_supported` -> the config-language surface that
//!   moved to `skit-store`'s `FileConfigStore` (`get("lang")` / `set("lang", …)`); the oracle's
//!   `is_supported` is exactly what the CLI `lang` command uses to validate a tag (i18n.py:202-212),
//!   and `FileConfigStore::set("lang", …)`'s accept/reject (via the private
//!   `normalize_supported_language`) is its reachable equivalent.
//! - `i18n._normalize`'s 4-char script-subtag title-casing -> `FileConfigStore::set("lang", …)` /
//!   `get("lang")` (skit-store), whose private `normalize_supported_language` applies the same
//!   `capitalize()` casing before storing the tag; `detect_locale(Some(tag))` (skit-i18n) then
//!   confirms the `zh-Hant-*` tag still resolves to Traditional Chinese.
//!
//! Buckets:
//! - ASSERTING (24 `#[test]`): everything the reachable public API can drive directly.
//! - STUBS (6 `#[ignore]`), recorded in the agent's structured output:
//!   * `test_write_injected_unique_and_private` — kind="cross-crate". `rewrite.write_injected`
//!     has no public Rust function; the injected-temp write is inlined in
//!     `crates/skit-cli/src/run/command.rs:679-699`. It also DIVERGES: the oracle writes the
//!     secret-bearing copy to the OS temp dir with a `.injected-` prefix (rewrite.py "3b": a
//!     crash must never strand a plaintext-secret file the store never sweeps), while Rust writes
//!     `.run-<id>` INTO `entry_dir` at 0o600. Adjudication item for the main agent.
//!   * `test_atomic_write_bytes_cleanup_on_error` — kind="cross-crate". White-box `mock.patch` of
//!     `os.fdopen`; the atomic writer lives in `skit-store` (`mutations/atomic.rs`) and offers no
//!     public fault-injection seam. `crates/skit-store/tests/port_test_atomic.rs` stubs the
//!     sibling fsync-failure cleanup test for the same reason.
//!   * `test_available_locales_missing_dir` — kind="architecture-closure". Rust embeds the
//!     complete catalog at compile time, so no runtime locales directory can disappear. The
//!     stronger packaged-catalog contract owns availability without reproducing Python I/O.
//!   * `test_detect_locale_locale_module_error` — kind="cross-crate". The oracle's no-arg
//!     `detect_locale()` reads the `SKIT_LANG` > config > `LC_ALL` > `LC_MESSAGES` > `LANG` >
//!     system chain; that precedence lives in `crates/skit-cli/src/cli.rs:219-238`
//!     (`requested_locale` + `system_locale`), and the `locale.getlocale()` ValueError branch has
//!     no analog (`sys_locale` yields an empty iterator, never raises).
//!   * `test_find_uv_private_bin_exe_variant` — kind="cross-crate". uv discovery is not a public
//!     `skit-runtime` function; the PATH -> managed-bin fallback is in
//!     `crates/skit-cli/src/run/command.rs` (`managed_uv_path` + `ensure_managed_uv`), and
//!     `managed_uv_path` picks `uv` vs `uv.exe` at COMPILE time (`cfg!(windows)`), so the runtime
//!     "try uv, then uv.exe" probe the oracle tests cannot exist.
//!   * `test_ensure_uv_downloaded_success` — kind="cross-crate". White-box `monkeypatch` of
//!     `urlopen` / `copyfileobj` / `_extract_uv` / `_verify_checksum`. The download success path is
//!     `skit_runtime::ensure_managed_uv` (`uv.rs`); its dedicated port is
//!     `crates/skit-runtime/tests/port_test_uvman.rs`.
//! - DIVERGENCE (failing-contract) tests: NONE.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skit_application::{
    CreateEntry, EntryMutationRepository, delivery::Assembly, form_state::FormStateRepository,
};
use skit_domain::{
    Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode,
    parameters::{
        NamedEdit, ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, SourceEditRequest,
    },
};
use skit_i18n::{Locale, detect_locale};
use skit_language::{
    LanguageError, edit_source_declarations, inject_values, managed_params, placeholder_params,
    read_uv_metadata, write_managed_params, write_uv_metadata,
};
use skit_runtime::{LaunchPaths, ProgramProbe, build_launch_plan, render_command_template};
use skit_store::{FileConfigStore, FileFormStateStore, FileStore};
use tempfile::TempDir;

// ==================================================================================
// Shared fixtures
// ==================================================================================

/// The `ProgramProbe` seam that replaces the oracle's `monkeypatch` of `shutil.which` and the
/// real filesystem (mirrors `crates/skit-runtime/tests/port_test_launcher.rs`).
#[derive(Debug, Default)]
struct FakeProbe {
    programs: BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    executable: Vec<PathBuf>,
}

impl ProgramProbe for FakeProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
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

/// One entry of a given kind, in the oracle's default mode.
fn entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    }
}

/// The three launch paths (script / entry dir / invoke cwd).
fn launch_paths(script: &str) -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from(script),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

/// A probe that finds the script and both known directories.
fn probe_for(script: &str) -> FakeProbe {
    FakeProbe {
        files: vec![PathBuf::from(script)],
        dirs: vec![
            PathBuf::from("/invoke"),
            PathBuf::from("/data/scripts/demo"),
        ],
        executable: vec![PathBuf::from(script)],
        ..FakeProbe::default()
    }
}

/// Oracle `ParamDecl(name, binding="const", type="str")`. Const implies `Inject` delivery, which
/// is what `plan_python_injection` selects on.
fn const_spec(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

/// Oracle `ParamDecl(name, binding="const", type="float")`.
fn float_spec(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Float;
    declaration
}

/// Oracle `ParamDecl(name, binding="input", order=order)`.
fn input_spec(name: &str, order: i64) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.order = order;
    declaration
}

/// The oracle's `{name: value}` fill-in dict.
fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// The store slice of `store.add_command(template, name=name)`.
fn command_create(name: &str, template: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Reference,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: None,
        settings: EntrySettings {
            template: template.to_owned(),
            ..EntrySettings::default()
        },
    }
}

/// The store slice of `store.add_python(source, mode="reference")`.
fn python_reference_create(name: &str, source: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Reference,
        source: source.to_owned(),
        workdir: "origin".to_owned(),
        description: String::new(),
        payload: None,
        settings: EntrySettings::default(),
    }
}

/// The store slice of `store.add_exe(source)`.
fn exe_reference_create(name: &str, source: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("exe").unwrap(),
        mode: StorageMode::Reference,
        source: source.to_owned(),
        workdir: "origin".to_owned(),
        description: String::new(),
        payload: None,
        settings: EntrySettings::default(),
    }
}

// ==================================================================================
// launcher: {{name}} escapes must not be replaced by a same-named placeholder
// ==================================================================================

#[test]
fn test_escaped_placeholder_not_substituted() {
    // `{{name}}` is a literal-brace escape, restored to `{name}`; the real `{name}` slot takes the
    // value. Both happen in one pass so a value containing braces is never re-scanned.
    let out = render_command_template("echo {{name}} {name}", &values(&[("name", "X")])).unwrap();
    assert_eq!(out, "echo {name} X");
}

#[test]
fn test_escape_unescaped_even_without_params() {
    // A template with only `{{literal}}` has no managed placeholder (`meta.params is None`), yet
    // the escape must still be unescaped to `{literal}` on render.
    assert!(placeholder_params("command", "echo {{literal}}").is_empty());
    let out = render_command_template("echo {{literal}}", &BTreeMap::new()).unwrap();
    assert_eq!(out, "echo {literal}");
}

#[cfg(not(windows))]
#[test]
fn test_extra_args_quoted_for_posix_shell() {
    // shlex quoting (Rust `quote_posix_arg`) must preserve $, backticks, and spaces as literals
    // when extra args are appended to a command entry's shell text. Python returns the bare shell
    // string; Rust lowers it to `sh -c "<rendered> <extra>"`, so the rendered text is `args[1]`.
    let mut command = entry("command");
    command.meta.workdir = "invoke".to_owned();
    EntrySettings {
        template: "echo hi".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut command.meta);
    let mut probe = probe_for("/unused");
    probe
        .programs
        .insert("sh".to_owned(), PathBuf::from("/bin/sh"));
    let extra = vec!["$HOME".to_owned(), "a b".to_owned(), "`whoami`".to_owned()];
    let assembly = Assembly {
        args: extra.clone(),
        masked_args: extra,
        ..Assembly::default()
    };

    let plan = build_launch_plan(
        &command,
        &launch_paths("/unused"),
        &assembly,
        None,
        None,
        &probe,
    )
    .unwrap();

    let command_text = &plan.args[1];
    assert!(command_text.contains("'$HOME'"), "{command_text}");
    assert!(command_text.contains("'a b'"), "{command_text}");
    assert!(command_text.contains("'`whoami`'"), "{command_text}");
}

// ==================================================================================
// shim: non-finite floats must be explicitly rejected (X = inf is not valid Python)
// ==================================================================================

#[test]
fn test_inject_rejects_non_finite_float() {
    // repr(inf/nan) is not a valid Python literal, so a non-finite float value is rejected
    // (oracle `shim.ShimValueError`, a `ShimError` subclass) rather than injected.
    let text = "RATE = 1.5\nprint(RATE)\n";
    for bad in ["inf", "-inf", "nan", "Infinity"] {
        let error = inject_values(
            "python",
            text,
            &[float_spec("RATE")],
            &values(&[("RATE", bad)]),
        )
        .expect_err(bad);
        assert!(
            matches!(error, LanguageError::InvalidValue { .. }),
            "{bad}: {error:?}"
        );
    }
}

#[test]
fn test_inject_accepts_normal_float() {
    // A finite float value is coerced and injected in place of the literal RHS.
    let text = "RATE = 1.5\nprint(RATE)\n";
    let out = inject_values(
        "python",
        text,
        &[float_spec("RATE")],
        &values(&[("RATE", "2.75")]),
    )
    .unwrap();
    assert!(out.contains("RATE = 2.75"), "{out}");
}

// ==================================================================================
// rewrite.write_injected: unique filename + private permissions
// ==================================================================================

#[test]
#[ignore = "UNMAPPED (cross-crate): `rewrite.write_injected` has no public Rust function. The \
injected-temp write is inlined in crates/skit-cli/src/run/command.rs:679-699. It also DIVERGES: \
the oracle writes the secret-bearing copy to the OS temp dir with a `.injected-` prefix \
(rewrite.py 3b: a crash must never strand a plaintext-secret file the store never sweeps); Rust \
writes `.run-<id>` INTO entry_dir at 0o600. Oracle ref: src/skit/rewrite.py:145-198."]
fn test_write_injected_unique_and_private() {
    // Oracle behavior (unreachable/divergent): two calls return distinct paths, name starts with
    // `.injected-`, suffix `.py`, content round-trips, and mode is 0o600 on POSIX.
}

// ==================================================================================
// metawriter: a prompt containing control characters must round-trip cleanly
// ==================================================================================

#[test]
fn test_write_params_prompt_with_newline_roundtrips() {
    // A prompt with a newline and a tab must survive the write -> read cycle byte-for-byte.
    let text = "CITY = \"Taipei\"\nprint(CITY)\n";
    let mut spec = const_spec("CITY");
    spec.prompt = "City:\nwith newline\t!".to_owned();
    let out = write_managed_params("python", text, &[spec]).unwrap();
    let back = managed_params("python", &out);
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].prompt, "City:\nwith newline\t!");
}

// ==================================================================================
// reconcile.edit_specs: pure function must not mutate the caller's specs
// ==================================================================================

#[test]
fn test_edit_specs_does_not_mutate_input() {
    let text = "CITY = \"Taipei\"\n";
    let original = vec![const_spec("CITY")];
    let before = original.clone();
    let result = edit_source_declarations(
        "python",
        text,
        &original,
        &SourceEditRequest {
            secret: vec!["CITY".to_owned()],
            prompts: vec![NamedEdit::new("CITY", "changed")],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();

    assert_eq!(original, before);
    assert!(!original[0].secret);
    assert!(original[0].prompt.is_empty());
    assert!(result.declarations[0].secret);
    assert_eq!(result.declarations[0].prompt, "changed");
}

// ==================================================================================
// pep723: multi-line dependency array with inline comment must not leave orphan lines
// ==================================================================================

#[test]
fn test_set_dependencies_multiline_array_with_comment() {
    // An inline comment on the deps-array opener line must not desync the block rewrite.
    let text =
        "# /// script\n# dependencies = [  # my deps\n#     \"requests\",\n# ]\n# ///\nprint(1)\n";
    let out = write_uv_metadata(text, &["httpx".to_owned()], "").unwrap();
    let meta = read_uv_metadata(&out).expect("metadata block present");
    assert_eq!(meta.dependencies, ["httpx"]);
}

// ==================================================================================
// i18n.is_supported: garbage tags must be rejected
// ==================================================================================

#[test]
fn test_is_supported_rejects_junk() {
    // The oracle `is_supported` (i18n.py:202-212) is exactly the predicate the CLI `lang` command
    // uses to accept or reject a tag. Its reachable Rust equivalent is `FileConfigStore::set("lang",
    // …)`, which validates through the private `normalize_supported_language`.
    let config = TempDir::new().unwrap();
    let store = FileConfigStore::new(config.path());
    for tag in ["zh-TW", "zh_TW.UTF-8", "en-US", "x-pseudo"] {
        assert!(store.set("lang", tag).is_ok(), "{tag}");
    }
    for tag in ["ent", "english", "fr"] {
        assert!(store.set("lang", tag).is_err(), "{tag}");
    }
    assert!(
        store.set("lang", "").is_ok(),
        "an exact empty value clears the setting"
    );
}

// ==================================================================================
// models.slugify: all-special input falls back to "script"
// ==================================================================================

#[test]
fn test_slugify_all_special_chars_fallback() {
    assert_eq!(Slug::from_display_name("---").as_str(), "script");
    assert_eq!(Slug::from_display_name("!!!").as_str(), "script");
    assert_eq!(Slug::from_display_name("  ").as_str(), "script");
}

// ==================================================================================
// metawriter.write_params: no block + no params -> text unchanged
// ==================================================================================

#[test]
fn test_write_params_no_block_no_params() {
    // If the source has no PEP 723 block and there are no params to write, return unchanged.
    let text = "print(1)\n";
    let result = write_managed_params("python", text, &[]).unwrap();
    assert_eq!(result, text);
}

// ==================================================================================
// pep723.parse_block: corrupt block body returns None
// ==================================================================================

#[test]
fn test_parse_block_corrupt_body_returns_none() {
    let bad = "# /// script\n# not: valid: toml: [\n# ///\nprint(1)\n";
    assert!(read_uv_metadata(bad).is_none());
}

// ==================================================================================
// atomic: exception cleanup deletes the temp file
// ==================================================================================

#[test]
#[ignore = "UNMAPPED (cross-crate): white-box mock.patch of os.fdopen with no public \
fault-injection seam. The atomic writer lives in crates/skit-store (mutations/atomic.rs); \
crates/skit-store/tests/port_test_atomic.rs stubs the sibling fsync-failure cleanup test for the \
same reason. Oracle ref: src/skit/atomic.py:202-225."]
fn test_atomic_write_bytes_cleanup_on_error() {
    // Oracle behavior (unreachable): a mid-write raise must leave no `.<name>.<id>.tmp` residue.
}

// ==================================================================================
// argstate: corrupt state file falls back to empty dict
// ==================================================================================

#[test]
fn test_argstate_corrupt_file_fallback() {
    // A corrupt `values/<slug>.toml` must not raise; the load degrades to empty state.
    let state = TempDir::new().unwrap();
    let values_dir = state.path().join("values");
    fs::create_dir_all(&values_dir).unwrap();
    fs::write(values_dir.join("myscript.toml"), "[[[invalid").unwrap();
    let store = FileFormStateStore::new(state.path());
    let loaded = store.load(&Slug::parse("myscript").unwrap());
    assert!(loaded.values.is_empty());
}

// ==================================================================================
// i18n: available_locales when locales dir absent
// ==================================================================================

#[test]
#[ignore = "ARCHITECTURE CLOSURE: Rust embeds its complete locale catalog at compile time, so there is no runtime locales directory that can disappear. `skit_i18n::available_locale_tags()` and the packaged catalog contract are stronger than Python's missing-directory fallback. Oracle ref: src/skit/i18n.py:75-82."]
fn test_available_locales_missing_dir() {
    // Python runtime-directory I/O has no Rust product boundary to execute.
}

// ==================================================================================
// i18n: 4-char subtag is capitalized
// ==================================================================================

#[test]
fn test_normalize_four_char_subtag() {
    // Oracle `_normalize("zh-hant-tw") == "zh-Hant-TW"` (i18n.py:82-99): the 4-char script subtag
    // is title-cased. The reachable Rust twin is the config `lang` setter, whose private
    // `normalize_supported_language` (skit-store/src/config.rs) applies the same casing — 2-char
    // subtag upper-cased, 4-char subtag `capitalize()`d — before it stores the tag. Read the
    // stored `lang` back and assert the EXACT normalized string, so a broken title-caser fails.
    let config = TempDir::new().unwrap();
    let store = FileConfigStore::new(config.path());
    store.set("lang", "zh-hant-tw").unwrap();
    let stored = store.get("lang").unwrap();
    assert_eq!(stored, "zh-Hant-TW");
    // The Hant-script tag is still recognized as Traditional Chinese.
    assert_eq!(detect_locale(Some(&stored)), Locale::ZhTw);
}

// ==================================================================================
// i18n: detect_locale ValueError/TypeError from locale.getlocale
// ==================================================================================

#[test]
#[ignore = "UNMAPPED (cross-crate): the oracle's no-arg detect_locale() reads the \
SKIT_LANG>config>LC_ALL>LC_MESSAGES>LANG>system chain; that precedence lives in \
crates/skit-cli/src/cli.rs:219-238 (requested_locale + system_locale), and the \
locale.getlocale() ValueError branch has no analog (sys_locale yields an empty iterator, never \
raises). Oracle ref: src/skit/i18n.py:176-199."]
fn test_detect_locale_locale_module_error() {
    // Oracle behavior (cross-crate): with no env preference and locale.getlocale raising,
    // detect_locale() == "".
}

// ==================================================================================
// i18n: _config_language with corrupt config file
// ==================================================================================

#[test]
fn test_config_language_corrupt_file() {
    // A corrupt config.toml must return an empty language silently, not raise. The config-language
    // surface moved to skit-store's FileConfigStore; `get("lang")` reads the stored `language` key.
    let config = TempDir::new().unwrap();
    fs::write(config.path().join("config.toml"), "[[[bad").unwrap();
    let store = FileConfigStore::new(config.path());
    assert_eq!(store.get("lang").unwrap(), "");
}

// ==================================================================================
// i18n: set_language OSError on corrupt existing config
// ==================================================================================

#[test]
fn test_set_language_with_existing_corrupt_config() {
    // Setting the language over a corrupt existing config must not crash; it falls back to an empty
    // document and writes the new language.
    let config = TempDir::new().unwrap();
    fs::write(config.path().join("config.toml"), "[[[bad").unwrap();
    let store = FileConfigStore::new(config.path());
    store.set("lang", "en-US").unwrap();
    assert!(config.path().join("config.toml").is_file());
}

// ==================================================================================
// models: slugify leading/trailing dashes stripped
// ==================================================================================

#[test]
fn test_slugify_leading_trailing_special() {
    // A name starting with a non-alnum char: no leading dash in output.
    assert_eq!(Slug::from_display_name("-hello-").as_str(), "hello");
    // A name where non-alnum appears mid-word: dash injected once only.
    assert_eq!(
        Slug::from_display_name("hello  world").as_str(),
        "hello-world"
    );
}

// ==================================================================================
// shim: AnnAssign (annotated assignment) is a const target
// ==================================================================================

#[test]
fn test_inject_annotated_assignment() {
    // `CITY: str = 'Taipei'` is an annotated assignment; its RHS is a valid const target.
    let src = "CITY: str = 'Taipei'\nprint(CITY)\n";
    let out = inject_values(
        "python",
        src,
        &[const_spec("CITY")],
        &values(&[("CITY", "Kaohsiung")]),
    )
    .unwrap();
    assert!(out.contains("'Kaohsiung'"), "{out}");
}

// ==================================================================================
// shim: preamble appended when body is only __future__ imports
// ==================================================================================

#[test]
fn test_preamble_appended_when_only_future_imports() {
    // A module whose first statement is a __future__ import has its insertion point AFTER that
    // line: the input preamble must appear after `from __future__`, never before it.
    let src = "from __future__ import annotations\nx = input()\nprint(x)\n";
    let out = inject_values(
        "python",
        src,
        &[input_spec("input-1", 0)],
        &values(&[("input-1", "v")]),
    )
    .unwrap();
    assert!(out.contains("# skit:shim"), "{out}");
    let lines = out.lines().collect::<Vec<_>>();
    let future_idx = lines
        .iter()
        .position(|line| line.contains("__future__"))
        .unwrap();
    let shim_idx = lines
        .iter()
        .position(|line| line.contains("# skit:shim"))
        .unwrap();
    assert!(shim_idx > future_idx, "{out}");
}

// ==================================================================================
// shim: _insert_preamble appends newline when last line has none
// ==================================================================================

#[test]
fn test_preamble_appends_newline_when_missing() {
    // A missing trailing newline must not cause the preamble to merge with a source line: the
    // preamble line stands alone (only the shim marker + its own code, no source prefix).
    let src = "from __future__ import annotations\nresult = input('val: ')\nprint(result)";
    let out = inject_values(
        "python",
        src,
        &[input_spec("input-1", 0)],
        &values(&[("input-1", "v")]),
    )
    .unwrap();
    let mut found = false;
    for line in out.lines() {
        if line.contains("# skit:shim") {
            let stripped = line.trim();
            assert!(
                stripped.starts_with("import") || stripped.starts_with("_skit"),
                "{stripped}"
            );
            found = true;
            break;
        }
    }
    assert!(found, "No shim preamble line found in output: {out}");
}

// ==================================================================================
// store: _unique_slug with multiple collisions
// ==================================================================================

#[test]
fn test_unique_slug_multiple_collisions() {
    // When base and base-2 are both taken, the next slug must be base-3. Three distinct display
    // names that all slugify to "hello" exercise the private `_unique_slug` behaviorally.
    let data = TempDir::new().unwrap();
    let store = FileStore::new(data.path());
    let first = store.create(command_create("hello", "echo a")).unwrap();
    let second = store.create(command_create("hello!", "echo b")).unwrap();
    let third = store.create(command_create("hello?", "echo c")).unwrap();
    assert_eq!(first.slug.as_str(), "hello");
    assert_eq!(second.slug.as_str(), "hello-2");
    assert_eq!(third.slug.as_str(), "hello-3");
}

// ==================================================================================
// store: update_dependencies reference mode (no PEP 723 sync)
// ==================================================================================

#[test]
fn test_update_dependencies_reference_mode() {
    // A reference-mode Python entry records deps in meta only; the original file must not be
    // touched (there is no stored copy to sync a PEP 723 block into).
    let data = TempDir::new().unwrap();
    let source_dir = TempDir::new().unwrap();
    let script = source_dir.path().join("tool.py");
    fs::write(&script, "print('hi')\n").unwrap();
    let store = FileStore::new(data.path());
    let created = store
        .create(python_reference_create("tool", script.to_str().unwrap()))
        .unwrap();
    let mut settings = EntrySettings::from_meta(&created.meta);
    settings.dependencies = vec!["httpx".to_owned()];
    let updated = store
        .update_settings(&created, &settings, &created.meta.workdir)
        .unwrap();
    assert_eq!(
        EntrySettings::from_meta(&updated.meta).dependencies,
        ["httpx"]
    );
    // The original file must not be touched.
    assert!(!fs::read_to_string(&script).unwrap().contains("httpx"));
}

// ==================================================================================
// store: update_dependencies exe entry (not python, no PEP 723 sync)
// ==================================================================================

#[test]
fn test_update_dependencies_exe_entry() {
    // An exe entry records deps in meta only (no source to sync).
    let data = TempDir::new().unwrap();
    let source_dir = TempDir::new().unwrap();
    let exe = source_dir.path().join("tool");
    fs::write(&exe, b"").unwrap();
    let store = FileStore::new(data.path());
    let created = store
        .create(exe_reference_create("tool", exe.to_str().unwrap()))
        .unwrap();
    let mut settings = EntrySettings::from_meta(&created.meta);
    settings.dependencies = vec!["libssl".to_owned()];
    let updated = store
        .update_settings(&created, &settings, &created.meta.workdir)
        .unwrap();
    assert_eq!(
        EntrySettings::from_meta(&updated.meta).dependencies,
        ["libssl"]
    );
}

// ==================================================================================
// launcher: find_uv private-bin .exe variant (Windows path)
// ==================================================================================

#[test]
#[ignore = "UNMAPPED (cross-crate): uv discovery is not a public skit-runtime function. The PATH \
-> managed-bin fallback is in crates/skit-cli/src/run/command.rs (managed_uv_path + \
ensure_managed_uv), and managed_uv_path picks `uv` vs `uv.exe` at COMPILE time (cfg!(windows)), so \
the runtime `try uv, then uv.exe` probe the oracle tests cannot exist. Oracle ref: \
src/skit/langs/launch.py:36-47."]
fn test_find_uv_private_bin_exe_variant() {
    // Oracle behavior (compile-time in Rust): with PATH empty and only `bin/uv.exe` present,
    // find_uv() returns the `.exe` path.
}

// ==================================================================================
// launcher: _build_python with only requires_python (no deps)
// ==================================================================================

#[test]
fn test_build_python_only_requires_python() {
    // A python entry with a requires-python constraint but no deps passes `--python <constraint>`
    // and no `--with`.
    let mut python = entry("python");
    python.meta.workdir = "invoke".to_owned();
    EntrySettings {
        requires_python: ">=3.11".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut python.meta);
    let script = "/data/scripts/demo/script.py";
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("uv".to_owned(), PathBuf::from("/fake/uv"));

    let plan = build_launch_plan(
        &python,
        &launch_paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert!(plan.args.iter().any(|arg| arg == "--python"));
    assert!(plan.args.iter().any(|arg| arg == ">=3.11"));
    assert!(!plan.args.iter().any(|arg| arg == "--with"));
}

// ==================================================================================
// uvman: ensure_uv_downloaded full success path (mocked network)
// ==================================================================================

#[test]
#[ignore = "UNMAPPED (cross-crate): white-box monkeypatch of urlopen / copyfileobj / _extract_uv / \
_verify_checksum. The download success path is skit_runtime::ensure_managed_uv (uv.rs); its \
dedicated port is crates/skit-runtime/tests/port_test_uvman.rs. Oracle ref: \
src/skit/uvman.py ensure_uv_downloaded."]
fn test_ensure_uv_downloaded_success() {
    // Oracle behavior (cross-crate): the success path prints progress, downloads/extracts, and
    // returns a path whose basename is the platform uv executable name.
}
