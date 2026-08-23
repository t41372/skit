use skit_application::parameter_edit::{ParameterEditError, finish_parameter_edit};
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType, ParameterValue};
use skit_i18n::{Locale, Localize as _};

fn flagged(parameter_type: ParameterType, default: Option<ParameterValue>) -> ParamDecl {
    ParamDecl {
        parameter_type,
        delivery: ParameterDelivery::Flag,
        default,
        flag: "--verbose".to_owned(),
        ..ParamDecl::new("verbose")
    }
}

#[test]
fn an_off_by_default_boolean_flag_gets_an_explicit_store_true_action() {
    for default in [None, Some(ParameterValue::Bool(false))] {
        let mut declaration = flagged(ParameterType::Bool, default);
        finish_parameter_edit(&mut declaration).unwrap();
        assert_eq!(declaration.action, "store_true");
    }
}

#[test]
fn an_on_by_default_boolean_flag_is_refused_without_mutating_the_row() {
    let mut declaration = flagged(ParameterType::Bool, Some(ParameterValue::Bool(true)));
    let before = declaration.clone();
    assert_eq!(
        finish_parameter_edit(&mut declaration),
        Err(ParameterEditError::BoolFlagOnByDefault {
            name: "verbose".to_owned(),
        })
    );
    assert_eq!(declaration, before);

    let error = ParameterEditError::BoolFlagOnByDefault {
        name: "verbose".to_owned(),
    };
    assert_eq!(
        error.message().localize(Locale::En),
        "verbose is on by default, so its flag could only ever turn it on again. Declare the flag that turns it OFF instead (--no-verbose and the like), with default false."
    );
    assert_eq!(error.to_string(), error.message().localize(Locale::En));
    assert_eq!(
        error.message().localize(Locale::ZhCn),
        "verbose 默认就是开的，它的标志只会再开一次。请改成声明用来关掉它的那个标志(--no-verbose 之类)，默认 false。"
    );
    assert_eq!(
        error.message().localize(Locale::ZhTw),
        "verbose 預設就是開的，它的旗標只會再開一次。請改成宣告用來關掉它的那個旗標(--no-verbose 之類)，預設 false。"
    );
}

#[test]
fn moving_away_from_boolean_clears_a_stale_action() {
    let mut declaration = flagged(ParameterType::Str, None);
    declaration.action = "store_true".to_owned();
    finish_parameter_edit(&mut declaration).unwrap();
    assert!(declaration.action.is_empty());
}

#[test]
fn an_explicit_boolean_action_is_preserved() {
    let mut declaration = flagged(ParameterType::Bool, Some(ParameterValue::Bool(true)));
    declaration.action = "store_false".to_owned();
    finish_parameter_edit(&mut declaration).unwrap();
    assert_eq!(declaration.action, "store_false");
}
