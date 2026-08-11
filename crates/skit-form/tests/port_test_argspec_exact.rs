//! Exact public-surface port of Python v0.4 `tests/test_argspec.py`.
//!
//! Python oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//! One Rust test maps to one Python `def test_*`, in source order. These use the real parser-owned
//! `cli_form_projection`; no argparse reader is reimplemented inside the tests.

use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_form::{CliFormProjection, cli_form_projection};
use skit_language::DegradationReason;

const STITCH: &str = r#"
import argparse
from pathlib import Path

def parse_color(value):
    return (0, 0, 0, 0)

def main():
    ap = argparse.ArgumentParser(description="stitch images")
    ap.add_argument("inputs", nargs="+", type=Path, help="input images")
    ap.add_argument("-o", "--output", type=Path, required=True, help="output path")
    ap.add_argument("-d", "--direction", choices=["vertical", "horizontal"],
                    default="vertical", help="direction")
    ap.add_argument("--gap", type=int, default=0, help="gap in px")
    ap.add_argument("--bg", type=parse_color, default=(255, 255, 255, 255), help="bg color")
    ap.add_argument("--match-size", action="store_true", help="unify sizes first")
    ap.add_argument("--align", choices=["start", "center", "end"], default="center")
    ap.add_argument("--no-sort", action="store_true", help="keep argv order")
    args = ap.parse_args()

if __name__ == "__main__":
    main()
"#;

fn fields(kind: &str, source: &str) -> Vec<ParamDecl> {
    match cli_form_projection(kind, source) {
        CliFormProjection::Static { fields, .. } => fields,
        other => panic!("expected static {kind} CLI surface, got {other:?}\nsource:\n{source}"),
    }
}

fn argparse_fields(source: &str) -> Vec<ParamDecl> {
    match cli_form_projection("python", source) {
        CliFormProjection::Static { framework, fields } => {
            assert_eq!(framework, "argparse");
            fields
        }
        other => panic!("expected static argparse surface, got {other:?}\nsource:\n{source}"),
    }
}

fn one(source: &str) -> ParamDecl {
    let fields = argparse_fields(source);
    let [field] = fields.as_slice() else {
        panic!("expected exactly one argparse field, got {fields:?}\nsource:\n{source}");
    };
    field.clone()
}

#[test]
fn test_no_argparse_returns_none() {
    for source in ["print('hi')\n", "import argparse\n"] {
        assert!(matches!(
            cli_form_projection("python", source),
            CliFormProjection::Absent
        ));
    }
}

#[test]
fn test_syntax_error_returns_none() {
    assert!(matches!(
        cli_form_projection("python", "def broken(:\n"),
        CliFormProjection::Absent
    ));
}

#[test]
fn test_stitch_reads_eight_fields_in_source_order() {
    let fields = argparse_fields(STITCH);
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        [
            "inputs",
            "output",
            "direction",
            "gap",
            "bg",
            "match_size",
            "align",
            "no_sort",
        ]
    );
}

#[test]
fn test_stitch_positional_multiple_required() {
    let fields = argparse_fields(STITCH);
    let input = &fields[0];
    assert_eq!(input.flag, "");
    assert!(input.multiple);
    assert!(input.required);
    assert_eq!(input.parameter_type, ParameterType::Path);
    assert_eq!(input.help, "input images");
}

#[test]
fn test_argparse_path_type_spellings() {
    let source = concat!(
        "import argparse\nimport pathlib\n",
        "ap = argparse.ArgumentParser()\n",
        "ap.add_argument('--a', type=Path)\n",
        "ap.add_argument('--b', type=pathlib.Path)\n",
        "ap.add_argument('--c', type=argparse.FileType('w'))\n",
        "ap.add_argument('--d', type=FileType())\n",
        "ap.parse_args()\n",
    );
    let fields = argparse_fields(source);
    assert_eq!(
        fields.iter().map(|field| field.parameter_type).collect::<Vec<_>>(),
        [ParameterType::Path; 4]
    );
    assert!(fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_argparse_choices_beat_path_type() {
    let field = one(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--m', choices=['a', 'b'], type=Path)\nap.parse_args()\n",
    ));
    assert_eq!(field.parameter_type, ParameterType::Choice);
}

#[test]
fn test_stitch_required_flag_and_long_name_preferred() {
    let fields = argparse_fields(STITCH);
    let output = &fields[1];
    assert_eq!(output.flag, "--output");
    assert!(output.required);
    assert!(!output.degraded);
}

#[test]
fn test_stitch_choices_with_default() {
    let fields = argparse_fields(STITCH);
    let direction = &fields[2];
    assert_eq!(direction.parameter_type, ParameterType::Choice);
    assert_eq!(direction.choices, ["vertical", "horizontal"]);
    assert_eq!(
        direction.default,
        Some(ParameterValue::String("vertical".to_owned()))
    );
}

#[test]
fn test_stitch_int_field() {
    let fields = argparse_fields(STITCH);
    let gap = &fields[3];
    assert_eq!(gap.parameter_type, ParameterType::Int);
    assert_eq!(gap.default, Some(ParameterValue::Integer(0)));
}

#[test]
fn test_stitch_custom_type_degrades_field() {
    let fields = argparse_fields(STITCH);
    let bg = &fields[4];
    assert!(bg.degraded);
    assert_eq!(bg.help, "bg color");
}

#[test]
fn test_stitch_store_true_checkbox() {
    let fields = argparse_fields(STITCH);
    let matched = &fields[5];
    assert_eq!(matched.parameter_type, ParameterType::Bool);
    assert_eq!(matched.action, "store_true");
    assert_eq!(matched.default, Some(ParameterValue::Bool(false)));
    assert_eq!(matched.flag, "--match-size");
}

#[test]
fn test_store_false_defaults_on() {
    let field = one(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--no-color', action='store_false')\n",
    ));
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.default, Some(ParameterValue::Bool(true)));
}

#[test]
fn test_subparsers_degrade_whole_spec() {
    let projection = cli_form_projection(
        "python",
        concat!(
            "import argparse\nap = argparse.ArgumentParser()\nsub = ap.add_subparsers()\n",
            "p = sub.add_parser('x')\np.add_argument('--y')\n",
        ),
    );
    assert!(matches!(
        projection,
        CliFormProjection::Dynamic {
            framework,
            reason: DegradationReason::Subcommands,
        } if framework == "argparse"
    ));
}

#[test]
fn test_loop_generated_arguments_degrade_whole_spec() {
    let projection = cli_form_projection(
        "python",
        concat!(
            "import argparse\nap = argparse.ArgumentParser()\n",
            "for name in NAMES:\n    ap.add_argument(name)\n",
        ),
    );
    assert!(matches!(
        projection,
        CliFormProjection::Dynamic {
            framework,
            reason: DegradationReason::DynamicDeclaration,
        } if framework == "argparse"
    ));
}

#[test]
fn test_append_action_degrades_field_only() {
    let fields = argparse_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--tag', action='append')\nap.add_argument('--n', type=int)\n",
    ));
    assert!(fields[0].degraded);
    assert!(!fields[1].degraded);
}

#[test]
fn test_non_literal_choices_degrade_field() {
    let field = one(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--mode', choices=MODES)\n",
    ));
    assert!(field.degraded);
}

#[test]
fn test_help_and_version_actions_are_not_fields() {
    let fields = argparse_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser(add_help=False)\n",
        "ap.add_argument('--version', action='version', version='1.0')\n",
        "ap.add_argument('--real')\n",
    ));
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["real"]
    );
}

#[test]
fn test_secret_name_precheck() {
    let field = one(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--api-key')\n",
    );
    assert!(field.secret);
}

#[test]
fn test_optional_positional_star_not_required() {
    let field = one(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('files', nargs='*')\n",
    );
    assert!(!field.required);
    assert!(field.multiple);
}

#[test]
fn test_dest_override_wins() {
    let field = one(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--out-file', dest='target')\n",
    ));
    assert_eq!(field.name, "target");
    assert_eq!(field.flag, "--out-file");
}

#[test]
fn test_type_float_and_str_map_to_kinds() {
    let fields = argparse_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--ratio', type=float)\nap.add_argument('--label', type=str)\n",
    ));
    assert_eq!(fields[0].parameter_type, ParameterType::Float);
    assert_eq!(fields[1].parameter_type, ParameterType::Str);
    assert!(!fields[0].degraded);
    assert!(!fields[1].degraded);
}

#[test]
fn test_default_none_literal_does_not_degrade() {
    let field = one(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--x', default=None)\n",
    );
    assert!(!field.degraded);
    assert_eq!(field.default, None);
}

#[test]
fn test_non_literal_argument_name_skips_that_field_only() {
    let fields = argparse_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument(FLAG_NAME)\nap.add_argument('--real')\n",
    ));
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["real"]
    );
}

#[test]
fn test_short_flag_only_keeps_short_name() {
    let field = one("import argparse\nap = argparse.ArgumentParser()\nap.add_argument('-v')\n");
    assert_eq!(field.flag, "-v");
    assert_eq!(field.name, "v");
}

#[test]
fn test_field_order_matches_source_order() {
    let fields = argparse_fields(STITCH);
    assert_eq!((0..fields.len()).collect::<Vec<_>>(), (0..8).collect::<Vec<_>>());
}

#[test]
fn test_choices_win_over_type_for_kind() {
    let field = one(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--n', type=int, choices=[1, 2, 3])\n",
    ));
    assert_eq!(field.parameter_type, ParameterType::Choice);
    assert_eq!(field.choices, ["1", "2", "3"]);
}

#[test]
fn test_required_false_literal_is_not_required() {
    let fields = argparse_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--x', required=False)\nap.add_argument('y', nargs='?')\n",
    ));
    assert!(!fields[0].required);
    assert!(!fields[1].required);
    assert!(!fields[1].multiple);
}

#[test]
fn test_partly_non_literal_name_list_skips_that_field_only() {
    let fields = argparse_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('-x', EXTRA)\nap.add_argument('--real')\n",
    ));
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["real"]
    );
}

#[test]
fn test_flag_dest_only_strips_dashes_not_letters() {
    let field = one(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--Xterm')\n",
    );
    assert_eq!(field.name, "Xterm");
}

#[test]
fn test_computed_default_degrades_field() {
    let field = one(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--size', default=(1, 2))\n",
    ));
    assert_eq!(field.name, "size");
    assert!(field.degraded);
    assert_eq!(field.default, None);
}

#[test]
fn test_argparse_fixed_nargs_is_a_multi_value_field() {
    let fields = argparse_fields(concat!(
        "import argparse\n",
        "ap = argparse.ArgumentParser()\n",
        "ap.add_argument('--point', nargs=2, type=int)\n",
        "ap.add_argument('--one', nargs=1)\n",
    ));
    let point = fields.iter().find(|field| field.name == "point").unwrap();
    let one = fields.iter().find(|field| field.name == "one").unwrap();
    assert!(point.multiple);
    assert!(!point.repeat);
    assert!(!one.multiple);
}

#[test]
fn test_click_fixed_nargs_is_a_multi_value_field() {
    let fields = fields(
        "python",
        concat!(
            "import click\n",
            "@click.command()\n",
            "@click.option('--pair', nargs=2)\n",
            "def main(pair):\n    pass\n",
        ),
    );
    let pair = fields.iter().find(|field| field.name == "pair").unwrap();
    assert!(pair.multiple);
    assert!(!pair.repeat);
}

#[test]
fn test_click_multiple_with_fixed_nargs_is_not_modelled_at_all() {
    let fields = fields(
        "python",
        concat!(
            "import click\n",
            "@click.command()\n",
            "@click.option('--point', nargs=2, type=int, multiple=True)\n",
            "@click.option('--tag', multiple=True)\n",
            "def main(point, tag):\n    pass\n",
        ),
    );
    let names = fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>();
    assert!(!names.contains(&"point"));
    assert!(names.contains(&"tag"));
}

#[test]
fn test_click_multiple_with_nargs_one_is_still_modelled() {
    let fields = fields(
        "python",
        concat!(
            "import click\n",
            "@click.command()\n",
            "@click.option('--tag', nargs=1, multiple=True)\n",
            "def main(tag):\n    pass\n",
        ),
    );
    let tag = fields.iter().find(|field| field.name == "tag").unwrap();
    assert_eq!((tag.multiple, tag.repeat), (true, true));
}
