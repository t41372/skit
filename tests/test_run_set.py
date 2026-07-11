"""`skit run --set NAME=VALUE` — explicit values without a form (issue #2).

Before --set, non-interactive runs could only draw values from defaults, last-used
values, and presets — an inject param or command placeholder was impossible to set
from automation at all (preset save --from-last needs a prior run: chicken and egg).
--set closes that hole under the non-interactive contract: strict parsing, usage
errors for unknown names, the field's own validation for values, and "an explicitly
set field is final" in the interactive form.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, launcher, metawriter, store
from skit.metawriter import ParamSpec

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    return tmp_path


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


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _inject_entry(tmp_path: Path, name: str = "trip") -> store.Entry:
    text = metawriter.write_params(
        'CITY = "Taipei"\nTIMES = 2\nprint(CITY, TIMES)\n',
        [
            ParamSpec(name="CITY", kind="const", type="str", default="Taipei"),
            ParamSpec(name="TIMES", kind="const", type="int", default=2),
        ],
    )
    return store.add_python(_py(tmp_path, text), name=name)


# --------------------------------------------------------------------------
# non-interactive: the agent path
# --------------------------------------------------------------------------


def test_set_inject_values_non_interactive(tmp_path, run_entry_spy):
    entry = _inject_entry(tmp_path)
    result = runner.invoke(
        cli.app, ["run", "trip", "--set", "CITY=Kaohsiung", "--set", "TIMES=3", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert run_entry_spy["override"] is not None  # values were injected
    saved = argstate.load_state(entry.slug)["values"]
    assert saved["CITY"] == "Kaohsiung"
    assert saved["TIMES"] == "3"


def test_set_makes_command_placeholders_runnable(run_entry_spy):
    # THE previously-impossible case: required placeholders, no prior run, no preset.
    result = runner.invoke(
        cli.app, ["add", "--cmd", "echo {target} {level}", "--name", "deploy", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    result = runner.invoke(
        cli.app, ["run", "deploy", "--set", "target=prod", "--set", "level=high", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert run_entry_spy["values"] == {"target": "prod", "level": "high"}


def test_set_wins_over_preset(run_entry_spy):
    runner.invoke(cli.app, ["add", "--cmd", "echo {target}", "--name", "d2", "--no-input"])
    entry = store.resolve("d2")
    argstate.save_preset(entry.slug, "stage", {"target": "staging"})
    result = runner.invoke(
        cli.app, ["run", "d2", "-p", "stage", "--set", "target=prod", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert run_entry_spy["values"] == {"target": "prod"}


def test_set_satisfies_required_argparse_field(tmp_path, run_entry_spy):
    text = (
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('-o', '--output', required=True)\nap.parse_args()\n"
    )
    store.add_python(_py(tmp_path, text), name="ar")
    result = runner.invoke(cli.app, ["run", "ar", "--set", "output=x.png", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["--output", "x.png"]


def test_set_saves_preset_with_dry_run_without_running(run_entry_spy):
    runner.invoke(cli.app, ["add", "--cmd", "echo {target}", "--name", "d3", "--no-input"])
    result = runner.invoke(
        cli.app,
        ["run", "d3", "--set", "target=stage", "--save-preset", "quick", "--dry-run", "--no-input"],
    )
    assert result.exit_code == 0, result.output
    assert "entry" not in run_entry_spy  # dry run: nothing executed
    assert argstate.load_state(store.resolve("d3").slug)["presets"] == {
        "quick": {"target": "stage"}
    }


def test_set_secret_never_persisted_and_masked_in_dry_run(tmp_path, run_entry_spy):
    text = metawriter.write_params(
        'KEY = "old"\nprint(KEY)\n',
        [ParamSpec(name="KEY", kind="const", type="str", secret=True)],
    )
    entry = store.add_python(_py(tmp_path, text), name="api")
    result = runner.invoke(
        cli.app, ["run", "api", "--set", "KEY=s3cret-value", "--dry-run", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert "s3cret-value" not in result.output
    assert "•••" in result.output
    result = runner.invoke(cli.app, ["run", "api", "--set", "KEY=s3cret-value", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "KEY" not in argstate.load_state(entry.slug)["values"]  # C3: never on disk


def test_set_token_values_expand_at_assembly(run_entry_spy):
    runner.invoke(cli.app, ["add", "--cmd", "echo {where}", "--name", "d4", "--no-input"])
    result = runner.invoke(cli.app, ["run", "d4", "--set", "where={cwd}", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["values"]["where"] == str(Path.cwd())
    # Intent is persisted, not expansion: the saved value keeps the token.
    assert argstate.load_state(store.resolve("d4").slug)["values"]["where"] == "{cwd}"


# --------------------------------------------------------------------------
# error contract: never guess
# --------------------------------------------------------------------------


def test_set_malformed_exits_2(tmp_path, run_entry_spy):
    _inject_entry(tmp_path)
    for bad in ("NOVALUE", "=v"):
        result = runner.invoke(cli.app, ["run", "trip", "--set", bad, "--no-input"])
        assert result.exit_code == 2, result.output
    assert "entry" not in run_entry_spy


def test_set_unknown_name_exits_2_and_lists_valid(tmp_path, run_entry_spy):
    _inject_entry(tmp_path)
    result = runner.invoke(cli.app, ["run", "trip", "--set", "NOPE=1", "--no-input"])
    assert result.exit_code == 2
    assert "NOPE" in result.output
    assert "CITY" in result.output
    assert "TIMES" in result.output
    assert "entry" not in run_entry_spy


def test_set_with_raw_has_no_fields_exits_2(tmp_path, run_entry_spy):
    _inject_entry(tmp_path)
    result = runner.invoke(cli.app, ["run", "trip", "--raw", "--set", "CITY=x", "--no-input"])
    assert result.exit_code == 2  # --raw skips the form: there is nothing to set
    assert "entry" not in run_entry_spy


def test_set_bad_typed_value_exits_125(tmp_path, run_entry_spy):
    _inject_entry(tmp_path)
    result = runner.invoke(cli.app, ["run", "trip", "--set", "TIMES=abc", "--no-input"])
    assert result.exit_code == 125
    assert "TIMES" in result.output
    assert "entry" not in run_entry_spy


def test_set_empty_value_on_required_placeholder_exits_125(run_entry_spy):
    runner.invoke(cli.app, ["add", "--cmd", "echo {target}", "--name", "d5", "--no-input"])
    result = runner.invoke(cli.app, ["run", "d5", "--set", "target=", "--no-input"])
    assert result.exit_code == 125
    assert "entry" not in run_entry_spy


# --------------------------------------------------------------------------
# interactive: an explicitly set field is final
# --------------------------------------------------------------------------


def test_interactive_form_skips_set_fields(tmp_path, run_entry_spy, monkeypatch):
    entry = _inject_entry(tmp_path)
    argstate.save_last(entry.slug, values={"CITY": "old-city"})
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    asked: dict[str, object] = {}

    def fake_collect(entry, plan, prefill, *, plain):
        asked["keys"] = [f.key for f in plan.fields]
        # The form's answer must win over any prefill for the fields it asked.
        return {"CITY": "form-city"}

    monkeypatch.setattr(cli, "_collect_values", fake_collect)
    result = runner.invoke(cli.app, ["run", "trip", "--set", "TIMES=9"])
    assert result.exit_code == 0, result.output
    assert asked["keys"] == ["CITY"]  # TIMES was --set, so the form never asks for it
    saved = argstate.load_state(entry.slug)["values"]
    assert saved == {"CITY": "form-city", "TIMES": "9"}


def test_interactive_all_fields_set_skips_the_form_entirely(tmp_path, run_entry_spy, monkeypatch):
    _inject_entry(tmp_path)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def explode(*a, **k):  # pragma: no cover — failing here IS the assertion
        raise AssertionError("the form must not open when every field is --set")

    monkeypatch.setattr(cli, "_collect_values", explode)
    result = runner.invoke(cli.app, ["run", "trip", "--set", "CITY=x", "--set", "TIMES=1"])
    assert result.exit_code == 0, result.output
