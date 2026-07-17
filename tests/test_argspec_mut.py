"""Mutation-hardening for langs/python/argspec.py: the binding/delivery axes of every
reflected CLI field.

A reflected argparse/click/typer field is, by the unified-form contract (see the ArgSpec
docstring and params.ParamDecl), a ``binding="none"`` / ``delivery="flag"`` ParamDecl: the
script owns its own parser, so skit anchors nothing in the source (binding "none") and
reaches the program by assembling real argv (delivery "flag"). These are internal contract
literals, not i18n copy, so they are asserted verbatim. Each reader sets the pair exactly
once, so one real read per language pins every value-mutation of those two literals.
"""

from __future__ import annotations

from skit.langs.python import argspec


def test_argparse_field_binding_is_none_and_delivery_is_flag():
    # A plain argparse surface: one option, one positional. Both come back as
    # binding="none" (skit reflects, never source-anchors) / delivery="flag" (real argv).
    spec = argspec.read_argparse(
        "import argparse\n"
        "ap = argparse.ArgumentParser()\n"
        "ap.add_argument('--output')\n"
        "ap.add_argument('count', type=int)\n"
    )
    assert spec is not None
    assert [f.name for f in spec.fields] == ["output", "count"]
    for f in spec.fields:
        assert f.binding == "none"
        assert f.delivery == "flag"


def test_click_field_binding_is_none_and_delivery_is_flag():
    # Both an @click.option and an @click.argument flow through _read_click_param; each
    # field carries the same reflected binding/delivery pair.
    spec = argspec.read_cli(
        "import click\n"
        "@click.command()\n"
        "@click.option('--output')\n"
        "@click.argument('name')\n"
        "def m(output, name): pass\n"
    )
    assert spec is not None
    assert sorted(f.name for f in spec.fields) == ["name", "output"]
    for f in spec.fields:
        assert f.binding == "none"
        assert f.delivery == "flag"


def test_typer_field_binding_is_none_and_delivery_is_flag():
    # A typer command read via typer.run(main): a required positional and an optional
    # value parameter, both reflected as binding="none" / delivery="flag".
    spec = argspec.read_cli(
        "import typer\n\ndef main(name: str, count: int = 3):\n    pass\n\ntyper.run(main)\n"
    )
    assert spec is not None
    assert [f.name for f in spec.fields] == ["name", "count"]
    for f in spec.fields:
        assert f.binding == "none"
        assert f.delivery == "flag"
