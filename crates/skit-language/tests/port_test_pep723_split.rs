//! Observable PEP 723/block-engine contracts from Python `tests/test_pep723_split.py`
//! at `main@206f9ef`.
//!
//! Python exposes `_block_re(...).pattern`, so two oracle tests pin its private regex text to stop
//! comment-leader generalization from changing accepted historical files. Rust has no regex-literal
//! product seam; the faithful architectural port pins the *observable grammar* that literal protects
//! at the public parsers instead. A mismatch is a parity failure, not a reason to weaken the test.

use skit_domain::parameters::ParamDecl;
use skit_language::{managed_params, read_uv_metadata, write_managed_params, write_uv_metadata};

#[test]
fn test_block_re_hash_pattern_is_byte_identical_to_the_frozen_literal() {
    // Representative edges encoded by Python's exact frozen `#` regex: whitespace after the opening
    // marker, a bare `#` body line, horizontal whitespace after the closing marker, and an optional
    // trailing newline before ordinary source. Existing user files in this grammar must continue to
    // parse after the engine learns other comment leaders.
    let source = concat!(
        "#!/usr/bin/env python\n",
        "# /// script   \n",
        "# dependencies = [\"requests\"]\n",
        "#\n",
        "# /// \t\n",
        "print(1)\n",
    );
    let metadata =
        read_uv_metadata(source).expect("the frozen historical # block grammar must parse");
    assert_eq!(metadata.dependencies, ["requests"]);
    assert_eq!(metadata.requires_python, "");

    // Exercising a write as well as a read catches a generalization that still recognizes the block
    // but inserts a second one instead of replacing the historical block it found.
    let rewritten = write_uv_metadata(source, &["requests".to_owned()], ">=3.11").unwrap();
    assert_eq!(
        rewritten.matches("# /// script").count(),
        1,
        "rewriting a frozen-form block must not duplicate it"
    );
    assert!(rewritten.starts_with("#!/usr/bin/env python\n"));
    assert!(rewritten.ends_with("print(1)\n"));
    let metadata =
        read_uv_metadata(&rewritten).expect("rewritten historical block must remain readable");
    assert_eq!(metadata.dependencies, ["requests"]);
    assert_eq!(metadata.requires_python, ">=3.11");
}

#[test]
fn test_block_re_double_slash_pattern_mirrors_the_hash_form() {
    // Build a real JS managed block first so this test follows the Rust architecture rather than
    // inventing a second metadata dialect. Then apply the same whitespace edges the frozen `//`
    // Python regex permits. The generalized block engine must recognize those bytes exactly as the
    // hash leader does.
    let declaration = ParamDecl::new("X");
    let generated = write_managed_params(
        "js",
        "#!/usr/bin/env node\nconst X = 5;\n",
        std::slice::from_ref(&declaration),
    )
    .unwrap();
    assert!(
        generated.contains("// /// script\n"),
        "fixture lacks the JS opening marker: {generated}"
    );
    assert!(
        generated.contains("\n// ///\n"),
        "fixture lacks the JS closing marker: {generated}"
    );

    let historical = generated
        .replacen("// /// script\n", "// /// script   \n", 1)
        .replacen("\n// ///\n", "\n// /// \t\n", 1);
    assert_ne!(
        historical, generated,
        "fixture mutation did not exercise marker whitespace"
    );
    assert!(historical.starts_with("#!/usr/bin/env node\n"));

    let parsed = managed_params("js", &historical);
    assert_eq!(
        parsed.len(),
        1,
        "the historical // block grammar stopped parsing: {historical}"
    );
    assert_eq!(parsed[0].name, "X");
}

#[test]
fn test_slash_block_round_trips_with_shebang_skip() {
    let source = "#!/usr/bin/env node\nconst X = 5;\n";
    let declaration = ParamDecl::new("X");
    let out = write_managed_params("js", source, std::slice::from_ref(&declaration)).unwrap();

    assert!(out.starts_with("#!/usr/bin/env node\n"));
    assert!(out.contains("// /// script"));
    assert!(out.find("#!").unwrap() < out.find("// /// script").unwrap());
    let parsed = managed_params("js", &out);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "X");
}

#[test]
fn test_build_block_escapes_double_quoted_marker() {
    let dependency = "requests; python_version >= \"3.8\"".to_owned();
    let block = write_uv_metadata("", std::slice::from_ref(&dependency), "").unwrap();
    let metadata = read_uv_metadata(&block).expect("generated block must parse");
    assert_eq!(metadata.dependencies, [dependency]);
}

#[test]
fn test_set_dependencies_escapes_double_quoted_marker() {
    let source = "# /// script\n# dependencies = []\n# ///\nprint(1)\n";
    let dependency = "httpx; sys_platform == \"darwin\"".to_owned();
    let out = write_uv_metadata(source, std::slice::from_ref(&dependency), "").unwrap();
    let metadata = read_uv_metadata(&out).expect("rewritten block must parse");
    assert_eq!(metadata.dependencies, [dependency]);
}

#[test]
fn test_build_block_escapes_backslash_in_dependency() {
    let dependency = "pkg; platform_release == \"5.10\\test\"".to_owned();
    let block = write_uv_metadata("", std::slice::from_ref(&dependency), "").unwrap();
    let metadata = read_uv_metadata(&block).expect("generated block must parse");
    assert_eq!(metadata.dependencies, [dependency]);
}
