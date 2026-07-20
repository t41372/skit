"""Mutation-kill tests for MenuApp._execute and MenuApp._has_drift (tui.py chunk 5).

These pin the run-execution glue's observable contract: the PendingRun the launcher
hands back in exit mode carries every piece of run material (and the real cwd threaded
into assembly); the in-TUI suspend path prints the run header, the drift banner, the
delivery transparency lines and the outcome banner — each flushed so it reaches the
terminal before the child process writes (the file is entirely about terminal handoff,
so an unflushed banner would interleave with the script's own output); a launch that
never starts prints an error line and lands a "couldn't launch" status; a nonzero exit
records a "failed (code N)" status; EOF at the "press Enter" prompt is swallowed rather
than crashing the workbench; and the lazy, mtime-keyed drift cache is trusted only while
fresh and reflects the plan's real drift.
"""

from __future__ import annotations

import contextlib
from pathlib import Path

import pytest
from textual.widgets import Static

from skit import config, flows, launcher, store, tui
from skit.langs.python import metawriter
from skit.langs.registry import spec_for
from skit.params import ParamDecl


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


class _PrintRecorder:
    """Captures every print(*args, **kwargs) so a test can assert BOTH the emitted text
    and that it was flushed (flush=True is load-bearing on the terminal-handoff path)."""

    def __init__(self) -> None:
        self.calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def __call__(self, *args: object, **kwargs: object) -> None:
        self.calls.append((args, kwargs))

    @property
    def lines(self) -> list[str]:
        return [" ".join(str(a) for a in args) for args, _ in self.calls]

    def flush_for(self, needle: str) -> object:
        """The flush kwarg of the FIRST printed line containing `needle`."""
        for args, kwargs in self.calls:
            if needle in " ".join(str(a) for a in args):
                return kwargs.get("flush", "__no-flush-kwarg__")
        return "__line-not-printed__"


@pytest.fixture
def stay_suspend(monkeypatch: pytest.MonkeyPatch) -> None:
    """The in-TUI (after_run=stay) run path with the terminal ownership neutralized:
    a no-op suspend, so _execute runs its print/record body headless. Individual tests
    add the run_entry stub (or unlink the target) and the print recorder."""
    config.save_after_run("stay")
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())


# ---------------------------------------------------------------------------
# exit mode: the PendingRun handed back carries every piece of run material
# ---------------------------------------------------------------------------


async def test_exit_mode_pendingrun_carries_run_material_with_real_cwd(tmp_path):
    """Out-of-box (after_run=exit), _execute assembles then hands run_menu a PendingRun
    instead of running under suspend. That hand-off must carry the plan, the assembly, the
    values, the raw extra args and the show_drift flag intact — and the assembly must be
    built against the real invocation cwd (a `{cwd}` extra expands to it)."""
    config.save_after_run("exit")
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    plan = flows.FormPlan(source="none")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._execute(entry, plan, {"CITY": "Taipei"}, ["{cwd}"], show_drift=True)
        pending = app.return_value
    assert isinstance(pending, tui.PendingRun)
    assert pending.plan is plan  # not None (mutmut_27)
    assert isinstance(pending.asm, flows.Assembly)  # not None (mutmut_28)
    # cwd=Path.cwd() is threaded into assemble: the {cwd} extra expands to the live cwd,
    # not "None" (mutmut_6 replaces cwd with None -> str(None)).
    assert pending.asm.args == [str(Path.cwd())]
    assert pending.values == {"CITY": "Taipei"}  # not None (mutmut_29)
    assert pending.extra == ["{cwd}"]  # raw, unexpanded, not None (mutmut_30)
    assert pending.show_drift is True  # not None (mutmut_31)


# ---------------------------------------------------------------------------
# in-TUI success path: header / drift / transparency / outcome banners + status
# ---------------------------------------------------------------------------


async def test_success_run_prints_flushed_banners_and_records_finished(
    tmp_path, stay_suspend, monkeypatch
):
    """The suspend body prints, in order and each flushed: the `── Run <name> ──` header,
    the drift banner lines (show_drift=True), the delivery transparency lines (via the emit
    lambda), and the `✓ finished` outcome banner (the stay path repaints immediately —
    no Enter-wait since #14); a clean run then
    stamps the exact `Last: <name> ✓ finished` status."""
    monkeypatch.setattr(launcher, "run_entry", lambda *a, **k: 0)
    rec = _PrintRecorder()
    monkeypatch.setattr("builtins.print", rec)
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    plan = flows.FormPlan(source="none")
    plan.drift_lines = ["DRIFT-SENTINEL"]
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._execute(entry, plan, {}, [], show_drift=True)
        await pilot.pause()
        status = str(app.query_one("#status", Static).render())
    lines = rec.lines
    # run header (mutmut_40/42 value, 46/47 msgid)
    assert any("── Run a ──" in line for line in lines)
    assert rec.flush_for("── Run a ──") is True  # (mutmut_41/43/51)
    # drift banner (mutmut_53/55/56 flush; the loop actually reached it)
    assert any("DRIFT-SENTINEL" in line for line in lines)
    assert rec.flush_for("DRIFT-SENTINEL") is True
    # delivery transparency, printed through the emit lambda (mutmut_66/67/69 value)
    assert any("→" in line for line in lines)
    assert rec.flush_for("→") is True  # (mutmut_68/70/71)
    # outcome banner (mutmut_85/87 value)
    assert any("✓ finished" in line for line in lines)
    assert rec.flush_for("✓ finished") is True  # (mutmut_86/88/90)
    # recorded status, exact so an XX-wrapped / lowercased msgid is caught (mutmut_118/119)
    assert status == "Last: a ✓ finished"


async def test_nonzero_exit_records_failed_status(tmp_path, stay_suspend, monkeypatch):
    """A launched-but-nonzero run records the exact `Last: <name> ✗ failed (code N)`
    status (mutmut_124/125 wrap/lowercase that msgid)."""
    monkeypatch.setattr(launcher, "run_entry", lambda *a, **k: 3)
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    plan = flows.FormPlan(source="none")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._execute(entry, plan, {}, [])
        await pilot.pause()
        status = str(app.query_one("#status", Static).render())
    assert status == "Last: a ✗ failed (code 3)"


# ---------------------------------------------------------------------------
# in-TUI launch-failure path: error line + "couldn't launch" status
# ---------------------------------------------------------------------------


async def test_launch_failure_prints_flushed_error_and_couldnt_launch_status(
    tmp_path, stay_suspend, monkeypatch
):
    """When the script never launches (target gone), the suspend body prints a flushed
    `Error: …` line and, after the block, stamps the exact `Last: <name> ✗ couldn't launch`
    status without recording a phantom run."""
    rec = _PrintRecorder()
    monkeypatch.setattr("builtins.print", rec)
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    plan = flows.FormPlan(source="none")
    entry.script_path.unlink()  # the target is gone -> run_entry raises, code is None
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._execute(entry, plan, {}, [])
        await pilot.pause()
        status = str(app.query_one("#status", Static).render())
    lines = rec.lines
    # the error line only prints because `if outcome.code is None` held (mutmut_72),
    # and it starts with the exact `Error: ` msgid (mutmut_73/75 value, 79/80 msgid).
    assert any(line.startswith("Error: ") for line in lines)
    assert rec.flush_for("Error: ") is True  # (mutmut_74/76/84)
    assert status == "Last: a ✗ couldn't launch"  # (mutmut_97/98)


# ---------------------------------------------------------------------------
# _has_drift: the guard and the lazy mtime-keyed cache
# ---------------------------------------------------------------------------


def test_has_drift_is_false_and_never_stats_a_missing_script(tmp_path):
    """The guard short-circuits to False (never touching stat/plan) when the script file is
    gone. mutmut_8 flips the guard's `return False` to True; mutmut_3 turns the guard's last
    `or` into `and`, which lets a missing-file entry fall through to `stat()` and crash."""
    python_spec = spec_for("python")  # the guard's discrimination needs a live analyzer
    assert python_spec is not None
    assert python_spec.analyzer is not None
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    entry.script_path.unlink()
    app = tui.MenuApp()
    assert app._has_drift(entry) is False


def test_has_drift_trusts_a_fresh_mtime_matching_cache(tmp_path):
    """A cache entry whose stored mtime equals the file's current mtime is trusted verbatim —
    the expensive plan/reconcile is skipped and the cached verdict returned. Planting True for
    a script whose real drift is False proves the cache is honored: mutmut_9 (mtime=None),
    mutmut_10 (cached=None), mutmut_11 (get(None)), mutmut_14 (cached[1]) and mutmut_15 (!=)
    all miss the plant and recompute the real False."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="clean")
    mtime = entry.script_path.stat().st_mtime
    app = tui.MenuApp()
    app._drift_cache[entry.slug] = (mtime, True)
    assert app._has_drift(entry) is True


def test_has_drift_reflects_the_plans_real_drift(tmp_path):
    """With an empty cache, drift is `bool(plan.drift_lines)`: a script that declares a param
    (GONE) it no longer defines drifts, so _has_drift is True. mutmut_20 replaces the plan's
    drift lines with bool(None), collapsing every script to no-drift."""
    drifted = metawriter.write_params(
        "CITY = 'x'\nprint(CITY)\n",
        [
            ParamDecl(name="CITY", binding="const", type="str"),
            ParamDecl(name="GONE", binding="const", type="str"),
        ],
    )
    entry = store.add_python(_py(tmp_path, drifted, "drifty.py"), name="drifty")
    app = tui.MenuApp()
    assert app._has_drift(entry) is True
