"""Behavioural coverage top-up for src/skit/cli.py.

Follows the conventions in test_cli.py / test_config_cmd.py: CliRunner for the non-interactive
(default) path, direct calls to module-level helpers (with a `tty` monkeypatch + stubbed
Prompt.ask) for interactive branches CliRunner cannot reliably drive.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, flows, launcher, promptform, store
from skit.langs.python import metawriter
from skit.params import ParamDecl

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


@pytest.fixture
def tty(monkeypatch):
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


@pytest.fixture
def run_entry_spy(monkeypatch):
    calls = {}

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
        calls["override"] = script_override
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake)
    return calls


# --------------------------------------------------------------------------
# _resolve_python_metadata: 73->75 (existing PEP 723 block, but no dependencies key)
# --------------------------------------------------------------------------


def test_resolve_metadata_existing_block_no_deps_no_print():
    # Block present but the dependencies list is empty -> the "PEP 723 metadata found" line must
    # NOT be printed (nothing to report), and no prompting/filling happens either.
    text = "# /// script\n# dependencies = []\n# ///\nprint(1)\n"
    deps, py = cli._resolve_python_metadata(text, None, None, no_input=False)
    assert deps == []
    assert py == ""


# --------------------------------------------------------------------------
# add: 226->295 (cmd entry with no detected placeholders), 304 (description line)
# --------------------------------------------------------------------------


def test_add_cmd_without_params_no_detected_message():
    result = runner.invoke(cli.app, ["add", "--cmd", "echo hi", "--name", "e"])
    assert result.exit_code == 0, result.output
    assert "Detected parameters" not in result.output
    assert store.resolve("e").meta.params is None


def test_add_prints_description_when_given(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    result = runner.invoke(cli.app, ["add", str(p), "--name", "d", "--description", "does a thing"])
    assert result.exit_code == 0, result.output
    assert "does a thing" in result.output


# --------------------------------------------------------------------------
# add: 274-289 (onboarding selected params get written + secret notice)
# --------------------------------------------------------------------------


def test_add_writes_selected_params_no_secret_notice(tmp_path, monkeypatch):
    p = _py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n')
    specs = [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")]
    monkeypatch.setattr(cli, "_onboard_params", lambda text, name, no_input: specs)
    result = runner.invoke(cli.app, ["add", str(p), "--name", "k"])
    assert result.exit_code == 0, result.output
    assert "Managed parameters: CITY" in result.output
    assert "never saved to disk" not in result.output


def test_add_writes_selected_params_and_secret_notice(tmp_path, monkeypatch):
    p = _py(tmp_path, 'API = "x"\nprint(API)\n')
    specs = [ParamDecl(name="API", binding="const", type="str", default="x", secret=True)]
    monkeypatch.setattr(cli, "_onboard_params", lambda text, name, no_input: specs)
    result = runner.invoke(cli.app, ["add", str(p), "--name", "j"])
    assert result.exit_code == 0, result.output
    assert "Managed parameters: API" in result.output
    assert "API" in result.output
    assert "Secret parameter values are never saved" in result.output
    entry = store.resolve("j")
    written = metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8"))
    assert [s.name for s in written] == ["API"]


# --------------------------------------------------------------------------
# _collect_command_values: 383 (preset merge), 390->386 (no recorded default -> key omitted)
# --------------------------------------------------------------------------


def test_command_prefill_uses_preset(tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_preset(ent.slug, "prod", {"msg": "from-preset"})
    plan = flows.plan_for_entry(ent)
    assert flows.prefill(plan, ent.slug, preset="prod") == {"msg": "from-preset"}


def test_collect_command_values_non_interactive_no_default_omits_key(tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    # No recorded last value and no preset -> the prefill must NOT invent a value; the
    # key is simply absent (left for the launcher to report as missing).
    plan = flows.plan_for_entry(ent)
    assert flows.prefill(plan, ent.slug) == {}


# --------------------------------------------------------------------------
# _collect_param_form: 423 (non-secret interactive assignment)
# --------------------------------------------------------------------------


def test_param_form_interactive_non_secret(monkeypatch, tty, tmp_path):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    plan = flows.plan_for_entry(ent)
    captured: list[dict[str, object]] = []

    def fake_ask(*_a: object, **kw: object) -> str:
        captured.append({"default": kw.get("default")})
        return "typed-city"

    monkeypatch.setattr(cli.Prompt, "ask", fake_ask)
    values = promptform.collect(plan, flows.prefill(plan, ent.slug), console=cli.console)
    assert values == {"CITY": "typed-city"}
    assert captured == [{"default": "Taipei"}]


# --------------------------------------------------------------------------
# run: 436-439 (drift warning printed), 447->exit (valid preset happy path)
# --------------------------------------------------------------------------


def test_run_prints_drift_warning_on_type_change(tmp_path, run_entry_spy):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    # Change CITY's source type from str to int, but keep the [tool.skit] block claiming str.
    script_path = entry.dir / "script.py"
    current = script_path.read_text(encoding="utf-8")
    drifted = current.replace('CITY = "Taipei"', "CITY = 42")
    script_path.write_text(drifted, encoding="utf-8")
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "drifted from the script" in result.output


def test_run_with_valid_preset_succeeds(tmp_path, run_entry_spy):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_preset(ent.slug, "prod", {})
    result = runner.invoke(cli.app, ["run", "j", "--preset", "prod", "--no-input"])
    assert result.exit_code == 0, result.output


# --------------------------------------------------------------------------
# preset save: 590-600 (python entry with managed params: prefilled values saved,
# secret values excluded with a notice)
# --------------------------------------------------------------------------


def test_preset_save_python_with_params_non_interactive_prefill(tmp_path):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    # CliRunner's stdin is not a tty, so _collect_param_form takes the non-interactive path and
    # returns the prefill (the definition's default) without prompting.
    result = runner.invoke(cli.app, ["preset", "save", "a", "prod"])
    assert result.exit_code == 0, result.output
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {"CITY": "Taipei"}


def test_preset_save_python_secret_param_excluded_with_notice(monkeypatch, tty, tmp_path, capsys):
    # Direct call (CliRunner swaps sys.stdin, hiding the tty): a secret value typed into
    # the preset form must be skipped with the notice, never persisted (C3).
    text = metawriter.write_params(
        'API = "x"\nprint(API)\n',
        [ParamDecl(name="API", binding="const", type="str", default="x", secret=True)],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "typed-secret")
    cli.preset_save("a", "prod", from_last=False)
    assert "never stored in presets" in capsys.readouterr().out
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {}


# --------------------------------------------------------------------------
# params: 686->690 (python entry whose stored copy is missing), 709 (non-secret last value shown)
# --------------------------------------------------------------------------


def test_params_python_missing_copy_reports_no_managed_params(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    ent.script_path.unlink()
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0, result.output
    assert "no managed parameters" in result.output


def test_params_python_shows_non_secret_last_value(tmp_path):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    argstate.save_last(ent.slug, values={"CITY": "Osaka"})
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0, result.output
    assert "Osaka" in result.output


# --------------------------------------------------------------------------
# edit: 772-774 (not found), 789-792 (copy missing), 813 (no managed params, no-op view),
# 816->822 (no undetected candidates to report)
# --------------------------------------------------------------------------


def test_edit_not_found():
    result = runner.invoke(cli.app, ["edit", "ghost"])
    assert result.exit_code == 1


def test_edit_copy_missing(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    ent.script_path.unlink()
    result = runner.invoke(cli.app, ["edit", "a"])
    assert result.exit_code == 1
    assert "no stored copy to edit" in result.output


def test_params_no_managed_params_message(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0, result.output
    assert "no managed parameters" in result.output


def test_params_view_no_new_candidates(tmp_path):
    # Every detectable candidate is already managed -> report.new is empty, so the "Detected but
    # not yet managed" hint must not appear.
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0, result.output
    assert "CITY" in result.output
    assert "Detected but not yet managed" not in result.output


# --------------------------------------------------------------------------
# deps: 891-893 (StoreError from update_dependencies surfaces as exit 1)
# --------------------------------------------------------------------------


def test_deps_set_store_error(tmp_path, monkeypatch):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")

    def boom(*a, **k):
        raise store.StoreError("nope")

    monkeypatch.setattr(store, "update_dependencies", boom)
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "requests"])
    assert result.exit_code == 1
    assert "nope" in result.output


# --------------------------------------------------------------------------
# doctor: 922 (--rebuild reports a problem line)
# --------------------------------------------------------------------------


def test_doctor_rebuild_reports_problem(monkeypatch, tmp_path):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    src = _py(tmp_path, "print(1)\n")
    store.add_python(src, name="ref", mode="reference")
    src.unlink()
    result = runner.invoke(cli.app, ["doctor", "--rebuild"])
    assert result.exit_code == 0, result.output
    assert "ref" in result.output
    assert "gone" in result.output
