"""Shell getopts reader unit pins: the optstring matrix + plan/assemble integration.

The reader turns a literal `while getopts "ab:c:" opt` into single-dash flag fields (a bare letter
⇒ store_true bool, `letter:` ⇒ str value flag), degrades a dynamic optstring / a broken script /
a getopts-less script to None, and — since shell carries analyzer + params_io + cli_reader — is
reached through the normal plan chain (in-file specs → declared riders → getopts → none).
"""

from __future__ import annotations

from pathlib import Path

from skit import flows, store
from skit.langs.shell import cli_reader as sc


def fields(src):
    spec = sc.read_cli(src)
    assert spec is not None
    return {f.name: f for f in spec.fields}


def test_value_and_bool_flags():
    fs = fields('while getopts "n:v" opt; do :; done\n')
    assert (fs["n"].type, fs["n"].flag, fs["n"].action) == ("str", "-n", "")
    assert (fs["v"].type, fs["v"].flag, fs["v"].action) == ("bool", "-v", "store_true")
    assert fs["v"].default is False


def test_leading_colon_silent_mode_is_skipped():
    fs = fields('while getopts ":ab:c:" opt; do :; done\n')
    assert list(fs) == ["a", "b", "c"]
    assert fs["a"].type == "bool"
    assert fs["b"].type == "str"


def test_non_alphanumeric_characters_are_skipped():
    fs = fields('while getopts "a-b" opt; do :; done\n')
    assert list(fs) == ["a", "b"]  # the stray '-' is ignored, both letters are bool flags


def test_repeated_letter_keeps_first():
    fs = fields('while getopts "vv" opt; do :; done\n')
    assert list(fs) == ["v"]


def test_dynamic_optstring_degrades_to_dynamic():
    """A dynamic optstring is DETECTED but unmodelable: the reader degrades honestly to
    ok=False 'dynamic' (the python/JS distinction), not None — None would claim the script
    has no CLI at all, and the run form must instead say it couldn't model this one."""
    spec = sc.read_cli('getopts "$OPTS" opt\n')
    assert spec is not None
    assert spec.ok is False
    assert spec.reason == "dynamic"
    assert spec.fields == []


def test_getopts_without_optstring_is_none():
    assert sc.read_cli("getopts\n") is None


def test_no_getopts_is_none():
    assert sc.read_cli("echo hello\n") is None


def test_unparseable_script_is_none():
    assert sc.read_cli("if\n") is None  # tree-sitter reports has_error -> no readable surface


def test_secret_letter_is_not_special():
    # A single option letter never matches the secret-name heuristic (KEY/TOKEN/…).
    assert fields('while getopts "k:" opt; do :; done\n')["k"].secret is False


# ---------------------------------------------------------------- plan / assemble


def test_plan_reads_getopts_and_assembles_flags(tmp_path: Path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    src = tmp_path / "tool.sh"
    src.write_text('#!/usr/bin/env bash\nwhile getopts "n:v" opt; do :; done\n')
    entry = store.add_script(src, kind="shell", name="gt")
    plan = flows.plan_for_entry(entry)
    assert plan.source == "argparse"
    assert [f.key for f in plan.fields] == ["n", "v"]
    asm = flows.assemble(plan, {"n": "Ada", "v": "true"}, [], cwd=tmp_path)
    assert asm.args == ["-n", "Ada", "-v"]


def test_plan_dynamic_getopts_degrades_with_reason(tmp_path: Path, monkeypatch):
    """A shell entry whose getopts optstring is dynamic surfaces the degraded-form notice
    on the run form (source='argparse', degraded_reason='dynamic') instead of silently
    claiming there is no CLI — the same honest degradation python/JS give."""
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    src = tmp_path / "dyn.sh"
    src.write_text('#!/usr/bin/env bash\nOPTS="n:v"\nwhile getopts "$OPTS" opt; do :; done\n')
    entry = store.add_script(src, kind="shell", name="dyn")
    plan = flows.plan_for_entry(entry)
    assert plan.source == "argparse"
    assert plan.degraded_reason == "dynamic"
    assert plan.fields == []
