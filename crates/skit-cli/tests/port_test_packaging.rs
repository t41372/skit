//! Mechanical port of the Python oracle module `tests/test_packaging.py`
//! (`origin/main@206f9ef`): distribution-packaging invariants read straight from
//! `pyproject.toml` plus the `skit.__version__` resolver in `src/skit/__init__.py`.
//!
//! The oracle module protects two kinds of thing. One survives the rewrite verbatim
//! (the shipped distribution metadata still lives in `pyproject.toml`, because Maturin
//! builds the binary wheel that keeps `uv tool install skit-cli` working). The other is
//! pure Python-implementation machinery — `importlib.metadata`, module `__getattr__`,
//! gettext `.po`/`.mo`, `mutmut` — that AGENTS.md removes: "The product has no Python
//! implementation." Those behaviors have no Rust analog by design; the version is a
//! compile-time constant, i18n is a static Rust catalog, and mutation testing is
//! `cargo-mutants`.
//!
//! Concept mapping used throughout:
//! - Python `PYPROJECT` (root `pyproject.toml` read with `tomllib`) -> the Maturin
//!   `pyproject.toml` at the workspace root, read via `CARGO_MANIFEST_DIR/../../` and the
//!   `toml` crate (same path the sibling `port_test_entrypoint.rs` uses).
//! - Python `skit.__version__` (single-sourced from the installed dist metadata, which
//!   comes from `pyproject.toml` at build time) -> `env!("CARGO_PKG_VERSION")` (the test
//!   crate is the same `skit-cli-rs` package, so this is the binary's version), which the
//!   CLI prints at `crates/skit-cli/src/cli.rs:722`.
//! - Python uv `tool.uv.build-backend.wheel-exclude = ["**/*.po", "**/*.pot"]` (keep
//!   maintainer-only catalog sources in the sdist, out of the end-user wheel) -> Maturin
//!   `tool.maturin.include` entries that are all `format = "sdist"` (the wheel ships only
//!   the binary; non-runtime source inputs ride the sdist only). Rust has no gettext
//!   catalogs — the only `.po`/`.pot` in this checkout live under the stale `mutants/`
//!   copy of the 0.4 Python oracle, which this port does not read.
//!
//! Buckets:
//! - Real asserting tests (3): the distribution-metadata invariants that survive the
//!   rewrite — no dead extras, maintainer sources kept out of the wheel, and the version
//!   single-sourced with no drift between the wheel metadata and the compiled binary.
//! - UNMAPPED, `#[ignore]` (4): `mutmut also_copy`, the `importlib.metadata` fallback,
//!   its lazy memoization, and the module `__getattr__` guard. Each is a
//!   Python-implementation artifact with no analog by design (recorded as kind=absent for
//!   schema completeness only — none is a feature gap).

use std::fs;

/// The Maturin `pyproject.toml` at the workspace root, parsed as a TOML table.
///
/// `CARGO_MANIFEST_DIR` is `crates/skit-cli`; the shipped distribution metadata lives two
/// directories up, exactly where the sibling `port_test_entrypoint.rs` reads it.
fn pyproject() -> toml::Table {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    fs::read_to_string(format!("{manifest_dir}/../../pyproject.toml"))
        .expect("workspace-root pyproject.toml is readable")
        .parse()
        .expect("pyproject.toml parses as TOML")
}

#[test]
fn test_no_dead_optional_dependencies() {
    // scripts/serve_preview.py (the only textual-serve consumer) is dev-only and not shipped
    // in the wheel; textual-serve belongs solely to [dependency-groups].dev. A public extra
    // here would let a user `pip install skit-cli[serve]` into installing a dependency that no
    // shipped code imports. The invariant survives the rewrite: the shipped [project] table must
    // carry no optional-dependencies (no dead extras).
    let pyproject = pyproject();
    let project = pyproject["project"]
        .as_table()
        .expect("[project] table present");
    assert!(
        !project.contains_key("optional-dependencies"),
        "[project] must declare no optional-dependencies (no dead extras): {project:?}"
    );
}

#[test]
fn test_wheel_excludes_catalog_sources() {
    // Oracle: `tool.uv.build-backend.wheel-exclude` ends with `*.po` and `*.pot` — catalog
    // sources are maintainer inputs to `scripts/i18n.py compile`; the runtime loads only compiled
    // `.mo` via stdlib gettext, so sources stay in the sdist, out of the end-user wheel.
    //
    // The Rust build has no gettext catalogs (i18n is a static Rust catalog) and uses Maturin, not
    // uv's build backend, so there is no `wheel-exclude` key to assert. The same intent — keep
    // non-runtime maintainer/source inputs out of the binary wheel — is carried by Maturin's
    // `include` list, whose entries are all `format = "sdist"`. Assert the invariant, not today's
    // paths: `include` is non-empty and every entry is sdist-only. This also guards Maturin's
    // default (an `include` entry with no `format` ships in BOTH sdist and wheel), which would
    // silently bloat the end-user wheel with source-only inputs.
    let pyproject = pyproject();
    let include = pyproject["tool"]["maturin"]["include"]
        .as_array()
        .expect("tool.maturin.include is an array");
    assert!(
        !include.is_empty(),
        "maturin include list must be non-empty"
    );
    assert!(
        include
            .iter()
            .all(|entry| entry.get("format").and_then(toml::Value::as_str) == Some("sdist")),
        "every maturin include entry must be format = \"sdist\" (sdist-only, not shipped in the wheel): {include:?}"
    );
}

#[test]
#[ignore = "UNMAPPED: mutmut `[tool.mutmut].also_copy` refreshes runtime package data (src/skit/locales/, src/skit/skills/) into a reused mutants/ worktree so a baseline sees current translations and the bundled skill. The Rust mutation tool is cargo-mutants (AGENTS.md `cargo mutants`), and runtime package data is embedded at compile time (SKILL.md via include_str!/include_bytes!; i18n is a static Rust catalog), so no runtime package-data staleness exists. Python-tooling artifact; NOT a must-fix gap -- no analog exists by design."]
fn test_mutmut_refreshes_all_runtime_package_data_in_a_reused_worktree() {
    // Oracle (tests/test_packaging.py:37-60): discovers every non-code file under src/skit/ and
    // asserts each is under a `[tool.mutmut].also_copy` root, so a reused mutants/ tree never runs
    // its baseline against stale translations or a stale bundled skill. cargo-mutants copies and
    // rebuilds the whole tree per mutant, and skit embeds SKILL.md + i18n at compile time, so the
    // staleness this guards against cannot arise.
}

#[test]
fn test_version_is_single_sourced_from_the_distribution() {
    // Oracle: `skit.__version__ == version("skit-cli")` — one source, no drift (the old hand-synced
    // literal in __init__.py once shipped a release with mismatched versions). The Rust analog is
    // the same anti-drift contract across the two independent declarations that reach PyPI: Maturin
    // reads the *wheel* version from pyproject's [project].version, while the *binary* prints
    // Cargo's CARGO_PKG_VERSION. They must be equal, or a release ships a wheel whose `skit
    // --version` disagrees with its package metadata.
    let pyproject = pyproject();
    assert_eq!(
        pyproject["project"]["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "pyproject [project].version must match the binary's CARGO_PKG_VERSION (no drift)"
    );
}

#[test]
#[ignore = "UNMAPPED: the `importlib.metadata` PackageNotFoundError fallback to \"0.0.0+unknown\" guards a bare Python checkout with no installed dist. env!(\"CARGO_PKG_VERSION\") is a compile-time constant baked from Cargo.toml -- always present, no runtime resolver, no failure path, no fallback literal. Python-implementation artifact; NOT a must-fix gap -- no analog exists by design."]
fn test_version_falls_back_when_no_distribution_is_installed() {
    // Oracle (src/skit/__init__.py:38-41): `version("skit-cli")` raising PackageNotFoundError
    // resolves `skit.__version__` to "0.0.0+unknown" so a bare checkout still imports and says so.
    // The Rust version can never be absent, so there is nothing to fall back from.
}

#[test]
#[ignore = "UNMAPPED: `skit.__version__` resolves lazily via importlib.metadata (~85 modules, the largest startup import) and memoizes into module globals so it runs at most once per interpreter. A compiled binary reads env!(\"CARGO_PKG_VERSION\") -- a constant with zero import cost and no resolver to reach twice, so there is no lazy resolution or memoization to observe. Python-implementation artifact; NOT a must-fix gap -- no analog exists by design."]
fn test_version_is_resolved_once_and_then_memoized() {
    // Oracle (src/skit/__init__.py:21-43): the PEP 562 `__getattr__` hook caches the resolved
    // value into globals()["__version__"], so a second `skit.__version__` access must not reach the
    // metadata layer again (asserted via a call-counting monkeypatch). Rust has no resolver call to
    // count.
}

#[test]
#[ignore = "UNMAPPED: the module-level PEP 562 `__getattr__` answers exactly `__version__` and raises AttributeError(\"module 'skit' has no attribute 'nope'\") for anything else, so a typo cannot resolve to a version string. A Rust module has no `__getattr__` hook -- an unknown path is a compile error, not a runtime attribute lookup. Python-implementation artifact; NOT a must-fix gap -- no analog exists by design."]
fn test_module_getattr_refuses_anything_but_the_version() {
    // Oracle (src/skit/__init__.py:34-35): `if name != "__version__": raise AttributeError(f"module
    // {__name__!r} has no attribute {name!r}")`. Rust name resolution is static, so the failure mode
    // this guards (any name silently resolving to the version) cannot exist.
}
