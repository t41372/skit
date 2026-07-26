"""Behavior coverage for the design-audit fixes (rounds 1 and 2), TUI half.

A. THE round-1 HIGH: the add-review panel wrote parameter blocks back with
   read_text/write_text, silently rewriting every line ending of the just-stored copy. Pinned
   at the SCREEN level — the helper-level round-trip lives in tests/test_design_audit_fixes.py,
   but this is the surface where the corruption actually shipped.
D. Extra-args provenance, TUI face: `r` replays a CLI-captured tail literally, the form's own
   tail expands, and the marker rides through the deferred (exit-mode) save on PendingRun.
E. The Library detail pane's plan cache: keyed on the script AND its meta.toml, popped by edit
   and by the settings-close callback, and _has_drift served off the very same entry.
F. AddReviewScreen._reader_modeled memoizes its probe — for a reader kind each call is a
   synchronous subprocess and the panel must not freeze because a radio button was clicked.
G. The chord grammar repair: Ctrl+O meant three things on three sibling screens while README
   documents one. Choose variables → Ctrl+L (both screens), Preferences' squatters → Ctrl+G /
   Ctrl+Y, and every moved chord gets a positive pilot test plus a negative for the vacated one.
"""

from __future__ import annotations

import contextlib
from pathlib import Path

import pytest
from textual.widgets import Checkbox, Input, Select

from skit import argstate, config, flows, launcher, store, tui
from skit.langs.python import metawriter
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


def _py(tmp_path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _plan_key(entry) -> tuple[float, float]:
    return (entry.script_path.stat().st_mtime, (entry.dir / "meta.toml").stat().st_mtime)


def _detail_text(app) -> str:
    return " ".join(str(s.render()) for s in app.query("#detail-body Static"))


# ==========================================================================
# A. the add-review panel's write-back keeps the copy's bytes
# ==========================================================================

_CRLF_SHELL = b'#!/usr/bin/env bash\r\nWIDTH=800\r\necho "$WIDTH"\r\n'


def _without_block(raw: bytes, newline: bytes) -> bytes:
    """Drop the inserted comment block, keeping every other byte — terminators included —
    exactly where it lies, so the comparison is a real byte-for-byte claim about the rest of
    the file rather than a normalized diff."""
    keep: list[bytes] = []
    inside = False
    for chunk in raw.split(newline):
        if chunk == b"# /// script":
            inside = True
            continue
        if inside:
            inside = chunk != b"# ///"
            continue
        keep.append(chunk)
    return newline.join(keep)


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
    assert _without_block(stored, b"\r\n") == _CRLF_SHELL


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
    assert _without_block(stored, b"\n") == original


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


async def test_form_submit_expands_its_tail_and_records_it_raw(tmp_path, quiet_run):
    """The form's extra field has no shell behind it, so its tokens expand on the way out
    (extra_raw defaults to True on _execute) and the tail is remembered MARKED, so `skit run`
    replaying it later expands it the same way."""
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

    (tail,) = quiet_run["extra"]
    assert tail != "out_{today}.txt"  # the form's own text expanded on the way out
    assert tail.startswith("out_20")
    assert tail.endswith(".txt")
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["out_{today}.txt"]  # intent persisted, never expansion
    assert state["extra_args_raw"] is True  # ...marked, so the CLI replays it the same way


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
    monkeypatch.setattr(flows, "execute", lambda *a, **k: flows.RunOutcome(0, "", ""))

    tui._finish_run(
        tui.PendingRun(entry, plan, asm, {}, ["out_{today}.txt"], extra_raw=True, show_drift=False)
    )
    assert argstate.load_state(entry.slug)["extra_args_raw"] is True

    tui._finish_run(
        tui.PendingRun(entry, plan, asm, {}, ["out_{today}.txt"], extra_raw=False, show_drift=False)
    )
    assert argstate.load_state(entry.slug)["extra_args_raw"] is False


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
        app._plan_cache[entry.slug] = (_plan_key(entry), planted)
        app.action_edit()
        await pilot.pause()
        assert app._plan_cache.get(entry.slug, (None, None))[1] is not planted


async def test_settings_close_pops_this_entrys_cache_key(tmp_path):
    """A Resync inside Entry settings can change the definitions without touching the script,
    so the close callback drops the key too."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_settings()
        await pilot.pause()
        planted = flows.FormPlan(source="none", drift_lines=["planted"])
        app._plan_cache[entry.slug] = (_plan_key(entry), planted)
        app.screen.dismiss(False)
        await pilot.pause()
        assert app._plan_cache.get(entry.slug, (None, None))[1] is not planted
        assert "The script changed" not in _detail_text(app)


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
