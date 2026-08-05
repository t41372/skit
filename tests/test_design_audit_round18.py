"""Behavior coverage for the design-audit round-18 fixes (the launch-boundary review).

Round 17 authorized the writes; this review showed the EXECUTION itself was still
address-bound, and two CAS gaps remained. Closed here:

R. The launch is identity-gated AND materialized under the entry lock: flows.execute
   re-verifies WHO the entry is, reads/renders/injects the payload, and SPAWNS the
   child inside the lock (launcher.start_entry — the OS opens the program before
   anything can swap it), waiting outside (launcher.finish_entry). A form that
   outlived its entry refuses with "nothing was run" — the stranger's program is
   never executed. An unverifiable meta refuses too; only symmetric unstamped pairs
   may still launch (an unstampable library deserves to run), and they persist
   nothing.
S. An idless handle is claimed by CONTENT, not by blank-vs-blank: an older skit's
   adds write no id, so `"" == ""` alone can bless a reincarnation. remove() refuses
   an empty expectation outright — both faces claim (stamping legacy metas) BEFORE
   their confirmation ask.
T. commit_copy_edit is a double CAS: entry identity AND source version. A
   `write_source_params` landing while the editor sat open refuses the whole-file
   replace instead of silently erasing it — the draft survives, the refusal says why.
U. The CLI's post-edit prompt reconcile claims for its picker hold and authorizes its
   managed-list write, closing the last slug-only writer in the edit pipeline.
"""

from __future__ import annotations

import hashlib
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import Any, cast

import pytest
from typer.testing import CliRunner

from conftest import patch_run_entry
from skit import cli, editor, flows, launcher, store, tui
from skit.atomic import try_advisory_file_lock
from skit.params import ParamDecl
from skit.paths import values_dir

runner = CliRunner()

NOT_RUN = "nothing was run"


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


def _strip_id_line(slug: str) -> None:
    path = store.scripts_dir() / slug / "meta.toml"
    kept = [
        line
        for line in path.read_text(encoding="utf-8").splitlines()
        if not line.startswith("id = ")
    ]
    path.write_text("\n".join(kept) + "\n", encoding="utf-8")


def _spy_launch(monkeypatch: pytest.MonkeyPatch) -> list[object]:
    launched: list[object] = []

    def _spy(entry: store.Entry, _extra: object = None, **_k: object) -> int:
        launched.append(entry)
        return 0

    patch_run_entry(monkeypatch, _spy)
    return launched


# ==========================================================================
# R. the launch gate — the stranger's program is never executed
# ==========================================================================


def test_a_form_that_outlived_its_entry_launches_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The review's core scenario at the delivery chokepoint: claim, hold (the form),
    reincarnate, submit. The gate refuses BEFORE the spawn — the new owner's program
    is never opened, let alone run."""
    old = store.claim_identity(_cmd("gate18"))
    plan = flows.plan_for_entry(old)
    launched = _spy_launch(monkeypatch)
    store.remove("gate18")
    _cmd("gate18")
    outcome = flows.execute(old, plan, flows.Assembly(), emit=lambda _l: None)
    assert outcome.code is None
    assert outcome.message == (
        "gate18 changed or was removed while the form was open — nothing was run."
    )
    assert launched == []  # the spawn never happened


def test_a_removed_entry_launches_nothing_either(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    old = store.claim_identity(_cmd("gone18"))
    plan = flows.plan_for_entry(old)
    launched = _spy_launch(monkeypatch)
    store.remove("gone18")
    outcome = flows.execute(old, plan, flows.Assembly(), emit=lambda _l: None)
    assert outcome.code is None
    assert NOT_RUN in outcome.message
    assert launched == []


def test_an_unreadable_meta_refuses_the_launch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Cannot verify, will not exec: a meta that rots between the claim and the
    submit refuses with the corruption named, instead of running whatever the path
    holds now."""
    entry = store.claim_identity(_cmd("rot18"))
    plan = flows.plan_for_entry(entry)
    launched = _spy_launch(monkeypatch)
    meta = store.scripts_dir() / entry.slug / "meta.toml"
    meta.write_text("not toml [[[", encoding="utf-8")
    outcome = flows.execute(entry, plan, flows.Assembly(), emit=lambda _l: None)
    assert outcome.code is None
    assert "doctor" in outcome.message  # the corruption message, verbatim from the store
    assert launched == []


def test_a_symmetric_unstamped_pair_still_launches_but_persists_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """An unstampable library cannot be verified — refusing it would brick every
    legacy run on read-only shares — so it runs, and persistence_target keeps its own
    rule: no stamped identity, no state."""
    entry = _cmd("blank18")
    _strip_id_line(entry.slug)
    held = store.resolve(entry.slug)
    plan = flows.plan_for_entry(held)
    launched = _spy_launch(monkeypatch)
    outcome = flows.execute(held, plan, flows.Assembly(), emit=lambda _l: None)
    assert outcome.code == 0
    assert len(launched) == 1
    flows.save_after_run(held, plan, {}, [], 0, at="2026-02-01T00:00:00+00:00", extra_raw=False)
    assert not (values_dir() / f"{held.slug}.toml").exists()


def test_the_spawn_happens_under_the_entry_lock_and_the_wait_outside(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The materialization contract: at spawn time the entry lock is HELD (nothing
    can swap the program between the identity check and the exec); at wait time it is
    RELEASED (no lock outlives a child)."""
    entry = store.claim_identity(_cmd("lock18"))
    plan = flows.plan_for_entry(entry)
    states: dict[str, bool] = {}

    class _Child:
        def wait(self) -> int:
            with try_advisory_file_lock(store.entry_lock_path(entry.slug)) as acquired:
                states["wait_lock_free"] = acquired
            return 0

        def kill(self) -> None:  # pragma: no cover — only the interrupt path
            pass

    def _start(*_a: object, **_k: object) -> _Child:
        with try_advisory_file_lock(store.entry_lock_path(entry.slug)) as acquired:
            states["spawn_lock_free"] = acquired
        return _Child()

    monkeypatch.setattr(launcher, "start_entry", _start)
    outcome = flows.execute(entry, plan, flows.Assembly(), emit=lambda _l: None)
    assert outcome.code == 0
    assert states == {"spawn_lock_free": False, "wait_lock_free": True}


async def test_a_tui_submit_after_a_reincarnation_runs_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """End to end on the workbench: the form is open, the entry is reincarnated, the
    user submits — the banner says nothing ran, and the spy proves it."""
    import contextlib

    from skit.tui_form import RunFormScreen

    @contextlib.contextmanager
    def _noop(_self: tui.MenuApp):
        yield

    monkeypatch.setattr(tui.MenuApp, "suspend", _noop)
    launched = _spy_launch(monkeypatch)
    _cmd("form18")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        store.remove("form18")
        _cmd("form18")
        screen.action_submit()
        await pilot.pause()
    assert launched == []


# ==========================================================================
# S. idless handles — content is the claim, and no blank deletion
# ==========================================================================


def test_an_idless_swap_with_different_content_refuses_the_claim(tmp_path: Path) -> None:
    """The cross-version reincarnation, at the claim: an older skit re-adds the slug
    without an id — blank-vs-blank must not bless it when the CONTENT says it is a
    different entry."""
    entry = _cmd("swap18", "echo one")
    _strip_id_line(entry.slug)
    held = store.resolve(entry.slug)
    store.remove("swap18")
    reborn = _cmd("swap18", "echo two")  # a different program under the same name
    _strip_id_line(reborn.slug)
    with pytest.raises(store.StaleEntryError):
        store.claim_identity(held)
    assert store.resolve("swap18").meta.id == ""  # the stranger was not stamped


def test_an_idless_twin_swap_is_claimable_by_content(tmp_path: Path) -> None:
    """The documented residual: a byte-identical idless twin (same name, kind,
    template, timestamps) is operationally the same entry — there is nothing left to
    protect one from the other with, so the claim proceeds and stamps it."""
    entry = _cmd("twin18")
    _strip_id_line(entry.slug)
    held = store.resolve(entry.slug)
    claimed = store.claim_identity(held)
    assert claimed.meta.id


def test_remove_refuses_an_empty_expectation(tmp_path: Path) -> None:
    entry = _cmd("noblank18")
    with pytest.raises(store.StaleEntryError):
        store.remove(entry.slug, expected_id="")
    assert store.resolve(entry.slug).meta.id == entry.meta.id  # still there


def test_an_idless_readd_during_the_remove_ask_refuses(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The lane claims (stamping the legacy meta) BEFORE the ask, so an old-version
    idless re-add during the ask meets a STAMPED expectation and refuses."""
    entry = _cmd("askless18")
    _strip_id_line(entry.slug)

    def _race(_message: str) -> None:
        store.remove("askless18")
        reborn = _cmd("askless18")
        _strip_id_line(reborn.slug)  # the old version's add: no id

    monkeypatch.setattr(cli, "_require_yes", lambda *_a, **_k: None)
    monkeypatch.setattr(cli, "_confirm_destructive", _race)
    result = runner.invoke(cli.app, ["remove", "askless18"])
    assert result.exit_code == 125
    assert "changed while this edit was underway" in result.output
    assert store.resolve("askless18") is not None  # the re-added entry survived


# ==========================================================================
# T. the source-version CAS
# ==========================================================================


def test_commit_copy_edit_refuses_a_source_that_moved_on(tmp_path: Path) -> None:
    entry = _shell(tmp_path, "cas18")
    base = hashlib.sha256(entry.script_path.read_bytes()).hexdigest()
    store.write_source_params(entry.slug, [ParamDecl(name="MODE", delivery="env")])
    moved_on = entry.script_path.read_bytes()
    with pytest.raises(store.StaleEntryError) as refusal:
        store.commit_copy_edit(
            entry.slug,
            b"#!/bin/sh\nstale draft\n",
            expected_id=entry.meta.id,
            expected_source_hash=base,
        )
    assert str(refusal.value) == (
        "cas18's source changed while the editor was open — reopen it and try again."
    )
    assert entry.script_path.read_bytes() == moved_on  # the other writer's work survives


def test_commit_copy_edit_lands_on_a_matching_source(tmp_path: Path) -> None:
    entry = _shell(tmp_path, "casok18")
    base = hashlib.sha256(entry.script_path.read_bytes()).hexdigest()
    store.commit_copy_edit(
        entry.slug,
        b"#!/bin/sh\nedited\n",
        expected_id=entry.meta.id,
        expected_source_hash=base,
    )
    assert entry.script_path.read_bytes() == b"#!/bin/sh\nedited\n"


def test_a_cli_edit_racing_a_schema_commit_keeps_the_draft_and_the_schema(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The lost-update scenario end to end: while the editor sits open, another
    writer commits a [tool.skit] block. The whole-file replace refuses; the block
    survives; the user's edit waits in the draft."""
    entry = _shell(tmp_path, "lost18")

    def _session(path: Path) -> int:
        path.write_bytes(b'#!/bin/sh\necho "mine"\n')
        store.write_source_params(entry.slug, [ParamDecl(name="MODE", delivery="env")])
        return 0

    monkeypatch.setattr(editor, "open_in_editor", _session)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["edit", "lost18"])
    assert result.exit_code == 125
    assert "source changed while the editor was open" in result.output
    assert "Your edit was kept at:" in result.output
    assert b"[tool.skit]" in store.resolve("lost18").script_path.read_bytes()
    drafts = list((store.scripts_dir().parent / "drafts").glob("edit-lost18-*.sh"))
    assert [d.read_bytes() for d in drafts] == [b'#!/bin/sh\necho "mine"\n']


# ==========================================================================
# U. the CLI post-edit reconcile is claimed and authorized
# ==========================================================================


def test_a_reconcile_pick_racing_a_reincarnation_refuses(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    src = tmp_path / "p.prompt.md"
    src.write_text("Ask {{x}}\n", encoding="utf-8")
    entry = store.add_prompt(src, name="rec18", managed=[])

    def _picker(new: list[str], _preselected: set[str]) -> set[str]:
        store.remove("rec18")
        store.add_prompt(src, name="rec18", managed=[])
        return set(new)

    from skit import tui_add

    monkeypatch.setattr(cli, "_wants_tui_form", lambda: True)
    monkeypatch.setattr(tui_add, "run_candidate_picker", _picker)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def _session(path: Path) -> int:
        text = path.read_text(encoding="utf-8")
        path.write_text(text + "And {{extra}}\n", encoding="utf-8")
        return 0

    monkeypatch.setattr(editor, "open_in_editor", _session)
    result = runner.invoke(cli.app, ["edit", "rec18"])
    assert result.exit_code == 125
    assert "changed while this edit was underway" in result.output
    assert store.resolve("rec18").meta.params is None  # the new owner keeps its own list
    assert entry.meta.name == "rec18"


def test_a_reincarnation_before_the_remove_claim_refuses(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The resolve→claim window on remove itself: the claim (not just the deletion)
    refuses the reissued slug, through the same seam every other pre-claim race
    uses."""
    _cmd("preclaim18")

    def _race(*_a: object, **_k: object) -> None:
        store.remove("preclaim18")
        _cmd("preclaim18")

    monkeypatch.setattr(cli, "_require_yes", _race)
    result = runner.invoke(cli.app, ["remove", "preclaim18", "--yes"])
    assert result.exit_code == 125
    assert "changed while this edit was underway" in result.output
    assert store.resolve("preclaim18") is not None  # the new owner survived


def test_finish_entry_kills_the_child_when_the_wait_is_interrupted() -> None:
    """subprocess.run's own discipline, preserved across the split: an interrupted
    wait kills the child before propagating — a split launch never leaks a process."""

    class _Child:
        def __init__(self) -> None:
            self.killed = False
            self.waits = 0

        def wait(self) -> int:
            self.waits += 1
            if self.waits == 1:
                raise KeyboardInterrupt
            return 130

        def kill(self) -> None:
            self.killed = True

    child = _Child()
    with pytest.raises(KeyboardInterrupt):
        launcher.finish_entry(cast("subprocess.Popen[bytes]", child))
    assert child.killed
    assert child.waits == 2  # the post-kill reap ran


async def test_a_stale_library_row_stops_the_remove_lane_before_the_modal(
    tmp_path: Path,
) -> None:
    from skit.tui import ConfirmRemove

    _cmd("tuirm18")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        store.remove("tuirm18")
        new = _cmd("tuirm18")
        app.action_remove()
        await pilot.pause()
        assert not isinstance(app.screen, ConfirmRemove)  # the claim stopped the lane
    assert store.resolve(new.slug).meta.id == new.meta.id


# ==========================================================================
# exact copy and forwarding pins for the launch pipeline
# ==========================================================================


def test_the_removed_refusal_copy_is_exact(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    old = store.claim_identity(_cmd("gonecopy18"))
    plan = flows.plan_for_entry(old)
    _spy_launch(monkeypatch)
    store.remove("gonecopy18")
    outcome = flows.execute(old, plan, flows.Assembly(), emit=lambda _l: None)
    assert outcome.message == (
        "gonecopy18 changed or was removed while the form was open — nothing was run."
    )
    assert flows.failure_reason(outcome) is not None


def test_the_launch_refusal_maps_to_the_not_found_exit(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """End to end through the CLI: a post-claim reincarnation (injected at the
    assembly seam) exits 127 — the same code a vanished script has always earned."""
    _cmd("mapped18")
    real_assemble = flows.assemble

    def _race(
        plan: flows.FormPlan,
        values: Mapping[str, str],
        extra: list[str],
        **kwargs: Any,
    ) -> flows.Assembly:
        asm = real_assemble(plan, values, extra, **kwargs)
        store.remove("mapped18")
        _cmd("mapped18")
        return asm

    monkeypatch.setattr(flows, "assemble", _race)
    _spy_launch(monkeypatch)
    result = runner.invoke(cli.app, ["run", "mapped18", "--no-input"])
    assert result.exit_code == 127
    assert NOT_RUN in result.output


def test_execute_forwards_the_full_launch_payload_to_the_spawn(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The spawn receives exactly what the pipeline assembled: entry, tail, values,
    cwd, override and overlay — dropping any one of them is a different launch."""
    entry = store.claim_identity(_cmd("fwd18", "echo {m}"))
    store.write_parameters(entry.slug, [ParamDecl(name="N", delivery="env")])
    entry = store.resolve(entry.slug)
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {"m": "x", "N": "5"}, ["--tail"], cwd=tmp_path, env={})
    seen: dict[str, object] = {}

    def _spy(spied: store.Entry, extra: object = None, **kwargs: object) -> int:
        seen["entry"] = spied
        seen["extra"] = extra
        seen.update(kwargs)
        return 0

    patch_run_entry(monkeypatch, _spy)
    marker = cast("Any", object())  # accept-and-ignore at the strategy, forwarded at the seam
    outcome = flows.execute(
        entry, plan, asm, emit=lambda _l: None, invoke_cwd=tmp_path, runner=marker
    )
    assert outcome.code == 0
    spied_entry = seen["entry"]
    assert isinstance(spied_entry, store.Entry)
    assert spied_entry.slug == entry.slug
    assert seen["extra"] == ["--tail"]
    assert seen["values"] == {"m": "x"}
    assert seen["invoke_cwd"] == tmp_path
    assert seen["script_override"] is None
    assert seen["env_overlay"] == {"N": "5"}
    assert seen["runner"] is marker


def test_execute_forwards_the_prompt_prepare_kwargs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The prompt lane's prepare gets the same fidelity: the snapshot is built from
    the run's own cwd and values, then handed to the spawn as-is."""
    src = tmp_path / "p.prompt.md"
    src.write_text("Ask {{x}}\n", encoding="utf-8")
    entry = store.claim_identity(store.add_prompt(src, name="prep18"))
    seen: dict[str, object] = {}
    sentinel = launcher.PreparedLaunch(
        payload=cast("Any", None),
        cwd=tmp_path,
        safe_display="display",
        prompt_runner=None,
        warning="",
    )

    def _prepare(spied: store.Entry, extra: object = None, **kwargs: object):
        seen["entry"] = spied
        seen["extra"] = extra
        seen.update(kwargs)
        return sentinel

    monkeypatch.setattr(launcher, "prepare_entry", _prepare)
    prepared_seen: dict[str, object] = {}

    def _start(_entry: store.Entry, _extra: object = None, **kwargs: object):
        prepared_seen["prepared"] = kwargs.get("prepared")

        class _C:
            def wait(self) -> int:
                return 0

            def kill(self) -> None:  # pragma: no cover
                pass

        return _C()

    monkeypatch.setattr(launcher, "start_entry", _start)
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {"x": "v"}, [], cwd=tmp_path, env={})
    outcome = flows.execute(entry, plan, asm, emit=lambda _l: None, invoke_cwd=tmp_path)
    assert outcome.code == 0
    spied_entry = seen["entry"]
    assert isinstance(spied_entry, store.Entry)
    assert spied_entry.slug == entry.slug
    assert seen["invoke_cwd"] == tmp_path
    assert seen["values"] == asm.command_values
    assert seen["runner"] is None
    assert prepared_seen["prepared"] is sentinel  # the snapshot travels, never a rebuild


def test_stale_refusal_copy_is_exact_at_the_store(tmp_path: Path) -> None:
    old = _cmd("copy18", "echo one")
    _strip_id_line(old.slug)
    held = store.resolve(old.slug)
    store.remove("copy18")
    reborn = _cmd("copy18", "echo two")
    _strip_id_line(reborn.slug)
    with pytest.raises(store.StaleEntryError) as refusal:
        store.claim_identity(held)
    assert str(refusal.value) == (
        "copy18 changed while this edit was underway — reopen it and try again."
    )
    with pytest.raises(store.StaleEntryError) as blank:
        store.remove("copy18", expected_id="")
    assert str(blank.value) == (
        "copy18 changed while this edit was underway — reopen it and try again."
    )


async def test_the_fail_closed_claim_status_is_exact(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from textual.widgets import Static

    _cmd("exact18")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()

        def _refuse(_entry: store.Entry) -> store.Entry:
            raise store.CorruptEntryError("meta rotted mid-click")

        monkeypatch.setattr(store, "claim_identity", _refuse)
        assert app._claimed(store.resolve("exact18")) is None
        status = str(app.query_one("#status", Static).render())
        assert status == "Error: meta rotted mid-click"


async def test_a_tui_edit_racing_a_schema_commit_keeps_both_sides(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The TUI face of the source CAS: the schema another writer committed survives,
    and the user's edit waits in the draft named on the status line."""
    import contextlib

    from textual.widgets import Static

    entry = _shell(tmp_path, "tuilost18")

    @contextlib.contextmanager
    def _noop(_self: tui.MenuApp):
        yield

    def _session(path: Path) -> int:
        path.write_bytes(b'#!/bin/sh\necho "mine"\n')
        store.write_source_params(entry.slug, [ParamDecl(name="MODE", delivery="env")])
        return 0

    monkeypatch.setattr(tui.MenuApp, "suspend", _noop)
    monkeypatch.setattr(editor, "open_in_editor", _session)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        status = str(app.query_one("#status", Static).render())
        assert "source changed while the editor was open" in status
        assert "Your edit was kept at:" in status
    assert b"[tool.skit]" in store.resolve("tuilost18").script_path.read_bytes()


def test_the_remove_ask_names_the_entry(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """The confirmation question is about a NAMED thing — an unnamed "are you sure?"
    is not a question anyone can answer."""
    _cmd("named18")
    asked: list[str] = []
    monkeypatch.setattr(cli, "_require_yes", lambda *_a, **_k: None)
    monkeypatch.setattr(cli, "_confirm_destructive", asked.append)
    result = runner.invoke(cli.app, ["remove", "named18"])
    assert result.exit_code == 0, result.output
    assert len(asked) == 1
    assert "named18" in asked[0]


def test_the_amp_one_shot_warning_is_exact(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """The amp runner's one-shot caveat, byte for byte — the message is the user's
    only warning that their 'interactive session' will not be one."""
    from skit import config

    src = tmp_path / "p.prompt.md"
    src.write_text("Hello\n", encoding="utf-8")
    entry = store.claim_identity(store.add_prompt(src, name="amp18"))
    amp_seed = next(r for r in config.PROMPT_RUNNER_SEEDS if r.name == "amp")
    sentinel = launcher.PreparedLaunch(
        payload=cast("Any", None),
        cwd=tmp_path,
        safe_display="display",
        prompt_runner=amp_seed,
        warning="",
    )
    monkeypatch.setattr(launcher, "prepare_entry", lambda *_a, **_k: sentinel)

    class _C:
        def wait(self) -> int:
            return 0

        def kill(self) -> None:  # pragma: no cover
            pass

    monkeypatch.setattr(launcher, "start_entry", lambda *_a, **_k: _C())
    warnings: list[str] = []
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {}, [], cwd=tmp_path, env={})
    outcome = flows.execute(
        entry, plan, asm, emit=lambda _l: None, warn=warnings.append, invoke_cwd=tmp_path
    )
    assert outcome.code == 0
    assert (
        "The built-in amp runner is one-shot: amp -x runs this prompt once "
        "and does not open an interactive session."
    ) in warnings


def test_execute_hands_the_runner_to_the_transparency_lines(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The transparency block must describe the run that actually happens — runner
    included, and the prompt's validated display exactly as prepared."""
    src = tmp_path / "p.prompt.md"
    src.write_text("Hello\n", encoding="utf-8")
    entry = store.claim_identity(store.add_prompt(src, name="lines18"))
    sentinel = launcher.PreparedLaunch(
        payload=cast("Any", None),
        cwd=tmp_path,
        safe_display="the-exact-display",
        prompt_runner=None,
        warning="",
    )
    monkeypatch.setattr(launcher, "prepare_entry", lambda *_a, **_k: sentinel)

    class _C:
        def wait(self) -> int:
            return 0

        def kill(self) -> None:  # pragma: no cover
            pass

    monkeypatch.setattr(launcher, "start_entry", lambda *_a, **_k: _C())
    seen: dict[str, object] = {}
    real_lines = flows.transparency_lines

    def _spy(spied_entry: store.Entry, asm: flows.Assembly, injected: object, **kwargs: Any):
        seen.update(kwargs)
        return real_lines(spied_entry, asm, cast("Path | None", injected), **kwargs)

    monkeypatch.setattr(flows, "transparency_lines", _spy)
    marker = object()
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {}, [], cwd=tmp_path, env={})
    outcome = flows.execute(
        entry,
        plan,
        asm,
        emit=lambda _l: None,
        invoke_cwd=tmp_path,
        runner=cast("Any", marker),
    )
    assert outcome.code == 0
    assert seen["runner"] is marker
    assert seen["validated_prompt_command"] == "the-exact-display"
