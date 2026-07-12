"""Residual coverage top-up for src/skit/cli.py.

Targets the lines still uncovered after test_cli.py / test_cli_cov.py / test_cli_mut.py /
test_config_cmd.py: the completion helpers, `add -` (stdin), the inline-form renderer branch
of `_collect_values`, run's save-preset / dry-run / degraded-parser / interactive-collect /
assemble-error paths, preset save --from-last, the params --json and --env-source surfaces,
deps --json / --clear / --python-only / conflict, and doctor --json / drift / mirror-on.

Style follows the sibling files: CliRunner for the non-interactive (default) path, and direct
calls to module-level helpers (with a `tty` monkeypatch and stubs) for branches CliRunner
cannot drive.
"""

from __future__ import annotations

import types
from pathlib import Path
from typing import cast

import pytest
import typer
from typer.testing import CliRunner

from skit import argstate, cli, flows, inlineform, launcher, store
from skit.langs.python import analyzer, metawriter
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
def tty(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


@pytest.fixture
def run_entry_spy(monkeypatch: pytest.MonkeyPatch):
    calls: dict[str, object] = {}

    def fake(
        entry,
        extra_args=None,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
    ):
        calls["entry"] = entry
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        calls["override"] = script_override
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake)
    return calls


# --------------------------------------------------------------------------
# _complete_script / _complete_preset (dynamic completion; lines 90-95, 99-107)
# --------------------------------------------------------------------------


def test_complete_script_returns_names_and_slugs(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="alpha")
    store.add_command("echo hi", name="beta")
    assert cli._complete_script("al") == ["alpha"]
    both = cli._complete_script("")
    assert "alpha" in both
    assert "beta" in both


def test_complete_script_swallows_store_errors(monkeypatch):
    def boom() -> list[store.Entry]:
        raise RuntimeError("store is broken")

    monkeypatch.setattr(cli.store, "list_entries", boom)
    assert cli._complete_script("x") == []  # completion must never crash the shell


def _ctx(name: object) -> typer.Context:
    # _complete_preset only reads ctx.params.get("name"); a namespace with a params dict is
    # the honest stand-in (a real typer.Context needs a bound command to construct).
    return cast(typer.Context, types.SimpleNamespace(params={"name": name}))


def test_complete_preset_without_name_is_empty():
    assert cli._complete_preset(_ctx(None), "") == []


def test_complete_preset_lists_matching_presets(tmp_path):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="alpha")
    argstate.save_preset(entry.slug, "prod", {})
    argstate.save_preset(entry.slug, "dev", {})
    assert cli._complete_preset(_ctx("alpha"), "pr") == ["prod"]


def test_complete_preset_swallows_resolve_errors():
    # An unknown name makes store.resolve raise; completion must degrade to [] not crash.
    assert cli._complete_preset(_ctx("ghost"), "") == []


# --------------------------------------------------------------------------
# _default_selection (lines 218-220) / _print_candidate demoted (line 244)
# --------------------------------------------------------------------------


def test_default_selection_all_demoted_is_none():
    demoted = [analyzer.Candidate(binding="const", name="ACC", type="int", default=0, demoted=True)]
    assert cli._default_selection(demoted) == "none"


def test_default_selection_mixed_lists_clean_indices_only():
    mixed = [
        analyzer.Candidate(binding="const", name="CITY", type="str", default="x"),
        analyzer.Candidate(binding="const", name="ACC", type="int", default=0, demoted=True),
    ]
    # Only the first (1-based index 1) is clean, so the demoted second index is excluded.
    assert cli._default_selection(mixed) == "1"


def test_print_candidate_demoted_prints_accumulator_warning(capsys):
    c = analyzer.Candidate(binding="const", name="ACC", type="int", default=0, demoted=True)
    cli._print_candidate(1, c)
    out = " ".join(capsys.readouterr().out.split())
    assert "looks like a loop accumulator" in out


# --------------------------------------------------------------------------
# _print_add_hints argv hint (line 252) / _onboard_params argparse-read message (line 284)
# --------------------------------------------------------------------------


def test_print_add_hints_argv_line(capsys):
    cli._print_add_hints(analyzer.Analysis(uses_argv=True), "tool")
    out = " ".join(capsys.readouterr().out.split())
    assert "reads command-line arguments" in out


def test_onboard_params_argparse_fields_message(capsys):
    text = (
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('-o', '--output', required=True)\nap.parse_args()\n"
    )
    specs = cli._onboard_params(text, "cli-tool", no_input=False)
    assert specs == []
    out = " ".join(capsys.readouterr().out.split())
    assert "skit read this script's own arguments" in out
    assert "1 fields" in out  # one add_argument -> one field


# --------------------------------------------------------------------------
# add - (stdin ingest): lines 406-431, 497-498
# --------------------------------------------------------------------------


def test_add_stdin_ingests_script(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "--name", "clip"], input="print('hi')\n")
    assert result.exit_code == 0, result.output
    assert store.resolve("clip").meta.kind == "python"
    assert store.resolve("clip").meta.mode == "copy"


def test_add_stdin_requires_name():
    result = runner.invoke(cli.app, ["add", "-"], input="print(1)\n")
    assert result.exit_code == 2
    assert "needs an explicit --name" in result.output


def test_add_stdin_empty_input_is_error():
    result = runner.invoke(cli.app, ["add", "-", "--name", "x"], input="")
    assert result.exit_code == 1
    assert "Nothing arrived on stdin" in result.output


def test_add_stdin_store_error_surfaces_as_exit_1():
    runner.invoke(cli.app, ["add", "-", "--name", "dup"], input="print(1)\n")
    result = runner.invoke(cli.app, ["add", "-", "--name", "dup"], input="print(2)\n")
    assert result.exit_code == 1  # duplicate name -> store.StoreError -> clean exit, not traceback


# --------------------------------------------------------------------------
# remove: command-entry confirmation branch (line 649)
# --------------------------------------------------------------------------


def test_remove_command_entry_confirmed(tmp_path):
    store.add_command("echo hi", name="c")
    result = runner.invoke(cli.app, ["remove", "c"], input="y\n")
    assert result.exit_code == 0, result.output
    assert "Removed: c" in result.output
    with pytest.raises(store.NotFoundError):
        store.resolve("c")


# --------------------------------------------------------------------------
# _collect_values: the inline-form ("tui") renderer branch (lines 756-766)
# --------------------------------------------------------------------------


def test_collect_values_inline_form_returns_values(tmp_path, monkeypatch):
    monkeypatch.setenv("TERM", "xterm-256color")  # not "dumb" -> the configured form (tui) wins
    ent = store.add_command("echo {msg}", name="e")
    plan = flows.plan_for_entry(ent)
    monkeypatch.setattr(inlineform, "collect", lambda entry, plan, prefill: {"msg": "typed"})
    values = cli._collect_values(ent, plan, {}, plain=False)
    assert values == {"msg": "typed"}


def test_collect_values_inline_form_cancel_exits_130(tmp_path, monkeypatch):
    monkeypatch.setenv("TERM", "xterm-256color")
    ent = store.add_command("echo {msg}", name="e")
    plan = flows.plan_for_entry(ent)
    monkeypatch.setattr(inlineform, "collect", lambda entry, plan, prefill: None)
    with pytest.raises(typer.Exit) as exc_info:
        cli._collect_values(ent, plan, {}, plain=False)
    assert exc_info.value.exit_code == 130  # EXIT_CANCELLED — cancelling is not a failure


# --------------------------------------------------------------------------
# run: degraded parser hint (842), interactive collect (851), assemble error (871-872),
#      --save-preset (874-880), --dry-run (886-888)
# --------------------------------------------------------------------------

# add_argument() inside a loop can't be modeled statically -> the whole parser degrades to
# the passthrough-args escape (argspec reason "dynamic"), which sets plan.degraded_reason.
DEGRADED_ARGPARSE = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    'for name in ["a", "b"]:\n    ap.add_argument("--" + name)\nap.parse_args()\n'
)


def test_run_degraded_parser_prints_passthrough_hint(tmp_path, run_entry_spy):
    store.add_python(_py(tmp_path, DEGRADED_ARGPARSE), name="sub")
    result = runner.invoke(cli.app, ["run", "sub", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "skit could not model this script's own arguments" in result.output


def test_run_interactive_uses_collect_values(tmp_path, run_entry_spy, monkeypatch):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    store.add_python(_py(tmp_path, text), name="j")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    called: dict[str, bool] = {}

    def fake_collect(entry, plan, prefill, *, plain):
        called["yes"] = True
        return prefill

    monkeypatch.setattr(cli, "_collect_values", fake_collect)
    result = runner.invoke(cli.app, ["run", "j"])  # no --no-input -> interactive path
    assert result.exit_code == 0, result.output
    assert called.get("yes") is True
    assert run_entry_spy["override"] is not None  # injection happened with the collected value


def test_run_assemble_form_error_exits_125(tmp_path, monkeypatch):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")

    def boom(*_a: object, **_k: object) -> object:
        raise flows.FormError("assembly blew up")

    monkeypatch.setattr(cli.flows, "assemble", boom)
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 125  # skit-side failure
    assert "assembly blew up" in result.output


def test_run_save_preset_persists_and_reports(tmp_path, run_entry_spy):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_last(ent.slug, values={"msg": "hi"})
    result = runner.invoke(cli.app, ["run", "e", "--no-input", "--save-preset", "prod"])
    assert result.exit_code == 0, result.output
    assert 'Preset "prod" saved for e.' in result.output
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {"msg": "hi"}


def test_run_dry_run_prints_command_and_exits_0(tmp_path, monkeypatch):
    def never(*_a: object, **_k: object) -> int:
        raise AssertionError("dry-run must not execute the script")

    monkeypatch.setattr(launcher, "run_entry", never)
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    result = runner.invoke(cli.app, ["run", "j", "--no-input", "--dry-run"])
    assert result.exit_code == 0, result.output
    assert "→" in result.output  # the transparency "what would run" line


# --------------------------------------------------------------------------
# preset save --from-last (lines 937-941) / preset list --json (977-978)
# --------------------------------------------------------------------------


def test_preset_save_from_last_saves_remembered_values(tmp_path):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    argstate.save_last(ent.slug, values={"CITY": "Osaka"})
    result = runner.invoke(cli.app, ["preset", "save", "a", "prod", "--from-last"])
    assert result.exit_code == 0, result.output
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {"CITY": "Osaka"}


def test_preset_save_from_last_without_values_errors(tmp_path):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["preset", "save", "a", "prod", "--from-last"])
    assert result.exit_code == 1  # nothing remembered yet
    assert "no remembered values yet" in result.output


def test_preset_list_json(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    argstate.save_preset(ent.slug, "prod", {"CITY": "Taipei"})
    result = runner.invoke(cli.app, ["preset", "list", "a", "--json"])
    assert result.exit_code == 0, result.output
    assert '"prod"' in result.output
    assert '"Taipei"' in result.output


# --------------------------------------------------------------------------
# params: _secret_cell env-source cell (1033), --json view (1050-1056),
#         _apply_env_sources (1209-1224) and its warning render (1278)
# --------------------------------------------------------------------------


def test_secret_cell_shows_env_source():
    s = ParamDecl(name="API", binding="const", type="str", secret=True, env_source="OPENAI_API_KEY")
    assert cli._secret_cell(s) == "yes ← $OPENAI_API_KEY"


def test_params_json_view(tmp_path):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["params", "a", "--json"])
    assert result.exit_code == 0, result.output
    assert '"params"' in result.output
    assert '"unmanaged"' in result.output
    assert '"placeholders"' in result.output
    assert '"CITY"' in result.output


def test_params_env_source_on_unmanaged_warns(tmp_path):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["params", "a", "--env-source", "GHOST=OPENAI"])
    assert result.exit_code == 0, result.output
    assert "GHOST isn't a managed parameter; --env-source skipped." in result.output


def test_params_env_source_on_non_secret_warns(tmp_path):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["params", "a", "--env-source", "CITY=OPENAI"])
    assert result.exit_code == 0, result.output
    assert "CITY isn't secret" in result.output


def test_params_env_source_on_secret_sets_it(tmp_path):
    text = metawriter.write_params(
        'API = "x"\nprint(API)\n',
        [ParamDecl(name="API", binding="const", type="str", default="x", secret=True)],
    )
    entry = store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["params", "a", "--env-source", "API=OPENAI_API_KEY"])
    assert result.exit_code == 0, result.output
    written = metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8"))
    assert written[0].name == "API"
    assert written[0].env_source == "OPENAI_API_KEY"


# --------------------------------------------------------------------------
# deps: --dep/--clear conflict (1330-1333), --json view (1337-1343),
#       --clear (1358), --python-only preserves deps (1362)
# --------------------------------------------------------------------------


def test_deps_dep_and_clear_conflict_is_usage_error(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "requests", "--clear"])
    assert result.exit_code == 2
    assert "not both" in result.output


def test_deps_json_view(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    runner.invoke(cli.app, ["deps", "a", "--dep", "requests", "--python", ">=3.12"])
    result = runner.invoke(cli.app, ["deps", "a", "--json"])
    assert result.exit_code == 0, result.output
    assert '"dependencies"' in result.output
    assert '"requests"' in result.output
    assert '"requires_python"' in result.output
    assert ">=3.12" in result.output


def test_deps_clear_empties_the_list(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    runner.invoke(cli.app, ["deps", "a", "--dep", "requests", "--dep", "rich"])
    result = runner.invoke(cli.app, ["deps", "a", "--clear"])
    assert result.exit_code == 0, result.output
    assert "updated: —" in result.output  # the empty-list dash
    assert not store.resolve("a").meta.dependencies  # cleared (stored as None/empty)


def test_deps_python_only_preserves_existing_deps(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    runner.invoke(cli.app, ["deps", "a", "--dep", "requests"])
    result = runner.invoke(cli.app, ["deps", "a", "--python", ">=3.13"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("a")
    assert entry.meta.dependencies == ["requests"]  # deps untouched by a python-only update
    assert entry.meta.requires_python == ">=3.13"


# --------------------------------------------------------------------------
# doctor: --json (1415-1429), drift line (1386, 1446), mirror-on line (1450)
# --------------------------------------------------------------------------


def test_doctor_json_uv_found(monkeypatch, tmp_path):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["doctor", "--json"])
    assert result.exit_code == 0, result.output
    assert '"uv"' in result.output
    assert '"entries"' in result.output
    assert '"drift"' in result.output


def test_doctor_json_uv_missing_exits_1(monkeypatch):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: None)
    result = runner.invoke(cli.app, ["doctor", "--json"])
    assert result.exit_code == 1  # missing uv -> non-zero even in JSON mode
    assert '"uv"' in result.output


def _drifted_entry(tmp_path: Path, name: str) -> store.Entry:
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name=name)
    script_path = entry.dir / "script.py"
    drifted = script_path.read_text(encoding="utf-8").replace('CITY = "Taipei"', "CITY = 42")
    script_path.write_text(drifted, encoding="utf-8")
    return entry


def test_doctor_reports_drift_with_resync_hint(monkeypatch, tmp_path):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    _drifted_entry(tmp_path, "widget")
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 0, result.output
    out = " ".join(result.output.split())
    assert "widget: form definitions are out of sync" in out
    assert "skit params widget --resync" in out


def test_doctor_mirror_on_line(monkeypatch, tmp_path):
    from skit import config

    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    config.save_mirror(config.preset("tsinghua"))
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 0, result.output
    out = " ".join(result.output.split())
    assert "Mirror: on" in out
    assert config.PYPI_PRESETS["tsinghua"] in out
