use std::path::PathBuf;

use skit_core::{
    Delivery, Entry, FormField, ParamDefault, ParamType, PlanSource, ScriptMeta, plan_for_entry,
};

fn entry(kind: &str, extra: &str) -> Result<Entry, Box<dyn std::error::Error>> {
    let source = format!("name = \"demo\"\nkind = \"{kind}\"\n{extra}");
    let meta: ScriptMeta = toml::from_str(&source)?;
    Ok(Entry {
        slug: "demo".to_owned(),
        meta,
        dir: PathBuf::from("/tmp/skit-form-plan-demo"),
    })
}

#[test]
fn exe_declared_rows_become_one_typed_plan_and_irrelevant_delivery_is_dropped()
-> Result<(), Box<dyn std::error::Error>> {
    let entry = entry(
        "exe",
        r#"
[[parameters]]
name = "width"
delivery = "flag"
type = "int"
default = 800
flag = "--width"

[[parameters]]
name = "DEBUG"
delivery = "env"
type = "bool"

[[parameters]]
name = "slot"
delivery = "placeholder"
"#,
    )?;

    let plan = plan_for_entry(&entry);
    assert_eq!(plan.source, PlanSource::Declared);
    assert_eq!(plan.fields.len(), 2);
    assert_eq!(plan.fields[0].key, "width");
    assert_eq!(plan.fields[0].param_type, ParamType::Integer);
    assert_eq!(plan.fields[0].default.as_deref(), Some("800"));
    assert_eq!(plan.fields[0].flag, "--width");
    assert_eq!(plan.fields[1].key, "DEBUG");
    assert_eq!(plan.fields[1].delivery, Delivery::Env);
    Ok(())
}

#[test]
fn command_placeholders_keep_template_order_and_declared_schema_overrides_synthesized_defaults()
-> Result<(), Box<dyn std::error::Error>> {
    let entry = entry(
        "command",
        r#"
template = "convert {size} {api_key}"
params = ["size", "api_key"]

[[parameters]]
name = "size"
delivery = "placeholder"
type = "choice"
choices = ["s", "m"]
default = "m"

[[parameters]]
name = "RETRIES"
delivery = "env"
type = "int"
default = 3
"#,
    )?;

    let plan = plan_for_entry(&entry);
    assert_eq!(plan.source, PlanSource::Command);
    assert_eq!(plan.fields.len(), 3);
    assert_eq!(plan.fields[0].key, "size");
    assert_eq!(plan.fields[0].param_type, ParamType::Choice);
    assert_eq!(plan.fields[0].choices, ["s", "m"]);
    assert_eq!(plan.fields[0].default.as_deref(), Some("m"));
    assert!(!plan.fields[0].secret);

    assert_eq!(plan.fields[1].key, "api_key");
    assert_eq!(plan.fields[1].delivery, Delivery::Placeholder);
    assert!(plan.fields[1].required);
    assert!(plan.fields[1].secret);

    assert_eq!(plan.fields[2].key, "RETRIES");
    assert_eq!(plan.fields[2].delivery, Delivery::Env);
    assert_eq!(plan.fields[2].default.as_deref(), Some("3"));
    Ok(())
}

#[test]
fn boolean_flag_without_action_gets_the_existing_store_true_hygiene() {
    let field = FormField::from_decl(&skit_core::ParamDecl {
        name: "force".to_owned(),
        delivery: Delivery::Flag,
        param_type: ParamType::Boolean,
        default: Some(ParamDefault::Boolean(false)),
        flag: "--force".to_owned(),
        ..skit_core::ParamDecl::default()
    });
    assert_eq!(field.action, "store_true");
}
