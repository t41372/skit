"""Round-16 design-audit fixes — the read-path stragglers the round-15 audit named the "last
two panes" of the round-14/15 rule: *the record every DISPLAY surface shows must be the effective
metadata a run enforces, and the deps save-diff must run on the same open-time clock as every other
axis*. Round-15 fixed the `deps` command and the settings prefill; round-16 completes it for the
two remaining display faces and the compose-time save baseline.

Every assertion pins an OBSERVABLE contract — the human `show` a user reads, the library detail
pane a user sees, the confirmation lines `deps` prints, the deps chokepoint a save does or does not
enter — never an internal flag:

  * show-human effective (MEDIUM): `skit show x` printed the Dependencies / Python-constraint lines
    from RAW meta, so a block-only add-time entry (deps + pin in the copy's PEP 723 block, meta
    deliberately blank) showed NEITHER line while its own `show --json` reported both. Now both
    faces read `effective_uv_metadata`.
  * detail-pane effective (LOW): the "Depends on" line read raw meta too — the pane showed nothing
    for a block-only entry while that same entry's settings screen showed the list (two panes of
    one TUI disagreeing about one record). Now it reads the effective deps.
  * per-axis deps confirmations (LOW): `skit deps x --dep … --python …` moved BOTH axes but printed
    only the deps line — silence about a constraint that DID move. Now each line prints exactly when
    its axis was edited (the both-axes case is pinned in test_round13_audit alongside its siblings).
  * compose-time save baseline (LOW): the settings save diffed the deps/constraint fields against a
    SAVE-time re-read of effective_uv_metadata, so a concurrent CLI write that moved the block
    underneath an open screen made an untouched field look like an explicit edit. Now the baseline is
    stashed at compose time — the same open-time clock every other axis diffs against.

These never chdir and never touch the real user dirs (the local SKIT_* fixture).
"""

from __future__ import annotations

from pathlib import Path

import pytest
from textual.widgets import Input, Static
from typer.testing import CliRunner

from conftest import footer_text
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


def _py(tmp_path: Path, body: str = "print(1)\n", name: str = "x.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _detail(app) -> str:
    return footer_text(app.query_one("#detail-body", Static))


# ==========================================================================
# 1. show-human reads EFFECTIVE metadata (MEDIUM)
# ==========================================================================


def test_show_human_block_only_prints_effective_deps_and_constraint(tmp_path):
    """`skit show x` (human) reads EFFECTIVE metadata: a block-only add-time entry (deps + pin in
    the copy's PEP 723 block, meta deliberately blank) prints its real Dependencies AND Python
    constraint lines — matching its own `show --json`, where raw meta showed a bare face with
    neither. Both `if effective_deps` / `if effective_python` truthy."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    assert store.resolve("x").meta.dependencies is None  # meta blank...
    assert store.resolve("x").meta.requires_python == ""  # ...on both axes
    result = runner.invoke(cli.app, ["show", "x"])
    assert result.exit_code == 0, result.output
    assert "Dependencies: requests" in result.output
    assert "Python constraint: >=3.11" in result.output


def test_show_human_meta_carried_deps_unchanged(tmp_path):
    """The meta-carried face is unchanged by the switch to effective_uv_metadata: a reference-mode
    entry records its deps in meta, and the human `show` prints them straight from meta (the block
    fallback is a python-copy-only path that never fires here)."""
    store.add_python(_py(tmp_path), name="x", mode="reference", dependencies=["rich"])
    assert store.resolve("x").meta.dependencies == ["rich"]  # meta carries it
    result = runner.invoke(cli.app, ["show", "x"])
    assert result.exit_code == 0, result.output
    assert "Dependencies: rich" in result.output


def test_show_human_no_uv_metadata_prints_neither_line(tmp_path):
    """An entry with no deps and no pin (effective_uv_metadata returns the empty pair) prints
    NEITHER line — the falsy branch of both display conditions."""
    store.add_python(_py(tmp_path), name="x")
    result = runner.invoke(cli.app, ["show", "x"])
    assert result.exit_code == 0, result.output
    assert "Dependencies:" not in result.output
    assert "Python constraint:" not in result.output


# ==========================================================================
# 2. library detail pane reads EFFECTIVE deps (LOW)
# ==========================================================================


async def test_detail_pane_block_only_shows_effective_depends_on(tmp_path):
    """The library detail pane reads EFFECTIVE deps: a block-only add-time python entry (meta blank,
    deps in the copy's block) shows "Depends on requests" — the same list its settings screen shows,
    where raw meta would have left the line off entirely."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"])
    assert store.resolve("x").meta.dependencies is None  # meta blank; deps live in the block
    app = tui.MenuApp()
    async with app.run_test():
        detail = _detail(app)
        assert "Depends on" in detail
        assert "requests" in detail


async def test_detail_pane_no_deps_omits_the_depends_on_line(tmp_path):
    """The falsy branch: an entry with no effective deps shows no "Depends on" line at all."""
    store.add_python(_py(tmp_path), name="x")
    app = tui.MenuApp()
    async with app.run_test():
        assert "Depends on" not in _detail(app)


# ==========================================================================
# 3. compose-time save baseline (LOW) — the settings deps diff runs on the
#    open-time clock, not a save-time re-read.
# ==========================================================================


async def test_settings_save_diffs_against_compose_time_baseline_not_a_re_read(
    tmp_path, monkeypatch
):
    """The deps/constraint save-diff runs against `_deps_baseline` stashed when the fields were
    composed — NOT a save-time re-read of effective_uv_metadata. A concurrent CLI write that moves
    the block underneath an open screen must not make an UNTOUCHED field look like an explicit edit.

    Pinned by driving exactly that race: after mount, monkeypatch effective_uv_metadata to a
    DIFFERENT pair (the "concurrent write") — a save-time re-read would now diff the unchanged Inputs
    against it and misclassify them as edits. Because the diff uses the compose-time baseline, an
    untouched save never enters the deps chokepoint: update_dependencies is not called."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    calls: list[object] = []
    real = store.update_dependencies

    def _counting(*args, **kwargs):
        calls.append(args)
        return real(*args, **kwargs)

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("x"))
        app.push_screen(screen)
        await pilot.pause()
        # The baseline equals the composed Input values — the same open-time clock a save diffs.
        assert screen._deps_baseline == (["requests"], ">=3.11")
        assert screen.query_one("#st-deps", Input).value == "requests"
        assert screen.query_one("#st-python", Input).value == ">=3.11"
        # A concurrent write moves the block AFTER the screen composed. A save-time re-read would
        # now see (numpy, >=3.13) and call the untouched Inputs an edit; the compose-time baseline
        # does not.
        monkeypatch.setattr(store, "effective_uv_metadata", lambda entry: (["numpy"], ">=3.13"))
        monkeypatch.setattr(store, "update_dependencies", _counting)
        screen.action_save()  # nothing edited
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # committed & dismissed
    assert calls == []  # the untouched save never entered the deps chokepoint
