"""CLI end-to-end (Typer CliRunner) + direct unit tests for interactive helper functions.

- Command layer uses CliRunner: the non-interactive path is the default (CliRunner's stdin is
  not a tty).
- Interactive branches (Prompt.ask / isatty True) are tested by calling the module functions
  directly with stubs, because CliRunner replaces sys.stdin during invoke and cannot reliably
  inject a tty.
- Assertions are behavioural (exit code, on-disk results, args passed to launcher), not tied
  to any locale copy.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import analyzer, argstate, cli, launcher, metawriter, shim, store
from skit.metawriter import ParamSpec

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# --------------------------------------------------------------------------
# main callback
# --------------------------------------------------------------------------


def test_version_flag_prints_and_exits():
    from skit import __version__

    result = runner.invoke(cli.app, ["--version"])
    assert result.exit_code == 0
    assert __version__ in result.output


def test_no_subcommand_dispatches_to_tui(monkeypatch):
    called = {}

    def fake_menu() -> int:
        called["ran"] = True
        return 0

    # cli imports run_menu inside the callback with `from .tui import run_menu`; patching the
    # module attribute is sufficient.
    monkeypatch.setattr("skit.tui.run_menu", fake_menu)
    result = runner.invoke(cli.app, [])
    assert result.exit_code == 0
    assert called.get("ran") is True


# --------------------------------------------------------------------------
# add
# --------------------------------------------------------------------------


def test_add_python_copy(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    result = runner.invoke(cli.app, ["add", str(p), "--name", "hi"])
    assert result.exit_code == 0, result.output
    assert store.resolve("hi").meta.mode == "copy"


def test_add_python_reference_skips_onboarding(tmp_path):
    p = _py(tmp_path, 'CITY = "x"\nprint(CITY)\n')
    result = runner.invoke(cli.app, ["add", str(p), "--name", "ref", "--ref"])
    assert result.exit_code == 0, result.output
    assert store.resolve("ref").meta.mode == "reference"


def test_add_rejects_non_py(tmp_path):
    p = _py(tmp_path, "data", name="notes.txt")
    result = runner.invoke(cli.app, ["add", str(p)])
    assert result.exit_code == 2


def test_add_needs_path():
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 2


def test_add_exe_needs_path():
    result = runner.invoke(cli.app, ["add", "--exe"])
    assert result.exit_code == 2


def test_add_exe(tmp_path):
    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(exe), "--exe", "--name", "tool"])
    assert result.exit_code == 0, result.output
    assert store.resolve("tool").meta.kind == "exe"


def test_add_cmd_needs_name():
    result = runner.invoke(cli.app, ["add", "--cmd", "echo hi"])
    assert result.exit_code == 2


def test_add_cmd_with_params(tmp_path):
    result = runner.invoke(cli.app, ["add", "--cmd", "echo {msg}", "--name", "e"])
    assert result.exit_code == 0, result.output
    assert store.resolve("e").meta.params == ["msg"]


def test_add_with_explicit_deps_records(tmp_path):
    p = _py(tmp_path, "import requests\nprint(requests)\n")
    result = runner.invoke(
        cli.app, ["add", str(p), "--name", "r", "--deps", "requests, rich", "--no-input"]
    )
    assert result.exit_code == 0, result.output


def test_add_name_conflict_errors(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    runner.invoke(cli.app, ["add", str(p), "--name", "dup"])
    result = runner.invoke(cli.app, ["add", str(p), "--name", "dup"])
    assert result.exit_code == 1


def test_add_onboards_params_non_interactive_skips(tmp_path):
    # --no-input: even when candidates are found, don't select any and don't write [tool.skit]
    p = _py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n')
    result = runner.invoke(cli.app, ["add", str(p), "--name", "j", "--no-input"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("j")
    assert metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8")) == []


# --------------------------------------------------------------------------
# list
# --------------------------------------------------------------------------


def test_list_empty():
    result = runner.invoke(cli.app, ["list"])
    assert result.exit_code == 0


def test_list_table(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["list"])
    assert result.exit_code == 0
    assert "a" in result.output


def test_list_json(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["list", "--json"])
    assert result.exit_code == 0
    assert '"slug"' in result.output


# --------------------------------------------------------------------------
# remove
# --------------------------------------------------------------------------


def test_remove_not_found():
    result = runner.invoke(cli.app, ["remove", "ghost"])
    assert result.exit_code == 1


def test_remove_with_yes(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["remove", "a", "--yes"])
    assert result.exit_code == 0
    with pytest.raises(store.NotFoundError):
        store.resolve("a")


def test_remove_confirm_abort(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["remove", "a"], input="n\n")
    assert result.exit_code != 0  # abort
    assert store.resolve("a")  # still there


# --------------------------------------------------------------------------
# run
# --------------------------------------------------------------------------


@pytest.fixture
def run_entry_spy(monkeypatch):
    calls = {}

    def fake(entry, extra_args=None, *, values=None, invoke_cwd=None, script_override=None):
        calls["entry"] = entry
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        calls["override"] = script_override
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake)
    return calls


def test_run_python_with_params_injects(tmp_path, run_entry_spy):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamSpec(name="CITY", kind="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 0, result.output
    # Has parameter definitions → an injected artifact path is passed to the launcher
    assert run_entry_spy["override"] is not None


def test_run_not_found():
    result = runner.invoke(cli.app, ["run", "ghost"])
    assert result.exit_code == 1


def test_run_raw_skips_form(tmp_path, run_entry_spy):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamSpec(name="CITY", kind="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["run", "j", "--raw", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["override"] is None  # --raw: no injection


def test_run_unknown_preset_rejected(tmp_path, run_entry_spy):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    result = runner.invoke(cli.app, ["run", "j", "--preset", "nope", "--no-input"])
    assert result.exit_code == 2


def test_run_passes_and_remembers_extra_args(tmp_path, run_entry_spy):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    result = runner.invoke(cli.app, ["run", "j", "--no-input", "--", "--flag", "v"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["--flag", "v"]
    # Run again without args: last-used args are reused
    result2 = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result2.exit_code == 0, result2.output
    assert run_entry_spy["extra"] == ["--flag", "v"]


def test_run_nonzero_exit_propagates(tmp_path, run_entry_spy):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    run_entry_spy["code"] = 3
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 3


def test_run_shim_error(tmp_path, run_entry_spy, monkeypatch):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamSpec(name="CITY", kind="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="j")

    def boom(*a, **k):
        raise shim.ShimError("nope")

    monkeypatch.setattr(shim, "inject", boom)
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 1


def test_run_launch_error(tmp_path, monkeypatch):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")

    def boom(*a, **k):
        raise launcher.LaunchError("bad")

    monkeypatch.setattr(launcher, "run_entry", boom)
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 1


def test_run_command_entry_collects_values(tmp_path, run_entry_spy):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_last(ent.slug, values={"msg": "hi"})
    result = runner.invoke(cli.app, ["run", "e", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["values"] == {"msg": "hi"}


# --------------------------------------------------------------------------
# preset
# --------------------------------------------------------------------------


def test_preset_list_none(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["preset", "list", "a"])
    assert result.exit_code == 0


def test_preset_list_shows(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    argstate.save_preset(ent.slug, "prod", {"CITY": "Taipei"})
    result = runner.invoke(cli.app, ["preset", "list", "a"])
    assert "prod" in result.output


def test_preset_list_not_found():
    result = runner.invoke(cli.app, ["preset", "list", "ghost"])
    assert result.exit_code == 1


def test_preset_delete(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    argstate.save_preset(ent.slug, "prod", {"CITY": "Taipei"})
    result = runner.invoke(cli.app, ["preset", "delete", "a", "prod"])
    assert result.exit_code == 0
    assert argstate.load_state(ent.slug)["presets"] == {}


def test_preset_delete_unknown(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["preset", "delete", "a", "nope"])
    assert result.exit_code == 1


def test_preset_delete_not_found():
    result = runner.invoke(cli.app, ["preset", "delete", "ghost", "p"])
    assert result.exit_code == 1


def test_preset_save_not_found():
    result = runner.invoke(cli.app, ["preset", "save", "ghost", "p"])
    assert result.exit_code == 1


def test_preset_save_python_no_params(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["preset", "save", "a", "p"], input="\n")
    assert result.exit_code == 1  # no managed parameters


def test_preset_save_command_no_params(tmp_path):
    store.add_command("echo hi", name="e")  # no placeholders
    result = runner.invoke(cli.app, ["preset", "save", "e", "p"])
    assert result.exit_code == 1


def test_preset_save_command_with_params(tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    result = runner.invoke(cli.app, ["preset", "save", "e", "prod"], input="hello\n")
    assert result.exit_code == 0, result.output
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {"msg": "hello"}


# --------------------------------------------------------------------------
# params
# --------------------------------------------------------------------------


def test_params_not_found():
    result = runner.invoke(cli.app, ["params", "ghost"])
    assert result.exit_code == 1


def test_params_empty(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0


def test_params_command_entry(tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_last(ent.slug, values={"msg": "hi"})
    result = runner.invoke(cli.app, ["params", "e"])
    assert result.exit_code == 0
    assert "msg" in result.output


def test_params_command_no_placeholders(tmp_path):
    store.add_command("echo hi", name="e")
    result = runner.invoke(cli.app, ["params", "e"])
    assert result.exit_code == 0


def test_params_python_table_with_secret(tmp_path):
    text = metawriter.write_params(
        'API = "x"\nprint(API)\n',
        [ParamSpec(name="API", kind="const", type="str", default="x", secret=True)],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    argstate.save_last(ent.slug, values={"API": "shown"}, secret_names={"API"})
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0
    assert "API" in result.output


# --------------------------------------------------------------------------
# deps
# --------------------------------------------------------------------------


def test_deps_view(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a"])
    assert result.exit_code == 0


def test_deps_not_found():
    result = runner.invoke(cli.app, ["deps", "ghost"])
    assert result.exit_code == 1


def test_deps_not_python(tmp_path):
    store.add_command("echo hi", name="e")
    result = runner.invoke(cli.app, ["deps", "e"])
    assert result.exit_code == 1


def test_deps_set(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--set", "requests, rich", "--python", ">=3.11"])
    assert result.exit_code == 0, result.output
    assert store.resolve("a").meta.dependencies == ["requests", "rich"]


def test_deps_view_with_requires_python(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    runner.invoke(cli.app, ["deps", "a", "--set", "requests", "--python", ">=3.12"])
    result = runner.invoke(cli.app, ["deps", "a"])
    assert result.exit_code == 0
    assert "3.12" in result.output


# --------------------------------------------------------------------------
# doctor
# --------------------------------------------------------------------------


def test_doctor_uv_found(monkeypatch, tmp_path):
    monkeypatch.setattr(launcher, "find_uv", lambda: "/usr/bin/uv")
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 0


def test_doctor_uv_missing(monkeypatch):
    monkeypatch.setattr(launcher, "find_uv", lambda: None)
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 1


def test_doctor_rebuild(monkeypatch, tmp_path):
    monkeypatch.setattr(launcher, "find_uv", lambda: "/usr/bin/uv")
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["doctor", "--rebuild"])
    assert result.exit_code == 0


def test_doctor_reports_missing_reference(monkeypatch, tmp_path):
    monkeypatch.setattr(launcher, "find_uv", lambda: "/usr/bin/uv")
    src = _py(tmp_path, "print(1)\n")
    store.add_python(src, name="ref", mode="reference")
    src.unlink()
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 0
    assert "ref" in result.output


# --------------------------------------------------------------------------
# lang
# --------------------------------------------------------------------------


def test_lang_show():
    result = runner.invoke(cli.app, ["lang"])
    assert result.exit_code == 0


def test_lang_set_valid():
    result = runner.invoke(cli.app, ["lang", "zh-TW"])
    assert result.exit_code == 0


def test_lang_auto():
    result = runner.invoke(cli.app, ["lang", "auto"])
    assert result.exit_code == 0


def test_lang_unknown():
    result = runner.invoke(cli.app, ["lang", "xx-YY"])
    assert result.exit_code == 2


# --------------------------------------------------------------------------
# Interactive helpers: called directly + stubbed (CliRunner cannot reliably inject a tty)
# --------------------------------------------------------------------------


@pytest.fixture
def tty(monkeypatch):
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)


def test_parse_selection_variants():
    assert cli._parse_selection("all", 3) == [0, 1, 2]
    assert cli._parse_selection("none", 3) == []
    assert cli._parse_selection("", 3) == []
    assert cli._parse_selection("1,3", 3) == [0, 2]
    assert cli._parse_selection("1,1,9,x", 3) == [0]  # dedup + out-of-range / non-numeric ignored


def test_parse_prompt_opts():
    prompts, bad = cli._parse_prompt_opts(["A=hello", "B=", "no-eq", "=novalue"])
    assert prompts == {"A": "hello", "B": ""}
    assert bad == ["no-eq", "=novalue"]


def test_resolve_metadata_existing_block_not_asked():
    text = '# /// script\n# dependencies = ["requests"]\n# ///\nprint(1)\n'
    deps, py = cli._resolve_python_metadata(text, None, None, no_input=False)
    assert deps == []
    assert py == ""


def test_resolve_metadata_explicit_opts():
    deps, py = cli._resolve_python_metadata("print(1)\n", "requests, rich", ">=3.11", False)
    assert deps == ["requests", "rich"]
    assert py == ">=3.11"


def test_resolve_metadata_no_suggestions():
    deps, py = cli._resolve_python_metadata("print(1)\n", None, None, no_input=False)
    assert deps == []
    assert py == ""


def test_resolve_metadata_non_interactive_uses_suggestions():
    deps, _py = cli._resolve_python_metadata(
        "import requests\nprint(requests)\n", None, None, no_input=True
    )
    assert deps == ["requests"]


def test_resolve_metadata_interactive(monkeypatch, tty):
    answers = iter(["requests, rich", ">=3.12"])
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: next(answers))
    deps, py = cli._resolve_python_metadata(
        "import requests\nprint(requests)\n", None, None, no_input=False
    )
    assert deps == ["requests", "rich"]
    assert py == ">=3.12"


def test_onboard_params_framework_detected(monkeypatch, tty):
    text = "import argparse\np = argparse.ArgumentParser()\n"
    specs = cli._onboard_params(text, "cli-tool", no_input=False)
    assert specs == []


def test_onboard_params_no_candidates(tty):
    assert cli._onboard_params("print(1)\n", "x", no_input=False) == []


def test_onboard_params_non_interactive_returns_empty():
    text = 'CITY = "Taipei"\nprint(CITY)\n'
    assert cli._onboard_params(text, "x", no_input=True) == []


def test_onboard_params_interactive_selection(monkeypatch, tty):
    text = 'CITY = "Taipei"\nRETRIES = 3\nwho = input("Name: ")\nprint(CITY, RETRIES, who)\n'
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "all")
    specs = cli._onboard_params(text, "x", no_input=False)
    assert len(specs) >= 2
    assert any(s.name == "CITY" for s in specs)


def test_spec_from_candidate_roundtrip():
    result = analyzer.analyze('CITY = "Taipei"\nprint(CITY)\n')
    spec = cli._spec_from_candidate(result.candidates[0])
    assert spec.name == "CITY"


def test_collect_command_values_interactive(monkeypatch, tty, tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "typed")
    values = cli._collect_command_values(ent, no_input=False, preset=None)
    assert values == {"msg": "typed"}


def test_collect_command_values_non_interactive_uses_last(tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_last(ent.slug, values={"msg": "remembered"})
    values = cli._collect_command_values(ent, no_input=True, preset=None)
    assert values == {"msg": "remembered"}


def test_collect_command_values_no_params(tmp_path):
    ent = store.add_command("echo hi", name="e")
    assert cli._collect_command_values(ent, no_input=True, preset=None) == {}


def test_collect_param_form_interactive_secret(monkeypatch, tty, tmp_path):
    text = metawriter.write_params(
        'API = "x"\nprint(API)\n',
        [ParamSpec(name="API", kind="const", type="str", secret=True)],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    specs = [ParamSpec(name="API", kind="const", type="str", secret=True)]
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "secretval")
    values = cli._collect_param_form(ent, specs, no_input=False, preset=None)
    assert values == {"API": "secretval"}


def test_collect_param_form_non_interactive_returns_prefill(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    specs = [ParamSpec(name="CITY", kind="const", type="str", default="Osaka")]
    values = cli._collect_param_form(ent, specs, no_input=True, preset=None)
    assert values == {"CITY": "Osaka"}
