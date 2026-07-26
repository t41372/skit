"""Behavioural coverage top-up for src/skit/cli.py.

Follows the conventions in test_cli.py / test_config_cmd.py: CliRunner for the non-interactive
(default) path, direct calls to module-level helpers (with a `tty` monkeypatch + stubbed
Prompt.ask) for interactive branches CliRunner cannot reliably drive.
"""

from __future__ import annotations

from pathlib import Path

import pytest
import typer
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
    # _is_interactive() now checks stdin AND stdout, so both must look like a tty.
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)


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
# preset save: the interactivity split (a terminal collects the form; a pipe refuses
# rather than guessing) and secret values excluded with a notice
# --------------------------------------------------------------------------


def test_preset_save_non_interactive_refuses_instead_of_guessing(tmp_path):
    """Non-interactively there is nothing to ask and nothing the user chose: the prefill
    is whatever defaults/last-used values happen to be lying around, so minting a preset
    from it would be exactly the "silently assemble" move the contract forbids. skit
    refuses (exit 2, the wrong-shape convention), names the deterministic sources
    (--set, --from-last), and writes NO preset."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    # CliRunner's stdin is not a tty, so _is_interactive() is False.
    result = runner.invoke(cli.app, ["preset", "save", "a", "prod"])
    assert result.exit_code == 2  # wrong-shape refusal, like every missing --yes/--set
    out = " ".join(result.output.split())
    assert "Saving a preset needs a value source in a pipe" in out
    assert "--set NAME=VALUE" in out
    assert "--from-last" in out
    assert argstate.load_state(ent.slug)["presets"] == {}  # nothing guessed into existence


def test_preset_save_piped_stdout_refuses_without_opening_the_form(tmp_path, monkeypatch):
    """The interactive predicate is _is_interactive() (stdin AND stdout): a tty stdin
    with a PIPED stdout must NOT prompt — and since there is no terminal to ask in, it
    refuses rather than falling back to the prefill."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("sys.stdout.isatty", lambda: False, raising=False)  # piped
    monkeypatch.setattr(
        promptform,
        "collect",
        lambda *a, **k: pytest.fail("piped stdout must not open the form"),
    )
    with pytest.raises(typer.Exit) as exc:
        cli.preset_save("a", "prod", from_last=False, set_opts=[])
    assert exc.value.exit_code == 2
    assert argstate.load_state(ent.slug)["presets"] == {}


def test_preset_save_interactive_collects_the_form(tmp_path, monkeypatch):
    """Both stdin and stdout a tty (interactive): the form is collected."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    called: dict[str, object] = {}
    monkeypatch.setattr(
        promptform,
        "collect",
        lambda plan, prefill, console: called.setdefault("v", {"CITY": "Kyoto"}),
    )
    cli.preset_save("a", "prod", from_last=False, set_opts=[])
    assert called["v"] == {"CITY": "Kyoto"}  # the form ran
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {"CITY": "Kyoto"}


def test_preset_save_set_mints_without_running_or_a_terminal(tmp_path):
    """The deterministic non-interactive lane (and the recipe SKILL.md teaches):
    explicit --set values become the preset, nothing executes, no terminal needed."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["preset", "save", "a", "nightly", "--set", "CITY=Kyoto"])
    assert result.exit_code == 0
    assert argstate.load_state(ent.slug)["presets"]["nightly"] == {"CITY": "Kyoto"}


def test_preset_save_set_unknown_name_is_usage_error(tmp_path):
    """--set reuses run's strict parser: an unknown parameter is exit 2, no preset."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["preset", "save", "a", "nightly", "--set", "TOWN=x"])
    assert result.exit_code == 2
    assert argstate.load_state(ent.slug)["presets"] == {}


def test_preset_save_set_and_from_last_conflict(tmp_path):
    """Two value sources in one invocation is a shape error, never a merge."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(
        cli.app,
        ["preset", "save", "a", "nightly", "--set", "CITY=Kyoto", "--from-last"],
    )
    assert result.exit_code == 2
    assert argstate.load_state(ent.slug)["presets"] == {}


def test_preset_save_set_skips_secrets_with_notice(tmp_path):
    """C3 holds on the --set lane too: a secret value never lands in the preset."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nTOKEN = "x"\nprint(CITY, TOKEN)\n',
        [
            ParamDecl(name="CITY", binding="const", type="str", default="Taipei"),
            ParamDecl(name="TOKEN", binding="const", type="str", default="", secret=True),
        ],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(
        cli.app,
        ["preset", "save", "a", "n", "--set", "CITY=Kyoto", "--set", "TOKEN=hunter2"],
    )
    assert result.exit_code == 0
    assert "never stored in presets" in result.output  # the notice, not just the outcome
    saved = argstate.load_state(ent.slug)["presets"]["n"]
    assert saved == {"CITY": "Kyoto"}
    assert "hunter2" not in str(argstate.load_state(ent.slug))


def test_preset_save_all_secret_values_refuses_instead_of_minting_a_husk(tmp_path):
    """When C3 would strip everything, the preset would be {} — the exact husk
    argstate.purge_secret exists to sweep. Refusing (exit 2) keeps the two modules
    agreeing that an empty preset must not exist."""
    text = metawriter.write_params(
        'TOKEN = "x"\nprint(TOKEN)\n',
        [ParamDecl(name="TOKEN", binding="const", type="str", default="", secret=True)],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["preset", "save", "a", "n", "--set", "TOKEN=hunter2"])
    assert result.exit_code == 2
    out = " ".join(result.output.split())
    assert "Nothing left to save" in out
    assert argstate.load_state(ent.slug)["presets"] == {}
    assert "hunter2" not in str(argstate.load_state(ent.slug))


def test_preset_save_python_all_secret_form_refuses_the_husk(monkeypatch, tmp_path):
    # Direct call (CliRunner swaps sys.stdin, hiding the tty): when the form's ONLY
    # field is secret, C3 would strip everything and the preset would be {} — the husk
    # argstate.purge_secret sweeps. The command refuses (exit 2) instead of reporting
    # success for a preset that must not exist; mixed forms keep the skip-notice lane
    # (test_preset_save_set_skips_secrets_with_notice).
    text = metawriter.write_params(
        'API = "x"\nprint(API)\n',
        [ParamDecl(name="API", binding="const", type="str", default="x", secret=True)],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)  # take the form path
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "typed-secret")
    with pytest.raises(typer.Exit) as exc:
        cli.preset_save("a", "prod", from_last=False, set_opts=[])
    assert exc.value.exit_code == 2
    assert argstate.load_state(ent.slug)["presets"] == {}
    assert "typed-secret" not in str(argstate.load_state(ent.slug))


def test_preset_save_set_backfills_unnamed_fields_from_defaults_not_history(tmp_path):
    """--set stores a FULL snapshot like every other preset writer: the field not named
    takes the entry's own declared default — never this machine's last-used value, or
    the "deterministic" lane would mint a different preset on every machine."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nMODE = "fast"\nprint(CITY, MODE)\n',
        [
            ParamDecl(name="CITY", binding="const", type="str", default="Taipei"),
            ParamDecl(name="MODE", binding="const", type="str", default="fast"),
        ],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    # Pollute local history: a past run chose MODE=slow on this machine.
    argstate.save_last(ent.slug, values={"MODE": "slow"})
    result = runner.invoke(cli.app, ["preset", "save", "a", "n", "--set", "CITY=Kyoto"])
    assert result.exit_code == 0
    saved = argstate.load_state(ent.slug)["presets"]["n"]
    assert saved == {"CITY": "Kyoto", "MODE": "fast"}  # default, NOT the remembered "slow"


def test_preset_save_set_only_secrets_on_a_mixed_entry_refuses(tmp_path):
    """Providing ONLY secret values must refuse even when non-secret fields exist:
    backfilling defaults around a fully-stripped input would report success while
    dropping everything the user explicitly asked to persist."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nTOKEN = "x"\nprint(CITY, TOKEN)\n',
        [
            ParamDecl(name="CITY", binding="const", type="str", default="Taipei"),
            ParamDecl(name="TOKEN", binding="const", type="str", default="", secret=True),
        ],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["preset", "save", "a", "n", "--set", "TOKEN=hunter2"])
    assert result.exit_code == 2
    assert argstate.load_state(ent.slug)["presets"] == {}


def test_preset_save_direct_call_without_set_opts_kwarg_still_works(tmp_path, monkeypatch):
    """A direct Python call without the new kwarg leaves typer's OptionInfo default in
    place; the command must treat that as "no --set given", not iterate it."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.promptform.collect", lambda *a, **k: {"CITY": "Kyoto"})
    cli.preset_save("a", "prod", from_last=False)  # no set_opts on purpose
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {"CITY": "Kyoto"}


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
    # The human "Index rebuilt: …" prose still prints when --json is absent.
    assert "Index rebuilt" in result.output


def test_doctor_rebuild_json_carries_report_in_payload(monkeypatch, tmp_path):
    """Under --json stdout is exactly one JSON document: the rebuild report rides in the
    payload (rebuilt count + rebuild_problems) instead of preceding it as prose, so the
    machine contract never has "Index rebuilt: …" prepended before the `{`."""
    import json

    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    src = _py(tmp_path, "print(1)\n")
    store.add_python(src, name="ref", mode="reference")
    src.unlink()  # a reference whose source is gone -> a rebuild problem
    result = runner.invoke(cli.app, ["doctor", "--rebuild", "--json"])
    assert result.exit_code == 0, result.output
    assert "Index rebuilt" not in result.output  # no human prose leaked before the JSON
    payload = json.loads(result.output)  # stdout parses whole
    assert payload["rebuilt"] == 1  # the ref entry was re-indexed
    assert any("ref" in p and "gone" in p for p in payload["rebuild_problems"])


def test_doctor_json_without_rebuild_has_null_rebuilt(monkeypatch, tmp_path):
    """Without --rebuild the payload's rebuilt key is null and rebuild_problems empty — the
    keys are always present (a stable machine contract), only populated when asked."""
    import json

    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["doctor", "--json"])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["rebuilt"] is None
    assert payload["rebuild_problems"] == []
