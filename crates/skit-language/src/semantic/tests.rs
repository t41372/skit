use super::*;

fn python_literal_for(expression: &str) -> Option<PythonLiteral> {
    let source = format!("value = {expression}\n");
    let ParseOutcome::Parsed(document) = parse_document("python", &source) else {
        panic!("the literal fixture must parse");
    };
    let statement = named_children(document.syntax_tree().root_node())
        .into_iter()
        .next()?;
    let assignment = assignment_node(statement)?;
    let value = assignment.child_by_field_name("right")?;
    python_literal(&document, value)
}

fn python_literal_value_for(expression: &str) -> Option<ParameterValue> {
    let source = format!("value = {expression}\n");
    let ParseOutcome::Parsed(document) = parse_document("python", &source) else {
        panic!("the literal fixture must parse");
    };
    let statement = named_children(document.syntax_tree().root_node())
        .into_iter()
        .next()?;
    let assignment = assignment_node(statement)?;
    literal_value(&document, assignment.child_by_field_name("right")?)
}

fn python_string_for(expression: &str) -> Option<String> {
    let source = format!("value = {expression}\n");
    let ParseOutcome::Parsed(document) = parse_document("python", &source) else {
        panic!("the literal fixture must parse");
    };
    let statement = named_children(document.syntax_tree().root_node())
        .into_iter()
        .next()?;
    let assignment = assignment_node(statement)?;
    literal_string(&document, assignment.child_by_field_name("right")?)
}

fn python_bool_for(expression: &str) -> Option<bool> {
    let source = format!("value = {expression}\n");
    let ParseOutcome::Parsed(document) = parse_document("python", &source) else {
        panic!("the literal fixture must parse");
    };
    let statement = named_children(document.syntax_tree().root_node())
        .into_iter()
        .next()?;
    let assignment = assignment_node(statement)?;
    literal_bool(&document, assignment.child_by_field_name("right")?)
}

#[test]
fn python_integer_literals_cover_every_supported_radix_and_sign() {
    for (source, expected) in [
        ("0", 0),
        ("1_000", 1_000),
        ("0xff", 255),
        ("0X_FF", 255),
        ("0o17", 15),
        ("0O_17", 15),
        ("0b101", 5),
        ("0B_101", 5),
    ] {
        assert_eq!(parse_python_integer(source), Some(expected));
    }
    assert_eq!(parse_python_integer("not-a-number"), None);
    assert_eq!(parse_python_integer("999999999999999999999999"), None);

    assert_eq!(
        python_literal_for("+7"),
        Some(PythonLiteral::Value(ParameterValue::Integer(7)))
    );
    assert_eq!(
        python_literal_for("-7"),
        Some(PythonLiteral::Value(ParameterValue::Integer(-7)))
    );
    assert_eq!(
        python_literal_for("+1.5"),
        Some(PythonLiteral::Value(ParameterValue::Float(1.5)))
    );
    assert_eq!(
        python_literal_for("-1.5"),
        Some(PythonLiteral::Value(ParameterValue::Float(-1.5)))
    );
    assert_eq!(python_literal_for("~1"), None);
    assert_eq!(python_literal_for("~1.5"), None);
    assert_eq!(python_literal_for("-True"), None);
    assert_eq!(python_literal_for("1e999"), None);
}

#[test]
fn python_string_literals_decode_prefixes_quotes_and_concatenation() {
    for (source, expected) in [
        ("'plain'", "plain"),
        ("\"double\"", "double"),
        ("'''triple'''", "triple"),
        ("\"\"\"triple double\"\"\"", "triple double"),
        ("u'Unicode'", "Unicode"),
        ("R'raw\\n'", "raw\\n"),
    ] {
        assert_eq!(decode_python_string(source).as_deref(), Some(expected));
    }
    assert_eq!(decode_python_string("no quotes"), None);
    assert_eq!(decode_python_string("b'bytes'"), None);
    assert_eq!(decode_python_string("f'{value}'"), None);
    assert_eq!(decode_python_string("q'unknown prefix'"), None);
    assert_eq!(decode_python_string("'unterminated"), None);

    assert_eq!(
        python_literal_for("('left' 'right')"),
        Some(PythonLiteral::Value(ParameterValue::String(
            "leftright".to_owned()
        )))
    );
    assert_eq!(python_literal_for("('left' f'{value}')"), None);
    assert_eq!(python_literal_for("None"), Some(PythonLiteral::None));
    assert_eq!(python_literal_for("..."), Some(PythonLiteral::Ellipsis));
    assert_eq!(python_literal_value_for("None"), None);
    assert_eq!(python_literal_value_for("..."), None);
    assert_eq!(python_literal_for("[1]"), None);
    assert_eq!(python_string_for("'text'"), Some("text".to_owned()));
    assert_eq!(python_string_for("1"), None);
    assert_eq!(python_bool_for("True"), Some(true));
    assert_eq!(python_bool_for("'true'"), None);
    assert_eq!(literal_as_string(&PythonLiteral::None), None);
    assert_eq!(literal_as_string(&PythonLiteral::Ellipsis), None);
    assert_eq!(parameter_value_text(&ParameterValue::Float(3.5)), "3.5");
}

#[test]
fn python_escape_decoding_covers_named_numeric_continuation_and_unknown_forms() {
    let source = concat!(
        "\\\\",
        "\\'",
        "\\\"",
        "\\a",
        "\\b",
        "\\f",
        "\\n",
        "\\r",
        "\\t",
        "\\v",
        "\\\n",
        "\\\r\n",
        "\\x41",
        "\\u4e2d",
        "\\U0001F680",
        "\\101",
        "\\q",
    );
    assert_eq!(
        decode_python_escapes(source).as_deref(),
        Some("\\'\"\u{7}\u{8}\u{c}\n\r\t\u{b}A中🚀A\\q")
    );
    assert_eq!(decode_python_escapes("trailing\\"), None);
    assert_eq!(decode_python_escapes("\\xG0"), None);
    assert_eq!(decode_python_escapes("\\u123"), None);
    assert_eq!(decode_python_escapes("\\U00110000"), None);

    let mut short = "a".chars().peekable();
    assert_eq!(read_radix(&mut short, 2, 16), None);
    let mut invalid = "gg".chars().peekable();
    assert_eq!(read_radix(&mut invalid, 2, 16), None);
}

#[test]
fn python_input_binding_targets_cover_comprehensions_decorators_and_attributes() {
    for source in [
        "values = [input() for input in items]\n",
        "def outer(*input, **rest):\n    return input()\n",
        "class input:\n    pass\ninput()\n",
        "@decorator\ndef input():\n    pass\ninput()\n",
        "for input in values:\n    pass\ninput()\n",
        "(input := factory())\ninput()\n",
    ] {
        let ParseOutcome::Parsed(document) = parse_document("python", source) else {
            panic!("the binding fixture must parse");
        };
        assert!(
            document
                .analysis()
                .candidates
                .iter()
                .all(|candidate| candidate.declaration.binding != ParameterBinding::Input),
            "{source}"
        );
    }

    for source in [
        "obj.input = replacement\ninput()\n",
        "values[input] = replacement\ninput()\n",
    ] {
        let ParseOutcome::Parsed(document) = parse_document("python", source) else {
            panic!("the nonbinding fixture must parse");
        };
        assert_eq!(
            document
                .analysis()
                .candidates
                .iter()
                .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
                .count(),
            1,
            "{source}"
        );
    }
}

#[test]
fn python_constant_binding_counts_distinguish_real_targets_from_attribute_reads() {
    let source = concat!(
        "CLEAN = 1\n",
        "obj.CLEAN = 2\n",
        "items[CLEAN] = 3\n",
        "LAMBDA = 4\n",
        "callback = lambda LAMBDA: LAMBDA\n",
        "STAR = 5\n",
        "KW = 6\n",
        "def consume(*STAR, **KW):\n    return STAR, KW\n",
    );
    let ParseOutcome::Parsed(document) = parse_document("python", source) else {
        panic!("the constant fixture must parse");
    };

    let environment = constant_environment(&document);
    assert_eq!(environment.get("CLEAN"), Some(&ParameterValue::Integer(1)));
    assert!(!environment.contains_key("LAMBDA"));
    assert!(!environment.contains_key("STAR"));
    assert!(!environment.contains_key("KW"));
}

#[test]
fn typed_python_strings_escape_tabs_and_unicode_line_boundaries() {
    let declaration = ParamDecl::new("VALUE");
    assert_eq!(
        typed_python_literal(&declaration, "left\tright\u{2029}tail").unwrap(),
        "'left\\tright\\u2029tail'"
    );
}

#[test]
fn test_const_default_that_no_longer_fits_the_declared_type_is_not_published() {
    let mut stored = ParamDecl::new("N");
    stored.binding = ParameterBinding::Const;
    stored.delivery = ParameterDelivery::Inject;
    stored.parameter_type = ParameterType::Int;
    stored.default = Some(ParameterValue::Integer(3));

    let mut current = stored.clone();
    current.default = Some(ParameterValue::String("three".to_owned()));
    let analysis = SemanticAnalysis {
        candidates: vec![SemanticCandidate {
            declaration: current,
            identity: BindingIdentity {
                binding: ParameterBinding::Const,
                key: "N".to_owned(),
                occurrence: 0,
                scope: Vec::new(),
            },
            span: SourceSpan {
                start: 0,
                end: 1,
                start_line: 1,
                end_line: 1,
            },
            demotion: None,
            empty_uses_default: false,
        }],
        ..SemanticAnalysis::default()
    };

    let report = reconcile_analysis(&analysis, &[stored]);

    assert_eq!(
        report
            .ok
            .iter()
            .map(|pair| pair.stored.name.as_str())
            .collect::<Vec<_>>(),
        ["N"]
    );
    assert!(report.current_defaults.is_empty());
}
