"""Round-14 design-audit fixes — the two round-13 findings the auditor named "stop-short":
the store's copy-mode PEP 723 python block-sync now tracks an explicit unpin instead of
silently preserving it, and the npm --python refusal keys on `is not None` (the empty spelling
is refused too). Every assertion pins an OBSERVABLE contract end to end:

  * pin → unpin → re-pin: `deps --python <c>` writes the block's `requires-python` line AND
    surfaces `--python` in the run command; `deps --python -` REMOVES the line and drops
    `--python` from the run command and `--json`; a later re-pin restores both — the block uv
    reads and the three reporting surfaces stay in agreement at every step;
  * a DEPS-ONLY edit (requires_python is None at the chokepoint) still PRESERVES an existing
    pin, whether that pin lives only in the block (add-time injection clears meta) or in meta
    (a prior `deps --python`) — both branches of the `not constraint and requires_python is
    None` preserve predicate;
  * the settings-screen twin: clearing #st-python on a pinned copy-mode entry reaches the same
    chokepoint (requires_python == "") and unpins the stored block after save.

These never chdir and never touch the real user dirs (the local SKIT_* fixture).
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from textual.widgets import Input
from typer.testing import CliRunner

from skit import cli, i18n, store, tui
from skit.tui_settings import ScriptSettingsScreen

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")


def _flat(text: str) -> str:
    return " ".join(text.split())


def _py(tmp_path, body: str, name: str = "s.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _stored_block(slug: str) -> str:
    return (store.resolve(slug).dir / "script.py").read_text(encoding="utf-8")


def _dry_run(slug: str) -> str:
    result = runner.invoke(cli.app, ["run", slug, "--dry-run", "--no-input"])
    assert result.exit_code == 0, result.output
    return _flat(result.output)


# ==========================================================================
# 1. pin → unpin → re-pin, tracked end to end across block + run command + --json
# ==========================================================================


def test_pin_unpin_repin_block_line_tracks_the_constraint_end_to_end(tmp_path):
    """The whole arc through the CLI: a pin writes the block's requires-python AND puts --python
    on the launch command; an explicit unpin removes both (and clears --json); a re-pin restores
    both. The stored block is what uv actually enforces, so the visible command and the block
    must never disagree — the exact drift the round-13 audit found (unpin cleared the command
    while the block stayed pinned)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")

    # --- pin ---
    pin = runner.invoke(cli.app, ["deps", "a", "--python", ">=3.12"])
    assert pin.exit_code == 0, pin.output
    assert 'requires-python = ">=3.12"' in _stored_block("a")  # block line written
    assert "--python" in _dry_run("a")  # and uv would launch with the constraint

    # --- unpin ---
    unpin = runner.invoke(cli.app, ["deps", "a", "--python", "-"])
    assert unpin.exit_code == 0, unpin.output
    assert "requires-python" not in _stored_block("a")  # block line removed
    assert "--python" not in _dry_run("a")  # the launch command drops it too
    view = runner.invoke(cli.app, ["deps", "a", "--json"])
    assert json.loads(view.stdout)["requires_python"] == ""  # --json agrees

    # --- re-pin ---
    repin = runner.invoke(cli.app, ["deps", "a", "--python", ">=3.13"])
    assert repin.exit_code == 0, repin.output
    assert 'requires-python = ">=3.13"' in _stored_block("a")  # block line returns
    assert "--python" in _dry_run("a")  # and the launch command carries it again
    assert store.resolve("a").meta.requires_python == ">=3.13"  # meta in step


# ==========================================================================
# 2. a DEPS-ONLY edit preserves the pin — both branches of the preserve predicate
# ==========================================================================


def test_deps_only_edit_preserves_a_pin_that_lives_only_in_the_block(tmp_path):
    """Branch A of `not constraint and requires_python is None`: an add-time constraint injects
    the block but leaves meta.requires_python "" (store.add_python's deps_injected path). A
    later deps-only edit reads the block for the constraint and PRESERVES it — the original
    derive rule, exercised on the meta-is-blank side it was written for."""
    src = _py(tmp_path, "print(1)\n")
    store.add_python(src, name="a", requires_python=">=3.11")
    assert store.resolve("a").meta.requires_python == ""  # add-time injection clears meta
    assert 'requires-python = ">=3.11"' in _stored_block("a")  # ...but the block carries it
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "requests"])
    assert result.exit_code == 0, result.output
    assert 'requires-python = ">=3.11"' in _stored_block("a")  # preserved from the block
    assert "requests" in _stored_block("a")  # the deps edit landed


def test_deps_only_edit_preserves_a_pin_that_lives_in_meta(tmp_path):
    """Branch B of the same predicate: a prior `deps --python` sets meta.requires_python, so a
    deps-only edit finds the constraint truthy in meta (the `if not constraint` guard is False)
    and preserves it there — the block stays pinned too."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_dependencies("a", [], requires_python=">=3.10")  # meta + block pinned
    assert store.resolve("a").meta.requires_python == ">=3.10"
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "requests"])
    assert result.exit_code == 0, result.output
    assert store.resolve("a").meta.requires_python == ">=3.10"  # meta pin preserved
    assert 'requires-python = ">=3.10"' in _stored_block("a")  # block still pinned
    assert "requests" in _stored_block("a")


# ==========================================================================
# 3. the settings-screen twin: clearing #st-python unpins the block
# ==========================================================================


async def test_settings_clearing_python_unpins_the_block(tmp_path):
    """The TUI face of the unpin: emptying #st-python on a pinned copy-mode entry reaches the
    same store chokepoint with requires_python == "", so the save removes the block's
    requires-python line — not just the meta field. (The '-' twin already covers meta; this one
    pins the stored block, the surface uv reads.)"""
    store.add_python(_py(tmp_path, "print(1)\n"), name="pinned")
    store.update_dependencies("pinned", ["requests"], requires_python=">=3.11")
    assert 'requires-python = ">=3.11"' in _stored_block("pinned")  # pinned first
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("pinned"))
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#st-python", Input).value == ">=3.11"  # prefilled from meta
        screen.query_one("#st-python", Input).value = ""  # clear the constraint
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # committed & dismissed
    assert store.resolve("pinned").meta.requires_python == ""  # meta cleared
    assert "requires-python" not in _stored_block("pinned")  # block unpinned after save
