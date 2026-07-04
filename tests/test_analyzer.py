"""Analyzer (candidate parameter detection): const, main-guard (C4), input ordering (B1),
framework detection, secret heuristics (C3 pre-stage)."""

from __future__ import annotations

from skit import analyzer


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
    inputs = [c for c in result.candidates if c.kind == "input"]
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
