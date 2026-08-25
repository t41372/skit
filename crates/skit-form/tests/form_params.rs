use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::{
    FormDrift, FormSource, form_params, form_params_from_managed, form_plan,
    parameter_section::{ParameterSection, ParameterSectionContext, parameter_section},
};

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
fn command_and_prompt_use_only_managed_placeholder_names_and_accept_env_riders() {
    let mut declared_name = ParamDecl::new("name");
    declared_name.delivery = ParameterDelivery::Placeholder;
    declared_name.help = "Declared help".to_owned();
    let settings = EntrySettings {
        params: vec!["name".to_owned(), "managed".to_owned()],
        parameters: vec![env("token"), declared_name],
        ..EntrySettings::default()
    };

    let command = form_params("command", "tool {name} {unmanaged}", &settings);
    assert_eq!(
        command
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["name", "managed", "token"]
    );
    assert_eq!(command[0].help, "Declared help");
    assert_eq!(command[1].delivery, ParameterDelivery::Placeholder);
    assert_eq!(command[2].delivery, ParameterDelivery::Env);

    let prompt = form_params("prompt", "Hello {{name}} {{unmanaged}}", &settings);
    assert_eq!(
        prompt
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["name", "managed", "token"]
    );
}

#[test]
fn a_prompt_with_interpolation_disabled_has_no_value_fields() {
    let mut placeholder = ParamDecl::new("name");
    placeholder.delivery = ParameterDelivery::Placeholder;
    let settings = EntrySettings {
        params: vec!["name".to_owned()],
        parameters: vec![placeholder, env("token")],
        interpolate: false,
        ..EntrySettings::default()
    };

    assert!(form_params("prompt", "Hello {{name}}", &settings).is_empty());
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
        parameters: vec![flag("extra"), env("WIDTH"), env("color"), {
            let mut ignored = ParamDecl::new("ignored");
            ignored.delivery = ParameterDelivery::Placeholder;
            ignored
        }],
        ..EntrySettings::default()
    };

    let fields = form_params("python", source, &settings);
    assert_eq!(
        fields
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["WIDTH", "extra", "color"]
    );
    assert_eq!(fields[0].binding, ParameterBinding::Const);
    assert_eq!(fields[0].delivery, ParameterDelivery::Inject);
    assert_eq!(fields[1].delivery, ParameterDelivery::Flag);
    assert_eq!(fields[2].delivery, ParameterDelivery::Env);
}

#[test]
fn prepared_managed_fields_can_skip_a_second_source_parse() {
    let mut width = ParamDecl::new("WIDTH");
    width.binding = ParameterBinding::Const;
    let settings = EntrySettings {
        parameters: vec![env("token"), flag("WIDTH")],
        ..EntrySettings::default()
    };

    let fields = form_params_from_managed(vec![width], &settings);

    assert_eq!(
        fields
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["WIDTH", "token"]
    );
    assert_eq!(fields[0].binding, ParameterBinding::Const);
}

#[test]
fn prepared_managed_fields_accept_only_unique_flag_and_environment_riders() {
    let mut placeholder = ParamDecl::new("placeholder");
    placeholder.delivery = ParameterDelivery::Placeholder;
    let mut inject = ParamDecl::new("inject");
    inject.delivery = ParameterDelivery::Inject;
    let settings = EntrySettings {
        parameters: vec![
            flag("flagged"),
            env("environment"),
            placeholder,
            inject,
            flag("managed"),
        ],
        ..EntrySettings::default()
    };

    let fields = form_params_from_managed(vec![ParamDecl::new("managed")], &settings);

    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["managed", "flagged", "environment"]
    );
}

#[test]
fn explicit_empty_delivery_is_limited_to_supported_text_riders() {
    for delivery in [
        ParameterDelivery::Inject,
        ParameterDelivery::Flag,
        ParameterDelivery::Env,
    ] {
        let mut declaration = ParamDecl::new("value");
        declaration.delivery = delivery;
        declaration.default = Some(ParameterValue::String("default".to_owned()));
        let field = skit_form::PreparedField {
            declaration,
            input_binding: false,
            empty_uses_default: false,
        };
        assert!(field.delivers_empty(), "delivery={delivery:?}");
    }

    let mut placeholder = ParamDecl::new("value");
    placeholder.delivery = ParameterDelivery::Placeholder;
    placeholder.default = Some(ParameterValue::String("default".to_owned()));
    let field = skit_form::PreparedField {
        declaration: placeholder,
        input_binding: false,
        empty_uses_default: false,
    };
    assert!(!field.delivers_empty());
}

#[test]
fn test_reader_kind_declared_env_rider_merges_not_erases() {
    // A PowerShell entry reads its param() block statically; a declared env row must ride after
    // the reader field, never short-circuit the plan and erase that field.
    let mut loglevel = ParamDecl::new("LOGLEVEL");
    loglevel.delivery = ParameterDelivery::Env;
    let settings = EntrySettings {
        parameters: vec![loglevel],
        ..EntrySettings::default()
    };

    let plan = form_plan("powershell", "param([string]$Region)\n", &settings);

    assert_eq!(plan.source, FormSource::Reader);
    assert_eq!(plan.source.as_str(), "argparse");
    assert_eq!(plan.fields.len(), 2);
    let region = &plan.fields[0].declaration;
    assert_eq!(region.name, "Region");
    assert_eq!(region.binding, ParameterBinding::None);
    assert_eq!(region.delivery, ParameterDelivery::Flag);
    assert_eq!(region.parameter_type, ParameterType::Str);
    assert_eq!(region.default, None);
    assert_eq!(region.flag, "-Region");
    assert!(!region.required);
    let loglevel = &plan.fields[1].declaration;
    assert_eq!(loglevel.name, "LOGLEVEL");
    assert_eq!(loglevel.binding, ParameterBinding::None);
    assert_eq!(loglevel.delivery, ParameterDelivery::Env);
    assert_eq!(loglevel.parameter_type, ParameterType::Str);
    assert_eq!(loglevel.default, None);
    assert_eq!(loglevel.env_target, "");
}

#[test]
fn powershell_static_reader_keeps_schema_deduplicates_and_filters_riders() {
    let source = concat!(
        "param(\n",
        " [Parameter(Mandatory)][ValidateSet('fast','safe')][string]$Mode = 'fast'\n",
        ")\n",
    );
    let mut duplicate = env("Mode");
    duplicate.default = Some(ParameterValue::String("wrong".to_owned()));
    let mut extra = flag("extra");
    extra.default = Some(ParameterValue::String("tail".to_owned()));
    let mut loglevel = env("loglevel");
    loglevel.prompt = "Log level".to_owned();
    loglevel.default = Some(ParameterValue::String("info".to_owned()));
    loglevel.env_target = "APP_LOG".to_owned();
    let mut placeholder = ParamDecl::new("ignored-placeholder");
    placeholder.delivery = ParameterDelivery::Placeholder;
    let mut inject = ParamDecl::new("ignored-inject");
    inject.delivery = ParameterDelivery::Inject;
    let settings = EntrySettings {
        parameters: vec![duplicate, extra, loglevel, placeholder, inject],
        ..EntrySettings::default()
    };

    let plan = form_plan("powershell", source, &settings);

    assert_eq!(plan.source, FormSource::Reader);
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["Mode", "extra", "loglevel"]
    );
    let mode = &plan.fields[0].declaration;
    assert_eq!(mode.parameter_type, ParameterType::Choice);
    assert_eq!(mode.choices, ["fast", "safe"]);
    assert_eq!(
        mode.default,
        Some(ParameterValue::String("fast".to_owned()))
    );
    assert!(mode.required);
    assert_eq!(mode.flag, "-Mode");
    assert_eq!(
        plan.fields[1].declaration.default,
        settings.parameters[1].default
    );
    assert_eq!(plan.fields[2].declaration.prompt, "Log level");
    assert_eq!(plan.fields[2].declaration.env_target, "APP_LOG");
}

#[test]
fn powershell_zero_and_absent_reader_surfaces_keep_distinct_sources() {
    let mut placeholder = ParamDecl::new("ignored");
    placeholder.delivery = ParameterDelivery::Placeholder;
    let settings = EntrySettings {
        parameters: vec![placeholder, flag("extra"), env("token")],
        ..EntrySettings::default()
    };

    let zero = form_plan("powershell", "param()\n", &settings);
    assert_eq!(zero.source, FormSource::Reader);
    assert_eq!(
        zero.declarations()
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["extra", "token"]
    );

    let absent = form_plan("powershell", "Write-Output 'hi'\n", &settings);
    assert_eq!(absent.source, FormSource::Declared);
    assert_eq!(
        absent
            .declarations()
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["extra", "token"]
    );

    let unreadable = form_plan("powershell", "param([string]$Name = )\n", &settings);
    assert_eq!(unreadable.source, FormSource::Declared);
    assert_eq!(
        unreadable
            .declarations()
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["extra", "token"]
    );

    let no_declarations = form_plan(
        "powershell",
        "Write-Output 'hi'\n",
        &EntrySettings::default(),
    );
    assert_eq!(no_declarations.source, FormSource::None);
    assert!(no_declarations.fields.is_empty());
}

#[test]
fn every_form_source_has_one_stable_machine_spelling() {
    assert_eq!(
        [
            FormSource::None,
            FormSource::Inject,
            FormSource::Reader,
            FormSource::Command,
            FormSource::Declared,
        ]
        .map(FormSource::as_str),
        ["none", "inject", "argparse", "command", "declared"]
    );
}

#[test]
fn generic_language_surfaces_preserve_static_dynamic_and_absent_states() {
    let static_plan = form_plan(
        "python",
        "p = argparse.ArgumentParser()\np.add_argument('--name')\n",
        &EntrySettings::default(),
    );
    assert_eq!(static_plan.source, FormSource::Reader);
    assert_eq!(static_plan.declarations()[0].name, "name");

    let dynamic_plan = form_plan(
        "python",
        "p = argparse.ArgumentParser()\np.add_argument('--name')\np.add_subparsers()\n",
        &EntrySettings::default(),
    );
    assert_eq!(dynamic_plan.source, FormSource::Reader);
    assert!(dynamic_plan.fields.is_empty());
    assert!(dynamic_plan.degradation.is_some());

    let absent_plan = form_plan("python", "print('plain')\n", &EntrySettings::default());
    assert_eq!(absent_plan.source, FormSource::None);
    assert!(absent_plan.fields.is_empty());
}

#[test]
fn an_unchanged_prompt_schema_has_no_missing_placeholder_drift() {
    let settings = EntrySettings {
        params: vec!["name".to_owned()],
        ..EntrySettings::default()
    };

    let plan = form_plan("prompt", "Hello {{name}}", &settings);

    assert_eq!(plan.source, FormSource::Command);
    assert!(plan.drift.is_empty());
    assert_eq!(plan.declarations()[0].name, "name");
}

#[test]
fn declared_riders_own_an_analyzable_form_before_static_cli_reflection() {
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

    let plan = form_plan("python", source, &settings);
    assert_eq!(plan.source, FormSource::Declared);
    let fields = plan.declarations();
    assert_eq!(
        fields
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["token", "count"]
    );
    assert_eq!(fields[0].delivery, ParameterDelivery::Env);
    assert_eq!(fields[1].flag, "--count");

    let dynamic = form_plan(
        "python",
        "p = argparse.ArgumentParser()\np.add_argument('--name')\np.add_subparsers()\n",
        &settings,
    );
    assert_eq!(dynamic.source, FormSource::Declared);
    assert_eq!(
        dynamic
            .declarations()
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["token", "count"]
    );
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
    assert_eq!(
        fields
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["ok"]
    );
}

#[test]
fn duplicate_template_declarations_use_the_last_definition_in_the_first_slot() {
    let mut first = ParamDecl::new("name");
    first.delivery = ParameterDelivery::Placeholder;
    first.help = "old".to_owned();
    let mut replacement = first.clone();
    replacement.help = "current".to_owned();
    let settings = EntrySettings {
        params: vec!["name".to_owned()],
        parameters: vec![first, replacement],
        ..EntrySettings::default()
    };

    let fields = form_params("command", "echo {name}", &settings);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].help, "current");
}

#[test]
fn managed_plan_reconciles_fields_and_refreshes_only_sound_source_defaults() {
    let source = r#"# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "KEEP"
# kind = "const"
# type = "str"
# default = "old"
#
# [[tool.skit.params]]
# name = "GONE"
# kind = "const"
# type = "str"
# default = "gone"
#
# [[tool.skit.params]]
# name = "CHANGED"
# kind = "const"
# type = "str"
# default = "stored"
# ///
KEEP = "current"
CHANGED = 42
"#;

    let plan = form_plan("python", source, &EntrySettings::default());

    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["KEEP", "CHANGED"]
    );
    assert_eq!(
        plan.fields[0].declaration.default,
        Some(skit_domain::parameters::ParameterValue::String(
            "current".to_owned()
        ))
    );
    assert_eq!(
        plan.fields[1].declaration.default,
        Some(skit_domain::parameters::ParameterValue::String(
            "stored".to_owned()
        ))
    );
    assert!(matches!(
        &plan.drift[0],
        FormDrift::Missing { declaration } if declaration.name == "GONE"
    ));
    assert!(matches!(
        &plan.drift[1],
        FormDrift::TypeChanged { stored, current }
            if stored.name == "CHANGED"
                && current.parameter_type == skit_domain::parameters::ParameterType::Int
    ));
}

#[test]
fn managed_input_whose_prompt_moved_is_exposed_as_rebound_drift() {
    let source = r#"# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "answer"
# kind = "input"
# type = "str"
# prompt = "Old question?"
# order = 0
# ///
answer = input("New question?")
"#;

    let plan = form_plan("python", source, &EntrySettings::default());

    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(plan.fields.len(), 1);
    assert!(
        matches!(
            plan.drift.as_slice(),
            [FormDrift::Rebound { stored, current }]
                if stored.name == "answer"
                    && stored.prompt == "Old question?"
                    && current.name == "input-1"
                    && current.prompt == "New question?"
        ),
        "drift={:?}",
        plan.drift
    );
}

#[test]
fn prepared_fields_publish_input_and_empty_delivery_semantics_directly() {
    let source = r#"# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "answer"
# kind = "input"
# type = "str"
# default = "fallback"
# prompt = "Answer?"
# order = 0
# ///
answer = input("Answer?")
"#;

    let plan = form_plan("python", source, &EntrySettings::default());

    assert!(plan.fields[0].input_binding);
    assert!(!plan.fields[0].empty_uses_default);
    assert!(!plan.fields[0].delivers_empty());
}

#[test]
fn shell_colon_defaults_keep_empty_delivery_semantics_out_of_frontends() {
    let source = r#"# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "FALLBACK_ON_EMPTY"
# kind = "envdefault"
# type = "str"
# default = "first"
#
# [[tool.skit.params]]
# name = "EMPTY_IS_VALUE"
# kind = "envdefault"
# type = "str"
# default = "second"
# ///
printf '%s %s\n' "${FALLBACK_ON_EMPTY:-first}" "${EMPTY_IS_VALUE-second}"
"#;

    let plan = form_plan("shell", source, &EntrySettings::default());

    assert!(plan.fields[0].empty_uses_default);
    assert!(!plan.fields[0].delivers_empty());
    assert!(!plan.fields[1].empty_uses_default);
    assert!(plan.fields[1].delivers_empty());
}

#[test]
fn managed_shell_form_plan_owns_self_location_warning_eligibility() {
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
WIDTH=800
printf '%s %s\n' "$WIDTH" "$BASH_SOURCE"
"#;

    let plan = form_plan("shell", source, &EntrySettings::default());
    assert!(plan.uses_self_location);

    let ordinary = form_plan(
        "shell",
        &source.replace(" \"$BASH_SOURCE\"", ""),
        &EntrySettings::default(),
    );
    assert!(!ordinary.uses_self_location);
}

#[test]
fn unmanaged_shell_form_plan_keeps_self_location_in_the_same_projection() {
    let plan = form_plan(
        "shell",
        "#!/usr/bin/env bash\nHERE=$(dirname \"$0\")\nREGION=us-east-1\n",
        &EntrySettings::default(),
    );

    assert!(plan.fields.is_empty());
    assert!(plan.uses_self_location);
    assert!(plan.has_injectable_const);

    let environment_default = form_plan(
        "shell",
        "#!/usr/bin/env bash\nHERE=$(dirname \"$0\")\nMODE=\"${MODE:-auto}\"\n",
        &EntrySettings::default(),
    );
    assert!(environment_default.uses_self_location);
    assert!(!environment_default.has_injectable_const);
}

#[test]
fn syntax_errors_keep_managed_metadata_visible_as_drift_without_inventing_fields() {
    let source = r#"# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "VALUE"
# kind = "const"
# type = "str"
# default = "stored"
# ///
if (
"#;

    let plan = form_plan("python", source, &EntrySettings::default());

    assert_eq!(plan.source, FormSource::Inject);
    assert!(plan.fields.is_empty());
    assert!(matches!(
        plan.drift.as_slice(),
        [FormDrift::Missing { declaration }] if declaration.name == "VALUE"
    ));
}

#[test]
fn source_summaries_and_declared_defaults_keep_distinct_display_grammars() {
    let context = |declared_schema| ParameterSectionContext {
        kind: if declared_schema { "exe" } else { "python" },
        reference_mode: false,
        declared_schema,
        has_analyzer: !declared_schema,
        reader_fields: 0,
    };
    let declaration = |value: ParameterValue| {
        let mut declaration = ParamDecl::new("VALUE");
        declaration.parameter_type = match &value {
            ParameterValue::String(_) => ParameterType::Str,
            ParameterValue::Integer(_) => ParameterType::Int,
            ParameterValue::Float(_) => ParameterType::Float,
            ParameterValue::Bool(_) => ParameterType::Bool,
        };
        declaration.default = Some(value);
        declaration
    };
    let source_summary = |value| {
        let section = parameter_section(context(false), &[declaration(value)], &[]);
        let ParameterSection::SourceManaged { rows, .. } = section else {
            panic!("expected the source-managed section");
        };
        rows[0].summary.clone()
    };
    assert_eq!(
        source_summary(ParameterValue::String("World".to_owned())),
        "VALUE  str 'World'"
    );
    assert_eq!(
        source_summary(ParameterValue::String("O'Reilly".to_owned())),
        "VALUE  str \"O'Reilly\""
    );
    assert_eq!(
        source_summary(ParameterValue::String("line\\break\n".to_owned())),
        "VALUE  str 'line\\\\break\\n'"
    );
    assert_eq!(
        source_summary(ParameterValue::String("both ' and \"\r\t\u{7}".to_owned())),
        "VALUE  str 'both \\' and \"\\r\\t\\x07'"
    );
    assert_eq!(source_summary(ParameterValue::Integer(3)), "VALUE  int 3");
    assert_eq!(
        source_summary(ParameterValue::Float(3.0)),
        "VALUE  float 3.0"
    );
    assert_eq!(
        source_summary(ParameterValue::Bool(true)),
        "VALUE  bool True"
    );
    let without_default = ParamDecl::new("VALUE");
    let ParameterSection::SourceManaged { rows, .. } =
        parameter_section(context(false), &[without_default], &[])
    else {
        unreachable!("the analyzer-backed context always builds a source section");
    };
    assert_eq!(rows[0].summary, "VALUE  str");

    let declared_default = |value| {
        let section = parameter_section(context(true), &[declaration(value)], &[]);
        let ParameterSection::Declared { rows } = section else {
            panic!("expected the declared section");
        };
        rows[0].field("default").unwrap().value().as_text()
    };
    assert_eq!(
        declared_default(ParameterValue::String("World".to_owned())),
        "World"
    );
    assert_eq!(declared_default(ParameterValue::Integer(3)), "3");
    assert_eq!(declared_default(ParameterValue::Float(3.5)), "3.5");
    assert_eq!(declared_default(ParameterValue::Bool(true)), "true");
}
