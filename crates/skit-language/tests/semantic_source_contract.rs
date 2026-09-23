use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery};
use skit_language::{
    ParseOutcome, ReconcileReport, managed_params, parse_document, source_parameter_semantics,
};

fn env_default(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::EnvDefault;
    declaration.delivery = ParameterDelivery::Env;
    declaration
}

#[test]
fn shell_default_operator_semantics_come_from_the_syntax_tree() {
    for (operator, expected) in [(":-", true), (":=", true), ("-", false), ("=", false)] {
        let source = format!("echo \"${{CITY{operator}Taipei}}\"\n");
        assert_eq!(
            source_parameter_semantics("shell", &source, &env_default("CITY")).empty_uses_default,
            expected,
            "operator {operator}"
        );

        let ParseOutcome::Parsed(document) = parse_document("shell", &source) else {
            panic!("shell source must parse");
        };
        assert_eq!(
            document
                .source_parameter_semantics(&env_default("CITY"))
                .empty_uses_default,
            expected,
            "operator {operator} through one parse session"
        );
    }

    assert!(
        !source_parameter_semantics(
            "fish",
            "set -q CITY; or set CITY Taipei\n",
            &env_default("CITY"),
        )
        .empty_uses_default
    );
    let mut constant = ParamDecl::new("CITY");
    constant.binding = ParameterBinding::Const;
    constant.delivery = ParameterDelivery::Inject;
    assert!(
        !source_parameter_semantics("shell", "echo \"${CITY:-Taipei}\"\n", &constant)
            .empty_uses_default
    );
}

#[test]
fn metadata_survives_a_body_syntax_error_and_reconcile_reports_the_error() {
    let source = concat!(
        "# /// script\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"CITY\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# ///\n",
        "def broken(:\n",
    );
    let stored = managed_params("python", source);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].name, "CITY");
    assert!(matches!(
        parse_document("python", source),
        ParseOutcome::SyntaxError(_)
    ));

    let report = ReconcileReport::from_syntax_error(&stored);
    assert!(report.syntax_error);
    assert!(report.has_drift());
    assert_eq!(report.missing, stored);
    assert!(report.usable().is_empty());
}
