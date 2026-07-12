"""Static click/typer reading: the unified form's third source, beyond argparse."""

from __future__ import annotations

from skit.langs.python import argspec

CLICK_SCRIPT = """
import click

@click.command()
@click.option("--output", "-o", required=True, help="output path")
@click.option("--gap", type=int, default=0)
@click.option("--mode", type=click.Choice(["a", "b"]), default="a")
@click.option("--fast", is_flag=True)
@click.argument("inputs", nargs=-1)
def main(output, gap, mode, fast, inputs):
    pass
"""

TYPER_SCRIPT = """
import typer

app = typer.Typer()

@app.command()
def main(
    inputs: str = typer.Argument(...),
    output: str = typer.Option(..., "--output", "-o", help="output path"),
    gap: int = typer.Option(0),
    fast: bool = typer.Option(False, "--fast"),
    label: str = "x",
):
    pass
"""


# --------------------------------------------------------------------------
# click
# --------------------------------------------------------------------------


def test_click_fields_bottom_up_order_matches_runtime():
    spec = argspec.read_cli(CLICK_SCRIPT)
    assert spec is not None
    assert spec.ok
    # click applies decorators bottom-up: the bottom @click.argument is param #1.
    assert [f.dest for f in spec.fields] == ["inputs", "fast", "mode", "gap", "output"]


def test_click_argument_variadic_is_multiple_not_required():
    spec = argspec.read_cli(CLICK_SCRIPT)
    assert spec is not None
    inputs = spec.fields[0]
    assert inputs.flag == ""
    assert inputs.multiple is True
    assert inputs.required is False  # nargs=-1 lifts click's argument-required default


def test_click_is_flag_choice_int_and_required():
    spec = argspec.read_cli(CLICK_SCRIPT)
    assert spec is not None
    by = {f.dest: f for f in spec.fields}
    assert by["fast"].kind == "bool"
    assert by["fast"].action == "store_true"
    assert by["mode"].kind == "choice"
    assert by["mode"].choices == ["a", "b"]
    assert by["mode"].default == "a"
    assert by["gap"].kind == "int"
    assert by["gap"].default == 0
    assert by["output"].required is True
    assert by["output"].flag == "--output"
    assert by["output"].help == "output path"


def test_click_plain_argument_is_required():
    spec = argspec.read_cli(
        "import click\n@click.command()\n@click.argument('name')\ndef m(name): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].required is True


def test_click_group_degrades_as_subcommands():
    spec = argspec.read_cli(
        "import click\n@click.group()\ndef cli(): pass\n"
        "@click.command()\n@click.option('--x')\ndef sub(x): pass\n"
    )
    assert spec is not None
    assert spec.ok is False
    assert spec.reason == "subparsers"


def test_click_count_option_degrades_field():
    spec = argspec.read_cli(
        "import click\n@click.command()\n@click.option('-v', '--verbose', count=True)\n"
        "def m(verbose): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True


# --------------------------------------------------------------------------
# typer
# --------------------------------------------------------------------------


def test_typer_signature_order_and_kinds():
    spec = argspec.read_cli(TYPER_SCRIPT)
    assert spec is not None
    assert spec.ok
    assert [f.dest for f in spec.fields] == ["inputs", "output", "gap", "fast", "label"]
    by = {f.dest: f for f in spec.fields}
    assert by["inputs"].flag == ""  # Argument -> positional
    assert by["inputs"].required is True  # Ellipsis default
    assert by["output"].flag == "--output"
    assert by["output"].required is True
    assert by["output"].help == "output path"
    assert by["gap"].kind == "int"
    assert by["gap"].default == 0
    assert by["gap"].flag == "--gap"  # derived from the parameter name
    assert by["fast"].kind == "bool"
    assert by["fast"].action == "store_true"
    assert by["label"].default == "x"  # plain literal default becomes an option
    assert by["label"].flag == "--label"


def test_typer_run_pattern_reads_the_function():
    spec = argspec.read_cli("import typer\n\ndef main(n: int = 3):\n    pass\n\ntyper.run(main)\n")
    assert spec is not None
    assert spec.fields[0].dest == "n"
    assert spec.fields[0].kind == "int"
    assert spec.fields[0].default == 3


def test_typer_bool_default_true_degrades_not_guesses():
    spec = argspec.read_cli(
        "import typer\n\ndef main(color: bool = True):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True  # --color/--no-color pairing can't be assembled faithfully
    assert f.action == ""


def test_typer_underscored_param_gets_kebab_flag():
    spec = argspec.read_cli(
        "import typer\n\ndef main(max_size: int = 1):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].flag == "--max-size"


def test_typer_two_commands_degrade_as_subcommands():
    spec = argspec.read_cli(
        "import typer\napp = typer.Typer()\n"
        "@app.command()\ndef a(x: int = 1): pass\n"
        "@app.command()\ndef b(y: int = 2): pass\n"
    )
    assert spec is not None
    assert spec.ok is False
    assert spec.reason == "subparsers"


def test_argparse_still_wins_when_present():
    text = (
        "import argparse\nimport click\n"
        "ap = argparse.ArgumentParser()\nap.add_argument('--x')\nap.parse_args()\n"
    )
    spec = argspec.read_cli(text)
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["x"]


def test_read_cli_none_for_plain_scripts():
    assert argspec.read_cli("print('hi')\n") is None
    assert argspec.read_cli("def broken(:\n") is None


# --------------------------------------------------------------------------
# mutation hardening: exact contracts for the click/typer readers
# --------------------------------------------------------------------------


def test_click_field_orders_increment_by_one():
    spec = argspec.read_cli(CLICK_SCRIPT)
    assert spec is not None
    assert [f.order for f in spec.fields] == [0, 1, 2, 3, 4]


def test_click_from_import_form_is_recognized():
    spec = argspec.read_cli(
        "from click import command, option\n@command()\n@option('--x', type=int)\ndef m(x): pass\n"
    )
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["x"]


def test_click_dotted_import_is_recognized():
    spec = argspec.read_cli(
        "import click.decorators\nimport click\n"
        "@click.command()\n@click.option('--x')\ndef m(x): pass\n"
    )
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["x"]


def test_click_secret_name_precheck_and_flag_default():
    spec = argspec.read_cli(
        "import click\n@click.command()\n"
        "@click.option('--api-key')\n@click.option('--fast', is_flag=True)\n"
        "def m(api_key, fast): pass\n"
    )
    assert spec is not None
    by = {f.dest: f for f in spec.fields}
    assert by["api_key"].secret is True
    assert by["fast"].secret is False
    assert by["fast"].default is False  # an is_flag option starts unchecked


def test_click_uppercase_type_constants():
    spec = argspec.read_cli(
        "import click\n@click.command()\n"
        "@click.option('--n', type=click.INT)\n"
        "@click.option('--r', type=click.FLOAT)\n"
        "@click.option('--s', type=click.STRING)\n"
        "def m(n, r, s): pass\n"
    )
    assert spec is not None
    # Bottom-up decorator order: STRING, FLOAT, INT.
    assert [f.kind for f in spec.fields] == ["str", "float", "int"]
    assert all(f.degraded is False for f in spec.fields)


def test_click_non_choice_call_type_degrades_even_with_list_arg():
    # A random callable taking a list is NOT a Choice — it must degrade, never be
    # mistaken for one (kills the and->or mutant in the Choice detection).
    spec = argspec.read_cli(
        "import click\n@click.command()\n"
        "@click.option('--c', type=Wrapper(['a', 'b']))\ndef m(c): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True
    assert spec.fields[0].choices == []


def test_click_non_literal_default_degrades():
    spec = argspec.read_cli(
        "import click\n@click.command()\n"
        "@click.option('--bg', default=(255, 255, 255))\ndef m(bg): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True


def test_click_multiple_option_flag():
    spec = argspec.read_cli(
        "import click\n@click.command()\n@click.option('--tag', multiple=True)\ndef m(tag): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].multiple is True


def test_click_short_flag_only_and_help():
    spec = argspec.read_cli(
        "import click\n@click.command()\n@click.option('-v', help='verbosity')\ndef m(v): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].flag == "-v"
    assert spec.fields[0].dest == "v"
    assert spec.fields[0].help == "verbosity"


def test_typer_from_import_form_is_recognized():
    spec = argspec.read_cli(
        "from typer import Typer\napp = Typer()\n@app.command()\ndef m(x: int = 1): pass\n"
    )
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["x"]


def test_typer_orders_match_signature_positions():
    spec = argspec.read_cli(TYPER_SCRIPT)
    assert spec is not None
    assert [f.order for f in spec.fields] == [0, 1, 2, 3, 4]


def test_typer_bare_positional_no_default():
    spec = argspec.read_cli("import typer\n\ndef main(n: int): pass\n\ntyper.run(main)\n")
    assert spec is not None
    f = spec.fields[0]
    assert f.flag == ""  # positional
    assert f.required is True
    assert f.kind == "int"
    assert f.degraded is False


def test_typer_unannotated_param_is_plain_text_not_degraded():
    spec = argspec.read_cli("import typer\n\ndef main(x=1): pass\n\ntyper.run(main)\n")
    assert spec is not None
    f = spec.fields[0]
    assert f.kind == "str"
    assert f.degraded is False
    assert f.default == 1


def test_typer_unmodelable_annotation_degrades():
    spec = argspec.read_cli("import typer\n\ndef main(xs: list = None): pass\n\ntyper.run(main)\n")
    assert spec is not None
    assert spec.fields[0].degraded is True


def test_typer_option_none_default_is_clean():
    spec = argspec.read_cli(
        "import typer\n\ndef main(x: str = typer.Option(None, '--x')): pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is False
    assert f.default is None
    assert f.required is False


def test_typer_secret_param_name_precheck():
    spec = argspec.read_cli(
        "import typer\n\ndef main(api_token: str = ''): pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].secret is True


def test_decorator_name_unnameable_callable_is_empty():
    import ast as _ast

    stmt = _ast.parse("(f())()").body[0]
    assert isinstance(stmt, _ast.Expr)
    assert argspec._decorator_name(stmt.value) == ""  # the callee is itself a Call


def test_click_is_flag_defaulting_on_degrades_not_guesses():
    spec = argspec.read_cli(
        "import click\n@click.command()\n"
        "@click.option('--color', is_flag=True, default=True)\ndef m(color): pass\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True  # --color/--no-color pairing can't be assembled faithfully
    assert f.action == ""


# --------------------------------------------------------------------------
# mutation tail: exact contracts for import guards, orders, dests, defaults
# --------------------------------------------------------------------------


def test_click_dotted_only_import_counts():
    # `import click.testing` (no plain `import click`) binds the click name at runtime;
    # the import guard must dot-split module paths on BOTH import forms.
    spec = argspec.read_cli(
        "import click.testing\n@click.command()\n@click.option('--x')\ndef m(x): pass\n"
    )
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["x"]


def test_click_from_dotted_module_counts():
    spec = argspec.read_cli(
        "from click.decorators import command, option\n@command()\n@option('--x')\ndef m(x): pass\n"
    )
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["x"]


def test_typer_dotted_only_import_counts():
    spec = argspec.read_cli(
        "import typer.main\n\ndef main(n: int = 1):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["n"]


def test_typer_from_dotted_module_counts():
    spec = argspec.read_cli(
        "from typer.main import Typer\napp = Typer()\n@app.command()\ndef m(x: int = 1): pass\n"
    )
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["x"]


def test_click_two_commands_without_group_degrade():
    spec = argspec.read_cli(
        "import click\n"
        "@click.command()\n@click.option('--x')\ndef a(x): pass\n"
        "@click.command()\n@click.option('--y')\ndef b(y): pass\n"
    )
    assert spec is not None
    assert spec.ok is False
    assert spec.reason == "subparsers"


def test_click_foreign_decorators_between_options_are_skipped_not_fatal():
    # A bare decorator AND a non-click call decorator sit between two options: the
    # reader must skip them and keep walking (a `break` would silently drop fields).
    spec = argspec.read_cli(
        "import click\nimport functools\n"
        "@click.command()\n"
        "@click.option('--first')\n"
        "@functools.cache\n"
        "@other.thing()\n"
        "@click.option('--second')\n"
        "def m(first, second): pass\n"
    )
    assert spec is not None
    assert sorted(f.dest for f in spec.fields) == ["first", "second"]


def test_click_non_literal_name_skips_that_call_only():
    spec = argspec.read_cli(
        "import click\n@click.command()\n"
        "@click.option(FLAG_CONST)\n@click.option('--real')\ndef m(real): pass\n"
    )
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["real"]


def test_click_partly_non_literal_names_skip_that_call_only():
    spec = argspec.read_cli(
        "import click\n@click.command()\n"
        "@click.option('-x', EXTRA)\n@click.option('--real')\ndef m(x, real): pass\n"
    )
    assert spec is not None
    assert [f.dest for f in spec.fields] == ["real"]


def test_click_short_first_declaration_still_prefers_long_flag():
    spec = argspec.read_cli(
        "import click\n@click.command()\n@click.option('-o', '--output')\ndef m(output): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].flag == "--output"
    assert spec.fields[0].dest == "output"


def test_click_dest_strips_dashes_not_letters():
    spec = argspec.read_cli(
        "import click\n@click.command()\n@click.option('--Xray')\ndef m(xray): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].dest == "Xray"


def test_click_default_none_is_clean():
    spec = argspec.read_cli(
        "import click\n@click.command()\n@click.option('--x', default=None)\ndef m(x): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is False
    assert spec.fields[0].default is None


def test_click_bare_float_and_str_types():
    spec = argspec.read_cli(
        "import click\n@click.command()\n"
        "@click.option('--r', type=float)\n@click.option('--s', type=str)\n"
        "def m(r, s): pass\n"
    )
    assert spec is not None
    # bottom-up: s first, then r
    assert [f.kind for f in spec.fields] == ["str", "float"]
    assert all(f.degraded is False for f in spec.fields)


def test_click_unknown_name_type_degrades():
    spec = argspec.read_cli(
        "import click\n@click.command()\n@click.option('--p', type=Path)\ndef m(p): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True  # click has no Path shortcut we can vouch for


def test_typer_option_extra_decl_positions():
    # The long flag may not be the FIRST declaration after the default.
    spec = argspec.read_cli(
        "import typer\n\ndef main(out: str = typer.Option('x', '-o', '--renamed')):\n"
        "    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].flag == "--renamed"
    assert spec.fields[0].default == "x"


def test_typer_non_constant_decl_is_ignored_not_fatal():
    spec = argspec.read_cli(
        "import typer\n\ndef main(out: str = typer.Option('x', SOME_DECL)):\n"
        "    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].flag == "--out"  # falls back to the derived flag
    assert spec.fields[0].default == "x"


def test_typer_computed_plain_default_degrades():
    spec = argspec.read_cli(
        "import typer\n\ndef main(x: int = make_default()):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True


def test_typer_option_computed_first_arg_degrades():
    spec = argspec.read_cli(
        "import typer\n\ndef main(x: str = typer.Option(CONST_REF)):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True


def test_typer_bool_true_degrade_renders_as_text():
    spec = argspec.read_cli(
        "import typer\n\ndef main(color: bool = True):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.kind == "str"  # the degrade path pins the free-text kind exactly


def test_typer_bool_false_flag_contract_exact():
    spec = argspec.read_cli(
        "import typer\n\ndef main(fast: bool = False):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.kind == "bool"
    assert f.action == "store_true"
    assert f.default is False
    assert f.degraded is False


def test_click_non_literal_choice_list_degrades():
    spec = argspec.read_cli(
        "import click\n@click.command()\n"
        "@click.option('--mode', type=click.Choice(MODES))\ndef m(mode): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True
    assert spec.fields[0].kind != "choice"


def test_typer_unmodelable_annotation_degrades_despite_literal_default():
    # The annotation-driven degrade must hold on its own — a clean literal default
    # (which does NOT degrade) must not mask it.
    spec = argspec.read_cli(
        "import typer\n\ndef main(mode: dict = 'x'):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True


def test_typer_option_single_extra_decl_is_read():
    # The declaration list starts right AFTER the default (args[1:], not args[2:]).
    spec = argspec.read_cli(
        "import typer\n\ndef main(out: str = typer.Option('x', '--renamed')):\n"
        "    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].flag == "--renamed"


# --------------------------------------------------------------------------
# typer Annotated[...] (A6) — the modern style AI-written typer scripts use
# --------------------------------------------------------------------------

ANNOTATED_SCRIPT = """
import typer
from typing import Annotated

app = typer.Typer()

@app.command()
def main(
    name: Annotated[str, typer.Argument(help="who")],
    count: Annotated[int, typer.Option(help="how many")] = 3,
    mode: Annotated[str, typer.Option("-m", "--mode")] = "fast",
    fast: Annotated[bool, typer.Option()] = False,
):
    pass
"""


def test_annotated_reads_type_and_metadata():
    spec = argspec.read_cli(ANNOTATED_SCRIPT)
    assert spec is not None
    assert spec.ok
    by = {f.dest: f for f in spec.fields}
    # Argument -> required positional, type from Annotated's first arg
    assert by["name"].flag == ""
    assert by["name"].required is True
    assert by["name"].kind == "str"
    assert by["name"].help == "who"
    # Option with a literal `= default`, type int
    assert by["count"].kind == "int"
    assert by["count"].default == 3
    assert by["count"].flag == "--count"
    assert by["count"].help == "how many"
    assert by["count"].degraded is False
    # Explicit flag declarations inside the Annotated Option
    assert by["mode"].flag == "--mode"
    assert by["mode"].default == "fast"
    # bool option defaulting False -> checkbox
    assert by["fast"].kind == "bool"
    assert by["fast"].action == "store_true"


def test_annotated_option_without_default_is_required():
    spec = argspec.read_cli(
        "import typer\nfrom typing import Annotated\n"
        "def main(x: Annotated[int, typer.Option()]):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.flag == "--x"  # still an option, not a positional
    assert f.required is True
    assert f.kind == "int"


def test_annotated_argument_with_default_is_optional_positional():
    spec = argspec.read_cli(
        "import typer\nfrom typing import Annotated\n"
        "def main(name: Annotated[str, typer.Argument()] = 'anon'):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.flag == ""
    assert f.required is False
    assert f.default == "anon"


def test_annotated_unmodelable_inner_type_degrades():
    spec = argspec.read_cli(
        "import typer\nfrom typing import Annotated\n"
        "def main(tags: Annotated[list, typer.Option()] = None):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True


def test_annotated_bool_default_true_degrades():
    spec = argspec.read_cli(
        "import typer\nfrom typing import Annotated\n"
        "def main(color: Annotated[bool, typer.Option()] = True):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.action == ""  # not a store_true — the --no-color pairing can't be assembled


def test_annotated_choice_via_typing_qualified_name():
    # `typing.Annotated` (attribute form) must be recognized too, not just a bare name.
    spec = argspec.read_cli(
        "import typer\nimport typing\n"
        "def main(gap: typing.Annotated[int, typer.Option()] = 0):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].kind == "int"
    assert spec.fields[0].default == 0


def test_annotated_help_kwarg_survives_on_degraded_field():
    spec = argspec.read_cli(
        "import typer\nfrom typing import Annotated\n"
        "def main(x: Annotated[MyType, typer.Option(help='hint')] = None):\n"
        "    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True
    assert spec.fields[0].help == "hint"  # the user's hint still reaches the form


def test_legacy_typer_style_still_works_after_annotated_refactor():
    # Regression: the non-Annotated `x: int = typer.Option(5, '--renamed')` path must be
    # untouched by the Annotated addition.
    spec = argspec.read_cli(TYPER_SCRIPT)
    assert spec is not None
    by = {f.dest: f for f in spec.fields}
    assert by["output"].required is True
    assert by["gap"].default == 0
    assert by["fast"].action == "store_true"


def test_annotated_only_recognizes_the_real_annotated_name():
    # A qualified subscript that is NOT typing.Annotated must not be unwrapped as one.
    spec = argspec.read_cli(
        "import typer\nimport mod\n"
        "def main(x: mod.Wrapper[int, typer.Option()] = 1):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True  # unknown subscript type -> free text, not int


def test_annotated_without_typer_metadata_reads_as_plain_type():
    # Annotated with only a doc string (no typer.Option/Argument): the field is still a
    # plain int with its signature default — the meta search must return None, not raise.
    spec = argspec.read_cli(
        "import typer\nfrom typing import Annotated\n"
        "def main(x: Annotated[int, 'a note'] = 5):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.kind == "int"
    assert f.default == 5
    assert f.degraded is False


def test_annotated_picks_the_typer_call_among_several():
    # Two calls in the metadata: the typer one must be chosen, not the first call.
    spec = argspec.read_cli(
        "import typer\nfrom typing import Annotated\n"
        "def main(x: Annotated[int, Validator(), typer.Option(help='H')] = 5):\n"
        "    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].help == "H"


def test_annotated_option_positional_decl_is_a_flag_not_a_default():
    # In the Annotated style, Option's positional strings are flag DECLARATIONS, never a
    # value default (that lives in the `= value`). A required Annotated Option with two
    # long decls keeps the first as the flag and stays default-less.
    spec = argspec.read_cli(
        "import typer\nfrom typing import Annotated\n"
        "def main(x: Annotated[str, typer.Option('--primary', '--secondary')]):\n"
        "    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.flag == "--primary"
    assert f.default is None  # not '--primary' (the True mutant would read it as a default)
    assert f.required is True
