//! Mechanical port of the Python oracle module `tests/test_corpus.py`
//! (`origin/main@206f9ef`): "Golden corpus fidelity tests (C series), across every analyzable
//! language." The corpus files are byte-identical between the oracle checkout and this workspace
//! (`tests/corpus/`), so each Python parametrized `def test_*` becomes one Rust `#[test]` that
//! loops over the same corpus set. Each keeps the Python name and its "WHY" comment.
//!
//! Concept mapping used throughout:
//! - Python `ParamDecl.from_candidate(c) for c in analyze(text).candidates` (the `_specs_for`
//!   helper) -> `detect_candidates(kind, text)` (returns the same field-aligned `ParamDecl`s).
//! - Python `py_analyzer.analyze` / `sh_analyzer.analyze` / `js_analyzer.analyze(.., lang=)`
//!   -> `detect_candidates(kind, text)` (kind is `"python"` / `"shell"` / `"js"` / `"ts"`).
//! - Python `metawriter.write_params` / `js_io.write_params` -> `write_managed_params(kind, ..)`.
//! - Python `metawriter.read_params` / `js_io.read_params` -> `managed_params(kind, ..)`.
//! - Python `shim.inject(text, specs, values) -> str` -> `inject_values("python", ..)`.
//! - Python `.syntax_error` false / `compile(out, ..)` accepts -> `source_is_valid(kind, out)`.
//!
//! ## The injection tests span layers (matches the sibling `port_test_shell_inject.rs`).
//!
//! The Python shell/JS injection checks drive `sh_inject.inject(InjectRequest(..)) -> InjectResult`
//! and `js_inject.inject(.., lang=)`, whose `.path` (temp copy) and `.env` (environment overlay for
//! envdefaults) are the CLI/runtime tier in the Rust architecture (`skit-runtime` / `skit run`).
//! `skit-language` exposes only the SOURCE-REWRITE half: `inject_values(kind, text, specs, values)
//! -> String`. So each injection `#[test]` is a REAL asserting test on the reachable, divergence-
//! catching behavior:
//! - "no values -> no rewrite" (`result.path is None`) -> `inject_values(.., {}) == text`.
//! - "full injection re-parses" (the mandatory gate) -> `inject_values(..)` is `Ok` AND
//!   `source_is_valid(kind, out)` (`inject_values` itself re-parses and returns `Err` otherwise).
//! - "comment lines survive verbatim" -> asserted on the rewritten bytes.
//! - "envdefaults deliver by environment, never rewrite" -> at this tier the envdefault assignment
//!   is left untouched; an all-envdefault script rewrites to the identity (its `result.path` is
//!   `None`). The exact `set(result.env) == env_names` set-equality and the temp-file cleanup
//!   (`not tmp_path.iterdir()`) are the `skit-runtime` `InjectResult` shape; recorded as
//!   cross-crate gaps in this port's structured output (owed a runtime-tier port).

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterType};
use skit_language::{
    detect_candidates, inject_values, inject_values_for_interpreter, managed_params,
    source_is_valid, write_managed_params,
};

// ---------------------------------------------------------------- corpus fixtures

/// The workspace corpus root (byte-identical to the oracle's `tests/corpus/`).
fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

/// Python `sorted((Path(__file__).parent / "corpus" / ..).glob("*.<ext>"))`.
fn files(directory: &Path, extension: &str) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.expect("corpus directory entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some(extension))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

/// Python `_read`: preserve CRLF exactly (`open(newline="")`). `fs::read` keeps the bytes as-is.
fn source(path: &Path) -> String {
    String::from_utf8(
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("{} is not UTF-8: {error}", path.display()))
}

/// Python `_specs_for` / `_js_specs`: the field-aligned candidate declarations for a source.
fn specs_for(kind: &str, text: &str) -> Vec<ParamDecl> {
    detect_candidates(kind, text)
}

/// Python `_SAMPLE = {"str": "sample", "int": "7", "float": "1.5", "bool": "true"}`, keyed by every
/// spec name (`{s.name: _SAMPLE.get(s.type, "sample") for s in specs}`). Envdefault entries are
/// carried through exactly like the oracle; `inject_values` ignores non-`Inject` deliveries.
fn sample_values(specs: &[ParamDecl]) -> BTreeMap<String, String> {
    specs
        .iter()
        .map(|declaration| {
            let value = match declaration.parameter_type {
                ParameterType::Int => "7",
                ParameterType::Float => "1.5",
                ParameterType::Bool => "true",
                ParameterType::Str | ParameterType::Choice | ParameterType::Path => "sample",
            };
            (declaration.name.clone(), value.to_owned())
        })
        .collect()
}

/// Python `[ln for ln in out.splitlines(keepends=True) if ln not in src.splitlines(keepends=True)]`.
/// `str::split_inclusive('\n')` keeps `\r\n` whole, so it matches `splitlines(keepends=True)` for
/// the LF and CRLF corpus sources exactly.
fn added_lines<'out>(out: &'out str, src: &str) -> Vec<&'out str> {
    let source_lines = src.split_inclusive('\n').collect::<Vec<_>>();
    out.split_inclusive('\n')
        .filter(|line| !source_lines.contains(line))
        .collect()
}

/// Python `PY_CORPUS`.
fn python_corpus() -> Vec<PathBuf> {
    files(&corpus_root(), "py")
}

/// Python `SH_CORPUS`.
fn shell_corpus() -> Vec<PathBuf> {
    files(&corpus_root().join("shell"), "sh")
}

/// Python `_JS_TS`: `(lang, path)` pairs, JS files under the `js` grammar, TS under `ts`.
fn js_ts_corpus() -> Vec<(&'static str, PathBuf)> {
    let root = corpus_root();
    files(&root.join("js"), "mjs")
        .into_iter()
        .map(|path| ("js", path))
        .chain(
            files(&root.join("ts"), "ts")
                .into_iter()
                .map(|path| ("ts", path)),
        )
        .collect()
}

// ---------------------------------------------------------------- language-blind checks (py + sh)

#[test]
fn test_analyzer_never_raises() {
    // Any script yields candidates (or an empty list) without exceptions — the analyzer is total.
    for path in python_corpus() {
        let _ = specs_for("python", &source(&path));
    }
    for path in shell_corpus() {
        let _ = specs_for("shell", &source(&path));
    }
}

#[test]
fn test_metawriter_byte_fidelity() {
    // Write [tool.skit] then read it back: the read-back round-trips the specs, and every line the
    // writer ADDED is a comment line (the `#`-comment block engine is language-blind, so this holds
    // for shell exactly as for python).
    for (kind, path) in python_corpus()
        .into_iter()
        .map(|path| ("python", path))
        .chain(shell_corpus().into_iter().map(|path| ("shell", path)))
    {
        let text = source(&path);
        let specs = specs_for(kind, &text);
        let written = write_managed_params(kind, &text, &specs)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        // Read back must equal what was written.
        assert_eq!(managed_params(kind, &written), specs, "{}", path.display());
        // Byte-for-byte fidelity: every added line is inside the comment block (`#` prefix).
        let added = added_lines(&written, &text);
        assert!(
            added.iter().all(|line| line.trim_start().starts_with('#')),
            "{}: {added:?}",
            path.display()
        );
    }
}

#[test]
fn test_block_roundtrip_preserves_shebang() {
    // An injected `# /// script` block lands AFTER the shebang, and every code (non-comment) line
    // survives verbatim.
    for (kind, path) in python_corpus()
        .into_iter()
        .map(|path| ("python", path))
        .chain(shell_corpus().into_iter().map(|path| ("shell", path)))
    {
        let text = source(&path);
        let specs = specs_for(kind, &text);
        if specs.is_empty() {
            continue; // no candidates -> no block written
        }
        let written = write_managed_params(kind, &text, &specs)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let lines = text.split_inclusive('\n').collect::<Vec<_>>();
        if lines.first().is_some_and(|line| line.starts_with("#!")) {
            // The shebang stays on line 1 and the injected block opens strictly after it.
            assert_eq!(
                written.split_inclusive('\n').next(),
                lines.first().copied(),
                "{}",
                path.display()
            );
            assert!(
                written.find("#!") < written.find("# /// script"),
                "{}",
                path.display()
            );
        }
        // metawriter only ever touches comment lines: every non-comment line survives verbatim.
        for line in lines {
            if !line.trim_start().starts_with('#') {
                assert!(written.contains(line), "{}: {line:?}", path.display());
            }
        }
    }
}

// ---------------------------------------------------------------- python injector (shim)

#[test]
fn test_shim_no_values_is_identity() {
    // The shim with no values is the identity: inject(text, specs, {}) returns the exact same bytes.
    for path in python_corpus() {
        let text = source(&path);
        let specs = specs_for("python", &text);
        assert_eq!(
            inject_values("python", &text, &specs, &BTreeMap::new())
                .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
            text,
            "{}",
            path.display()
        );
    }
}

#[test]
fn test_shim_full_injection_compiles() {
    // Inject a type-compatible sample value for each candidate and verify the result is valid Python
    // (the oracle's `compile(out, .., "exec")`), with the PEP 723 block untouched.
    for path in python_corpus() {
        let text = source(&path);
        let specs = specs_for("python", &text);
        if specs.is_empty() {
            continue; // no candidates
        }
        let values = sample_values(&specs);
        let out = inject_values("python", &text, &specs, &values)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        // The injected output must be valid Python.
        assert!(source_is_valid("python", &out), "{}", path.display());
        // The PEP 723 block (`# /// script` .. `# ///`) must be untouched. Oracle uses a raw
        // (un-lstripped) prefix check.
        for line in text.lines() {
            if line.starts_with("# ///") || line.starts_with("# dependencies") {
                assert!(out.contains(line), "{}: {line:?}", path.display());
            }
        }
    }
}

// ---------------------------------------------------------------- shell injector

#[test]
fn test_shell_inject_no_values_writes_nothing() {
    // No values -> no rewrite: the injector never rewrites for free. In `skit-language` this is the
    // source-identity of `inject_values(.., {})`; the oracle's `result.path is None` and empty temp
    // dir are the `skit-runtime` InjectResult tier (cross-crate gap in this port's output).
    for path in shell_corpus() {
        let text = source(&path);
        let specs = specs_for("shell", &text);
        // `19_zsh_dialect.sh` is the honest-empty fixture tree-sitter-bash cannot parse. The oracle
        // `inject()` returns `InjectResult(env={})` (path None) for it because it short-circuits on
        // empty spans BEFORE any syntax gate; Rust `inject_values` gates on parse up front by
        // contract, so injection is not reachable for an unparseable source. Assert the oracle's own
        // honest-empty claim here; "the original runs unchanged" is the runtime tier's claim.
        if !source_is_valid("shell", &text) {
            assert!(specs.is_empty(), "{}", path.display());
            continue;
        }
        assert_eq!(
            inject_values_for_interpreter("shell", &text, &specs, &BTreeMap::new(), Some("bash"))
                .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
            text,
            "{}",
            path.display()
        );
    }
}

#[test]
fn test_shell_full_injection_reparses() {
    // Inject a type-compatible sample value for every candidate; the result must still parse (the
    // mandatory gate `inject_values` itself enforces), and a const-only file's comment lines survive
    // untouched. Files whose candidates are ALL envdefaults deliver purely by environment and
    // rewrite to the identity (`result.path is None`). The `set(result.env) == env_names` set is the
    // `skit-runtime` overlay tier (cross-crate gap in this port's output).
    for path in shell_corpus() {
        let text = source(&path);
        let specs = specs_for("shell", &text);
        if specs.is_empty() {
            continue; // no candidates
        }
        let values = sample_values(&specs);
        let out = inject_values_for_interpreter("shell", &text, &specs, &values, Some("bash"))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        if specs
            .iter()
            .all(|declaration| declaration.binding == ParameterBinding::EnvDefault)
        {
            // Env-only script: envdefaults go out by environment, so nothing is rewritten.
            assert_eq!(out, text, "{}", path.display());
            continue;
        }
        // The injected output must still parse (the same gate inject() enforces).
        assert!(source_is_valid("shell", &out), "{}", path.display());
        // Only code bytes change: every comment line of the original survives verbatim.
        for line in text.lines() {
            if line.trim_start().starts_with('#') {
                assert!(out.contains(line), "{}: {line:?}", path.display());
            }
        }
    }
}

// ---------------------------------------------------------------- JS/TS corpus

#[test]
fn test_js_analyzer_never_raises() {
    // Each file parsed under its kind's grammar yields candidates without exceptions.
    for (lang, path) in js_ts_corpus() {
        let _ = specs_for(lang, &source(&path));
    }
}

#[test]
fn test_js_block_byte_fidelity() {
    // Write the block then read it back (the `//`-block engine round-trips), and every line the
    // writer ADDED is a `//`-comment line.
    for (lang, path) in js_ts_corpus() {
        let text = source(&path);
        let specs = specs_for(lang, &text);
        let written = write_managed_params(lang, &text, &specs)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(managed_params(lang, &written), specs, "{}", path.display());
        let added = added_lines(&written, &text);
        assert!(
            added.iter().all(|line| line.trim_start().starts_with("//")),
            "{}: {added:?}",
            path.display()
        );
    }
}

#[test]
fn test_js_inject_no_values_is_identity() {
    // No values -> no rewrite, no temp copy. In `skit-language` this is the source-identity of
    // `inject_values(.., {})`; `result.path is None`, `result.env == {}`, and the empty temp dir are
    // the `skit-runtime` InjectResult tier (cross-crate gap in this port's output).
    for (lang, path) in js_ts_corpus() {
        let text = source(&path);
        let specs = specs_for(lang, &text);
        assert_eq!(
            inject_values(lang, &text, &specs, &BTreeMap::new())
                .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
            text,
            "{}",
            path.display()
        );
    }
}

#[test]
fn test_js_full_injection_reparses() {
    // Inject a type-compatible sample value for every candidate; JS delivers every value by
    // temp-copy rewrite (no env channel), so the result must still parse and every `//`-comment line
    // of the original survives verbatim. `result.path is not None` / `result.env == {}` are the
    // `skit-runtime` InjectResult tier (cross-crate gap in this port's output).
    for (lang, path) in js_ts_corpus() {
        let text = source(&path);
        let specs = specs_for(lang, &text);
        if specs.is_empty() {
            continue; // no candidates
        }
        let values = sample_values(&specs);
        let out = inject_values(lang, &text, &specs, &values)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        // The injected output must still parse (the same mandatory gate inject() enforces).
        assert!(source_is_valid(lang, &out), "{}", path.display());
        for line in text.lines() {
            if line.trim_start().starts_with("//") {
                assert!(out.contains(line), "{}: {line:?}", path.display());
            }
        }
    }
}
