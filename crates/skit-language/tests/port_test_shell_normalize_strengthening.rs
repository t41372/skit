//! Rust-additive normalization coverage plus the exact frozen no-values injection contract.
//!
//! Python's structured batch/refusal-code result has no one-to-one Rust API, so those normalization
//! projections stay explicitly `rust_additive_*`. The empty-value-map contract does have an exact
//! public Rust projection: the injector must return the original source byte-for-byte.

use skit_language::{LanguageError, ParseOutcome, normalize_shell_default, parse_document};

fn candidate_names(source: &str) -> Vec<String> {
    match parse_document("shell", source) {
        ParseOutcome::Parsed(document) => document
            .analysis()
            .candidates
            .into_iter()
            .map(|candidate| candidate.declaration.name)
            .collect(),
        ParseOutcome::SyntaxError(_) | ParseOutcome::ParserUnavailable(_) => Vec::new(),
    }
}

#[test]
fn rust_additive_normalize_preserves_the_python_refusal_matrix_without_partial_rewrite() {
    let cases = [
        "A='literal $VAR'\n",
        "A='say \"hi\"'\n",
        "A='back\\slash'\n",
        "A='tick `x`'\n",
        "A='brace }'\n",
        "readonly A=1\n",
        "declare -r A=1\n",
        "A=1\nA=2\n",
        "A=\"${A:-1}\"\n",
        "B=1\n",
        "A=$(date)\n",
        "A=\"pre${OTHER}post\"\n",
        "A=\n",
        "A+=1\n",
    ];
    for source in cases {
        let before = source.to_owned();
        assert!(normalize_shell_default(source, "A").is_err(), "source should be refused: {source:?}");
        assert_eq!(source, before, "normalization refusal must never mutate caller-owned source");
    }
}

#[test]
fn rust_additive_normalize_rejects_every_shell_metacharacter_that_breaks_the_envdefault_idiom() {
    for meta in [';', '|', '&', '(', ')', '<', '>'] {
        let source = format!("MSG='a{meta}b'\n");
        assert!(candidate_names(&source).iter().any(|name| name == "MSG"), "fixture must be an offered const: {source:?}");
        assert!(normalize_shell_default(&source, "MSG").is_err(), "metacharacter {meta:?} must be refused");
    }
    assert_eq!(
        normalize_shell_default("MSG='plain'\n", "MSG").unwrap(),
        "MSG=\"${MSG:-plain}\"\n"
    );
}

#[test]
fn rust_additive_normalize_keeps_array_targets_out_and_rewrites_only_the_scalar_const() {
    let source = "#!/usr/bin/env bash\nARR[0]=1\nWIDTH=800\n";
    let normalized = normalize_shell_default(source, "WIDTH").unwrap();
    assert_eq!(
        normalized,
        "#!/usr/bin/env bash\nARR[0]=1\nWIDTH=\"${WIDTH:-800}\"\n"
    );
    assert!(normalize_shell_default(source, "ARR").is_err());
}

#[test]
fn rust_additive_normalize_unparseable_shell_is_a_typed_invalid_source_refusal() {
    let source = "#!/usr/bin/env zsh\nif [[ -n $X ]] {\n  print hi\n}\nA=1\n";
    assert_eq!(
        normalize_shell_default(source, "A").unwrap_err(),
        LanguageError::InvalidSource { kind: "shell".to_owned() }
    );
}

#[test]
fn rust_additive_normalize_mixed_names_preserves_success_and_independent_refusals() {
    let source = "#!/usr/bin/env bash\nWIDTH=800\nreadonly MAX=100\n";
    let width = normalize_shell_default(source, "WIDTH").unwrap();
    assert!(width.contains("WIDTH=\"${WIDTH:-800}\""), "{width}");
    assert!(width.contains("readonly MAX=100"), "{width}");
    assert!(normalize_shell_default(source, "MAX").is_err());
    assert!(normalize_shell_default(source, "NOPE").is_err());
}

#[test]
fn test_no_values_writes_nothing_at_all() {
    use std::collections::BTreeMap;
    use skit_language::inject_values;

    let source = "#!/usr/bin/env bash\nWIDTH=800\nread -p 'Name: ' who\n";
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("fixture must parse");
    };
    let declarations = document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();
    let rewritten = inject_values("shell", source, &declarations, &BTreeMap::new()).unwrap();
    assert_eq!(rewritten.as_bytes(), source.as_bytes());
}
