"""Mutation-kill tests for skit.params.

Companion to test_params_model.py / test_params_edit.py: each test here pins an exact
literal or a control-flow edge that mutmut perturbs and the broader suites don't already
nail down — the bool-default spellings coerce_default accepts, the field defaults baked
into synthesized_placeholder and edit_declared's freshly-added rows, the defensive
delivery-coercion fallback, and the ``continue`` (not ``break``) semantics of the
add/tweak loops.
"""

from __future__ import annotations

import pytest

from skit.params import ParamDecl, coerce_default, edit_declared, synthesized_placeholder


def _by_name(decls: list[ParamDecl]) -> dict[str, ParamDecl]:
    return {d.name: d for d in decls}


# --------------------------------------------------------------- coerce_default bool spellings
# The true/false tuples each carry spellings the existing suite does not exercise ("1", "y",
# "no", "n"); a mutant that drops or upper-cases one of them makes coerce_default reject that
# exact word. (low is already lower-cased, so an upper-cased tuple entry can never match.)


@pytest.mark.parametrize(
    ("value", "expected"),
    [("1", True), ("y", True), ("no", False), ("n", False)],
)
def test_coerce_default_bool_extra_spellings(value: str, expected: bool) -> None:
    assert coerce_default(value, "bool") is expected


# --------------------------------------------------------------- synthesized_placeholder fields
# An undeclared template placeholder synthesizes to binding "none" / delivery "placeholder"
# / required. binding must be the exact literal "none" (a source-free hand-declared param).


def test_synthesized_placeholder_field_defaults() -> None:
    d = synthesized_placeholder("host")
    assert d.binding == "none"
    assert d.delivery == "placeholder"
    assert d.required is True


def test_synthesized_placeholder_secret_by_name() -> None:
    # C3 name heuristic still applies to synthesized placeholders.
    assert synthesized_placeholder("API_TOKEN").secret is True
    assert synthesized_placeholder("host").secret is False


# --------------------------------------------------------------- edit_declared: added-row defaults
# A bare `--add NAME` on a template whose {placeholders} include NAME creates a placeholder row
# with binding "none" and type "str" (and stays required so an empty slot can't assemble).


def test_add_placeholder_row_defaults() -> None:
    res = edit_declared(
        [], add=["size"], allowed_deliveries=("placeholder", "env"), placeholder_names=["size"]
    )
    d = res.decls[0]
    assert d.binding == "none"
    assert d.type == "str"
    assert d.delivery == "placeholder"
    assert d.required is True


def test_add_non_placeholder_row_delivery_falls_back_to_flag() -> None:
    # The added row's delivery is _coerce_literal(allowed_deliveries[0], _DELIVERIES, "flag").
    # When the caller-declared default delivery is not a real skit Delivery literal, the row
    # degrades to the "flag" fallback rather than to None / a bogus string.
    res = edit_declared([], add=["x"], allowed_deliveries=("sideband",))
    assert res.decls[0].delivery == "flag"


def test_add_non_placeholder_row_delivery_passes_valid_literal_through() -> None:
    # The mirror case: a VALID caller-allowed delivery passes through as-is (the fallback test
    # above can't see the first _coerce_literal argument, because an invalid value and a nulled
    # value both collapse to "flag"). A real delivery must survive the coercion unchanged.
    res = edit_declared([], add=["x"], allowed_deliveries=("env",))
    assert res.decls[0].delivery == "env"


# --------------------------------------------------------------- _apply_declared_tweaks fallback
# The delivery tweak coerces the caller-allowed value through _DELIVERIES with the decl's
# CURRENT delivery as the fallback: an allowed-but-unknown literal keeps the existing delivery
# rather than nulling it.


def test_delivery_tweak_unknown_literal_keeps_current_delivery() -> None:
    res = edit_declared(
        [ParamDecl(name="a", delivery="flag")],
        deliveries={"a": "sideband"},
        allowed_deliveries=("flag", "sideband"),
    )
    assert res.warnings == []
    assert _by_name(res.decls)["a"].delivery == "flag"


# --------------------------------------------------------------- loop control: continue vs break


def test_add_already_declared_continues_to_next_add() -> None:
    # The duplicate 'a' warns and continues; 'b' must still be added (a break would drop it).
    res = edit_declared([ParamDecl(name="a")], add=["a", "b"])
    assert res.warnings == ["already-declared:a"]
    assert "b" in _by_name(res.decls)


def test_tweak_on_unknown_name_continues_to_next_tweak() -> None:
    # 'ghost' is unknown -> not-declared + continue; the real 'a' tweak must still land.
    res = edit_declared(
        [ParamDecl(name="a", type="str")],
        types={"ghost": "int", "a": "int"},
    )
    assert "not-declared:ghost" in res.warnings
    assert _by_name(res.decls)["a"].type == "int"
