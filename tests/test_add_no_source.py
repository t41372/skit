"""Issue #10 — "make adding weird stuff intuitive": a bare `skit add` (no path) and an
unclassifiable file no longer error with a lecture on CLI flags. In a terminal they ASK;
in a pipe / under --no-input they still refuse honestly. These tests pin the CLI face of
both asks (the plain-line lane and the TUI-form gate), the refuse-never-drop guards, and
the picked-kind-rejoins-dispatch property.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
import typer
from typer.testing import CliRunner

from skit import cli, config, store

runner = CliRunner()

_TERM = {"TERM": "xterm"}


def _interpreted() -> list[str]:
    from skit.langs.registry import KNOWN_KINDS, spec_for

    return sorted(
        k
        for k in KNOWN_KINDS
        if (s := spec_for(k)) is not None and s.family == "interpreted" and k != "prompt"
    )


# ---------------------------------------------------------------------------
# 1. Bare add, non-interactive: the honest lane list, never an ask.
# ---------------------------------------------------------------------------


def test_bare_add_no_input_lists_the_lanes(tmp_path):
    result = runner.invoke(cli.app, ["add", "--no-input"])
    assert result.exit_code == 2
    out = result.output
    assert "Provide a source path" in out
    # The message names ONLY the lanes that work under --no-input / in a pipe: the
    # stdin spellings and --cmd. It no longer recommends --edit/--prompt-with-editor
    # (both refuse without a terminal).
    assert "--edit" not in out
    assert "--prompt" in out
    assert "--cmd" in out
    assert "skit add -" in out
    assert "-n NAME" in out


def test_bare_add_piped_lists_the_lanes(tmp_path, monkeypatch):
    # A pipe (no TTY) is non-interactive even without --no-input.
    monkeypatch.setattr(cli, "_is_interactive", lambda: False)
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 2
    assert "Provide a source path" in result.output


# ---------------------------------------------------------------------------
# 2. Bare add, interactive, with a flag that has nothing to attach to → refused.
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("flag", "shown"),
    [
        (["--name", "x"], "--name"),
        (["--description", "d"], "--description"),
        (["--ref"], "--ref"),
        (["--exe"], "--exe"),
        (["--kind", "shell"], "--kind"),
        (["--runner", "claude"], "--runner"),
        (["--dep", "rich"], "--dep"),
        (["--python", ">=3.11"], "--python"),
        (["--no-interpolate"], "--no-interpolate"),
    ],
)
def test_bare_add_interactive_refuses_each_orphan_flag(tmp_path, monkeypatch, flag, shown):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["add", *flag], env=_TERM)
    assert result.exit_code == 2, result.output
    assert "need a source" in result.output
    assert shown in result.output


# ---------------------------------------------------------------------------
# 3. Bare add, plain menu (form=plain / TERM=dumb): the four lanes.
# ---------------------------------------------------------------------------


def test_plain_menu_choice2_opens_the_python_editor_lane(tmp_path, monkeypatch):
    """Choice 2 routes to the editor lane. (No flags can ride along — the withheld
    guard refuses any bare add that carries one — so the forwarded values are the
    defaults; what this pins is the routing and the keyword wiring.)"""
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "2"))
    seen: dict[str, object] = {}
    monkeypatch.setattr(
        cli,
        "_create_python_in_editor",
        lambda name, description, *, deps_opt, python_opt, no_input: seen.update(
            name=name,
            description=description,
            deps_opt=deps_opt,
            python_opt=python_opt,
            no_input=no_input,
        ),
    )
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 0, result.output
    assert seen == {
        "name": None,
        "description": None,
        "deps_opt": None,
        "python_opt": None,
        "no_input": False,
    }


def test_plain_menu_choice3_opens_the_prompt_editor_lane(tmp_path, monkeypatch):
    """Choice 3 routes to the prompt-editor lane with interpolate=True (the
    --no-interpolate default) — the keyword wiring, since no flag can ride along."""
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "3"))
    seen: dict[str, object] = {}
    monkeypatch.setattr(
        cli,
        "_create_prompt_in_editor",
        lambda name, description, runner_opt, *, interpolate, no_input: seen.update(
            name=name,
            description=description,
            runner=runner_opt,
            interpolate=interpolate,
            no_input=no_input,
        ),
    )
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 0, result.output
    assert seen == {
        "name": None,
        "description": None,
        "runner": None,
        "interpolate": True,
        "no_input": False,
    }


def test_plain_menu_choice4_command_template_happy_path(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    # Order: template, name, description (the retry loop is gone).
    answers = iter(["4", "ffmpeg -i {input}", "encode", ""])

    def ask(question, **kwargs):
        return next(answers)

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(ask))
    result = runner.invoke(cli.app, ["add"], env={"TERM": "dumb"})
    assert result.exit_code == 0, result.output
    entry = store.resolve("encode")
    assert entry.meta.kind == "command"
    assert entry.meta.params == ["input"]
    assert "Detected parameters: input" in result.output


def test_plain_menu_choice4_empty_template_cancels(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["4", "   "])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()
    assert store.list_entries() == []


def test_plain_menu_choice4_empty_name_cancels(tmp_path, monkeypatch):
    """One cancellation rule: an empty NAME cancels (130), no retry loop."""
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["4", "echo {x}", "   "])  # template ok, name blank -> cancel
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()
    assert store.list_entries() == []


def test_plain_menu_choice4_stores_the_description(tmp_path, monkeypatch):
    """The new Description (optional) ask lands on the command entry."""
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["4", "echo {x}", "shout", "say it loud"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("shout")
    assert entry.meta.kind == "command"
    assert entry.meta.description == "say it loud"


def test_plain_menu_choice1_path_continues_into_a_real_add(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    # A file skit infers as exe on every platform: POSIX reads the exec bit, Windows has
    # none, so there it must wear a PATHEXT extension (the _is_executable_file rule). The
    # slug is the stem either way, so "tool" resolves on both.
    exe = tmp_path / ("tool.exe" if sys.platform == "win32" else "tool")
    exe.write_text("opaque bytes\n", encoding="utf-8")
    exe.chmod(0o755)

    def ask(question, **kwargs):
        if "Which one?" in question:
            return "1"
        if "Path to the file" in question:
            return str(exe)
        return kwargs.get("default", "")

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(ask))
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 0, result.output
    assert store.resolve("tool").meta.kind == "exe"


def test_plain_menu_choice1_empty_path_cancels(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def ask(question, **kwargs):
        return "1" if "Which one?" in question else ""

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(ask))
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()


# ---------------------------------------------------------------------------
# 4. Bare add, TUI form: the hosted source step.
# ---------------------------------------------------------------------------


def test_bare_add_tui_form_summary_on_success(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    made = store.add_command("echo {msg}", name="viatui")

    monkeypatch.setattr("skit.tui_add.run_add_source", lambda: made.slug)
    result = runner.invoke(cli.app, ["add"], env=_TERM)
    assert result.exit_code == 0, result.output
    assert "viatui" in result.output


def test_bare_add_tui_form_cancel_exits_130(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_add_source", lambda: None)
    result = runner.invoke(cli.app, ["add"], env=_TERM)
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()


# ---------------------------------------------------------------------------
# 5. Unknown-kind plain ask (_ask_kind_plain): contents, order, routing, cancel.
# ---------------------------------------------------------------------------


def test_ask_kind_plain_lists_sorted_interpreted_plus_exe_and_prompt(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "-"))
    got = cli._ask_kind_plain("weird.xyz", has_shebang=False, offer_exe=True)
    assert got is None
    out = capsys.readouterr().out
    assert "What is weird.xyz?" in out
    # sorted interpreted kinds, then "A program", then "A prompt for an AI agent", no
    # duplicate prompt among the interpreted block.
    from skit.kindnames import kind_label

    labels = [kind_label(k) for k in _interpreted()]
    lines = [ln.strip() for ln in out.splitlines() if ln.strip().startswith(tuple("123456789"))]
    numbered = [ln.split(". ", 1)[1] for ln in lines]
    assert numbered == [*labels, "A program (run it directly)", "A prompt for an AI agent"]
    assert numbered.count("A prompt for an AI agent") == 1


def test_ask_kind_plain_no_exe_when_offer_exe_false(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "-"))
    cli._ask_kind_plain("draft", has_shebang=False, offer_exe=False)
    out = capsys.readouterr().out
    assert "A program (run it directly)" not in out
    assert "A prompt for an AI agent" in out


def test_ask_kind_plain_shebang_question_variant(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "-"))
    cli._ask_kind_plain("tool", has_shebang=True, offer_exe=True)
    out = capsys.readouterr().out
    assert "names no interpreter skit knows" in out
    assert "can't tell from the name" not in out


def test_ask_kind_plain_returns_the_picked_language(tmp_path, monkeypatch):
    idx = _interpreted().index("shell") + 1
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: str(idx)))
    assert cli._ask_kind_plain("x.xyz", has_shebang=False, offer_exe=True) == "shell"


def test_ask_kind_plain_returns_exe_and_prompt(tmp_path, monkeypatch):
    n = len(_interpreted())
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: str(n + 1)))
    assert cli._ask_kind_plain("x", has_shebang=False, offer_exe=True) == "exe"
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: str(n + 2)))
    assert cli._ask_kind_plain("x", has_shebang=False, offer_exe=True) == "prompt"


# ---------------------------------------------------------------------------
# 6. Unknown-kind ask end to end: routing + picked-kind-rejoins-dispatch.
# ---------------------------------------------------------------------------


def _unknown(tmp_path: Path, body: str = "some opaque text\n") -> Path:
    p = tmp_path / "mystery.xyz"
    p.write_text(body, encoding="utf-8")
    return p


def test_unknown_plain_pick_language_adds_it(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    idx = _interpreted().index("shell") + 1

    def ask(question, **kwargs):
        return str(idx) if "Which one?" in question else kwargs.get("default", "")

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(ask))
    result = runner.invoke(cli.app, ["add", str(_unknown(tmp_path, "echo hi\n"))])
    assert result.exit_code == 0, result.output
    assert store.resolve("mystery").meta.kind == "shell"


def test_unknown_plain_pick_exe_adds_it(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    n = len(_interpreted())

    def ask(question, **kwargs):
        return str(n + 1) if "Which one?" in question else kwargs.get("default", "")

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(ask))
    result = runner.invoke(cli.app, ["add", str(_unknown(tmp_path))])
    assert result.exit_code == 0, result.output
    assert store.resolve("mystery").meta.kind == "exe"


def test_unknown_plain_cancel_exits_130(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "-"))
    result = runner.invoke(cli.app, ["add", str(_unknown(tmp_path))])
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()
    assert store.list_entries() == []


def test_unknown_plain_pick_language_with_runner_hits_prompt_only_refusal(tmp_path, monkeypatch):
    """The picked kind rejoins the ordinary dispatch: picking a language while --runner
    rode along fires the prompt-only refusal, exactly as an explicit --kind would."""
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    idx = _interpreted().index("shell") + 1
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: str(idx)))
    result = runner.invoke(
        cli.app, ["add", str(_unknown(tmp_path, "echo hi\n")), "--runner", "claude"]
    )
    assert result.exit_code == 2
    assert "--runner only applies to prompt entries" in result.output


def test_unknown_plain_pick_prompt_runs_prompt_onboarding(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    n = len(_interpreted())

    def ask(question, **kwargs):
        if "Which one?" in question:
            return str(n + 2)  # "A prompt for an AI agent"
        if "Run this prompt" in question:
            return "-"  # no runner pin
        return kwargs.get("default", "")

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(ask))
    result = runner.invoke(cli.app, ["add", str(_unknown(tmp_path, "do {{thing}}\n"))])
    assert result.exit_code == 0, result.output
    assert store.resolve("mystery").meta.kind == "prompt"


def test_unknown_plain_kept_draft_offers_no_program_option(tmp_path, monkeypatch, capsys):
    """A kept draft passes offer_exe=False into the ask (the drafts boundary forbids exe)."""
    from skit.paths import drafts_dir

    drafts_dir().mkdir(parents=True, exist_ok=True)
    draft = drafts_dir() / "skit-new-mystery"
    draft.write_text("some opaque text\n", encoding="utf-8")
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "-"))
    result = runner.invoke(cli.app, ["add", str(draft)])
    assert result.exit_code == 130
    assert "A program (run it directly)" not in result.output


# ---------------------------------------------------------------------------
# 7. Unknown-kind TUI form: the hosted kind ask.
# ---------------------------------------------------------------------------


def test_unknown_tui_form_pick_routes_to_the_kind(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    seen: dict[str, object] = {}

    def fake_pick(filename, *, has_shebang, offer_exe, suggested=None):
        seen.update(filename=filename, has_shebang=has_shebang, offer_exe=offer_exe)
        return "shell"

    monkeypatch.setattr("skit.tui_add.run_kind_pick", fake_pick)
    # The picked kind rejoins the ordinary dispatch, which in form=tui hosts the review
    # panel — stub it so it commits (a real panel needs a terminal, pilot-tested apart).
    monkeypatch.setattr(
        "skit.tui_add.run_add_review",
        lambda path, **kw: store.add_script(path, kind=str(kw["kind"])).slug,
    )
    result = runner.invoke(cli.app, ["add", str(_unknown(tmp_path, "echo hi\n"))], env=_TERM)
    assert result.exit_code == 0, result.output
    assert store.resolve("mystery").meta.kind == "shell"
    assert seen["filename"] == "mystery.xyz"
    assert seen["offer_exe"] is True
    assert seen["has_shebang"] is False


def test_unknown_tui_form_cancel_exits_130(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_kind_pick", lambda *a, **k: None)
    result = runner.invoke(cli.app, ["add", str(_unknown(tmp_path))], env=_TERM)
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()
    assert store.list_entries() == []


def test_unknown_tui_form_shebang_flag_forwarded(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    seen: dict[str, object] = {}

    def fake_pick(filename, *, has_shebang, offer_exe, suggested=None):
        seen.update(has_shebang=has_shebang)
        return "shell"

    p = tmp_path / "mystery.xyz"
    p.write_text("#!/usr/bin/env florblang\necho hi\n", encoding="utf-8")
    monkeypatch.setattr("skit.tui_add.run_kind_pick", fake_pick)
    monkeypatch.setattr(
        "skit.tui_add.run_add_review",
        lambda path, **kw: store.add_script(path, kind=str(kw["kind"])).slug,
    )
    result = runner.invoke(cli.app, ["add", str(p)], env=_TERM)
    assert result.exit_code == 0, result.output
    assert seen["has_shebang"] is True


def test_md_tui_form_passes_suggested_prompt(tmp_path, monkeypatch):
    """A bare .md under form=tui goes STRAIGHT to run_kind_pick with suggested='prompt'
    (no line Confirm) — the prompt option pre-highlighted in the modal."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    # A line Confirm here would be a second interaction paradigm — it must NOT fire.
    monkeypatch.setattr(
        cli.Confirm, "ask", staticmethod(lambda *a, **k: pytest.fail("Confirm must not fire"))
    )
    seen: dict[str, object] = {}

    def fake_pick(filename, *, has_shebang, offer_exe, suggested=None):
        seen.update(filename=filename, suggested=suggested)
        return "prompt"

    monkeypatch.setattr("skit.tui_add.run_kind_pick", fake_pick)
    monkeypatch.setattr(
        "skit.tui_add.run_prompt_review",
        lambda path, **kw: store.add_prompt(path, name="notes").slug,
    )
    p = tmp_path / "notes.md"
    p.write_text("hello\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(p)], env=_TERM)
    assert result.exit_code == 0, result.output
    assert seen["filename"] == "notes.md"
    assert seen["suggested"] == "prompt"


def test_unknown_tui_form_pick_exe_hosts_the_review_panel(tmp_path, monkeypatch):
    """Picking "A program" from the TUI kind modal hosts the SAME ExeReviewScreen the
    Library's `a` opens — NOT a line prompt glued to the just-closed modal (the `run`
    rule; mouse-only operability). Prompt.ask must never fire on this path."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(
        cli.Prompt, "ask", staticmethod(lambda *a, **k: pytest.fail("no line prompt on form=tui"))
    )
    monkeypatch.setattr("skit.tui_add.run_kind_pick", lambda *a, **k: "exe")
    seen: dict[str, object] = {}

    def fake_exe_review(path, *, name=None, description=None):
        seen.update(path=path, name=name, description=description)
        return store.add_exe(path, name="mystery").slug

    monkeypatch.setattr("skit.tui_add.run_exe_review", fake_exe_review)
    result = runner.invoke(cli.app, ["add", str(_unknown(tmp_path))], env=_TERM)
    assert result.exit_code == 0, result.output
    assert store.resolve("mystery").meta.kind == "exe"
    forwarded = seen["path"]
    assert isinstance(forwarded, Path)
    assert forwarded.name == "mystery.xyz"  # the resolved source, forwarded intact


def test_unknown_tui_form_pick_exe_cancel_exits_130(tmp_path, monkeypatch):
    """Cancelling the hosted ExeReviewScreen (it returns None) cancels the add — exit 130,
    nothing stored — exactly as the script/prompt panels do."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_kind_pick", lambda *a, **k: "exe")
    monkeypatch.setattr("skit.tui_add.run_exe_review", lambda *a, **k: None)
    result = runner.invoke(cli.app, ["add", str(_unknown(tmp_path))], env=_TERM)
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()
    assert store.list_entries() == []


def test_exe_flag_tui_form_hosts_the_panel_and_prefills_flags(tmp_path, monkeypatch):
    """The explicit --exe lane joins the same rule: under form=tui it hosts the review
    panel too (not line prompts), and --name/--description prefill it — exact parity with
    the kind-pick route, so the two spellings of "add a program" can't drift."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(
        cli.Prompt, "ask", staticmethod(lambda *a, **k: pytest.fail("no line prompt on form=tui"))
    )
    seen: dict[str, object] = {}

    def fake_exe_review(path, *, name=None, description=None):
        seen.update(name=name, description=description)
        return store.add_exe(path, name=name or path.stem, description=description or "").slug

    monkeypatch.setattr("skit.tui_add.run_exe_review", fake_exe_review)
    prog = tmp_path / "tool"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    result = runner.invoke(
        cli.app,
        ["add", str(prog), "--exe", "--name", "given", "--description", "prewritten"],
        env=_TERM,
    )
    assert result.exit_code == 0, result.output
    assert seen == {"name": "given", "description": "prewritten"}
    entry = store.resolve("given")
    assert entry.meta.kind == "exe"
    assert entry.meta.description == "prewritten"


def _shell_with_secret(path: Path, *, name: str) -> str:
    """Add a real shell entry, then write a secret-marked decl into its stored copy —
    the trace a review panel leaves — and return its slug."""
    from skit.langs.registry import spec_for
    from skit.params import ParamDecl

    entry = store.add_script(path, kind="shell", name=name)
    spec = spec_for("shell")
    assert spec is not None
    assert spec.params_io is not None
    text = entry.script_path.read_text(encoding="utf-8")
    entry.script_path.write_text(
        spec.params_io.write(text, [ParamDecl(name="city", secret=True)]), encoding="utf-8"
    )
    return entry.slug


def test_hosted_interpreted_branch_prints_managed_and_secret_lines(tmp_path, monkeypatch):
    """The interpreted (shell/js/ts) hosted branch reports the stored copy's managed decls
    and its secret subset — the same trace the plain path prints, not a thinner one."""
    config.save_form("tui")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    src = tmp_path / "mystery.xyz"
    src.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    monkeypatch.setattr(
        "skit.tui_add.run_kind_pick", lambda *a, **k: "shell"
    )  # the unknown ask picks shell
    monkeypatch.setattr(
        "skit.tui_add.run_add_review",
        lambda path, **kw: _shell_with_secret(path, name="shmystery"),
    )
    result = runner.invoke(cli.app, ["add", str(src)], env=_TERM)
    assert result.exit_code == 0, result.output
    assert "Managed parameters" in result.output
    assert "city" in result.output
    assert "Secret parameter values are never saved" in result.output


def test_hosted_python_branch_prints_managed_and_secret_lines(tmp_path, monkeypatch):
    """The python hosted branch reports the stored copy's managed decls and secret subset
    (deps via effective_uv_metadata) — the full trace, not just deps."""
    config.save_form("tui")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    entry = _real_python_with_params(tmp_path, name="pyhosted")
    src = tmp_path / "another.py"
    src.write_text("print(1)\n", encoding="utf-8")
    monkeypatch.setattr("skit.tui_add.run_add_review", lambda path, **kw: entry.slug)
    result = runner.invoke(cli.app, ["add", str(src)], env=_TERM)
    assert result.exit_code == 0, result.output
    assert "Managed parameters" in result.output
    assert "city" in result.output  # decl.secret, listed as managed
    assert "Dependencies" in result.output
    assert "rich>=13" in result.output
    assert "Secret parameter values are never saved" in result.output


# ===========================================================================
# Mutation-kill battery: the extracted helpers (_ask_kind_plain,
# _report_command_params, _add_no_source_ask) are now standalone and mutated.
# These pin exact message text, exact prompt labels/choices, and exact call
# arguments so no string/logic mutant in them survives.
# ===========================================================================

_WIDE = {"TERM": "dumb", "COLUMNS": "200", "SKIT_LANG": "en"}


def _lines(out: str) -> list[str]:
    return out.splitlines()


# --- _add_no_source_ask: the editor lanes start from a blank slate (no forwarded
#     flags — the caller refuses every flag while nothing is picked yet). ---


def test_ans_choice2_python_lane_uses_blank_defaults(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "2"))
    seen: dict[str, object] = {}
    monkeypatch.setattr(
        cli,
        "_create_python_in_editor",
        lambda name, description, *, deps_opt, python_opt, no_input: seen.update(
            name=name,
            description=description,
            deps_opt=deps_opt,
            python_opt=python_opt,
            no_input=no_input,
        ),
    )
    out = cli._add_no_source_ask()
    assert out is None
    assert seen == {
        "name": None,
        "description": None,
        "deps_opt": None,
        "python_opt": None,
        "no_input": False,
    }


def test_ans_choice3_prompt_lane_uses_blank_defaults(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "3"))
    seen: dict[str, object] = {}
    monkeypatch.setattr(
        cli,
        "_create_prompt_in_editor",
        lambda name, description, runner_opt, *, interpolate, no_input: seen.update(
            name=name,
            description=description,
            runner=runner_opt,
            interpolate=interpolate,
            no_input=no_input,
        ),
    )
    out = cli._add_no_source_ask()
    assert out is None
    # interpolate=True is the default (the --no-interpolate default), no_input=False.
    assert seen == {
        "name": None,
        "description": None,
        "runner": None,
        "interpolate": True,
        "no_input": False,
    }


# --- _add_no_source_ask: TUI branch (form=tui, TERM!=dumb). ---


def _real_python_with_params(tmp_path: Path, *, name: str = "thing") -> store.Entry:
    """A REAL stored python entry whose stored copy carries a PEP 723 deps block and a
    [tool.skit] params table with a secret-marked decl — the shape the review panel
    leaves behind. The secret decl (`city`) is NOT name-heuristic-secret, and the
    heuristic-secret name (`API_TOKEN`) is left unmarked: reading decl.secret must yield
    exactly {city}, proving _hosted_add_summary honors the block, not is_secret_name."""
    from skit.langs.registry import spec_for
    from skit.params import ParamDecl

    src = tmp_path / f"{name}.py"
    src.write_text("print(1)\n", encoding="utf-8")
    entry = store.add_python(src, name=name, dependencies=["rich>=13"])
    spec = spec_for("python")
    assert spec is not None
    assert spec.params_io is not None
    text = entry.script_path.read_text(encoding="utf-8")
    decls = [ParamDecl(name="API_TOKEN", secret=False), ParamDecl(name="city", secret=True)]
    entry.script_path.write_text(spec.params_io.write(text, decls), encoding="utf-8")
    return store.resolve(entry.slug)


def test_ans_tui_summary_receives_deps_params_and_secrets(tmp_path, monkeypatch):
    """The TUI branch resolves the new slug and hands _print_add_summary the entry's
    effective deps (via effective_uv_metadata), the stored copy's [tool.skit] decls as
    the managed list, and the decl.secret subset (not the name heuristic)."""
    config.save_form("tui")
    monkeypatch.setenv("TERM", "xterm")
    entry = _real_python_with_params(tmp_path)

    monkeypatch.setattr("skit.tui_add.run_add_source", lambda: entry.slug)
    captured: dict[str, object] = {}
    monkeypatch.setattr(
        cli,
        "_print_add_summary",
        lambda entry, deps, managed, secrets: captured.update(
            slug=entry.slug, deps=deps, managed=managed, secrets=secrets
        ),
    )
    out = cli._add_no_source_ask()
    assert out is None
    assert captured["slug"] == entry.slug  # the resolved entry, not a stand-in
    assert captured["deps"] == ["rich>=13"]  # from effective_uv_metadata, not raw meta
    assert captured["managed"] == ["API_TOKEN", "city"]
    assert captured["secrets"] == ["city"]  # decl.secret honored, not is_secret_name


def test_hosted_add_summary_script_reads_decls_and_honors_decl_secret(tmp_path):
    """params_io path: managed = every stored [tool.skit] decl, secrets = the decl.secret
    subset. `city` is secret by decl though not by name; `API_TOKEN` is name-secret but
    unmarked — proving the block, not is_secret_name, decides."""
    entry = _real_python_with_params(tmp_path)
    deps, managed, secrets = cli._hosted_add_summary(entry)
    assert deps == ["rich>=13"]
    assert managed == ["API_TOKEN", "city"]
    assert secrets == ["city"]


def test_hosted_add_summary_prompt_falls_back_to_meta_and_name_heuristic(tmp_path):
    """A prompt entry has no params_io, so _hosted_add_summary takes the meta.params +
    is_secret_name fallback (not the block read) — managed mirrors meta.params exactly."""
    src = tmp_path / "greet.prompt.md"
    src.write_text("Say hi to {name} using {API_KEY}", encoding="utf-8")
    entry = store.add_prompt(src, name="greet")
    deps, managed, secrets = cli._hosted_add_summary(entry)
    assert deps == []
    assert managed == list(entry.meta.params or [])
    assert secrets == [n for n in managed if cli.is_secret_name(n)]


def test_hosted_add_summary_command_uses_meta_fallback(tmp_path):
    """A command entry (no params_io) reports its meta.params holes as managed, and the
    name-heuristic subset as secrets — proving the fallback branch, not the block read."""
    entry = store.add_command("echo {msg} {API_KEY}", name="cmdsum")
    deps, managed, secrets = cli._hosted_add_summary(entry)
    assert deps == []
    assert managed == ["msg", "API_KEY"]
    assert secrets == ["API_KEY"]  # is_secret_name filters the meta.params list


def test_ans_tui_cancel_prints_exact_message_and_exits_130(tmp_path, monkeypatch, capsys):
    config.save_form("tui")
    monkeypatch.setenv("TERM", "xterm")
    monkeypatch.setenv("COLUMNS", "200")
    monkeypatch.setattr("skit.tui_add.run_add_source", lambda: None)
    with pytest.raises(typer.Exit) as exc:
        cli._add_no_source_ask()
    assert exc.value.exit_code == 130
    assert "Cancelled — nothing was added." in _lines(capsys.readouterr().out)


def test_ans_term_dumb_forces_the_plain_menu_even_with_form_tui(tmp_path, monkeypatch, capsys):
    """TERM=dumb can't host Textual: the AND short-circuits to the plain line menu even
    when form=tui (kills the TERM/'dumb' condition mutants)."""
    config.save_form("tui")
    monkeypatch.setenv("TERM", "dumb")
    monkeypatch.setenv("COLUMNS", "200")
    # Guard against a condition mutant wrongly taking the TUI branch: stub run_add_source
    # so it returns fast (never launches Textual) — a wrong branch then prints the cancel
    # notice instead of the plain menu, and the assertion below fails (mutant killed).
    monkeypatch.setattr("skit.tui_add.run_add_source", lambda: None)
    # menu -> "1" (a file), path -> "" -> cancel.
    answers = iter(["1", ""])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    with pytest.raises(typer.Exit):
        cli._add_no_source_ask()
    assert "What would you like to add?" in _lines(capsys.readouterr().out)


# --- _add_no_source_ask: the plain menu's exact printed lines. ---


def test_ans_plain_menu_lines_are_exact(tmp_path, monkeypatch, capsys):
    config.save_form("plain")
    monkeypatch.setenv("COLUMNS", "200")
    # A non-dumb TERM with form=plain: the AND must yield the plain menu, not the TUI
    # branch — so this also pins `and` (an `or` would take the TUI branch here). Stub
    # run_add_source so a wrong branch returns instead of launching Textual.
    monkeypatch.setenv("TERM", "xterm")
    monkeypatch.setattr("skit.tui_add.run_add_source", lambda: None)
    answers = iter(["1", ""])  # a file; then empty path -> cancel
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    with pytest.raises(typer.Exit):
        cli._add_no_source_ask()
    lines = _lines(capsys.readouterr().out)
    assert "What would you like to add?" in lines
    assert "  1. A file you already have — a script, program, or prompt" in lines
    assert "  2. A new script, written in your editor" in lines
    assert "  3. A new AI-agent prompt, written in your editor" in lines
    assert "  4. A command template (e.g. ffmpeg -i {input})" in lines


def test_ans_choice4_reports_params_and_stores_description(tmp_path, monkeypatch, capsys):
    config.save_form("plain")
    monkeypatch.setenv("COLUMNS", "200")
    answers = iter(["4", "tpl {a} {b}", "cmd4", "a fine command"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    out = cli._add_no_source_ask()
    assert out is None
    entry = store.resolve("cmd4")
    assert entry.meta.kind == "command"
    assert entry.meta.params == ["a", "b"]
    assert entry.meta.description == "a fine command"  # the Description ask lands
    assert (
        "Detected parameters: a, b (the run form asks for them; your last values are remembered)"
        in _lines(capsys.readouterr().out)
    )


def test_ans_choice4_empty_template_cancels_with_exact_message(tmp_path, monkeypatch, capsys):
    config.save_form("plain")
    monkeypatch.setenv("COLUMNS", "200")
    answers = iter(["4", "   "])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    with pytest.raises(typer.Exit) as exc:
        cli._add_no_source_ask()
    assert exc.value.exit_code == 130
    assert "Cancelled — nothing was added." in _lines(capsys.readouterr().out)
    assert store.list_entries() == []


def test_ans_choice4_empty_name_cancels_with_exact_message(tmp_path, monkeypatch, capsys):
    config.save_form("plain")
    monkeypatch.setenv("COLUMNS", "200")
    answers = iter(["4", "echo {x}", "  "])  # template ok, blank name -> cancel
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    with pytest.raises(typer.Exit) as exc:
        cli._add_no_source_ask()
    assert exc.value.exit_code == 130
    assert "Cancelled — nothing was added." in _lines(capsys.readouterr().out)
    assert store.list_entries() == []


def test_ans_choice1_empty_path_cancels_with_exact_message(tmp_path, monkeypatch, capsys):
    config.save_form("plain")
    monkeypatch.setenv("COLUMNS", "200")
    answers = iter(["1", "  "])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    with pytest.raises(typer.Exit) as exc:
        cli._add_no_source_ask()
    assert exc.value.exit_code == 130
    assert "Cancelled — nothing was added." in _lines(capsys.readouterr().out)


def test_ans_choice1_returns_the_typed_path(tmp_path, monkeypatch):
    config.save_form("plain")
    answers = iter(["1", "  ~/tool.py  "])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    out = cli._add_no_source_ask()
    assert out == "~/tool.py"  # stripped, handed back for the path lane


# --- Real-prompt (rich) CLI tests: the prompt LABELS and choice lists print,
#     so their string mutants die on exact substrings. ---


def test_cli_plain_choice4_prompt_labels_and_choices(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    # 4, template, name, description.
    result = runner.invoke(cli.app, ["add"], input="4\ntpl {a} {b}\nenc\n\n", env=_WIDE)
    assert result.exit_code == 0, result.output
    joined = " ".join(result.output.split())
    assert "Which one? [1/2/3/4] (1):" in joined
    assert "Command template:" in joined
    assert "Name for the command:" in joined
    assert "Description (optional)" in joined


def test_cli_plain_choice1_path_label(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    # Inferred exe on every platform (see the choice-1 continue test): a bare extensionless
    # file is unknown on Windows and would divert into the kind ask.
    exe = tmp_path / ("tool.exe" if sys.platform == "win32" else "tool")
    exe.write_text("bytes\n", encoding="utf-8")
    exe.chmod(0o755)
    result = runner.invoke(cli.app, ["add"], input=f"1\n{exe}\n\n\n", env=_WIDE)
    assert result.exit_code == 0, result.output
    assert "Path to the file:" in " ".join(result.output.split())
    assert store.resolve("tool").meta.kind == "exe"


# --- _ask_kind_plain: exact question, options, cancel hint, choice list. ---


def test_cli_ask_kind_plain_full_layout(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    p = tmp_path / "mystery.xyz"
    p.write_text("opaque text\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(p)], input="-\n", env=_WIDE)
    assert result.exit_code == 130, result.output
    lines = _lines(result.output)
    assert "What is mystery.xyz? skit can't tell from the name." in lines
    from skit.kindnames import kind_label

    labels = [kind_label(k) for k in _interpreted()]
    expected_opts = [
        *labels,
        "A program (run it directly)",
        "A prompt for an AI agent",
    ]
    for i, label in enumerate(expected_opts, start=1):
        assert f"  {i}. {label}" in lines
    assert "- = cancel" in lines
    n = len(expected_opts)
    bracket = "[" + "/".join(str(i) for i in range(1, n + 1)) + "/-]"
    assert bracket in " ".join(result.output.split())


def test_cli_ask_kind_plain_shebang_question(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    p = tmp_path / "mystery.xyz"
    p.write_text("#!/usr/bin/env florblang\ncode\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(p)], input="-\n", env=_WIDE)
    assert result.exit_code == 130, result.output
    assert "The #! in mystery.xyz names no interpreter skit knows. What is it?" in _lines(
        result.output
    )


# --- Call contracts: capture Prompt.ask / store.add_command / _print_add_summary
#     kwargs so console=console, exact labels/choices/default, and the summary's
#     empty-list arguments are all pinned (kills the arg mutants directly). ---


def _capture(monkeypatch, target, attr, answers):
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []
    it = iter(answers)

    def fake(*a, **kw):
        calls.append((a, kw))
        return next(it)

    monkeypatch.setattr(target, attr, staticmethod(fake))
    return calls


def test_ask_kind_plain_prompt_call_contract(tmp_path, monkeypatch):
    calls = _capture(monkeypatch, cli.Prompt, "ask", ["-"])
    cli._ask_kind_plain("f.xyz", has_shebang=False, offer_exe=True)
    (args, kw) = calls[0]
    assert args[0] == "Which one?"
    n = len(_interpreted()) + 2  # + exe + prompt
    assert kw["choices"] == [*(str(i) for i in range(1, n + 1)), "-"]
    assert kw["console"] is cli.console


def test_ans_which_one_prompt_call_contract(tmp_path, monkeypatch):
    config.save_form("plain")
    calls = _capture(monkeypatch, cli.Prompt, "ask", ["1", ""])  # a file; blank path -> cancel
    with pytest.raises(typer.Exit):
        cli._add_no_source_ask()
    (args, kw) = calls[0]
    assert args[0] == "Which one?"
    assert kw["choices"] == ["1", "2", "3", "4"]
    assert kw["default"] == "1"
    assert kw["console"] is cli.console


def test_ans_choice4_call_contracts(tmp_path, monkeypatch):
    """Command-template lane: the Command-template, Name, and Description prompts, the
    add_command call, and the summary call all carry their exact arguments (incl
    console=console and the three empty summary lists)."""
    config.save_form("plain")
    calls = _capture(monkeypatch, cli.Prompt, "ask", ["4", "tpl {a} {b}", "cmd4x", "desc4x"])

    class _FakeMeta:
        params: object = ["a", "b"]

    class _FakeEntry:
        meta = _FakeMeta()

    cmd_calls: list[tuple[tuple[object, ...], dict[str, object]]] = []
    monkeypatch.setattr(
        cli.store,
        "add_command",
        lambda *a, **kw: (cmd_calls.append((a, kw)), _FakeEntry())[1],
    )
    summary_calls: list[tuple[object, ...]] = []
    monkeypatch.setattr(cli, "_print_add_summary", lambda *a: summary_calls.append(a))
    out = cli._add_no_source_ask()
    assert out is None
    # prompt labels + console
    assert calls[1][0][0] == "Command template"
    assert calls[1][1]["console"] is cli.console
    assert calls[2][0][0] == "Name for the command"
    assert calls[2][1]["console"] is cli.console
    assert calls[3][0][0] == "Description (optional)"
    assert calls[3][1]["console"] is cli.console
    assert calls[3][1]["default"] == ""  # optional: empty answer -> empty description
    # add_command args (template positional, name + the typed description)
    (cargs, ckw) = cmd_calls[0]
    assert cargs[0] == "tpl {a} {b}"
    assert ckw["name"] == "cmd4x"
    assert ckw["description"] == "desc4x"
    # summary is handed the entry, empty deps/managed, and the secret-named holes
    # (none here — command entries surface params via _report_command_params, not
    # the summary's managed list; secrets DO flow so the never-saved caveat prints).
    assert isinstance(summary_calls[0][0], _FakeEntry)
    assert summary_calls[0][1:] == ([], [], [])


def test_ans_path_prompt_call_contract(tmp_path, monkeypatch):
    config.save_form("plain")
    calls = _capture(monkeypatch, cli.Prompt, "ask", ["1", "~/x.py"])
    cli._add_no_source_ask()
    assert calls[1][0][0] == "Path to the file"
    assert calls[1][1]["console"] is cli.console


def test_ans_no_stray_markup_tokens_in_output(tmp_path, monkeypatch, capsys):
    """Belt-and-braces: the plain menu render must not leak an XX string-mutation marker."""
    config.save_form("plain")
    monkeypatch.setenv("COLUMNS", "200")
    _capture(monkeypatch, cli.Prompt, "ask", ["1", ""])
    with pytest.raises(typer.Exit):
        cli._add_no_source_ask()
    assert "XX" not in capsys.readouterr().out


# ---------------------------------------------------------------------------
# 9. Interactive directory adds: the --exe escape is COLLECTED, not taught.
#    (Pipes/--no-input keep the flag-teaching refusal — pinned in section 8.)
# ---------------------------------------------------------------------------


def test_add_unknown_directory_plain_confirm_yes_adds_program(tmp_path, monkeypatch):
    """form=plain: a Confirm collects the consent the non-interactive message can only
    teach; yes rejoins the ordinary exe lane (name/description line prompts)."""
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    d = tmp_path / "bundle.dir"
    d.mkdir()
    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(lambda *a, **kw: True))
    answers = iter(["toolname", "a dir-shaped tool"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **kw: next(answers)))
    result = runner.invoke(cli.app, ["add", str(d)], env=_TERM)
    assert result.exit_code == 0, result.output
    entry = store.resolve("toolname")
    assert entry.meta.kind == "exe"
    assert entry.meta.description == "a dir-shaped tool"


def test_add_unknown_directory_plain_confirm_no_cancels(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    d = tmp_path / "bundle.dir"
    d.mkdir()
    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(lambda *a, **kw: False))
    result = runner.invoke(cli.app, ["add", str(d)], env=_TERM)
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()
    assert store.list_entries() == []


def test_add_unknown_directory_plain_confirm_call_contract(tmp_path, monkeypatch):
    """The Confirm's exact question, default-yes, and console — the ask must name the
    input's actual shape (a directory) and default to the one lane it can take."""
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    d = tmp_path / "bundle.dir"
    d.mkdir()
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def fake_confirm(*a, **kw):
        calls.append((a, kw))
        return False

    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(fake_confirm))
    runner.invoke(cli.app, ["add", str(d)], env=_TERM)
    (args, kw) = calls[0]
    assert args[0] == "bundle.dir is a directory. Add it as a program that runs directly?"
    assert kw["default"] is True
    assert kw["console"] is cli.console


def test_add_unknown_directory_tui_hosts_exe_review_with_no_line_confirm(tmp_path, monkeypatch):
    """form=tui: the exe review panel IS the consent ask (Esc cancels) — never a bare
    line Confirm glued to a Textual app (the `run` rule), and the resolved directory
    reaches the panel."""
    config.save_form("tui")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    d = tmp_path / "bundle.dir"
    d.mkdir()
    monkeypatch.setattr(
        cli.Confirm,
        "ask",
        staticmethod(
            lambda *a, **kw: (_ for _ in ()).throw(AssertionError("no line Confirm under form=tui"))
        ),
    )
    seen: dict[str, object] = {}

    def fake_review(path, *, name=None, description=None):
        seen["path"] = path
        return store.add_exe(path, name="bundled").slug

    monkeypatch.setattr("skit.tui_add.run_exe_review", fake_review)
    result = runner.invoke(cli.app, ["add", str(d)], env=_TERM)
    assert result.exit_code == 0, result.output
    assert seen["path"] == d.resolve()
    assert store.resolve("bundled").meta.kind == "exe"


# ---------------------------------------------------------------------------
# 10. Command-template adds report the SAME trace through every door: the
#     teaching note plus the never-saved secrets caveat — never a summary that
#     claims a secret hole's "last values are remembered".
# ---------------------------------------------------------------------------


def test_command_secret_names_picks_the_secret_holes(tmp_path):
    entry = store.add_command("curl -H {API_KEY} {url}", name="curler")
    assert cli._command_secret_names(entry) == ["API_KEY"]


def test_cmd_flag_secret_hole_gets_never_saved_note(tmp_path):
    result = runner.invoke(cli.app, ["add", "--cmd", "curl -H {API_KEY} {url}", "-n", "curler"])
    assert result.exit_code == 0, result.output
    assert "Detected parameters" in result.output
    assert "Secret parameter values are never saved" in result.output


def test_plain_menu_choice4_secret_hole_gets_never_saved_note(tmp_path, monkeypatch):
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["4", "deploy {AUTH_TOKEN}", "deployer", ""])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **kw: next(answers)))
    result = runner.invoke(cli.app, ["add"], env=_TERM)
    assert result.exit_code == 0, result.output
    assert "Detected parameters" in result.output
    assert "Secret parameter values are never saved" in result.output


def test_bare_add_tui_command_door_matches_the_cmd_door(tmp_path, monkeypatch):
    """The form=tui bare-add door reports a template exactly like `--cmd` and the plain
    menu do: the teaching note + the secrets caveat, and never a second 'Managed
    parameters' spelling of the same names."""
    config.save_form("tui")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    made = store.add_command("echo {API_KEY} {msg}", name="viatui3")
    monkeypatch.setattr("skit.tui_add.run_add_source", lambda: made.slug)
    result = runner.invoke(cli.app, ["add"], env=_TERM)
    assert result.exit_code == 0, result.output
    assert "Detected parameters" in result.output
    assert "Managed parameters" not in result.output
    assert "Secret parameter values are never saved" in result.output


@pytest.mark.parametrize(
    ("flag", "advice"),
    [
        (["--ref"], None),
        (["--exe"], None),
        (["--kind", "shell"], None),
        (["--dep", "rich"], "--edit"),
        (["--python", ">=3.11"], "--edit"),
        (["--runner", "claude"], "--prompt"),
        (["--no-interpolate"], "--prompt"),
        (["--name", "x"], "--edit, --prompt, --cmd"),
        (["--description", "d"], "--edit, --prompt, --cmd"),
    ],
)
def test_bare_add_refusal_names_only_lanes_that_honor_the_flag(tmp_path, monkeypatch, flag, advice):
    """The lane advice must never teach a guaranteed second refusal: a recommended
    lane honors EVERY withheld flag (--ref/--exe/--kind fit none, so no lane is
    named; --dep/--python fit only --edit; --runner/--no-interpolate only --prompt;
    -n/-d fit all three)."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["add", *flag], env=_TERM)
    assert result.exit_code == 2, result.output
    flat = " ".join(result.output.split())
    if advice is None:
        assert "pick a lane" not in flat
    else:
        assert f"pick a lane outright with {advice} (nothing was added)" in flat


# ---------------------------------------------------------------------------
# 11. The shared helpers, pinned directly (mutation kills that must not depend
#     on which lane's test happens to route through them).
# ---------------------------------------------------------------------------


def test_wants_tui_form_matrix(monkeypatch):
    """TERM=dumb (exactly, case-sensitive) forces plain regardless of form; otherwise
    the config decides."""
    monkeypatch.setenv("TERM", "xterm")
    config.save_form("tui")
    assert cli._wants_tui_form() is True
    config.save_form("plain")
    assert cli._wants_tui_form() is False
    config.save_form("tui")
    monkeypatch.setenv("TERM", "dumb")
    assert cli._wants_tui_form() is False


def test_cancelled_add_exact_line_and_exit_code(capsys):
    with pytest.raises(typer.Exit) as exc:
        cli._cancelled_add()
    assert exc.value.exit_code == 130
    out = capsys.readouterr().out
    assert "Cancelled — nothing was added." in _lines(out)
    assert "XX" not in out  # belt: no string-mutation marker may leak


def test_bare_add_tui_command_door_summary_call_contract(tmp_path, monkeypatch):
    """The command door hands _print_add_summary the resolved entry, EMPTY deps and
    managed (the note owns the params vocabulary), and exactly the secret holes."""
    config.save_form("tui")
    monkeypatch.setenv("TERM", "xterm")
    made = store.add_command("echo {API_KEY} {msg}", name="viatui4")
    monkeypatch.setattr("skit.tui_add.run_add_source", lambda: made.slug)
    captured: dict[str, object] = {}
    monkeypatch.setattr(
        cli,
        "_print_add_summary",
        lambda entry, deps, managed, secrets: captured.update(
            slug=entry.slug, deps=deps, managed=managed, secrets=secrets
        ),
    )
    assert cli._add_no_source_ask() is None
    assert captured["slug"] == made.slug
    assert captured["deps"] == []
    assert captured["managed"] == []
    assert captured["secrets"] == ["API_KEY"]
