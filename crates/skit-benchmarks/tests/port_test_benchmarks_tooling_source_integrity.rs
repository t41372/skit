//! Frozen benchmark source-integrity contracts from `tests/test_benchmarks_tooling.py`.

use sha2::{Digest as _, Sha256};
use skit_benchmarks::sources::{LANGUAGES, generate, generate_broken};
use skit_language::{ParseOutcome, detect_candidates, parse_document};

fn sha256(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn expected_normal(language: &str, lines: usize) -> &'static str {
    match (language, lines) {
        ("python", 20) => "c39ba209945af7d61ecab53cfd0a7cd918b732441fd7f0db53b79b13cdc56426",
        ("python", 200) => "a419060f4eabd016858bf48b4ba4c371170578f7f1732c068651f47a4fbd2f59",
        ("python", 2_000) => "aeccbe8427b18382a03c1a7fdd7404d840ec27d38ea2b3fd4d7031d71e41b70",
        ("shell", 20) => "35231366cdfff4125c8a7efc7b0dfd7f5aa7d272b720245845cbc1166326baeb",
        ("shell", 200) => "e8bca8f80e08e3e4605c3d313fac19754097bb604b514f823249347534a4311b",
        ("shell", 2_000) => "442226682042641fab94b626072af8c7f3ef9a9912e53e7b54ca886aad2f1ecf",
        ("js", 20) => "c800571adaa171506e9ccd6e96fb563ed968898b0a3a3d9ac06c9f2290fcfac6",
        ("js", 200) => "74a5203f477650052df73844cb6e64d3b32836a1a85c2f9355fe84bd2db4d6a",
        ("js", 2_000) => "c3dcc00d80c6f58aebb6eaf6930579370f2ec7dbd848406fa323858ad8a8aded",
        ("ts", 20) => "7d58fa23ae213316220379af6f68b23c42d6b8e0b779e27b8feb2cd7b1f290d",
        ("ts", 200) => "08fd926c74e820cbdc8aeb2563477750ce440002640412a5cc25d3f80acbc638",
        ("ts", 2_000) => "f1231ecc240efa4806c262570898b88b4100ea2ee3ebc767e02fd0649074c3a4",
        other => panic!("no frozen normal workload hash for {other:?}"),
    }
}

fn expected_broken(language: &str) -> &'static str {
    match language {
        "python" => "af266a68d9826a6d003dc4e4b1609c15ff74ac4a8dd76a21217505fcbe2fdb22",
        "shell" => "1cfa59b046ee775f9d87356b3d09079c2fc06affd0d8522a22ae035139b8aa29",
        "js" => "e31877e4b67e2f702d5483e8447ae985257b66603a736529ddec58772657f8eb",
        "ts" => "2b33c2da4b6c6f2ad29b2f1f05861f0451c43891ee72baabc72efd4bffc0d127",
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
