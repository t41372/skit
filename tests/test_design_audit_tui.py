"""Behavior coverage for the design-audit fixes (rounds 1, 2, 5 and 6), TUI half.

A. THE round-1 HIGH: the add-review panel wrote parameter blocks back with
   read_text/write_text, silently rewriting every line ending of the just-stored copy. Pinned
   at the SCREEN level — the helper-level round-trip lives in tests/test_design_audit_fixes.py,
   but this is the surface where the corruption actually shipped.
D. Extra-args provenance, TUI face: `r` replays a CLI-captured tail literally, the form's own
   tail expands, the marker rides through the deferred (exit-mode) save on PendingRun — and
   (round 5, the rule's third face) a PREFILLED tail submitted untouched keeps the provenance
   it was recorded with; only text the user actually edited is re-captured as form text. Round
   6 moved that verdict INTO the form (a real dirt bit on the extra row, against its own
   compose-time snapshot), which is the only place that can tell a cleared-and-retyped
   identical tail from an untouched one — or a concurrent CLI write from a user edit.
E. The Library detail pane's plan cache: keyed on both files' (mtime_ns, size) plus the
   reader-tool fingerprint, validated by a second stat so a racing write can't pin stale
   content under a fresh key AND (round 7) by a meta re-read for the half of the window the
   stats cannot see, caching snapshot fallbacks instead of a subprocess per cursor move,
   popped by edit and by the settings-close callback, _has_drift served off the same
   entry — and (E2) freshness owned by MenuApp._fresh, so the pane and both LAUNCH paths
   describe one generation of one record.
J. The as-is note on the TUI's two run faces, from the ONE msgid both faces share
   (flows.as_is_note, round 7 — tui._as_is_note was a second copy of the same source string).
F. AddReviewScreen._reader_modeled memoizes its probe on (text, reader tool) — for a reader
   kind each call is a synchronous subprocess and the panel must not freeze because a radio
   button was clicked, but a pwsh installed mid-session must not be invisible either.
G. The chord grammar repair: Ctrl+O meant three things on three sibling screens while README
   documents one. Choose variables → Ctrl+L (both screens), Preferences' squatters → Ctrl+G /
   Ctrl+Y, and every moved chord gets a positive pilot test plus a negative for the vacated one.
"""

from __future__ import annotations

import contextlib
from dataclasses import replace
from pathlib import Path

import pytest
from textual.widgets import Checkbox, Input, Select

from conftest import plan_cache_key, without_block
from skit import argstate, argv_text, config, flows, launcher, store, tui, tui_form
from skit.analysis import ArgSpec
from skit.langs.base import CliReader
from skit.langs.python import metawriter
from skit.langs.registry import spec_for
from skit.params import ParamDecl
from skit.tui_add import AddReviewScreen, PromptReviewScreen
from skit.tui_form import _EXTRA_KEY, FieldRow, RunFormScreen
from skit.tui_prefs import PreferencesScreen
from skit.tui_prompt import PromptCandidatePickerModal
from skit.tui_settings import ScriptSettingsScreen


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


@contextlib.contextmanager
def _noop_suspend():
    yield


@pytest.fixture
def quiet_run(monkeypatch):
    """Neutralize the terminal-ownership pieces of _execute and capture the launch, keeping
    the app alive afterwards (after_run=stay) so the test can inspect the Library."""
    config.save_after_run("stay")
    calls: dict[str, object] = {}

    def fake_run(
        entry,
        extra_args=None,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
        runner=None,
    ):
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    return calls


@pytest.fixture
def expansions(monkeypatch):
    """Every expand_extra decision _execute hands the assembler — the provenance rule's own
    output, before it becomes an argv."""
    seen: list[bool] = []
    real = flows.assemble

    def spy(plan, values, extra, **kwargs):
        seen.append(kwargs.get("expand_extra", True))
        return real(plan, values, extra, **kwargs)

    monkeypatch.setattr(tui.flows, "assemble", spy)
    return seen


def _py(tmp_path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _detail_text(app) -> str:
    """The detail pane's rendered text — "" when the size tier dropped the pane entirely.
    (#detail-body IS the Static: a descendant selector matches nothing and silently makes
    every assertion here vacuous.)"""
    return " ".join(str(s.render()) for s in app.query("#detail-body"))


# ==========================================================================
# A. the add-review panel's write-back keeps the copy's bytes
# ==========================================================================

_CRLF_SHELL = b'#!/usr/bin/env bash\r\nWIDTH=800\r\necho "$WIDTH"\r\n'


async def test_add_review_accept_keeps_a_crlf_copy_byte_exact(tmp_path):
    """THE round-1 HIGH regression, at the surface where it shipped: ticking a detected
    candidate on a CRLF shell script and accepting must leave the STORED copy CRLF, with every
    non-block byte exactly as it was. read_text/write_text (the old path here) re-expanded \\n
    to the host os.linesep, rewriting every line of a file skit had just been handed."""
    src = tmp_path / "crlf.sh"
    src.write_bytes(_CRLF_SHELL)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(src, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        review.query_one("#rv-name", Input).value = "crlfsh"
        review.query_one("#rv-cand-0", Checkbox).value = True  # tick WIDTH
        review.action_accept()
        await pilot.pause()

    stored = store.resolve("crlfsh").script_path.read_bytes()
    assert b"[tool.skit]" in stored  # the tick really was written
    assert b'name = "WIDTH"' in stored
    # CRLF end to end: stripping every CRLF pair must leave no bare terminator behind.
    stripped = stored.replace(b"\r\n", b"")
    assert b"\r" not in stripped
    assert b"\n" not in stripped
    # ...and every byte that isn't part of the inserted block is identical.
    assert without_block(stored, b"\r\n") == _CRLF_SHELL


async def test_add_review_accept_keeps_non_utf8_bytes(tmp_path):
    """The same write-back carries arbitrary bytes through: the panel READS with
    errors="replace" for display, so writing that text back would have baked U+FFFD over every
    one of them."""
    src = tmp_path / "raw.sh"
    original = b"#!/usr/bin/env bash\nWIDTH=800\nprintf '\xff\\n'\n"
    src.write_bytes(original)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        review = AddReviewScreen(src, kind="shell")
        app.push_screen(review)
        await pilot.pause()
        review.query_one("#rv-name", Input).value = "rawsh"
        review.query_one("#rv-cand-0", Checkbox).value = True
        review.action_accept()
        await pilot.pause()

    stored = store.resolve("rawsh").script_path.read_bytes()
    assert b"[tool.skit]" in stored
    assert b"\xff" in stored  # round-tripped exactly
    assert b"\xef\xbf\xbd" not in stored  # ...never replaced
    assert without_block(stored, b"\n") == original


# ==========================================================================
# D. extra-args provenance, TUI face
# ==========================================================================

MANAGED = 'CITY = "Taipei"\nprint(CITY)\n'


def _managed_entry(tmp_path, name: str = "j"):
    text = metawriter.write_params(
        MANAGED, [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")]
    )
    return store.add_python(_py(tmp_path, text), name=name)


async def test_rerun_replays_a_cli_captured_tail_literally(tmp_path, quiet_run):
    """A tail the user's shell already processed must NOT get a second token/glob pass just
    because the rerun happens from the TUI. Before the marker, `r` re-expanded every stored
    tail — rewriting exactly what the user had deliberately quoted. `{today}` is the
    discriminator here: a second pass would rewrite it, cwd or no cwd."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["out_{today}.txt", "*.png"])  # unmarked → literal
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_rerun()
        await pilot.pause()

    assert quiet_run["extra"] == ["out_{today}.txt", "*.png"]  # verbatim, both pieces
    assert argstate.load_state(entry.slug)["extra_args_raw"] is False


async def test_rerun_expands_a_form_captured_tail(tmp_path, quiet_run):
    """The complement: a tail the FORM captured is raw intent text, so `r` expands it — the
    same argv the run form would have produced."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["out_{today}.txt"], extra_args_raw=True)
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_rerun()
        await pilot.pause()

    (tail,) = quiet_run["extra"]
    assert tail != "out_{today}.txt"
    assert tail.startswith("out_20")
    assert tail.endswith(".txt")
    # Intent, not expansion, stays on disk — still marked raw for the next replay.
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["out_{today}.txt"]
    assert state["extra_args_raw"] is True


async def _submit_form(app, pilot, *, tail: str | None = None) -> str:
    """Open the launch menu, optionally REPLACE the prefilled extra tail, submit. Returns the
    tail text the form was prefilled with, so a test can pin the prefill it left untouched."""
    app.action_run()
    await pilot.pause()
    screen = app.screen
    assert isinstance(screen, RunFormScreen)
    extra_row = next(r for r in screen.query(FieldRow) if r.field.key == _EXTRA_KEY)
    prefilled = extra_row.value
    if tail is not None:
        extra_row.set_value(tail)
        await pilot.pause()
    screen.action_submit()
    await pilot.pause()
    return prefilled


async def test_form_submit_expands_its_tail_and_records_it_raw(tmp_path, quiet_run, expansions):
    """The form's extra field has no shell behind it, so text the user TYPES there is raw
    intent: its tokens expand on the way out and the tail is remembered MARKED, so `skit run`
    replaying it later expands it the same way. (Nothing was remembered, so the field started
    empty — every character in it is the user's.)"""
    entry = _managed_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        assert await _submit_form(app, pilot, tail="out_{today}.txt") == ""  # nothing prefilled

    (tail,) = quiet_run["extra"]
    assert tail != "out_{today}.txt"  # the form's own text expanded on the way out
    assert tail.startswith("out_20")
    assert tail.endswith(".txt")
    assert expansions == [True]
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["out_{today}.txt"]  # intent persisted, never expansion
    assert state["extra_args_raw"] is True  # ...marked, so the CLI replays it the same way


async def test_submitting_a_prefilled_literal_tail_untouched_keeps_it_literal(
    tmp_path, quiet_run, expansions
):
    """THE round-5 provenance bug, third face: the extra field is PREFILLED from the remembered
    tail, so a CLI-captured literal tail that the user merely looked at and pressed Enter past
    was re-expanded here — and its stored marker flipped to raw, so every later replay on BOTH
    faces expanded it too. One Enter-Enter pass silently rewrote what the user's shell had
    already processed. An untouched tail keeps the provenance it was recorded with."""
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, extra_args=["out_{today}.txt"])  # unmarked → literal
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        # Prefilled from the record (quoted the way the field's own join/split pair
        # round-trips a token-bearing word), then submitted untouched.
        assert await _submit_form(app, pilot) == argv_text.join(["out_{today}.txt"])

    assert quiet_run["extra"] == ["out_{today}.txt"]  # delivered verbatim, like `skit run`
    assert expansions == [False]
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["out_{today}.txt"]
    assert state["extra_args_raw"] is False  # the marker did NOT flip


async def test_submitting_a_prefilled_raw_tail_untouched_keeps_it_raw(
    tmp_path, quiet_run, expansions
):
    """The other half of "keeps its recorded provenance": an untouched tail that WAS captured
    raw still expands. The rule follows the record, not the direction — a form-captured tail
    must not become literal just because the next run passed through the form silently."""
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, extra_args=["out_{today}.txt"], extra_args_raw=True)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        assert await _submit_form(app, pilot) == argv_text.join(["out_{today}.txt"])

    (tail,) = quiet_run["extra"]
    assert tail.startswith("out_20")  # expanded, as its record says
    assert expansions == [True]
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["out_{today}.txt"]
    assert state["extra_args_raw"] is True


async def test_editing_the_prefilled_tail_recaptures_it_as_form_text(
    tmp_path, quiet_run, expansions
):
    """Only text the user ACTUALLY edited is re-captured as raw form text — and that is also the
    documented one-time repair for a legacy literal tail: re-enter it in the launch menu once
    and it becomes expanding form text from then on."""
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, extra_args=["stale_{today}.txt"])  # unmarked → literal
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        prefilled = await _submit_form(app, pilot, tail="out_{today}.txt")
        assert prefilled == argv_text.join(["stale_{today}.txt"])  # ...and was replaced

    (tail,) = quiet_run["extra"]
    assert tail.startswith("out_20")  # the edited text expanded
    assert expansions == [True]
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["out_{today}.txt"]
    assert state["extra_args_raw"] is True  # ...and is marked raw for every later replay


async def test_exit_mode_hands_the_forms_marker_to_the_pending_run(tmp_path, quiet_run):
    """Out of the box skit is a launcher: the form's submit exits the TUI with a PendingRun and
    the save happens afterwards, in _finish_run. The tail's provenance has to ride ALONG — a
    run must not be remembered under a different expansion regime just because after_run is
    "exit" rather than "stay". (quiet_run pins stay for the workbench tests; flip back here.)"""
    config.save_after_run("exit")
    entry = _managed_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        extra_row = next(r for r in screen.query(FieldRow) if r.field.key == _EXTRA_KEY)
        extra_row.set_value("out_{today}.txt")
        await pilot.pause()
        screen.action_submit()
        await pilot.pause()

    pending = app.return_value
    assert isinstance(pending, tui.PendingRun)
    assert pending.entry.slug == entry.slug
    assert pending.extra == ["out_{today}.txt"]  # intent, carried unexpanded
    assert pending.extra_raw is True  # ...and marked, so _finish_run records it raw


async def test_pending_run_carries_the_marker_into_the_deferred_save(tmp_path, monkeypatch):
    """Exit mode defers the save to _finish_run, so PendingRun has to carry the bit an
    immediate save would have recorded — otherwise the same run would be remembered under a
    different expansion regime depending only on after_run."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {}, ["out_{today}.txt"], cwd=tmp_path)
    monkeypatch.setattr("builtins.print", lambda *a, **k: None)
    monkeypatch.setattr(flows, "execute", lambda *a, **k: flows.RunOutcome(0))

    tui._finish_run(
        tui.PendingRun(entry, plan, asm, {}, ["out_{today}.txt"], extra_raw=True, show_drift=False)
    )
    assert argstate.load_state(entry.slug)["extra_args_raw"] is True

    tui._finish_run(
        tui.PendingRun(entry, plan, asm, {}, ["out_{today}.txt"], extra_raw=False, show_drift=False)
    )
    assert argstate.load_state(entry.slug)["extra_args_raw"] is False


async def _open_run_form(app, pilot) -> RunFormScreen:
    app.action_run()
    await pilot.pause()
    screen = app.screen
    assert isinstance(screen, RunFormScreen)
    return screen


def _extra_row(screen: RunFormScreen) -> FieldRow:
    return next(r for r in screen.query(FieldRow) if r.field.key == _EXTRA_KEY)


async def _retype(pilot, row: FieldRow, text: str) -> None:
    """Clear the extra field and type `text` into it with real keystrokes — the only way to
    prove the path a user takes (a programmatic .value assignment posts the same Input.Changed,
    so it would prove nothing about the keyboard)."""
    box = row.query_one(Input)
    box.focus()
    await pilot.pause()
    await pilot.press("end", *(["backspace"] * len(box.value)))
    await pilot.press(*text)
    await pilot.pause()


async def test_clearing_and_retyping_the_identical_tail_counts_as_typing(
    tmp_path, quiet_run, expansions
):
    """THE round-6 finding. Provenance used to be inferred by comparing the submitted tail
    against the state on disk, so a user who selected the remembered tail, deleted it and typed
    it back — the documented one-time repair for a legacy literal tail — produced identical
    text and was judged untouched. The repair silently did nothing, twice, forever.

    The form now tracks a real dirt bit on the extra row: the EVENT is the truth, and typing
    into the launch menu is typing into the launch menu whatever the letters come out as."""
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, extra_args=["out_{today}.txt"])  # unmarked → literal
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        screen = await _open_run_form(app, pilot)
        row = _extra_row(screen)
        prefilled = row.value
        await _retype(pilot, row, prefilled)
        assert row.value == prefilled  # byte-identical to what it replaced
        screen.action_submit()
        await pilot.pause()

    (tail,) = quiet_run["extra"]
    assert tail.startswith("out_20")  # the retyped text is form text: it expanded
    assert expansions == [True]
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["out_{today}.txt"]  # intent persisted, never expansion
    assert state["extra_args_raw"] is True  # ...and the repair actually took


async def test_a_concurrent_write_to_the_stored_tail_cannot_fake_an_edit(
    tmp_path, quiet_run, expansions
):
    """The other half of the same repair. Diffing the submitted tail against freshly-reloaded
    state also read the reverse case wrong: an agent running `skit run` in another terminal
    while the launch menu sits open changed the record, so the untouched prefill no longer
    matched it and was re-captured as form text — a write by someone else promoted to a user
    edit. The verdict is the FORM's, judged against its own compose-time snapshot, and the
    tail that launches is the one the user was looking at."""
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, extra_args=["out_{today}.txt"])  # unmarked → literal
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        screen = await _open_run_form(app, pilot)
        # …an agent runs `skit run j -- other.txt` in another terminal, right now.
        argstate.save_last(entry.slug, extra_args=["other.txt"], extra_args_raw=True)
        screen.action_submit()
        await pilot.pause()

    assert quiet_run["extra"] == ["out_{today}.txt"]  # what the form showed, verbatim
    assert expansions == [False]  # ...under the provenance the form composed with
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["out_{today}.txt"]
    assert state["extra_args_raw"] is False  # the concurrent write did not flip the marker


def test_the_forms_dirt_bits_start_disarmed_and_clean(tmp_path):
    """The dirt bit follows the runner picker's own two-flag idiom: a form starts DISARMED (the
    compose-time value settling must never count as a user edit) and CLEAN, each exactly False
    rather than merely falsy — a None reads the same inside an `if` and quietly turns a boolean
    into a tri-state nobody wrote a rule for."""
    entry = _managed_entry(tmp_path)
    screen = RunFormScreen(entry, flows.plan_for_entry(entry), {})
    assert screen._extra_armed is False
    assert screen._extra_dirty is False
    assert screen._runner_pick_armed is False
    assert screen._runner_was_picked is False
    assert screen._skip_apply_until is None  # the guard is off, not armed on an empty name
    assert screen._runner_default == ""  # the omitted-arg default, not a sentinel


def test_a_form_given_no_state_loads_the_entrys_own(tmp_path):
    """The default path is unchanged: every caller that does NOT already hold the state (the
    CLI's inline frame) keeps getting the entry's stored snapshot, and the tail's provenance
    baseline comes from that same read."""
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, extra_args=["out.txt"], extra_args_raw=True)

    screen = RunFormScreen(entry, flows.plan_for_entry(entry), {})

    assert screen._state["extra_args"] == ["out.txt"]
    assert screen._extra_prefill_raw is True


def test_a_form_given_a_state_uses_that_one_and_reads_nothing(tmp_path, monkeypatch):
    """…and a caller that already holds the snapshot hands it over WHOLE: the tail, its
    provenance and the preset list all come from the one object, so no part of one form can
    describe a different generation than another part."""
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, extra_args=["on-disk.txt"])
    monkeypatch.setattr(
        tui_form.argstate, "load_state", lambda slug: pytest.fail("re-read the state file")
    )
    handed = {
        "values": {},
        "presets": {"trip": {"CITY": "Nara"}},
        "extra_args": ["handed.txt"],
        "extra_args_raw": True,
        "last_run": {},
    }

    screen = RunFormScreen(entry, flows.plan_for_entry(entry), {}, state=handed)

    assert screen._state is handed
    assert screen._presets is handed["presets"]
    assert screen._extra_prefill_raw is True


@pytest.fixture
def state_reads(monkeypatch):
    """Every argstate.load_state a test's window performs. Installed by the test AFTER the
    app has mounted, so the initial detail render is not in the count."""
    reads: list[str] = []
    real = argstate.load_state

    def counting(slug):
        reads.append(slug)
        return real(slug)

    def install() -> list[str]:
        monkeypatch.setattr(tui.argstate, "load_state", counting)
        return reads

    return install


@pytest.fixture
def prefill_calls(monkeypatch):
    """Every (plan, slug, kwargs) triple handed to flows.prefill. The state kwarg makes the
    slug unused DOWNSTREAM today, so only the call site itself can pin that the entry's own
    slug (never None, never another entry's) is what the launch asks about — remove the state
    kwarg later and a wrong slug there is a form filled from the wrong entry's memory."""
    calls: list[tuple[object, object, dict[str, object]]] = []
    real = flows.prefill

    def spy(plan, slug, preset=None, **kwargs):
        calls.append((plan, slug, kwargs))
        return real(plan, slug, preset, **kwargs)

    monkeypatch.setattr(tui.flows, "prefill", spy)
    return calls


async def test_action_run_asks_prefill_about_this_entry_with_the_one_snapshot(
    tmp_path, quiet_run, prefill_calls
):
    """The launch's prefill call is wired to THIS entry's slug and to the very state object the
    form is handed — one snapshot, one entry, no second read hiding behind a default."""
    entry = _managed_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        screen = await _open_run_form(app, pilot)
        (plan, slug, kwargs) = prefill_calls[-1]
        assert slug == entry.slug
        assert plan is screen._plan
        assert kwargs["state"] is screen._state


async def test_action_rerun_asks_prefill_about_this_entry_with_the_one_snapshot(
    tmp_path, quiet_run, prefill_calls
):
    """…and so is the form-free `r` path, which assembles the launch straight out of that
    prefill: a wrong slug here replays another entry's remembered values with no form to show
    them in."""
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, values={"CITY": "Kyoto"})
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.action_rerun()
        await pilot.pause()

    (_plan, slug, kwargs) = prefill_calls[-1]
    assert slug == entry.slug
    assert kwargs["state"]["values"] == {"CITY": "Kyoto"}  # the one snapshot, not a fresh read
    assert quiet_run["extra"] == []  # ...and that prefill really carried the launch


async def test_a_launch_interaction_reads_the_entrys_state_exactly_once(tmp_path, state_reads):
    """One interaction, one snapshot. The prefill, the form's remembered tail, its preset list
    and the provenance baseline used to come from FOUR separate reads of one file — four
    chances for an agent writing beside the TUI to hand different parts of one form different
    generations. (Exit mode, so the post-run reload's own render is outside the window.)"""
    config.save_after_run("exit")
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, values={"CITY": "Kyoto"}, extra_args=["out.txt"])
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        reads = state_reads()
        screen = await _open_run_form(app, pilot)
        screen.action_submit()
        await pilot.pause()
        assert reads == [entry.slug]

    pending = app.return_value
    assert isinstance(pending, tui.PendingRun)  # ...and that one read served the whole launch
    assert pending.extra == ["out.txt"]  # the remembered tail reached the form
    assert pending.values == {"CITY": "Kyoto"}  # ...and so did the remembered values
    assert pending.extra_raw is False  # ...and the provenance baseline came from it too


async def test_the_rerun_path_reads_the_entrys_state_exactly_once(tmp_path, state_reads):
    """The `r` path had the same repeated read — the last-run guard, the prefill and the tail's
    provenance each opened the file, so a rerun could check one generation's "has it run" and
    replay another generation's arguments. One read now answers all three."""
    config.save_after_run("exit")
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, values={"CITY": "Kyoto"}, extra_args=["out.txt"])
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        reads = state_reads()
        app.action_rerun()
        await pilot.pause()
        assert reads == [entry.slug]

    pending = app.return_value
    assert isinstance(pending, tui.PendingRun)
    assert pending.extra == ["out.txt"]
    assert pending.values == {"CITY": "Kyoto"}
    assert pending.extra_raw is False


# ==========================================================================
# E. the Library detail pane's plan cache
# ==========================================================================


@pytest.fixture
def plan_builds(monkeypatch):
    """Count real plan builds. The detail pane re-renders on every RowHighlighted, and for a
    reader kind a build is a synchronous subprocess with a cold-start runtime."""
    calls: list[str] = []
    real = flows.plan_for_entry

    def counting(entry):
        calls.append(entry.slug)
        return real(entry)

    monkeypatch.setattr(tui.flows, "plan_for_entry", counting)
    return calls


def test_cached_plan_serves_a_second_call_without_rebuilding(tmp_path, plan_builds):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    first = app._cached_plan(entry)
    assert plan_builds == [entry.slug]
    assert app._cached_plan(entry) is first  # the very same object, not a rebuild
    assert plan_builds == [entry.slug]


def test_cached_plan_invalidates_when_the_script_changes(tmp_path, plan_builds):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    app._cached_plan(entry)
    import os

    stamp = entry.script_path.stat().st_mtime + 10
    os.utime(entry.script_path, (stamp, stamp))
    app._cached_plan(entry)
    assert plan_builds == [entry.slug, entry.slug]  # the body changed → rebuilt


def test_cached_plan_invalidates_when_meta_toml_changes(tmp_path, plan_builds):
    """A plan is a function of meta.toml too (declared [[parameters]] rows, a prompt's managed
    list / interpolate switch). Keying on the script alone left the detail pane stale FOREVER
    when an agent ran `skit params --add` beside an open TUI — the product's own coexistence
    story."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    app._cached_plan(entry)
    import os

    meta = entry.dir / "meta.toml"
    stamp = meta.stat().st_mtime + 10
    os.utime(meta, (stamp, stamp))
    app._cached_plan(entry)
    assert plan_builds == [entry.slug, entry.slug]


def test_cached_plan_builds_fresh_when_the_script_cannot_be_stat_ed(tmp_path, plan_builds):
    """An entry whose script is gone (a removed target) can't produce a key at all: it just
    builds fresh — and caches nothing — rather than letting the OSError escape a cursor-
    movement handler and crash the app."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="ghost")
    entry.script_path.unlink()
    app = tui.MenuApp()
    app._cached_plan(entry)
    app._cached_plan(entry)
    assert plan_builds == [entry.slug, entry.slug]  # never cached, never crashed
    assert entry.slug not in app._plan_cache


def test_cached_plan_key_is_mtime_ns_and_size_per_file_plus_the_reader_tool(tmp_path):
    """The key is (mtime_ns, size) of the script AND of meta.toml, then the kind's reader-tool
    fingerprint. A float st_mtime is second- (FAT: two-second-) granular on some filesystems,
    so a same-tick edit landed under a key that still looked fresh; the size half narrows that
    blind spot to same-tick same-size writes. The fifth element is None for a kind whose
    reading needs no external tool (python parses in-process)."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    app._cached_plan(entry)

    (key, _plan) = app._plan_cache[entry.slug]
    assert key == plan_cache_key(entry)
    script, meta = entry.script_path.stat(), (entry.dir / "meta.toml").stat()
    assert key == (script.st_mtime_ns, script.st_size, meta.st_mtime_ns, meta.st_size, None)


def _exe(tmp_path, name: str = "prog"):
    prog = tmp_path / "t"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    return store.add_exe(prog, name=name)


def test_cached_plan_caches_the_generation_its_caller_handed_it(tmp_path):
    """Round 6 moved freshness OUT of the cache: _cached_plan no longer re-resolves, it caches
    whatever generation its CALLER renders. Handed the current record it caches the current
    plan; handed a stale snapshot it does not silently second-guess the caller. One owner of
    freshness (MenuApp._fresh), not two racing ones — the pane's own guarantee is pinned in the
    test below, and _fresh is the only thing standing between the two.

    The scenario behind it is the product's own coexistence story: an agent runs `skit params
    prog --add WIDTH` beside an open TUI, and nothing in the app has reloaded yet."""
    stale = _exe(tmp_path)
    app = tui.MenuApp()
    assert [f.key for f in app._cached_plan(stale).fields] == []

    store.write_parameters(
        stale.slug, [ParamDecl(name="WIDTH", delivery="flag", flag="--width", type="int")]
    )

    # The current record in — the current plan out, and THAT is what lands in the cache.
    plan = app._cached_plan(store.resolve(stale.slug))
    assert [f.key for f in plan.fields] == ["WIDTH"]
    assert app._cached_plan(store.resolve(stale.slug)) is plan

    # The stale snapshot in — the stale plan out (a caller asking about last week's record
    # gets an answer about last week's record). Nothing in the app does this: every
    # production call site passes _fresh().
    app._plan_cache.clear()
    assert [f.key for f in app._cached_plan(stale).fields] == []


async def test_the_detail_pane_catches_up_with_a_parameter_declared_beside_it(tmp_path):
    """Where that guarantee lives now: _refresh_detail resolves the record itself, so the very
    meta.toml write the key noticed is the one the plan is built from. Before round 6 the miss
    re-read inside the cache; the pane's OTHER lines still came from the Library snapshot, so
    one render could show a fresh parameter row under a stale description."""
    entry = _exe(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        assert "WIDTH" not in _detail_text(app)

        store.write_parameters(
            entry.slug, [ParamDecl(name="WIDTH", delivery="flag", flag="--width", type="int")]
        )
        store.update_description(entry.slug, "resized by an agent")

        app._refresh_detail()  # a cursor movement, with no _reload in between
        await pilot.pause()
        text = _detail_text(app)

    assert "WIDTH" in text  # the plan the cache serves is the new generation...
    assert "resized by an agent" in text  # ...and so is the description beside it


def test_fresh_degrades_to_the_snapshot_when_the_record_no_longer_resolves(tmp_path, monkeypatch):
    """_fresh is the ONE owner of "what generation is this interaction about", so it has to
    survive the entry being removed (or its meta corrupted) between the Library's reload and
    this render: it hands back the snapshot the caller already holds and lets the caller's own
    missing/error path speak. A cursor movement must never raise."""
    entry = _managed_entry(tmp_path, name="a")
    app = tui.MenuApp()
    assert app._fresh(entry).meta.name == "a"  # resolves for real when it can

    def removed(_slug):
        raise store.NotFoundError("removed mid-render")

    monkeypatch.setattr(tui.store, "resolve", removed)
    assert app._fresh(entry) is entry  # the snapshot, verbatim — not an exception


def test_cached_plan_caches_an_unresolvable_entrys_plan_instead_of_rebuilding_per_highlight(
    tmp_path, plan_builds, monkeypatch
):
    """A corrupt meta.toml made _fresh degrade to the snapshot on EVERY render, and the old
    cache refused to store what the resolve couldn't confirm — so a reader kind paid a
    subprocess per cursor move for as long as the corruption lasted. The files still stat, so
    the key is real: cache the plan the caller could build."""
    entry = _managed_entry(tmp_path, name="a")
    app = tui.MenuApp()

    def removed(_slug):
        raise store.NotFoundError("removed mid-render")

    monkeypatch.setattr(tui.store, "resolve", removed)

    plan = app._cached_plan(app._fresh(entry))

    assert [f.key for f in plan.fields] == ["CITY"]  # a real plan, built from the snapshot
    assert app._cached_plan(app._fresh(entry)) is plan  # ...served from the cache
    assert plan_builds == [entry.slug]  # exactly ONE build, not one per highlight


def test_cached_plan_caches_nothing_when_a_write_lands_during_the_build(
    tmp_path, plan_builds, monkeypatch
):
    """The poison this cache once shipped: the stat comes BEFORE the plan's own file reads, so
    a write landing in that window is read by the build yet keyed by the pre-write stat —
    pinning content under a key that every later highlight cache-HITS. A second _plan_key after
    the build closes it: when the two disagree, nothing is cached and the next render rebuilds
    (one wasted build beats an indefinitely wrong pane)."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    real_plan_for_entry = tui.flows.plan_for_entry

    def writing(e):
        # A racing `skit params --add` (or an $EDITOR save) between the two stats.
        plan = real_plan_for_entry(e)
        (e.dir / "meta.toml").write_text(
            (e.dir / "meta.toml").read_text(encoding="utf-8") + "\n# raced\n", encoding="utf-8"
        )
        return plan

    monkeypatch.setattr(tui.flows, "plan_for_entry", writing)

    app._cached_plan(entry)
    assert entry.slug not in app._plan_cache  # the key it computed no longer describes the files

    monkeypatch.setattr(tui.flows, "plan_for_entry", real_plan_for_entry)
    app._cached_plan(entry)
    assert app._plan_cache[entry.slug][0] == plan_cache_key(entry)  # ...and the next one settles


def test_cached_plan_caches_nothing_when_the_snapshot_predates_the_meta_on_disk(
    tmp_path, plan_builds
):
    """The OTHER half of the cache window, and the one stat equality cannot see. The write
    lands before the FIRST stat — an agent's `skit params --add` between the caller's resolve
    and this render — so both stats agree while the plan is built from the caller's older
    meta (plan_for_entry reads declared rows from the in-memory entry.meta, not from disk).

    Round 6's single proof pinned that stale plan under the CURRENT key, which every later
    highlight then cache-HIT: the parameter an agent just declared would never appear, for as
    long as nothing else touched the files. The meta re-read closes it."""
    entry = _exe(tmp_path)  # the caller's snapshot, taken before the write
    store.write_parameters(
        entry.slug, [ParamDecl(name="WIDTH", delivery="flag", flag="--width", type="int")]
    )

    app = tui.MenuApp()
    stale = app._cached_plan(entry)

    assert [f.key for f in stale.fields] == []  # built from the snapshot it was handed...
    assert entry.slug not in app._plan_cache  # ...and not pinned under the current key
    # ...so the next render — the fresh record the pane really passes — sees the new row.
    plan = app._cached_plan(store.resolve(entry.slug))
    assert [f.key for f in plan.fields] == ["WIDTH"]
    assert app._plan_cache[entry.slug] == (plan_cache_key(entry), plan)
    assert plan_builds == [entry.slug] * 2


def test_meta_unchanged_compares_the_whole_record_and_treats_gone_as_unchanged(
    tmp_path, monkeypatch
):
    """The proof itself, at its three answers. It re-reads the record and compares BOTH halves
    — a plan built from a different meta is stale, and so is one built against a different
    directory — while an entry that no longer resolves counts as unchanged, because the
    snapshot is then the best generation there is and caching it is the round-6 fix for the
    subprocess-per-highlight corrupt-meta case."""
    entry = _managed_entry(tmp_path, name="a")
    app = tui.MenuApp()
    assert app._meta_unchanged(entry) is True

    store.update_description(entry.slug, "described by an agent")
    assert app._meta_unchanged(entry) is False  # the meta half

    fresh = store.resolve(entry.slug)
    assert app._meta_unchanged(fresh) is True
    monkeypatch.setattr(tui.store, "resolve", lambda _slug: replace(fresh, dir=tmp_path / "moved"))
    assert app._meta_unchanged(fresh) is False  # the dir half: same meta, different home

    def removed(_slug):
        raise store.NotFoundError("removed mid-render")

    monkeypatch.setattr(tui.store, "resolve", removed)
    assert app._meta_unchanged(fresh) is True  # unresolvable is not "changed"


def test_meta_unchanged_asks_fresh_rather_than_keeping_its_own_degrade_policy(
    tmp_path, monkeypatch
):
    """ROUND 8. _meta_unchanged used to run its own store.resolve + StoreError catch beside
    _fresh's — the same decision ("what counts as the current generation?") written twice, in a
    codebase where round 6 had just spent a whole round giving that decision ONE owner because
    the pane and the launch it advertised were reading different generations.

    Two copies do not have to disagree today to be a bug; they have to be able to. So the proof
    is delegation, not equal answers: change what _fresh returns and _meta_unchanged must follow
    it, because it never looks at the store itself."""
    entry = _managed_entry(tmp_path, name="a")
    app = tui.MenuApp()
    assert app._meta_unchanged(entry) is True

    asked: list[str] = []

    def moved(e):
        asked.append(e.slug)
        return replace(e, dir=tmp_path / "elsewhere")

    monkeypatch.setattr(tui.MenuApp, "_fresh", staticmethod(moved))
    assert app._meta_unchanged(entry) is False  # ...the answer came from _fresh
    assert asked == [entry.slug]
    # ...and the store is not consulted behind _fresh's back.
    monkeypatch.setattr(tui.store, "resolve", _never_called)
    assert app._meta_unchanged(entry) is False


def _never_called(_slug):
    raise AssertionError("_meta_unchanged must reach the store through _fresh only")


def test_cached_plan_reprobes_when_the_reader_tool_appears_mid_session(
    tmp_path, plan_builds, monkeypatch
):
    """A reader plan is a function of the reader TOOL too, so a key of file stats alone kept
    serving the tool-less plan after the user installed pwsh in another terminal — the pane
    would have claimed the script has no parameters for the rest of the session. Same rule the
    add panel's memo follows, same fingerprint. (The kind is stubbed rather than run against a
    real pwsh: CI has none, and the key is what's under test, not PowerShell.)"""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    tool: list[str | None] = [None]
    real = spec_for("python")
    assert real is not None
    gated = _tool_gated_spec(real, tool)
    # Only tui's OWN spec lookup (the key's fingerprint) is stubbed; flows builds the plan
    # through its own import, so the builds counted below are the real ones.
    monkeypatch.setattr(tui, "spec_for", lambda kind: gated if kind == "python" else real)

    app._cached_plan(entry)
    app._cached_plan(entry)
    assert plan_builds == [entry.slug]  # memoized while the fingerprint holds

    tool[0] = "/opt/pwsh"  # installed in another terminal, mid-session
    app._cached_plan(entry)
    assert plan_builds == [entry.slug] * 2  # ...seen, not served from the stale key

    tool[0] = None  # ...and uninstalled again
    app._cached_plan(entry)
    assert plan_builds == [entry.slug] * 3


def test_has_drift_is_served_off_the_same_cache(tmp_path, plan_builds):
    """The drift badge and the detail pane are one read, not two: asking for drift after the
    pane already built the plan must not build a second one."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    app._cached_plan(entry)
    assert app._has_drift(entry) is False
    assert plan_builds == [entry.slug]  # ...still only the one build


def test_has_drift_short_circuits_before_building_any_plan(tmp_path, plan_builds):
    """Drift is the EXPENSIVE check, and the guard exists so a kind that cannot produce drift
    lines never pays for a plan to find that out. An exe has no analyzer, so the answer is
    False without a single build — even though its stored copy is right there to be read."""
    prog = tmp_path / "t"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    entry = store.add_exe(prog, name="prog")
    assert entry.script_path.exists()  # the guard's other arms are NOT what answers here
    app = tui.MenuApp()

    assert app._has_drift(entry) is False
    assert plan_builds == []  # short-circuited: no plan was ever built


def test_has_drift_reports_the_cached_plans_drift(tmp_path):
    """It really reads the plan's drift rather than always answering False."""
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


async def test_edit_pops_this_entrys_cache_key(tmp_path, monkeypatch):
    """The stored copy may change under $EDITOR without its mtime helping (same-second
    writes), so the edit drops this slug's key outright."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    monkeypatch.setattr(tui.editor, "open_in_editor", lambda p: None)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        planted = flows.FormPlan(source="none", drift_lines=["planted"])
        app._plan_cache[entry.slug] = (plan_cache_key(entry), planted)
        app.action_edit()
        await pilot.pause()
        assert app._plan_cache.get(entry.slug, (None, None))[1] is not planted


async def test_settings_close_pops_this_entrys_cache_key(tmp_path):
    """A Resync inside Script settings can change the definitions without touching the script,
    so the close callback drops the key too."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_settings()
        await pilot.pause()
        planted = flows.FormPlan(source="none", drift_lines=["planted"])
        app._plan_cache[entry.slug] = (plan_cache_key(entry), planted)
        app.screen.dismiss(False)
        await pilot.pause()
        assert app._plan_cache.get(entry.slug, (None, None))[1] is not planted
        assert "The script changed" not in _detail_text(app)


# ==========================================================================
# E2. …and both LAUNCH paths read the same fresh record the pane described
# ==========================================================================


async def test_enter_opens_a_form_built_from_the_record_on_disk_right_now(tmp_path, quiet_run):
    """The pane was made fresher than the launch it fronts: the detail pane re-resolved on
    every render while Enter still built its form from the Library's snapshot, so the pane
    could advertise a parameter and the form beside it launch without one. Both paths now go
    through _fresh, so the form has the field the user is looking at."""
    entry = _exe(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        # An agent declares a parameter beside the open TUI; nothing has reloaded.
        store.write_parameters(
            entry.slug,
            [ParamDecl(name="WIDTH", delivery="flag", flag="--width", type="int", default="800")],
        )
        screen = await _open_run_form(app, pilot)
        assert [f.key for f in screen._plan.fields] == ["WIDTH"]
        assert [r.field.key for r in screen.query(FieldRow) if r.field.key != _EXTRA_KEY] == [
            "WIDTH"
        ]
        screen.action_submit()
        await pilot.pause()

    assert quiet_run["extra"] == ["--width", "800"]  # ...and the launch really carries it


async def test_rerun_launches_the_record_on_disk_not_the_library_snapshot(tmp_path, quiet_run):
    """Same rule on the form-free path: `r` skips the form, never the freshness. A rerun built
    from the snapshot would replay yesterday's argv for a definition that changed a second
    ago — silently dropping the flag the entry now takes."""
    entry = _exe(tmp_path)
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        store.write_parameters(
            entry.slug,
            [ParamDecl(name="WIDTH", delivery="flag", flag="--width", type="int", default="800")],
        )
        app.action_rerun()
        await pilot.pause()

    assert quiet_run["extra"] == ["--width", "800"]


async def test_the_launch_paths_still_work_on_the_snapshot_when_the_record_wont_resolve(
    tmp_path, quiet_run, monkeypatch
):
    """A CORRUPT record degrades rather than raising, and the launch paths carry on:
    the claim cannot verify identity against an unreadable meta, so the snapshot the
    app already holds is what launches — reaching the launcher's own missing/error
    handling, never crashing a keypress handler. (A record that resolves to NOTHING is
    different since round 17: gone/reissued rows STOP the lane — see
    test_a_stale_library_row_stops_the_run_lane_and_refreshes.)"""
    entry = _exe(tmp_path)
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")

    def unresolvable(_slug):
        raise store.CorruptEntryError("meta corrupted mid-keypress")

    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()  # ...the Library's snapshot is taken here, without WIDTH
        store.write_parameters(
            entry.slug,
            [ParamDecl(name="WIDTH", delivery="flag", flag="--width", type="int", default="800")],
        )
        monkeypatch.setattr(tui.store, "resolve", unresolvable)
        app.action_rerun()
        await pilot.pause()

    assert quiet_run["extra"] == []  # the snapshot launched: nothing raised, nothing invented


# ==========================================================================
# J. the as-is note, TUI face — both of them, and the CLI's own msgid
# ==========================================================================

_AS_IS = "(passed as-is"


class _PrintRecorder:
    """Every print(*args, **kwargs) the run path makes, so a test can assert the text AND the
    flush that keeps it ahead of the child's own output."""

    def __init__(self) -> None:
        self.calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def __call__(self, *args: object, **kwargs: object) -> None:
        self.calls.append((args, kwargs))

    @property
    def lines(self) -> list[str]:
        return [" ".join(str(a) for a in args) for args, _ in self.calls]

    def flush_for(self, needle: str) -> object:
        for args, kwargs in self.calls:
            if needle in " ".join(str(a) for a in args):
                return kwargs.get("flush", "__no-flush-kwarg__")
        return "__line-not-printed__"


def test_the_note_has_one_home_and_resolves_at_print_time(monkeypatch):
    """ONE msgid across both faces — the sentence that explains provenance cannot be allowed
    to drift into two wordings. Round 7 gave it one HOME too (flows.as_is_note): tui._as_is_note
    was a second copy of the same source string, and two copies of a msgid are two things a
    translator can be asked about separately. Resolved when it prints, not at import: a
    module-level constant would freeze the note in whatever locale happened to be active when
    the module was imported."""
    assert flows.as_is_note() == (
        "(passed as-is — a remembered tail only expands {tokens} and globs "
        "when it was typed into the launch menu)"
    )
    assert not hasattr(tui, "_as_is_note")  # ...and the second copy is gone, not shadowed
    from skit import i18n

    monkeypatch.setenv("SKIT_LANG", "zh-TW")
    i18n.init("zh-TW")
    try:
        translated = flows.as_is_note()
    finally:
        i18n.init("en")
    assert _AS_IS not in translated  # the active locale really applies
    assert translated  # ...and it is translated, not blanked


async def test_rerun_says_a_marker_less_expandable_tail_is_passed_as_is(
    tmp_path, quiet_run, monkeypatch
):
    """The `r` path replays a CLI-captured tail literally BY DESIGN — the bug was doing it
    silently. The user sees `*.png` come back and reasonably expects the glob to expand, so
    the run output says it did not."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["out_{today}.txt", "*.png"])  # unmarked
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    rec = _PrintRecorder()
    monkeypatch.setattr("builtins.print", rec)

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_rerun()
        await pilot.pause()

    assert quiet_run["extra"] == ["out_{today}.txt", "*.png"]  # the note explains, never changes
    assert flows.as_is_note() in rec.lines  # the shared home's sentence, verbatim
    assert rec.flush_for(_AS_IS) is True  # ...ahead of the child's own output


@pytest.mark.parametrize(
    ("extra", "raw"),
    [(["out_{today}.txt"], True), (["--limit", "MAX"], False)],
    ids=["marked-raw", "plain-tail"],
)
async def test_rerun_stays_quiet_for_a_marked_or_plain_tail(
    tmp_path, quiet_run, monkeypatch, extra: list[str], raw: bool
):
    """The note is a surprise-avoidance line, not a banner. A raw-marked tail EXPANDS, so the
    note would claim the opposite of what just happened; a tail of plain words expands to
    itself under either regime, so saying anything would be noise on every rerun."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=extra, extra_args_raw=raw)
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    rec = _PrintRecorder()
    monkeypatch.setattr("builtins.print", rec)

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_rerun()
        await pilot.pause()

    assert not any(_AS_IS in line for line in rec.lines)


@pytest.mark.parametrize(
    ("extra", "extra_raw", "noted"),
    [
        (["out_{today}.txt"], False, True),
        (["~/backups"], False, True),
        # Round 7, both directions of the predicate's repair on this face too: `}}` would have
        # halved, a bare `{x}` would not have changed at all.
        (["done}}"], False, True),
        (["{x}"], False, False),
        (["out_{today}.txt"], True, False),
        (["--limit", "MAX"], False, False),
    ],
    ids=[
        "token-literal",
        "tilde-literal",
        "close-escape-literal",
        "unknown-brace",
        "marked-raw",
        "plain-tail",
    ],
)
def test_the_exit_after_run_path_prints_the_note_too(
    tmp_path, monkeypatch, extra: list[str], extra_raw: bool, noted: bool
):
    """Out of the box skit is a launcher: the run happens AFTER the TUI exits, in _finish_run.
    A transparency line that only the stay-mode path printed would be missing from the default
    experience — the same run, the same tail, one face silent."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {}, list(extra), cwd=tmp_path, expand_extra=extra_raw)
    rec = _PrintRecorder()
    monkeypatch.setattr("builtins.print", rec)
    monkeypatch.setattr(flows, "execute", lambda *a, **k: flows.RunOutcome(0))

    tui._finish_run(
        tui.PendingRun(entry, plan, asm, {}, list(extra), extra_raw=extra_raw, show_drift=False)
    )

    assert any(_AS_IS in line for line in rec.lines) is noted
    if noted:
        assert flows.as_is_note() in rec.lines  # the shared home's sentence, verbatim
        assert rec.flush_for(_AS_IS) is True


# ==========================================================================
# F. AddReviewScreen._reader_modeled memoizes its probe
# ==========================================================================


_UNMODELED_SH = "#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n"
# Exactly ONE reader field: "modeled" is `> 0`, not "more than one" — a single-option script
# is as modeled as a ten-option one, and its form is the entry's real interface.
_MODELED_SH = "#!/usr/bin/env bash\nwhile getopts 'n:' o; do :; done\n"


@pytest.fixture
def reader_probes(monkeypatch):
    """Every text flows.reader_fields is actually asked about. The screen is built but NOT
    pushed, so compose can't warm the memo behind the count."""
    probes: list[str] = []
    real = flows.reader_fields

    def counting(spec, text):
        probes.append(text)
        return real(spec, text)

    monkeypatch.setattr(flows, "reader_fields", counting)
    return probes


def test_reader_modeled_probes_once_per_text(tmp_path, reader_probes):
    """The probe fires on every mode toggle and on every accept-gate check. For a reader kind
    (PowerShell) each one is a synchronous subprocess, so the panel would freeze for seconds
    because a radio button was clicked — three calls, exactly one probe."""
    src = tmp_path / "m.sh"
    src.write_text(_UNMODELED_SH, encoding="utf-8")
    review = AddReviewScreen(src, kind="shell")

    assert review._reader_modeled() is False
    assert review._reader_modeled() is False
    assert review._reader_modeled() is False

    assert reader_probes == [_UNMODELED_SH]  # exactly one, then the memo answers


def test_reader_modeled_recomputes_after_the_text_changes(tmp_path, reader_probes):
    """The memo is keyed on the TEXT, so the Ctrl+E edit→rescan path recomputes naturally — a
    memo keyed on nothing would pin the panel to the pre-edit verdict forever."""
    src = tmp_path / "m.sh"
    src.write_text(_UNMODELED_SH, encoding="utf-8")
    review = AddReviewScreen(src, kind="shell")
    assert review._reader_modeled() is False

    # What the edit→rescan path does: new text in, verdict re-derived from it.
    review._text = _MODELED_SH
    assert review._reader_modeled() is True
    assert review._reader_modeled() is True  # ...and the new verdict memoizes too

    assert reader_probes == [_UNMODELED_SH, _MODELED_SH]


def _tool_gated_spec(spec, tool: list[str | None]):
    """A spec whose reader is shaped exactly like PowerShell's: its verdict is a function of
    the text AND of which tool answers, and it publishes that tool's identity as its
    runtime_fingerprint. `tool` is the mutable stand-in for PATH."""
    reader = CliReader(
        read_cli=lambda _text: (
            None
            if tool[0] is None
            else ArgSpec(fields=[ParamDecl(name="Name", delivery="flag", flag="-Name")])
        ),
        runtime_fingerprint=lambda: tool[0],
    )
    return spec.with_capabilities(replace(spec.resolved_capabilities, cli_reader=reader))


def test_reader_modeled_reprobes_when_the_reader_tool_appears_or_vanishes(tmp_path, reader_probes):
    """A tool-backed reader has no verdict without its tool, so a memo keyed on the text alone
    kept serving "no form" after the user installed pwsh in another terminal — the panel would
    have claimed the script has no interface for the rest of the session. The memo keys on the
    reader's runtime fingerprint too, so the tool appearing (and vanishing) is SEEN."""
    src = tmp_path / "m.ps1"
    src.write_text(_UNMODELED_SH, encoding="utf-8")
    review = AddReviewScreen(src, kind="shell")
    tool: list[str | None] = [None]  # no pwsh on PATH yet
    review._spec = _tool_gated_spec(review._spec, tool)

    assert review._reader_modeled() is False
    assert review._reader_modeled() is False
    assert len(reader_probes) == 1  # still memoized while the fingerprint holds

    tool[0] = "/opt/pwsh"  # installed in another terminal, mid-session
    assert review._reader_modeled() is True
    assert len(reader_probes) == 2  # ...re-probed rather than serving the tool-less verdict

    tool[0] = None  # ...and uninstalled again
    assert review._reader_modeled() is False
    assert len(reader_probes) == 3


def test_reader_modeled_memoizes_while_the_fingerprint_is_stable(tmp_path, reader_probes):
    """The fingerprint is a cheap PATH scan; the probe behind it is a synchronous subprocess
    with a cold-start runtime. An unchanged tool and unchanged text must still cost exactly one
    probe, or every radio click would freeze the panel for seconds."""
    src = tmp_path / "m.ps1"
    src.write_text(_UNMODELED_SH, encoding="utf-8")
    review = AddReviewScreen(src, kind="shell")
    review._spec = _tool_gated_spec(review._spec, ["/opt/pwsh"])

    assert review._reader_modeled() is True
    assert review._reader_modeled() is True
    assert review._reader_modeled() is True

    assert len(reader_probes) == 1


@pytest.mark.parametrize(
    ("kind", "name", "body"),
    [
        ("python", "m.py", "import argparse\np = argparse.ArgumentParser()\n"),
        ("js", "m.js", "const x = 1;\nconsole.log(x);\n"),
    ],
)
def test_purely_static_readers_carry_no_fingerprint_and_memoize_on_text_alone(
    tmp_path, reader_probes, kind: str, name: str, body: str
):
    """The fingerprint is opt-in: a reader that parses the text and nothing else has no
    external identity to key on, so it stays None and the memo is the text — no PATH scan on
    every call, no re-probe that can never change its answer."""
    spec = spec_for(kind)
    assert spec is not None
    assert spec.cli_reader is not None
    assert spec.cli_reader.runtime_fingerprint is None

    src = tmp_path / name
    src.write_text(body, encoding="utf-8")
    review = AddReviewScreen(src, kind=kind)
    first = review._reader_modeled()
    assert review._reader_modeled() is first
    assert review._reader_modeled() is first

    assert reader_probes == [body]


def test_the_powershell_registry_wires_the_reader_to_its_fingerprint() -> None:
    """The one kind whose read shells out is the one kind that publishes a fingerprint — the
    wiring is what makes the memo key above mean anything in production. Asserted on the
    BUILDER as well as on the resolved spec: capabilities are built once per process and then
    cached, so the spec any single test observes may have been resolved by an unrelated
    earlier one."""
    from skit.langs import registry
    from skit.langs.powershell import cli_reader as ps_reader

    built = registry._powershell_caps()
    assert built.cli_reader is not None
    assert built.cli_reader.read_cli is ps_reader.read_cli
    assert built.cli_reader.runtime_fingerprint is ps_reader.runtime_fingerprint

    spec = spec_for("powershell")
    assert spec is not None
    assert spec.cli_reader is not None
    assert spec.cli_reader.read_cli is ps_reader.read_cli
    assert spec.cli_reader.runtime_fingerprint is ps_reader.runtime_fingerprint


# ==========================================================================
# G. the chord grammar repair — one chord, one meaning
# ==========================================================================


def _prompt_entry(tmp_path, text="{{a}} {{b}} {{c}}\n", name="p"):
    src = tmp_path / f"{name}.prompt.md"
    src.write_text(text, encoding="utf-8")
    return store.add_prompt(src, name=name)


async def _open_settings(app, pilot):
    app.action_settings()
    await pilot.pause()
    assert isinstance(app.screen, ScriptSettingsScreen)
    await pilot.pause()
    return app.screen


def _many_names(count: int) -> list[str]:
    return [f"u{i}" for i in range(count)]


async def test_prompt_review_ctrl_l_opens_choose_variables(tmp_path):
    """Positive pilot test for the moved chord (AGENTS.md: every key a footer advertises
    must have one)."""
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    names = _many_names(AUTO_MANAGE_LIMIT + 2)
    src = tmp_path / "big.prompt.md"
    src.write_text(" ".join(f"{{{{{n}}}}}" for n in names), encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        await pilot.press("ctrl+l")
        await pilot.pause()
        assert isinstance(app.screen, PromptCandidatePickerModal)
        await pilot.press("escape")
        await pilot.pause()


async def test_prompt_review_ctrl_o_no_longer_opens_choose_variables(tmp_path):
    """The negative twin: Ctrl+O is the grammar chord for "restore the default" and must not
    mean anything else on a sibling screen."""
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    names = _many_names(AUTO_MANAGE_LIMIT + 2)
    src = tmp_path / "big.prompt.md"
    src.write_text(" ".join(f"{{{{{n}}}}}" for n in names), encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        review = PromptReviewScreen(src)
        app.push_screen(review)
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        assert app.screen is review  # nothing opened
        await pilot.press("escape")
        await pilot.pause()


async def test_settings_ctrl_l_opens_choose_variables(tmp_path):
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    names = _many_names(LIST_PREVIEW_LIMIT + 3)
    entry = _prompt_entry(tmp_path, text=" ".join(f"{{{{{n}}}}}" for n in names))
    store.write_prompt_managed(entry.slug, [])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        await _open_settings(app, pilot)
        await pilot.press("ctrl+l")
        await pilot.pause()
        assert isinstance(app.screen, PromptCandidatePickerModal)
        await pilot.press("escape", "escape")
        await pilot.pause()


async def test_settings_ctrl_o_no_longer_opens_choose_variables(tmp_path):
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    names = _many_names(LIST_PREVIEW_LIMIT + 3)
    entry = _prompt_entry(tmp_path, text=" ".join(f"{{{{{n}}}}}" for n in names))
    store.write_prompt_managed(entry.slug, [])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        await pilot.press("ctrl+o")
        await pilot.pause()
        assert app.screen is screen  # Ctrl+O stays reserved for "restore the default"
        await pilot.press("escape")
        await pilot.pause()


async def test_prefs_ctrl_g_opens_manage_agents_and_ctrl_o_does_not(tmp_path):
    """Preferences' two squatters moved off the grammar chords. Both halves in one test so the
    positive and the negative can never drift apart."""
    from skit.tui_runner import RunnerManageScreen

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        screen.query_one("#pf-lang", Select).focus()
        await pilot.pause()

        await pilot.press("ctrl+o")  # the vacated chord
        await pilot.pause()
        assert app.screen is screen  # ...opens nothing

        await pilot.press("ctrl+g")
        await pilot.pause()
        assert isinstance(app.screen, RunnerManageScreen)


async def test_prefs_ctrl_y_installs_the_skill_and_ctrl_k_does_not(tmp_path, monkeypatch):
    """Ctrl+K is every Input's delete-to-end-of-line; on a screen full of text fields it must
    open nothing. Ctrl+Y — a chord with no meaning anywhere else in skit — carries the action."""
    from skit import agentskill
    from skit.tui_prefs import SkillInstallModal

    monkeypatch.setattr(agentskill, "detect_targets", lambda *, home, cwd: [])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        screen.query_one("#pf-lang", Select).focus()
        await pilot.pause()

        await pilot.press("ctrl+k")  # the vacated chord
        await pilot.pause()
        assert app.screen is screen

        await pilot.press("ctrl+y")
        await pilot.pause()
        assert isinstance(app.screen, SkillInstallModal)


async def test_run_form_ctrl_o_still_restores_the_default(tmp_path):
    """The chord the others were cleared FOR: Ctrl+O on the run form restores the field's
    definition default over a remembered value — the one meaning README documents."""
    entry = _managed_entry(tmp_path, name="reset")
    argstate.save_last(entry.slug, values={"CITY": "Kaohsiung"})
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "CITY")
        box = row.query_one(Input)
        assert box.value == "Kaohsiung"
        box.focus()
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        assert box.value == "Taipei"  # the script's own default came back
