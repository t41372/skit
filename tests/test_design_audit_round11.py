"""Behavior coverage for the design-audit round-11 fixes, headless + CLI half.

Z.  `skit edit` was the one editor door with no interactivity gate — and the door the
    bundled Agent Skill teaches. In a pipe it spawned $EDITOR against a stdin nobody was
    typing into (`vi` hung forever, `cat` dumped the file into the caller's stdout) and
    skit then printed "Saved" about an edit that could not have happened. The gate now
    lives in ``editor.open_in_editor``, where all four lanes pass.
AA. Ctrl+L on Entry settings read `#st-interpolate` before its pure-Python guard, so on
    every non-prompt entry it raised NoMatches and took the whole workbench down — with
    every unsaved edit on the screen. Ctrl+L is also the terminal's clear-screen reflex.
BB. The preset-delete confirmation was a BOOLEAN that is set once and never cleared, so a
    save that confirmed one deletion and then aborted on an unrelated validation error let
    the NEXT untick through unasked — round 10's own fix, re-opened by its own flag.
CC. Two interactivity oracles disagreed about the same terminal in both directions:
    `skit run x > out` had cli decline to prompt while uvman blocked on one; `skit run x
    2> log` had cli open a form while uvman silently downloaded and executed a network
    binary with no consent. One oracle now, told which stream it is answering about.
DD. `skit remove` repeated "your original file will not be deleted" for a copy-mode entry
    whose original the user had already deleted — trusting exactly that promise. The TUI
    modal already withheld the line.
EE. `--plain` could not spell a cleared delivers-empty field, and a degraded language
    parser told shell users that "programs have no managed parameters".
"""

from __future__ import annotations

import subprocess
import sys
import types
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, editor, interaction, launcher, store, tui
from skit.tui_settings import ScriptSettingsScreen

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _py(tmp_path: Path, body: str = "W = 3\nprint(W)\n", name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _terminal(monkeypatch: pytest.MonkeyPatch, *, value: bool = True) -> None:
    monkeypatch.setattr(sys, "stdin", types.SimpleNamespace(isatty=lambda: value), raising=False)
    monkeypatch.setattr(sys.stdout, "isatty", lambda: value, raising=False)
    monkeypatch.setattr(sys.stderr, "isatty", lambda: value, raising=False)


# ==========================================================================
# Z. Every editor door passes one gate
# ==========================================================================


def test_the_editor_refuses_when_skit_may_not_prompt(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """An editor session IS interaction — the words the two lanes that already refused
    use. In a pipe this spawned the editor anyway, and `cat` as $EDITOR dumped the file
    into the caller's stdout while skit reported a save. The refusal names the stored
    path, which is the thing a non-interactive caller can actually act on."""
    spawned: list[list[str]] = []
    monkeypatch.setattr(editor.subprocess, "run", lambda argv, check=False: spawned.append(argv))
    _terminal(monkeypatch, value=False)

    with pytest.raises(editor.EditorError) as excinfo:
        editor.open_in_editor(tmp_path / "x.py")

    assert spawned == []  # nothing was launched
    # The WHOLE sentence: a substring check passes on copy that has quietly become
    # something else, and this one has to name the escape (the file) to be worth printing.
    assert str(excinfo.value) == (
        "Opening an editor needs an interactive terminal — not a pipe, CI, or "
        f"--no-input. Edit the file directly instead: {tmp_path / 'x.py'}"
    )


def test_skit_edit_refuses_in_a_pipe_instead_of_claiming_a_save(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The user-visible shape, on the command SKILL.md hands to agents. The old path
    printed "Saved hello." about an edit that never happened — a lie the caller had no
    way to detect."""
    store.add_python(_py(tmp_path), name="hello")
    monkeypatch.setattr(editor.subprocess, "run", lambda *a, **k: pytest.fail("editor spawned"))
    _terminal(monkeypatch, value=False)

    result = runner.invoke(cli.app, ["edit", "hello"])

    # ROUND 12 moved the refusal to `edit`'s FRONT DOOR: down in editor.py it shared one
    # exception class with "the editor could not be launched", so the two could only ever
    # get one exit code. Here it is a usage refusal (2, like its `add --edit` twin) and it
    # can name the resolved file — the thing a non-interactive caller can act on.
    assert result.exit_code == 2
    assert "Saved" not in result.output
    assert "interactive terminal" in result.output


def test_the_editor_still_opens_at_a_terminal(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The complement: the gate must not break the ordinary case it exists to protect."""
    spawned: list[list[str]] = []

    class _Ok:
        returncode = 0

    monkeypatch.setattr(
        editor.subprocess, "run", lambda argv, check=False: (spawned.append(argv), _Ok())[1]
    )
    monkeypatch.setenv("EDITOR", "myed")
    _terminal(monkeypatch, value=True)

    assert editor.open_in_editor(tmp_path / "x.py") == 0
    assert spawned
    assert spawned[0][0] == "myed"


def test_no_input_reaches_the_editor_gate_too(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The gate reads interaction, not a local isatty pair — so a caller sitting at a real
    terminal that has DECLARED itself non-interactive is refused as well. That is the
    round-10 lesson applied one layer up, and it is what makes `--no-input` a contract
    rather than a per-command habit."""
    monkeypatch.setattr(editor.subprocess, "run", lambda *a, **k: pytest.fail("editor spawned"))
    _terminal(monkeypatch, value=True)
    interaction.forbid()

    with pytest.raises(editor.EditorError):
        editor.open_in_editor(tmp_path / "x.py")


# ==========================================================================
# AA. A chord that cannot work is disabled, not merely inert
# ==========================================================================


async def test_ctrl_l_no_longer_takes_the_workbench_down(tmp_path: Path) -> None:
    """THE round-11 HIGH. #st-interpolate is composed only for prompts, and the DOM read
    came BEFORE the pure-Python guard — so Ctrl+L on any python/shell/js/exe/command entry
    raised NoMatches out of the action handler and killed the app, losing every unsaved
    edit on the screen. Ctrl+L is the terminal's universal clear-screen reflex: it gets
    pressed by people who meant nothing by it."""
    entry = store.add_command("echo hi", name="c")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        screen.action_choose_prompt_candidates()  # total, even called directly
        await pilot.press("ctrl+l")
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # still alive


async def test_the_chord_is_disabled_where_it_cannot_work(tmp_path: Path) -> None:
    """Disabled, not silently inert: the Ctrl+R chip beside it already states the rule
    ("advertising a key that silently no-ops teaches a dead chord"), and check_action makes
    the binding obey the same predicate the chip is built from — so the keyboard can never
    advertise what the mouse doesn't."""
    entry = store.add_command("echo hi", name="c")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        assert screen.check_action("choose_prompt_candidates", ()) is False
        assert screen.check_action("new_runner", ()) is False
        assert screen.check_action("save", ()) is True  # …and ordinary chords are untouched


async def test_a_prompt_with_few_variables_advertises_neither(tmp_path: Path) -> None:
    """The picker exists for the overflow case. A prompt whose whole candidate list is
    already on screen has nothing to open, so the chord is disabled there too — one
    predicate, so the chip and the binding agree at every size of list."""
    body = tmp_path / "p.prompt.md"
    body.write_text("Summarise {{topic}}.\n", encoding="utf-8")
    entry = store.add_prompt(body, name="p")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        assert screen._can_choose_candidates() is False
        assert screen.check_action("choose_prompt_candidates", ()) is False
        assert screen.check_action("new_runner", ()) is True  # prompt-only, and this IS one


async def test_the_picker_opens_only_once_the_list_actually_overflows(tmp_path: Path) -> None:
    """The boundary, because the chip is built from the same comparison: a list of exactly
    LIST_PREVIEW_LIMIT names is fully on screen, so there is nothing the picker could show
    that the user cannot already see and tick. One more, and there is."""
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    body = tmp_path / "p.prompt.md"
    body.write_text("Do {{a}}.\n", encoding="utf-8")
    entry = store.add_prompt(body, name="p")
    stored = store.resolve(entry.slug).script_path

    async def _can(count: int) -> bool:
        stored.write_text(" ".join(f"{{{{v{i}}}}}" for i in range(count)) + "\n", encoding="utf-8")
        app = tui.MenuApp()
        async with app.run_test() as pilot:
            screen = ScriptSettingsScreen(store.resolve(entry.slug))
            app.push_screen(screen)
            await pilot.pause()
            return screen._can_choose_candidates()

    assert await _can(LIST_PREVIEW_LIMIT) is False
    assert await _can(LIST_PREVIEW_LIMIT + 1) is True


# ==========================================================================
# BB. The confirmation tracks names, not a latch
# ==========================================================================


async def test_a_second_untick_after_an_aborted_save_is_still_asked_about(
    tmp_path: Path,
) -> None:
    """Round 10's fix, re-opened by its own flag: a boolean set on confirm and never
    cleared. Confirm `alpha`, abort the save on an unrelated validation error (an invalid
    workdir), then untick `beta` — the latch was still standing, so `beta` was deleted
    without any question ever naming it. Confirming against WHAT THIS SAVE WOULD DELETE is
    correct under retick/untick churn, where a reset-on-abort boolean still is not."""
    from textual.widgets import Checkbox, Input

    from skit import argstate
    from skit.tui_settings import PresetDeleteConfirm

    entry = store.add_command("echo hi", name="pre")
    argstate.save_preset(entry.slug, "alpha", {"X": "1"})
    argstate.save_preset(entry.slug, "beta", {"Y": "2"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-preset-0", Checkbox).value = False  # untick alpha
        # …and break the save on something unrelated: an emptied command template.
        screen.query_one("#st-template", Input).value = "   "
        await pilot.pause()

        screen.action_save()
        await pilot.pause()
        confirm = app.screen
        assert isinstance(confirm, PresetDeleteConfirm)
        confirm.action_confirm()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # the save aborted
        assert set(argstate.load_state(entry.slug)["presets"]) == {"alpha", "beta"}

        screen.query_one("#st-preset-1", Checkbox).value = False  # now untick beta too
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        second = app.screen
        assert isinstance(second, PresetDeleteConfirm)  # asked again…
        assert second._names == ["beta"]  # …and only about the name never agreed to


# ==========================================================================
# CC. One oracle, told which stream it answers about
# ==========================================================================


def test_the_cli_and_the_gates_below_it_share_one_oracle(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Two implementations of "is anyone there?" disagreed about the same terminal in both
    directions, and the flag that forbids prompting could only reach one of them. cli's
    name for the question now delegates, so a refusal cannot apply to half of skit."""
    _terminal(monkeypatch, value=True)
    assert cli._is_interactive() is True

    interaction.forbid()
    assert cli._is_interactive() is False  # --no-input now reaches the CLI's own gates too


# ==========================================================================
# DD. The removal promise is only made when it holds
# ==========================================================================


def test_the_removal_question_is_honest_about_the_only_copy(tmp_path: Path) -> None:
    """skit's copy-mode pitch ("your original is safe") is exactly why people delete their
    working file — which makes the moment the promise stops holding the moment it must not
    be repeated, on the destructive door, in the face that has no Esc."""
    original = _py(tmp_path, name="only.py")
    entry = store.add_python(original, name="onlycopy")
    kept = store.add_python(_py(tmp_path, name="kept.py"), name="kept")
    template = store.add_command("echo hi", name="tmpl")
    original.unlink()  # the user trusted the promise and deleted their working file

    gone = cli._remove_question(store.resolve(entry.slug))
    safe = cli._remove_question(store.resolve(kept.slug))
    plain = cli._remove_question(store.resolve(template.slug))

    assert gone == 'Remove "onlycopy"? skit holds the only copy — it will be gone.'
    assert safe == 'Remove "kept"? Your original file will not be deleted.'
    assert plain == 'Remove "tmpl"?'  # a template has no original to speak about


def test_both_removal_faces_ask_the_same_predicate(tmp_path: Path) -> None:
    """The CLI asked only whether the KIND has an original; the TUI asked whether the file
    is still there. One predicate now, so the two faces cannot promise different things
    about one entry."""
    original = _py(tmp_path, name="only.py")
    entry = store.add_python(original, name="onlycopy")
    assert launcher.original_survives(store.resolve(entry.slug)) is True

    original.unlink()
    assert launcher.original_survives(store.resolve(entry.slug)) is False
    # …and a kind with no original at all never claims to have a surviving one.
    assert launcher.original_survives(store.add_command("echo hi", name="t")) is False


async def test_the_tui_modal_withholds_the_line_too(tmp_path: Path) -> None:
    """The face that already got this right must keep getting it right through the shared
    predicate — a refactor that fixed one caller and broke the other would be a net loss."""
    from textual.widgets import Static

    original = _py(tmp_path, name="only.py")
    entry = store.add_python(original, name="onlycopy")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(tui.ConfirmRemove(store.resolve(entry.slug)))
        await pilot.pause()
        with_original = "\n".join(str(w.render()) for w in app.screen.query(Static))
    assert "will not be deleted" in with_original

    original.unlink()
    app2 = tui.MenuApp()
    async with app2.run_test() as pilot:
        app2.push_screen(tui.ConfirmRemove(store.resolve(entry.slug)))
        await pilot.pause()
        without = "\n".join(str(w.render()) for w in app2.screen.query(Static))
    assert "will not be deleted" not in without


# ==========================================================================
# EE. Two smaller honesty fixes
# ==========================================================================


def test_the_plain_form_names_the_spelling_it_cannot_offer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A delivers-empty field can be cleared in the TUI (empty the Input) and by `--set
    NAME=`, but the line form's Enter means "keep the default" and nothing means "clear".
    No sentinel token was invented: '-' is the house clear word where it can never be a
    real value, and on a free-text or path field it very much can be. So the form names
    the spelling that works."""
    from rich.console import Console

    from skit import flows, promptform

    field = flows.FormField(
        key="OUT", label="OUT", kind="str", source="inject", default="build", has_default=True
    )
    assert field.delivers_empty is True
    console = Console(force_terminal=False, no_color=True, width=100)
    monkeypatch.setattr(promptform.Prompt, "ask", staticmethod(lambda *a, **k: "build"))

    with console.capture() as cap:
        promptform._ask_once(field, "build", console)

    assert "Enter keeps it; to send an empty value, run with --set OUT=" in cap.get()


def test_the_hint_is_scoped_to_fields_that_can_actually_deliver_empty(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Both halves of the condition. A field with a default that does NOT deliver empty
    (an int, where '' is never a value) must not be told to send one — the hint would name
    a spelling that changes nothing — and a delivers-empty field with no default has
    nothing to keep, so there is nothing for Enter to preserve either."""
    from rich.console import Console

    from skit import flows, promptform

    monkeypatch.setattr(promptform.Prompt, "ask", staticmethod(lambda *a, **k: "1"))
    counted = flows.FormField(
        key="N", label="N", kind="int", source="inject", default="1", has_default=True
    )
    assert counted.delivers_empty is False
    console = Console(force_terminal=False, no_color=True, width=100)
    with console.capture() as cap:
        promptform._ask_once(counted, "1", console)
    assert "--set" not in cap.get()

    blank = flows.FormField(key="OUT", label="OUT", kind="str", source="inject")
    assert blank.delivers_empty is False  # no known default to keep
    with console.capture() as cap:
        promptform._ask_once(blank, "", console)
    assert "--set" not in cap.get()


async def test_a_degraded_parser_says_so_instead_of_calling_a_script_a_program(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Every kind that HAS params_io also has an analyzer, so an analyzer of None means the
    A2 degradation fired: the language's parser package failed to import. Telling a shell
    user "programs have no managed parameters" said something false about their script
    instead of something true about their install."""
    from dataclasses import replace as dc_replace

    from textual.widgets import Static

    from skit.langs import registry
    from skit.langs.base import Capabilities

    entry = store.add_script(_py(tmp_path, "X=1\n", name="s.sh"), kind="shell", name="s")
    real = registry.spec_for

    def degraded(kind: str):
        """What the A2 import guard leaves behind when a tree-sitter grammar won't load:
        the spec survives, its analysis capabilities are None."""
        spec = real(kind)
        return spec and dc_replace(spec, capabilities=Capabilities())

    monkeypatch.setattr("skit.tui_settings.spec_for", degraded)

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        body = "\n".join(str(w.render()) for w in screen.query(Static))

    assert "language parser failed to load" in body
    assert "programs have no managed parameters" not in body


def test_subprocess_is_never_reached_without_the_gate() -> None:
    """A structural check on the fix's placement: the refusal sits above the spawn in the
    ONE function every editor lane calls, so a future caller cannot route around it by
    forgetting a check of its own."""
    source = Path(editor.__file__).read_text(encoding="utf-8")
    body = source.split("def open_in_editor")[1]
    assert body.index("interaction.allowed") < body.index("subprocess.run")
    assert isinstance(subprocess.run, type(subprocess.run))  # the real spawn, unpatched here


async def test_the_prompt_only_actions_stay_total_when_called_directly(tmp_path: Path) -> None:
    """check_action filters the CHORD; it does not make the handler safe. Both prompt-only
    actions are reachable programmatically (a click route, a test, a future caller), so
    each keeps its own guard — the belt-and-braces the sibling already had, now applied to
    the one that was missing it."""
    entry = store.add_command("echo hi", name="c")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        screen.action_new_runner()  # no picker, no crash
        screen.action_choose_prompt_candidates()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)


async def test_an_interpolation_off_prompt_opens_no_picker(tmp_path: Path) -> None:
    """The remaining guard inside the action, and it has to stay INSIDE it: the toggle is
    live, so a user can turn insertion off after the screen composed. With insertion off
    nothing is filled at run time, so there are no variables to choose — the same
    "off = no scanning" gate the params CLI enforces."""
    from textual.widgets import Checkbox

    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    body = tmp_path / "p.prompt.md"
    body.write_text("Do {{a}}.\n", encoding="utf-8")
    entry = store.add_prompt(body, name="p")
    # The body grew past what is managed — the ordinary drift the picker exists for.
    many = " ".join("{{v" + str(i) + "}}" for i in range(LIST_PREVIEW_LIMIT + 3))
    store.resolve(entry.slug).script_path.write_text(many + "\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        assert screen._can_choose_candidates() is True  # the list really does overflow
        screen.query_one("#st-interpolate", Checkbox).value = False
        await pilot.pause()
        screen.action_choose_prompt_candidates()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # no picker was pushed


async def test_the_gate_does_not_lock_the_tui_out_of_its_own_editor(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The regression the round-11 gate could have caused, pinned rather than reasoned
    about. Ctrl+E runs INSIDE a live Textual app, where sys.stdout is Textual's
    _PrintCapture rather than the real stream — if that proxy reported isatty() False, the
    new gate would have refused the workbench's own editor and the fix would have cost
    more than the bug. It reports honestly, so a real session passes; this test is what
    keeps that true when Textual changes."""
    spawned: list[list[str]] = []

    class _Ok:
        returncode = 0

    monkeypatch.setattr(
        editor.subprocess, "run", lambda argv, check=False: (spawned.append(argv), _Ok())[1]
    )
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: __import__("contextlib").nullcontext())
    store.add_python(_py(tmp_path), name="hello")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        monkeypatch.setattr(sys, "stdin", types.SimpleNamespace(isatty=lambda: True), raising=False)
        assert interaction.allowed() is True  # the app's own stdout proxy is honest
        app.action_edit()
        await pilot.pause()

    assert spawned, "the TUI's own editor must still open"
