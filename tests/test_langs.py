"""Language registry (langs/): completeness gate + unknown-kind degradation contract.

The registry replaces the old closed Literal["python","exe","command"] — with an open
kind string, exhaustiveness can't come from the type checker anymore, so this file IS
the completeness gate: every KNOWN_KIND must resolve to a fully-populated LangSpec, and
an unknown kind (a meta written by a newer skit) must degrade at every consumer instead
of crashing (list/show keep working; only run fails, with a clean LaunchError).
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import launcher
from skit.langs import registry
from skit.models import Entry, ScriptMeta

runner = CliRunner()


def _entry(
    tmp_path: Path,
    kind: str,
    *,
    template: str = "",
    source: str = "",
    workdir: str = "origin",
) -> Entry:
    meta = ScriptMeta(name="thing", kind=kind, template=template, source=source, workdir=workdir)
    return Entry(slug="thing", meta=meta, dir=tmp_path / "thing")


# ---- registry completeness -------------------------------------------------------------------


def test_every_known_kind_resolves_to_a_complete_spec():
    assert {
        "python",
        "shell",
        "fish",
        "js",
        "ts",
        "powershell",
        "ruby",
        "perl",
        "lua",
        "r",
        "exe",
        "command",
        "prompt",
    } == registry.KNOWN_KINDS
    for kind in registry.KNOWN_KINDS:
        spec = registry.spec_for(kind)
        assert spec is not None
        assert spec.kind == kind
        assert spec.glyph  # every kind has a badge glyph
        assert spec.family in ("interpreted", "binary", "template")
        # the launch strategy is required and fully formed
        for method in ("build", "describe", "target", "preflight"):
            assert callable(getattr(spec.launch, method))


def test_python_spec_capabilities_and_pinned_store_name():
    spec = registry.spec_for("python")
    assert spec is not None
    # stored_name is PINNED: existing stores carry script.py on disk (compat trap #2 in
    # docs/design/multilang.md) — renaming it would orphan every installed library.
    assert spec.stored_name == "script.py"
    assert spec.extensions == (".py",)
    assert spec.comment is not None
    assert spec.comment.prefix == "#"
    assert spec.analyzer is not None
    assert spec.cli_reader is not None
    assert spec.params_io is not None
    assert spec.supports_modes
    assert spec.supports_deps
    assert spec.takes_argv
    assert spec.editable
    assert spec.has_original_file


def test_exe_and_command_specs_have_no_analysis_capabilities():
    exe = registry.spec_for("exe")
    cmd = registry.spec_for("command")
    assert exe is not None
    assert cmd is not None
    for spec in (exe, cmd):
        assert spec.analyzer is None
        assert spec.cli_reader is None
        assert spec.params_io is None
        assert not spec.supports_modes
        assert not spec.supports_deps
        assert not spec.editable  # no stored text source to open in an editor
    assert exe.family == "binary"
    assert exe.has_original_file
    assert cmd.family == "template"
    assert not cmd.has_original_file
    # command's "arguments" are its placeholders — appended argv is not its interface,
    # so run's reuse-last-args affordance must skip it.
    assert exe.takes_argv
    assert not cmd.takes_argv


def test_spec_for_unknown_kind_is_none_and_cached():
    assert registry.spec_for("martian") is None
    assert registry.spec_for("martian") is None  # cached path, same answer


def test_stored_name_unknown_kind_falls_back_to_payload():
    # A newer store's copy-mode entry must still resolve to *some* path (never crash).
    assert registry.stored_name("martian") == "payload"
    assert registry.stored_name("python") == "script.py"
    assert registry.stored_name("exe") == ""
    assert registry.stored_name("command") == ""


# ---- unknown-kind degradation at every launcher consumer --------------------------------------


def test_unknown_kind_build_command_raises_clean_launch_error(tmp_path: Path):
    entry = _entry(tmp_path, "martian")
    with pytest.raises(launcher.LaunchError) as exc:
        launcher.build_command(entry)
    assert "martian" in str(exc.value)


def test_unknown_kind_run_entry_raises_before_spawning(tmp_path: Path):
    entry = _entry(tmp_path, "martian")
    with pytest.raises(launcher.LaunchError):
        launcher.run_entry(entry)


def test_unknown_kind_describe_returns_template_and_never_raises(tmp_path: Path):
    # describe_command is contracted side-effect-free and total: for a kind this skit
    # version doesn't know, the template is the only launch material meta carries.
    entry = _entry(tmp_path, "martian", template="frob --it")
    assert launcher.describe_command(entry) == "frob --it"
    assert launcher.describe_command(_entry(tmp_path, "martian")) == ""


def test_unknown_kind_never_reports_missing(tmp_path: Path):
    # Nothing this version can check — a missing-marker would be a false alarm.
    entry = _entry(tmp_path, "martian", source=str(tmp_path / "gone"))
    assert launcher.target_missing(entry) is False
    assert launcher.missing_marker(entry) is None


def test_unknown_kind_preflight_still_checks_workdir(tmp_path: Path):
    ok = _entry(tmp_path, "martian", workdir=str(tmp_path))
    launcher.preflight(ok, invoke_cwd=tmp_path)  # no strategy checks, workdir fine
    gone = _entry(tmp_path, "martian", workdir=str(tmp_path / "nope"))
    with pytest.raises(launcher.LaunchError):
        launcher.preflight(gone, invoke_cwd=tmp_path)


def test_unknown_kind_script_path_uses_payload_fallback(tmp_path: Path):
    entry = _entry(tmp_path, "martian")
    assert entry.script_path == entry.dir / "payload"


# ---- launcher's dynamic uv delegates -----------------------------------------------------------


def test_launcher_uv_delegates_follow_patches_on_the_canonical_module(monkeypatch):
    # The delegation must be dynamic: patching the canonical skit.langs.launch namespace
    # has to reach consumers that call through launcher.find_uv/ensure_uv, or test/user
    # monkeypatching would split-brain between the two module namespaces.
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/patched/uv")
    assert launcher.find_uv() == "/patched/uv"
    monkeypatch.setattr("skit.langs.launch.ensure_uv", lambda: "/patched/uv2")
    assert launcher.ensure_uv() == "/patched/uv2"


# ---- audited fixes: capability-honest CLI behavior --------------------------------------------


def _exe(tmp_path: Path) -> Path:
    prog = tmp_path / ("tool.exe" if sys.platform == "win32" else "tool")
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    return prog


def test_params_exe_prints_plain_message_without_manage_dead_end(tmp_path: Path):
    # `--manage` hard-errors for kinds without an analyzer, so the empty-params message
    # must not send exe users down that dead end (it used to suggest --manage).
    from skit import cli, store

    store.add_exe(_exe(tmp_path), name="prog")
    result = runner.invoke(cli.app, ["params", "prog"])
    assert result.exit_code == 0
    assert "has no managed parameters" in result.output
    assert "--manage" not in result.output


def test_doctor_missing_uv_pure_exe_library_exits_zero(tmp_path: Path, monkeypatch):
    # A library with no python entries runs fine without uv — exit 1 there sent
    # automation chasing a phantom problem. The red uv line still prints.
    from skit import cli, store

    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: None)
    store.add_exe(_exe(tmp_path), name="prog")
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 0
    assert "uv" in result.output


def test_doctor_missing_uv_with_python_entry_exits_one(tmp_path: Path, monkeypatch):
    from skit import cli, store

    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: None)
    py = tmp_path / "a.py"
    py.write_text("print(1)\n", encoding="utf-8")
    store.add_python(py, name="a")
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 1


def test_doctor_json_missing_uv_pure_exe_library_exits_zero(tmp_path: Path, monkeypatch):
    from skit import cli, store

    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: None)
    store.add_exe(_exe(tmp_path), name="prog")
    result = runner.invoke(cli.app, ["doctor", "--json"])
    assert result.exit_code == 0


# ---- plan_for_entry: capability degradation ----------------------------------------------------


def test_plan_without_cli_reader_degrades_to_none_plan(tmp_path: Path, monkeypatch):
    # A future kind can carry params_io+analyzer but no static CLI reader (e.g. shell
    # before a getopts reader exists) — the plan must fall through to "none", not crash.
    import dataclasses

    from skit import flows, store

    python_spec = registry.spec_for("python")
    assert python_spec is not None
    stripped = dataclasses.replace(python_spec, cli_reader=None)
    monkeypatch.setattr("skit.flows.spec_for", lambda kind: stripped)
    py = tmp_path / "b.py"
    py.write_text("import argparse\np = argparse.ArgumentParser()\np.add_argument('--x')\n")
    entry = store.add_python(py, name="b")
    plan = flows.plan_for_entry(entry)
    assert plan.source == "none"
