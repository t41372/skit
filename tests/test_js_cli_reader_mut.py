"""Mutation-kill pins for langs/javascript/cli_reader.py.

Targeted at the specific surviving mutants the broad reader suite in test_js_analyzer.py leaves
alive. Each test drives the real `util.parseArgs` reader over representative JS CLI source and
asserts an observable field of the resulting ArgSpec / ParamDecl — no mocks, no touching internals.
"""

from __future__ import annotations

from skit.langs.javascript import cli_reader


def read(src: str, *, lang: str = "js"):
    return cli_reader.read_cli(src, lang=lang)


# ---- _apply_option_spec loop: the two skip branches must CONTINUE, not BREAK ----------------


def test_option_spec_skips_computed_pair_then_keeps_reading_the_real_type():
    # A computed pair `[dyn]:1` sits BEFORE the real `type` inside the option spec. The reader must
    # skip just that pair (continue) and go on to read `type:"boolean"`. A `break` would stop at the
    # computed pair and never apply the type, leaving the field a plain str — kills the
    # key/value/computed guard's `continue`->`break`.
    (f,) = read('parseArgs({options:{flag:{[dyn]:1, type:"boolean"}}});\n').fields
    assert (f.type, f.action, f.default) == ("bool", "store_true", False)


def test_option_spec_skips_non_pair_then_keeps_reading_the_real_type():
    # A spread element `...rest` (not a pair) sits BEFORE the real `type`. The reader must skip it
    # (continue) and still read `type:"boolean"`. A `break` at the spread would leave the field a
    # plain str — kills the `if pair.type != "pair"` `continue`->`break`.
    (f,) = read('parseArgs({options:{flag:{...rest, type:"boolean"}}});\n').fields
    assert f.type == "bool"


# ---- _apply_type: the literal "string" must be matched exactly (else the field degrades) ------


def test_string_type_yields_a_clean_str_field_not_a_degraded_one():
    # `type:"string"` must produce a clean str field. Any mutation of the `text == "string"`
    # comparison falls through to `field.degraded = True` — the type stays str either way, so the
    # kill hinges on `degraded` being False.
    (f,) = read('parseArgs({options:{name:{type:"string"}}});\n').fields
    assert f.type == "str"
    assert f.degraded is False


# ---- _find_parseargs: BOTH match branches need `and`, not `or` --------------------------------


def test_identifier_call_that_is_not_parseargs_is_not_read_as_a_surface():
    # `foo(...)` is an identifier call whose name isn't parseArgs. The identifier branch is
    # `fn.type == "identifier" AND text == "parseArgs"`; an `or` there would match ANY identifier
    # call and mis-read its `{options:{...}}` as a parseArgs surface.
    assert read('foo({options:{n:{type:"string"}}});\n') is None


def test_member_call_that_is_not_parseargs_is_not_read_as_a_surface():
    # `console.log(...)` is a member call whose property isn't parseArgs. The member branch is
    # `prop is not None AND text == "parseArgs"`; an `or` there would match ANY member call.
    assert read('console.log({options:{n:{type:"string"}}});\n') is None


# ---- _property_name: the non-nameable fallback must be "" so a numeric key names no field ------


def test_numeric_option_key_names_no_field():
    # A numeric option key `0:{...}` is neither a property_identifier nor a string, so _property_name
    # must return "" and _read_option drops it (`if not name: return None`). A non-empty fallback
    # would mint a bogus field.
    assert read('parseArgs({options:{0:{type:"string"}}});\n').fields == []


# ---- _read_option ParamDecl: binding/delivery must land on their intended (default) values -----


def test_read_option_defaults_binding_none_delivery_flag():
    # A parseArgs option is a flag-delivery parameter with no source anchor: binding "none",
    # delivery "flag". Pins the values the (now default-omitted) kwargs used to carry and kills the
    # binding=/delivery= value-mutations in the stored-mutant tree.
    (f,) = read('parseArgs({options:{name:{type:"string"}}});\n').fields
    assert f.binding == "none"
    assert f.delivery == "flag"
    assert f.flag == "--name"
