"""Mutation-kill tests for skit/argstate.py.

argstate persists last-used values, extra args and named presets, and enforces C3 (secret keys
never hit disk, and retroactively scrubbing a value that predates a param becoming secret). These
exercise purge_secret's accumulation of *which* names were cleaned, and save_last's secret-drop on
a values=None call, through the real on-disk read-modify-write.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import argstate


@pytest.fixture(autouse=True)
def _isolated_state(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))


def test_purge_secret_reports_names_removed_across_values_and_presets() -> None:
    """purge_secret returns the subset of names it actually scrubbed (so the caller can tell the
    user what was cleaned). The name lives in [values]; a preset that does NOT hold it must not
    reset that accumulation — pins that the per-preset union keeps the value-side hit."""
    slug = "purge-demo"
    argstate.save_last(slug, values={"API_TOKEN": "abc", "REGION": "us"})
    argstate.save_preset(slug, "prod", {"REGION": "eu"})

    removed = argstate.purge_secret(slug, ["API_TOKEN"])

    # The token was stored in [values] and gets reported as removed even though the surviving
    # preset never held it. (mutant_34 `removed = …` / mutant_35 `removed &= …` would drop it to set().)
    assert removed == {"API_TOKEN"}

    state = argstate.load_state(slug)
    assert state["values"] == {"REGION": "us"}  # secret plaintext scrubbed from last-used
    assert state["presets"] == {"prod": {"REGION": "eu"}}  # non-secret preset preserved intact


def test_save_last_drops_secret_with_no_stored_values_table() -> None:
    """save_last strips now-secret keys even on a values=None call. When the on-disk doc carries
    only extra_args (no [values] table at all), the strip must default the absent table to {} —
    not None — or it would crash trying to filter a None (mutant_21 `doc.get("values", None)` /
    mutant_23 `doc.get("values")`). The call must complete and leave extra_args untouched."""
    slug = "no-values-table"
    argstate.save_last(slug, extra_args=["--verbose"])
    assert argstate.load_state(slug)["values"] == {}  # precondition: no stored values

    # values=None (no new data) but a param just became secret: reaches the elif banned branch,
    # whose doc.get("values", {}) default is what the mutants attack.
    argstate.save_last(slug, values=None, secret_names=["SECRET"])

    state = argstate.load_state(slug)
    assert state["values"] == {}
    assert state["extra_args"] == ["--verbose"]  # unrelated stored data survived the secret-drop
