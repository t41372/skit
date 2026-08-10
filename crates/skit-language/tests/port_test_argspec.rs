//! Mechanical port of the Python oracle module `tests/test_argspec.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name so it
//! traces back to its origin, and the Python "WHY" comment is preserved above it.
//!
//! Concept mapping used throughout:
//! - Python `argspec.read_argparse(src)` / `argspec.read_cli(src)` -> `parsed(src).cli_surface()`.
//! - Python `spec is None` (no parser, or an argparse-imported-but-unused script) ->
//!   `CliSurface::Absent`. A syntax error is `ParseOutcome::SyntaxError` (no parsed document),
//!   which is the faithful mapping of the Python `read_*` returning `None` on `SyntaxError`.
//! - Python `spec.ok is True` -> `CliSurface::Static(surface)`.
//! - Python `spec.ok is False` + `spec.reason == "subparsers"` ->
//!   `CliSurface::Dynamic(surface)` with `DegradationReason::Subcommands`.
//! - Python `spec.reason == "dynamic"` -> `DegradationReason::DynamicDeclaration`.
//! - Python `spec.fields[i]` -> `surface.fields[i].declaration` (a `ParamDecl`).
//! - Python field `type` strings -> `ParameterType`; Python `default` -> `Option<ParameterValue>`
//!   (`ParameterValue::String`, not `Text`); Python tuple `choices` -> `Vec<String>`.
//! - Python `f.flag == ""` -> `field.flag.is_empty()` (positional); Python `f.action == ""` ->
//!   `field.action.is_empty()`.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_language::{CliSurface, DegradationReason, ParseOutcome, ParsedDocument, parse_document};

fn parsed(source: &str) -> ParsedDocument {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid Python, got {other:?}"),
    }
}

/// Python `spec.ok` static path: the fields of a `CliSurface::Static` surface.
fn static_fields(source: &str) -> Vec<ParamDecl> {
    let CliSurface::Static(surface) = parsed(source).cli_surface() else {
        panic!("expected a static CLI surface");
    };
    surface
        .fields
        .into_iter()
        .map(|field| field.declaration)
        .collect()
}

/// Python `[f.name for f in spec.fields]`.
fn names(fields: &[ParamDecl]) -> Vec<&str> {
    fields.iter().map(|field| field.name.as_str()).collect()
}

/// Python `{f.name: f for f in spec.fields}`.
fn by_name(fields: &[ParamDecl]) -> BTreeMap<&str, &ParamDecl> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect()
}

/// Python `spec is None`: no CLI parser is present at all (a plain script).
fn is_absent(source: &str) -> bool {
    matches!(parsed(source).cli_surface(), CliSurface::Absent)
}

/// Python `spec.ok is False`: the typed whole-surface degradation reason.
fn dynamic_reason(source: &str) -> DegradationReason {
    match parsed(source).cli_surface() {
        CliSurface::Dynamic(surface) => surface.reason,
        _ => panic!("expected a dynamic CLI surface"),
    }
}

/// The reference/stitch.py shape: a realistic AI-written argparse script (parser built
/// inside main(), Path types, choices, store_true flags, one unreadable custom type).
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
    assert!(is_absent("print('hi')\n"));
    assert!(is_absent("import argparse\n")); // imported but never used
}

#[test]
fn test_syntax_error_returns_none() {
    assert!(matches!(
        parse_document("python", "def broken(:\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

#[test]
fn test_stitch_reads_eight_fields_in_source_order() {
    let fields = static_fields(STITCH);
    assert_eq!(
        names(&fields),
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
    let fields = static_fields(STITCH);
    let inputs = &fields[0];
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
    let src = concat!(
        "import argparse\nimport pathlib\n",
        "ap = argparse.ArgumentParser()\n",
        "ap.add_argument('--a', type=Path)\n",
        "ap.add_argument('--b', type=pathlib.Path)\n",
        "ap.add_argument('--c', type=argparse.FileType('w'))\n",
        "ap.add_argument('--d', type=FileType())\n",
        "ap.parse_args()\n",
    );
    let fields = static_fields(src);
    assert_eq!(
        fields
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
    assert!(fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_argparse_choices_beat_path_type() {
    // choices win exactly as they do over scalar type=: the selector already
    // constrains input, so the field stays a choice.
    let src = concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--m', choices=['a', 'b'], type=Path)\nap.parse_args()\n",
    );
    let fields = static_fields(src);
    assert_eq!(fields[0].parameter_type, ParameterType::Choice);
}

#[test]
fn test_stitch_required_flag_and_long_name_preferred() {
    let fields = static_fields(STITCH);
    let output = &fields[1];
    assert_eq!(output.flag, "--output");
    assert!(output.required);
    assert!(!output.degraded);
}

#[test]
fn test_stitch_choices_with_default() {
    let fields = static_fields(STITCH);
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
    let fields = static_fields(STITCH);
    let gap = &fields[3];
    assert_eq!(gap.parameter_type, ParameterType::Int);
    assert_eq!(gap.default, Some(ParameterValue::Integer(0)));
}

#[test]
fn test_stitch_custom_type_degrades_field() {
    let fields = static_fields(STITCH);
    let bg = &fields[4];
    assert!(bg.degraded);
    assert_eq!(bg.help, "bg color"); // help survives degradation -- it's the user's hint
}

#[test]
fn test_stitch_store_true_checkbox() {
    let fields = static_fields(STITCH);
    let match_size = &fields[5];
    assert_eq!(match_size.parameter_type, ParameterType::Bool);
    assert_eq!(match_size.action, "store_true");
    assert_eq!(match_size.default, Some(ParameterValue::Bool(false)));
    assert_eq!(match_size.flag, "--match-size");
}

#[test]
fn test_store_false_defaults_on() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--no-color', action='store_false')\n",
    ));
    let field = &fields[0];
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.default, Some(ParameterValue::Bool(true)));
}

#[test]
fn test_subparsers_degrade_whole_spec() {
    assert_eq!(
        dynamic_reason(concat!(
            "import argparse\nap = argparse.ArgumentParser()\nsub = ap.add_subparsers()\n",
            "p = sub.add_parser('x')\np.add_argument('--y')\n",
        )),
        DegradationReason::Subcommands
    );
}

#[test]
fn test_loop_generated_arguments_degrade_whole_spec() {
    assert_eq!(
        dynamic_reason(concat!(
            "import argparse\nap = argparse.ArgumentParser()\n",
            "for name in NAMES:\n    ap.add_argument(name)\n",
        )),
        DegradationReason::DynamicDeclaration
    );
}

#[test]
fn test_append_action_degrades_field_only() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--tag', action='append')\nap.add_argument('--n', type=int)\n",
    ));
    assert!(fields[0].degraded);
    assert!(!fields[1].degraded);
}

#[test]
fn test_non_literal_choices_degrade_field() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--mode', choices=MODES)\n",
    ));
    assert!(fields[0].degraded);
}

#[test]
fn test_help_and_version_actions_are_not_fields() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser(add_help=False)\n",
        "ap.add_argument('--version', action='version', version='1.0')\n",
        "ap.add_argument('--real')\n",
    ));
    assert_eq!(names(&fields), ["real"]);
}

#[test]
fn test_secret_name_precheck() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--api-key')\n",
    ));
    assert!(fields[0].secret);
}

#[test]
fn test_optional_positional_star_not_required() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('files', nargs='*')\n",
    ));
    let field = &fields[0];
    assert!(!field.required);
    assert!(field.multiple);
}

#[test]
fn test_dest_override_wins() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--out-file', dest='target')\n",
    ));
    assert_eq!(fields[0].name, "target");
    assert_eq!(fields[0].flag, "--out-file");
}

// --------------------------------------------------------------------------
// mutation hardening
// --------------------------------------------------------------------------

#[test]
fn test_type_float_and_str_map_to_kinds() {
    let fields = static_fields(concat!(
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
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--x', default=None)\n",
    ));
    assert!(!fields[0].degraded);
    assert!(fields[0].default.is_none());
}

#[test]
fn test_non_literal_argument_name_skips_that_field_only() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument(FLAG_NAME)\nap.add_argument('--real')\n",
    ));
    assert_eq!(names(&fields), ["real"]);
}

#[test]
fn test_short_flag_only_keeps_short_name() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('-v')\n",
    ));
    assert_eq!(fields[0].flag, "-v");
    assert_eq!(fields[0].name, "v");
}

#[test]
fn test_field_order_matches_source_order() {
    // Declaration order is carried by list position now (no per-field order attribute):
    // the eight add_argument calls come back as eight fields, indexed in source order.
    let fields = static_fields(STITCH);
    assert_eq!(fields.len(), 8);
}

#[test]
fn test_choices_win_over_type_for_kind() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--n', type=int, choices=[1, 2, 3])\n",
    ));
    let field = &fields[0];
    assert_eq!(field.parameter_type, ParameterType::Choice);
    assert_eq!(field.choices, ["1", "2", "3"]);
}

#[test]
fn test_required_false_literal_is_not_required() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--x', required=False)\nap.add_argument('y', nargs='?')\n",
    ));
    assert!(!fields[0].required);
    assert!(!fields[1].required);
    assert!(!fields[1].multiple); // '?' is optional, not multiple
}

#[test]
fn test_partly_non_literal_name_list_skips_that_field_only() {
    // A first literal name but a *second*, non-literal positional (len(names) != len(args)):
    // we can't trust the declaration, so the whole call is skipped -- this is the `len != len`
    // half of the guard, distinct from the empty-names half already covered above.
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('-x', EXTRA)\nap.add_argument('--real')\n",
    ));
    assert_eq!(names(&fields), ["real"]);
}

#[test]
fn test_flag_dest_only_strips_dashes_not_letters() {
    // dest is derived by stripping *leading dashes* -- not arbitrary characters. A flag whose
    // name begins with a capital letter after the dashes must keep that letter.
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--Xterm')\n",
    ));
    assert_eq!(fields[0].name, "Xterm");
}

#[test]
fn test_computed_default_degrades_field() {
    // A non-literal default (here a tuple) can't be modeled: the field shows but degrades so it
    // is omitted when left empty and the script's own default applies. `is True` also pins that
    // it degrades rather than silently staying modelled.
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--size', default=(1, 2))\n",
    ));
    assert_eq!(fields[0].name, "size");
    assert!(fields[0].degraded);
    assert!(fields[0].default.is_none()); // a computed default is never read as a value
}

// ---------------------------------------------------------------------------
// nargs arity: a fixed count is a multi-value field, and click's multiple+nargs
// pair has no shape at all
// ---------------------------------------------------------------------------

#[test]
fn test_argparse_fixed_nargs_is_a_multi_value_field() {
    // `nargs=2` wants `--point 1 2` -- the same one-flag-many-values shape as `+`/`*`.
    // Modelled as single, the only legal input went through as one quoted token and
    // argparse answered "expected 2 arguments" at exit 2.
    let fields = static_fields(concat!(
        "import argparse\n",
        "ap = argparse.ArgumentParser()\n",
        "ap.add_argument('--point', nargs=2, type=int)\n",
        "ap.add_argument('--one', nargs=1)\n",
    ));
    let by = by_name(&fields);
    assert!(by["point"].multiple);
    assert!(!by["point"].repeat); // nargs grammar, not repeat-the-flag
    assert!(!by["one"].multiple); // nargs=1 still takes exactly one value
}

#[test]
fn test_click_fixed_nargs_is_a_multi_value_field() {
    let fields = static_fields(concat!(
        "import click\n",
        "@click.command()\n",
        "@click.option('--pair', nargs=2)\n",
        "def main(pair):\n    pass\n",
    ));
    let pair = by_name(&fields)["pair"];
    assert!(pair.multiple);
    assert!(!pair.repeat);
}

#[test]
fn test_click_multiple_with_fixed_nargs_is_not_modelled_at_all() {
    // `multiple=True, nargs=2` repeats the flag AND takes two values per occurrence.
    // Nothing in a ParamDecl records that arity, so there is no assembly shape: the option
    // is dropped to the passthrough extra-args field rather than emitted as
    // `--point 1 --point 2`, which click rejects at exit 2.
    let fields = static_fields(concat!(
        "import click\n",
        "@click.command()\n",
        "@click.option('--point', nargs=2, type=int, multiple=True)\n",
        "@click.option('--tag', multiple=True)\n",
        "def main(point, tag):\n    pass\n",
    ));
    let names = names(&fields);
    assert!(!names.contains(&"point"));
    assert!(names.contains(&"tag")); // plain multiple= is still modelled (repeat the flag)
}

#[test]
fn test_click_multiple_with_nargs_one_is_still_modelled() {
    // Only a nargs GREATER than one is unrepresentable: `nargs=1` is the ordinary
    // one-value-per-occurrence shape click's multiple= already means.
    let fields = static_fields(concat!(
        "import click\n",
        "@click.command()\n",
        "@click.option('--tag', nargs=1, multiple=True)\n",
        "def main(tag):\n    pass\n",
    ));
    let tag = by_name(&fields)["tag"];
    assert_eq!((tag.multiple, tag.repeat), (true, true));
}
