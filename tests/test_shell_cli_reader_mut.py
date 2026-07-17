"""Mutation pins for the shell getopts cli_reader.

Companion to test_shell_getopts.py, which covers the broad optstring matrix. Each test here nails
one behaviour that matrix left un-pinned: the command scan continues past earlier, non-getopts
commands; the value marker `:` at the very end of the optstring still makes a str flag; a repeated
letter emits exactly one field in the LIST (a name-keyed dict would hide a duplicate); and the flag
axes `_option` stamps on every field (binding / delivery / secret / flag).
"""

from __future__ import annotations

from skit.langs.shell import cli_reader as sc


def fields(src):
    spec = sc.read_cli(src)
    assert spec is not None
    return {f.name: f for f in spec.fields}


def test_getopts_found_after_an_earlier_non_getopts_command():
    # The command scan must keep walking (`continue`) past commands that aren't getopts — a real
    # script does setup (here `echo`) before its option loop. A `break` at the first non-getopts
    # command would abandon the scan and report no readable surface.
    fs = fields('echo starting\nwhile getopts "a:v" opt; do :; done\n')
    assert (fs["a"].type, fs["v"].type) == ("str", "bool")


def test_trailing_value_marker_makes_a_str_flag():
    # The `:` that marks a value flag can sit at the very last index of the optstring; the boundary
    # test is `i + 1 < n` (peek the next char), not `i + 2 < n`. With the tighter bound the final
    # `n:` would be misread as a bare boolean.
    fs = fields('while getopts "vn:" opt; do :; done\n')
    assert fs["n"].type == "str"  # value flag, `:` is the last char
    assert fs["v"].type == "bool"


def test_repeated_letter_emits_exactly_one_field():
    # Dedup keys on the letter itself (`seen.add(ch)`), so a repeated option letter yields ONE
    # field. Asserting on the raw fields LIST (not a name-keyed dict) is what catches a duplicate:
    # if dedup tracked a constant instead, the second `v` would slip through as a second field.
    spec = sc.read_cli('while getopts "vv" opt; do :; done\n')
    assert spec is not None
    assert [f.name for f in spec.fields] == ["v"]


def test_option_binding_and_delivery_and_flag():
    # Every getopts field is an un-anchored, single-dash flag: binding "none" (no source anchor),
    # delivery "flag", flag "-<letter>". binding/delivery are the ParamDecl defaults, but the
    # reader depends on them staying that way — pin the assembled shape directly.
    d = sc._option("x", False)
    assert (d.binding, d.delivery, d.flag) == ("none", "flag", "-x")


def test_option_carries_secret_from_the_name():
    # `_option` stamps `secret` from the name via is_secret_name. Real getopts letters are single
    # chars (never secret), but the helper honours a secret-looking name — pin that wiring so a
    # field whose secret flag is hardcoded rather than derived is caught.
    assert sc._option("KEY", True).secret is True
    assert sc._option("a", False).secret is False


def test_bool_flag_shape_from_a_bare_letter():
    # A bare letter (no trailing `:`) is a store_true boolean defaulting to False; a `letter:` is a
    # str value flag. Pins the two branches of `_option`'s takes_value split end-to-end.
    fs = fields('while getopts "n:v" opt; do :; done\n')
    assert (fs["v"].type, fs["v"].action, fs["v"].default) == ("bool", "store_true", False)
    assert (fs["n"].type, fs["n"].action) == ("str", "")
