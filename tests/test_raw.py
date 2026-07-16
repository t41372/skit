"""`skit run --raw` escape hatch: skip the parameter form and injection, run the script as-is."""

from __future__ import annotations

import pytest
from typer.testing import CliRunner

from skit import cli, store

SCRIPT = 'CITY = "Taipei"\nprint(CITY)\n'
TOOL_SCT = (
    "# [tool.skit]\n# [[tool.skit.params]]\n# name = 'CITY'\n# kind = 'const'\n# type = 'str'\n"
)


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))


@pytest.fixture
def entry_with_params(tmp_path):
    from skit.langs.python import metawriter
    from skit.params import ParamDecl

    script = tmp_path / "hello.py"
    text = metawriter.write_params(SCRIPT, [ParamDecl(name="CITY", binding="const", type="str")])
    script.write_text(text, encoding="utf-8")
    return store.add_python(script, mode="copy")


def _run(monkeypatch, args: list[str]):
    """Run the CLI and intercept launcher.run_entry. Returns (exit_code, captured kwargs)."""
    captured: dict[str, object] = {}

    def fake_run_entry(
        entry, extra, *, values=None, invoke_cwd=None, script_override=None, env_overlay=None
    ):
        captured["script_override"] = script_override
        captured["values"] = values
        return 0

    monkeypatch.setattr(cli.launcher, "run_entry", fake_run_entry)
    runner = CliRunner()
    result = runner.invoke(cli.app, ["run", *args])
    return result, captured


def test_raw_skips_form_and_injection(monkeypatch, entry_with_params):
    result, captured = _run(monkeypatch, [entry_with_params.meta.name, "--raw", "--no-input"])
    assert result.exit_code == 0, result.output
    assert captured["script_override"] is None  # no injected artifact; the copy runs directly


def test_default_run_injects(monkeypatch, entry_with_params):
    # A managed value exists (remembered from a "previous run"), so the default path
    # injects it into a temp copy. With no value at all there is nothing to inject and
    # the stored copy runs directly — that case is test_no_values_runs_copy_directly.
    from skit import argstate

    argstate.save_last(entry_with_params.slug, values={"CITY": "Kaohsiung"})
    result, captured = _run(monkeypatch, [entry_with_params.meta.name, "--no-input"])
    assert result.exit_code == 0, result.output
    assert captured["script_override"] is not None


def test_no_values_runs_copy_directly(monkeypatch, entry_with_params):
    # No default, no last value: nothing to inject; the copy runs as written.
    result, captured = _run(monkeypatch, [entry_with_params.meta.name, "--no-input"])
    assert result.exit_code == 0, result.output
    assert captured["script_override"] is None


def test_raw_does_not_leave_injected_artifact(monkeypatch, entry_with_params):
    _run(monkeypatch, [entry_with_params.meta.name, "--raw", "--no-input"])
    assert not list(entry_with_params.dir.glob(".injected*"))


def test_normal_run_cleans_injected_artifact(monkeypatch, entry_with_params):
    _run(monkeypatch, [entry_with_params.meta.name, "--no-input"])
    assert not list(entry_with_params.dir.glob(".injected*"))
