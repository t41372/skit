use skit_domain::{EntrySettings, parameters::{ParamDecl, ParameterBinding, ParameterDelivery}};
use skit_language::form_params;

fn env(name: &str) -> ParamDecl {
    let mut value = ParamDecl::new(name);
    value.delivery = ParameterDelivery::Env;
    value.env_target = name.to_ascii_uppercase();
    value
}

fn flag(name: &str) -> ParamDecl {
    let mut value = ParamDecl::new(name);
    value.delivery = ParameterDelivery::Flag;
    value.flag = format!("--{name}");
    value
}

#[test]
fn command_and_prompt_placeholders_control_order_and_accept_env_riders() {
    let mut declared_name = ParamDecl::new("name");
    declared_name.delivery = ParameterDelivery::Placeholder;
    declared_name.help = "Declared help".to_owned();
    let settings = EntrySettings {
        parameters: vec![env("token"), declared_name],
        ..EntrySettings::default()
    };

    let command = form_params("command", "tool {name} {implicit}", &settings);
    assert_eq!(
        command.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(),
        ["name", "implicit", "token"]
    );
    assert_eq!(command[0].help, "Declared help");
    assert_eq!(command[1].delivery, ParameterDelivery::Placeholder);
    assert_eq!(command[2].delivery, ParameterDelivery::Env);

    let prompt = form_params("prompt", "Hello {{name}} {{implicit}}", &settings);
    assert_eq!(
        prompt.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(),
        ["name", "implicit", "token"]
    );
}

#[test]
fn managed_source_fields_win_and_declared_flag_env_riders_follow_without_duplicates() {
    let source = r#"# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "WIDTH"
# kind = "const"
# type = "int"
# default = 800
# ///
WIDTH = 800
"#;
    let settings = EntrySettings {
        parameters: vec![flag("extra"), env("WIDTH"), env("color")],
        ..EntrySettings::default()
    };

    let fields = form_params("python", source, &settings);
    assert_eq!(
        fields.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(),
        ["WIDTH", "extra", "color"]
    );
    assert_eq!(fields[0].binding, ParameterBinding::Const);
    assert_eq!(fields[0].delivery, ParameterDelivery::Inject);
    assert_eq!(fields[1].delivery, ParameterDelivery::Flag);
    assert_eq!(fields[2].delivery, ParameterDelivery::Env);
}

#[test]
fn static_cli_fields_are_used_when_there_is_no_managed_block() {
    let source = r#"
import argparse
p = argparse.ArgumentParser()
p.add_argument("input")
p.add_argument("--count", type=int)
"#;
    let settings = EntrySettings {
        parameters: vec![env("token"), flag("count")],
        ..EntrySettings::default()
    };

    let fields = form_params("python", source, &settings);
    assert_eq!(
        fields.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(),
        ["input", "count", "token"]
    );
    assert_eq!(fields[1].flag, "--count");
    assert_eq!(fields[2].delivery, ParameterDelivery::Env);
}

#[test]
fn declared_fields_are_the_fallback_when_static_source_has_no_schema() {
    let settings = EntrySettings {
        parameters: vec![flag("output"), env("token")],
        ..EntrySettings::default()
    };

    let fields = form_params("ruby", "puts 'ok'\n", &settings);
    assert_eq!(fields, settings.parameters);
}

#[test]
fn unsupported_declared_delivery_does_not_leak_into_program_source_forms() {
    let mut placeholder = ParamDecl::new("bad");
    placeholder.delivery = ParameterDelivery::Placeholder;
    let mut inject = ParamDecl::new("also_bad");
    inject.delivery = ParameterDelivery::Inject;
    let settings = EntrySettings {
        parameters: vec![placeholder, inject, env("ok")],
        ..EntrySettings::default()
    };

    let fields = form_params("ruby", "puts 'ok'\n", &settings);
    assert_eq!(fields.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(), ["ok"]);
}
