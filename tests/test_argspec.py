"""Static argparse reader: literal add_argument calls -> form fields, honest degradation."""

from __future__ import annotations

from skit.langs.python import argspec

# The reference/stitch.py shape: a realistic AI-written argparse script (parser built
# inside main(), Path types, choices, store_true flags, one unreadable custom type).
STITCH = """
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
"""


def test_no_argparse_returns_none():
    assert argspec.read_argparse("print('hi')\n") is None
    assert argspec.read_argparse("import argparse\n") is None  # imported but never used


def test_syntax_error_returns_none():
    assert argspec.read_argparse("def broken(:\n") is None


def test_stitch_reads_eight_fields_in_source_order():
    spec = argspec.read_argparse(STITCH)
    assert spec is not None
    assert spec.ok
    assert [f.name for f in spec.fields] == [
        "inputs",
        "output",
        "direction",
        "gap",
        "bg",
        "match_size",
        "align",
        "no_sort",
    ]


def test_stitch_positional_multiple_required():
    spec = argspec.read_argparse(STITCH)
    assert spec is not None
    inputs = spec.fields[0]
    assert inputs.flag == ""
    assert inputs.multiple is True
    assert inputs.required is True
    assert inputs.type == "str"  # Path renders as text
    assert inputs.help == "input images"


def test_stitch_required_flag_and_long_name_preferred():
    spec = argspec.read_argparse(STITCH)
    assert spec is not None
    output = spec.fields[1]
    assert output.flag == "--output"
    assert output.required is True
    assert output.degraded is False


def test_stitch_choices_with_default():
    spec = argspec.read_argparse(STITCH)
    assert spec is not None
    direction = spec.fields[2]
    assert direction.type == "choice"
    assert direction.choices == ("vertical", "horizontal")
    assert direction.default == "vertical"


def test_stitch_int_field():
    spec = argspec.read_argparse(STITCH)
    assert spec is not None
    gap = spec.fields[3]
    assert gap.type == "int"
    assert gap.default == 0


def test_stitch_custom_type_degrades_field():
    spec = argspec.read_argparse(STITCH)
    assert spec is not None
    bg = spec.fields[4]
    assert bg.degraded is True
    assert bg.help == "bg color"  # help survives degradation — it's the user's hint


def test_stitch_store_true_checkbox():
    spec = argspec.read_argparse(STITCH)
    assert spec is not None
    match_size = spec.fields[5]
    assert match_size.type == "bool"
    assert match_size.action == "store_true"
    assert match_size.default is False
    assert match_size.flag == "--match-size"


def test_store_false_defaults_on():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--no-color', action='store_false')\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.type == "bool"
    assert f.default is True


def test_subparsers_degrade_whole_spec():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\nsub = ap.add_subparsers()\n"
        "p = sub.add_parser('x')\np.add_argument('--y')\n"
    )
    assert spec is not None
    assert spec.ok is False
    assert spec.reason == "subparsers"


def test_loop_generated_arguments_degrade_whole_spec():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "for name in NAMES:\n    ap.add_argument(name)\n"
    )
    assert spec is not None
    assert spec.ok is False
    assert spec.reason == "dynamic"


def test_append_action_degrades_field_only():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--tag', action='append')\nap.add_argument('--n', type=int)\n"
    )
    assert spec is not None
    assert spec.ok is True
    assert spec.fields[0].degraded is True
    assert spec.fields[1].degraded is False


def test_non_literal_choices_degrade_field():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--mode', choices=MODES)\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True


def test_help_and_version_actions_are_not_fields():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser(add_help=False)\n"
        "ap.add_argument('--version', action='version', version='1.0')\n"
        "ap.add_argument('--real')\n"
    )
    assert spec is not None
    assert [f.name for f in spec.fields] == ["real"]


def test_secret_name_precheck():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--api-key')\n"
    )
    assert spec is not None
    assert spec.fields[0].secret is True


def test_optional_positional_star_not_required():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('files', nargs='*')\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.required is False
    assert f.multiple is True


def test_dest_override_wins():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--out-file', dest='target')\n"
    )
    assert spec is not None
    assert spec.fields[0].name == "target"
    assert spec.fields[0].flag == "--out-file"


# --------------------------------------------------------------------------
# mutation hardening
# --------------------------------------------------------------------------


def test_type_float_and_str_map_to_kinds():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--ratio', type=float)\nap.add_argument('--label', type=str)\n"
    )
    assert spec is not None
    assert spec.fields[0].type == "float"
    assert spec.fields[1].type == "str"
    assert spec.fields[0].degraded is False
    assert spec.fields[1].degraded is False


def test_default_none_literal_does_not_degrade():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--x', default=None)\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is False
    assert spec.fields[0].default is None


def test_non_literal_argument_name_skips_that_field_only():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument(FLAG_NAME)\nap.add_argument('--real')\n"
    )
    assert spec is not None
    assert [f.name for f in spec.fields] == ["real"]


def test_short_flag_only_keeps_short_name():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('-v')\n"
    )
    assert spec is not None
    assert spec.fields[0].flag == "-v"
    assert spec.fields[0].name == "v"


def test_field_order_matches_source_order():
    spec = argspec.read_argparse(STITCH)
    assert spec is not None
    # Declaration order is carried by list position now (no per-field order attribute):
    # the eight add_argument calls come back as eight fields, indexed in source order.
    assert [i for i, _ in enumerate(spec.fields)] == list(range(8))


def test_choices_win_over_type_for_kind():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--n', type=int, choices=[1, 2, 3])\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.type == "choice"
    assert f.choices == ("1", "2", "3")


def test_required_false_literal_is_not_required():
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--x', required=False)\nap.add_argument('y', nargs='?')\n"
    )
    assert spec is not None
    assert spec.fields[0].required is False
    assert spec.fields[1].required is False
    assert spec.fields[1].multiple is False  # '?' is optional, not multiple


def test_partly_non_literal_name_list_skips_that_field_only():
    # A first literal name but a *second*, non-literal positional (len(names) != len(args)):
    # we can't trust the declaration, so the whole call is skipped — this is the `len != len`
    # half of the guard, distinct from the empty-names half already covered above.
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('-x', EXTRA)\nap.add_argument('--real')\n"
    )
    assert spec is not None
    assert [f.name for f in spec.fields] == ["real"]


def test_flag_dest_only_strips_dashes_not_letters():
    # dest is derived by stripping *leading dashes* — not arbitrary characters. A flag whose
    # name begins with a capital letter after the dashes must keep that letter.
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--Xterm')\n"
    )
    assert spec is not None
    assert spec.fields[0].name == "Xterm"


def test_computed_default_degrades_field():
    # A non-literal default (here a tuple) can't be modeled: the field shows but degrades so it
    # is omitted when left empty and the script's own default applies. `is True` also pins that
    # it degrades rather than silently staying modelled.
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--size', default=(1, 2))\n"
    )
    assert spec is not None
    assert spec.fields[0].name == "size"
    assert spec.fields[0].degraded is True
    assert spec.fields[0].default is None  # a computed default is never read as a value
