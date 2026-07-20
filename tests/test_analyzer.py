"""Analyzer (candidate parameter detection): const, main-guard (C4), input ordering (B1),
framework detection, secret heuristics (C3 pre-stage)."""

from __future__ import annotations

from skit import callmatch
from skit.langs.python import analyzer


def test_module_level_consts():
    src = (
        "CITY = 'Taipei'\n"
        "RETRIES = 3\n"
        "THRESHOLD = -0.5\n"
        "VERBOSE = True\n"
        "_INTERNAL = 'skip me'\n"
        "derived = RETRIES * 2\n"  # non-literal, not a candidate
    )
    result = analyzer.analyze(src)
    names = {c.name: c for c in result.candidates}
    assert set(names) == {"CITY", "RETRIES", "THRESHOLD", "VERBOSE"}
    assert names["CITY"].type == "str"
    assert names["CITY"].default == "Taipei"
    assert names["RETRIES"].type == "int"
    assert names["RETRIES"].default == 3
    assert names["THRESHOLD"].type == "float"
    assert names["THRESHOLD"].default == -0.5
    assert names["VERBOSE"].type == "bool"
    assert names["VERBOSE"].default is True


def test_ann_assign_and_bool_not_int():
    src = "count: int = 10\nflag: bool = False\n"
    result = analyzer.analyze(src)
    types = {c.name: c.type for c in result.candidates}
    assert types == {"count": "int", "flag": "bool"}


def test_main_guard_scanned_c4():
    src = (
        "import sys\n"
        "TOP = 1\n"
        'if __name__ == "__main__":\n'
        "    GUARD_CONST = 'hello'\n"
        "    TOP = 99\n"  # same name: module-level wins, no duplicate
        "    print(GUARD_CONST)\n"
    )
    result = analyzer.analyze(src)
    names = [c.name for c in result.candidates]
    assert names.count("TOP") == 1
    assert "GUARD_CONST" in names


def test_main_guard_reversed_form():
    src = 'if "__main__" == __name__:\n    X = 5\n'
    result = analyzer.analyze(src)
    assert [c.name for c in result.candidates] == ["X"]


def test_input_calls_ordered_b1():
    src = 'name = input("Name: ")\ndef f():\n    return input("Inner: ")\nage = input()\n'
    result = analyzer.analyze(src)
    inputs = [c for c in result.candidates if c.binding == "input"]
    assert [c.order for c in inputs] == [0, 1, 2]
    assert inputs[0].prompt == "Name: "
    assert inputs[1].prompt == "Inner: "
    assert inputs[2].prompt == ""
    assert inputs[0].name == "input-1"


def test_secret_heuristics():
    src = 'API_KEY = "x"\ntoken = "y"\npw = input("Password: ")\nCITY = "z"\n'
    result = analyzer.analyze(src)
    by_name = {c.name: c for c in result.candidates}
    assert by_name["API_KEY"].secret is True
    assert by_name["token"].secret is True
    assert by_name["CITY"].secret is False
    assert by_name["input-1"].secret is True  # prompt contains "Password"


def test_framework_detection():
    assert analyzer.analyze("import argparse\n").frameworks == ["argparse"]
    assert analyzer.analyze("from click import command\n").frameworks == ["click"]
    assert analyzer.analyze("import typer\nimport click\n").frameworks == ["typer", "click"]
    assert analyzer.analyze("import os\n").uses_cli_framework is False


def test_syntax_error_returns_empty():
    result = analyzer.analyze("def broken(:\n")
    assert result.syntax_error is True
    assert result.candidates == []


# ---------- duplicate top-level const names (corrupted/wrong injected run) ----------


def test_duplicate_top_level_const_is_deduped_to_one_candidate():
    # A name bound twice at module top level (e.g. from hand-editing) must yield exactly one
    # candidate, not two: two same-named ParamDecls made the shim compute and apply the same
    # replacement span twice (see shim.inject), corrupting the injected source.
    src = "CITY = 'a'\nCITY = 'b'\nprint(CITY)\n"
    result = analyzer.analyze(src)
    names = [c.name for c in result.candidates]
    assert names.count("CITY") == 1


def test_duplicate_top_level_const_keeps_last_occurrence_value():
    # Module top-level execution is sequential, so by the time the script finishes running, CITY
    # holds 'b' (the second assignment), not 'a'. The kept candidate's type/default must reflect
    # that runtime-effective value, or the onboarding form default and the injected type would
    # disagree with what the script actually does when left unmanaged.
    src = "N = 1\nN = 2\nprint(N)\n"
    result = analyzer.analyze(src)
    (cand,) = [c for c in result.candidates if c.name == "N"]
    assert cand.default == 2
    assert cand.type == "int"


def test_duplicate_top_level_const_keeps_first_occurrence_position():
    # Display/onboarding order should still read top-to-bottom like the source: the de-duplicated
    # candidate keeps the *first* occurrence's slot even though its value comes from the last one.
    src = "X = 1\nY = 5\nX = 2\n"
    result = analyzer.analyze(src)
    names = [c.name for c in result.candidates]
    assert names.index("X") < names.index("Y")


def test_duplicate_top_level_const_mixed_ann_assign():
    src = "X: int = 1\nX = 2\n"
    result = analyzer.analyze(src)
    names = [c.name for c in result.candidates]
    assert names.count("X") == 1
    (cand,) = result.candidates
    assert cand.default == 2


def test_duplicate_const_injection_no_longer_corrupts_source():
    # A valid script with a duplicate top-level const used to become unparseable (str case) or
    # silently run with the wrong value (int case) once
    # injected. With a single deduped candidate/spec, shim replaces every same-named occurrence
    # exactly once and the result stays valid and correct.
    from skit.langs.python import shim
    from skit.params import ParamDecl

    src = "CITY = 'a'\nCITY = 'b'\nprint(CITY)\n"
    result = analyzer.analyze(src)
    (cand,) = result.candidates
    spec = ParamDecl.from_candidate(cand)
    injected = shim.inject(src, [spec], {"CITY": "Paris"})
    assert injected == "CITY = 'Paris'\nCITY = 'Paris'\nprint(CITY)\n"
    import ast

    ast.parse(injected)  # must still be valid Python (used to raise SyntaxError)


# ---------- callmatch.match_calls: prompt-keyed input matching ----------


def test_match_inputs_prompt_survives_position_shift():
    # A source edit inserted a new input() call before the stored one, shifting its bare position
    # from 0 to 1 -- but the prompt text is unchanged, so it must still resolve correctly, and not
    # be flagged (ambiguous=False): this is exactly the "no silent rebind" case working as intended.
    stored = [(0, "Password: ")]
    current = [(0, "Username: "), (1, "Password: ")]
    bindings = callmatch.match_calls(stored, current)
    assert bindings == {0: (1, False)}


def test_match_inputs_falls_back_to_position_when_no_prompt_recorded():
    # Legacy/dynamic-prompt entries (prompt="") have no stronger signal than position, and that's
    # not a newly introduced risk, so it resolves silently (ambiguous=False), matching the
    # previous positional behavior.
    stored = [(0, "")]
    current = [(0, "Anything: ")]
    assert callmatch.match_calls(stored, current) == {0: (0, False)}


def test_match_inputs_flags_ambiguous_when_prompt_renamed_but_position_still_exists():
    # The stored prompt no longer appears anywhere in the current source (renamed), but a call
    # still exists at the stored position: fall back to position, but flag it -- the caller must
    # surface a warning rather than silently trusting it.
    stored = [(0, "Old prompt: ")]
    current = [(0, "New prompt: ")]
    bindings = callmatch.match_calls(stored, current)
    assert bindings == {0: (0, True)}


def test_match_inputs_flags_ambiguous_when_two_call_sites_share_a_prompt():
    # Two distinct call sites with the identical literal prompt text can't be told apart by prompt
    # alone; falling back to position is still flagged as ambiguous rather than silently trusted.
    stored = [(0, "Value: ")]
    current = [(0, "Value: "), (1, "Value: ")]
    bindings = callmatch.match_calls(stored, current)
    assert bindings == {0: (0, True)}


def test_match_inputs_missing_when_neither_prompt_nor_position_resolves():
    stored = [(2, "Gone: ")]
    current = [(0, "Other: ")]
    assert callmatch.match_calls(stored, current) == {}


# ---------- callmatch.match_calls: duplicate STORED prompts must never map two-to-one (regression) ----------


def test_match_inputs_duplicate_stored_prompts_never_double_bind_on_delete():
    # Two stored specs shared the identical literal prompt (a retry pattern: two input("Go? ")
    # calls, both managed). The user deletes one of the two calls, leaving a single current call
    # site with that prompt. The first-listed stored entry wins the exact match; the second must
    # NOT also resolve to that same current order (that would corrupt the injected copy) -- its
    # bare position (1) no longer exists either, so it must come back missing entirely.
    stored = [(0, "Go? "), (1, "Go? ")]
    current = [(0, "Go? ")]
    bindings = callmatch.match_calls(stored, current)
    assert bindings == {0: (0, False)}
    # Explicit invariant: no two stored keys ever resolve to the same current order.
    resolved = [current_order for current_order, _ in bindings.values()]
    assert len(resolved) == len(set(resolved))


def test_match_inputs_duplicate_stored_prompts_edit_one_flags_rebind_for_loser():
    # Same duplicate-prompt setup, but this time the call at position 1 still exists -- its prompt
    # was just edited to something else. The losing stored entry can't get an exact match (its
    # prompt's one candidate was already claimed by the winner), so it falls back to bare position
    # 1, which now holds a *different* question -- that must be flagged ambiguous (rebind), never
    # silently trusted and never double-bound onto position 0.
    stored = [(0, "Go? "), (1, "Go? ")]
    current = [(0, "Go? "), (1, "Different: ")]
    bindings = callmatch.match_calls(stored, current)
    assert bindings == {0: (0, False), 1: (1, True)}
    resolved = [current_order for current_order, _ in bindings.values()]
    assert len(resolved) == len(set(resolved))


def test_match_inputs_triple_duplicate_stored_prompts_only_one_winner():
    # Three stored specs share one prompt; only one current call site remains. Exactly one stored
    # entry may claim it; the other two must come back missing (their bare positions 1 and 2 don't
    # exist in the current source either) -- never sharing the winner's current order.
    stored = [(0, "Go? "), (1, "Go? "), (2, "Go? ")]
    current = [(0, "Go? ")]
    bindings = callmatch.match_calls(stored, current)
    assert bindings == {0: (0, False)}
    resolved = [current_order for current_order, _ in bindings.values()]
    assert len(resolved) == len(set(resolved))
