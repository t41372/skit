"""Pure edit ops on the declared [[parameters]] schema (params.edit_declared) + the
default-coercion / type-validation helpers. Every op and every closed warning code is
exercised here; the CLI/TUI wiring on top is covered in test_declared_params.py and
test_tui_settings_cov.py."""

from __future__ import annotations

import math
import re

import pytest

from skit import params
from skit.params import ParamDecl, coerce_default, edit_declared


def _by_name(decls: list[ParamDecl]) -> dict[str, ParamDecl]:
    return {d.name: d for d in decls}


# --------------------------------------------------------------------------- add / rm


def test_add_defaults_to_first_allowed_delivery_for_a_binary():
    res = edit_declared([], add=["width"], allowed_deliveries=("flag", "env"))
    assert res.warnings == []
    d = res.decls[0]
    assert (d.name, d.delivery, d.binding, d.type, d.required) == (
        "width",
        "flag",
        "none",
        "str",
        False,
    )


def test_add_on_a_template_placeholder_name_becomes_a_required_placeholder():
    res = edit_declared(
        [], add=["size"], allowed_deliveries=("placeholder", "env"), placeholder_names=["size"]
    )
    d = res.decls[0]
    assert (d.delivery, d.required) == ("placeholder", True)


def test_add_non_placeholder_name_on_a_template_uses_first_allowed_delivery():
    res = edit_declared(
        [], add=["RETRIES"], allowed_deliveries=("placeholder", "env"), placeholder_names=["size"]
    )
    d = res.decls[0]
    assert d.delivery == "placeholder"  # allowed_deliveries[0]; only a matching name overrides
    assert d.required is False


def test_add_existing_name_warns_already_declared():
    res = edit_declared([ParamDecl(name="a")], add=["a"])
    assert res.warnings == ["already-declared:a"]
    assert [d.name for d in res.decls] == ["a"]


def test_rm_drops_the_row():
    res = edit_declared([ParamDecl(name="a"), ParamDecl(name="b")], rm=["a"])
    assert [d.name for d in res.decls] == ["b"]


def test_rm_unknown_name_warns_not_declared():
    res = edit_declared([ParamDecl(name="a")], rm=["ghost"])
    assert res.warnings == ["not-declared:ghost"]
    assert [d.name for d in res.decls] == ["a"]


def test_apply_order_is_rm_then_add_then_tweak():
    # rm a, add a fresh, then tweak the fresh one — the tweak must land on the re-added row.
    res = edit_declared(
        [ParamDecl(name="a", type="int")],
        rm=["a"],
        add=["a"],
        types={"a": "float"},
    )
    d = _by_name(res.decls)["a"]
    assert d.type == "float"


def test_inputs_are_never_mutated():
    original = ParamDecl(name="a", type="str", prompt="orig")
    edit_declared([original], prompts={"a": "changed"}, secret=["a"])
    assert original.prompt == "orig"
    assert original.secret is False


# --------------------------------------------------------------------------- tweaks


def test_delivery_tweak_within_allowed_set():
    res = edit_declared(
        [ParamDecl(name="a", delivery="flag")],
        deliveries={"a": "env"},
        allowed_deliveries=("flag", "env"),
    )
    assert _by_name(res.decls)["a"].delivery == "env"


def test_delivery_outside_allowed_set_warns_bad_delivery():
    res = edit_declared(
        [ParamDecl(name="a", delivery="flag")],
        deliveries={"a": "placeholder"},
        allowed_deliveries=("flag", "env"),
    )
    assert res.warnings == ["bad-delivery:a"]
    assert _by_name(res.decls)["a"].delivery == "flag"  # unchanged


def test_placeholder_delivery_on_a_non_placeholder_name_warns():
    res = edit_declared(
        [ParamDecl(name="a", delivery="env")],
        deliveries={"a": "placeholder"},
        allowed_deliveries=("placeholder", "env"),
        placeholder_names=["other"],
    )
    assert res.warnings == ["not-a-placeholder:a"]
    assert _by_name(res.decls)["a"].delivery == "env"


def test_placeholder_delivery_on_a_matching_placeholder_name_is_allowed():
    res = edit_declared(
        [ParamDecl(name="size", delivery="env")],
        deliveries={"size": "placeholder"},
        allowed_deliveries=("placeholder", "env"),
        placeholder_names=["size"],
    )
    assert _by_name(res.decls)["size"].delivery == "placeholder"


def test_type_tweak_valid():
    res = edit_declared([ParamDecl(name="a")], types={"a": "int"})
    assert _by_name(res.decls)["a"].type == "int"


def test_type_tweak_invalid_warns_bad_type():
    res = edit_declared([ParamDecl(name="a", type="str")], types={"a": "integer"})
    assert res.warnings == ["bad-type:a"]
    assert _by_name(res.decls)["a"].type == "str"


def test_choices_tweak_sets_the_tuple():
    res = edit_declared([ParamDecl(name="a", type="choice")], choices={"a": ["x", "y"]})
    assert _by_name(res.decls)["a"].choices == ("x", "y")


def test_default_coerced_to_the_declared_type():
    res = edit_declared([ParamDecl(name="a", type="int")], defaults={"a": "42"})
    d = _by_name(res.decls)["a"]
    assert d.default == 42
    assert isinstance(d.default, int)


def test_default_type_set_in_same_call_applies_before_coercion():
    res = edit_declared([ParamDecl(name="a")], types={"a": "float"}, defaults={"a": "1.5"})
    d = _by_name(res.decls)["a"]
    assert d.default == 1.5
    assert isinstance(d.default, float)


def test_default_bad_value_warns_bad_default_and_keeps_old():
    res = edit_declared([ParamDecl(name="a", type="int", default=3)], defaults={"a": "notanint"})
    assert res.warnings == ["bad-default:a"]
    assert _by_name(res.decls)["a"].default == 3


def test_flag_tweak_strips_and_sets_empty_for_positional():
    res = edit_declared([ParamDecl(name="a", delivery="flag")], flags={"a": "  --out "})
    assert _by_name(res.decls)["a"].flag == "--out"
    res2 = edit_declared([ParamDecl(name="a", delivery="flag", flag="--out")], flags={"a": ""})
    assert _by_name(res2.decls)["a"].flag == ""  # empty ⇒ positional


def test_required_and_optional_tweaks():
    res = edit_declared([ParamDecl(name="a")], required=["a"])
    assert _by_name(res.decls)["a"].required is True
    res2 = edit_declared([ParamDecl(name="a", required=True)], optional=["a"])
    assert _by_name(res2.decls)["a"].required is False


def test_help_text_and_prompt_tweaks():
    res = edit_declared(
        [ParamDecl(name="a")], help_texts={"a": "what it does"}, prompts={"a": "A?"}
    )
    d = _by_name(res.decls)["a"]
    assert d.help == "what it does"
    assert d.prompt == "A?"


def test_secret_and_env_source_together():
    res = edit_declared([ParamDecl(name="tok")], secret=["tok"], env_sources={"tok": " API_TOKEN "})
    d = _by_name(res.decls)["tok"]
    assert d.secret is True
    assert d.env_source == "API_TOKEN"  # stripped


def test_env_source_on_a_non_secret_param_is_ignored():
    res = edit_declared([ParamDecl(name="a", secret=False)], env_sources={"a": "VAR"})
    assert _by_name(res.decls)["a"].env_source == ""  # only means anything on a secret param


def test_no_secret_clears_the_env_source():
    res = edit_declared(
        [ParamDecl(name="tok", secret=True, env_source="API_TOKEN")], no_secret=["tok"]
    )
    d = _by_name(res.decls)["tok"]
    assert d.secret is False
    assert d.env_source == ""


def test_tweak_on_unknown_name_warns_not_declared():
    res = edit_declared([ParamDecl(name="a")], types={"ghost": "int"})
    assert res.warnings == ["not-declared:ghost"]


def test_a_name_touched_by_two_ops_is_listed_once_and_both_apply():
    # exercises the dedup in the tweak-name gather (dict + dict, and seq overlapping a dict)
    res = edit_declared(
        [ParamDecl(name="a")],
        types={"a": "int"},
        defaults={"a": "5"},
        secret=["a"],
        prompts={"a": "A?"},
    )
    d = _by_name(res.decls)["a"]
    assert (d.type, d.default, d.secret, d.prompt) == ("int", 5, True, "A?")


# --------------------------------------------------------------------------- revert on invalid


def test_choice_type_without_choices_reverts_and_warns():
    res = edit_declared(
        [ParamDecl(name="a", type="str", help="keep me")],
        types={"a": "choice"},
        help_texts={"a": "changed"},
    )
    assert res.warnings == ["choice-without-choices:a"]
    d = _by_name(res.decls)["a"]
    assert d.type == "str"  # reverted to pre-tweak state
    assert d.help == "keep me"  # the whole row reverted, so the help edit is dropped too


def test_choice_type_with_choices_in_the_same_call_is_valid():
    res = edit_declared([ParamDecl(name="a")], types={"a": "choice"}, choices={"a": ["r", "g"]})
    assert res.warnings == []
    d = _by_name(res.decls)["a"]
    assert d.type == "choice"
    assert d.choices == ("r", "g")


# --------------------------------------------------------------------------- coerce_default


@pytest.mark.parametrize(
    ("value", "type_name", "expected"),
    [
        ("42", "int", 42),
        ("3.5", "float", 3.5),
        ("true", "bool", True),
        ("YES", "bool", True),
        ("on", "bool", True),
        ("false", "bool", False),
        ("0", "bool", False),
        ("off", "bool", False),
        ("anything", "str", "anything"),
        ("anything", "choice", "anything"),
    ],
)
def test_coerce_default_success(value, type_name, expected):
    assert coerce_default(value, type_name) == expected


@pytest.mark.parametrize(
    ("value", "type_name"),
    [("x", "int"), ("x", "float"), ("maybe", "bool"), ("inf", "float"), ("nan", "float")],
)
def test_coerce_default_rejects_bad_values(value, type_name):
    with pytest.raises(ValueError, match=re.escape(value)):
        coerce_default(value, type_name)


def test_coerce_default_rejects_infinity_specifically():
    assert math.isinf(float("inf"))
    with pytest.raises(ValueError, match="1e999"):
        coerce_default("1e999", "float")


# --------------------------------------------------------------------------- as_param_type


@pytest.mark.parametrize("value", ["str", "int", "float", "bool", "choice"])
def test_as_param_type_accepts_the_five(value):
    assert params.as_param_type(value) == value


@pytest.mark.parametrize("value", ["integer", "", "STR", "number"])
def test_as_param_type_rejects_others(value):
    assert params.as_param_type(value) is None
