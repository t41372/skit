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


def _flat(text: str) -> str:
    """Collapse rich's soft-wrap newlines/whitespace so a message split across the 80-col
    CliRunner width still matches as one string."""
    return " ".join(text.split())


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


def test_params_workdir_origin_on_a_command_fails_cleanly(tmp_path):
    """`skit params <command> --workdir origin` fails cleanly (exit 1, StoreUsageError
    surfaced) — a command has no original file for "origin" to mean (finding 8)."""
    runner.invoke(cli.app, ["add", "--cmd", "echo hi", "--name", "c", "--no-input"])
    result = runner.invoke(cli.app, ["params", "c", "--workdir", "origin"])
    assert result.exit_code == 1
    assert "no original file — origin doesn't apply" in _flat(result.output)
    assert store.resolve("c").meta.workdir != "origin"  # nothing persisted


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


# ============================================================ params: one op per call


def _fingerprint(name):
    """Everything a params edit could touch: the meta AND the stored script text. A
    refused one-op combo must leave BOTH byte-identical (the "nothing was changed"
    contract), so the snapshot has to see the script's own [tool.skit] block too."""
    entry = store.resolve(name)
    script = entry.script_path.read_text(encoding="utf-8") if entry.script_path.exists() else ""
    return (entry.meta, script)


def _assert_one_op_refusal(name, argv, op):
    before = _fingerprint(name)
    result = runner.invoke(cli.app, ["params", name, *argv])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert f"{op} is its own operation" in flat
    assert "nothing was changed" in flat
    assert _fingerprint(name) == before  # neither the policy op NOR the rider applied


def test_params_workdir_with_schema_flag_refused_unchanged(tmp_path):
    _shell(tmp_path, name="sh")
    _assert_one_op_refusal(
        "sh", ["--workdir", "invoke", "--manage", "CITY"], "--workdir/--interpreter/--template"
    )


def test_params_interpolate_with_schema_flag_refused_unchanged(tmp_path):
    _prompt(tmp_path, name="pr")
    _assert_one_op_refusal(
        "pr", ["--interpolate", "--secret", "a"], "--interpolate/--no-interpolate"
    )


def test_params_runner_with_schema_flag_refused_unchanged(tmp_path):
    config.ensure_prompt_runners_seeded()
    _prompt(tmp_path, name="pr")
    _assert_one_op_refusal("pr", ["--runner", "claude", "--resync"], "--runner")


def test_params_normalize_with_schema_flag_refused_unchanged(tmp_path):
    _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="sh")
    _assert_one_op_refusal("sh", ["--normalize", "CITY", "--manage", "CITY"], "--normalize")


def test_params_normalize_with_json_emits_the_read_view(tmp_path):
    """The --normalize own-op honors --json like every other own-op: the source idiom is
    rewritten, then a valid read-view JSON is emitted on stdout — not a silent drop (the
    fourth own-op face, so the rule holds on ALL of interpolate/runner/workdir/normalize)."""
    import json

    _shell(tmp_path, body="#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n", name="sh")
    result = runner.invoke(cli.app, ["params", "sh", "--normalize", "CITY", "--json"])
    assert result.exit_code == 0, result.output
    # The stored copy now carries the ${CITY:-Taipei} idiom (the semantic rewrite).
    assert "${CITY:-Taipei}" in store.resolve("sh").script_path.read_text(encoding="utf-8")
    payload = json.loads(result.output[result.output.index("{") :])  # valid JSON follows
    assert "params" in payload  # the entry's read view


def test_params_two_policy_groups_refused_unchanged(tmp_path):
    """Two policy ops in one call is refused just like a policy op + a schema flag — the
    first-listed op names the refusal, and nothing changes."""
    config.ensure_prompt_runners_seeded()
    _prompt(tmp_path, name="pr")
    _assert_one_op_refusal(
        "pr", ["--interpolate", "--runner", "claude"], "--interpolate/--no-interpolate"
    )


def test_params_workdir_plus_runner_refused_names_runner_first(tmp_path):
    config.ensure_prompt_runners_seeded()
    _prompt(tmp_path, name="pr")
    # --runner is listed before the workdir group, so it's the op named in the refusal.
    _assert_one_op_refusal("pr", ["--workdir", "invoke", "--runner", "claude"], "--runner")


def test_params_launch_policy_group_stays_combinable(tmp_path):
    """--workdir/--interpreter/--template are ONE launch-policy group, not three ops — so
    --workdir + --interpreter in a single call is allowed and both apply (exit 0)."""
    _shell(tmp_path, name="sh")
    result = runner.invoke(cli.app, ["params", "sh", "--workdir", "invoke", "--interpreter", "zsh"])
    assert result.exit_code == 0, result.output
    assert "is its own operation" not in result.output  # NOT one-op-refused
    meta = store.resolve("sh").meta
    assert meta.workdir == "invoke"  # both landed
    assert meta.interpreter == "zsh"


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
    # The message now points at the shebang (the draft's real kind signal), not a Python
    # starter (finding 7): --edit reads the kind from the draft, so --kind is redundant.
    flat = _flat(result.output)
    assert "reads the kind from your draft's shebang" in flat
    assert "pipe it in" in flat


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


def test_run_forget_args_below_usage_gates_survives_a_refusal(tmp_path, spawn_spy):
    """--forget-args moved BELOW every usage gate: an exit-2 invocation must leave no
    fingerprints, so a refused run (here an unknown preset) leaves the remembered tail
    intact — the same "nothing is changed" rule the raw refusal states. The clean-erase
    twin above pins that a passing --forget-args run still wipes it."""
    entry = _py(tmp_path, name="a")
    argstate.save_last(entry.slug, extra_args=["--old", "value"])
    result = runner.invoke(cli.app, ["run", "a", "--forget-args", "--preset", "nope", "--no-input"])
    assert result.exit_code == 2, result.output  # unknown-preset usage error, before the clear
    assert argstate.load_state(entry.slug)["extra_args"] == ["--old", "value"]  # NOT erased
    assert "entry" not in spawn_spy  # nothing launched


# ============================================================ raw refusal


def test_run_raw_refused_on_placeholder_kind(tmp_path, spawn_spy):
    """On a kind whose {placeholders} ARE the artifact (here: a command template) there is
    no "as-is" to run — `--raw` is REFUSED (exit 2) with the placeholder-keyed message, not
    a notice-then-run-anyway. Nothing launches. The gate keys off placeholder_params (the
    interface trait), not an internal analyzer capability."""
    store.add_command("echo hi", name="cmd")
    result = runner.invoke(cli.app, ["run", "cmd", "--raw", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "--raw doesn't apply to a command entry" in flat
    assert "there is no as-is without them" in flat
    assert "entry" not in spawn_spy  # refused before any launch


def test_run_raw_allowed_on_injected_kind(tmp_path, spawn_spy):
    """The twin: on an injected kind (python) --raw stays a legitimate escape hatch — it
    runs the stored copy as-is, no refusal."""
    _py(tmp_path, name="job")
    result = runner.invoke(cli.app, ["run", "job", "--raw", "--no-input"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["entry"].meta.name == "job"  # it really launched


def test_run_raw_runs_an_exe_with_the_skip_notice(tmp_path, spawn_spy):
    """A program (exe) has NO {placeholders} — it runs as-is exactly like python does.
    --raw now RUNS it (exit 0) with the "Raw mode: skipping…" notice, not a refusal: the
    gate flipped from the analyzer capability to the placeholder_params interface trait."""
    prog = tmp_path / "tool"
    prog.write_text("bytes\n", encoding="utf-8")
    store.add_exe(prog, name="tool")
    result = runner.invoke(cli.app, ["run", "tool", "--raw", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "Raw mode: skipping the parameter form and injection." in _flat(result.output)
    assert spawn_spy["entry"].meta.name == "tool"  # it really launched


def test_run_raw_on_unpinned_prompt_refuses_before_runner_resolution(tmp_path, spawn_spy):
    """--raw on a prompt (its {placeholders} ARE the artifact) is refused with exit 2 BEFORE
    runner resolution: the refusal never first asks which agent (nor sends the caller through
    the unpinned 126), and last-picked state is NOT written by a refused run (finding 2)."""
    _prompt(tmp_path, name="p")  # no pin, no --runner
    result = runner.invoke(cli.app, ["run", "p", "--raw", "--no-input"])
    assert result.exit_code == 2, result.output  # usage, NOT the unpinned 126
    assert "--raw doesn't apply to a prompt entry" in _flat(result.output)
    assert "there is no as-is without them" in _flat(result.output)
    assert "entry" not in spawn_spy  # nothing launched
    # last-picked state must be untouched (a refused run picks nothing).
    assert not (argstate.state_dir() / "prompt.toml").exists()
    assert argstate.load_last_runner() == ""


def test_run_raw_on_unpinned_prompt_refuses_without_asking_when_interactive(tmp_path, monkeypatch):
    """Even in an interactive terminal with agents configured, the raw refusal fires
    before the runner ask — Prompt.ask is never reached (finding 2)."""
    _prompt(tmp_path, name="p")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def _boom(*a, **k):
        raise AssertionError("runner ask reached before the raw refusal")

    monkeypatch.setattr(cli.Prompt, "ask", _boom)
    result = runner.invoke(cli.app, ["run", "p", "--raw"])
    assert result.exit_code == 2, result.output
    assert argstate.load_last_runner() == ""  # no pick recorded


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
    # Replacing says "updated", never "added" — the word must match the act.
    assert "Runner codex updated:" in result.output
    assert "added" not in result.output


def test_runner_add_fresh_says_added(tmp_path):
    config.ensure_prompt_runners_seeded()
    result = runner.invoke(cli.app, ["runner", "add", "sonnet", "--", "claude", "{{prompt}}"])
    assert result.exit_code == 0, result.output
    assert "Runner sonnet added:" in result.output  # a genuinely new row says "added"
    assert "updated" not in result.output


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


def test_run_prompt_inline_pin_left_untouched_is_not_a_pick(tmp_path, spawn_spy, monkeypatch):
    """A PINNED prompt whose form comes back with the pin unchanged did not "pick" a
    runner — the last-picked state that prefills future pickers stays put (argstate's
    contract: using a pin is not a pick)."""
    _prompt(tmp_path, pin="claude")
    _prompt_run_interactive(monkeypatch)
    monkeypatch.setattr("skit.inlineform.collect", lambda *a, **k: ({"a": "hi"}, "claude"))
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["runner"] == config.find_prompt_runner("claude")  # ran with the pin
    assert argstate.load_last_runner() == ""  # last-picked state untouched


def test_run_prompt_inline_pick_differing_from_pin_is_saved(tmp_path, spawn_spy, monkeypatch):
    """The twin: choosing a runner that DIFFERS from the pin is a real pick and IS
    remembered for the next picker."""
    _prompt(tmp_path, pin="claude")
    _prompt_run_interactive(monkeypatch)
    monkeypatch.setattr("skit.inlineform.collect", lambda *a, **k: ({"a": "hi"}, "codex"))
    result = runner.invoke(cli.app, ["run", "p"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["runner"] == config.find_prompt_runner("codex")
    assert argstate.load_last_runner() == "codex"  # a real change is remembered


def test_run_prompt_dry_run_still_hosts_the_runner_picker(tmp_path, spawn_spy, monkeypatch):
    """--dry-run no longer forces a bare line ask: an interactive tui prompt dry-run hosts
    the runner picker in the form exactly like a real run (finding 5), then prints the
    resolved command instead of launching."""
    _prompt(tmp_path)
    _prompt_run_interactive(monkeypatch)
    seen: dict[str, object] = {}

    def fake_collect(entry, plan, prefill, runners=None, runner_default=""):
        seen["runners"] = list(runners or [])
        return {"a": "hi"}, "codex"

    monkeypatch.setattr("skit.inlineform.collect", fake_collect)
    ask_hit = {"n": 0}
    monkeypatch.setattr(
        cli.Prompt, "ask", staticmethod(lambda *a, **k: ask_hit.__setitem__("n", 1))
    )
    result = runner.invoke(cli.app, ["run", "p", "--dry-run"])
    assert result.exit_code == 0, result.output
    assert seen["runners"] == [r.name for r in config.load_prompt_runners()]  # picker hosted
    assert ask_hit["n"] == 0  # NOT line-asked, despite --dry-run
    assert "entry" not in spawn_spy  # dry-run launches nothing
    assert "codex" in result.output  # the resolved runner's real command is shown
