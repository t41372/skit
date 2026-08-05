"""Behavior coverage for the design-audit round-10 fixes, headless + CLI half.

R. The non-interactive contract stopped being a `cli.py` local. `--no-input` threaded
   through cli's own gates and no further, so the ONE interactive gate below it
   (``uvman._ask_consent``) re-derived interactivity from ``sys.stdin.isatty()`` — an
   oracle the flag cannot reach — and ``skit run x --no-input`` on a machine without uv
   printed a question and blocked on ``input()`` forever. ``interaction`` holds one
   verdict for the whole invocation, readable at any depth with no parameter to forget.
S. The panic pane told a user whose index had just vanished to "open Health (h)". There
   is no ``h`` binding; it is ``D``. Round 9 wrote that line. The glyph now has ONE
   spelling (``tui.HEALTH_KEY``) and the line is a clickable chip, not prose.
T. …and the status line — the surface that outlives the detail pane at every size tier —
   was the third blank-library surface, still asserting a first run.
U. Entry settings forked on ``kind == "prompt"`` where AGENTS.md says to key off
   ``placeholder_params``, so a `command` entry's "Parameters (the run form's fields)"
   section showed none of them.
V. One ``store.NotFoundError`` had two exit codes: 127 from `run`, 1 from nine other
   commands, against a docs table publishing 127 CLI-wide.
W. ``--forget-args`` cleared the remembered tail ABOVE four gates that can still refuse,
   so a refused invocation destroyed it — the invariant the line's own comment stated.
X. The mirror radios rendered ``on``/``off``/``custom`` untranslated in the one section
   written for Chinese-speaking users, and the i18n gate reported "every scanned UI sink
   routes through gettext" because the literals sat one hop away in a module constant.
Y. Unticking a preset — unrecoverable user data — was the one destructive act in skit
   with no confirmation.
"""

from __future__ import annotations

import ast
import builtins
import sys
import types
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, interaction, store, tui, uvman
from skit.langs.registry import spec_for

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _cmd(name: str, template: str = "echo hi") -> store.Entry:
    return store.add_command(template, name=name)


def _gate_module():
    """The i18n gate, loaded by path — the same idiom tests/test_i18n.py uses (it also
    survives mutmut's copied tree, where scripts/ rides along via also_copy)."""
    import importlib.util

    root = Path(__file__).resolve().parent.parent
    if "mutants" in root.parts:
        root = Path(*root.parts[: root.parts.index("mutants")])
    spec = importlib.util.spec_from_file_location(
        "i18n_coverage_r10", root / "scripts" / "i18n_coverage.py"
    )
    assert spec is not None
    assert spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _tty(monkeypatch: pytest.MonkeyPatch, *, value: bool = True) -> None:
    """All three streams, because `allowed()` asks about stdin plus the stream the
    question would be printed on — stdout for cli.py's prompts, stderr for uvman's."""
    monkeypatch.setattr(sys, "stdin", types.SimpleNamespace(isatty=lambda: value), raising=False)
    monkeypatch.setattr(sys.stdout, "isatty", lambda: value, raising=False)
    monkeypatch.setattr(sys.stderr, "isatty", lambda: value, raising=False)


# ==========================================================================
# R. One interaction verdict, readable below cli.py
# ==========================================================================


def test_a_terminal_may_be_asked_and_a_pipe_may_not(monkeypatch: pytest.MonkeyPatch) -> None:
    """Both ends must be a terminal: skit asks on stderr (stdout belongs to the launched
    script), so a piped stderr means the question would never be seen even though stdin
    could answer it."""
    _tty(monkeypatch, value=True)
    assert interaction.allowed() is True
    assert interaction.allowed(on=sys.stderr) is True

    # skit has TWO answering surfaces and they are not interchangeable: cli.py's
    # Prompt.ask writes to stdout, uvman's consent to stderr (stdout there belongs to
    # the launched script). Asking one question about the other's stream is how the two
    # oracles disagreed in both directions at once.
    monkeypatch.setattr(sys.stdout, "isatty", lambda: False, raising=False)
    assert interaction.allowed() is False  # nobody would see a stdout question…
    assert interaction.allowed(on=sys.stderr) is True  # …but stderr is still watched

    _tty(monkeypatch, value=True)
    monkeypatch.setattr(sys.stderr, "isatty", lambda: False, raising=False)
    assert interaction.allowed() is True
    assert interaction.allowed(on=sys.stderr) is False


def test_forbid_outranks_the_terminal(monkeypatch: pytest.MonkeyPatch) -> None:
    """--no-input is an assertion about the caller, not about the file descriptors: an
    agent driving skit from a pty has two terminals and still must never be asked."""
    _tty(monkeypatch, value=True)
    interaction.forbid()
    assert interaction.allowed() is False
    assert interaction.allowed(on=sys.stderr) is False  # …on every surface
    interaction.reset()
    assert interaction.allowed() is True


def test_the_gate_below_cli_reads_the_verdict_not_isatty(monkeypatch: pytest.MonkeyPatch) -> None:
    """THE round-10 HIGH, at the exact line it broke. uvman._ask_consent lives four calls
    below cli.py (flows.execute → launcher.run_entry → UvLaunch.build → ensure_uv), so no
    amount of threading inside cli.py could reach it and `--no-input` blocked on input()
    forever — precisely what SKILL.md promises agents cannot happen."""
    _tty(monkeypatch, value=True)

    def _boom() -> str:
        raise AssertionError("prompted under --no-input")

    monkeypatch.setattr(builtins, "input", _boom)
    interaction.forbid()

    # Refused: takes the path a pipe already takes (proceed silently, A9), never a prompt.
    assert uvman._ask_consent(Path("/tmp/x")) is True


def test_the_same_gate_still_asks_a_real_terminal(monkeypatch: pytest.MonkeyPatch) -> None:
    """The complement that keeps the fix honest: pulling an executable off the network
    still is not silent for a person sitting at a terminal."""
    _tty(monkeypatch, value=True)
    asked: list[str] = []
    monkeypatch.setattr(builtins, "input", lambda: asked.append("asked") or "y")

    assert uvman._ask_consent(Path("/tmp/x")) is True
    assert asked == ["asked"]


@pytest.mark.parametrize(
    "argv",
    [
        ["add", "--cmd", "echo hi", "-n", "x", "--no-input"],
        ["remove", "x", "--yes", "--no-input"],
        ["run", "x", "--no-input"],
    ],
)
def test_every_no_input_command_records_the_verdict(
    argv: list[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """The flag has to REACH interaction, or the module is decoration. Checked through the
    real CLI for each command that offers --no-input, because a command that forgets the
    call is exactly the bug this replaces."""
    if argv[0] != "add":
        _cmd("x")
    _tty(monkeypatch, value=True)
    interaction.reset()

    runner.invoke(cli.app, argv)

    assert interaction.allowed() is False


def test_a_command_without_the_flag_leaves_the_terminal_askable(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """forbid() is one-way within an invocation, but it must not be set by an invocation
    that never asked for it — otherwise the first run's consent question disappears."""
    _cmd("x")
    _tty(monkeypatch, value=True)
    interaction.reset()

    runner.invoke(cli.app, ["show", "x"])

    assert interaction.allowed() is True


# ==========================================================================
# S/T. Three blank-library surfaces, one question
# ==========================================================================


def test_the_health_key_has_exactly_one_spelling() -> None:
    """Round 9 hand-wrote the glyph in prose and got it wrong, on the one screen where the
    reader has a single instruction and no patience. The binding, the footer chip, the help
    overlay and the recovery line now all read HEALTH_KEY, so a rebind cannot leave prose
    behind — and `h`, which the panic pane advertised, is bound to nothing."""
    assert tui.HEALTH_KEY == "D"
    bound = {b.key for b in tui.MenuApp.BINDINGS if getattr(b, "action", "") == "health"}
    assert bound == {tui.HEALTH_KEY}
    # …and nothing is bound to the glyph the round-9 prose advertised.
    assert "h" not in {b.key for b in tui.MenuApp.BINDINGS}


def test_the_recovery_line_is_a_clickable_chip(monkeypatch: pytest.MonkeyPatch) -> None:
    """Principle 2: every action needs a mouse path. Prose naming a key has none — and the
    detail pane renders markup, so there was never a reason for this one to be prose."""
    _cmd("one")
    store.registry_path().unlink()

    lines = tui.MenuApp._blank_library_lines()

    assert any("@click=app.health" in line for line in lines)
    assert any(tui.HEALTH_KEY in line for line in lines)


def test_the_status_line_asks_the_same_question_the_pane_does() -> None:
    """The THIRD blank surface. `#detail` is display:none at -h-short/-h-tiny, and
    tui_layout documents the status line as "the error/feedback channel that stays at
    every tier" — so on a small terminal it is the only blank-state copy there is, and it
    was the one still asserting a first run over an intact library."""
    assert tui.MenuApp._lost_index_count() == 0
    _cmd("one")
    store.registry_path().unlink()
    assert tui.MenuApp._lost_index_count() == 1


async def test_the_status_line_survives_the_tier_that_hides_the_pane(tmp_path) -> None:
    """Proved at the size where it matters: the detail pane is gone, so whatever the status
    line says is the entire answer the user gets."""
    _cmd("one")
    store.registry_path().unlink()
    app = tui.MenuApp()
    async with app.run_test(size=(70, 10)) as pilot:
        await pilot.pause()
        from textual.widgets import Static

        status = str(app.screen.query_one("#status", Static).render())
        detail = app.screen.query("#detail")
        visible = bool(detail) and detail.first().display
    assert not visible  # the tier really did drop the pane...
    assert "index lost" in status  # ...and the surviving line tells the truth
    assert tui.HEALTH_KEY in status


async def test_a_first_run_still_gets_the_invitation(tmp_path) -> None:
    """The other half: with a genuinely empty library the recovery copy would be nonsense."""
    app = tui.MenuApp()
    async with app.run_test(size=(70, 10)) as pilot:
        await pilot.pause()
        from textual.widgets import Static

        status = str(app.screen.query_one("#status", Static).render())
    assert "Your entries will appear here." in status


# ==========================================================================
# U. Entry settings keys off the trait, not the kind
# ==========================================================================


async def test_a_command_entry_shows_the_run_forms_fields(tmp_path) -> None:
    """The heading promises "Parameters (the run form's fields)" and showed none of them
    for a command entry: to give {width} a type you had to retype its name from memory into
    "Add a parameter", with no list in front of you. The prompt branch ten lines away in the
    same file already did it right — which made this the kind's exception, not the rule."""
    from skit.tui_settings import DeclParamRow, ScriptSettingsScreen

    entry = _cmd("greet", "echo {greeting} {name}")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        rows = [r.decl.name for r in screen.query(DeclParamRow)]
    assert rows == ["greeting", "name"]


def test_the_fork_reads_the_trait_every_other_surface_reads() -> None:
    """AGENTS.md names this exact shape: template/non-template decisions key off
    placeholder_params, never off family — and `kind == "prompt"` is a level worse than the
    spelling the rule forbids. Both placeholder kinds must qualify, and no other kind may."""
    placeholder_kinds = {k for k in ("command", "prompt", "python", "shell", "exe")
                        if (s := spec_for(k)) is not None and s.placeholder_params}  # fmt: skip
    assert placeholder_kinds == {"command", "prompt"}


# ==========================================================================
# V. One error, one exit code
# ==========================================================================


@pytest.mark.parametrize(
    "argv",
    [
        ["show", "ghost"],
        ["params", "ghost"],
        ["deps", "ghost"],
        ["describe", "ghost", "x"],
        ["rename", "ghost", "x"],
        ["remove", "ghost", "--yes"],
        ["preset", "list", "ghost"],
        ["preset", "save", "ghost", "p", "--from-last"],
        ["preset", "delete", "ghost", "p", "--yes"],
        ["run", "ghost", "--no-input"],
    ],
)
def test_no_such_entry_is_127_everywhere(argv: list[str]) -> None:
    """The same store.NotFoundError, from the same store.resolve, used to answer 127 from
    `run` and 1 from everything else — while docs/content/docs/cli.mdx published 127
    CLI-wide and SKILL.md told agents to trust exit codes over output text. 1 is also
    inside the 1-124 band the same page reserves for the launched script, so an agent had
    no way at all to tell "no such entry" from "the script failed"."""
    assert runner.invoke(cli.app, argv).exit_code == 127


def test_a_name_collision_is_still_a_different_failure(tmp_path: Path) -> None:
    """rename catches two errors and only one of them is "no such entry": a collision is a
    usage problem with the NEW name, and folding it into 127 would make the code a lie in
    the other direction."""
    _cmd("a")
    _cmd("b")

    result = runner.invoke(cli.app, ["rename", "a", "b"])

    assert result.exit_code == 2
    assert "already taken" in result.output


# ==========================================================================
# W. --forget-args happens when the invocation is accepted, not before
# ==========================================================================


def _tail(slug: str) -> list[str]:
    return argstate.load_state(slug)["extra_args"]


def test_a_refused_run_no_longer_destroys_the_tail() -> None:
    """The clear sat ABOVE four gates that can still refuse (an unresolvable runner → 126,
    --save-preset on a field-less entry → 2, an unknown --set name → 2, a headless
    validation error → 125), while its own comment stated the invariant it was breaking:
    an exit-2 invocation must leave no fingerprints. A rule enforced by WHERE a line sits
    is a rule the next gate breaks by being added above it."""
    entry = _cmd("ec")
    argstate.save_last(entry.slug, extra_args=["hello", "world"])

    result = runner.invoke(
        cli.app, ["run", entry.slug, "--no-input", "--forget-args", "--set", "nope=1"]
    )

    assert result.exit_code == 2
    assert _tail(entry.slug) == ["hello", "world"]


def test_a_refused_save_preset_leaves_the_tail_too() -> None:
    """The second gate below the old placement, so the fix is proved against the ordering
    and not against one message."""
    entry = _cmd("ec")
    argstate.save_last(entry.slug, extra_args=["alpha"])

    result = runner.invoke(
        cli.app, ["run", entry.slug, "--no-input", "--forget-args", "--save-preset", "p"]
    )

    assert result.exit_code == 2
    assert _tail(entry.slug) == ["alpha"]


def test_an_accepted_run_still_forgets() -> None:
    """The command has to keep doing what it says. Both halves matter: the tail is cleared
    AND it is not replayed on the way past — "forget it" that replays the tail one last
    time, then writes it straight back after the run, forgets nothing."""
    entry = _cmd("ec")
    argstate.save_last(entry.slug, extra_args=["hello", "world"])

    result = runner.invoke(cli.app, ["run", entry.slug, "--no-input", "--forget-args"])

    assert result.exit_code == 0
    assert _tail(entry.slug) == []
    assert "Reusing your last arguments" not in result.output


def test_forgetting_does_not_disturb_the_remembered_values() -> None:
    """--forget-args names the argv tail and nothing else."""
    entry = _cmd("ec", "echo {msg}")
    runner.invoke(cli.app, ["run", entry.slug, "--set", "msg=hi", "--no-input"])
    argstate.save_last(entry.slug, extra_args=["tail"])

    runner.invoke(cli.app, ["run", entry.slug, "--no-input", "--forget-args"])

    assert _tail(entry.slug) == []
    assert argstate.load_state(entry.slug)["values"] == {"msg": "hi"}


# ==========================================================================
# X. The mirror radios speak the user's language; the gate can see them
# ==========================================================================


def test_the_radio_label_and_the_stored_token_are_two_vocabularies() -> None:
    """The token is what the CLI accepts and what lands in config.toml (`skit config
    mirror off`) and must never be localized; the label is UI copy and must always be.
    Vendor names are proper nouns and pass through."""
    from skit import i18n
    from skit.tui_prefs import _choice_label

    assert _choice_label("tsinghua") == "tsinghua"
    try:
        i18n.init("zh_TW")
        for token in ("on", "off", "custom"):
            assert _choice_label(token) != token, token
    finally:
        i18n.init("en")
    assert _choice_label("on") == "on"  # English is the identity locale


def test_the_gate_can_see_a_literal_one_hop_away(tmp_path: Path) -> None:
    """The scanner only inspected a literal sitting DIRECTLY in the sink argument, so
    `for choice in _CHOICES: RadioButton(choice)` hid eight English labels behind a module
    constant — and the gate printed "every scanned UI sink routes through gettext" anyway.
    A gate that publishes a claim it never checked is worse than no gate."""
    i18n_coverage = _gate_module()

    pkg = tmp_path / "pkg"
    pkg.mkdir()
    (pkg / "screen.py").write_text(
        "_CHOICES = ['Save the file', 'off']\n"
        "def compose():\n"
        "    for choice in _CHOICES:\n"
        "        yield RadioButton(choice)\n",
        encoding="utf-8",
    )

    problems = i18n_coverage.scan_unwrapped(pkg)

    assert any("Save the file" in p for p in problems)


def test_the_resolver_is_one_hop_and_module_scope_only(tmp_path: Path) -> None:
    """Deliberately not a dataflow engine: a heuristic that chased arbitrary bindings would
    report things it cannot justify, and a gate nobody trusts gets suppressed. A local
    name stays out of scope, and a gettext-wrapped sequence stays clean."""
    i18n_coverage = _gate_module()

    tree = ast.parse("_A = ['x']\n_B = ('y',)\n_C = 'z'\ndef f():\n    _D = ['w']\n")
    seqs = i18n_coverage._module_literal_sequences(tree)

    assert set(seqs) == {"_A", "_B"}  # sequences only, module level only


# ==========================================================================
# Y. The fifth destructive door gets the ask the other four get
# ==========================================================================


async def test_cancelling_the_confirm_keeps_the_preset(tmp_path) -> None:
    """An untick plus the Ctrl+S the user pressed for an unrelated edit used to delete the
    preset silently, with no undo — while `skit preset delete`, entry removal, runner
    removal and draft deletion all ask. Keeping must really keep: nothing is written, and
    the screen stays open so the edit is not lost either."""
    from textual.widgets import Checkbox

    from skit.tui_settings import PresetDeleteConfirm, ScriptSettingsScreen

    entry = _cmd("pre")
    argstate.save_preset(entry.slug, "alpha", {"X": "1"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-preset-0", Checkbox).value = False
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        confirm = app.screen
        assert isinstance(confirm, PresetDeleteConfirm)
        confirm.action_cancel()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # still editing

    assert set(argstate.load_state(entry.slug)["presets"]) == {"alpha"}


async def test_a_save_with_no_deletion_never_asks(tmp_path) -> None:
    """The ask is scoped to the destructive part: an ordinary save must not grow a modal."""
    from skit.tui_settings import ScriptSettingsScreen

    entry = _cmd("pre")
    argstate.save_preset(entry.slug, "alpha", {"X": "1"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve(entry.slug))
        app.push_screen(screen)
        await pilot.pause()
        screen.action_save()
        await pilot.pause()

    assert set(argstate.load_state(entry.slug)["presets"]) == {"alpha"}


def test_the_question_names_the_presets_it_is_about() -> None:
    """ "Are you sure?" about an unnamed thing is not a question anyone can answer — and
    the names are exactly what the user needs to notice they unticked the wrong row."""
    from skit.tui_settings import PresetDeleteConfirm

    assert PresetDeleteConfirm(["alpha", "beta"])._names == ["alpha", "beta"]
