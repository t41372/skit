//! Delivery-channel facet of Python `tests/test_corpus.py` from `main@206f9ef`.
//!
//! `skit-language::inject_values` is intentionally a pure source rewriter, while Python's shell
//! injector returns both a temporary-source path and an environment overlay. These tests therefore
//! assert the missing channel split at the application assembly boundary: every envdefault must go
//! only to `env_values`, every other shell corpus candidate must go only to `inject_values`, and an
//! empty value map must route nothing at all. This does not pretend to test temp-file lifecycle;
//! lifecycle remains a separate runtime/public-process contract.

use std::{collections::{BTreeMap, BTreeSet}, fs, path::{Path, PathBuf}};

use skit_application::delivery::{PreparedValue, assemble};
use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::detect_candidates;

fn shell_corpus() -> Vec<PathBuf> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/shell");
    let mut paths = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.expect("shell corpus entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("sh"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn text(path: &Path) -> String {
    String::from_utf8(fs::read(path).unwrap()).unwrap()
}

fn sample(declaration: &ParamDecl) -> String {
    match declaration.parameter_type {
        ParameterType::Int => "7",
        ParameterType::Float => "1.5",
        ParameterType::Bool => "true",
        ParameterType::Str | ParameterType::Choice | ParameterType::Path => "sample",
    }
    .to_owned()
}

fn id(path: &Path) -> &str {
    path.file_name().and_then(|name| name.to_str()).unwrap_or("<non-utf8>")
}

#[test]
fn test_shell_inject_no_values_routes_no_env_and_no_rewrite_values() {
    for path in shell_corpus() {
        let declarations = detect_candidates("shell", &text(&path));
        let assembly = assemble(&declarations, &BTreeMap::new(), &[])
            .unwrap_or_else(|error| panic!("{} empty assembly failed: {error}", id(&path)));
        assert!(assembly.env_values.is_empty(), "{} routed env without a value", id(&path));
        assert!(assembly.inject_values.is_empty(), "{} routed rewrite without a value", id(&path));
        assert!(assembly.args.is_empty(), "{} routed argv without a value", id(&path));
        assert!(assembly.command_values.is_empty(), "{} routed placeholders without a value", id(&path));
    }
}

#[test]
fn test_shell_full_injection_routes_envdefaults_only_by_environment() {
    for path in shell_corpus() {
        let declarations = detect_candidates("shell", &text(&path));
        if declarations.is_empty() {
            continue;
        }
        let values = declarations
            .iter()
            .map(|declaration| {
                (
                    declaration.name.clone(),
                    PreparedValue::Scalar(sample(declaration)),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let assembly = assemble(&declarations, &values, &[])
            .unwrap_or_else(|error| panic!("{} full assembly failed: {error}", id(&path)));

        let expected_env = declarations
            .iter()
            .filter(|declaration| declaration.binding == ParameterBinding::EnvDefault)
            .map(|declaration| declaration.env_var().to_owned())
            .collect::<BTreeSet<_>>();
        let expected_rewrite = declarations
            .iter()
            .filter(|declaration| declaration.binding != ParameterBinding::EnvDefault)
            .map(|declaration| declaration.name.clone())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            assembly.env_values.keys().cloned().collect::<BTreeSet<_>>(),
            expected_env,
            "{} envdefault delivery drifted",
            id(&path)
        );
        assert_eq!(
            assembly.inject_values.keys().cloned().collect::<BTreeSet<_>>(),
            expected_rewrite,
            "{} source-rewrite delivery drifted",
            id(&path)
        );
        assert!(assembly.args.is_empty(), "{} unexpectedly routed shell corpus values to argv", id(&path));
        assert!(
            assembly.command_values.is_empty(),
            "{} unexpectedly routed shell corpus values to placeholders",
            id(&path)
        );

        for declaration in &declarations {
            match declaration.binding {
                ParameterBinding::EnvDefault => assert_eq!(
                    declaration.delivery,
                    ParameterDelivery::Env,
                    "{}:{} must remain environment-delivered",
                    id(&path),
                    declaration.name
                ),
                _ => assert_eq!(
                    declaration.delivery,
                    ParameterDelivery::Inject,
                    "{}:{} must remain source-rewrite delivered",
                    id(&path),
                    declaration.name
                ),
            }
        }
    }
}
