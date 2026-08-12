//! Observable PEP 723/block-engine contracts from Python `tests/test_pep723_split.py`
//! at `main@206f9ef`.
//!
//! The Python-only `_block_re(...).pattern` string assertions are intentionally not reproduced: Rust
//! does not expose or depend on that Python regex literal. The behavior those regexes protect is
//! covered here at the public source-edit boundary instead.

use skit_domain::parameters::ParamDecl;
use skit_language::{managed_params, read_uv_metadata, write_managed_params, write_uv_metadata};

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
