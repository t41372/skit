//! Direct ports from Python `tests/test_source_default_semantics.py`
//! (`origin/main@206f9ef`). The Python implementation is the behavioral oracle.

use std::{collections::BTreeMap, sync::Mutex};

use skit_application::{
    delivery::{PreparedValue, assemble},
    form_state::{
        FormStateRepository, FormStateService, LastRunState, PersistedFormState, StateWriteError,
        remembered_values,
    },
    tokens::TokenContext,
    value_preparation::prepare_values,
    value_resolution::resolve_values,
};
use skit_domain::{
    Slug,
    parameters::{ParamDecl, ParameterDelivery, ParameterType, ParameterValue},
};

fn text_map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn scalar_map(items: &[(&str, &str)]) -> BTreeMap<String, PreparedValue> {
    items
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                PreparedValue::Scalar((*value).to_owned()),
            )
        })
        .collect()
}

fn greeting() -> ParamDecl {
    let mut declaration = ParamDecl::new("GREETING");
    declaration.delivery = ParameterDelivery::Inject;
    declaration.default = Some(ParameterValue::String("bonjour".to_owned()));
    declaration
}

#[test]
fn test_assemble_injects_a_value_that_equals_the_source_default() {
    // Whatever the form shows is what the script gets. Equality with the source default is not a
    // reason to skip delivery because another source occurrence can rebind the same name.
    let declaration = greeting();
    let equal = assemble(
        std::slice::from_ref(&declaration),
        &scalar_map(&[("GREETING", "bonjour")]),
        &[],
    )
    .unwrap();
    assert_eq!(equal.inject_values, text_map(&[("GREETING", "bonjour")]));
    assert_eq!(
        equal.display,
        vec![("GREETING".to_owned(), "bonjour".to_owned())]
    );

    let changed = assemble(&[declaration], &scalar_map(&[("GREETING", "other")]), &[]).unwrap();
    assert_eq!(changed.inject_values, text_map(&[("GREETING", "other")]));
    assert_eq!(
        changed.display,
        vec![("GREETING".to_owned(), "other".to_owned())]
    );
}

#[test]
fn test_assemble_injects_the_expansion_of_an_untouched_token_default() {
    let mut declaration = greeting();
    declaration.default = Some(ParameterValue::String("out_{today}.csv".to_owned()));
    let raw = text_map(&[("GREETING", "out_{today}.csv")]);
    let context = TokenContext {
        cwd: "/work".to_owned(),
        home: None,
        env: BTreeMap::new(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    };
    let resolved = resolve_values(std::slice::from_ref(&declaration), &raw, &context).unwrap();
    let prepared = prepare_values(std::slice::from_ref(&declaration), &raw, &resolved).unwrap();
    let assembly = assemble(&[declaration], &prepared, &[]).unwrap();
    assert_eq!(
        assembly.inject_values,
        text_map(&[("GREETING", "out_2026-07-09.csv")])
    );
}

#[test]
fn test_assemble_inject_delivers_empty_string_when_cleared() {
    let assembly = assemble(&[greeting()], &scalar_map(&[("GREETING", "")]), &[]).unwrap();
    assert_eq!(assembly.inject_values, text_map(&[("GREETING", "")]));
    assert!(
        assembly
            .display
            .contains(&("GREETING".to_owned(), "''".to_owned()))
    );
}

#[test]
fn test_assemble_env_delivers_empty_string_when_cleared() {
    let mut declaration = ParamDecl::new("CITY");
    declaration.delivery = ParameterDelivery::Env;
    declaration.default = Some(ParameterValue::String("Taipei".to_owned()));
    let assembly = assemble(&[declaration], &scalar_map(&[("CITY", "")]), &[]).unwrap();
    assert_eq!(assembly.env_values, text_map(&[("CITY", "")]));
}

#[test]
fn test_assemble_flag_delivers_empty_string_when_cleared() {
    let mut declaration = ParamDecl::new("x");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.flag = "--x".to_owned();
    declaration.default = Some(ParameterValue::String("def".to_owned()));
    let assembly = assemble(&[declaration], &scalar_map(&[("x", "")]), &[]).unwrap();
    assert_eq!(assembly.args, ["--x", ""]);
}

fn persisted_declarations() -> Vec<ParamDecl> {
    let mut greeting = greeting();
    greeting.name = "GREETING".to_owned();

    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Inject;
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));
    vec![greeting, width]
}

#[test]
fn test_last_used_drops_values_equal_to_their_default() {
    assert!(
        remembered_values(
            &persisted_declarations(),
            &text_map(&[("GREETING", "bonjour"), ("WIDTH", "800")]),
        )
        .is_empty()
    );
}

#[test]
fn test_last_used_keeps_a_cleared_empty_only_where_it_was_delivered() {
    assert_eq!(
        remembered_values(
            &persisted_declarations(),
            &text_map(&[("GREETING", ""), ("WIDTH", "")]),
        ),
        text_map(&[("GREETING", "")])
    );
}

#[derive(Debug, Default)]
struct MemoryState {
    state: Mutex<PersistedFormState>,
}

impl FormStateRepository for MemoryState {
    fn load(&self, _slug: &Slug) -> PersistedFormState {
        self.state.lock().unwrap().clone()
    }

    fn last_run(&self, _slug: &Slug) -> LastRunState {
        self.state.lock().unwrap().last_run.clone()
    }

    fn update<T, F>(&self, _slug: &Slug, update: F) -> Result<T, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> T,
    {
        Ok(update(&mut self.state.lock().unwrap()))
    }

    fn forget(&self, _slug: &Slug) -> Result<(), StateWriteError> {
        *self.state.lock().unwrap() = PersistedFormState::default();
        Ok(())
    }
}

#[test]
fn test_save_after_run_persists_via_the_remembered_rule() {
    let declarations = persisted_declarations();
    let values = text_map(&[("GREETING", "bonjour"), ("WIDTH", "900")]);
    let slug = Slug::parse("rem").unwrap();
    let service = FormStateService::new(MemoryState::default());

    service
        .save_last(&slug, &declarations, Some(&values), Some(Vec::new()), false)
        .unwrap();
    service
        .record_run(
            &slug,
            0,
            "2026-07-09T14:30:05+00:00",
            &declarations,
            Some(&values),
        )
        .unwrap();

    let state = service.load(&slug);
    assert_eq!(state.values, text_map(&[("WIDTH", "900")]));
    assert_eq!(state.last_run.values, Some(values));
    assert_eq!(state.last_run.exit, Some(0));
}
