from __future__ import annotations

import importlib.util
import json
from pathlib import Path

import pytest

_SPEC = importlib.util.spec_from_file_location(
    "check_mutation_stats", Path(__file__).parent.parent / "scripts" / "check_mutation_stats.py"
)
assert _SPEC is not None
assert _SPEC.loader is not None
check_mutation_stats = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(check_mutation_stats)


def _clean_stats() -> dict[str, int]:
    return {
        "total": 12,
        "killed": 10,
        "skipped": 2,
        "survived": 0,
        "no_tests": 0,
        "timeout": 0,
        "suspicious": 0,
        "check_was_interrupted_by_user": 0,
        "segfault": 0,
    }


def test_clean_mutation_stats_pass():
    assert check_mutation_stats.failure_detail(_clean_stats()) == ""


@pytest.mark.parametrize("state", check_mutation_stats.FAILED_STATES)
def test_every_unsuccessful_mutation_state_fails(state):
    stats = _clean_stats()
    stats["killed"] -= 1
    stats[state] = 1
    assert check_mutation_stats.failure_detail(stats) == f"{state}=1"


def test_empty_or_unaccounted_mutation_stats_fail():
    assert check_mutation_stats.failure_detail({"total": 0}) == "no mutants were recorded"
    stats = _clean_stats()
    stats["killed"] -= 1
    assert check_mutation_stats.failure_detail(stats) == "unaccounted=1"


def test_main_returns_failure_and_emits_ci_annotation(tmp_path, capsys):
    path = tmp_path / "stats.json"
    path.write_text(json.dumps({"total": 1, "survived": 1}), encoding="utf-8")
    assert check_mutation_stats.main(path) == 1
    assert "::error::" in capsys.readouterr().out
