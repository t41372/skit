"""Preset save/resolution order + C3 structural secret stripping.

No secret value may ever land on disk.
"""

from __future__ import annotations

import pytest

from skit import argstate
from skit.metawriter import ParamSpec
from skit.paths import values_dir


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))


def spec(name: str, *, default=None, secret: bool = False) -> ParamSpec:
    return ParamSpec(name=name, kind="const", type="str", default=default, secret=secret)


def test_preset_roundtrip(tmp_store):
    argstate.save_preset("s", "prod", {"CITY": "Taipei"})
    state = argstate.load_state("s")
    assert state["presets"]["prod"] == {"CITY": "Taipei"}
    assert argstate.delete_preset("s", "prod") is True
    assert argstate.load_state("s")["presets"] == {}
    assert argstate.delete_preset("s", "nope") is False


def test_resolution_order_preset_over_last_over_default(tmp_store):
    specs = [spec("CITY", default="Osaka"), spec("N", default="1")]
    # Definition default only
    assert argstate.resolve_defaults(specs, "s") == {"CITY": "Osaka", "N": "1"}
    # Last-used value overrides default
    argstate.save_last("s", values={"CITY": "Taipei"})
    assert argstate.resolve_defaults(specs, "s")["CITY"] == "Taipei"
    # Preset overrides last-used
    argstate.save_preset("s", "jp", {"CITY": "Kyoto"})
    assert argstate.resolve_defaults(specs, "s", "jp")["CITY"] == "Kyoto"
    # Stale keys in state must never leak into the form
    argstate.save_last("s", values={"STALE": "x"})
    assert "STALE" not in argstate.resolve_defaults(specs, "s")


def test_c3_secret_never_touches_disk(tmp_store):
    argstate.save_last(
        "s", values={"API_KEY": "hunter2", "CITY": "Taipei"}, secret_names={"API_KEY"}
    )
    argstate.save_preset(
        "s", "prod", {"API_KEY": "hunter2", "CITY": "Taipei"}, secret_names={"API_KEY"}
    )
    state = argstate.load_state("s")
    assert "API_KEY" not in state["values"]
    assert "API_KEY" not in state["presets"]["prod"]
    # Scan the raw bytes of every state file to guarantee the value itself was never written
    for p in values_dir().glob("*.toml"):
        assert "hunter2" not in p.read_text(encoding="utf-8")


def test_preset_preserved_across_save_last(tmp_store):
    argstate.save_preset("s", "prod", {"CITY": "Taipei"})
    argstate.save_last("s", values={"CITY": "Tainan"}, extra_args=["-v"])
    state = argstate.load_state("s")
    assert state["presets"]["prod"] == {"CITY": "Taipei"}
    assert state["values"]["CITY"] == "Tainan"
    assert state["extra_args"] == ["-v"]
