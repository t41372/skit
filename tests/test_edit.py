"""`skit edit`: TOML-free parameter definition editing, and reconcile.edit_specs pure logic."""

from __future__ import annotations

import pytest
from typer.testing import CliRunner

from skit import cli, metawriter, reconcile, store
from skit.metawriter import ParamSpec

# Two candidates: CITY (const) and input-1 (order 0) — used by add/resync tests.
SCRIPT = 'CITY = "Taipei"\nRETRIES = 3\nwho = input("Name: ")\nprint(CITY, RETRIES, who)\n'


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_LANG", "en")


def spec(name, kind="const", type="str", order=-1, secret=False, prompt=""):
    return ParamSpec(name=name, kind=kind, type=type, order=order, secret=secret, prompt=prompt)


# ---------- reconcile.edit_specs pure logic ----------


def test_resync_drops_missing_and_keeps_matching():
    specs = [spec("CITY"), spec("GONE")]  # GONE not in the script
    res = reconcile.edit_specs(SCRIPT, specs, resync=True)
    names = [s.name for s in res.specs]
    assert names == ["CITY"]
    assert "resync-dropped:GONE" in res.warnings


def test_resync_updates_changed_type_preserving_customization():
    # RETRIES is int in the script but was mis-annotated as str; the user added secret/prompt.
    specs = [spec("RETRIES", type="str", secret=True, prompt="How many? ")]
    res = reconcile.edit_specs(SCRIPT, specs, resync=True)
    s = res.specs[0]
    assert s.type == "int"  # type corrected to match the script
    assert s.secret is True  # user customisation preserved
    assert s.prompt == "How many? "


def test_add_brings_candidate_under_management():
    res = reconcile.edit_specs(SCRIPT, [spec("CITY")], add=["RETRIES"])
    names = [s.name for s in res.specs]
    assert names == ["CITY", "RETRIES"]  # newly added appended at the end
    assert res.specs[1].type == "int"


def test_add_input_candidate_by_display_name():
    res = reconcile.edit_specs(SCRIPT, [], add=["input-1"])
    assert res.specs[0].kind == "input"
    assert res.specs[0].order == 0


def test_add_already_managed_and_not_candidate_warn():
    res = reconcile.edit_specs(SCRIPT, [spec("CITY")], add=["CITY", "NOPE"])
    assert "already-managed:CITY" in res.warnings
    assert "not-a-candidate:NOPE" in res.warnings


def test_remove_and_secret_toggles():
    specs = [spec("CITY"), spec("RETRIES", type="int")]
    res = reconcile.edit_specs(
        SCRIPT, specs, remove=["CITY"], secret=["RETRIES"], prompts={"RETRIES": "N: "}
    )
    assert [s.name for s in res.specs] == ["RETRIES"]
    assert res.specs[0].secret is True
    assert res.specs[0].prompt == "N: "


def test_no_secret_and_missing_name_warns():
    res = reconcile.edit_specs(SCRIPT, [spec("CITY", secret=True)], no_secret=["CITY", "GHOST"])
    assert res.specs[0].secret is False
    assert "not-managed:GHOST" in res.warnings


def test_edit_specs_is_pure_no_mutation_of_input_list():
    original = [spec("CITY")]
    reconcile.edit_specs(SCRIPT, original, remove=["CITY"])
    assert [s.name for s in original] == ["CITY"]  # input list must not be mutated


# ---------- CLI end-to-end ----------


@pytest.fixture
def entry(tmp_path):
    script = tmp_path / "job.py"
    text = metawriter.write_params(
        SCRIPT, [spec("CITY"), spec("RETRIES", type="int"), spec("GONE")]
    )
    # Make GONE a drift item: defined but absent from SCRIPT (no GONE assignment)
    script.write_text(text, encoding="utf-8")
    return store.add_python(script, mode="copy")


def _read_back(entry) -> list[ParamSpec]:
    return metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8"))


def test_cli_resync_prunes_and_persists(entry):
    runner = CliRunner()
    result = runner.invoke(cli.app, ["params", entry.meta.name, "--resync"])
    assert result.exit_code == 0, result.output
    names = [s.name for s in _read_back(entry)]
    assert "GONE" not in names
    assert set(names) == {"CITY", "RETRIES"}


def test_cli_secret_and_prompt_persist(entry):
    runner = CliRunner()
    result = runner.invoke(
        cli.app, ["params", entry.meta.name, "--secret", "CITY", "--prompt", "CITY=Where? "]
    )
    assert result.exit_code == 0, result.output
    by_name = {s.name: s for s in _read_back(entry)}
    assert by_name["CITY"].secret is True
    assert by_name["CITY"].prompt == "Where? "


def test_cli_params_view_no_ops(entry):
    runner = CliRunner()
    result = runner.invoke(cli.app, ["params", entry.meta.name])
    assert result.exit_code == 0, result.output
    assert "CITY" in result.output
    # The read view must not modify any definitions
    assert len(_read_back(entry)) == 3


def test_cli_bad_prompt_is_warned_not_fatal(entry):
    runner = CliRunner()
    result = runner.invoke(cli.app, ["params", entry.meta.name, "--prompt", "no-equals-sign"])
    assert result.exit_code == 0, result.output


def test_cli_params_edit_reference_refused(tmp_path):
    script = tmp_path / "ref.py"
    script.write_text(SCRIPT, encoding="utf-8")
    ent = store.add_python(script, mode="reference")
    runner = CliRunner()
    result = runner.invoke(cli.app, ["params", ent.meta.name, "--resync"])
    assert result.exit_code == 1
    # The original file must never be modified
    assert script.read_text(encoding="utf-8") == SCRIPT


def test_cli_edit_command_entry_has_no_source(monkeypatch):
    # `skit edit` on a non-Python entry must refuse before ever launching an editor.
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda *a, **k: _no_editor())
    ent = store.add_command("echo {x}", name="ec")
    runner = CliRunner()
    result = runner.invoke(cli.app, ["edit", ent.meta.name])
    assert result.exit_code == 1


def _no_editor():
    raise AssertionError("editor must not be launched")
