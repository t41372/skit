"""Fail CI unless every recorded mutmut result is acceptable."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

FAILED_STATES = (
    "survived",
    "no_tests",
    "timeout",
    "suspicious",
    "check_was_interrupted_by_user",
    "segfault",
)


def failure_detail(stats: dict[str, Any]) -> str:
    """An empty string means the mutation run is a clean gate pass."""
    total = int(stats.get("total", 0))
    if total <= 0:
        return "no mutants were recorded"

    failures = {state: int(stats.get(state, 0)) for state in FAILED_STATES if stats.get(state, 0)}
    accepted = int(stats.get("killed", 0)) + int(stats.get("skipped", 0))
    accounted = accepted + sum(failures.values())
    parts = [f"{state}={count}" for state, count in failures.items()]
    if accounted != total:
        parts.append(f"unaccounted={total - accounted}")
    return ", ".join(parts)


def main(path: Path = Path("mutants/mutmut-cicd-stats.json")) -> int:
    with path.open(encoding="utf-8") as handle:
        stats = json.load(handle)
    print("mutation summary:", json.dumps(stats, sort_keys=True))
    if detail := failure_detail(stats):
        print(f"::error::Mutation testing was not clean: {detail}")
        return 1
    print("Mutation gate passed: every mutant has an accepted result.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
