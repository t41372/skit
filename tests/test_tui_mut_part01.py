"""Mutation-kill tests for src/skit/tui.py (chunk 1/10).

Covers the pure recency/fuzzy helpers and the exit-mode run tail:

* ``_fuzzy_match`` — the search box's subsequence matcher (ordered, position-advancing).
* ``_activity_key`` — the Library's recency sort key (last run vs. added time).
* ``_finish_run`` — run_menu's exit-after-run tail, printed on the plain terminal once
  the TUI is gone: the run banner, drift lines, transparency emit, error line, and the
  docker-convention exit code. The banner/drift/error/emit prints must be flushed so
  they reach the terminal before (and are ordered against) the subprocess's own output.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import argstate, flows, launcher, store, tui
from skit.models import Entry, ScriptMeta


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# _fuzzy_match — ordered, position-advancing subsequence match
# ---------------------------------------------------------------------------


def test_fuzzy_match_requires_ordered_subsequence():
    """The match must respect order: "zy" is NOT a subsequence of "xyz" (the y precedes
    the z), so a matcher that resets its scan position back to 1 after each hit — instead
    of advancing past the char it just consumed — would wrongly accept it."""
    assert tui._fuzzy_match("zy", "xyz") is False
    assert tui._fuzzy_match("yz", "xyz") is True  # the genuinely-ordered subsequence


def test_fuzzy_match_advances_past_each_consumed_char():
    """A repeated query char must find a LATER occurrence, not re-match the same one:
    "aa" is not a subsequence of "ba" (only one 'a'). A matcher that ignores the running
    position and always searches from the start would re-find the single 'a' twice."""
    assert tui._fuzzy_match("aa", "ba") is False
    assert tui._fuzzy_match("aa", "aba") is True  # two 'a's → genuinely matches


# ---------------------------------------------------------------------------
# _activity_key — recency sort key
# ---------------------------------------------------------------------------


def test_activity_key_uses_added_time_when_never_run(tmp_path):
    """A freshly added script has never run (last_run empty), so its added time is the
    recency key that surfaces it in the list — the added time must not be discarded."""
    meta = ScriptMeta(name="fresh", kind="python", added_at="2026-05-05T00:00:00+00:00")
    entry = Entry(slug="mut-actkey-fresh", meta=meta, dir=tmp_path)
    assert argstate.load_state(entry.slug)["last_run"] == {}  # never run
    assert tui._activity_key(entry) == "2026-05-05T00:00:00+00:00"


def test_activity_key_uses_last_run_when_no_added_time(tmp_path):
    """With no recorded added time, the last-run stamp is the recency key — the empty-string
    fallback must be empty (a non-empty sentinel would outrank a real timestamp and pin the
    entry to the top of the list forever)."""
    meta = ScriptMeta(name="old", kind="python", added_at="")
    entry = Entry(slug="mut-actkey-old", meta=meta, dir=tmp_path)
    argstate.record_run(entry.slug, 0, at="2026-05-05T00:00:00+00:00")
    assert tui._activity_key(entry) == "2026-05-05T00:00:00+00:00"


# ---------------------------------------------------------------------------
# _finish_run — the exit-after-run tail on the plain terminal
# ---------------------------------------------------------------------------


@pytest.fixture
def fake_launch(monkeypatch):
    """Replace the real process launch so flows.execute (and thus _finish_run) runs its
    banner/transparency/record pipeline without spawning a subprocess."""
    state: dict[str, object] = {}

    def fake_run(
        entry,
        extra_args=None,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
        runner=None,
        prepared=None,
    ):
        state["ran"] = True
        return state.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    return state


def test_finish_run_prints_banner_drift_and_transparency_all_flushed(
    tmp_path, fake_launch, monkeypatch
):
    """The successful exit tail: the exact "── Run <name> ──" banner, the drift lines, and
    the transparency emit each reach the terminal, and each is flushed=True so it lands
    before the script's own output rather than sitting in a buffer. The run is recorded so
    the next launch can r-rerun it, and the script's exit code passes through."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="banner")
    plan = flows.plan_for_entry(entry)
    plan.drift_lines = ["DRIFT-XYZ"]
    asm = flows.assemble(plan, {}, [], cwd=tmp_path)
    fake_launch["code"] = 0

    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []
    monkeypatch.setattr("builtins.print", lambda *a, **k: calls.append((a, k)))

    pending = tui.PendingRun(entry, plan, asm, {}, [], show_drift=True)
    assert tui._finish_run(pending) == 0  # the script's own code passes through

    # The run banner: exact framing + msgid, flushed.
    header = next(((a, k) for a, k in calls if a and "──" in str(a[0])), None)
    assert header is not None
    assert "── Run banner ──" in str(header[0][0])
    assert header[1].get("flush") is True

    # The drift banner line prints on the exit path (where the form's usual banner is gone),
    # flushed.
    drift = next(((a, k) for a, k in calls if a and str(a[0]) == "DRIFT-XYZ"), None)
    assert drift is not None
    assert drift[1].get("flush") is True

    # The transparency emit ("→ …") is delivered verbatim through the emit callback, flushed.
    trans = next(((a, k) for a, k in calls if a and str(a[0]).startswith("→")), None)
    assert trans is not None
    assert trans[1].get("flush") is True

    # The run is recorded so r-rerun has context next launch.
    assert argstate.load_state(entry.slug)["last_run"]["exit"] == 0


def test_finish_run_launch_failure_prints_error_and_uses_docker_code(tmp_path, monkeypatch):
    """A launch that never starts after teardown: the "Error: …" line is printed (flushed),
    the docker-convention exit code for a missing target is 127, and no phantom run is
    recorded."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="ghost")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {}, [], cwd=tmp_path)
    entry.script_path.unlink()  # the target is gone → FAIL_MISSING → 127

    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []
    monkeypatch.setattr("builtins.print", lambda *a, **k: calls.append((a, k)))

    pending = tui.PendingRun(entry, plan, asm, {}, [], show_drift=False)
    assert tui._finish_run(pending) == 127  # docker code for a missing target

    err = next(((a, k) for a, k in calls if a and str(a[0]).startswith("Error:")), None)
    assert err is not None  # the failure is named, not swallowed
    assert err[1].get("flush") is True  # flushed before the shell prompt returns

    assert argstate.load_state(entry.slug)["last_run"] == {}  # nothing ran, nothing recorded


def test_finish_run_unmapped_failure_falls_back_to_generic_skit_error_code(tmp_path, monkeypatch):
    """Defensive default: a failure name outside FAILURE_EXIT_CODES maps to 125, the generic
    skit-error docker code — never None (would break the int contract) and never another
    code. Reached by handing _finish_run an outcome carrying an unrecognized failure."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="weird")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {}, [], cwd=tmp_path)
    monkeypatch.setattr("builtins.print", lambda *a, **k: None)
    monkeypatch.setattr(
        flows,
        "execute",
        lambda *a, **k: flows.RunOutcome(None, "totally-unknown-failure", "boom"),
    )
    pending = tui.PendingRun(entry, plan, asm, {}, [], show_drift=False)
    assert tui._finish_run(pending) == 125
