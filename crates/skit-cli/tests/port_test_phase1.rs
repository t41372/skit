//! Mechanical port of the Python oracle module `tests/test_phase1.py`
//! (`origin/main@206f9ef`): "Phase 1 — PEP 723 completion, parameter persistence, command
//! placeholders, uv download URL." Each `#[test]` keeps its Python `def test_*` name so it traces
//! back to its origin, and each Python "WHY" comment is preserved above it.
//!
//! This oracle module is a cross-cutting smoke suite: it drives FIVE flat Python modules that the
//! Rust rewrite split across four crates, so the port lives in the composition-root crate
//! (`skit-cli-rs`) — the only crate whose dependency graph reaches `skit-language`, `skit-store`,
//! `skit-runtime`, and `skit-application` at once. The five concerns are driven through their real
//! public surfaces:
//!
//! Concept mapping used throughout:
//! - Python `pep723.parse_block(text)` -> `read_uv_metadata(text)` (its `dependencies` /
//!   `requires_python` fields cover every assertion these tests make on the parsed dict).
//! - Python `pep723.has_block(text)` -> `has_uv_metadata_block(text)`.
//! - Python `pep723.suggest_dependencies(text, script_dir=…)` -> `external_dependencies("python",
//!   text)` / `external_dependencies_at("python", text, script_dir)` (import scan + the
//!   import-name -> PyPI-distribution map at `skit-language/src/lib.rs:1163`, sorted + deduped).
//! - Python `pep723.inject_block(text, deps, req)` -> `write_uv_metadata(text, &deps, req)` for the
//!   INJECTION half (the exemplar-blessed mapping; see `port_test_metawriter.rs`). The oracle's
//!   final IDEMPOTENCY assertion (`inject_block(out, …) == out` when a block already exists) has no
//!   idempotent standalone function in the rewrite: the injection is folded into `write_uv_metadata`
//!   (the `set_dependencies` analog — always replaces), and the "don't re-inject when a block
//!   exists" guard is `has_uv_metadata_block_bytes` at the add orchestration (skit-ui/add.rs:519,
//!   skit-cli/cli.rs:2912). So `test_inject_block_roundtrip` asserts the injection round-trip AND
//!   asserts `has_uv_metadata_block(&out)` — the exact predicate that makes the oracle's idempotency
//!   hold — with the end-to-end no-clobber outcome covered by `test_add_python_existing_block_not_touched`.
//! - Python `store.add_python(src, …, dependencies=…, requires_python=…)` (copy/reference
//!   orchestration: copy the file, inject the PEP 723 block into the copy, or record the flags in
//!   meta) -> the real `skit add <path> [--ref] --dep … --python …` binary via `assert_cmd`, then
//!   read the stored `scripts/<slug>/script.py` bytes and `meta.toml` text. This mirrors the
//!   established `port_test_uv_metadata_views.rs` convention.
//! - Python `store.extract_placeholders(template)` -> `placeholder_params("command", template)`
//!   (mapped to its `.name`s).
//! - Python `launcher.build_command(entry, values=…)` -> `build_launch_plan(entry, paths, assembly,
//!   …, probe)` returning a `LaunchPlan { program, args, … }`. Python monkeypatches
//!   `skit.langs.launch.find_uv`; the Rust seam is the `ProgramProbe` (a `FakeProbe` supplies uv/sh
//!   and the filesystem facts). A command template's `{name}` values ride `Assembly.command_values`.
//! - Python `launcher.LaunchError` (missing template value) -> `LaunchError::MissingTemplateValue`.
//! - Python `argstate.load_state/save_last/forget` (free functions over `<state_dir>/values/<slug>.toml`)
//!   -> `FormStateService<FileFormStateStore>` rooted at a `TempDir`. The Rust `save_last` folds
//!   `flows.remembered_values` in, so a stored value needs a matching non-secret `ParamDecl` (the
//!   oracle's lower-level `save_last` stored the dict verbatim). `store.remove` also clearing the
//!   state file is wired in the composition root (cli.rs:3214), so that test drives the real binary.
//! - Python `uvman.download_url(triple)` -> `uv_asset(&target, mirror_base).url`; `uvman.UV_VERSION`
//!   -> `skit_runtime::UV_VERSION` (deliberately bumped 0.11.26 -> 0.12.3, like `port_test_uvman.rs`);
//!   `uvman._triple()` (host resolution) -> `UvTarget::current()`; `uvman.ensure_uv_downloaded(quiet=True)`
//!   -> `ensure_managed_uv(data_dir, mirror_base)` + `managed_uv_path(data_dir)`.
//!
//! Buckets: every one of the 27 Python defs is a REAL asserting `#[test]` driving the reachable
//! public API. No cross-crate stubs, no absent gaps, no divergence-ignores.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skit_application::{delivery::Assembly, form_state::FormStateService};
use skit_domain::{
    Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType},
};
use skit_language::{
    external_dependencies, external_dependencies_at, has_uv_metadata_block, placeholder_params,
    read_uv_metadata, write_uv_metadata,
};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, UV_VERSION, UvTarget, build_launch_plan,
    ensure_managed_uv, managed_uv_path, uv_asset,
};
use skit_store::FileFormStateStore;
use tempfile::TempDir;

// ===========================================================================
// The oracle's module-level BLOCK fixture (a strict-UTF-8 PEP 723 script).
// ===========================================================================
const BLOCK: &str = "# /// script\n# requires-python = \">=3.11\"\n# dependencies = [\n#     \"requests\",\n# ]\n# ///\nimport requests\nprint(requests.__version__)\n";

// ---------------------------------------------------------------------------
// pep723 — read/has (skit-language, pure functions)
// ---------------------------------------------------------------------------

#[test]
fn test_parse_block() {
    let meta = read_uv_metadata(BLOCK).expect("metadata block present");
    assert_eq!(meta.dependencies, ["requests"]);
    assert_eq!(meta.requires_python, ">=3.11");
}

#[test]
fn test_parse_no_block() {
    assert!(read_uv_metadata("print('hi')\n").is_none());
    assert!(!has_uv_metadata_block("print('hi')\n"));
}

// ---------------------------------------------------------------------------
// pep723 — suggest_dependencies (skit-language import scan + PyPI-name map)
// ---------------------------------------------------------------------------

#[test]
fn test_suggest_dependencies() {
    let text = "import requests\nimport os\nfrom rich.table import Table\nimport mymod.sub\n";
    let got = external_dependencies("python", text);
    assert!(got.iter().any(|name| name == "requests"));
    assert!(got.iter().any(|name| name == "rich"));
    assert!(!got.iter().any(|name| name == "os")); // stdlib excluded
}

#[test]
fn test_suggest_syntax_error_returns_empty() {
    assert!(external_dependencies("python", "def broken(:\n").is_empty());
}

#[test]
fn test_suggest_dependencies_maps_import_name_to_pypi_package() {
    // The failure the mapping fixes: `from PIL import Image` must suggest the installable `Pillow`,
    // not the import name `PIL` (which uv can't resolve).
    assert_eq!(
        external_dependencies("python", "from PIL import Image\n"),
        ["Pillow"]
    );
    assert_eq!(
        external_dependencies("python", "import cv2\n"),
        ["opencv-python"]
    );
    assert_eq!(external_dependencies("python", "import yaml\n"), ["PyYAML"]);
}

#[test]
fn test_suggest_dependencies_dedupes_after_mapping() {
    // Two imports that collapse onto the same distribution must appear once.
    let src = "from Crypto.Cipher import AES\nimport Crypto.Hash\n";
    assert_eq!(external_dependencies("python", src), ["pycryptodome"]);
}

#[test]
fn test_suggest_dependencies_unmapped_name_unchanged() {
    // Names not in the table pass through verbatim (we only rewrite the ones we're sure about).
    assert_eq!(
        external_dependencies("python", "import requests\n"),
        ["requests"]
    );
}

#[test]
fn test_suggest_dependencies_excludes_sibling_py_module() {
    // A bare `import helpers` next to a sibling `helpers.py` resolves to that local file at run
    // time (the script's own directory leads sys.path), so it must NOT be suggested as a PyPI
    // dependency; the genuine third-party import beside it still is.
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("helpers.py"), "X = 1\n").unwrap();
    let src = "import helpers\nimport requests\n";
    assert_eq!(
        external_dependencies_at("python", src, Some(dir.path())),
        ["requests"]
    );
}

#[test]
fn test_suggest_dependencies_excludes_sibling_package_dir() {
    // A sibling `helpers/` PACKAGE shadows any same-named distribution too. It needs real Python in
    // it to do that: an empty directory is only a PEP 420 namespace portion, which never wins over
    // an installed regular package.
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("helpers")).unwrap();
    fs::write(dir.path().join("helpers").join("__init__.py"), "x = 1\n").unwrap();
    let src = "import helpers\nimport requests\n";
    assert_eq!(
        external_dependencies_at("python", src, Some(dir.path())),
        ["requests"]
    );
}

#[test]
fn test_suggest_dependencies_keeps_name_without_a_sibling() {
    // No sibling of that name in the directory -> still a real suggestion (the exclusion is scoped
    // to names that actually resolve locally).
    let dir = TempDir::new().unwrap();
    assert_eq!(
        external_dependencies_at("python", "import helpers\n", Some(dir.path())),
        ["helpers"]
    );
}

#[test]
fn test_suggest_dependencies_default_script_dir_none_does_not_filter() {
    // The old behavior, pinned: with no script_dir (the default), nothing is treated as local, so a
    // name that WOULD be a sibling elsewhere is suggested here.
    assert_eq!(
        external_dependencies("python", "import helpers\n"),
        ["helpers"]
    );
}

#[test]
fn test_suggest_dependencies_from_import_sibling_excluded() {
    // `from helpers import x` (level 0) resolves to the sibling `helpers.py` exactly as a plain
    // `import helpers` does, so it is excluded the same way.
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("helpers.py"), "x = 1\n").unwrap();
    let src = "from helpers import x\nimport requests\n";
    assert_eq!(
        external_dependencies_at("python", src, Some(dir.path())),
        ["requests"]
    );
}

#[test]
fn test_suggest_dependencies_submodule_of_sibling_dir_excluded() {
    // `import helpers.sub` splits to the top-level `helpers`, which is the sibling package dir.
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("helpers")).unwrap();
    fs::write(dir.path().join("helpers").join("sub.py"), "x = 1\n").unwrap();
    let src = "import helpers.sub\nimport requests\n";
    assert_eq!(
        external_dependencies_at("python", src, Some(dir.path())),
        ["requests"]
    );
}

// ---------------------------------------------------------------------------
// pep723 — inject_block (skit-language write_uv_metadata; see module doc)
// ---------------------------------------------------------------------------

#[test]
fn test_inject_block_roundtrip() {
    let src = "#!/usr/bin/env python3\nimport requests\n";
    let out = write_uv_metadata(src, &["requests".to_owned()], ">=3.10").unwrap();
    assert!(out.starts_with("#!/usr/bin/env python3\n# /// script\n"));
    let meta = read_uv_metadata(&out).expect("metadata block present");
    assert_eq!(meta.dependencies, ["requests"]);
    assert_eq!(meta.requires_python, ">=3.10");
    // Idempotent when a block already exists: the oracle's `inject_block(out, …) == out` holds
    // because `has_block(out)` is true, and the add path only injects when the predicate is false
    // (skit-cli/cli.rs:2912). The rewrite has no idempotent standalone injector — `write_uv_metadata`
    // is the `set_dependencies` analog (always replaces) — so the load-bearing assertion is exactly
    // this guard predicate. The end-to-end no-clobber outcome is in
    // test_add_python_existing_block_not_touched.
    assert!(has_uv_metadata_block(&out));
}

#[test]
fn test_inject_preserves_body() {
    let src = "import requests\nprint('x')\n";
    let out = write_uv_metadata(src, &["requests".to_owned()], "").unwrap();
    assert!(out.ends_with("import requests\nprint('x')\n"));
}

// ---------------------------------------------------------------------------
// store — copy injection vs reference meta (real `skit add` binary via assert_cmd)
// ---------------------------------------------------------------------------

/// A local `SKIT_*` fixture: three temporary directories, never the real user dirs, never a chdir.
/// Every `skit` invocation points DATA/STATE/CONFIG at fresh temp paths, matching the sibling
/// `port_test_uv_metadata_views.rs` harness so nothing lands in the repo or cwd.
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

    /// The single committed entry's slug (there is exactly one per test), read from `scripts/`.
    fn sole_slug(&self) -> String {
        let scripts = self.data.path().join("scripts");
        let mut names: Vec<String> = fs::read_dir(&scripts)
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.unwrap();
                entry
                    .file_type()
                    .unwrap()
                    .is_dir()
                    .then(|| entry.file_name().to_string_lossy().into_owned())
            })
            .collect();
        assert_eq!(
            names.len(),
            1,
            "expected exactly one stored entry: {names:?}"
        );
        names.remove(0)
    }

    fn meta_text(&self, slug: &str) -> String {
        fs::read_to_string(
            self.data
                .path()
                .join("scripts")
                .join(slug)
                .join("meta.toml"),
        )
        .unwrap()
    }

    fn script_text(&self, slug: &str) -> String {
        fs::read_to_string(
            self.data
                .path()
                .join("scripts")
                .join(slug)
                .join("script.py"),
        )
        .unwrap()
    }
}

#[test]
fn test_add_python_copy_injects_pep723() {
    // copy mode: dependency completion is written into the copy's PEP 723 block (comment-only, A5),
    // so the copy is portable; meta.toml is left blank on both axes (single source of truth), and
    // the original file must never be touched.
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("s.py");
    fs::write(&source, "import requests\nprint('hi')\n").unwrap();

    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "s",
            "--dep",
            "requests",
            "--python",
            ">=3.11",
        ])
        .assert()
        .success();

    let slug = sandbox.sole_slug();
    let stored = sandbox.script_text(&slug);
    let meta_in_copy = read_uv_metadata(&stored).expect("the copy carries a PEP 723 block");
    assert_eq!(meta_in_copy.dependencies, ["requests"]);
    // After injecting into the copy, meta.toml doesn't duplicate the info (single source of truth).
    let meta = sandbox.meta_text(&slug);
    assert!(!meta.contains("dependencies"), "{meta}");
    assert!(!meta.contains("requires_python"), "{meta}");
    // The original file must never be touched.
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        "import requests\nprint('hi')\n"
    );
}

#[test]
fn test_add_python_reference_records_in_meta() {
    // reference mode: never touch the original; record the deps + constraint in meta, and the
    // launcher passes them via --with/--python at run time.
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("s.py");
    fs::write(&source, "import requests\n").unwrap();

    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--ref",
            "--name",
            "s",
            "--dep",
            "requests",
            "--python",
            ">=3.11",
        ])
        .assert()
        .success();

    let slug = sandbox.sole_slug();
    let meta = sandbox.meta_text(&slug);
    // meta records both axes (no toml dev-dep here, so assert on the rendered keys + values).
    assert!(meta.contains("dependencies"), "{meta}");
    assert!(meta.contains("requests"), "{meta}");
    assert!(meta.contains("requires_python"), "{meta}");
    assert!(meta.contains(">=3.11"), "{meta}");
    // original untouched, and no stored copy exists for a reference entry.
    assert_eq!(fs::read_to_string(&source).unwrap(), "import requests\n");
    assert!(
        !self_script_path(&sandbox, &slug).exists(),
        "a reference entry stores no script copy"
    );
}

/// The path a copy-mode entry WOULD store its script at (absent for a reference entry).
fn self_script_path(sandbox: &Sandbox, slug: &str) -> PathBuf {
    sandbox
        .data
        .path()
        .join("scripts")
        .join(slug)
        .join("script.py")
}

#[test]
fn test_add_python_existing_block_not_touched() {
    // A source that already carries its own PEP 723 block: the stored copy keeps that original
    // block verbatim (never re-injected on top of it), so the block's deps stay `["requests"]`.
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("s.py");
    fs::write(&source, BLOCK).unwrap();

    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "s",
            "--dep",
            "other",
            "--python",
            ">=3.12",
        ])
        .assert()
        .success();

    let slug = sandbox.sole_slug();
    let stored = sandbox.script_text(&slug);
    let block_meta = read_uv_metadata(&stored).expect("the original block survives");
    assert_eq!(block_meta.dependencies, ["requests"]); // original block preserved
}

// ---------------------------------------------------------------------------
// launcher — --with / --python passthrough (skit-runtime build_launch_plan)
// ---------------------------------------------------------------------------

/// The `ProgramProbe` seam replaces the oracle's monkeypatching of `find_uv` / `shutil.which` and
/// the real filesystem (mirrors `port_test_launcher.rs`).
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

#[test]
fn test_build_command_reference_deps() {
    // Reference-mode deps and python constraint pass via --with/--python. The oracle asserts the
    // exact prefix `["/fake/uv", "run", "--no-project", "--python"]`; the Rust plan splits the
    // program out, so `program == /fake/uv` and `args[0..3] == ["run", "--no-project", "--python"]`.
    let mut py = entry("python");
    py.meta.mode = StorageMode::Reference;
    py.meta.workdir = "invoke".to_owned();
    EntrySettings {
        requires_python: ">=3.11".to_owned(),
        dependencies: vec!["requests".to_owned(), "rich".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut py.meta);
    let script = "/refsrc/s.py";
    let mut probe = FakeProbe {
        files: vec![PathBuf::from(script)],
        dirs: vec![PathBuf::from("/invoke")],
        executable: vec![PathBuf::from(script)],
        ..FakeProbe::default()
    };
    probe
        .programs
        .insert("uv".to_owned(), PathBuf::from("/fake/uv"));

    let plan = build_launch_plan(
        &py,
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert_eq!(plan.program, PathBuf::from("/fake/uv"));
    assert_eq!(plan.args[0..3], ["run", "--no-project", "--python"]);
    assert!(plan.args.iter().any(|arg| arg == ">=3.11"));
    assert_eq!(plan.args.iter().filter(|arg| *arg == "--with").count(), 2);
    assert!(plan.args.iter().any(|arg| arg == "--script"));
}

// ---------------------------------------------------------------------------
// command placeholders (skit-language placeholder_params + build_launch_plan)
// ---------------------------------------------------------------------------

/// The `{name}` placeholders of a command template, in stored order — the oracle's
/// `store.extract_placeholders` analog.
fn extract_placeholders(template: &str) -> Vec<String> {
    placeholder_params("command", template)
        .into_iter()
        .map(|declaration| declaration.name)
        .collect()
}

#[test]
fn test_extract_placeholders() {
    assert_eq!(
        extract_placeholders("ffmpeg -i {input} -o {output} {input}"),
        ["input", "output"]
    );
    assert_eq!(extract_placeholders("echo {{literal}} {x}"), ["x"]);
    assert!(extract_placeholders("echo plain").is_empty());
}

/// A command entry whose template is `template`, workdir `invoke`, and whose declared placeholder
/// list is the template's extracted placeholders — exactly what `store.add_command` stores (the
/// launcher's missing-value check reads `settings.params`, its analog of the oracle's `meta.params`).
fn command_entry(template: &str) -> Entry {
    let mut command = entry("command");
    command.meta.workdir = "invoke".to_owned();
    EntrySettings {
        template: template.to_owned(),
        params: extract_placeholders(template),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut command.meta);
    command
}

fn command_probe() -> FakeProbe {
    let mut probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };
    probe
        .programs
        .insert("sh".to_owned(), PathBuf::from("/bin/sh"));
    probe
}

#[test]
fn test_command_params_fill_and_escape() {
    // The template's placeholders become the entry's params, and filling them renders the shell
    // string with `{{braces}}` restored to a literal `{braces}`. (The oracle's `entry.meta.params`
    // is `add_command`'s extraction — asserted here via `extract_placeholders`, its real source.)
    let template = "convert {src} to {dst} keep {{braces}}";
    assert_eq!(extract_placeholders(template), ["src", "dst"]);

    let command = command_entry(template);
    let assembly = Assembly {
        command_values: BTreeMap::from([
            ("src".to_owned(), "a.png".to_owned()),
            ("dst".to_owned(), "b.jpg".to_owned()),
        ]),
        masked_command_values: BTreeMap::from([
            ("src".to_owned(), "a.png".to_owned()),
            ("dst".to_owned(), "b.jpg".to_owned()),
        ]),
        ..Assembly::default()
    };

    let plan = build_launch_plan(
        &command,
        &paths("/unused"),
        &assembly,
        None,
        None,
        &command_probe(),
    )
    .unwrap();
    // Rust lowers a command template to `sh -c "<rendered>"`, so the rendered text is `args[1]`.
    assert_eq!(plan.args[0], "-c");
    assert_eq!(plan.args[1], "convert a.png to b.jpg keep {braces}");
}

#[test]
fn test_command_missing_values_raises() {
    // A command template with an unfilled `{x}` refuses before spawn (LaunchError), never renders a
    // half-filled command line.
    let command = command_entry("echo {x}");
    let error = build_launch_plan(
        &command,
        &paths("/unused"),
        &Assembly::default(),
        None,
        None,
        &command_probe(),
    )
    .unwrap_err();
    assert!(
        matches!(error, LaunchError::MissingTemplateValue { ref name } if name == "x"),
        "{error:?}"
    );
}

// ---------------------------------------------------------------------------
// argstate (skit-application FormStateService over skit-store FileFormStateStore)
// ---------------------------------------------------------------------------

fn state_service(root: &TempDir) -> FormStateService<FileFormStateStore> {
    FormStateService::new(FileFormStateStore::new(root.path()))
}

/// A declared, non-secret `ParamDecl` — the oracle's `argstate.save_last` stored a bare dict, so a
/// value only survives the rewrite's `remembered_values` fold when it names a live declaration.
fn plain_spec(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

fn value_map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn test_argstate_roundtrip_and_forget() {
    let root = TempDir::new().unwrap();
    let service = state_service(&root);

    assert!(
        service
            .load(&Slug::parse("nope").unwrap())
            .values
            .is_empty()
    );

    let slug = Slug::parse("s1").unwrap();
    service
        .save_last(
            &slug,
            &[plain_spec("x")],
            Some(&value_map(&[("x", "1")])),
            Some(vec!["--fast".to_owned()]),
            false,
        )
        .unwrap();
    let got = service.load(&slug);
    assert_eq!(got.values, value_map(&[("x", "1")]));
    assert_eq!(got.extra_args, ["--fast"]);

    service.forget(&slug).unwrap();
    assert!(service.load(&slug).values.is_empty());
    service.forget(&slug).unwrap(); // idempotent
}

#[test]
fn test_remove_clears_argstate() {
    // Removing an entry drops its last-used values too: `store.remove` -> `argstate.forget` is
    // wired in the composition root (skit-cli/cli.rs:3214), so this drives the real binary.
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("s.py");
    fs::write(&source, "print('hi')\n").unwrap();
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "s"])
        .assert()
        .success();

    let slug = sandbox.sole_slug();
    // Seed a saved last-used state file, as `argstate.save_last(slug, extra_args=["--x"])` would.
    let values_dir = sandbox.state.path().join("values");
    fs::create_dir_all(&values_dir).unwrap();
    let values_file = values_dir.join(format!("{slug}.toml"));
    fs::write(&values_file, "extra_args = [\"--x\"]\n").unwrap();

    sandbox
        .command()
        .args(["remove", "s", "--yes"])
        .assert()
        .success();

    assert!(
        !values_file.exists(),
        "remove must clear the entry's saved state"
    );
}

// ---------------------------------------------------------------------------
// uvman — private-uv bootstrap (skit-runtime, no network)
// ---------------------------------------------------------------------------

#[test]
fn test_uv_download_url_shape() {
    let target = UvTarget::from_parts("x86_64", "linux", false).unwrap();
    let url = uv_asset(&target, None).unwrap().url;
    assert!(url.starts_with("https://github.com/astral-sh/uv/releases/download/"));
    assert!(url.contains(UV_VERSION)); // the pinned version appears in the URL
    assert!(url.ends_with("uv-x86_64-unknown-linux-gnu.tar.gz"));

    let windows = UvTarget::from_parts("x86_64", "windows", false).unwrap();
    assert!(uv_asset(&windows, None).unwrap().url.ends_with(".zip"));
}

#[test]
fn test_uv_triple_current_platform() {
    // The current CI / sandbox platform must be resolvable (no bootstrap error).
    let triple = UvTarget::current().unwrap().triple().to_owned();
    assert!(
        ["linux", "darwin", "windows"]
            .iter()
            .any(|k| triple.contains(k))
    );
}

#[test]
fn test_ensure_uv_downloaded_skips_when_present() {
    // If the binary is already present, skip download and return its path immediately (a dead
    // mirror proves the network is never touched). The oracle asserts against `private_bin_dir()`;
    // the rewrite roots the managed uv under an explicit data dir (`managed_uv_path`).
    let data_dir = TempDir::new().unwrap();
    let dest = managed_uv_path(data_dir.path());
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "#!/bin/sh\n").unwrap();
    assert_eq!(
        ensure_managed_uv(data_dir.path(), Some("http://127.0.0.1:1/dead")).unwrap(),
        dest
    );
}
