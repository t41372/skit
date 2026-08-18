//! Frozen benchmark source-integrity contracts from `tests/test_benchmarks_tooling.py`.

use sha2::{Digest as _, Sha256};
use skit_benchmarks::sources::{LANGUAGES, generate, generate_broken};
use skit_language::{ParseOutcome, detect_candidates, parse_document};

fn sha256(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn expected_normal(language: &str, lines: usize) -> &'static str {
    match (language, lines) {
        ("python", 20) => "331e49e5afdc220ec7072bce1c36bcadfbf4ef27a272fbcfea6c58232048c5eb",
        ("python", 200) => "bddbff14521b972353a786a23d26878241ebd201bb3abddbf41816f5b6a0e30c",
        ("python", 2_000) => "d3930447d045e075d42e9de5af26d7fc82b00aadddfd22ab5e3280c71611c2fd",
        ("shell", 20) => "18313d3c90cf19940c4d7929f98175c78e581a6a8ae2beb3298e11e92dd6e71f",
        ("shell", 200) => "1580868ff9403e7994f6154dcf140ef662296cffe4b8270323e6f7f27a46bdd9",
        ("shell", 2_000) => "f06766fb3268b15d1ef41004302894cab8dcc0f60e7cd8e40481ff07b8883076",
        ("js", 20) => "a71c521298245d452903ff3c72ecb46587eaa0a2d94207d1752a3a50c2c66c0b",
        ("js", 200) => "3c5dce15dd809b1943dfd33347a27d9f5bf63b7100f5c4570032450b58d3f78c",
        ("js", 2_000) => "d6fcca3167e919aa5745cea2789e995ce312d9cda6e8486ae2bc08321298de4d",
        ("ts", 20) => "53d5404ce5ee784b79841b1897a8c63c81e064076d62cfe56404f761ca8c0e1e",
        ("ts", 200) => "9dfa7b5ba9138fd6a9e23166eead6e9a73fae74eb9aa39e178a4d8ee927c18f4",
        ("ts", 2_000) => "4fa53b7268b29fccdac92e0c751ba48d48980f6d4ad75c3fdb45cb88ecd8f558",
        other => panic!("no frozen normal workload hash for {other:?}"),
    }
}

fn expected_broken(language: &str) -> &'static str {
    match language {
        "python" => "af266a68d9826a6d003dc4e4b1609c15ff74ac4a8dd76a21217505fcbe2fdb22",
        "shell" => "cd792b3d0663223351bb25f585e0da85d220b7dbada1720dc3185504076b0b2b",
        "js" => "deaf98beca99466bcf3f651f757adbb1ddb2f94733087156b1b6a68514437ff2",
        "ts" => "8e64a7453ec0fbf7d32dc257f1c44c13d4927c8571055e59c34e87985176de21",
        other => panic!("no frozen broken workload hash for {other:?}"),
    }
}

#[test]
fn test_analyzer_workloads_are_byte_stable() {
    for language in LANGUAGES {
        for lines in [20, 200, 2_000] {
            let source = generate(language, lines).unwrap();
            assert_eq!(
                sha256(&source),
                expected_normal(language, lines),
                "{language}:{lines} benchmark workload bytes drifted from the frozen Python corpus"
            );
        }
    }
}

#[test]
fn test_broken_workloads_are_byte_stable_and_actually_broken() {
    for language in LANGUAGES {
        let valid = generate(language, 2_000).unwrap();
        let broken = generate_broken(language, 2_000).unwrap();
        assert_eq!(
            sha256(&broken),
            expected_broken(language),
            "{language} broken workload bytes drifted from the frozen Python corpus"
        );
        assert_eq!(broken.lines().count(), valid.lines().count());
        let broken_lines = broken.lines().collect::<Vec<_>>();
        let valid_lines = valid.lines().collect::<Vec<_>>();
        assert_eq!(broken_lines.len(), 2_000);
        assert_eq!(broken_lines[..broken_lines.len() - 1], valid_lines[..valid_lines.len() - 1]);
        assert_ne!(broken, valid, "{language} broken twin collapsed to the valid workload");
    }

    let broken_python = generate_broken("python", 2_000).unwrap();
    assert!(
        matches!(parse_document("python", &broken_python), ParseOutcome::SyntaxError(_)),
        "the frozen half-written Python workload no longer fails the product's Python parser"
    );
    assert!(
        matches!(
            parse_document("python", &generate("python", 2_000).unwrap()),
            ParseOutcome::Parsed(_)
        ),
        "the valid Python twin no longer parses"
    );
}

#[test]
fn test_analyzers_survive_a_half_written_source() {
    for language in LANGUAGES {
        let broken = generate_broken(language, 200).unwrap();
        let _ = detect_candidates(language, &broken);
    }
}

#[test]
fn test_js_ts_braces_balance() {
    for language in ["js", "ts"] {
        for lines in [20, 200, 2_000] {
            let source = generate(language, lines).unwrap();
            assert_eq!(
                source.matches('{').count(),
                source.matches('}').count(),
                "{language}:{lines} generated unbalanced braces"
            );
        }
    }
}

#[test]
fn test_python_compiles() {
    // The frozen oracle compiled these exact bytes with CPython. Keep byte identity as well as the
    // native parser verdict so a Rust-side grammar check cannot silently replace the stronger
    // CPython-validity fact with a different workload.
    for lines in [20, 200, 2_000] {
        let source = generate("python", lines).unwrap();
        assert_eq!(sha256(&source), expected_normal("python", lines));
        assert!(
            matches!(parse_document("python", &source), ParseOutcome::Parsed(_)),
            "frozen CPython-valid source no longer parses in skit's Python adapter"
        );
    }
}

#[test]
fn test_tree_sitter_parses_without_errors() {
    for language in ["js", "ts"] {
        for lines in [20, 200, 2_000] {
            let source = generate(language, lines).unwrap();
            assert!(
                matches!(parse_document(language, &source), ParseOutcome::Parsed(_)),
                "{language}:{lines} generated source is not accepted by skit's tree-sitter adapter"
            );
        }
    }
}
