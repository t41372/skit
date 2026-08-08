use std::collections::BTreeMap;

use skit_core::{
    Binding, Delivery, ParamDecl, ParamType, PythonInjectError, inject_python_consts,
};

fn const_spec(name: &str, param_type: ParamType) -> ParamDecl {
    ParamDecl {
        name: name.to_owned(),
        binding: Binding::Const,
        delivery: Delivery::Inject,
        param_type,
        ..ParamDecl::default()
    }
}

#[test]
fn const_injection_replaces_module_and_main_guard_targets_only() {
    let source = concat!(
        "# CITY = 'comment stays'\n",
        "CITY = 'Taipei'\n",
        "print(\"CITY = 'string stays'\")\n",
        "if __name__ == '__main__':\n",
        "    CITY = 'Tainan'\n",
        "    print(CITY)\n",
    );
    let output = inject_python_consts(
        source,
        &[const_spec("CITY", ParamType::String)],
        &BTreeMap::from([("CITY".to_owned(), "Paris".to_owned())]),
    )
    .expect("const injection should succeed");
    assert_eq!(
        output,
        concat!(
            "# CITY = 'comment stays'\n",
            "CITY = \"Paris\"\n",
            "print(\"CITY = 'string stays'\")\n",
            "if __name__ == '__main__':\n",
            "    CITY = \"Paris\"\n",
            "    print(CITY)\n",
        )
    );
}

#[test]
fn typed_values_render_as_valid_python_literals() {
    let source = "COUNT = 1\nRATIO = 0.5\nON = False\nTEXT = 'old'\n";
    let specs = vec![
        const_spec("COUNT", ParamType::Integer),
        const_spec("RATIO", ParamType::Float),
        const_spec("ON", ParamType::Boolean),
        const_spec("TEXT", ParamType::String),
    ];
    let values = BTreeMap::from([
        ("COUNT".to_owned(), "7".to_owned()),
        ("RATIO".to_owned(), "2.25".to_owned()),
        ("ON".to_owned(), "yes".to_owned()),
        ("TEXT".to_owned(), "a\"b\\c\n市".to_owned()),
    ]);
    let output = inject_python_consts(source, &specs, &values)
        .expect("typed injection should succeed");
    assert_eq!(
        output,
        "COUNT = 7\nRATIO = 2.25\nON = True\nTEXT = \"a\\\"b\\\\c\\n市\"\n"
    );
}

#[test]
fn vanished_const_target_is_a_named_drift_refusal() {
    let result = inject_python_consts(
        "OTHER = 1\n",
        &[const_spec("COUNT", ParamType::Integer)],
        &BTreeMap::from([("COUNT".to_owned(), "2".to_owned())]),
    );
    assert_eq!(
        result,
        Err(PythonInjectError::MissingTarget("COUNT".to_owned()))
    );
}

#[test]
fn invalid_typed_value_is_never_written_into_source() {
    let result = inject_python_consts(
        "COUNT = 1\n",
        &[const_spec("COUNT", ParamType::Integer)],
        &BTreeMap::from([("COUNT".to_owned(), "not-an-int".to_owned())]),
    );
    assert_eq!(
        result,
        Err(PythonInjectError::InvalidValue {
            name: "COUNT".to_owned(),
            value: "not-an-int".to_owned(),
        })
    );
}

#[test]
fn supplied_managed_input_is_explicitly_refused_in_const_only_slice() {
    let input = ParamDecl {
        name: "input-1".to_owned(),
        binding: Binding::Input,
        delivery: Delivery::Inject,
        prompt: "Name: ".to_owned(),
        order: 0,
        ..ParamDecl::default()
    };
    let result = inject_python_consts(
        "name = input('Name: ')\n",
        &[input],
        &BTreeMap::from([("input-1".to_owned(), "Ada".to_owned())]),
    );
    assert_eq!(
        result,
        Err(PythonInjectError::ManagedInputUnsupported(
            "input-1".to_owned()
        ))
    );
}

#[test]
fn no_values_is_byte_identical_and_syntax_errors_are_named() {
    let broken = "def broken(:\n";
    assert_eq!(
        inject_python_consts(broken, &[], &BTreeMap::new())
            .expect("no values must not parse or rewrite source"),
        broken
    );
    assert_eq!(
        inject_python_consts(
            broken,
            &[const_spec("X", ParamType::String)],
            &BTreeMap::from([("X".to_owned(), "x".to_owned())]),
        ),
        Err(PythonInjectError::Syntax)
    );
}
