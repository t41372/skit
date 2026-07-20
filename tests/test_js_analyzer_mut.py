"""Mutation-kill pins for src/skit/langs/javascript/analyzer.py.

Each test targets a specific surviving mutant by exercising a real analyzer code path whose
observable output (candidate list, demotion marker, external-imports list) differs between the
original and the mutated source. Companion to tests/test_js_analyzer.py; no behaviour is mocked.
"""

from __future__ import annotations

from skit.langs.javascript import analyzer as js


def names(src: str, *, lang: str = "js") -> list[str]:
    return [c.name for c in js.analyze(src, lang=lang).candidates]


# ---------------------------------------------------------------- _const_candidates guard


def test_destructuring_with_literal_value_is_not_a_candidate():
    # The name-guard is `name_node is None OR type != identifier OR value is None`. An object
    # (or array) destructuring pattern bound to a *literal* — `const {p} = 5;` — has a non-identifier
    # name_node and a real literal value, so it must be rejected by the `type != identifier` arm.
    # (Mutating the first `or` to `and` would let the pattern text "{p}" leak through as a candidate.)
    assert js.analyze("const {p} = 5;\n").candidates == []
    assert names("const [x] = 5;\n") == []
    # A destructuring statement before a real const must not swallow the const either.
    assert names("const {p} = 5;\nconst KEEP = 7;\n") == ["KEEP"]


def test_non_literal_const_is_skipped_but_later_literals_still_land():
    # A non-literal value (`foo()`) makes `_literal_value` return None: that ONE declarator is
    # skipped (`continue`), the walk goes on, and the following literal const is still collected.
    # A `break` here would abandon every later declaration.
    assert names("const A = foo();\nconst B = 5;\n") == ["B"]
    assert names("const A = foo();\nconst B = 5;\nconst C = 9;\n") == ["B", "C"]


# ---------------------------------------------------------------- reassigned-const demotion


def test_reassigned_const_carries_the_accumulator_demotion_marker():
    # A const reassigned at top level (`C = 2`) is demoted in analyze()'s post-pass. The marker is
    # the exact symbolic id "accumulator" — not None, not a cased/renamed variant — because the UI
    # maps that id to human wording. Pins analyze()'s `c.demotion = "accumulator"` assignment.
    (c,) = js.analyze("const C = 1;\nC = 2;\n").candidates
    assert c.demoted is True
    assert c.demotion == "accumulator"


def test_augmented_reassigned_const_demotion_marker():
    (c,) = js.analyze("const N = 0;\nN += 5;\n").candidates
    assert (c.demoted, c.demotion) == (True, "accumulator")


# ---------------------------------------------------------------- external_imports / _import_source


def test_external_imports_skips_sourceless_export_statements():
    # `export const X = 5;` is an export_statement with NO `source` field. _import_source must read
    # that as "imports nothing" (None) rather than dereferencing the absent source node. A real
    # import alongside it is still reported, and the sourceless export is silently ignored.
    assert js.external_imports("import chalk from 'chalk';\nexport const X = 5;\n") == ["chalk"]
    assert js.external_imports("export const X = 5;\n") == []
