"""CLI surfaces added in the audit round: curation (rename/describe), launch policy
(params --workdir/--interpreter/--template), stdin --kind, run --forget-args, the raw
no-op notice, show's interpreter key, doctor's launch_blocked, runner add --force, the
interpreted-add review routing, the line-mode script onboarding, and the prompt-run
inline runner picker.

Every test drives the real command tree and asserts the persisted state, the exit code,
or the exact user-facing wording — never that a line merely ran.
"""

from __future__ import annotations

import json
import sys
import types

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, config, store
from skit.langs.registry import spec_for

runner = CliRunner()


@pytest.fixture(autouse=True)
def _isolated(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


@pytest.fixture
def spawn_spy(monkeypatch):
    calls: dict[str, object] = {}

    def fake(
        entry,
        extra_args=None,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
        runner=None,
    ):
        calls["entry"] = entry
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        calls["runner"] = runner
        return calls.get("code", 0)

    monkeypatch.setattr(cli.launcher, "run_entry", fake)
    return calls


def _py(tmp_path, body="print('hi')\n", name="job"):
    p = tmp_path / f"{name}.py"
    p.write_text(body, encoding="utf-8")
    return store.add_python(p, name=name)


def _shell(tmp_path, body="#!/usr/bin/env bash\necho hi\n", name="sh"):
    p = tmp_path / f"{name}.sh"
    p.write_text(body, encoding="utf-8")
    return store.add_script(p, kind="shell", name=name)


def _prompt(tmp_path, text="Do {{a}}\n", name="p", pin=""):
    p = tmp_path / f"{name}.prompt.md"
    p.write_text(text, encoding="utf-8")
    entry = store.add_prompt(p, name=name)
    if pin:
        entry = store.write_prompt_runner(entry.slug, pin)
    return entry


def _spec(kind):
    s = spec_for(kind)
    assert s is not None
    return s


def _find_runner(name):
    r = config.find_prompt_runner(name)
    assert r is not None
    return r


# ============================================================ curation


def test_rename_success_keeps_presets_and_state(tmp_path):
    old = _py(tmp_path, name="old")
    argstate.save_preset(old.slug, "nightly", {"x": "1"})
    argstate.save_last(old.slug, extra_args=["--flag"])
    result = runner.invoke(cli.app, ["rename", "old", "new"])
    assert result.exit_code == 0, result.output
    assert "Renamed to new." in result.output
    entry = store.resolve("new")
    assert entry.meta.name == "new"
    assert entry.slug == old.slug  # slug is immutable
    # presets and remembered args survive under the same slug.
    state = argstate.load_state(entry.slug)
    assert list(state["presets"]) == ["nightly"]
    assert state["extra_args"] == ["--flag"]


def test_rename_not_found(tmp_path):
    result = runner.invoke(cli.app, ["rename", "ghost", "x"])
    assert result.exit_code == 1


def test_rename_conflict(tmp_path):
    _py(tmp_path, name="a")
    _py(tmp_path, name="b")
    result = runner.invoke(cli.app, ["rename", "a", "b"])
    assert result.exit_code == 1
    assert "already taken" in result.output
    assert store.resolve("a").meta.name == "a"  # untouched


def test_describe_set_and_clear(tmp_path):
    _py(tmp_path, name="a")
    set_res = runner.invoke(cli.app, ["describe", "a", "  A tidy helper  "])
    assert set_res.exit_code == 0, set_res.output
    assert "Description updated for a." in set_res.output
    assert store.resolve("a").meta.description == "A tidy helper"
    clr_res = runner.invoke(cli.app, ["describe", "a", ""])
    assert clr_res.exit_code == 0, clr_res.output
    assert "Description cleared for a." in clr_res.output
    assert store.resolve("a").meta.description == ""


def test_describe_not_found(tmp_path):
    result = runner.invoke(cli.app, ["describe", "ghost", "x"])
    assert result.exit_code == 1


# ============================================================ launch policy: workdir


@pytest.mark.parametrize("literal", ["origin", "store", "invoke"])
def test_params_workdir_literals(tmp_path, literal):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["params", "sh", "--workdir", literal])
    assert result.exit_code == 0, result.output
    assert f"now runs in: {literal}" in result.output
    assert store.resolve("sh").meta.workdir == literal


def test_params_workdir_absolute_path(tmp_path):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["params", "sh", "--workdir", "/opt/data"])
    assert result.exit_code == 0, result.output
    assert store.resolve("sh").meta.workdir == "/opt/data"


def test_params_workdir_relative_is_clean_error(tmp_path):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["params", "sh", "--workdir", "rel/ative"])
    assert result.exit_code == 1
    assert "origin, store, invoke, or an absolute path" in result.output


# ============================================================ launch policy: interpreter


def test_params_interpreter_set_and_clear_shell(tmp_path):
    _shell(tmp_path)
    set_res = runner.invoke(cli.app, ["params", "sh", "--interpreter", "zsh"])
    assert set_res.exit_code == 0, set_res.output
    assert "now runs with: zsh" in set_res.output
    assert store.resolve("sh").meta.interpreter == "zsh"
    clr_res = runner.invoke(cli.app, ["params", "sh", "--interpreter", ""])
    assert clr_res.exit_code == 0, clr_res.output
    assert "back to automatic interpreter detection" in clr_res.output
    assert store.resolve("sh").meta.interpreter == ""


def test_params_interpreter_set_js(tmp_path):
    p = tmp_path / "j.js"
    p.write_text("console.log(1)\n", encoding="utf-8")
    store.add_script(p, kind="js", name="j")
    result = runner.invoke(cli.app, ["params", "j", "--interpreter", "bun"])
    assert result.exit_code == 0, result.output
    assert store.resolve("j").meta.interpreter == "bun"


def test_params_interpreter_refused_on_python_and_prompt(tmp_path):
    _py(tmp_path, name="py")
    _prompt(tmp_path, name="pr")
    for name in ("py", "pr"):
        result = runner.invoke(cli.app, ["params", name, "--interpreter", "zsh"])
        assert result.exit_code == 1, (name, result.output)
        assert "pinnable interpreter" in result.output


def test_params_interpreter_refused_on_exe_and_command(tmp_path):
    prog = tmp_path / "tool"
    prog.write_text("#!/bin/sh\necho\n", encoding="utf-8")
    prog.chmod(0o755)
    store.add_exe(prog, name="ex")
    store.add_command("echo {m}", name="cmd")
    for name in ("ex", "cmd"):
        result = runner.invoke(cli.app, ["params", name, "--interpreter", "zsh"])
        assert result.exit_code == 1, (name, result.output)
        assert "pinnable interpreter" in result.output


# ============================================================ launch policy: template


def test_params_template_rewrite_reextracts_placeholders(tmp_path):
    store.add_command("echo {old}", name="cmd")
    result = runner.invoke(cli.app, ["params", "cmd", "--template", "ffmpeg -i {input} {output}"])
    assert result.exit_code == 0, result.output
    assert "Placeholders: input, output" in result.output
    assert store.resolve("cmd").meta.params == ["input", "output"]


def test_params_template_refused_on_non_command(tmp_path):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["params", "sh", "--template", "echo {x}"])
    assert result.exit_code == 1
    assert "isn't a command entry" in result.output


def test_params_template_empty_refused(tmp_path):
    store.add_command("echo {x}", name="cmd")
    result = runner.invoke(cli.app, ["params", "cmd", "--template", "   "])
    assert result.exit_code == 1
    assert store.resolve("cmd").meta.template == "echo {x}"  # unchanged


# ============================================================ show interpreter key


def test_show_json_and_human_carry_interpreter(tmp_path):
    _shell(tmp_path)
    store.write_interpreter("sh", "zsh")
    j = runner.invoke(cli.app, ["show", "sh", "--json"])
    assert j.exit_code == 0, j.output
    assert json.loads(j.output)["interpreter"] == "zsh"
    human = runner.invoke(cli.app, ["show", "sh"])
    assert human.exit_code == 0, human.output
    assert "Interpreter: zsh" in human.output


def test_show_json_interpreter_none_when_unset(tmp_path):
    _shell(tmp_path)
    j = runner.invoke(cli.app, ["show", "sh", "--json"])
    assert json.loads(j.output)["interpreter"] is None


# ============================================================ stdin --kind


def test_add_stdin_kind_shell_with_shebang_pins_interpreter(tmp_path):
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "shell", "-n", "x"], input="#!/bin/bash\necho $1\n"
    )
    assert result.exit_code == 0, result.output
    entry = store.resolve("x")
    assert entry.meta.kind == "shell"
    assert entry.script_path.suffix == ".sh"
    assert entry.meta.interpreter == "bash"  # shebang program is a known shell shebang


def test_add_stdin_kind_js_records_scanned_deps(tmp_path):
    result = runner.invoke(
        cli.app,
        ["add", "-", "--kind", "js", "-n", "j"],
        input="import chalk from 'chalk'\nconsole.log(chalk)\n",
    )
    assert result.exit_code == 0, result.output
    assert "chalk" in (store.resolve("j").meta.dependencies or [])


def test_add_stdin_exe_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--exe", "-n", "x"], input="echo\n")
    assert result.exit_code == 2
    assert "needs an existing program on disk" in result.output


def test_add_stdin_kind_exe_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--kind", "exe", "-n", "x"], input="echo\n")
    assert result.exit_code == 2
    assert "needs an existing program on disk" in result.output


def test_add_edit_with_kind_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "--edit", "--kind", "shell"])
    assert result.exit_code == 2
    assert "pipe it in" in result.output


def test_add_edit_with_exe_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "--edit", "--exe"])
    assert result.exit_code == 2


def test_add_stdin_kind_js_without_imports_records_no_deps(tmp_path):
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "js", "-n", "j"], input="console.log(1)\n"
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("j").meta.dependencies in (None, [])  # nothing scanned → no deps


def test_add_stdin_kind_duplicate_name_is_store_error(tmp_path):
    _shell(tmp_path, name="dup")
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "shell", "-n", "dup"], input="#!/bin/bash\necho x\n"
    )
    assert result.exit_code == 1
    assert "already taken" in result.output


def test_add_stdin_kind_python_routes_to_the_python_lane(tmp_path):
    # --kind python is not the non-python twin's job: it falls through to the PEP 723 lane.
    result = runner.invoke(
        cli.app, ["add", "-", "--kind", "python", "-n", "x"], input="print('hi')\n"
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("x").meta.kind == "python"


def test_add_stdin_kind_missing_name(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--kind", "shell"], input="echo hi\n")
    assert result.exit_code == 2
    assert "explicit --name" in result.output


def test_add_stdin_kind_empty_input(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--kind", "shell", "-n", "x"], input="   \n")
    assert result.exit_code == 1
    assert "Nothing arrived on stdin" in result.output


# ============================================================ interpreted add review routing


def test_add_shell_interactive_routes_through_review_panel(tmp_path, monkeypatch):
    """A real terminal with form=tui hosts the SAME review panel python gets — the flags
    ride along as prefills and the panel's slug feeds the summary."""
    src = tmp_path / "tool.sh"
    src.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    seen: dict[str, object] = {}

    def fake_panel(path, **kwargs):
        seen["path"] = path
        seen.update(kwargs)
        return store.add_script(path, kind="shell", name="panelled").slug

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_add_review", fake_panel)
    result = runner.invoke(cli.app, ["add", str(src), "-d", "helper"])
    assert result.exit_code == 0, result.output
    assert seen["kind"] == "shell"  # the kind kwarg threads through
    assert seen["description"] == "helper"
    assert "panelled" in result.output


def test_add_shell_interactive_panel_cancel_exits_130(tmp_path, monkeypatch):
    src = tmp_path / "tool.sh"
    src.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_add_review", lambda path, **kw: None)
    result = runner.invoke(cli.app, ["add", str(src)])
    assert result.exit_code == 130
    assert "Cancelled" in result.output


# ============================================================ line-mode onboarding


def _fake_tty(monkeypatch, isatty=True):
    monkeypatch.setattr(sys, "stdin", types.SimpleNamespace(isatty=lambda: isatty, read=lambda: ""))


def test_onboard_script_params_writes_picked_shell_params(tmp_path, monkeypatch):
    entry = _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="d")
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "1"))
    managed = cli._onboard_script_params(store.resolve("d"), _spec("shell"), no_input=False)
    assert managed == ["CITY"]
    # The pick was written into the stored copy — read_params / skit params see it.
    text = entry.script_path.read_text(encoding="utf-8")
    io = _spec("shell").params_io
    assert io is not None
    assert "CITY" in [d.name for d in io.read(text)]


def test_onboard_script_params_none_selection_writes_nothing(tmp_path, monkeypatch):
    entry = _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="d")
    before = entry.script_path.read_text(encoding="utf-8")
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "none"))
    assert cli._onboard_script_params(store.resolve("d"), _spec("shell"), no_input=False) == []
    assert entry.script_path.read_text(encoding="utf-8") == before  # copy untouched


def test_onboard_script_params_no_input_selects_nothing(tmp_path, monkeypatch):
    _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="d")
    _fake_tty(monkeypatch)
    assert cli._onboard_script_params(store.resolve("d"), _spec("shell"), no_input=True) == []


def test_onboard_script_params_skips_reference_entries(tmp_path, monkeypatch):
    src = tmp_path / "d.sh"
    src.write_text("#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", encoding="utf-8")
    entry = store.add_script(src, kind="shell", name="d", mode="reference")
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "1"))
    assert cli._onboard_script_params(entry, _spec("shell"), no_input=False) == []


def test_onboard_script_params_skips_cli_framework_scripts(tmp_path, monkeypatch):
    # The guard is kind-agnostic: when the entry's OWN analyzer reports the script parses
    # its own arguments (a CLI framework), onboarding manages nothing and never asks. The
    # python analyzer is the one that detects frameworks (argparse), so drive it directly.
    p = tmp_path / "d.py"
    p.write_text(
        "import argparse\nargparse.ArgumentParser().parse_args()\nCITY = 'x'\n", encoding="utf-8"
    )
    entry = store.add_python(p, name="d")
    _fake_tty(monkeypatch)
    ask_hit = {"n": 0}
    monkeypatch.setattr(
        cli.Prompt, "ask", staticmethod(lambda *a, **k: ask_hit.__setitem__("n", 1))
    )
    assert cli._onboard_script_params(entry, _spec("python"), no_input=False) == []
    assert ask_hit["n"] == 0  # never reached the ask


# ============================================================ run --forget-args


def test_run_forget_args_erases_remembered_tail(tmp_path, spawn_spy):
    entry = _py(tmp_path, name="a")
    argstate.save_last(entry.slug, extra_args=["--old", "value"])
    result = runner.invoke(cli.app, ["run", "a", "--forget-args", "--no-input"])
    assert result.exit_code == 0, result.output
    assert argstate.load_state(entry.slug)["extra_args"] == []  # erased up front
    assert spawn_spy["extra"] == []  # the run reused nothing


# ============================================================ raw no-op notice


def test_run_raw_notice_on_analyzer_less_kind(tmp_path, spawn_spy):
    store.add_command("echo hi", name="cmd")
    result = runner.invoke(cli.app, ["run", "cmd", "--raw", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "no injection to skip" in result.output  # the honest no-op notice


# ============================================================ doctor launch_blocked


def test_doctor_reports_launch_blocked(tmp_path, monkeypatch):
    _shell(tmp_path, name="blocked")
    monkeypatch.setattr("skit.langs.launch._which", lambda _name: None)
    j = runner.invoke(cli.app, ["doctor", "--json"])
    assert j.exit_code in (0, 1), j.output
    payload = json.loads(j.output)
    assert "blocked" in payload["launch_blocked"]
    human = runner.invoke(cli.app, ["doctor"])
    assert "a run would refuse to start" in human.output


# ============================================================ runner add --force


def test_runner_add_force_replaces_in_place(tmp_path):
    config.ensure_prompt_runners_seeded()
    before = [r.name for r in config.load_prompt_runners()]
    result = runner.invoke(
        cli.app, ["runner", "add", "codex", "--force", "--", "codex", "--model", "o1", "{{prompt}}"]
    )
    assert result.exit_code == 0, result.output
    assert [r.name for r in config.load_prompt_runners()] == before  # order held
    assert _find_runner("codex").argv == ("codex", "--model", "o1", "{{prompt}}")


def test_runner_add_without_force_refuses_with_force_hint(tmp_path):
    config.ensure_prompt_runners_seeded()
    result = runner.invoke(cli.app, ["runner", "add", "codex", "--", "codex", "{{prompt}}"])
    assert result.exit_code == 1
    assert "--force" in result.output


# ============================================================ prompt run inline picker


def _prompt_run_interactive(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    config.save_form("tui")


def test_run_prompt_inline_picker_resolves_the_runner(tmp_path, spawn_spy, monkeypatch):
    _prompt(tmp_path)
    _prompt_run_interactive(monkeypatch)
    seen: dict[str, object] = {}

    def fake_collect(entry, plan, prefill, runners=None, runner_default=""):
        seen["runners"] = list(runners or [])
        seen["default"] = runner_default
        return {"a": "hi"}, "codex"

    monkeypatch.setattr("skit.inlineform.collect", fake_collect)
    ask_hit = {"n": 0}
    monkeypatch.setattr(
        cli.Prompt, "ask", staticmethod(lambda *a, **k: ask_hit.__setitem__("n", 1))
    )
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert seen["runners"] == [r.name for r in config.load_prompt_runners()]
    assert ask_hit["n"] == 0  # never line-asked for a runner
    assert spawn_spy["runner"] == config.find_prompt_runner("codex")
    assert spawn_spy["values"] == {"a": "hi"}
    assert argstate.load_last_runner() == "codex"  # the pick was remembered


def test_run_prompt_inline_picker_none_falls_back(tmp_path, spawn_spy, monkeypatch):
    _prompt(tmp_path)
    _prompt_run_interactive(monkeypatch)
    monkeypatch.setattr("skit.inlineform.collect", lambda *a, **k: ({"a": "hi"}, None))
    called: dict[str, object] = {}

    def fake_resolve(entry, runner_flag, no_input):
        called["hit"] = True
        return config.find_prompt_runner("amp")

    monkeypatch.setattr(cli, "_resolve_run_runner", fake_resolve)
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert called.get("hit") is True  # degraded → deterministic line-mode resolution
    assert spawn_spy["runner"] == config.find_prompt_runner("amp")


def test_run_prompt_inline_picker_removed_runner_is_126(tmp_path, spawn_spy, monkeypatch):
    _prompt(tmp_path)
    _prompt_run_interactive(monkeypatch)
    monkeypatch.setattr("skit.inlineform.collect", lambda *a, **k: ({"a": "hi"}, "ghostrunner"))
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 126
    assert "ghostrunner" in result.output
    assert "entry" not in spawn_spy  # never launched
