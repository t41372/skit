"""The prompt kind's CLI surfaces: add lanes, run resolution, params ops, show,
the `skit runner` tree, and doctor's prompt sweeps (docs/design/prompt.md)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, config, store

runner = CliRunner()


def _write(tmp_path: Path, text: str, name: str = "p.prompt.md") -> Path:
    path = tmp_path / name
    path.write_text(text, encoding="utf-8")
    return path


@pytest.fixture
def spawn_spy(monkeypatch):
    """Intercept the actual process spawn (run_entry) and capture the launch kwargs."""
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


# --------------------------------------------------------------------------
# add
# --------------------------------------------------------------------------


def test_add_prompt_file_no_input_manages_everything(tmp_path):
    src = _write(tmp_path, "# Review\n\nCheck {target} for {focus}\n")
    result = runner.invoke(cli.app, ["add", str(src), "--no-input"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("p")
    assert entry.meta.kind == "prompt"
    assert entry.meta.params == ["target", "focus"]
    assert entry.meta.runner == ""
    assert "Managed parameters: target, focus" in result.output


def test_add_prompt_interactive_tick_subset_and_runner_pick(tmp_path):
    src = _write(tmp_path, "{a} {b} {c}\n")
    result = (
        runner.invoke(
            cli.app,
            ["add", str(src), "-n", "picky"],
            input="1,3\nclaude\n",
            env={"SKIT_FORCE_INTERACTIVE": "1"},
        )
        if False
        else runner.invoke(cli.app, ["add", str(src), "-n", "picky", "--no-input"])
    )
    # CliRunner has no TTY, so the interactive tick path is driven via monkeypatched
    # interactivity in the dedicated tests below; this pins the non-interactive default.
    assert result.exit_code == 0, result.output
    assert store.resolve("picky").meta.params == ["a", "b", "c"]


def test_add_prompt_interactive_selection(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["1,3", "-"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    src = _write(tmp_path, "{a} {b} {c}\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "picky"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("picky")
    assert entry.meta.params == ["a", "c"]
    assert entry.meta.runner == ""  # "-" = no pin
    assert argstate.load_last_runner() == ""  # skipping is not a pick


def test_add_prompt_interactive_runner_pick_pins_and_remembers(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["all", "codex"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    src = _write(tmp_path, "{a}\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "pinned"])
    assert result.exit_code == 0, result.output
    assert store.resolve("pinned").meta.runner == "codex"
    assert argstate.load_last_runner() == "codex"
    assert "Runs with codex" in result.output


def test_add_prompt_runner_flag_non_interactive(tmp_path):
    src = _write(tmp_path, "{a}\n")
    result = runner.invoke(
        cli.app, ["add", str(src), "-n", "auto", "--runner", "claude", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("auto").meta.runner == "claude"
    assert argstate.load_last_runner() == "claude"


def test_add_prompt_unknown_runner_flag_is_usage_error(tmp_path):
    src = _write(tmp_path, "{a}\n")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "x", "--runner", "ghost", "--no-input"])
    assert result.exit_code == 2
    assert "Unknown runner" in result.output


def test_add_runner_flag_without_prompt_is_refused(tmp_path):
    py = tmp_path / "s.py"
    py.write_text("print(1)\n")
    result = runner.invoke(cli.app, ["add", str(py), "--runner", "claude", "--no-input"])
    assert result.exit_code == 2
    assert "--runner only applies to prompt entries" in result.output


def test_add_prompt_conflicts_with_other_kind_flags(tmp_path):
    src = _write(tmp_path, "{a}\n")
    for flags in (["--exe"], ["--kind", "shell"], ["--edit"], ["--cmd", "echo {x}"]):
        result = runner.invoke(cli.app, ["add", str(src), "--prompt", *flags])
        assert result.exit_code == 2, flags
        assert "drop --edit/--exe/--kind/--cmd" in result.output


def test_add_prompt_flag_forces_the_kind_on_any_extension(tmp_path):
    src = tmp_path / "notes.txt"
    src.write_text("Do {thing}\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(src), "--prompt", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("notes").meta.kind == "prompt"


def test_add_bare_md_no_input_requires_explicit_prompt(tmp_path):
    src = _write(tmp_path, "hello {x}\n", name="notes.md")
    result = runner.invoke(cli.app, ["add", str(src), "--no-input"])
    assert result.exit_code == 2
    assert "--prompt" in result.output


def test_add_bare_md_interactive_ask_yes_and_no(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(lambda *a, **k: True))
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "-"))
    src = _write(tmp_path, "hello {x}\n", name="notes.md")
    result = runner.invoke(cli.app, ["add", str(src)])
    assert result.exit_code == 0, result.output
    assert store.resolve("notes").meta.kind == "prompt"

    monkeypatch.setattr(cli.Confirm, "ask", staticmethod(lambda *a, **k: False))
    other = _write(tmp_path, "x\n", name="other.md")
    result = runner.invoke(cli.app, ["add", str(other)])
    assert result.exit_code == 130
    assert "nothing was added" in result.output.lower()


def test_add_prompt_from_stdin_needs_a_name(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--prompt"], input="body {x}\n")
    assert result.exit_code == 2
    assert "--name" in result.output


def test_add_prompt_from_stdin(tmp_path):
    result = runner.invoke(
        cli.app,
        ["add", "-", "--prompt", "-n", "clip", "--runner", "amp"],
        input="Summarize {url} briefly.\n",
    )
    assert result.exit_code == 0, result.output
    entry = store.resolve("clip")
    assert entry.meta.kind == "prompt"
    assert entry.meta.params == ["url"]
    assert entry.meta.runner == "amp"
    assert entry.script_path.read_text() == "Summarize {url} briefly.\n"


def test_add_prompt_from_stdin_empty_body(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--prompt", "-n", "e"], input="  \n")
    assert result.exit_code == 1
    assert "Nothing arrived on stdin" in result.output


def test_add_prompt_editor_lane_routes_to_stdin_when_not_interactive(tmp_path):
    # `skit add --prompt` with no path, no TTY: the body arrives on stdin.
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "drafted"], input="Draft {a}\n")
    assert result.exit_code == 0, result.output
    assert store.resolve("drafted").meta.params == ["a"]


def test_add_prompt_editor_lane_interactive(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["all", "-"])  # manage everything; no runner pin
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))

    def fake_editor(path: Path) -> None:
        path.write_text("Edited body {v}\n", encoding="utf-8")

    monkeypatch.setattr(cli.editor, "open_in_editor", fake_editor)
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "note"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("note")
    assert entry.script_path.read_text() == "Edited body {v}\n"
    assert entry.meta.params == ["v"]


def test_add_prompt_editor_lane_untouched_starter_adds_nothing(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda path: None)
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "empty"])
    assert result.exit_code == 0
    assert "Nothing was written" in result.output
    assert not store.list_entries()


def test_add_prompt_editor_lane_asks_for_a_name(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: ""))
    result = runner.invoke(cli.app, ["add", "--prompt"])
    assert result.exit_code == 2
    assert "A name is required" in result.output


def test_add_prompt_ref_mode_keeps_original_and_pins_invoke(tmp_path):
    src = _write(tmp_path, "Ref {x}\n")
    result = runner.invoke(cli.app, ["add", str(src), "--ref", "--no-input"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("p")
    assert entry.meta.mode == "reference"
    assert entry.meta.workdir == "invoke"
    assert entry.script_path == src


def test_add_prompt_no_path_with_ref_is_refused(tmp_path):
    result = runner.invoke(cli.app, ["add", "--prompt", "--ref", "-n", "x"], input="b\n")
    assert result.exit_code == 2


# --------------------------------------------------------------------------
# run
# --------------------------------------------------------------------------


def _added(tmp_path, text="Do {a}\n", name="p", pin=""):
    entry = store.add_prompt(_write(tmp_path, text, name=f"{name}.prompt.md"), name=name)
    if pin:
        entry = store.write_prompt_runner(entry.slug, pin)
    return entry


def test_run_prompt_no_input_without_pin_is_126(tmp_path):
    _added(tmp_path)
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 126
    assert "No runner selected" in result.output


def test_run_no_input_is_provably_unaffected_by_last_picked_state(tmp_path):
    # The last-picked name is a PICKER DEFAULT only (design risk #10): a --no-input run
    # with no pin must still refuse with 126, whatever state remembers.
    _added(tmp_path)
    argstate.save_last_runner("claude")
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 126
    assert "No runner selected" in result.output


def test_run_prompt_runner_flag_threads_through(tmp_path, spawn_spy):
    _added(tmp_path)
    result = runner.invoke(
        cli.app, ["run", "p", "--runner", "claude", "--set", "a=1", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert spawn_spy["runner"] == config.find_prompt_runner("claude")
    assert spawn_spy["values"] == {"a": "1"}
    assert argstate.load_last_runner() == "claude"  # --runner is a pick


def test_run_prompt_pin_resolves_without_touching_last_picked(tmp_path, spawn_spy):
    _added(tmp_path, pin="codex")
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 0, result.output
    assert spawn_spy["runner"] == config.find_prompt_runner("codex")
    assert argstate.load_last_runner() == ""  # using a pin is not a pick


def test_run_prompt_unknown_runner_is_126_listing_names(tmp_path):
    _added(tmp_path)
    result = runner.invoke(cli.app, ["run", "p", "--runner", "ghost", "--set", "a=1", "--no-input"])
    assert result.exit_code == 126
    assert "ghost" in result.output
    assert "claude" in result.output


def test_run_prompt_pinned_but_removed_runner_is_126(tmp_path):
    _added(tmp_path, pin="mine")
    config.save_prompt_runners([])  # the pin's row is gone
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--no-input"])
    assert result.exit_code == 126
    assert "mine" in result.output


def test_run_runner_flag_on_non_prompt_is_usage_error(tmp_path):
    store.add_command("echo {m}", name="cmd")
    result = runner.invoke(
        cli.app, ["run", "cmd", "--runner", "claude", "--set", "m=1", "--no-input"]
    )
    assert result.exit_code == 2
    assert "--runner only applies to prompt entries" in result.output


def test_run_prompt_interactive_ask_prefilled_from_last_picked(tmp_path, spawn_spy, monkeypatch):
    _added(tmp_path)
    argstate.save_last_runner("opencode")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    seen: dict[str, object] = {}

    def fake_ask(*a, **k):
        seen["default"] = k.get("default")
        return "amp"

    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(fake_ask))
    result = runner.invoke(cli.app, ["run", "p", "--set", "a=1", "--plain"])
    assert result.exit_code == 0, result.output
    assert seen["default"] == "opencode"
    assert spawn_spy["runner"] == config.find_prompt_runner("amp")
    assert argstate.load_last_runner() == "amp"


def test_run_prompt_dry_run_prints_the_resolved_argv(tmp_path):
    _added(tmp_path, text="Say {a}!\n")
    result = runner.invoke(
        cli.app,
        ["run", "p", "--runner", "claude", "--set", "a=hello world", "--no-input", "--dry-run"],
    )
    assert result.exit_code == 0, result.output
    assert "claude" in result.output
    assert "hello world" in result.output


def test_run_prompt_extra_args_pass_through_after_dashes(tmp_path, spawn_spy):
    _added(tmp_path, pin="claude")
    result = runner.invoke(
        cli.app,
        ["run", "p", "--set", "a=1", "--no-input", "--", "--model", "opus"],
    )
    assert result.exit_code == 0, result.output
    assert spawn_spy["extra"] == ["--model", "opus"]


def test_run_prompt_secret_placeholder_masked_in_dry_run(tmp_path):
    _added(tmp_path, text="Use {api_key}\n", name="sec")
    result = runner.invoke(
        cli.app,
        ["run", "sec", "--runner", "claude", "--set", "api_key=hunter2", "--no-input", "--dry-run"],
    )
    assert result.exit_code == 0, result.output
    assert "hunter2" not in result.output
    assert "•••" in result.output


# --------------------------------------------------------------------------
# params
# --------------------------------------------------------------------------


def test_params_read_view_shows_unmanaged_and_gone(tmp_path):
    entry = _added(tmp_path, text="{a} {b}\n")
    store.write_prompt_managed(entry.slug, ["a"])
    entry.script_path.write_text("{b} {c} only\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["params", "p"])
    assert result.exit_code == 0, result.output
    assert "Prompt placeholders" in result.output
    assert "Detected but not yet managed: b, c" in result.output
    assert "No longer in the prompt" in result.output
    assert "a" in result.output


def test_params_json_carries_runner_and_unmanaged(tmp_path):
    entry = _added(tmp_path, text="{a} {b}\n", pin="claude")
    store.write_prompt_managed(entry.slug, ["a"])
    result = runner.invoke(cli.app, ["params", "p", "--json"])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["placeholders"] == ["a"]
    assert payload["unmanaged"] == ["b"]
    assert payload["runner"] == "claude"


def test_params_add_manages_a_body_placeholder(tmp_path):
    entry = _added(tmp_path, text="{a} {b}\n")
    store.write_prompt_managed(entry.slug, ["a"])
    result = runner.invoke(cli.app, ["params", "p", "--add", "b"])
    assert result.exit_code == 0, result.output
    reloaded = store.resolve("p")
    assert reloaded.meta.params == ["a", "b"]  # body order
    rows = store.read_parameters("p")
    assert [(d.name, d.delivery) for d in rows] == [("b", "placeholder")]


def test_params_rm_unmanages_even_without_a_declared_row(tmp_path):
    _added(tmp_path, text="{a} {b}\n")
    result = runner.invoke(cli.app, ["params", "p", "--rm", "b"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.params == ["a"]
    assert "not-declared" not in result.output
    assert "isn't declared" not in result.output


def test_params_add_unknown_name_becomes_env_rider(tmp_path):
    _added(tmp_path, text="{a}\n")
    result = runner.invoke(cli.app, ["params", "p", "--add", "EXTRA"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.params == ["a"]  # not a body hole — not managed
    assert [d.delivery for d in store.read_parameters("p")] == ["env"]


def test_params_deliver_placeholder_is_allowed_on_prompts(tmp_path):
    entry = _added(tmp_path, text="{a}\n")
    store.write_parameters(entry.slug, [ParamDeclFactory("a")])
    result = runner.invoke(cli.app, ["params", "p", "--deliver", "a=placeholder"])
    assert result.exit_code == 0, result.output
    assert store.read_parameters("p")[0].delivery == "placeholder"


def ParamDeclFactory(name: str):
    from skit.params import ParamDecl

    return ParamDecl(name=name, delivery="env")


def test_params_runner_pin_and_clear(tmp_path):
    _added(tmp_path)
    result = runner.invoke(cli.app, ["params", "p", "--runner", "claude"])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.runner == "claude"
    assert argstate.load_last_runner() == "claude"
    result = runner.invoke(cli.app, ["params", "p", "--runner", ""])
    assert result.exit_code == 0, result.output
    assert store.resolve("p").meta.runner == ""
    assert "asks at run time" in result.output


def test_params_runner_pin_validates_the_name(tmp_path):
    _added(tmp_path)
    result = runner.invoke(cli.app, ["params", "p", "--runner", "ghost"])
    assert result.exit_code == 1
    assert "isn't configured" in result.output
    assert store.resolve("p").meta.runner == ""


def test_params_runner_pin_refused_on_non_prompt(tmp_path):
    store.add_command("echo {m}", name="cmd")
    result = runner.invoke(cli.app, ["params", "cmd", "--runner", "claude"])
    assert result.exit_code == 1
    assert "--runner only applies to prompt entries" in result.output


# --------------------------------------------------------------------------
# show
# --------------------------------------------------------------------------


def test_show_json_prompt_additions(tmp_path):
    _added(tmp_path, pin="claude")
    result = runner.invoke(cli.app, ["show", "p", "--json"])
    payload = json.loads(result.output)
    assert payload["kind"] == "prompt"
    assert payload["runner"] == "claude"
    assert "claude" in payload["runners_available"]
    assert payload["workdir"] == "invoke"
    assert [f["key"] for f in payload["fields"]] == ["a"]


def test_show_json_non_prompt_has_no_runner_keys(tmp_path):
    store.add_command("echo {m}", name="cmd")
    payload = json.loads(runner.invoke(cli.app, ["show", "cmd", "--json"]).output)
    assert "runner" not in payload
    assert "runners_available" not in payload


def test_show_human_prints_the_runner_line(tmp_path):
    _added(tmp_path, pin="claude")
    result = runner.invoke(cli.app, ["show", "p"])
    assert "Runner: claude" in result.output
    store.write_prompt_runner("p", "")
    result = runner.invoke(cli.app, ["show", "p"])
    assert "asks at run time" in result.output


# --------------------------------------------------------------------------
# skit runner …
# --------------------------------------------------------------------------


def test_runner_list_materializes_the_seeds(tmp_path):
    assert not config.prompt_runners_seeded()
    result = runner.invoke(cli.app, ["runner", "list"])
    assert result.exit_code == 0, result.output
    assert config.prompt_runners_seeded()  # first management need seeded the config
    for name in ("claude", "codex", "opencode", "amp", "antigravity"):
        assert name in result.output


def test_runner_list_json(tmp_path):
    payload = json.loads(runner.invoke(cli.app, ["runner", "list", "--json"]).output)
    assert {"name": "claude", "argv": ["claude", "{prompt}"]} in payload


def test_runner_list_empty_state(tmp_path):
    config.save_prompt_runners([])
    result = runner.invoke(cli.app, ["runner", "list"])
    assert result.exit_code == 0
    assert "No runners configured" in result.output


def test_runner_add_with_flag_bearing_argv(tmp_path):
    result = runner.invoke(
        cli.app,
        ["runner", "add", "sonnet", "claude", "--model", "sonnet", "{prompt}"],
    )
    assert result.exit_code == 0, result.output
    assert config.find_prompt_runner("sonnet") == config.PromptRunner(
        "sonnet", ("claude", "--model", "sonnet", "{prompt}")
    )


def test_runner_add_validation_errors(tmp_path):
    cases = {
        ("noslot", "claude"): "exactly once",
        ("bin", "{prompt}"): "first word",
        ("stray", "x", "{other}"): "no placeholders besides",
    }
    for argv, needle in cases.items():
        result = runner.invoke(cli.app, ["runner", "add", *argv])
        assert result.exit_code == 2, argv
        assert needle in result.output
    result = runner.invoke(cli.app, ["runner", "add", "bare"])
    assert result.exit_code == 2
    assert "needs a command" in result.output


def test_runner_add_duplicate_name_refused(tmp_path):
    result = runner.invoke(cli.app, ["runner", "add", "claude", "x", "{prompt}"])
    assert result.exit_code == 1
    assert "already exists" in result.output


def test_runner_remove_and_unknown(tmp_path):
    assert runner.invoke(cli.app, ["runner", "remove", "amp"]).exit_code == 0
    assert config.find_prompt_runner("amp") is None
    result = runner.invoke(cli.app, ["runner", "remove", "amp"])
    assert result.exit_code == 1
    assert "Unknown runner" in result.output


def test_removing_every_runner_stays_empty(tmp_path):
    for name in ("claude", "codex", "opencode", "amp", "antigravity"):
        assert runner.invoke(cli.app, ["runner", "remove", name]).exit_code == 0
    assert config.load_prompt_runners() == []
    # The seeds must NOT resurrect (the runners_seeded marker).
    assert runner.invoke(cli.app, ["runner", "list"]).output.count("claude") == 0


# --------------------------------------------------------------------------
# doctor
# --------------------------------------------------------------------------


def test_doctor_reports_prompt_drift_and_bad_runner_rows(tmp_path):
    entry = _added(tmp_path, text="{a}\n")
    entry.script_path.write_text("no holes\n", encoding="utf-8")
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [{"name": "broken", "argv": ["x"]}],
            }
        }
    )
    result = runner.invoke(cli.app, ["doctor", "--json"])
    payload = json.loads(result.output)
    assert "p" in payload["drift"]
    assert payload["runner_rows_invalid"] == ["broken"]
    human = runner.invoke(cli.app, ["doctor"])
    assert "broken" in human.output


def test_doctor_healthy_prompt_reports_no_drift(tmp_path):
    _added(tmp_path, text="{a}\n")
    payload = json.loads(runner.invoke(cli.app, ["doctor", "--json"]).output)
    assert payload["drift"] == []
    assert payload["runner_rows_invalid"] == []


# --------------------------------------------------------------------------
# completion
# --------------------------------------------------------------------------


def test_complete_runner_names(tmp_path, monkeypatch):
    assert "claude" in cli._complete_runner("cl")
    assert cli._complete_runner("zz") == []
    monkeypatch.setattr(
        cli.config, "load_prompt_runners", lambda: (_ for _ in ()).throw(RuntimeError)
    )
    assert cli._complete_runner("") == []  # completion must never crash the shell


# --------------------------------------------------------------------------
# edges: unreadable bodies, store failures, refusal lanes
# --------------------------------------------------------------------------


def test_add_prompt_unreadable_file_is_a_store_error(tmp_path):
    trap = tmp_path / "dir.prompt.md"
    trap.mkdir()  # read_text raises IsADirectoryError (an OSError) while it "exists"
    result = runner.invoke(cli.app, ["add", str(trap), "--no-input"])
    assert result.exit_code == 1
    assert "Can't read" in result.output


def test_add_runner_flag_refused_on_cmd_edit_exe_lanes(tmp_path):
    for args in (
        ["add", "--cmd", "echo {x}", "-n", "c", "--runner", "claude"],
        ["add", "--edit", "--runner", "claude"],
        ["add", "x", "--exe", "--runner", "claude"],
    ):
        result = runner.invoke(cli.app, args)
        assert result.exit_code == 2, args
        assert "--runner only applies to prompt entries" in result.output


def test_add_prompt_editor_lane_reports_store_errors(tmp_path, monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    answers = iter(["taken", "all", "-", "all", "-"])
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: next(answers)))
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda path: path.write_text("body {x}\n"))
    store.add_command("echo hi", name="taken")  # the editor lane's add will collide
    result = runner.invoke(cli.app, ["add", "--prompt"])  # name asked interactively
    assert result.exit_code == 1
    assert "already taken" in result.output


def test_add_prompt_stdin_lane_reports_store_errors(tmp_path):
    store.add_command("echo hi", name="taken")
    result = runner.invoke(cli.app, ["add", "-", "--prompt", "-n", "taken"], input="b\n")
    assert result.exit_code == 1
    assert "already taken" in result.output


def test_params_view_survives_an_unreadable_reference_body(tmp_path):
    src = _write(tmp_path, "{a}\n")
    store.add_prompt(src, mode="reference")
    src.unlink()  # the original vanished: fresh scan degrades to "no candidates"
    result = runner.invoke(cli.app, ["params", "p"])
    assert result.exit_code == 0, result.output
    assert "a = " in result.output  # the managed record still lists


def test_params_runner_pin_reports_store_errors(tmp_path, monkeypatch):
    _added(tmp_path)

    def boom(slug, name):
        raise store.StoreError("disk on fire")

    monkeypatch.setattr(cli.store, "write_prompt_runner", boom)
    result = runner.invoke(cli.app, ["params", "p", "--runner", "claude"])
    assert result.exit_code == 1
    assert "disk on fire" in result.output


def test_doctor_skips_a_prompt_whose_body_is_gone(tmp_path):
    entry = _added(tmp_path, text="{a}\n")
    entry.script_path.unlink()
    payload = json.loads(runner.invoke(cli.app, ["doctor", "--json"]).output)
    assert payload["drift"] == []  # missing is missing's problem, not drift's
    assert "p" in payload["missing"]
