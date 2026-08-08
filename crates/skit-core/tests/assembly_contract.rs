use std::collections::BTreeMap;

use skit_core::{
    AssemblyError, Delivery, FormField, FormPlan, ParamDecl, ParamType, PlanSource,
    assemble_delivery,
};

fn plan(fields: Vec<ParamDecl>) -> FormPlan {
    FormPlan {
        source: PlanSource::Declared,
        fields: fields.iter().map(FormField::from_decl).collect(),
        ..FormPlan::default()
    }
}

#[test]
fn mixed_flag_env_and_extra_args_route_to_their_own_channels()
-> Result<(), Box<dyn std::error::Error>> {
    let plan = plan(vec![
        ParamDecl {
            name: "src".to_owned(),
            delivery: Delivery::Flag,
            required: true,
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "width".to_owned(),
            delivery: Delivery::Flag,
            param_type: ParamType::Integer,
            flag: "--width".to_owned(),
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "DEBUG".to_owned(),
            delivery: Delivery::Env,
            env_target: "APP_DEBUG".to_owned(),
            ..ParamDecl::default()
        },
    ]);
    let values = BTreeMap::from([
        ("src".to_owned(), "in.png".to_owned()),
        ("width".to_owned(), "800".to_owned()),
        ("DEBUG".to_owned(), "1".to_owned()),
    ]);
    let assembly = assemble_delivery(&plan, &values, &["--verbose".to_owned()], &BTreeMap::new())?;

    assert_eq!(assembly.args, ["in.png", "--width", "800", "--verbose"]);
    assert_eq!(assembly.masked_args, assembly.args);
    assert_eq!(assembly.env_values["APP_DEBUG"], "1");
    assert_eq!(assembly.masked_env["APP_DEBUG"], "1");
    assert!(assembly.command_values.is_empty());
    assert!(assembly.inject_values.is_empty());
    Ok(())
}

#[test]
fn repeat_multi_flag_repeats_the_option_for_each_shell_quoted_piece()
-> Result<(), Box<dyn std::error::Error>> {
    let plan = plan(vec![ParamDecl {
        name: "tag".to_owned(),
        delivery: Delivery::Flag,
        multiple: true,
        repeat: true,
        flag: "--tag".to_owned(),
        ..ParamDecl::default()
    }]);
    let values = BTreeMap::from([("tag".to_owned(), "alpha 'two words' gamma".to_owned())]);
    let assembly = assemble_delivery(&plan, &values, &[], &BTreeMap::new())?;
    assert_eq!(
        assembly.args,
        ["--tag", "alpha", "--tag", "two words", "--tag", "gamma"]
    );
    Ok(())
}

#[test]
fn bool_flags_fire_only_for_the_action_state() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan(vec![
        ParamDecl {
            name: "force".to_owned(),
            delivery: Delivery::Flag,
            param_type: ParamType::Boolean,
            flag: "--force".to_owned(),
            action: "store_true".to_owned(),
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "quiet".to_owned(),
            delivery: Delivery::Flag,
            param_type: ParamType::Boolean,
            flag: "--no-quiet".to_owned(),
            action: "store_false".to_owned(),
            ..ParamDecl::default()
        },
    ]);
    let values = BTreeMap::from([
        ("force".to_owned(), "on".to_owned()),
        ("quiet".to_owned(), "false".to_owned()),
    ]);
    let assembly = assemble_delivery(&plan, &values, &[], &BTreeMap::new())?;
    assert_eq!(assembly.args, ["--force", "--no-quiet"]);
    Ok(())
}

#[test]
fn secret_values_stay_real_in_delivery_and_mask_every_transparency_projection()
-> Result<(), Box<dyn std::error::Error>> {
    let plan = FormPlan {
        source: PlanSource::Command,
        fields: vec![
            FormField::from_decl(&ParamDecl {
                name: "password".to_owned(),
                delivery: Delivery::Placeholder,
                secret: true,
                required: true,
                ..ParamDecl::default()
            }),
            FormField::from_decl(&ParamDecl {
                name: "TOKEN".to_owned(),
                delivery: Delivery::Env,
                secret: true,
                env_source: "TOKEN_ENV".to_owned(),
                env_target: "API_TOKEN".to_owned(),
                ..ParamDecl::default()
            }),
            FormField::from_decl(&ParamDecl {
                name: "CITY".to_owned(),
                delivery: Delivery::Inject,
                secret: true,
                ..ParamDecl::default()
            }),
        ],
        ..FormPlan::default()
    };
    let values = BTreeMap::from([
        ("password".to_owned(), "s3cret".to_owned()),
        ("CITY".to_owned(), "Taipei".to_owned()),
    ]);
    let environment = BTreeMap::from([("TOKEN_ENV".to_owned(), "env-secret".to_owned())]);
    let assembly = assemble_delivery(&plan, &values, &[], &environment)?;

    assert_eq!(assembly.command_values["password"], "s3cret");
    assert_eq!(assembly.masked_command_values["password"], "•••");
    assert_eq!(assembly.env_values["API_TOKEN"], "env-secret");
    assert_eq!(assembly.masked_env["API_TOKEN"], "•••");
    assert_eq!(assembly.inject_values["CITY"], "Taipei");
    assert_eq!(assembly.masked_inject["CITY"], "•••");
    Ok(())
}

#[test]
fn missing_secret_environment_source_is_a_named_error() {
    let plan = plan(vec![ParamDecl {
        name: "token".to_owned(),
        delivery: Delivery::Env,
        secret: true,
        env_source: "API_TOKEN".to_owned(),
        ..ParamDecl::default()
    }]);
    let error = assemble_delivery(&plan, &BTreeMap::new(), &[], &BTreeMap::new());
    assert!(matches!(
        error,
        Err(AssemblyError::MissingSecretEnvironment { field, variable })
            if field == "token" && variable == "API_TOKEN"
    ));
}

#[test]
fn invalid_typed_value_refuses_before_any_delivery_is_built() {
    let plan = plan(vec![ParamDecl {
        name: "count".to_owned(),
        delivery: Delivery::Flag,
        param_type: ParamType::Integer,
        flag: "--count".to_owned(),
        ..ParamDecl::default()
    }]);
    let values = BTreeMap::from([("count".to_owned(), "many".to_owned())]);
    let error = assemble_delivery(&plan, &values, &[], &BTreeMap::new());
    assert!(matches!(
        error,
        Err(AssemblyError::InvalidValues(errors)) if errors.contains_key("count")
    ));
}
