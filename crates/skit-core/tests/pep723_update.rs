use skit_core::{Pep723Metadata, parse_pep723, set_pep723_axes};

fn parsed(text: &str, leader: &str) -> Pep723Metadata {
    let Some(metadata) = parse_pep723(text, leader) else {
        panic!("updated metadata block must parse");
    };
    metadata
}

#[test]
fn existing_block_updates_uv_axes_and_keeps_tool_skit_section() {
    let source = r#"# /// script
# requires-python = ">=3.11"
# dependencies = []
# [tool.skit]
# managed = true
# name = "demo"
# ///
print(1)
"#;
    let output = set_pep723_axes(source, &["requests>=2,<3".to_owned()], ">=3.12,<3.13", "#");
    let metadata = parsed(&output, "#");
    assert_eq!(metadata.dependencies, ["requests>=2,<3"]);
    assert_eq!(metadata.requires_python, ">=3.12,<3.13");
    assert!(metadata.extra.contains_key("tool"));
    assert!(output.contains("# [tool.skit]\n# managed = true\n# name = \"demo\"\n"));
}

#[test]
fn multiline_dependency_array_is_removed_through_structural_closer() {
    let source = r#"# /// script
# dependencies = [
#   "foo]bar",
#   "pkg[extra]", # pin later [
# ] # actual closer
# [tool.skit]
# keep = "yes"
# ///
print(1)
"#;
    let output = set_pep723_axes(source, &["rich".to_owned()], "", "#");
    let metadata = parsed(&output, "#");
    assert_eq!(metadata.dependencies, ["rich"]);
    assert!(output.contains("# [tool.skit]\n# keep = \"yes\"\n"));
    assert!(!output.contains("foo]bar"));
    assert!(!output.contains("pkg[extra]"));
}

#[test]
fn inline_empty_array_does_not_swallow_following_sections() {
    let source = r#"# /// script
# dependencies = []
# [tool.skit]
# name = "x"
# ///
print(1)
"#;
    let output = set_pep723_axes(source, &["requests".to_owned()], "", "#");
    assert!(output.contains("# [tool.skit]\n# name = \"x\"\n"));
    let metadata = parsed(&output, "#");
    assert_eq!(metadata.dependencies, ["requests"]);
}

#[test]
fn slash_comment_leader_threads_through_update() {
    let source = r#"// /// script
// dependencies = ["old"]
// [tool.skit]
// keep = true
// ///
console.log(1)
"#;
    let output = set_pep723_axes(source, &["new".to_owned()], "", "//");
    let metadata = parsed(&output, "//");
    assert_eq!(metadata.dependencies, ["new"]);
    assert!(output.contains("// [tool.skit]\n// keep = true\n"));
    assert!(!output.contains("\"old\""));
}

#[test]
fn no_existing_block_falls_back_to_injection() {
    let source = "#!/usr/bin/env python3\nprint(1)\n";
    let output = set_pep723_axes(source, &["rich".to_owned()], ">=3.12", "#");
    let metadata = parsed(&output, "#");
    assert_eq!(metadata.dependencies, ["rich"]);
    assert_eq!(metadata.requires_python, ">=3.12");
    assert!(output.starts_with("#!/usr/bin/env python3\n# /// script\n"));
}

#[test]
fn crlf_existing_block_keeps_crlf_when_rewritten() {
    let source = "# /// script\r\n# dependencies = [\"old\"]\r\n# [tool.skit]\r\n# keep = true\r\n# ///\r\nprint(1)\r\n";
    let output = set_pep723_axes(source, &["rich".to_owned()], ">=3.12", "#");
    assert!(output.contains("# requires-python = \">=3.12\"\r\n"));
    assert!(output.contains("# [tool.skit]\r\n# keep = true\r\n"));
    assert!(!output.contains("# dependencies = [\"old\"]\r\n"));
    assert!(!output.contains("# /// script\n"));
}

#[test]
fn malformed_existing_toml_is_repaired_without_duplicating_block() {
    let source = "# /// script\n# dependencies = [ broken\n# ]\n# [tool.skit]\n# keep = true\n# ///\nprint(1)\n";
    let output = set_pep723_axes(source, &["rich".to_owned()], "", "#");
    assert_eq!(output.matches("# /// script").count(), 1);
    let metadata = parsed(&output, "#");
    assert_eq!(metadata.dependencies, ["rich"]);
    assert!(output.contains("# [tool.skit]\n# keep = true\n"));
}
