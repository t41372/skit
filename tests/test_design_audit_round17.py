"""Behavior coverage for the design-audit round-17 fixes (the claim-by-address review).

Round 16 claimed identities — by slug. A claim that re-resolves an address adopts
whoever owns it NOW, so a remove + same-name re-add between a lane's first resolve and
its claim was silently blessed, and every guard after it protected the stranger. This
round makes the model actually hold:

N. ``store.claim_identity(entry)`` is compare-and-claim: it verifies — under the entry
   lock — that the disk still holds THE ENTRY the caller resolved, stamps a pre-id
   meta there, and REFUSES a changed owner. The races below are real: reincarnations
   injected between a lane's resolve and its claim, with no patching of the claim.
O. ``skit remove`` (both faces) authorizes the deletion against the entry the
   confirmation ask NAMED — a slug reissued while the user was answering refuses
   instead of deleting the new owner's everything.
P. The external editor never touches the stored path: copy-mode sessions edit a
   STAGED draft and land through ``store.commit_copy_edit``'s identity-checked
   transaction; a stale landing keeps the draft and names it.
Q. ``"" == ""`` is no longer an authorization anywhere: an OLDER skit's adds write no
   id, so a symmetric blank can be a reincarnation this version never saw — unknown
   identity does not persist, full stop.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, editor, flows, store, tui
from skit.paths import values_dir
from skit.tui import ConfirmRemove

runner = CliRunner()

STALE = "changed while this edit was underway"


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _cmd(name: str, template: str = "echo hi") -> store.Entry:
    return store.add_command(template, name=name)


def _shell(tmp_path: Path, name: str, body: str = '#!/bin/sh\necho "hi"\n') -> store.Entry:
    src = tmp_path / f"{name}.sh"
    src.write_text(body, encoding="utf-8")
    return store.add_script(src, kind="shell", name=name)


def _reincarnate(name: str, factory) -> store.Entry:
    store.remove(name)
    return factory()


def _strip_id_line(slug: str) -> None:
    path = store.scripts_dir() / slug / "meta.toml"
    kept = [
        line
        for line in path.read_text(encoding="utf-8").splitlines()
        if not line.startswith("id = ")
    ]
    path.write_text("\n".join(kept) + "\n", encoding="utf-8")


# ==========================================================================
# N. compare-and-claim — real pre-claim races, the claim itself unpatched
# ==========================================================================


def test_claim_identity_refuses_a_reissued_slug(tmp_path: Path) -> None:
    """The finding's core, at the API: the disk's owner changed between the caller's
    resolve and its claim — a claim that adopted the stranger would put every later
    guard on the wrong side."""
    old = _cmd("core17")
    new = _reincarnate("core17", lambda: _cmd("core17"))
    with pytest.raises(store.StaleEntryError):
        store.claim_identity(old)
    assert store.resolve(new.slug).meta.id == new.meta.id  # untouched, unclaimed


def test_a_run_that_races_a_reincarnation_before_its_claim_refuses(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """`run` resolves, refuses statically, THEN claims: a reincarnation inside that
    window must surface as the stale refusal — never a run of the stranger with the
    old entry's checks. The seam is real (the preset validator, which runs between
    the resolve and the claim); the claim is not patched."""
    _cmd("racerun")
    raced: list[store.Entry] = []
    real_validate = cli._validate_preset

    def _race(entry: store.Entry, preset: str | None) -> None:
        raced.append(_reincarnate("racerun", lambda: _cmd("racerun")))
        real_validate(entry, preset)

    monkeypatch.setattr(cli, "_validate_preset", _race)
    result = runner.invoke(cli.app, ["run", "racerun", "--no-input"])
    assert result.exit_code == 125
    assert STALE in result.output
    assert not (values_dir() / f"{raced[0].slug}.toml").exists()  # the stranger got nothing


def test_a_preset_save_that_races_a_reincarnation_before_its_claim_refuses(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Same window on `preset save`: the plan is built from the resolved entry, and a
    reincarnation before the claim refuses — the old schema's form never lands on the
    new owner."""
    _cmd("racesave", "echo {x}")
    real_plan = flows.plan_for_entry

    def _race(entry: store.Entry) -> flows.FormPlan:
        plan = real_plan(entry)
        if entry.meta.name == "racesave":
            _reincarnate("racesave", lambda: _cmd("racesave", "echo {x}"))
        return plan

    monkeypatch.setattr(flows, "plan_for_entry", _race)
    result = runner.invoke(cli.app, ["preset", "save", "racesave", "p", "--from-last"])
    assert result.exit_code == 125
    assert STALE in result.output
    assert argstate.load_state(store.resolve("racesave").slug)["presets"] == {}


async def test_a_stale_library_row_stops_the_run_lane_and_refreshes(tmp_path: Path) -> None:
    """The TUI's claim, fully real: the Library list still shows the OLD row when the
    user presses Enter — the claim refuses, the lane stops (no form for the stranger),
    and the list refreshes with the reason on the status line."""
    from textual.widgets import Static

    _cmd("tuirow")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        _reincarnate("tuirow", lambda: _cmd("tuirow"))  # the list is now a lie
        app.action_run()
        await pilot.pause()
        assert app.screen is app.screen_stack[0]  # no form was pushed
        status = str(app.query_one("#status", Static).render())
        assert status == "tuirow changed or was removed — the list has been refreshed."
    assert not (values_dir() / "tuirow.toml").exists()


async def test_a_stale_library_row_stops_the_settings_lane_too(tmp_path: Path) -> None:
    _cmd("tuiset")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        new = _reincarnate("tuiset", lambda: _cmd("tuiset"))
        app.action_settings()
        await pilot.pause()
        assert app.screen is app.screen_stack[0]  # no settings screen for the stranger
    assert store.resolve(new.slug).meta.id == new.meta.id


# ==========================================================================
# O. remove — the deletion is authorized against the entry the ask named
# ==========================================================================


def test_cli_remove_refuses_a_slug_reissued_during_the_ask(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The most destructive race there was: confirm "remove old?", have the answer
    delete the NEW owner's registry row, stored copy, meta and state. Refused now."""
    _cmd("rmrace")
    new_holder: list[store.Entry] = []

    def _race(_message: str) -> None:
        new_holder.append(_reincarnate("rmrace", lambda: _cmd("rmrace")))

    monkeypatch.setattr(cli, "_require_yes", lambda *_a, **_k: None)
    monkeypatch.setattr(cli, "_confirm_destructive", _race)
    result = runner.invoke(cli.app, ["remove", "rmrace"])
    assert result.exit_code == 125
    assert STALE in result.output
    survivor = store.resolve("rmrace")
    assert survivor.meta.id == new_holder[0].meta.id  # the new owner survived, intact
    assert survivor.script_path.parent.exists()


async def test_tui_remove_refuses_a_slug_reissued_during_the_modal(tmp_path: Path) -> None:
    from textual.widgets import Static

    _cmd("tuirm")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_remove()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, ConfirmRemove)
        new = _reincarnate("tuirm", lambda: _cmd("tuirm"))
        modal.action_confirm()
        await pilot.pause()
        status = str(app.query_one("#status", Static).render())
        assert status == (
            "Error: tuirm changed while this edit was underway — reopen it and try again."
        )
    assert store.resolve("tuirm").meta.id == new.meta.id  # still the new owner's


async def test_tui_remove_success_reports_no_error(tmp_path: Path) -> None:
    from textual.widgets import Static

    _cmd("tuirmok")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_remove()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, ConfirmRemove)
        modal.action_confirm()
        await pilot.pause()
        assert "Error" not in str(app.query_one("#status", Static).render())
    with pytest.raises(store.NotFoundError):
        store.resolve("tuirmok")


def test_remove_survives_an_unknown_kind(tmp_path: Path) -> None:
    """A hand-edited kind must not crash the npm-flavor probe on the way out."""
    entry = _cmd("weirdkind")
    meta_path = store.scripts_dir() / entry.slug / "meta.toml"
    meta_path.write_text(
        meta_path.read_text(encoding="utf-8").replace('kind = "command"', 'kind = "bogus"'),
        encoding="utf-8",
    )
    assert store.remove("weirdkind") == "weirdkind"


def test_a_legacy_remove_still_removes(tmp_path: Path) -> None:
    """ "" against "" refuses nothing here: an unstamped entry the user confirmed is
    still removable (the strict asymmetry only fires when the disk got a NEW owner
    stamped by a modern add)."""
    entry = _cmd("legacyrm")
    _strip_id_line(entry.slug)
    result = runner.invoke(cli.app, ["remove", "legacyrm", "--yes"])
    assert result.exit_code == 0, result.output
    with pytest.raises(store.NotFoundError):
        store.resolve("legacyrm")


# ==========================================================================
# P. the staged editor session
# ==========================================================================


def _noop_suspend():
    import contextlib

    @contextlib.contextmanager
    def _suspend(_self: tui.MenuApp):
        yield

    return _suspend


def _fake_editor(monkeypatch: pytest.MonkeyPatch, write: bytes | None, race=None) -> None:
    """Stand in for the whole $EDITOR session: optionally rewrite the draft, then
    optionally reincarnate the entry mid-session."""

    def _session(path: Path) -> int:
        if write is not None:
            path.write_bytes(write)
        if race is not None:
            race()
        return 0

    monkeypatch.setattr(editor, "open_in_editor", _session)


def test_cli_edit_commits_a_staged_copy_edit(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _shell(tmp_path, "editok")
    entry.script_path.chmod(0o744)
    _fake_editor(monkeypatch, b'#!/bin/sh\necho "edited"\n')
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["edit", "editok"])
    assert result.exit_code == 0, result.output
    assert entry.script_path.read_bytes() == b'#!/bin/sh\necho "edited"\n'
    assert os.stat(entry.script_path).st_mode & 0o777 == 0o744  # the copy keeps its mode
    assert not list(Path(store.scripts_dir().parent / "drafts").glob("edit-*"))  # cleaned up


def test_cli_edit_keeps_the_draft_when_the_slug_was_reissued_mid_session(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The editor session is the longest hold skit has. Its save must never land on
    the stranger: the commit refuses, and the refusal NAMES the draft that holds the
    user's work."""
    entry = _shell(tmp_path, "editrace")
    original = entry.script_path.read_bytes()
    new_holder: list[store.Entry] = []

    def _race() -> None:
        new_holder.append(_reincarnate("editrace", lambda: _shell(tmp_path, "editrace")))

    _fake_editor(monkeypatch, b'#!/bin/sh\necho "stolen"\n', race=_race)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["edit", "editrace"])
    assert result.exit_code == 125
    assert new_holder[0].script_path.read_bytes() == original  # the new owner's copy, untouched
    drafts = list(Path(store.scripts_dir().parent / "drafts").glob("edit-editrace-*.sh"))
    assert len(drafts) == 1  # named for its entry, carrying the copy's own suffix
    assert drafts[0].read_bytes() == b'#!/bin/sh\necho "stolen"\n'  # the work survives
    # The whole refusal, byte for byte: the stale sentence plus the recovery path.
    expected = (
        "editrace changed while this edit was underway — reopen it and try again. "
        f"Your edit was kept at: {drafts[0]}"
    )
    assert expected in " ".join(result.output.split())


def test_cli_edit_without_changes_commits_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _shell(tmp_path, "editnoop")
    before = os.stat(entry.script_path).st_mtime_ns
    _fake_editor(monkeypatch, None)  # the editor closes without writing
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["edit", "editnoop"])
    assert result.exit_code == 0, result.output
    assert os.stat(entry.script_path).st_mtime_ns == before
    assert not list(Path(store.scripts_dir().parent / "drafts").glob("edit-*"))


async def test_tui_edit_keeps_the_draft_on_a_stale_landing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from textual.widgets import Static

    entry = _shell(tmp_path, "tuiedit")
    original = entry.script_path.read_bytes()

    def _race() -> None:
        _reincarnate("tuiedit", lambda: _shell(tmp_path, "tuiedit"))

    _fake_editor(monkeypatch, b'#!/bin/sh\necho "stolen"\n', race=_race)
    monkeypatch.setattr(tui.MenuApp, "suspend", _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        status = str(app.query_one("#status", Static).render())
        drafts = list(Path(store.scripts_dir().parent / "drafts").glob("edit-tuiedit-*.sh"))
        assert len(drafts) == 1
        assert status == (
            "Error: tuiedit changed while this edit was underway — reopen it and try "
            f"again. Your edit was kept at: {drafts[0]}"
        )
    assert store.resolve("tuiedit").script_path.read_bytes() == original


async def test_tui_edit_commits_the_staged_copy(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _shell(tmp_path, "tuieditok")
    _fake_editor(monkeypatch, b'#!/bin/sh\necho "edited"\n')
    monkeypatch.setattr(tui.MenuApp, "suspend", _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
    assert entry.script_path.read_bytes() == b'#!/bin/sh\necho "edited"\n'


def test_a_refused_prompt_edit_never_touches_the_stored_copy(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Strictly better than the old direct session: invalid prompt bytes used to land
    on the STORED PATH and be refused there — staged, they never leave the draft."""
    src = tmp_path / "p.prompt.md"
    src.write_text("Hello\n", encoding="utf-8")
    entry = store.add_prompt(src, name="pedit")
    original = entry.script_path.read_bytes()
    _fake_editor(monkeypatch, b"\xff\xfe not utf-8")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["edit", "pedit"])
    assert result.exit_code == 125
    assert entry.script_path.read_bytes() == original  # the copy never saw the bytes


def test_commit_copy_edit_is_identity_checked_and_mode_preserving(tmp_path: Path) -> None:
    entry = _shell(tmp_path, "commit17")
    entry.script_path.chmod(0o700)
    store.commit_copy_edit(entry.slug, b"#!/bin/sh\nnew\n", expected_id=entry.meta.id)
    assert entry.script_path.read_bytes() == b"#!/bin/sh\nnew\n"
    assert os.stat(entry.script_path).st_mode & 0o777 == 0o700
    with pytest.raises(store.StaleEntryError):
        store.commit_copy_edit(entry.slug, b"#!/bin/sh\nother\n", expected_id="someone-else")
    assert entry.script_path.read_bytes() == b"#!/bin/sh\nnew\n"


def test_commit_copy_edit_names_a_missing_copy(tmp_path: Path) -> None:
    entry = _shell(tmp_path, "commitgone")
    entry.script_path.unlink()
    with pytest.raises(store.NotFoundError) as refusal:
        store.commit_copy_edit(entry.slug, b"#!/bin/sh\n", expected_id=entry.meta.id)
    assert str(refusal.value) == "commitgone has no stored copy to edit."


# ==========================================================================
# Q. unknown identity persists nothing — across versions too
# ==========================================================================


def test_an_old_versions_idless_readd_gets_no_state(tmp_path: Path) -> None:
    """The cross-version hole, closed: an OLDER skit's add writes no id, so its
    re-add of a removed slug is a reincarnation this version's blank-vs-blank
    comparison cannot see. Unknown identity therefore persists NOTHING."""
    entry = _cmd("oldworld", "echo {x}")
    _strip_id_line(entry.slug)
    held = store.resolve(entry.slug)  # a legacy, unstamped handle
    plan = flows.plan_for_entry(held)
    store.remove("oldworld")
    reborn = _cmd("oldworld", "echo {x}")
    _strip_id_line(reborn.slug)  # the old version's add: no id, same slug
    flows.save_after_run(
        held, plan, {"x": "leak"}, [], 0, at="2026-02-01T00:00:00+00:00", extra_raw=False
    )
    assert not (values_dir() / f"{held.slug}.toml").exists()


def test_a_run_on_an_unstampable_library_skips_persistence(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The fail-closed price, pinned: a library this process cannot stamp is not
    provably safe from other writers, so the run happens and its state does not."""
    entry = _cmd("frozenrun")
    _strip_id_line(entry.slug)

    def _denied(*_a: object, **_k: object) -> None:
        raise OSError(30, "Read-only file system", "meta.toml")

    monkeypatch.setattr(store, "_write_meta_and_row", _denied)
    monkeypatch.setattr(flows, "execute", lambda *_a, **_k: flows.RunOutcome(0))
    result = runner.invoke(cli.app, ["run", "frozenrun", "--no-input"])
    assert result.exit_code == 0, result.output
    assert not (values_dir() / f"{entry.slug}.toml").exists()


async def test_a_stale_library_row_stops_the_edit_lane_before_any_editor(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The editor must never even OPEN for a ghost row: the claim stops the lane
    before a draft exists."""
    sessions: list[Path] = []
    monkeypatch.setattr(editor, "open_in_editor", sessions.append)
    _shell(tmp_path, "editghost")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        _reincarnate("editghost", lambda: _shell(tmp_path, "editghost"))
        app.action_edit()
        await pilot.pause()
    assert sessions == []


async def test_a_stale_row_stops_the_reconcile_picker_too(tmp_path: Path) -> None:
    src = tmp_path / "p.prompt.md"
    src.write_text("Ask {{x}}\n", encoding="utf-8")
    old = store.add_prompt(src, name="pghost", managed=[])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        _reincarnate("pghost", lambda: store.add_prompt(src, name="pghost", managed=[]))
        assert app._offer_prompt_reconcile(old) is True  # the claim owns the story
        await pilot.pause()
        assert app.screen is app.screen_stack[0]  # no picker for the stranger
    assert store.resolve("pghost").meta.params is None


def test_edit_draft_paths_are_unique_suffixed_and_deep_creating(tmp_path: Path) -> None:
    """The staging contract: per-session unique names (a kept rescue draft is never
    clobbered by the next session), the copy's own suffix (editors key syntax off it),
    and a drafts dir minted on demand however deep the data dir is."""
    a = editor.edit_draft_path("s", ".zsh")
    b = editor.edit_draft_path("s", ".zsh")
    assert a != b
    assert a.suffix == ".zsh"
    assert a.name.startswith("edit-s-")
    assert a.parent.name == "drafts"


def test_edit_draft_path_creates_the_drafts_dir_from_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "deep" / "never" / "made"))
    draft = editor.edit_draft_path("s", ".sh")
    assert draft.exists()


def test_commit_copy_edit_returns_the_committed_entry(tmp_path: Path) -> None:
    entry = _shell(tmp_path, "commitret")
    committed = store.commit_copy_edit(entry.slug, b"#!/bin/sh\nret\n", expected_id=entry.meta.id)
    assert committed.slug == entry.slug
    assert committed.meta.id == entry.meta.id
    assert committed.dir == entry.dir


def test_claim_identity_verifies_unlocked_when_the_lock_cannot_exist(tmp_path: Path) -> None:
    """A .locks dir nobody can create (a read-only data dir) still gets an EXACT
    verification — unserialized, on a library this process cannot mutate anyway —
    and a reissued slug still refuses through it."""
    entry = _cmd("nolock")

    def _denied(_path: Path, **_kwargs: object):
        raise OSError(30, "Read-only file system", ".locks")

    with pytest.MonkeyPatch.context() as mp:
        mp.setattr(store, "advisory_file_lock", _denied)
        held = store.claim_identity(entry)
    assert held.meta.id == entry.meta.id

    new = _reincarnate("nolock", lambda: _cmd("nolock"))
    with pytest.MonkeyPatch.context() as mp:
        mp.setattr(store, "advisory_file_lock", _denied)
        with pytest.raises(store.StaleEntryError):
            store.claim_identity(entry)
    assert store.resolve(new.slug).meta.id == new.meta.id


async def test_a_stale_library_row_stops_the_rerun_lane_too(tmp_path: Path) -> None:
    entry = _cmd("tuirerun")
    argstate.record_run(entry.slug, 0, at="2026-01-01T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        _reincarnate("tuirerun", lambda: _cmd("tuirerun"))
        app.action_rerun()
        await pilot.pause()
        assert app.screen is app.screen_stack[0]
    assert not (values_dir() / "tuirerun.toml").exists()
