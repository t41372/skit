//! Public-dispatch port of Python v0.4 `test_registry_capabilities` from `tests/test_fish.py`.
//!
//! Rust does not expose Python's `LanguageSpec` registry object. Exercise the same capabilities
//! through the public dispatch surface instead of recreating a test-only registry.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParameterBinding, ParameterDelivery};
use skit_language::{
    CliSurface, LanguageError, ParseOutcome, inject_values, managed_params, parse_document,
    write_managed_params,
};

#[test]
fn test_registry_capabilities() {
    let env_source = "set -q PORT; or set PORT 8080\n";
    let ParseOutcome::Parsed(document) = parse_document("fish", env_source) else {
        panic!("Fish must have a registered parser/analyzer");
    };
    let analysis = document.analysis();
    let port = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "PORT")
        .expect("Fish analyzer dispatch must expose the env-default candidate")
        .declaration
        .clone();
    assert_eq!(port.binding, ParameterBinding::EnvDefault);
    assert_eq!(port.delivery, ParameterDelivery::Env);

    let ParseOutcome::Parsed(cli_document) =
        parse_document("fish", "argparse 'n/name=' -- $argv\n")
    else {
        panic!("Fish CLI reader must share the registered parser");
    };
    match cli_document.cli_surface() {
        CliSurface::Static(surface) => {
            assert_eq!(surface.framework, "argparse");
            assert_eq!(surface.fields.len(), 1);
            assert_eq!(surface.fields[0].declaration.name, "name");
        }
        other => panic!("Fish argparse reader is not registered: {other:?}"),
    }

    let managed = write_managed_params("fish", env_source, std::slice::from_ref(&port))
        .expect("Fish must have registered inline params I/O");
    let read_back = managed_params("fish", &managed);
    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].name, "PORT");
    assert_eq!(read_back[0].binding, ParameterBinding::EnvDefault);
    assert_eq!(read_back[0].delivery, ParameterDelivery::Env);

    let error = inject_values(
        "fish",
        env_source,
        std::slice::from_ref(&port),
        &BTreeMap::from([("PORT".to_owned(), "9090".to_owned())]),
    )
    .unwrap_err();
    assert!(matches!(error, LanguageError::UnsupportedKind { ref kind } if kind == "fish"));
}
