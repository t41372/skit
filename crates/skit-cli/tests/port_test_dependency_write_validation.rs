//! Mechanical port of the Python oracle module `tests/test_dependency_write_validation.py`
//! (`origin/main@206f9ef`): "Dependency-write validation and draft-refusal contracts
//! (exit codes, exact refusal copy, filesystem/meta state, stored PEP 723 text, the
//! store's validate-then-write chokepoint, the suggest-dependencies self-fabrication
//! filter)." Each `#[test]` keeps its Python `def test_*` name and its WHY comment.
//!
//! WHY skit-cli-rs: the oracle drives `cli.app` (the `deps` and `add` commands) end to
//! end AND calls `store.*` / `pep723.*` directly. The Rust rewrite DISPERSES the Python
//! `store._validate_uv_metadata` "one chokepoint" into the composition root: the CLI
//! `deps` handler (cli.rs:3448-3465) and the `add` handler (cli.rs:2943-2952) each call
//! `validate_pep508_requirement` / `validate_pep440_specifiers` themselves before writing.
//! There is NO single store function that validates-then-writes, so the store-tier tests
//! (section 2) are driven through the CLI, whose `deps` command IS that chokepoint. Only
//! `skit-cli-rs` can spawn the real `skit` binary AND depends on `skit-language`, so the
//! port lives here.
//!
//! Concept mapping used throughout:
//! - Python `cli.app` (`deps` / `add`) -> the real `skit` binary via `assert_cmd`.
//! - Python `store.add_python(src, name="a")` -> `skit add <src> --kind python --name a --no-input`.
//! - Python `store.add_script(js, kind="js", name="jsx")` -> `skit add <js> --kind js --name jsx --no-input`.
//! - Python `store.update_dependencies(slug, deps, requires_python=…)` -> `skit deps slug --dep … --python …`
//!   (the Rust `deps` command composes the validate-then-write the Python store did in one call;
//!   the `-`/`none` -> "" normalization is at cli.rs:3454, the strip-and-drop at cli.rs:3402).
//! - Python `store.update_needs(slug, ["jq"])` -> `skit deps slug --need jq`.
//! - Python `pep723.suggest_dependencies(text)` -> `skit_language::external_dependencies("python", text)`
//!   (both scan imports, map to the distribution name, drop stdlib/underscore/local, and FILTER
//!   the result through the PEP 508 check; the Rust scanner collects into a `BTreeSet`, matching
//!   the oracle's `sorted(...)`).
//! - Python `pep723.requirement_error(s) is None` -> `validate_pep508_requirement(s).is_ok()`.
//! - Python `_stored_block(slug)` -> read `<SKIT_DATA_DIR>/scripts/<slug>/script.py`.
//! - Python `store.resolve(slug).meta.*` -> read the raw `<SKIT_DATA_DIR>/scripts/<slug>/meta.toml`.
//!   skit OMITS an empty-string field, so `meta.requires_python == ""` ports to
//!   `!meta.contains("requires_python =")` and `meta.dependencies is None` ports to a byte-exact
//!   before/after `meta.toml` (same rule as `port_test_uv_metadata_unpinning.rs`).
//! - Python `_flat(result.output)` -> collapse ASCII whitespace over combined stdout+stderr.
//! - Python `drafts_dir()` / `is_draft(path)` -> `<SKIT_DATA_DIR>/drafts` + the `skit-` name prefix
//!   (Rust `is_owned_draft`, cli.rs:5803).
//!
//! Buckets (recorded per test in the structured result):
//! - REAL asserting `#[test]` (API exists, behavior agrees): the unpin/preserve/clear/valid deps
//!   writes, the deps-before-needs abort order, the npm-skip, the whitespace strip-and-drop, the
//!   `-`/`none` normalization, the oracle PEP 440/508 refusal copy, and the suggest filter +
//!   no-block add.
//! - DIVERGENCE (full asserting body, `#[ignore]`d): an unclassifiable file outside the drafts
//!   directory gets the generic `pass --kind KIND` refusal instead of the complete oracle escape.
//! - ABSENT / GAP (full asserting body, `#[ignore]`d, MUST-FIX): the drafts guard and the
//!   draft-aware "can't classify" variant (Python cli.py:1894-1933, cli.py:2053-2066) are not
//!   built in `add_with_config`, so a draft added as exe/ref SUCCEEDS instead of being refused,
//!   and a shebang-less draft gets the generic `could not infer the entry kind` message. Pending
//!   task #15 ("Refuse the add-lane inputs version 0.4 refuses") corroborates this is a known gap,
//!   not a mistranslation. The bodies are full assertions (the CLI call compiles), not stubs.

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use assert_cmd::Command;
use serde_json::Value;
use skit_language::{external_dependencies, validate_pep508_requirement};
use tempfile::TempDir;

/// A self-contained sandbox: isolated SKIT_* dirs and HOME, driving the real `skit` binary.
struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    src: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            src: TempDir::new().unwrap(),
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
            .current_dir(self.home.path());
        command
    }

    /// Run `skit <args>` and return the raw process output (exit code + streams).
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    /// Run `skit <args>`, assert exit 0, and return stdout as text.
    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(args)).unwrap()
    }

    /// Python `_py(tmp_path, body, name)`: write a source file OUTSIDE the store dirs.
    fn write_source(&self, name: &str, body: &str) -> String {
        let path = self.src.path().join(name);
        fs::write(&path, body).unwrap();
        path.to_str().unwrap().to_owned()
    }

    /// Python `_draft(name, body)`: write a kept draft into `<SKIT_DATA_DIR>/drafts`.
    fn draft(&self, name: &str, body: &str) -> PathBuf {
        let drafts = self.data.path().join("drafts");
        fs::create_dir_all(&drafts).unwrap();
        let path = drafts.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    /// Python `_stored_block(slug)`: the stored copy's `script.py` text.
    fn stored_block(&self, slug: &str) -> String {
        fs::read_to_string(self.data.path().join(format!("scripts/{slug}/script.py"))).unwrap()
    }

    /// The raw stored `meta.toml` text (for `store.resolve(slug).meta.*`).
    fn stored_meta(&self, slug: &str) -> String {
        fs::read_to_string(self.data.path().join(format!("scripts/{slug}/meta.toml"))).unwrap()
    }

    /// Add a fresh copy-mode python entry named `slug` from `print(1)\n`.
    fn add_python_print(&self, slug: &str) {
        let src = self.write_source(&format!("{slug}.py"), "print(1)\n");
        self.ok(&[
            "add",
            &src,
            "--kind",
            "python",
            "--name",
            slug,
            "--no-input",
        ]);
    }
}

/// Python `_flat(text)`: collapse every run of whitespace to one space.
fn flat(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ==========================================================================
// 1. `skit deps` validate-then-write (the CLI face of the store chokepoint)
// ==========================================================================

#[test]
fn test_deps_garbage_dep_is_refused_and_nothing_changes() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_source(
        "s.py",
        "import requests\n# /// script\n# dependencies = [\"rich\"]\n# ///\nprint(1)\n",
    );
    sandbox.ok(&["add", &src, "--kind", "python", "--name", "a", "--no-input"]);
    let before_block = sandbox.stored_block("a");
    let before_meta = sandbox.stored_meta("a");
    let output = sandbox.run(&["deps", "a", "--dep", "@@@"]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    assert!(flat(&output).contains("isn't a package requirement"));
    assert_eq!(sandbox.stored_meta("a"), before_meta); // meta untouched
    assert_eq!(sandbox.stored_block("a"), before_block); // block untouched (no partial write)
}

#[test]
fn test_deps_garbage_python_is_refused_and_nothing_changes() {
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    sandbox.ok(&["deps", "a", "--dep", "requests", "--python", ">=3.11"]);
    let before_block = sandbox.stored_block("a");
    let output = sandbox.run(&["deps", "a", "--python", "not-a-version"]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    assert!(flat(&output).contains("isn't a Python version constraint"));
    // requires_python unchanged (still pinned to >=3.11).
    assert!(
        sandbox
            .stored_meta("a")
            .contains("requires_python = \">=3.11\"")
    );
    assert_eq!(sandbox.stored_block("a"), before_block);
}

#[test]
fn test_deps_dash_python_clears_meta_and_unpins_the_block() {
    // '-' means automatic AND an explicit unpin. meta clears to "", the block's requires-python
    // line is REMOVED, `--json` agrees, and the dry-run command carries no `--python`.
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    sandbox.ok(&["deps", "a", "--dep", "requests", "--python", ">=3.11"]);
    assert!(
        sandbox
            .stored_block("a")
            .contains("requires-python = \">=3.11\"")
    ); // pinned first
    sandbox.ok(&["deps", "a", "--python", "-"]);
    assert!(!sandbox.stored_meta("a").contains("requires_python =")); // meta cleared
    assert!(!sandbox.stored_block("a").contains("requires-python")); // block unpinned — line gone
    assert_eq!(
        sandbox.json(&["deps", "a", "--json"])["requires_python"],
        "" // --json agrees (no stale pin)
    );
    let dry = sandbox.run(&["run", "a", "--dry-run", "--no-input"]);
    assert_eq!(dry.status.code(), Some(0), "{}", flat(&dry));
    assert!(!flat(&dry).contains("--python")); // the run uv would launch carries no constraint
}

#[test]
fn test_deps_only_edit_still_preserves_the_blocks_own_pin() {
    // A DEPS-ONLY edit (no --python flag) still PRESERVES the block's own requires-python.
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    sandbox.ok(&["deps", "a", "--dep", "requests", "--python", ">=3.11"]);
    sandbox.ok(&["deps", "a", "--dep", "rich"]);
    assert!(
        sandbox
            .stored_block("a")
            .contains("requires-python = \">=3.11\"")
    ); // block pin preserved
    assert!(sandbox.stored_block("a").contains("rich")); // and the deps edit landed
}

#[test]
fn test_deps_none_python_clears_meta_when_nothing_to_preserve() {
    // 'none' is the other automatic token: meta clears to "" and, with no prior block constraint
    // to preserve, the block carries none either.
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    sandbox.ok(&["deps", "a", "--dep", "requests"]); // deps only, no python
    sandbox.ok(&["deps", "a", "--dep", "requests", "--python", "none"]);
    assert!(!sandbox.stored_meta("a").contains("requires_python ="));
    assert!(!sandbox.stored_block("a").contains("requires-python"));
}

#[test]
fn test_deps_valid_dep_and_python_still_write() {
    // A valid requirement + a valid constraint pass the validator and land in meta and the block.
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    sandbox.ok(&["deps", "a", "--dep", "requests>=2,<3", "--python", "~=3.12"]);
    let meta = sandbox.stored_meta("a");
    assert!(meta.contains("\"requests>=2,<3\""));
    assert!(meta.contains("requires_python = \"~=3.12\""));
    let block = sandbox.stored_block("a");
    assert!(block.contains("requests>=2,<3"));
    assert!(block.contains("requires-python = \"~=3.12\""));
}

#[test]
fn test_deps_refused_write_leaves_needs_untouched() {
    // The deps-before-needs abort order: a --dep refusal raises before ANY write, so a --need in
    // the same call never lands (a partial apply a --json/CI caller couldn't detect).
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    sandbox.ok(&["deps", "a", "--need", "jq"]);
    let output = sandbox.run(&["deps", "a", "--dep", "@@@", "--need", "ffmpeg"]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    let meta = sandbox.stored_meta("a");
    assert!(meta.contains("\"jq\"")); // the needs write never ran
    assert!(!meta.contains("ffmpeg"));
}

#[test]
fn test_deps_npm_entry_takes_an_npm_shaped_dep_that_fails_pep508() {
    // An npm-flavor (js) entry is NOT routed through the PEP 508 validator: a scoped package
    // (`@scope/thing` — which the PEP 508 check rejects) is accepted, because the npm installer
    // owns that grammar, not skit's validator.
    let sandbox = Sandbox::new();
    let js = sandbox.write_source("t.js", "import x from \"@scope/thing\";\nconsole.log(x)\n");
    sandbox.ok(&["add", &js, "--kind", "js", "--name", "jsx", "--no-input"]);
    assert!(validate_pep508_requirement("@scope/thing").is_err()); // would fail if validated
    sandbox.ok(&["deps", "jsx", "--dep", "@scope/thing"]);
    assert_eq!(
        sandbox.json(&["deps", "jsx", "--json"])["dependencies"],
        serde_json::json!(["@scope/thing"])
    );
}

// ==========================================================================
// 2. store._validate_uv_metadata via the public update_dependencies
//    (in Rust the chokepoint is the CLI `deps` command; see the file doc)
// ==========================================================================

#[test]
fn test_update_dependencies_uv_invalid_dep_raises_usage_error() {
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    let before_meta = sandbox.stored_meta("a");
    let output = sandbox.run(&["deps", "a", "--dep", "@@@"]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    assert!(flat(&output).contains("isn't a package requirement"));
    assert_eq!(sandbox.stored_meta("a"), before_meta); // nothing written
}

#[test]
fn test_update_dependencies_uv_invalid_python_raises_usage_error() {
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    let output = sandbox.run(&[
        "deps",
        "a",
        "--dep",
        "requests",
        "--python",
        "not-a-version",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    assert!(flat(&output).contains("isn't a Python version constraint"));
}

#[test]
fn test_update_dependencies_drops_a_whitespace_only_dep_at_the_chokepoint() {
    // The chokepoint strip-and-drops empty entries BEFORE validating or writing: a whitespace-only
    // requirement is "nothing", never recorded — the valid neighbour commits alone (in Rust the
    // strip lives in the `deps` handler, cli.rs:3402; the observable result is identical).
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    sandbox.ok(&["deps", "a", "--dep", "  ", "--dep", "requests"]);
    let meta = sandbox.stored_meta("a");
    assert!(meta.contains("\"requests\"")); // dropped, not tolerated
    assert!(!meta.contains("\"  \""));
    let block = sandbox.stored_block("a");
    assert!(block.contains("requests"));
    assert!(!block.contains("\"  \"")); // the whitespace entry never reached the PEP 723 block
}

#[test]
fn test_update_dependencies_all_whitespace_list_clears_deps() {
    // A list of nothing-but-whitespace strip-and-drops to empty, which clears the record — the
    // meta dependencies key is gone and the block's dependencies are emptied.
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    sandbox.ok(&["deps", "a", "--dep", "requests"]);
    sandbox.ok(&["deps", "a", "--dep", "   ", "--dep", "\t"]);
    let meta = sandbox.stored_meta("a");
    assert!(!meta.contains("dependencies")); // cleared
    let block = sandbox.stored_block("a");
    assert!(block.contains("dependencies = []"));
    assert!(!block.contains("requests"));
}

#[test]
fn test_update_dependencies_npm_flavor_skips_uv_validation() {
    // The npm branch: an @scope/thing that PEP 508 rejects is accepted for a js entry — the
    // validator is not called on a js/ts kind.
    let sandbox = Sandbox::new();
    let js = sandbox.write_source("t.js", "console.log(1)\n");
    sandbox.ok(&["add", &js, "--kind", "js", "--name", "jsx", "--no-input"]);
    sandbox.ok(&["deps", "jsx", "--dep", "@scope/thing"]);
    assert_eq!(
        sandbox.json(&["deps", "jsx", "--json"])["dependencies"],
        serde_json::json!(["@scope/thing"])
    );
}

#[test]
fn test_update_dependencies_normalizes_dash_python_before_validating() {
    // A literal '-' reaches the chokepoint on the deps path too: it normalizes to "" BEFORE the
    // validator (which would reject '-' as a specifier), leaving meta automatic.
    let sandbox = Sandbox::new();
    sandbox.add_python_print("a");
    sandbox.ok(&["deps", "a", "--dep", "requests", "--python", "-"]);
    assert!(!sandbox.stored_meta("a").contains("requires_python ="));
}

// ==========================================================================
// 3. suggest_dependencies filters its own fabrications through the PEP 508 check
// ==========================================================================

#[test]
fn test_suggest_dependencies_drops_a_name_pep508_refuses() {
    // `café` is a legal Python identifier but an illegal PEP 508 distribution name — it must not be
    // suggested (the non-interactive add takes suggestions as-is). `requests` is kept.
    let suggested = external_dependencies("python", "import café\nimport requests\nprint(1)\n");
    assert_eq!(suggested, ["requests"]);
    assert!(
        suggested
            .iter()
            .all(|s| validate_pep508_requirement(s).is_ok())
    );
}

#[test]
fn test_no_input_add_of_an_illegally_named_import_writes_no_block() {
    // A --no-input add of a script whose only third-party import is `café` writes NO PEP 723 block
    // (the old code fabricated `café` into the block, bricking every run).
    let sandbox = Sandbox::new();
    let src = sandbox.write_source("cafe.py", "import café\nprint(café)\n");
    sandbox.ok(&["add", &src, "--name", "cafe", "--no-input"]);
    let stored = sandbox.stored_block("cafe");
    assert!(!stored.contains("# /// script")); // no block fabricated (nothing valid to declare)
    assert!(!sandbox.stored_meta("cafe").contains("dependencies")); // café never recorded as a dep
}

// ==========================================================================
// 4. The two new draft refusal messages + the outside-drafts regression
// ==========================================================================

const DRAFT_HEAD: &str = "one of skit's own kept drafts";

#[cfg(unix)]
#[test]
#[ignore = "ABSENT (GAP, MUST-FIX): the drafts guard (Python cli.py:1894-1933) is not built in \
`add_with_config`, so an inferred-exe kept draft is ADDED as an exe reference (exit 0) instead of \
being refused (exit 2) with the --kind variant. The body is a full assertion (the CLI call \
compiles). Pending task #15 corroborates the add-lane gap. Python ref \
test_dependency_write_validation.py:281-293."]
fn test_inferred_exe_draft_gets_the_kind_variant() {
    use std::os::unix::fs::PermissionsExt;
    // A hand-planted +x on an extensionless draft INFERS exe with no flag passed — the refusal
    // points at --kind (there is nothing to drop), not the flag-drop message.
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-binish", "opaque program bytes\n");
    fs::set_permissions(&draft, fs::Permissions::from_mode(0o755)).unwrap();
    let output = sandbox.run(&["add", draft.to_str().unwrap(), "--name", "b1", "--no-input"]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    let flat = flat(&output);
    assert!(flat.contains(DRAFT_HEAD));
    assert!(flat.contains("pass --kind <language> to name its language"));
    assert!(!flat.contains("Drop")); // not the flag-route message — nothing was passed to drop
    assert!(draft.exists()); // a refused add consumes nothing
}

#[test]
#[ignore = "ABSENT (GAP, MUST-FIX): with the drafts guard absent (Python cli.py:1894-1933), --exe \
on a kept draft is ADDED as an exe reference (exit 0) instead of being refused (exit 2) with `Drop \
--exe.`. The body is a full assertion. Python ref test_dependency_write_validation.py:296-308."]
fn test_exe_flag_on_the_same_draft_gets_the_drop_variant_naming_only_exe() {
    // The flag route on the same kind of file: --exe WAS passed, so the message tells the user to
    // drop it — and names ONLY --exe, since that is the only flag passed.
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-binish2", "opaque program bytes\n");
    let output = sandbox.run(&[
        "add",
        draft.to_str().unwrap(),
        "--name",
        "b2",
        "--exe",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    let flat = flat(&output);
    assert!(flat.contains("Drop --exe."));
    assert!(!flat.contains("--ref")); // never passed — never named
    assert!(!flat.contains("--kind"));
    assert!(!flat.contains("to name its language")); // not the inferred-route message
    assert!(draft.exists());
}

#[test]
#[ignore = "ABSENT (GAP, MUST-FIX): the draft-aware `can't classify` variant (Python \
cli.py:2053-2066) is not built — a shebang-less kept draft gets the GENERIC `could not infer the \
entry kind; pass --kind KIND` refusal (exit 2) that names none of --kind/--prompt-for-a-draft. \
The body is a full assertion. Python ref test_dependency_write_validation.py:311-324."]
fn test_shebang_less_unclassifiable_draft_gets_the_classify_variant() {
    // A weird-extension, shebang-less kept draft infers 'unknown' with no #! — the classify variant
    // offers only --kind / --prompt (never --exe or --cmd).
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-weird.xyz", "just some content\n");
    let output = sandbox.run(&["add", draft.to_str().unwrap(), "--name", "w1", "--no-input"]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    let flat = flat(&output);
    assert!(flat.contains("kept draft skit can't classify"));
    assert!(flat.contains("--kind <language> to add it as a script"));
    assert!(flat.contains("--prompt for an AI-agent prompt"));
    assert!(!flat.contains("--exe")); // the draft variant never offers the program escape
    assert!(!flat.contains("--cmd")); // nor the command-template escape
    assert!(draft.exists());
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the SAME unclassifiable file OUTSIDE drafts/ IS refused \
(exit 2), but with the generic `could not infer the entry kind; pass --kind KIND` (cli.rs:2896), \
not the full escape naming --exe/--cmd. The `kept draft` negative already holds. Fixing the copy \
alone turns this green. Python ref test_dependency_write_validation.py:327-338."]
fn test_same_unclassifiable_file_outside_drafts_gets_the_full_escape() {
    // The SAME shebang-less weird-extension file OUTSIDE drafts/ is not a draft, so it keeps the
    // full escape message naming --exe and --cmd (which an on-disk file can take).
    let sandbox = Sandbox::new();
    let f = sandbox.write_source("weird.xyz", "just some content\n");
    let output = sandbox.run(&["add", &f, "--name", "w2", "--no-input"]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    let flat = flat(&output);
    assert!(flat.contains("isn't a script or an executable"));
    assert!(flat.contains("--exe for a program"));
    assert!(flat.contains("--cmd for a command template"));
    assert!(!flat.contains("kept draft"));
}

// ==========================================================================
// 5. The drafts guard precedes the .md "looks like a prompt" ask
// ==========================================================================

#[test]
#[ignore = "ABSENT (GAP, MUST-FIX): the drafts guard (Python cli.py:1894-1933) is absent, so a \
.md kept draft with --ref reaches the generic `could not infer the entry kind` refusal (exit 2) \
instead of the draft-head `Drop --ref.` guard. The oracle's Confirm.ask monkeypatch (proving the \
guard precedes the prompt ask) is an untranslatable in-process mechanism; this asserts the \
observable refusal contract. The body is a full assertion. Python ref \
test_dependency_write_validation.py:346-364."]
fn test_ref_on_an_md_draft_is_refused_before_the_prompt_ask() {
    // A .md kept draft with --ref must be refused at the drafts guard BEFORE the 'looks like a
    // prompt' ask. Only --ref was passed — only --ref is named.
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-note.md", "# Summarize {{text}}.\n");
    let output = sandbox.run(&["add", draft.to_str().unwrap(), "--name", "md1", "--ref"]);
    assert_eq!(output.status.code(), Some(2), "{}", flat(&output));
    let flat = flat(&output);
    assert!(flat.contains(DRAFT_HEAD));
    assert!(flat.contains("Drop --ref.")); // only --ref was passed — only --ref is named
    assert!(!flat.contains("--exe"));
    assert!(draft.exists()); // nothing consumed
}
