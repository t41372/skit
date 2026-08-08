use skit_core::{build_pep723, has_pep723, inject_pep723, parse_pep723};

#[test]
fn parser_reads_dependencies_constraint_and_retains_unknown_tables() {
    let text = r#"#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12,<3.13"
# dependencies = [
#   "requests>=2,<3",
#   "rich[markdown]>=13",
# ]
# [tool.skit]
# managed = true
# ///
print("ok")
"#;
    let Some(metadata) = parse_pep723(text, "#") else {
        panic!("expected valid PEP 723 metadata");
    };
    assert_eq!(
        metadata.dependencies,
        ["requests>=2,<3", "rich[markdown]>=13"]
    );
    assert_eq!(metadata.requires_python, ">=3.12,<3.13");
    assert!(metadata.extra.contains_key("tool"));
}

#[test]
fn parser_accepts_crlf_without_rewriting_the_input() {
    let text = "# /// script\r\n# dependencies = [\r\n#   \"rich\",\r\n# ]\r\n# ///\r\nprint(1)\r\n";
    let before = text.as_bytes().to_vec();
    let Some(metadata) = parse_pep723(text, "#") else {
        panic!("expected CRLF block");
    };
    assert_eq!(metadata.dependencies, ["rich"]);
    assert_eq!(text.as_bytes(), before);
}

#[test]
fn malformed_block_is_present_but_not_parseable_and_is_never_duplicated() {
    let text = "# /// script\n# dependencies = [ nope\n# ///\nprint(1)\n";
    assert!(has_pep723(text, "#"));
    assert!(parse_pep723(text, "#").is_none());
    assert_eq!(inject_pep723(text, &["rich".to_owned()], "", "#"), text);
}

#[test]
fn block_builder_escapes_toml_strings_and_round_trips() {
    let dependencies = vec![
        "demo[one,two]>=1".to_owned(),
        "pkg; python_version >= \"3.12\"".to_owned(),
        "path\\name".to_owned(),
    ];
    let block = build_pep723(&dependencies, ">=3.12,<3.13", "#");
    let Some(metadata) = parse_pep723(&block, "#") else {
        panic!("generated block must parse");
    };
    assert_eq!(metadata.dependencies, dependencies);
    assert_eq!(metadata.requires_python, ">=3.12,<3.13");
}

#[test]
fn injection_follows_shebang_and_python_coding_declaration() {
    let source = "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\nprint('ok')\n";
    let injected = inject_pep723(source, &["rich".to_owned()], ">=3.12", "#");
    assert!(injected.starts_with(
        "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\n# /// script\n"
    ));
    assert!(injected.contains("# requires-python = \">=3.12\"\n"));
    assert!(injected.contains("#     \"rich\",\n"));
    assert!(injected.ends_with("\nprint('ok')\n"));
}

#[test]
fn crlf_injection_uses_the_source_newline_style() {
    let source = "#!/usr/bin/env python3\r\nprint('ok')\r\n";
    let injected = inject_pep723(source, &[], "", "#");
    assert!(injected.contains("# /// script\r\n# dependencies = []\r\n# ///\r\n"));
    assert!(!injected.contains("# /// script\n"));
    assert!(injected.ends_with("\r\nprint('ok')\r\n"));
}

#[test]
fn javascript_comment_leader_uses_the_same_metadata_engine() {
    let source = "#!/usr/bin/env node\nconsole.log('ok')\n";
    let injected = inject_pep723(source, &["chalk".to_owned()], "", "//");
    assert!(injected.starts_with("#!/usr/bin/env node\n// /// script\n"));
    let Some(metadata) = parse_pep723(&injected, "//") else {
        panic!("expected JS-style metadata block");
    };
    assert_eq!(metadata.dependencies, ["chalk"]);
}

#[test]
fn existing_block_is_byte_identical_on_inject() {
    let source = "# /// script\n# dependencies = [\"rich\"]\n# ///\n\nprint(1)\n";
    assert_eq!(
        inject_pep723(source, &["requests".to_owned()], ">=3.12", "#"),
        source
    );
}
