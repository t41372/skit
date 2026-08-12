//! Exact behavioral port of Python `tests/test_corpus.py` from `main@206f9ef`.
//!
//! The existing Rust `corpus.rs` adds useful candidate-name oracles, but it aggregates several
//! Python contracts and previously skipped the no-value rewrite contract for syntax-invalid source.
//! This file keeps the Python test names and fixture scopes explicit. Red results are parity findings;
//! do not weaken these assertions to match the current Rust implementation.

use std::{
    collections::BTreeMap,
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
};

use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};
use skit_language::{
    detect_candidates, inject_values, managed_params, source_is_valid, write_managed_params,
};

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn files(directory: &Path, extension: &str) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.expect("corpus directory entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some(extension))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn read_exact(path: &Path) -> String {
    String::from_utf8(
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("{} is not UTF-8: {error}", path.display()))
}

fn neutral_corpus() -> Vec<(&'static str, PathBuf)> {
    let root = corpus_root();
    files(&root, "py")
        .into_iter()
        .map(|path| ("python", path))
        .chain(
            files(&root.join("shell"), "sh")
                .into_iter()
                .map(|path| ("shell", path)),
        )
        .collect()
}

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

fn sample_values(declarations: &[ParamDecl]) -> BTreeMap<String, String> {
    declarations
        .iter()
        .filter(|declaration| declaration.delivery == ParameterDelivery::Inject)
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

/// Match Python `splitlines(keepends=True)`: CRLF is one line terminator, and lone CR is also a
/// terminator. Splitting on `['\r', '\n']` separately would silently weaken the CRLF corpus cases.
fn lines_keepends(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                index += 1;
                if index < bytes.len() && bytes[index] == b'\n' {
                    index += 1;
                }
                lines.push(&text[start..index]);
                start = index;
            }
            b'\n' => {
                index += 1;
                lines.push(&text[start..index]);
                start = index;
            }
            _ => index += 1,
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

fn path_id(kind: &str, path: &Path) -> String {
    format!(
        "{kind}:{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<non-utf8>")
    )
}

#[test]
fn test_analyzer_never_raises() {
    let corpus = neutral_corpus();
    assert!(!corpus.is_empty());
    for (kind, path) in corpus {
        let text = read_exact(&path);
        let result = catch_unwind(AssertUnwindSafe(|| detect_candidates(kind, &text)));
        assert!(
            result.is_ok(),
            "analyzer raised for {}",
            path_id(kind, &path)
        );
    }
}

#[test]
fn test_metawriter_byte_fidelity() {
    for (kind, path) in neutral_corpus() {
        let text = read_exact(&path);
        let specs = detect_candidates(kind, &text);
        let written = write_managed_params(kind, &text, &specs).unwrap_or_else(|error| {
            panic!(
                "metadata write failed for {}: {error}",
                path_id(kind, &path)
            )
        });
        assert_eq!(
            managed_params(kind, &written),
            specs,
            "{}",
            path_id(kind, &path)
        );

        let original_lines = lines_keepends(&text);
        let added = lines_keepends(&written)
            .into_iter()
            .filter(|line| !original_lines.contains(line))
            .collect::<Vec<_>>();
        assert!(
            added.iter().all(|line| line.trim_start().starts_with('#')),
            "{} added non-comment metadata lines: {added:?}",
            path_id(kind, &path)
        );
    }
}

#[test]
fn test_block_roundtrip_preserves_shebang() {
    for (kind, path) in neutral_corpus() {
        let text = read_exact(&path);
        let specs = detect_candidates(kind, &text);
        if specs.is_empty() {
            continue;
        }
        let written = write_managed_params(kind, &text, &specs).unwrap_or_else(|error| {
            panic!(
                "metadata write failed for {}: {error}",
                path_id(kind, &path)
            )
        });
        let lines = lines_keepends(&text);
        if lines.first().is_some_and(|line| line.starts_with("#!")) {
            assert_eq!(
                lines_keepends(&written).first(),
                lines.first(),
                "{} moved the shebang",
                path_id(kind, &path)
            );
            let shebang = written.find("#!").expect("written source lost shebang");
            let block = written
                .find("# /// script")
                .expect("written source lost metadata block");
            assert!(
                shebang < block,
                "{} placed metadata before shebang",
                path_id(kind, &path)
            );
        }
        for line in lines {
            if !line.trim_start().starts_with('#') {
                assert!(
                    written.contains(line),
                    "{} lost code line {line:?}",
                    path_id(kind, &path)
                );
            }
        }
    }
}

#[test]
fn test_shim_no_values_is_identity() {
    for path in files(&corpus_root(), "py") {
        let text = read_exact(&path);
        let specs = detect_candidates("python", &text);
        let out =
            inject_values("python", &text, &specs, &BTreeMap::new()).unwrap_or_else(|error| {
                panic!(
                    "empty Python injection failed for {}: {error}",
                    path.display()
                )
            });
        assert_eq!(out.as_bytes(), text.as_bytes(), "{}", path.display());
    }
}

#[test]
fn test_shim_full_injection_compiles() {
    for path in files(&corpus_root(), "py") {
        let text = read_exact(&path);
        let specs = detect_candidates("python", &text);
        if specs.is_empty() {
            continue;
        }
        let out = inject_values("python", &text, &specs, &sample_values(&specs)).unwrap_or_else(
            |error| panic!("Python injection failed for {}: {error}", path.display()),
        );
        assert!(
            source_is_valid("python", &out),
            "injected Python is invalid: {}",
            path.display()
        );
        for line in text.lines() {
            if line.starts_with("# ///") || line.starts_with("# dependencies") {
                assert!(
                    out.lines().any(|out_line| out_line == line),
                    "{} lost PEP 723 line {line:?}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn test_shell_inject_no_values_writes_nothing() {
    for path in files(&corpus_root().join("shell"), "sh") {
        let text = read_exact(&path);
        let specs = detect_candidates("shell", &text);
        // This intentionally includes `19_zsh_dialect.sh`. Python returns without creating a temp
        // copy even though tree-sitter-bash cannot parse that dialect; empty values must be free.
        let out = inject_values("shell", &text, &specs, &BTreeMap::new()).unwrap_or_else(|error| {
            panic!(
                "empty shell injection failed for {}: {error}",
                path.display()
            )
        });
        assert_eq!(out.as_bytes(), text.as_bytes(), "{}", path.display());
    }
}

#[test]
fn test_shell_full_injection_reparses() {
    for path in files(&corpus_root().join("shell"), "sh") {
        let text = read_exact(&path);
        let specs = detect_candidates("shell", &text);
        if specs.is_empty() {
            continue;
        }
        let inject_specs = specs
            .iter()
            .filter(|spec| spec.delivery == ParameterDelivery::Inject)
            .cloned()
            .collect::<Vec<_>>();
        let values = sample_values(&specs);
        let out = inject_values("shell", &text, &specs, &values).unwrap_or_else(|error| {
            panic!("shell injection failed for {}: {error}", path.display())
        });
        if inject_specs.is_empty() {
            assert_eq!(
                out.as_bytes(),
                text.as_bytes(),
                "env-only shell source was rewritten: {}",
                path.display()
            );
            continue;
        }
        assert!(
            source_is_valid("shell", &out),
            "injected shell is invalid: {}",
            path.display()
        );
        for line in text.lines() {
            if line.trim_start().starts_with('#') {
                assert!(
                    out.lines().any(|out_line| out_line == line),
                    "{} lost comment line {line:?}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn test_js_analyzer_never_raises() {
    let corpus = js_ts_corpus();
    assert!(!corpus.is_empty());
    for (kind, path) in corpus {
        let text = read_exact(&path);
        let result = catch_unwind(AssertUnwindSafe(|| detect_candidates(kind, &text)));
        assert!(
            result.is_ok(),
            "analyzer raised for {}",
            path_id(kind, &path)
        );
    }
}

#[test]
fn test_js_block_byte_fidelity() {
    for (kind, path) in js_ts_corpus() {
        let text = read_exact(&path);
        let specs = detect_candidates(kind, &text);
        let written = write_managed_params(kind, &text, &specs).unwrap_or_else(|error| {
            panic!(
                "metadata write failed for {}: {error}",
                path_id(kind, &path)
            )
        });
        assert_eq!(
            managed_params(kind, &written),
            specs,
            "{}",
            path_id(kind, &path)
        );
        let original_lines = lines_keepends(&text);
        let added = lines_keepends(&written)
            .into_iter()
            .filter(|line| !original_lines.contains(line))
            .collect::<Vec<_>>();
        assert!(
            added.iter().all(|line| line.trim_start().starts_with("//")),
            "{} added non-comment metadata lines: {added:?}",
            path_id(kind, &path)
        );
    }
}

#[test]
fn test_js_inject_no_values_is_identity() {
    for (kind, path) in js_ts_corpus() {
        let text = read_exact(&path);
        let specs = detect_candidates(kind, &text);
        let out = inject_values(kind, &text, &specs, &BTreeMap::new()).unwrap_or_else(|error| {
            panic!(
                "empty JS/TS injection failed for {}: {error}",
                path_id(kind, &path)
            )
        });
        assert_eq!(out.as_bytes(), text.as_bytes(), "{}", path_id(kind, &path));
    }
}

#[test]
fn test_js_full_injection_reparses() {
    for (kind, path) in js_ts_corpus() {
        let text = read_exact(&path);
        let specs = detect_candidates(kind, &text);
        if specs.is_empty() {
            continue;
        }
        assert!(
            specs
                .iter()
                .all(|spec| spec.delivery == ParameterDelivery::Inject),
            "{} unexpectedly exposes a non-rewrite JS delivery channel",
            path_id(kind, &path)
        );
        let out =
            inject_values(kind, &text, &specs, &sample_values(&specs)).unwrap_or_else(|error| {
                panic!(
                    "JS/TS injection failed for {}: {error}",
                    path_id(kind, &path)
                )
            });
        assert_ne!(
            out.as_bytes(),
            text.as_bytes(),
            "{} did not materialize full injection",
            path_id(kind, &path)
        );
        assert!(
            source_is_valid(kind, &out),
            "injected source is invalid: {}",
            path_id(kind, &path)
        );
        for line in text.lines() {
            if line.trim_start().starts_with("//") {
                assert!(
                    out.lines().any(|out_line| out_line == line),
                    "{} lost comment line {line:?}",
                    path_id(kind, &path)
                );
            }
        }
    }
}
