//! Mechanical port of the Python oracle module `tests/test_metawriter.py`
//! (`origin/main@206f9ef`): "MetaWriter ([tool.skit] plain-text writes): A5 fidelity,
//! round-trip, idempotent replacement." Each `#[test]` keeps its Python `def test_*`
//! name so it traces back to its origin, and each Python "WHY" comment is preserved
//! above it.
//!
//! Concept mapping used throughout:
//! - Python `metawriter.write_params(src, params)` -> `write_managed_params("python", src, &decls)`
//!   (the Rust writer returns a `Result`; valid input `.unwrap()`s).
//! - Python `metawriter.read_params(text)` -> `managed_params("python", text)`.
//! - Python `pep723.has_block(text)` -> `has_uv_metadata_block(text)`.
//! - Python `pep723.parse_block(text)` -> `read_uv_metadata(text)` (its `dependencies` /
//!   `requires_python` fields cover every assertion these tests make on the parsed dict).
//! - Python `pep723.set_dependencies(text, deps, req)` -> `write_uv_metadata(text, &deps, req)`.
//! - Python `ParamDecl.from_block_dict(d)` -> `ParamDecl::from_block_map(&map)` (skit-domain).
//! - Python `compile(out, "<test>", "exec")` (injected section is pure comments) ->
//!   `source_is_valid("python", &out)`: the output still parses as valid Python.
//!
//! Buckets:
//! - Bucket 1 (block-writer / round-trip byte-logic): the bulk below; asserts on exact bytes.
//! - Bucket 2 (white-box private helpers): the two `_structural_bracket_delta` tests are
//!   `#[ignore]`d — the Rust `write_uv_metadata` parses the block as TOML rather than tracking
//!   bracket depth line-by-line, so that Python-private helper has no public equivalent.
//! - Bucket 3 (CLI/store integration): NONE. This oracle module drives only pure functions
//!   (`metawriter` / `pep723`); no CliRunner, store, or atomic-write path is exercised.

use std::collections::BTreeMap;

use serde_json::Value;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    has_uv_metadata_block, managed_params, read_uv_metadata, source_is_valid, write_managed_params,
    write_uv_metadata,
};

// --- The oracle's module-level PARAMS fixture ---
//
// PARAMS = [
//     ParamDecl(name="API_KEY", binding="const", type="str", default="abc", secret=True),
//     ParamDecl(name="RETRIES", binding="const", type="int", default=3),
//     ParamDecl(name="input-1", binding="input", type="str", prompt="City: ", order=0),
// ]

fn api_key() -> ParamDecl {
    let mut declaration = ParamDecl::new("API_KEY");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = Some(ParameterValue::String("abc".to_owned()));
    declaration.secret = true;
    declaration
}

fn retries() -> ParamDecl {
    let mut declaration = ParamDecl::new("RETRIES");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Int;
    declaration.default = Some(ParameterValue::Integer(3));
    declaration
}

fn input_one() -> ParamDecl {
    let mut declaration = ParamDecl::new("input-1");
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.prompt = "City: ".to_owned();
    declaration.order = 0;
    declaration
}

/// The full three-parameter PARAMS fixture, in oracle order.
fn params() -> Vec<ParamDecl> {
    vec![api_key(), retries(), input_one()]
}

/// Names of the declarations read back from a script, in stored order.
fn read_names(text: &str) -> Vec<String> {
    managed_params("python", text)
        .into_iter()
        .map(|declaration| declaration.name)
        .collect()
}

/// Python `[ln for ln in out.splitlines(keepends=True) if ln not in src.splitlines(keepends=True)]`.
/// `str::split_inclusive('\n')` mirrors `splitlines(keepends=True)` for these ASCII/LF sources.
fn added_lines<'a>(out: &'a str, src: &str) -> Vec<&'a str> {
    let source_lines = src.split_inclusive('\n').collect::<Vec<_>>();
    out.split_inclusive('\n')
        .filter(|line| !source_lines.contains(line))
        .collect()
}

#[test]
fn test_write_creates_block_when_missing() {
    let src = "print('hi')\n";
    let out = write_managed_params("python", src, &params()).unwrap();
    assert!(has_uv_metadata_block(&out));
    assert!(out.contains("print('hi')")); // user code is untouched
    assert_eq!(read_names(&out), ["API_KEY", "RETRIES", "input-1"]);
}

#[test]
fn test_write_creates_block_adds_no_line_outside_the_block() {
    // Regression: fixing _BLOCK_RE's greedy closer (blank-line-swallowing bug) exposed that this
    // "no block yet" path recurses through pep723.inject_block(), which inserts a blank-line
    // separator before the following code for its own (standalone) readability. write_params()
    // immediately overwrites that same block's body with params in the same call, so the separator
    // must not survive — it would be the only line ever added outside the "# /// … # ///" block,
    // violating the comment-only-edits contract (A5) and the corpus byte-fidelity invariant.
    let src = "CITY = 'Taipei'\nprint(CITY)\n";
    let out = write_managed_params("python", src, &params()[..1]).unwrap();
    let added = added_lines(&out, src);
    assert!(
        added.iter().all(|line| line.trim_start().starts_with('#')),
        "{added:?}"
    );
    // block directly adjacent to the code, no blank line
    assert!(out.ends_with(format!("# ///\n{src}").as_str()));
}

#[test]
fn test_write_creates_block_after_shebang_adds_no_line_outside_the_block() {
    let src = "#!/usr/bin/env python3\nCITY = 'Taipei'\nprint(CITY)\n";
    let out = write_managed_params("python", src, &params()[..1]).unwrap();
    let added = added_lines(&out, src);
    assert!(
        added.iter().all(|line| line.trim_start().starts_with('#')),
        "{added:?}"
    );
    assert!(out.starts_with("#!/usr/bin/env python3\n"));
    assert!(out.ends_with("# ///\nCITY = 'Taipei'\nprint(CITY)\n"));
}

#[test]
fn test_write_creates_block_preserves_a_pre_existing_leading_blank_line() {
    // When the source body already begins with a blank line at the block insertion point,
    // inject_block skips its synthetic separator, so _drop_synthetic_separator must return the
    // base unchanged — the user's own blank line survives, and no line is dropped or doubled.
    let src = "\nprint(1)\n";
    let out = write_managed_params("python", src, &params()[..1]).unwrap();
    assert_eq!(read_names(&out), ["API_KEY"]);
    // exactly one blank line between the block closer and the code (the original's own),
    // not zero, not two
    assert!(out.ends_with("# ///\n\nprint(1)\n"));
    assert!(!out.contains("# ///\n\n\nprint(1)\n"));
    // nothing outside the comment block except the untouched original body
    let added = added_lines(&out, src);
    assert!(
        added.iter().all(|line| line.trim_start().starts_with('#')),
        "{added:?}"
    );
}

#[test]
fn test_roundtrip_types_and_fields() {
    let out = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let got = managed_params("python", &out)
        .into_iter()
        .map(|declaration| (declaration.name.clone(), declaration))
        .collect::<BTreeMap<_, _>>();
    assert!(got["API_KEY"].secret);
    assert_eq!(
        got["API_KEY"].default,
        Some(ParameterValue::String("abc".to_owned()))
    );
    assert_eq!(got["RETRIES"].default, Some(ParameterValue::Integer(3)));
    assert_eq!(got["RETRIES"].parameter_type, ParameterType::Int);
    assert_eq!(got["input-1"].order, 0);
    assert_eq!(got["input-1"].prompt, "City: ");
}

#[test]
fn test_preserves_existing_dependencies() {
    let src = concat!(
        "# /// script\n",
        "# requires-python = \">=3.11\"\n",
        "# dependencies = [\n",
        "#     \"requests\",\n",
        "# ]\n",
        "# ///\n",
        "import requests\n",
    );
    let out = write_managed_params("python", src, &params()).unwrap();
    let meta = read_uv_metadata(&out).expect("metadata block present");
    assert_eq!(meta.dependencies, ["requests"]);
    assert_eq!(meta.requires_python, ">=3.11");
    assert_eq!(managed_params("python", &out).len(), 3);
}

#[test]
fn test_rewrite_replaces_not_duplicates() {
    let out1 = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let out2 = write_managed_params("python", &out1, &params()[..1]).unwrap();
    assert_eq!(read_names(&out2), ["API_KEY"]);
    assert_eq!(out2.matches("[tool.skit]").count(), 1);
}

#[test]
fn test_empty_params_removes_section() {
    let out1 = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let out2 = write_managed_params("python", &out1, &[]).unwrap();
    assert!(managed_params("python", &out2).is_empty());
    assert!(!out2.contains("[tool.skit]"));
    // the PEP 723 block itself remains (dependencies preserved)
    assert!(has_uv_metadata_block(&out2));
}

#[test]
fn test_string_escaping() {
    let mut spec = ParamDecl::new("MSG");
    spec.binding = ParameterBinding::Const;
    spec.delivery = ParameterDelivery::Inject;
    spec.default = Some(ParameterValue::String(r#"say "hi" \ bye"#.to_owned()));
    let out = write_managed_params("python", "x = 1\n", &[spec]).unwrap();
    let got = managed_params("python", &out);
    assert_eq!(
        got[0].default,
        Some(ParameterValue::String(r#"say "hi" \ bye"#.to_owned()))
    );
}

#[test]
fn test_shebang_preserved_first() {
    let src = "#!/usr/bin/env python3\nprint('x')\n";
    let out = write_managed_params("python", src, &params()[..1]).unwrap();
    assert!(out.starts_with("#!/usr/bin/env python3\n"));
}

#[test]
fn test_script_still_valid_python() {
    let src = "CITY = 'Taipei'\nprint(CITY)\n";
    let out = write_managed_params("python", src, &params()).unwrap();
    // injected section is pure comments; semantics unchanged (A5)
    assert!(source_is_valid("python", &out));
}

#[test]
fn test_set_dependencies_preserves_tool_skit() {
    // Updating deps must not destroy [tool.skit] parameter definitions (core constraint of
    // `skit deps`).
    let out = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let updated =
        write_uv_metadata(&out, &["requests".to_owned(), "rich".to_owned()], ">=3.12").unwrap();
    let meta = read_uv_metadata(&updated).expect("metadata block present");
    assert_eq!(meta.dependencies, ["requests", "rich"]);
    assert_eq!(meta.requires_python, ">=3.12");
    assert_eq!(read_names(&updated), ["API_KEY", "RETRIES", "input-1"]);
    // Clear deps; params must still be there
    let cleared = write_uv_metadata(&updated, &[], "").unwrap();
    let cleared_meta = read_uv_metadata(&cleared).expect("metadata block present");
    assert_eq!(cleared_meta.dependencies, Vec::<String>::new());
    assert_eq!(managed_params("python", &cleared).len(), 3);
    assert!(source_is_valid("python", &cleared));
}

#[test]
fn test_set_dependencies_without_block_injects() {
    let out = write_uv_metadata("print('x')\n", &["httpx".to_owned()], "").unwrap();
    let meta = read_uv_metadata(&out).expect("metadata block present");
    assert_eq!(meta.dependencies, ["httpx"]);
}

// --- set_dependencies must survive a hand-edited (not skit-generated) deps array closer ---
//
// The skit-generated form always puts the closing "]" alone on its own line, which is why the bug
// (the `in_deps_array` flag never resetting) went unnoticed: only hand-edited variants trigger it.

/// Oracle helper: the source that wraps one hand-edited `deps_block` in a block whose
/// `[tool.skit]` params section must survive a `set_dependencies` rewrite.
fn hand_edited_src(deps_block: &str) -> String {
    format!(
        "# /// script\n{deps_block}#\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# name = \"API_KEY\"\n# ///\n"
    )
}

fn assert_set_dependencies_survives(deps_block: &str) {
    let src = hand_edited_src(deps_block);
    // Sanity: read_params on the untouched hand-edited block already sees the param.
    assert_eq!(read_names(&src), ["API_KEY"]);
    let updated = write_uv_metadata(&src, &["httpx".to_owned()], "").unwrap();
    let meta = read_uv_metadata(&updated).expect("metadata block present");
    assert_eq!(meta.dependencies, ["httpx"]);
    // The [tool.skit] params block must survive — this is the core bug: the old line-shape
    // assumption (closer must be alone on its line) dropped the entire rest of the block body.
    assert_eq!(read_names(&updated), ["API_KEY"]);
    assert!(source_is_valid("python", &updated));
}

#[test]
fn test_set_dependencies_survives_hand_edited_deps_closer_close_on_last_element_line() {
    assert_set_dependencies_survives("# dependencies = [\n#     \"requests\"]\n");
}

#[test]
fn test_set_dependencies_survives_hand_edited_deps_closer_trailing_comment_on_closer() {
    assert_set_dependencies_survives("# dependencies = [\n#     \"requests\",\n# ]  # pin\n");
}

#[test]
fn test_set_dependencies_survives_hand_edited_deps_closer_multi_item_close_on_last() {
    assert_set_dependencies_survives("# dependencies = [\n#     \"a\",\n#     \"b\"]\n");
}

#[test]
fn test_set_dependencies_survives_hand_edited_deps_closer_extras_bracket_in_requirement_string() {
    assert_set_dependencies_survives("# dependencies = [\n#     \"pkg[extra]\",\n# ]\n");
}

#[test]
fn test_set_dependencies_survives_hand_edited_deps_closer_comment_with_bracket() {
    assert_set_dependencies_survives(
        "# dependencies = [\n#     \"requests\",  # pin later [\n#     \"httpx\",\n# ]\n",
    );
}

#[test]
fn test_set_dependencies_survives_hand_edited_deps_closer_string_with_bracket() {
    assert_set_dependencies_survives("# dependencies = [\n#     \"foo]bar\",\n# ]\n");
}

// --- Bracket-depth tracking must ignore brackets inside TOML strings and inline comments, not
// just count them naively over the whole line. A raw
// `line.count("[") - line.count("]")`, which itself desyncs on a `[`/`]` living inside a string
// value or an inline `#` comment and can drop the following parameter block.

#[test]
fn test_set_dependencies_handles_unbalanced_bracket_in_inline_comment() {
    // An in-array comment containing an unbalanced `[` must not desync the depth counter
    // and swallow the following `# [tool.skit]` params block.
    let src = concat!(
        "# /// script\n",
        "# dependencies = [\n",
        "#     \"requests\",  # pin later [\n",
        "#     \"httpx\",\n",
        "# ]\n",
        "#\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"API_KEY\"\n",
        "# ///\n",
    );
    // Sanity: the untouched hand-edited block already parses to the one param.
    assert_eq!(read_names(src), ["API_KEY"]);
    let updated = write_uv_metadata(src, &["rich".to_owned()], "").unwrap();
    let meta = read_uv_metadata(&updated).expect("metadata block present");
    assert_eq!(meta.dependencies, ["rich"]);
    // Before the fix this returned [] — the whole [tool.skit] section was silently dropped.
    assert_eq!(read_names(&updated), ["API_KEY"]);
    assert!(source_is_valid("python", &updated));
}

// --- pep723._structural_bracket_delta: direct branch coverage for string-quoting edge cases not
// otherwise exercised by the set_dependencies-level tests above (escaped quotes in a basic string,
// and literal ('...') strings, which TOML never escapes). ---

#[test]
#[ignore = "UNMAPPED (bucket 2): white-box test of the Python-private helper pep723._structural_bracket_delta. The Rust write_uv_metadata parses the whole comment block as TOML (toml::from_str) instead of tracking structural bracket depth line-by-line, so this helper has no public equivalent to observe."]
fn test_structural_bracket_delta_escaped_quote_in_basic_string() {
    // A backslash-escaped quote inside a basic ("...") string must not end the string early, so a
    // bracket immediately after it (still inside the string) is not counted.
    // assert pep723._structural_bracket_delta('"a\\"]b"') == 0
}

#[test]
#[ignore = "UNMAPPED (bucket 2): white-box test of the Python-private helper pep723._structural_bracket_delta. The Rust write_uv_metadata parses the whole comment block as TOML (toml::from_str) instead of tracking structural bracket depth line-by-line, so this helper has no public equivalent to observe."]
fn test_structural_bracket_delta_literal_string_has_no_escapes() {
    // TOML literal ('...') strings never treat backslash as an escape: it is a literal character,
    // and a bracket following it (still inside the string) must not be counted either.
    // assert pep723._structural_bracket_delta("'a\\]b'") == 0
}

// --- _BLOCK_RE's closer must not swallow blank lines that follow the block ---

#[test]
fn test_write_params_preserves_blank_lines_after_block() {
    let src = "# /// script\n# dependencies = []\n# ///\n\n\nimport requests\n";
    let out = write_managed_params("python", src, &params()[..1]).unwrap();
    // Exactly the two original blank lines must still separate the block from the following code.
    let suffix = out.split_once("# ///\n").expect("closer present").1;
    assert_eq!(suffix, "\n\nimport requests\n");
}

#[test]
fn test_set_dependencies_preserves_blank_lines_after_block() {
    let src = "# /// script\n# dependencies = []\n# ///\n\n\nimport requests\n";
    let out = write_uv_metadata(src, &["httpx".to_owned()], "").unwrap();
    let suffix = out.split_once("# ///\n").expect("closer present").1;
    assert_eq!(suffix, "\n\nimport requests\n");
}

// --- read_params must be total: malformed-but-valid TOML shapes return [] instead of raising ---

#[test]
fn test_read_params_tolerates_malformed_container_shapes_tool_is_scalar() {
    let src = "# /// script\n# dependencies = []\n# tool = 5\n# ///\n";
    assert!(managed_params("python", src).is_empty());
}

#[test]
fn test_read_params_tolerates_malformed_container_shapes_skit_is_scalar() {
    let src = "# /// script\n# dependencies = []\n# [tool]\n# skit = 5\n# ///\n";
    assert!(managed_params("python", src).is_empty());
}

#[test]
fn test_read_params_tolerates_malformed_container_shapes_params_is_scalar() {
    let src = "# /// script\n# dependencies = []\n# [tool.skit]\n# params = 5\n# ///\n";
    assert!(managed_params("python", src).is_empty());
}

#[test]
fn test_read_params_tolerates_non_numeric_order() {
    let src = concat!(
        "# /// script\n",
        "# dependencies = []\n",
        "#\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"X\"\n",
        "# order = \"abc\"\n",
        "# ///\n",
    );
    let got = managed_params("python", src);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "X");
    assert_eq!(got[0].order, -1); // uncoercible -> falls back rather than raising ValueError
}

#[test]
fn test_from_dict_coerces_non_numeric_order() {
    let non_numeric = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("order".to_owned(), Value::String("abc".to_owned())),
    ]);
    assert_eq!(ParamDecl::from_block_map(&non_numeric).order, -1);
    let null_order = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("order".to_owned(), Value::Null),
    ]);
    assert_eq!(ParamDecl::from_block_map(&null_order).order, -1);
}

#[test]
fn test_from_dict_still_coerces_numeric_string_and_float_order() {
    // The fix must not regress previously-working coercions (only non-numeric values fall back).
    let numeric_string = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("order".to_owned(), Value::String("3".to_owned())),
    ]);
    assert_eq!(ParamDecl::from_block_map(&numeric_string).order, 3);
    let float_order = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("order".to_owned(), serde_json::json!(1.9)),
    ]);
    assert_eq!(ParamDecl::from_block_map(&float_order).order, 1);
}

#[test]
fn test_write_params_survives_unicode_line_separators_in_prompt() {
    // str.splitlines() breaks on U+0085/U+2028/U+2029 as well as newlines; if _toml_str
    // leaves one raw, _commentify shreds the comment body and every managed param definition
    // is lost on the next read. Escaping them keeps the block whole and round-trips the value.
    for separator in ['\u{85}', '\u{2028}', '\u{2029}'] {
        let mut spec = ParamDecl::new("CITY");
        spec.binding = ParameterBinding::Const;
        spec.delivery = ParameterDelivery::Inject;
        spec.parameter_type = ParameterType::Str;
        spec.default = Some(ParameterValue::String("x".to_owned()));
        spec.prompt = format!("a{separator}b");
        let text = write_managed_params("python", "CITY = 'x'\n", &[spec]).unwrap();
        let back = managed_params("python", &text);
        assert_eq!(back.len(), 1, "block lost for separator {separator:?}");
        assert_eq!(back[0].prompt, format!("a{separator}b"));
    }
}
