"""Mutation-kill tests for langs/shell/normalize.py — the consent-gated A5 `--normalize`
rewriter (`NAME=value` → `NAME="${NAME:-value}"`). Each test pins a real, observable behaviour
of `normalize_idiom` through its public surface."""

from __future__ import annotations

from skit.langs.shell import normalize


def test_plusassign_before_a_const_does_not_abort_the_scan():
    # `_by_name` walks every top-level assignment; a `+=` accumulator is not a plain-`=` const, so
    # it must be SKIPPED (continue) and the scan must go on. A `break` there would stop at the first
    # `+=` and never register a genuine const defined after it. With `COUNT+=1` sitting before
    # `CITY=Taipei`, CITY must still normalize to the env-default idiom.
    src = "COUNT+=1\nCITY=Taipei\n"
    result = normalize.normalize_idiom(src, ["CITY"])
    assert result.normalized == ["CITY"]
    assert result.refused == []
    assert result.text == 'COUNT+=1\nCITY="${CITY:-Taipei}"\n'


def test_subscript_target_is_not_grouped_as_a_normalizable_const():
    # `X[0]=hello` is a `variable_assignment` whose name node is a `subscript`, not a
    # `variable_name`. The `name_node is None or type != "variable_name"` guard must SKIP it — an
    # array element is not a const skit can re-home in `${NAME:-…}`. Flipping that `or` to `and`
    # would let the subscript through (grouped under the key "X[0]") and `--normalize X[0]` would
    # then wrongly rewrite the element instead of refusing it as not-a-const.
    src = "X[0]=hello\n"
    result = normalize.normalize_idiom(src, ["X[0]"])
    assert result.normalized == []
    assert result.refused == ["not-a-const:X[0]"]
    assert result.text == src  # a refusal never touches a single byte
