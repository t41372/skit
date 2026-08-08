use skit_core::{
    Binding, Delivery, ParamDecl, ParamDefault, ParamType, read_python_params,
    render_python_params, write_python_params,
};

fn sample_params() -> Vec<ParamDecl> {
    vec![
        ParamDecl {
            name: "CITY".to_owned(),
            binding: Binding::Const,
            delivery: Delivery::Inject,
            param_type: ParamType::String,
            default: Some(ParamDefault::String("Taipei\n市".to_owned())),
            secret: false,
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "input-1".to_owned(),
            binding: Binding::Input,
            delivery: Delivery::Inject,
            param_type: ParamType::String,
            prompt: "Password: ".to_owned(),
            order: 0,
            secret: true,
            env_source: "APP_PASSWORD".to_owned(),
            ..ParamDecl::default()
        },
    ]
}

#[test]
fn frozen_render_shape_matches_python_era_contract() {
    assert_eq!(
        render_python_params(&sample_params()),
        r#"[tool.skit]
schema = 1

[[tool.skit.params]]
name = "CITY"
kind = "const"
type = "str"
default = "Taipei\n市"

[[tool.skit.params]]
name = "input-1"
kind = "input"
type = "str"
prompt = "Password: "
order = 0
secret = true
env_source = "APP_PASSWORD"
"#
    );
}

#[test]
fn existing_block_replaces_only_tool_skit_and_preserves_other_lines() {
    let source = r#"#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "requests>=2,<3",
# ]
# [tool.other]
# exact = "keep me"
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "OLD"
# kind = "const"
# type = "str"
# default = "gone"
#
# [tool.after]
# also = "keep"
# ///
print("ok")
"#;
    let output = write_python_params(source, &sample_params());
    assert!(output.contains(
        "# requires-python = \">=3.12\"\n# dependencies = [\n#     \"requests>=2,<3\",\n# ]\n"
    ));
    assert!(output.contains("# [tool.other]\n# exact = \"keep me\"\n"));
    assert!(output.contains("# [tool.after]\n# also = \"keep\"\n"));
    assert!(!output.contains("name = \"OLD\""));
    assert_eq!(read_python_params(&output), sample_params());
    assert!(output.ends_with("# ///\nprint(\"ok\")\n"));
}

#[test]
fn no_block_inserts_one_after_shebang_and_coding_without_external_blank_line() {
    let source = "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\nprint('ok')\n";
    let output = write_python_params(source, &sample_params());
    assert!(output.starts_with(
        "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\n# /// script\n# dependencies = []\n#\n# [tool.skit]\n"
    ));
    assert!(output.contains("# ///\nprint('ok')\n"));
    assert!(!output.contains("# ///\n\nprint('ok')"));
    assert_eq!(output.matches("# /// script").count(), 1);
}

#[test]
fn crlf_block_stays_crlf_and_outside_code_bytes_are_unchanged() {
    let source = "# /// script\r\n# dependencies = []\r\n# [tool.skit]\r\n# schema = 1\r\n# ///\r\nprint('ok')\r\n";
    let output = write_python_params(source, &sample_params());
    assert!(output.contains("# [tool.skit]\r\n# schema = 1\r\n"));
    assert!(output.ends_with("# ///\r\nprint('ok')\r\n"));
    assert!(!output.contains("# /// script\n"));
    assert_eq!(read_python_params(&output), sample_params());
}

#[test]
fn empty_params_remove_only_tool_skit_and_leave_block_and_dependencies() {
    let source = r#"# /// script
# dependencies = ["rich"]
# [tool.skit]
# schema = 1
# [[tool.skit.params]]
# name = "X"
# kind = "const"
# type = "int"
# default = 1
# [tool.other]
# x = 1
# ///
print(1)
"#;
    let output = write_python_params(source, &[]);
    assert!(output.contains("# dependencies = [\"rich\"]\n"));
    assert!(output.contains("# [tool.other]\n# x = 1\n"));
    assert!(!output.contains("tool.skit"));
    assert!(read_python_params(&output).is_empty());
}

#[test]
fn no_block_and_empty_params_is_byte_identical() {
    let source = "#!/usr/bin/env python3\r\nprint('ok')\r\n";
    assert_eq!(write_python_params(source, &[]), source);
}

#[test]
fn reader_is_total_for_hand_broken_shapes() {
    for source in [
        "# /// script\n# tool = 5\n# ///\n",
        "# /// script\n# [tool]\n# skit = 5\n# ///\n",
        "# /// script\n# [tool.skit]\n# params = 5\n# ///\n",
        "# /// script\n# [tool.skit]\n# params = [5, \"x\"]\n# ///\n",
        "# /// script\n# [tool.skit]\n# [[tool.skit.params]]\n# type = \"str\"\n# ///\n",
    ] {
        assert!(read_python_params(source).is_empty());
    }
}

#[test]
fn frozen_reader_coerces_hand_edited_scalars_without_crashing() {
    let source = r#"# /// script
# dependencies = []
# [tool.skit]
# schema = 1
# [[tool.skit.params]]
# name = 7
# kind = "future"
# type = "future"
# prompt = 5
# order = "bad"
# secret = 1
# env_source = 9
# ///
"#;
    let params = read_python_params(source);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "7");
    assert_eq!(params[0].binding, Binding::Const);
    assert_eq!(params[0].delivery, Delivery::Inject);
    assert_eq!(params[0].param_type, ParamType::String);
    assert_eq!(params[0].prompt, "5");
    assert_eq!(params[0].order, -1);
    assert!(params[0].secret);
    assert_eq!(params[0].env_source, "9");
}
