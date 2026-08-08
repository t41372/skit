use skit_core::{
    Binding, Delivery, ParamDecl, ParamDefault, ParamType, declared_from_meta, is_secret_name,
    synthesized_placeholder,
};

#[test]
fn metadata_reader_is_total_and_coerces_hand_edited_scalars()
-> Result<(), Box<dyn std::error::Error>> {
    let value: toml::Value = toml::from_str(
        r#"name = 7
binding = "future-binding"
delivery = "future-delivery"
type = "future-type"
default = [1, 2]
required = 1
choices = ["a", 2, true]
order = "not-an-int"
"#,
    )?;
    let Some(table) = value.as_table() else {
        return Err("expected table".into());
    };
    let decl = ParamDecl::from_meta_table(table);
    assert_eq!(decl.name, "7");
    assert_eq!(decl.binding, Binding::None);
    assert_eq!(decl.delivery, Delivery::Flag);
    assert_eq!(decl.param_type, ParamType::String);
    assert!(decl.default.is_none());
    assert!(decl.required);
    assert_eq!(decl.choices, ["a", "2", "true"]);
    assert_eq!(decl.order, -1);
    Ok(())
}

#[test]
fn metadata_writer_preserves_the_additive_full_model() {
    let decl = ParamDecl {
        name: "WIDTH".to_owned(),
        binding: Binding::None,
        delivery: Delivery::Env,
        param_type: ParamType::Choice,
        default: Some(ParamDefault::String("m".to_owned())),
        required: true,
        choices: vec!["s".to_owned(), "m".to_owned()],
        prompt: "Size".to_owned(),
        help: "Rendered size".to_owned(),
        env_target: "APP_WIDTH".to_owned(),
        ..ParamDecl::default()
    };
    let row = decl.to_meta_table();
    assert_eq!(row["name"].as_str(), Some("WIDTH"));
    assert_eq!(row["delivery"].as_str(), Some("env"));
    assert_eq!(row["type"].as_str(), Some("choice"));
    assert_eq!(row["default"].as_str(), Some("m"));
    assert_eq!(row["required"].as_bool(), Some(true));
    assert_eq!(row["env_target"].as_str(), Some("APP_WIDTH"));
    assert!(!row.contains_key("binding"));
}

#[test]
fn declared_reader_drops_nameless_rows() {
    let rows = vec![
        toml::Table::from_iter([(
            "delivery".to_owned(),
            toml::Value::String("flag".to_owned()),
        )]),
        toml::Table::from_iter([
            ("name".to_owned(), toml::Value::String("ok".to_owned())),
            ("delivery".to_owned(), toml::Value::String("env".to_owned())),
        ]),
    ];
    let decls = declared_from_meta(&rows);
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].name, "ok");
}

#[test]
fn synthesized_placeholder_keeps_required_and_secret_heuristic() {
    let plain = synthesized_placeholder("target");
    assert_eq!(plain.delivery, Delivery::Placeholder);
    assert!(plain.required);
    assert!(!plain.secret);

    let secret = synthesized_placeholder("api_key");
    assert!(secret.secret);
    assert!(is_secret_name("PasswordFile"));
    assert!(!is_secret_name("monkey"));
}
