//! Mechanical port of the Python oracle module `tests/test_pep723_split.py`
//! (`origin/main@206f9ef`): "split_requirements: comma-splitting that respects PEP 508 internals",
//! plus the `# /// script` block regex generalization and the `build_block` / `set_dependencies`
//! TOML-escaping round-trips. Each `#[test]` keeps its exact Python `def test_*` name, and each
//! Python "WHY" comment (or the frozen input/expected pair) is preserved above/inside it.
//!
//! Concept mapping used throughout:
//! - Python `pep723.set_dependencies(text, deps)` -> `write_uv_metadata(text, &deps, "")`.
//! - Python `pep723.build_block([dep])` -> `write_uv_metadata("", &[dep], "")` (no 1:1 exists; a
//!   write onto an empty source produces exactly the standalone block that `build_block` returns).
//! - Python `pep723.parse_block(text)` -> `read_uv_metadata(text)` (its `dependencies` field covers
//!   every assertion these tests make on the parsed dict).
//!
//! Buckets:
//! - Bucket 1 (block-writer / round-trip byte-logic): the three `build_block` / `set_dependencies`
//!   escaping tests below; asserted on the parsed-back `dependencies` values.
//! - Bucket 2 (white-box Python internals): `_block_re(...).pattern`, `_next_nonspace(...)`, and the
//!   `// /// script` (deps) injection path — no public skit-language equivalent (see each WHY).
//! - Bucket 3 (off-crate / CLI+store): the whole `split_requirements` family and the three CLI call
//!   sites — see the central FINDING below.
//!
//! CENTRAL FINDING for the supervisor: the module's documented subject, `pep723.split_requirements`
//! (14 unit tests), maps to `skit_ui::add::split_pep508_requirements`, which is `pub(crate)` in the
//! **skit-ui** crate — NOT in `skit-language`, and NOT publicly reachable from any integration test.
//! `skit-language` exposes only the single-item validator `validate_pep508_requirement`, never a
//! requirement splitter. So the comma-splitting byte-logic cannot be verified from this crate. Those
//! 14 tests are therefore ported as `#[ignore]` stubs (with their frozen input/expected preserved),
//! not because the behavior matches, but because the function lives off-crate behind `pub(crate)`.

use skit_language::{read_uv_metadata, write_uv_metadata};

// ---------------------------------------------------------------- comment-leader generalization

#[test]
#[ignore = "UNMAPPED (bucket 2, white-box): asserts the exact frozen text of the Python-private regex `pep723._block_re(\"#\").pattern`. The Rust fence detector is the private `block_regex(\"#\")`, whose pattern is deliberately a DIFFERENT string (it captures a named `close` group and tolerates CRLF), so no public equivalent reproduces the frozen literal."]
fn test_block_re_hash_pattern_is_byte_identical_to_the_frozen_literal() {
    // frozen = r"(?m)^# /// script\s*$\n(?P<body>(?:^#(?:| .*)$\n)*?)^# ///[^\S\n]*$\n?"
    // assert pep723._block_re("#").pattern == frozen
}

#[test]
#[ignore = "UNMAPPED (bucket 2, white-box): asserts the exact frozen text of the Python-private regex `pep723._block_re(\"//\").pattern`. Same reason as the `#` variant: the Rust `block_regex` is private and uses a different pattern string."]
fn test_block_re_double_slash_pattern_mirrors_the_hash_form() {
    // assert pep723._block_re("//").pattern == (
    //     r"(?m)^// /// script\s*$\n(?P<body>(?:^//(?:| .*)$\n)*?)^// ///[^\S\n]*$\n?"
    // )
}

#[test]
#[ignore = "UNMAPPED (bucket 2, no public `//` deps path): `pep723.inject_block(src, [], leader=\"//\")` injects a `// /// script` block whose body is `dependencies = []`. skit-language's deps writer/reader (`write_uv_metadata` / `read_uv_metadata` / `has_uv_metadata_block`) are all hardwired to the Python `#` leader; the only `//`-leader path (`write_managed_params`/`managed_params` for kind \"js\") carries a `[tool.skit]` params table, never a bare `dependencies` block. No public function reproduces this injection."]
fn test_slash_block_round_trips_with_shebang_skip() {
    // src = "#!/usr/bin/env node\nconst X = 5;\n"
    // out = pep723.inject_block(src, [], leader="//")
    // assert pep723.has_block(out, "//")
    // assert out.startswith("#!/usr/bin/env node\n")
    // assert out.index("#!") < out.index("// /// script")
    // assert pep723.parse_block(out, "//") == {"dependencies": []}
}

// --------------------------------------------------------------------------
// unit: split_requirements  (Rust equivalent: skit_ui::add::split_pep508_requirements, pub(crate))
// --------------------------------------------------------------------------
//
// Every test below is off-crate: the splitter is `pub(crate)` in skit-ui and skit-language exposes
// no requirement splitter. Ports keep the frozen input/expected pair for later re-homing in a
// skit-ui unit test. Reason string is identical across the family.

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_simple_list_splits() {
    // assert pep723.split_requirements("requests, rich") == ["requests", "rich"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_single_item_no_commas() {
    // assert pep723.split_requirements("requests") == ["requests"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_specifier_commas_stay_joined() {
    // assert pep723.split_requirements("requests>=2,<3") == ["requests>=2,<3"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_specifier_lists_split_only_between_requirements() {
    // assert pep723.split_requirements("requests>=2,<3, pillow!=9.0,>=8") == [
    //     "requests>=2,<3",
    //     "pillow!=9.0,>=8",
    // ]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_spaces_around_specifier_commas() {
    // The continuation clause may be padded with spaces; the comma still belongs
    // to the specifier because what follows is an operator, not a name.
    // assert pep723.split_requirements("foo >= 1 , < 2 , bar") == ["foo >= 1 , < 2", "bar"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_extras_bracket_commas_stay_joined() {
    // assert pep723.split_requirements("requests[security,socks]>=2, rich") == [
    //     "requests[security,socks]>=2",
    //     "rich",
    // ]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_parenthesized_specifier_commas_stay_joined() {
    // assert pep723.split_requirements("foo (>=1.0,<2.0), bar") == ["foo (>=1.0,<2.0)", "bar"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_double_quoted_marker_comma_stays_joined() {
    // assert pep723.split_requirements('a; sys_platform in "linux,darwin", b') == [
    //     'a; sys_platform in "linux,darwin"',
    //     "b",
    // ]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_single_quoted_marker_comma_stays_joined() {
    // assert pep723.split_requirements("a; extra in 'x,y', b") == ["a; extra in 'x,y'", "b"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_name_starting_with_digit_splits() {
    // PEP 508 names may start with a digit; isalnum (not isalpha) is the predicate.
    // assert pep723.split_requirements("rich, 2captcha-python") == ["rich", "2captcha-python"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_trailing_comma_dropped() {
    // assert pep723.split_requirements("requests>=2,<3,") == ["requests>=2,<3"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_empty_and_blank_input() {
    // assert pep723.split_requirements("") == []
    // assert pep723.split_requirements("   ") == []
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_uppercase_x_in_name_is_ordinary_text() {
    // Guards the bracket character classes against corruption: an 'X' in a package
    // name must not perturb the bracket-nesting depth (kills the "XX([XX" mutants).
    // assert pep723.split_requirements("pkgX, rich") == ["pkgX", "rich"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, off-crate): pep723.split_requirements -> skit_ui::add::split_pep508_requirements, pub(crate) in skit-ui, unreachable from a skit-language test. See CENTRAL FINDING."]
fn test_nested_brackets_tracked_by_depth_not_flag() {
    // Depth must accumulate (+=), not be pinned to 1: with nesting, a pinned depth
    // hits zero at the first closer and lets an inner comma split mid-requirement.
    // assert pep723.split_requirements("a[[x],y], b") == ["a[[x],y]", "b"]
}

#[test]
#[ignore = "UNMAPPED (bucket 2, white-box): `pep723._next_nonspace(text, i)` is a Python-private helper. The Rust splitter (`split_pep508_requirements`) never exposes a next-non-space probe; it drives partitioning through `validate_pep508_requirement`, so this helper has no public equivalent to observe."]
fn test_next_nonspace_end_of_text_is_empty_string() {
    // The trailing-comma path relies on the exact "" sentinel; through the caller a
    // non-empty alnum return is coincidentally equivalent, so pin the contract here.
    // assert pep723._next_nonspace("a,  ", 2) == ""
    // assert pep723._next_nonspace("a, b", 2) == "b"
}

// --------------------------------------------------------------------------
// CLI call sites
// --------------------------------------------------------------------------

#[test]
#[ignore = "UNMAPPED (bucket 3, CLI/store): drives `runner.invoke(cli.app, [\"add\", ...])` then `store.resolve`. This is the Clap/composition-root + store path in skit-cli, not a skit-language function. -> Tier 3/4."]
fn test_add_dep_flags_carry_specifier_commas() {
    // result = runner.invoke(cli.app, ["add", p, "--name", "r", "--dep", "requests>=2,<3",
    //     "--dep", "rich", "--no-input"])
    // entry = store.resolve("r")
    // block = pep723.parse_block(entry.script_path.read_text())
    // assert block["dependencies"] == ["requests>=2,<3", "rich"]
}

#[test]
#[ignore = "UNMAPPED (bucket 3, CLI internal): drives `cli._resolve_python_metadata(...)`, a skit-cli composition-root helper (Prompt.ask + interactive deps), not a skit-language function. -> Tier 3/4."]
fn test_interactive_deps_answer_keeps_specifier_commas() {
    // deps, py = cli._resolve_python_metadata("import requests\nprint(requests)\n", None, None,
    //     no_input=False)  with Prompt.ask answering "requests>=2,<3, rich" then ""
    // assert deps == ["requests>=2,<3", "rich"]
    // assert py == ""
}

#[test]
#[ignore = "UNMAPPED (bucket 3, CLI/store): drives `store.add_python` + `runner.invoke(cli.app, [\"deps\", ...])` and reads back `store.resolve(...).meta.dependencies`. skit-cli + store path, not a skit-language function. -> Tier 3/4."]
fn test_deps_dep_flags_carry_specifier_commas() {
    // store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    // runner.invoke(cli.app, ["deps", "a", "--dep", "requests>=2,<3", "--dep", "rich"])
    // assert store.resolve("a").meta.dependencies == ["requests>=2,<3", "rich"]
}

// ---------------------------------------------------------------------------
// build_block / set_dependencies: TOML-string escaping of dependency values
// ---------------------------------------------------------------------------

#[test]
fn test_build_block_escapes_double_quoted_marker() {
    // A PEP 508 marker carries embedded double quotes (python_version >= "3.8"). Emitted
    // raw into a "..." TOML string it would terminate the string early, so the generated
    // block fails to re-parse and the whole dependency list is silently lost. The block must
    // round-trip through parse_block intact.
    let dep = r#"requests; python_version >= "3.8""#;
    // build_block([dep]) -> write onto an empty source produces the same standalone block.
    let block = write_uv_metadata("", &[dep.to_owned()], "").unwrap();
    let meta = read_uv_metadata(&block).expect("generated block does not parse");
    assert_eq!(meta.dependencies, [dep]);
}

#[test]
fn test_set_dependencies_escapes_double_quoted_marker() {
    let text = "# /// script\n# dependencies = []\n# ///\nprint(1)\n";
    let dep = r#"httpx; sys_platform == "darwin""#;
    let out = write_uv_metadata(text, &[dep.to_owned()], "").unwrap();
    let meta = read_uv_metadata(&out).expect("generated block does not parse");
    assert_eq!(meta.dependencies, [dep]);
}

#[test]
fn test_build_block_escapes_backslash_in_dependency() {
    // Python literal 'pkg; platform_release == "5.10\\test"' is the value below: a single
    // backslash before `test`. A raw Rust string keeps that byte exact.
    let dep = r#"pkg; platform_release == "5.10\test""#;
    let block = write_uv_metadata("", &[dep.to_owned()], "").unwrap();
    let meta = read_uv_metadata(&block).expect("generated block does not parse");
    assert_eq!(meta.dependencies, [dep]);
}
