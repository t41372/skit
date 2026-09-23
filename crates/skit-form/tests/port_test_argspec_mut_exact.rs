//! Exact public-surface ports of Python v0.4 `tests/test_argspec_mut.py`.
//!
//! The mutation suite independently freezes the binding/delivery axes for every reflected Python
//! CLI framework. It is intentionally not folded into broader argspec tests.

use skit_domain::parameters::{ParameterBinding, ParameterDelivery};
use skit_form::{CliFormProjection, onboarding_plan};

fn fields(source: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    let plan = onboarding_plan("python", source);
    match plan.cli_surface {
        CliFormProjection::Static { fields, .. } => fields,
        other => panic!("expected a static Python CLI surface: {other:?}"),
    }
}

fn assert_reflected_axes(fields: &[skit_domain::parameters::ParamDecl]) {
    for field in fields {
        assert_eq!(field.binding, ParameterBinding::None, "{field:?}");
        assert_eq!(field.delivery, ParameterDelivery::Flag, "{field:?}");
    }
}

#[test]
fn test_argparse_field_binding_is_none_and_delivery_is_flag() {
    let actual = fields(concat!(
        "import argparse\n",
        "ap = argparse.ArgumentParser()\n",
        "ap.add_argument('--output')\n",
        "ap.add_argument('count', type=int)\n",
    ));
    assert_eq!(
        actual
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["output", "count"]
    );
    assert_reflected_axes(&actual);
}

#[test]
fn test_click_field_binding_is_none_and_delivery_is_flag() {
    let actual = fields(concat!(
        "import click\n",
        "@click.command()\n",
        "@click.option('--output')\n",
        "@click.argument('name')\n",
        "def m(output, name): pass\n",
    ));
    let mut names = actual
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, ["name", "output"]);
    assert_reflected_axes(&actual);
}

#[test]
fn test_typer_field_binding_is_none_and_delivery_is_flag() {
    let actual = fields(
        "import typer\n\ndef main(name: str, count: int = 3):\n    pass\n\ntyper.run(main)\n",
    );
    assert_eq!(
        actual
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["name", "count"]
    );
    assert_reflected_axes(&actual);
}
