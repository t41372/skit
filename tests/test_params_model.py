"""ParamDecl: the universal parameter model (contract tests).

The block shape is a FROZEN on-disk contract (existing user files carry it), so the
round-trip tests here pin exact dict shapes, not just semantic equality.
"""

from __future__ import annotations

from skit.params import ParamDecl, field_replace, normalize, validate_invariants

# ---- block (in-file [tool.skit]) shape: frozen -------------------------------------------------


def test_block_dict_const_shape_is_frozen():
    d = ParamDecl(
        name="API_KEY", binding="const", delivery="inject", type="str", default="xxx", secret=True
    )
    assert d.to_block_dict() == {
        "name": "API_KEY",
        "kind": "const",  # the FROZEN key/value: existing user files carry exactly this
        "type": "str",
        "default": "xxx",
        "secret": True,
    }


def test_block_dict_input_shape_is_frozen():
    d = ParamDecl(
        name="input-1",
        binding="input",
        delivery="inject",
        prompt="Name: ",
        order=0,
        env_source="MY_NAME",
    )
    assert d.to_block_dict() == {
        "name": "input-1",
        "kind": "input",
        "type": "str",
        "prompt": "Name: ",
        "order": 0,
        "env_source": "MY_NAME",
    }


def test_block_roundtrip_derives_delivery_from_binding():
    src = ParamDecl(name="N", binding="const", delivery="inject", type="int", default=3)
    back = ParamDecl.from_block_dict(src.to_block_dict())
    assert back == src
    envd = ParamDecl.from_block_dict({"name": "V", "kind": "envdefault", "default": "x"})
    assert envd.binding == "envdefault"
    assert envd.delivery == "env"  # implied, never stored in the block


def test_from_block_dict_is_total_on_garbage():
    d = ParamDecl.from_block_dict(
        {"name": 5, "kind": "martian", "type": [], "order": "NaN", "default": {"t": 1}}
    )
    assert d.name == "5"
    assert d.binding == "const"  # unknown binding degrades to the historical default
    assert d.type == "str"
    assert d.order == -1
    assert d.default is None  # a table is not an injectable scalar


# ---- from a source candidate --------------------------------------------------------------------


def test_from_candidate_maps_fields_and_derives_delivery():
    from skit.analysis import Candidate

    const = ParamDecl.from_candidate(
        Candidate(binding="const", name="CITY", type="str", default="Taipei", secret=True)
    )
    assert const.name == "CITY"
    assert const.binding == "const"
    # delivery is derived from the binding (a Candidate has no delivery of its own).
    assert const.delivery == "inject"
    assert (const.type, const.default, const.secret) == ("str", "Taipei", True)
    assert (const.order, const.prompt) == (-1, "")

    inp = ParamDecl.from_candidate(
        Candidate(binding="input", name="input-1", prompt="Name: ", order=0)
    )
    assert (inp.binding, inp.delivery) == ("input", "inject")
    assert (inp.prompt, inp.order) == ("Name: ", 0)


# ---- meta [[parameters]] shape ------------------------------------------------------------------


def test_meta_roundtrip_full_model():
    src = ParamDecl(
        name="width",
        binding="none",
        delivery="flag",
        type="choice",
        default="800",
        required=True,
        multiple=True,
        choices=("400", "800"),
        prompt="Width",
        help="output width",
        secret=False,
        flag="--width",
        action="",
        env_target="",
    )
    back = ParamDecl.from_meta_dict(src.to_meta_dict())
    assert back == src


def test_meta_dict_omits_defaults():
    d = ParamDecl(name="x").to_meta_dict()
    assert d == {"name": "x", "delivery": "flag", "type": "str"}


def test_meta_dict_omits_repeat_when_false():
    # repeat rides the truthiness-gated tail: at its False default it is absent entirely,
    # never serialized as `repeat = false` (additive-only forward contract).
    assert "repeat" not in ParamDecl(name="x", delivery="flag").to_meta_dict()


def test_meta_dict_repeat_emitted_and_roundtrips_only_when_set():
    src = ParamDecl(name="tag", delivery="flag", flag="--tag", multiple=True, repeat=True)
    d = src.to_meta_dict()
    assert d["repeat"] is True  # emitted only because it is truthy
    back = ParamDecl.from_meta_dict(d)
    assert back == src
    assert back.repeat is True


def test_from_meta_dict_repeat_defaults_false_when_absent():
    assert ParamDecl.from_meta_dict({"name": "x", "delivery": "flag"}).repeat is False


def test_from_meta_dict_repeat_coerces_truthy_to_bool():
    # A hand-edited meta.toml may carry a non-bool truthy scalar; from_meta_dict normalizes it to
    # a real bool (kills the bool()-wrapper drop mutant, which would leave repeat as the raw int).
    assert ParamDecl.from_meta_dict({"name": "x", "delivery": "flag", "repeat": 1}).repeat is True


def test_meta_dict_includes_binding_and_order_when_set():
    # The two truthiness-gated head fields of to_meta_dict: a source-anchored binding and a
    # call-order key are emitted only when present, and round-trip back unchanged.
    src = ParamDecl(name="input-1", binding="input", delivery="inject", order=2)
    d = src.to_meta_dict()
    assert d["binding"] == "input"
    assert d["order"] == 2
    assert ParamDecl.from_meta_dict(d) == src


def test_meta_roundtrip_env_delivery_and_target():
    src = ParamDecl(name="width", delivery="env", env_target="WIDTH_PX", secret=True)
    back = ParamDecl.from_meta_dict(src.to_meta_dict())
    assert back == src
    assert back.env_var == "WIDTH_PX"


def test_from_meta_dict_is_total_on_garbage():
    d = ParamDecl.from_meta_dict(
        {"name": "x", "delivery": "carrier-pigeon", "choices": "abc", "order": None}
    )
    assert d.delivery == "flag"
    assert d.choices == ()
    assert d.order == -1


# ---- env_var / invariants / normalize -----------------------------------------------------------


def test_env_var_defaults_to_name():
    assert ParamDecl(name="WIDTH", delivery="env").env_var == "WIDTH"
    assert ParamDecl(name="w", delivery="env", env_target="WIDTH").env_var == "WIDTH"


def test_invariants_binding_implies_delivery():
    ok = ParamDecl(name="a", binding="const", delivery="inject")
    assert validate_invariants(ok) is None
    bad = ParamDecl(name="a", binding="const", delivery="env")
    assert validate_invariants(bad) == "binding-delivery-mismatch"
    envd = ParamDecl(name="a", binding="envdefault", delivery="flag")
    assert validate_invariants(envd) == "binding-delivery-mismatch"
    free = ParamDecl(name="a", binding="none", delivery="env")
    assert validate_invariants(free) is None


def test_invariants_choice_needs_choices():
    assert validate_invariants(ParamDecl(name="a", type="choice")) == "choice-without-choices"
    assert validate_invariants(ParamDecl(name="a", type="choice", choices=("x",))) is None


def test_normalize_repairs_delivery_from_binding():
    bad = ParamDecl(name="a", binding="envdefault", delivery="flag")
    fixed = normalize(bad)
    assert fixed.delivery == "env"
    ok = ParamDecl(name="b", binding="none", delivery="env")
    assert normalize(ok) is ok  # nothing to repair: same object back


def test_field_replace_returns_modified_copy():
    a = ParamDecl(name="a", type="int")
    b = field_replace(a, type="float")
    assert b.type == "float"
    assert a.type == "int"
