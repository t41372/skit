"""Preset save/resolution order + C3 structural secret stripping.

No secret value may ever land on disk.
"""

from __future__ import annotations

import pytest

from skit import argstate
from skit.params import ParamDecl
from skit.paths import values_dir


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))


def spec(name: str, *, default=None, secret: bool = False) -> ParamDecl:
    return ParamDecl(
        name=name, binding="const", delivery="inject", type="str", default=default, secret=secret
    )


def test_preset_roundtrip(tmp_store):
    argstate.save_preset("s", "prod", {"CITY": "Taipei"})
    state = argstate.load_state("s")
    assert state["presets"]["prod"] == {"CITY": "Taipei"}
    assert argstate.delete_preset("s", "prod") is True
    assert argstate.load_state("s")["presets"] == {}
    assert argstate.delete_preset("s", "nope") is False


def test_resolution_order_preset_over_last_over_default(tmp_store):
    # Resolution moved from argstate.resolve_defaults into flows.prefill (the unified
    # form layer); the contract is the same: preset > last-used > definition default.
    from skit import flows

    specs = [spec("CITY", default="Osaka"), spec("N", default="1")]
    plan = flows.FormPlan(source="inject", fields=[flows.FormField.from_decl(s) for s in specs])
    # Definition default only
    assert flows.prefill(plan, "s") == {"CITY": "Osaka", "N": "1"}
    # Last-used value overrides default
    argstate.save_last("s", values={"CITY": "Taipei"})
    assert flows.prefill(plan, "s")["CITY"] == "Taipei"
    # Preset overrides last-used
    argstate.save_preset("s", "jp", {"CITY": "Kyoto"})
    assert flows.prefill(plan, "s", "jp")["CITY"] == "Kyoto"
    # Stale keys in state must never leak into the form
    argstate.save_last("s", values={"STALE": "x", "CITY": "Taipei"})
    assert "STALE" not in flows.prefill(plan, "s")


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


# --------------------------------------------------------------------------
# purge_secret + save_last stale-key dropping ("secrets aren't fully secret" gap)
# --------------------------------------------------------------------------


def test_purge_secret_removes_from_values_and_every_preset(tmp_store):
    # A value stored while a param was still public, plus a copy saved to two presets, must all
    # disappear once the param transitions to secret.
    argstate.save_last("s", values={"API_KEY": "shown", "CITY": "Taipei"})
    argstate.save_preset("s", "prod", {"API_KEY": "shown", "CITY": "Taipei"})
    argstate.save_preset("s", "dev", {"API_KEY": "shown"})
    removed = argstate.purge_secret("s", {"API_KEY"})
    assert removed == {"API_KEY"}
    state = argstate.load_state("s")
    assert "API_KEY" not in state["values"]
    assert state["values"]["CITY"] == "Taipei"  # unrelated, still-public value untouched
    assert "API_KEY" not in state["presets"]["prod"]
    assert state["presets"]["prod"]["CITY"] == "Taipei"
    # 'dev' held only API_KEY, so purging it leaves the preset empty -> it is dropped entirely
    # (mirroring delete_preset), not kept as a confusing value-less table.
    assert "dev" not in state["presets"]
    for p in values_dir().glob("*.toml"):
        assert "shown" not in p.read_text(encoding="utf-8")


def test_purge_secret_drops_a_preset_left_empty_but_keeps_others(tmp_store):
    # A preset whose only key was the now-secret param is removed; a preset with surviving
    # public keys is retained (minus the secret). No dangling empty [presets.*] table remains.
    argstate.save_preset("s", "onlysecret", {"API_KEY": "shown"})
    argstate.save_preset("s", "mixed", {"API_KEY": "shown", "CITY": "Taipei"})
    argstate.purge_secret("s", {"API_KEY"})
    state = argstate.load_state("s")
    assert "onlysecret" not in state["presets"]
    assert state["presets"]["mixed"] == {"CITY": "Taipei"}
    text = (values_dir() / "s.toml").read_text(encoding="utf-8")
    assert "onlysecret" not in text
    assert "shown" not in text


def test_purge_secret_empty_names_is_noop(tmp_store):
    argstate.save_last("s", values={"CITY": "Taipei"})
    path = values_dir() / "s.toml"
    before = path.read_text(encoding="utf-8")
    assert argstate.purge_secret("s", []) == set()
    assert path.read_text(encoding="utf-8") == before


def test_purge_secret_reports_only_names_actually_stored(tmp_store):
    argstate.save_last("s", values={"CITY": "Taipei"})
    removed = argstate.purge_secret("s", {"API_KEY", "CITY"})
    assert removed == {"CITY"}  # API_KEY was never stored, so it can't have been removed


def test_save_last_drops_stale_value_once_param_becomes_secret(tmp_store):
    # Reproduces the argstate.py gap directly: save_last's read-modify-write only replaced
    # doc["values"] when this call's own (now-stripped) snapshot was non-empty. A script with a
    # single, newly-secret parameter collects nothing else, so `clean` is empty and the old guard
    # left the stale plaintext in place forever.
    argstate.save_last("s", values={"API_KEY": "old-secret"})
    assert argstate.load_state("s")["values"]["API_KEY"] == "old-secret"
    argstate.save_last("s", values={"API_KEY": "new-typed"}, secret_names={"API_KEY"})
    state = argstate.load_state("s")
    assert "API_KEY" not in state["values"]
    for p in values_dir().glob("*.toml"):
        assert "old-secret" not in p.read_text(encoding="utf-8")
        assert "new-typed" not in p.read_text(encoding="utf-8")


def test_save_last_values_are_a_snapshot_not_a_merge(tmp_store):
    # values is the run's complete snapshot: replace semantics are what make "the user
    # cleared this field" persist (the old merge semantics resurrected cleared values).
    argstate.save_last("s", values={"API_KEY": "old-secret", "CITY": "Taipei"})
    argstate.save_last("s", values={"API_KEY": "x"}, secret_names={"API_KEY"})
    state = argstate.load_state("s")
    assert "API_KEY" not in state["values"]  # C3 strip on the new write
    assert "CITY" not in state["values"]  # not in this snapshot -> gone


def test_save_last_none_values_still_scrubs_stale_secret(tmp_store):
    # No new data at all (values=None): stored values stay, EXCEPT names that just
    # became secret — those are scrubbed even without a resupply.
    argstate.save_last("s", values={"API_KEY": "old-secret", "CITY": "Taipei"})
    argstate.save_last("s", secret_names={"API_KEY"})
    state = argstate.load_state("s")
    assert "API_KEY" not in state["values"]
    assert state["values"]["CITY"] == "Taipei"


def test_save_last_regression_non_secret_values_persist_normally(tmp_store):
    # Non-secret params must keep behaving exactly as before: stored and read back verbatim.
    argstate.save_last("s", values={"CITY": "Taipei", "N": "3"})
    assert argstate.load_state("s")["values"] == {"CITY": "Taipei", "N": "3"}
