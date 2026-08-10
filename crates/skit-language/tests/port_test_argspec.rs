//! Mechanical port of the Python oracle module `tests/test_argspec.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name and the Python
//! "WHY" comment is preserved verbatim above it.
//!
//! Concept mapping:
//! - Python `argspec.read_argparse` / `read_cli` -> `parse_document("python", source)` followed by
//!   `ParsedDocument::cli_surface()`.
//! - Python `None` -> a syntax failure or `CliSurface::Absent`.
//! - Python `ArgSpec(ok=False, reason="subparsers"|"dynamic")` -> `CliSurface::Dynamic` with the
//!   corresponding typed `DegradationReason`.
//! - Python `ArgSpec.fields` -> the `ParamDecl`s inside `CliSurface::Static` in source/runtime order.

use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_language::{CliSurface, DegradationReason, ParseOutcome, parse_document};

#[derive(Debug)]
struct Spec {
    framework: String,
    ok: bool,
    reason: Option<DegradationReason>,
    fields: Vec<ParamDecl>,
}

fn read_spec(source: &str) -> Option<Spec> {
    let document = match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document,
        _ => return None,
    };
    match document.cli_surface() {
        CliSurface::Absent => None,
        CliSurface::Static(surface) => Some(Spec {
            framework: surface.framework,
            ok: true,
            reason: None,
            fields: surface
                .fields
                .into_iter()
                .map(|field| field.declaration)
                .collect(),
        }),
        CliSurface::Dynamic(surface) => Some(Spec {
            framework: surface.framework,
            ok: false,
            reason: Some(surface.reason),
            fields: Vec::new(),
        }),
    }
}

fn read_argparse(source: &str) -> Option<Spec> {
    let spec = read_spec(source)?;
    (spec.framework == "argparse").then_some(spec)
}

fn read_cli(source: &str) -> Option<Spec> {
    read_spec(source)
}

fn names(spec: &Spec) -> Vec<&str> {
    spec.fields
        .iter()
        .map(|field| field.name.as_str())
        .collect()
}

fn field<'a>(spec: &'a Spec, name: &str) -> &'a ParamDecl {
    spec.fields
        .iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("missing field {name}"))
}

// The reference/stitch.py shape: a realistic AI-written argparse script (parser built
// inside main(), Path types, choices, store_true flags, one unreadable custom type).
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

#[test]
fn test_no_argparse_returns_none() {
    assert!(read_argparse("print('hi')\n").is_none());
    assert!(read_argparse("import argparse\n").is_none()); // imported but never used
}

#[test]
fn test_syntax_error_returns_none() {
    assert!(read_argparse("def broken(:\n").is_none());
}

#[test]
fn test_stitch_reads_eight_fields_in_source_order() {
    let spec = read_argparse(STITCH).unwrap();
    assert!(spec.ok);
    assert_eq!(
        names(&spec),
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
    let spec = read_argparse(STITCH).unwrap();
    let inputs = &spec.fields[0];
    assert!(inputs.flag.is_empty());
    assert!(inputs.multiple);
    assert!(inputs.required);
    assert_eq!(inputs.parameter_type, ParameterType::Path); // type=Path is a path signal (docs/design/path.md)
    assert_eq!(inputs.help, "input images");
}

#[test]
fn test_argparse_path_type_spellings() {
    // Bare Path, dotted pathlib.Path, and FileType (bare and dotted) all mean "the
    // user supplies a filename"; none of them degrade the field.
    let source = concat!(
        "import argparse\nimport pathlib\n",
        "ap = argparse.ArgumentParser()\n",
        "ap.add_argument('--a', type=Path)\n",
        "ap.add_argument('--b', type=pathlib.Path)\n",
        "ap.add_argument('--c', type=argparse.FileType('w'))\n",
        "ap.add_argument('--d', type=FileType())\n",
        "ap.parse_args()\n",
    );
    let spec = read_argparse(source).unwrap();
    assert_eq!(
        spec.fields
            .iter()
            .map(|field| field.parameter_type)
            .collect::<Vec<_>>(),
        [
            ParameterType::Path,
            ParameterType::Path,
            ParameterType::Path,
            ParameterType::Path,
        ]
    );
    assert!(spec.fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_argparse_choices_beat_path_type() {
    // choices win exactly as they do over scalar type=: the selector already
    // constrains input, so the field stays a choice.
    let source = concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--m', choices=['a', 'b'], type=Path)\nap.parse_args()\n",
    );
    let spec = read_argparse(source).unwrap();
    assert_eq!(spec.fields[0].parameter_type, ParameterType::Choice);
}

#[test]
fn test_stitch_required_flag_and_long_name_preferred() {
    let spec = read_argparse(STITCH).unwrap();
    let output = &spec.fields[1];
    assert_eq!(output.flag, "--output");
    assert!(output.required);
    assert!(!output.degraded);
}

#[test]
fn test_stitch_choices_with_default() {
    let spec = read_argparse(STITCH).unwrap();
    let direction = &spec.fields[2];
    assert_eq!(direction.parameter_type, ParameterType::Choice);
    assert_eq!(direction.choices, ["vertical", "horizontal"]);
    assert_eq!(
        direction.default,
        Some(ParameterValue::String("vertical".to_owned()))
    );
}

#[test]
fn test_stitch_int_field() {
    let spec = read_argparse(STITCH).unwrap();
    let gap = &spec.fields[3];
    assert_eq!(gap.parameter_type, ParameterType::Int);
    assert_eq!(gap.default, Some(ParameterValue::Integer(0)));
}

#[test]
fn test_stitch_custom_type_degrades_field() {
    let spec = read_argparse(STITCH).unwrap();
    let bg = &spec.fields[4];
    assert!(bg.degraded);
    assert_eq!(bg.help, "bg color"); // help survives degradation — it's the user's hint
}

#[test]
fn test_stitch_store_true_checkbox() {
    let spec = read_argparse(STITCH).unwrap();
    let match_size = &spec.fields[5];
    assert_eq!(match_size.parameter_type, ParameterType::Bool);
    assert_eq!(match_size.action, "store_true");
    assert_eq!(match_size.default, Some(ParameterValue::Bool(false)));
    assert_eq!(match_size.flag, "--match-size");
}

#[test]
fn test_store_false_defaults_on() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--no-color', action='store_false')\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.default, Some(ParameterValue::Bool(true)));
}

#[test]
fn test_subparsers_degrade_whole_spec() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\nsub = ap.add_subparsers()\n",
        "p = sub.add_parser('x')\np.add_argument('--y')\n",
    ))
    .unwrap();
    assert!(!spec.ok);
    assert_eq!(spec.reason, Some(DegradationReason::Subcommands));
}

#[test]
fn test_loop_generated_arguments_degrade_whole_spec() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "for name in NAMES:\n    ap.add_argument(name)\n",
    ))
    .unwrap();
    assert!(!spec.ok);
    assert_eq!(
        spec.reason,
        Some(DegradationReason::DynamicDeclaration)
    );
}

#[test]
fn test_append_action_degrades_field_only() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--tag', action='append')\nap.add_argument('--n', type=int)\n",
    ))
    .unwrap();
    assert!(spec.ok);
    assert!(spec.fields[0].degraded);
    assert!(!spec.fields[1].degraded);
}

#[test]
fn test_non_literal_choices_degrade_field() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--mode', choices=MODES)\n",
    ))
    .unwrap();
    assert!(spec.fields[0].degraded);
}

#[test]
fn test_help_and_version_actions_are_not_fields() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser(add_help=False)\n",
        "ap.add_argument('--version', action='version', version='1.0')\n",
        "ap.add_argument('--real')\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["real"]);
}

#[test]
fn test_secret_name_precheck() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--api-key')\n",
    ))
    .unwrap();
    assert!(spec.fields[0].secret);
}

#[test]
fn test_optional_positional_star_not_required() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('files', nargs='*')\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert!(!field.required);
    assert!(field.multiple);
}

#[test]
fn test_dest_override_wins() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--out-file', dest='target')\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].name, "target");
    assert_eq!(spec.fields[0].flag, "--out-file");
}

// --------------------------------------------------------------------------
// mutation hardening
// --------------------------------------------------------------------------

#[test]
fn test_type_float_and_str_map_to_kinds() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--ratio', type=float)\nap.add_argument('--label', type=str)\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].parameter_type, ParameterType::Float);
    assert_eq!(spec.fields[1].parameter_type, ParameterType::Str);
    assert!(!spec.fields[0].degraded);
    assert!(!spec.fields[1].degraded);
}

#[test]
fn test_default_none_literal_does_not_degrade() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--x', default=None)\n",
    ))
    .unwrap();
    assert!(!spec.fields[0].degraded);
    assert_eq!(spec.fields[0].default, None);
}

#[test]
fn test_non_literal_argument_name_skips_that_field_only() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument(FLAG_NAME)\nap.add_argument('--real')\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["real"]);
}

#[test]
fn test_short_flag_only_keeps_short_name() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('-v')\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].flag, "-v");
    assert_eq!(spec.fields[0].name, "v");
}

#[test]
fn test_field_order_matches_source_order() {
    let spec = read_argparse(STITCH).unwrap();
    // Declaration order is carried by list position now (no per-field order attribute):
    // the eight add_argument calls come back as eight fields, indexed in source order.
    assert_eq!((0..spec.fields.len()).collect::<Vec<_>>(), (0..8).collect::<Vec<_>>());
}

#[test]
fn test_choices_win_over_type_for_kind() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--n', type=int, choices=[1, 2, 3])\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert_eq!(field.parameter_type, ParameterType::Choice);
    assert_eq!(field.choices, ["1", "2", "3"]);
}

#[test]
fn test_required_false_literal_is_not_required() {
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--x', required=False)\nap.add_argument('y', nargs='?')\n",
    ))
    .unwrap();
    assert!(!spec.fields[0].required);
    assert!(!spec.fields[1].required);
    assert!(!spec.fields[1].multiple); // '?' is optional, not multiple
}

#[test]
fn test_partly_non_literal_name_list_skips_that_field_only() {
    // A first literal name but a *second*, non-literal positional (len(names) != len(args)):
    // we can't trust the declaration, so the whole call is skipped — this is the `len != len`
    // half of the guard, distinct from the empty-names half already covered above.
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('-x', EXTRA)\nap.add_argument('--real')\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["real"]);
}

#[test]
fn test_flag_dest_only_strips_dashes_not_letters() {
    // dest is derived by stripping *leading dashes* — not arbitrary characters. A flag whose
    // name begins with a capital letter after the dashes must keep that letter.
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--Xterm')\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].name, "Xterm");
}

#[test]
fn test_computed_default_degrades_field() {
    // A non-literal default (here a tuple) can't be modeled: the field shows but degrades so it
    // is omitted when left empty and the script's own default applies. `is True` also pins that
    // it degrades rather than silently staying modelled.
    let spec = read_argparse(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--size', default=(1, 2))\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].name, "size");
    assert!(spec.fields[0].degraded);
    assert_eq!(spec.fields[0].default, None); // a computed default is never read as a value
}

// ---------------------------------------------------------------------------
// nargs arity: a fixed count is a multi-value field, and click's multiple+nargs
// pair has no shape at all
// ---------------------------------------------------------------------------

#[test]
fn test_argparse_fixed_nargs_is_a_multi_value_field() {
    // `nargs=2` wants `--point 1 2` — the same one-flag-many-values shape as `+`/`*`.
    // Modelled as single, the only legal input went through as one quoted token and
    // argparse answered "expected 2 arguments" at exit 2.
    let spec = read_argparse(concat!(
        "import argparse\n",
        "ap = argparse.ArgumentParser()\n",
        "ap.add_argument('--point', nargs=2, type=int)\n",
        "ap.add_argument('--one', nargs=1)\n",
    ))
    .unwrap();
    assert!(field(&spec, "point").multiple);
    assert!(!field(&spec, "point").repeat); // nargs grammar, not repeat-the-flag
    assert!(!field(&spec, "one").multiple); // nargs=1 still takes exactly one value
}

#[test]
fn test_click_fixed_nargs_is_a_multi_value_field() {
    let spec = read_cli(concat!(
        "import click\n",
        "@click.command()\n",
        "@click.option('--pair', nargs=2)\n",
        "def main(pair):\n    pass\n",
    ))
    .unwrap();
    let field = field(&spec, "pair");
    assert!(field.multiple);
    assert!(!field.repeat);
}

#[test]
fn test_click_multiple_with_fixed_nargs_is_not_modelled_at_all() {
    // `multiple=True, nargs=2` repeats the flag AND takes two values per occurrence.
    // Nothing in a ParamDecl records that arity, so there is no assembly shape: the option
    // is dropped to the passthrough extra-args field rather than emitted as
    // `--point 1 --point 2`, which click rejects at exit 2.
    let spec = read_cli(concat!(
        "import click\n",
        "@click.command()\n",
        "@click.option('--point', nargs=2, type=int, multiple=True)\n",
        "@click.option('--tag', multiple=True)\n",
        "def main(point, tag):\n    pass\n",
    ))
    .unwrap();
    let names = names(&spec);
    assert!(!names.contains(&"point"));
    assert!(names.contains(&"tag")); // plain multiple= is still modelled (repeat the flag)
}

#[test]
fn test_click_multiple_with_nargs_one_is_still_modelled() {
    // Only a nargs GREATER than one is unrepresentable: `nargs=1` is the ordinary
    // one-value-per-occurrence shape click's multiple= already means.
    let spec = read_cli(concat!(
        "import click\n",
        "@click.command()\n",
        "@click.option('--tag', nargs=1, multiple=True)\n",
        "def main(tag):\n    pass\n",
    ))
    .unwrap();
    let field = field(&spec, "tag");
    assert_eq!((field.multiple, field.repeat), (true, true));
}
