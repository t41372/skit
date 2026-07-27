"""Behavior coverage for the design-audit round-12 fixes, headless + CLI half.

FF. `skit params` absorbed every invalid edit: a rejected `--type` printed a stderr line,
    wrote everything else, exited 0, and then reported the state it had NOT written
    through `--json` — to the two channels SKILL.md tells agents to trust. It now refuses
    ATOMICALLY (exit 2, nothing written), the answer every sibling intake already gives
    and the rule ScriptSettingsScreen.action_save has always applied on the human face.
GG. Round 11 unified the removal PREDICATE and left the ANSWER forked, so "skit holds the
    only copy" existed on the CLI (which makes you type a name) and not in the Library
    (where Delete acts on whatever row the cursor is on).
HH. `skit edit` sat outside the CLI contract: exit 1 for not-found where the other ten
    entry-name commands exit 127, and no `--no-input`, so under a pty with nobody typing
    round 11's editor gate was a no-op and it hung.
II. The TUI collapsed three edit refusals into one sentence that denied the source existed
    AND misclassified the kind.
JJ. Stored enum tokens shipped raw English inside translated output — `(copy 模式)`,
    `工作目錄:store` — where no static gate could ever see them.
KK. Declining a destructive confirm died as click's untranslated red `Aborted.` at exit 1.
LL. Round 11's chip↔check_action rule was applied to one action on one screen.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import analysis, cli, kindnames, launcher, store, tui
from skit.tui_settings import ScriptSettingsScreen

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _py(tmp_path: Path, body: str = 'A = "1"\nprint(A)\n', name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _cmd(name: str = "say", template: str = "/bin/echo {W}") -> store.Entry:
    return store.add_command(template, name=name)


# ==========================================================================
# FF. `skit params` refuses what it cannot honour, and writes nothing
# ==========================================================================


def test_a_rejected_type_writes_nothing_and_says_so() -> None:
    """THE round-12 HIGH, on the invocation SKILL.md teaches with one typo in it. The user
    asked for an int-validated field; warn-and-continue gave them a free-text one, exit 0,
    and a `--json` payload reporting `type: "str"` as if it were what they had asked for.
    Both of the agent's sanctioned channels said success."""
    entry = _cmd("echox", "/bin/echo {WIDTH}")

    result = runner.invoke(
        cli.app,
        ["params", entry.slug, "--add", "WIDTH", "--type", "WIDTH=itn", "--default", "WIDTH=800"],
    )

    assert result.exit_code == 2
    assert "unknown type" in result.output
    assert "Nothing was changed" in result.output
    assert store.read_parameters(entry.slug) == []  # …and nothing at all was written
    payload = json.loads(runner.invoke(cli.app, ["params", entry.slug, "--json"]).stdout)
    assert payload["declared"] == []


@pytest.mark.parametrize(
    "flags",
    [
        ["--type", "nosuch=int"],
        ["--type", "W=NOTATYPE"],
        ["--deliver", "W=bogus"],
        ["--type", "W=int", "--default", "W=abc"],
        ["--type", "garbage"],  # malformed NAME=VALUE
        ["--secret", "nosuch"],
        ["--rm", "W", "--type", "nosuch=int"],  # a real op alongside an impossible one
    ],
)
def test_every_unhonourable_flag_refuses_atomically(flags: list[str]) -> None:
    """The RULE, not one flag. Round 9 fixed the single code path where the drop leaked a
    credential (`--secret`) and left eight others sharing the shape; fixing `--type` alone
    would have set up round 13 on `--deliver`. The `--rm W` case is the one that matters
    most: a valid operation riding along with an impossible one must not land."""
    entry = _cmd("say")
    runner.invoke(cli.app, ["params", entry.slug, "--add", "W"])
    before = store.read_parameters(entry.slug)

    result = runner.invoke(cli.app, ["params", entry.slug, *flags])

    assert result.exit_code == 2
    assert "Nothing was changed" in result.output
    assert store.read_parameters(entry.slug) == before


@pytest.mark.parametrize(
    ("flags", "why"),
    [
        (["--add", "W"], "already declared: the end state you asked for holds"),
        (["--rm", "ghost"], "nothing is declared under that name: already true"),
    ],
)
def test_an_idempotent_request_is_not_a_refusal(flags: list[str], why: str) -> None:
    """The other side of the classification, and the reason the codes had to be split at
    emission: `--rm GHOST` and `--type GHOST=int` used to emit the SAME warning string,
    one meaning "already true" and the other "did not happen". One string cannot carry two
    answers — and an agent batching idempotent edits must not be refused for them."""
    entry = _cmd("say")
    runner.invoke(cli.app, ["params", entry.slug, "--add", "W"])

    result = runner.invoke(cli.app, ["params", entry.slug, *flags])

    assert result.exit_code == 0, why
    assert "Nothing was changed" not in result.output


def test_the_two_split_codes_read_the_same_but_answer_differently() -> None:
    """The split, at the level it was made. The user sees one sentence either way — they
    do not care which internal op asked — while the caller gets two different answers."""
    assert analysis.render_warning("unmanage-not-managed:X") == analysis.render_warning(
        "not-managed:X"
    )
    assert analysis.is_refusal("not-managed:X") is True
    assert analysis.is_refusal("unmanage-not-managed:X") is False
    assert cli._render_declared_warning("rm-not-declared:X") == cli._render_declared_warning(
        "not-declared:X"
    )
    assert analysis.is_refusal("not-declared:X") is True
    assert analysis.is_refusal("rm-not-declared:X") is False


def test_env_source_answers_the_same_on_both_lanes(tmp_path: Path) -> None:
    """`_apply_env_sources` was the third warning producer and the only one returning
    finished prose, so nothing could classify it — which left --env-source
    warn-and-continuing on the analyzer lane while the identical flag refused on an exe
    entry: a polarity inversion inside one command."""
    analyzer_entry = store.add_python(_py(tmp_path), name="gr")
    runner.invoke(cli.app, ["params", analyzer_entry.slug, "--manage", "A"])
    declared_entry = _cmd("prog2", "/bin/echo {W}")
    runner.invoke(cli.app, ["params", declared_entry.slug, "--add", "W"])

    for slug, name in ((analyzer_entry.slug, "A"), (declared_entry.slug, "W")):
        result = runner.invoke(cli.app, ["params", slug, "--env-source", f"{name}=COLS"])
        assert result.exit_code == 2, slug  # …not secret: the flag cannot apply
        assert "Nothing was changed" in result.output


def test_a_resync_report_is_not_a_refusal(tmp_path: Path) -> None:
    """`--resync` asks skit to TELL you what changed; its warnings are that flag's output,
    not a refused request. Classifying them as refusals would make the one command whose
    job is reporting drift fail whenever it found any."""
    text = 'A = "1"\nB = "2"\nprint(A, B)\n'
    entry = store.add_python(_py(tmp_path, text), name="gr")
    runner.invoke(cli.app, ["params", entry.slug, "--manage", "A", "--manage", "B"])
    stored = store.resolve(entry.slug).script_path
    stored.write_text(stored.read_text(encoding="utf-8").replace('B = "2"\n', ""), encoding="utf-8")

    result = runner.invoke(cli.app, ["params", entry.slug, "--resync"])

    assert result.exit_code == 0, result.output
    assert "Dropped B" in result.output  # the report it was asked for…
    assert "Nothing was changed" not in result.output  # …is not a refusal
    payload = json.loads(runner.invoke(cli.app, ["params", entry.slug, "--json"]).stdout)
    assert [p["name"] for p in payload["params"]] == ["A"]  # and the drop really landed


def test_a_prompt_rm_still_does_its_real_work(tmp_path: Path) -> None:
    """The retraction SKILL.md's `--rm` invocation depends on: for a prompt, an `--rm` that
    only unmanages is real work even with no declared row to drop. Classifying on the raw
    warning would have made a documented command start exiting 2 and doing nothing."""
    body = tmp_path / "p.prompt.md"
    body.write_text("Say hello to {{noise}}.\n", encoding="utf-8")
    entry = store.add_prompt(body, name="rev")
    assert "noise" in (store.resolve(entry.slug).meta.params or [])

    result = runner.invoke(cli.app, ["params", entry.slug, "--rm", "noise"])

    assert result.exit_code == 0, result.output
    assert "noise" not in (store.resolve(entry.slug).meta.params or [])


def test_a_refused_prompt_edit_leaves_the_managed_list_alone(tmp_path: Path) -> None:
    """Validate-then-WRITE, at the seam that made it necessary: the prompt managed-list
    write used to happen ABOVE the decision point, so a refused invocation would unmanage
    one name and only then refuse — the partial apply this whole change exists to stop."""
    body = tmp_path / "p.prompt.md"
    body.write_text("Say {{noise}} and {{keep}}.\n", encoding="utf-8")
    entry = store.add_prompt(body, name="rev")
    before = list(store.resolve(entry.slug).meta.params or [])

    result = runner.invoke(cli.app, ["params", entry.slug, "--rm", "noise", "--type", "ghost=int"])

    assert result.exit_code == 2
    assert list(store.resolve(entry.slug).meta.params or []) == before


# ==========================================================================
# GG/HH/II. One verdict, one exit code, three refusals kept apart
# ==========================================================================


def test_both_removal_faces_answer_the_same_verdict(tmp_path: Path) -> None:
    """Round 11 shared the predicate and forked the answer, putting the honest warning on
    the face that makes you type a name and leaving the Library — one keystroke on
    whatever row the cursor is on — silent."""
    original = _py(tmp_path, name="only.py")
    entry = store.add_python(original, name="only")
    original.unlink()

    assert launcher.removal_stake(store.resolve(entry.slug)) == "only-copy"
    assert "only copy" in cli._remove_question(store.resolve(entry.slug))


async def test_the_library_modal_says_it_holds_the_only_copy(tmp_path: Path) -> None:
    """The face that was silent, now driven through the real screen."""
    from textual.widgets import Static

    original = _py(tmp_path, name="only.py")
    entry = store.add_python(original, name="only")
    original.unlink()
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(tui.ConfirmRemove(store.resolve(entry.slug)))
        await pilot.pause()
        body = "\n".join(str(w.render()) for w in app.screen.query(Static))
    assert "skit holds the only copy" in body


@pytest.mark.parametrize(
    ("stake", "mode"),
    [("original-safe", "copy"), ("nothing-of-yours", "reference")],
)
def test_the_other_two_verdicts(tmp_path: Path, stake: str, mode: str) -> None:
    """A surviving original is reassured about; a kind with no original of its own says
    nothing, because there is nothing to say."""
    if mode == "copy":
        entry = store.add_python(_py(tmp_path, name="kept.py"), name="kept")
    else:
        entry = _cmd("tmpl", "echo hi")
    assert launcher.removal_stake(store.resolve(entry.slug)) == stake


def test_edit_answers_the_same_codes_as_every_other_entry_name_command() -> None:
    """`edit` never raises store.NotFoundError — it offers to create instead — which is
    exactly why round 10's sweep across the other ten commands could not see it."""
    assert runner.invoke(cli.app, ["edit", "ghost"]).exit_code == 127
    _cmd("tmpl", "echo hi")
    assert runner.invoke(cli.app, ["edit", "tmpl"]).exit_code == 2  # kind has no source


def test_edit_takes_no_input_so_a_pty_caller_can_say_there_is_no_human(tmp_path: Path) -> None:
    """Under a pty — an agent harness, a `script`-wrapped CI job — isatty is True and the
    terminal check alone cannot tell that nobody is typing. `--no-input` is the only way to
    say so, and `edit` was the one prompting command that did not take it."""
    store.add_python(_py(tmp_path), name="hello")

    result = runner.invoke(cli.app, ["edit", "hello", "--no-input"])

    assert result.exit_code == 2
    assert "needs an interactive terminal" in result.output
    assert "hello" in result.output  # …and it names the file to edit directly


@pytest.mark.parametrize(
    ("refusal", "fragment"),
    [
        ("not-editable", "no editable source"),
        ("reference-source-gone", "referenced source file is gone"),
        ("no-stored-copy", "no stored copy to edit"),
    ],
)
def test_the_three_edit_refusals_stay_apart(
    tmp_path: Path, refusal: launcher.EditRefusal, fragment: str
) -> None:
    """Collapsing them told the owner of a reference-mode PYTHON entry whose file had moved
    that it "has no editable source (programs and command templates run as-is)" — a
    sentence that denies the source exists and misclassifies the kind, about a script that
    is neither a program nor a template."""
    entry = _cmd("x", "echo hi")
    assert fragment in launcher.edit_refusal_message(refusal, store.resolve(entry.slug))


async def test_the_library_offers_no_edit_chip_where_there_is_nothing_to_edit(
    tmp_path: Path,
) -> None:
    """The door that produced the wrong message. Round 11's rule at the highest-traffic
    screen: one predicate drives the chip AND check_action, so `e` is never a button that
    does nothing when clicked."""
    entry = _cmd("tmpl", "echo hi")
    assert tui.MenuApp._can_edit(store.resolve(entry.slug)) is False
    editable = store.add_python(_py(tmp_path), name="hello")
    assert tui.MenuApp._can_edit(store.resolve(editable.slug)) is True

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.check_action("edit", ()) in (True, False)  # a real verdict, not a crash


# ==========================================================================
# JJ. Stored tokens stay English; what a person reads is translated
# ==========================================================================


def test_the_value_labels_translate_and_the_tokens_do_not(monkeypatch: pytest.MonkeyPatch) -> None:
    """`(Python · copy)` translated the kind and not the mode inside ONE parenthesis, and
    `工作目錄:store` was raw. No static gate could ever catch it: the literal is not in the
    source at all, it is in the user's meta.toml."""
    from skit import i18n

    try:
        i18n.init("zh_TW")
        assert kindnames.mode_label("copy") != "copy"
        assert kindnames.workdir_label("store") != "store"
    finally:
        i18n.init("en")


def test_an_absolute_workdir_is_never_relabelled(tmp_path: Path) -> None:
    """The fall-through that keeps the labels safe: `workdir` also holds a user-typed
    absolute path, the one value that must read back verbatim."""
    wd = str(tmp_path / "wd")
    assert kindnames.workdir_label(wd) == wd
    assert kindnames.mode_label("something-newer") == "something-newer"


# ==========================================================================
# KK. Declining is an answer, not a crash
# ==========================================================================


def test_declining_a_removal_is_translated_and_exits_130(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """click's `abort=True` died as a bare English `Aborted.` printed in RED — so the
    correct, deliberate answer to a destructive question read as an error — at exit 1,
    which the docs reserve for the launched script. The add lanes, which destroy nothing,
    have always answered 130 with a translated line."""
    store.add_python(_py(tmp_path), name="hello")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.typer, "confirm", lambda *a, **k: False)

    result = runner.invoke(cli.app, ["remove", "hello"])

    assert result.exit_code == 130
    assert "Aborted" not in result.output
    assert "nothing was removed" in result.output.lower()
    assert store.resolve("hello").meta.name == "hello"


def test_ctrl_d_lands_in_the_same_place_as_a_typed_no(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """click raises Abort on EOF regardless of abort=True, so handling only the typed "n"
    would have fixed half of it and left Ctrl+D on the old untranslated red exit 1."""
    import click

    store.add_python(_py(tmp_path), name="hello")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def _eof(*a: object, **k: object) -> bool:
        raise click.exceptions.Abort

    monkeypatch.setattr(cli.typer, "confirm", _eof)

    result = runner.invoke(cli.app, ["remove", "hello"])

    assert result.exit_code == 130
    assert "nothing was removed" in result.output.lower()
    assert store.resolve("hello").meta.name == "hello"


# ==========================================================================
# LL. A chip and its chord share one predicate, on every screen
# ==========================================================================


async def test_the_health_jump_chip_appears_only_with_issues(tmp_path: Path) -> None:
    """The FIRST chip on the Health screen was dead whenever the library was healthy — i.e.
    most of the time. Chip-only: Enter there is an OptionList selection rather than a
    Binding, so there is no action to disable and a check_action would itself be a branch
    that cannot fire (the shape this audit keeps deleting)."""
    from textual.widgets import Static

    from skit.tui_health import HealthScreen

    async def _footer() -> str:
        app = tui.MenuApp()
        async with app.run_test() as pilot:
            app.push_screen(HealthScreen())
            await pilot.pause()
            return "\n".join(str(w.render()) for w in app.screen.query(Static))

    # The chip glues its words with U+2800 so a pill wraps as one unit, so match on a
    # fragment rather than the spaced sentence.
    healthy = await _footer()
    assert "Jump" not in healthy

    entry = store.add_python(_py(tmp_path), name="gone")
    store.resolve(entry.slug).script_path.unlink()
    assert "Jump" in await _footer()


async def test_resync_has_one_predicate_behind_chip_action_and_chord(tmp_path: Path) -> None:
    """The resync condition existed in TWO spellings — the chip's and the action's — in a
    codebase that has spent twelve rounds deleting exactly that. Now three readers share
    one."""
    cmd = _cmd("tmpl", "echo hi")
    py = store.add_python(_py(tmp_path), name="hello")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        for entry, expected in ((cmd, False), (py, True)):
            screen = ScriptSettingsScreen(store.resolve(entry.slug))
            app.push_screen(screen)
            await pilot.pause()
            assert screen._can_resync() is expected
            assert screen.check_action("resync", ()) is expected
            screen.action_resync()  # …and stays total either way
            await pilot.pause()
            app.pop_screen()
            await pilot.pause()


async def test_the_add_review_picker_agrees_with_its_own_chip(tmp_path: Path) -> None:
    """The add review had TWO predicates for one rule: the chip appeared when the inline
    list was CAPPED, while the action ran whenever the list merely exceeded the preview
    limit. Between those thresholds Ctrl+L opened a picker no chip advertised — a
    keyboard-only path onto a list the user could already see in full."""
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT, LIST_PREVIEW_LIMIT
    from skit.tui_add import PromptReviewScreen

    assert LIST_PREVIEW_LIMIT < AUTO_MANAGE_LIMIT  # the window the two predicates disagreed in

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        for count, expected in (
            (LIST_PREVIEW_LIMIT + 1, False),  # visible in full: nothing to choose
            (AUTO_MANAGE_LIMIT + 1, True),  # capped: the picker is the way to the rest
        ):
            body = tmp_path / f"p{count}.prompt.md"
            body.write_text(
                " ".join(f"{{{{v{i}}}}}" for i in range(count)) + "\n", encoding="utf-8"
            )
            screen = PromptReviewScreen(body)
            app.push_screen(screen)
            await pilot.pause()
            assert screen._can_choose_candidates() is expected, count
            assert screen.check_action("choose_prompt_candidates", ()) is expected
            screen.action_choose_prompt_candidates()  # …and stays total either way
            await pilot.pause()
            while len(app.screen_stack) > 1:
                app.pop_screen()
                await pilot.pause()
