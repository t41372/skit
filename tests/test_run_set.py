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

from skit import argstate, cli, launcher, store
from skit.langs.python import metawriter
from skit.params import ParamDecl

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


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _inject_entry(tmp_path: Path, name: str = "trip") -> store.Entry:
    text = metawriter.write_params(
        'CITY = "Taipei"\nTIMES = 2\nprint(CITY, TIMES)\n',
        [
            ParamDecl(name="CITY", binding="const", type="str", default="Taipei"),
            ParamDecl(name="TIMES", binding="const", type="int", default=2),
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


def test_save_preset_on_field_less_entry_refused_saves_nothing(run_entry_spy):
    """A field-less entry has nothing to put in a preset — `--save-preset` is refused
    with the same sentence `skit preset save` uses, and nothing is saved OR run. The
    exit code is USAGE (2), NOT 1: inside `run`, 1-124 belongs to the script (docker
    convention), so a skit-side refusal must never look like the script ran."""
    runner.invoke(cli.app, ["add", "--cmd", "echo hi", "--name", "noargs", "--no-input"])
    result = runner.invoke(cli.app, ["run", "noargs", "--save-preset", "nope", "--no-input"])
    assert result.exit_code == 2, result.output
    assert "has no form fields, so there's nothing to save." in result.output
    assert "entry" not in run_entry_spy  # refused before any launch
    assert argstate.load_state(store.resolve("noargs").slug)["presets"] == {}  # saved nothing


def test_save_preset_deferred_until_a_real_run_is_accepted(run_entry_spy):
    """A normal `run --save-preset` persists the preset AFTER the launch is accepted, so
    the 'Preset saved' line prints AFTER the script's own output (deferred persistence)."""
    ent = store.add_command("echo {msg}", name="e")
    result = runner.invoke(
        cli.app, ["run", "e", "--set", "msg=hi", "--save-preset", "prod", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert run_entry_spy["entry"].slug == ent.slug  # it ran
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {"msg": "hi"}


def test_save_preset_not_written_when_launch_is_refused(run_entry_spy, monkeypatch):
    """A launch refusal (outcome.code is None) leaves NO preset — the deferred write is
    gated on acceptance, not merely reaching the run body."""
    from skit import flows

    store.add_command("echo {msg}", name="e")
    monkeypatch.setattr(
        cli.flows,
        "execute",
        lambda *a, **k: flows.RunOutcome(None, flows.FAIL_LAUNCH, "runner vanished"),
    )
    result = runner.invoke(
        cli.app, ["run", "e", "--set", "msg=hi", "--save-preset", "prod", "--no-input"]
    )
    assert result.exit_code != 0
    assert argstate.load_state(store.resolve("e").slug)["presets"] == {}  # nothing persisted


def test_save_preset_dry_run_validation_failure_writes_nothing(tmp_path):
    """`--save-preset X --dry-run` on a prompt whose render is over-long exits 125 and
    persists NO preset — the deferred write sits AFTER dry-run validation."""
    from skit import argstate as _argstate
    from skit.langs.prompt import render

    body = "Do {{a}}\n"
    p = tmp_path / "big.prompt.md"
    p.write_text(body, encoding="utf-8")
    store.add_prompt(p, name="big")
    store.write_prompt_runner(store.resolve("big").slug, "claude")
    huge = "x" * (render.ARGV_LIMIT + 1)
    result = runner.invoke(
        cli.app,
        ["run", "big", "--set", f"a={huge}", "--save-preset", "toolong", "--dry-run", "--no-input"],
    )
    assert result.exit_code == 125, result.output
    assert _argstate.load_state(store.resolve("big").slug).get("presets", {}) == {}


def test_set_secret_never_persisted_and_masked_in_dry_run(tmp_path, run_entry_spy):
    text = metawriter.write_params(
        'KEY = "old"\nprint(KEY)\n',
        [ParamDecl(name="KEY", binding="const", type="str", secret=True)],
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


def test_set_malformed_exits_2_with_exact_message(tmp_path, run_entry_spy):
    _inject_entry(tmp_path)
    for bad in ("NOVALUE", "=v"):
        result = runner.invoke(cli.app, ["run", "trip", "--set", bad, "--no-input"])
        assert result.exit_code == 2, result.output
        # Line-exact: XX-wrapped msgid mutants still contain the substring, and the
        # `and`→`or` parse-guard mutant reroutes these through unknown-name (also 2).
        assert f"Malformed --set (expected NAME=VALUE): {bad}" in result.output.splitlines()
        assert "Unknown parameter" not in result.output
    # Both bad items in one invocation: pins the ", " join between them.
    result = runner.invoke(
        cli.app, ["run", "trip", "--set", "NOVALUE", "--set", "=v", "--no-input"]
    )
    assert result.exit_code == 2, result.output
    assert "Malformed --set (expected NAME=VALUE): NOVALUE, =v" in result.output.splitlines()
    assert "entry" not in run_entry_spy


def test_set_value_may_contain_equals_signs(tmp_path, run_entry_spy):
    entry = _inject_entry(tmp_path)
    result = runner.invoke(cli.app, ["run", "trip", "--set", "CITY=a=b", "--no-input"])
    assert result.exit_code == 0, result.output
    # partition, not rpartition: the FIRST '=' splits, the rest belongs to the value.
    assert argstate.load_state(entry.slug)["values"]["CITY"] == "a=b"


def test_set_key_is_stripped(tmp_path, run_entry_spy):
    entry = _inject_entry(tmp_path)
    result = runner.invoke(cli.app, ["run", "trip", "--set", " CITY =Kaohsiung", "--no-input"])
    assert result.exit_code == 0, result.output
    assert argstate.load_state(entry.slug)["values"]["CITY"] == "Kaohsiung"


def test_set_unknown_name_exits_2_and_lists_valid(tmp_path, run_entry_spy):
    _inject_entry(tmp_path)
    result = runner.invoke(
        cli.app, ["run", "trip", "--set", "NOPE=1", "--set", "ALSO=2", "--no-input"]
    )
    assert result.exit_code == 2
    # Line-exact, with two unknown names so their ", " join is exercised too.
    assert (
        "Unknown parameter for --set: ALSO, NOPE. This entry's parameters: CITY, TIMES"
        in result.output.splitlines()
    )
    assert "entry" not in run_entry_spy


def test_set_on_entry_without_fields_lists_a_dash(tmp_path, run_entry_spy):
    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    exe.chmod(0o755)
    result = runner.invoke(cli.app, ["add", "--exe", str(exe), "--name", "tool", "--no-input"])
    assert result.exit_code == 0, result.output
    result = runner.invoke(cli.app, ["run", "tool", "--set", "X=1", "--no-input"])
    assert result.exit_code == 2
    assert (
        "Unknown parameter for --set: X. This entry's parameters: —" in result.output.splitlines()
    )
    assert "entry" not in run_entry_spy


RAW_CONFLICT = "--raw runs the script as-is; --set, --preset, and --save-preset do not apply."


def test_set_with_raw_is_a_usage_conflict(tmp_path, run_entry_spy):
    _inject_entry(tmp_path)
    result = runner.invoke(cli.app, ["run", "trip", "--raw", "--set", "CITY=x", "--no-input"])
    assert result.exit_code == 2
    # Not the misleading "unknown parameter" — CITY exists; --raw is the conflict.
    assert RAW_CONFLICT in result.output.splitlines()
    assert "entry" not in run_entry_spy


def test_preset_with_raw_is_a_usage_conflict(tmp_path, run_entry_spy):
    entry = _inject_entry(tmp_path)
    argstate.save_preset(entry.slug, "loud", {"CITY": "Tainan"})
    result = runner.invoke(cli.app, ["run", "trip", "--raw", "-p", "loud", "--no-input"])
    assert result.exit_code == 2  # refusing beats silently dropping the preset's values
    assert RAW_CONFLICT in result.output.splitlines()
    assert "entry" not in run_entry_spy


def test_save_preset_with_raw_is_a_usage_conflict(tmp_path, run_entry_spy):
    entry = _inject_entry(tmp_path)
    result = runner.invoke(
        cli.app, ["run", "trip", "--raw", "--save-preset", "ghost", "--no-input"]
    )
    assert result.exit_code == 2
    assert RAW_CONFLICT in result.output.splitlines()
    # The old silent path persisted an EMPTY preset that later validated for -p ghost.
    assert argstate.load_state(entry.slug)["presets"] == {}
    assert "entry" not in run_entry_spy


def test_raw_never_replays_last_extra_args(tmp_path, run_entry_spy):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    result = runner.invoke(cli.app, ["run", "j", "--no-input", "--", "--verbose", "x.png"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["--verbose", "x.png"]
    # --raw promises "as-is": the previous run's arguments must NOT come back.
    result = runner.invoke(cli.app, ["run", "j", "--raw", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == []
    # The escape hatch leaves no fingerprints (beyond the run stamp): a plain run
    # afterwards still reuses the remembered args.
    assert argstate.load_state(entry.slug)["last_run"]["exit"] == 0
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["--verbose", "x.png"]
    # Positive stream pin: the reuse notice is skit chrome and belongs on stderr
    # (SKILL.md documents "it says so on stderr"); the script's stdout stays clean.
    assert "Reusing your last arguments" in result.stderr
    assert "Reusing your last arguments" not in result.stdout


def test_set_bad_typed_value_exits_125(tmp_path, run_entry_spy):
    _inject_entry(tmp_path)
    result = runner.invoke(cli.app, ["run", "trip", "--set", "TIMES=abc", "--no-input"])
    assert result.exit_code == 125
    # The FORM validation message — were --set validation skipped, the shim would
    # still fail with 125 but with its own "isn't a valid" wording.
    assert "TIMES needs a whole number — you typed 'abc'." in result.output.splitlines()
    assert "entry" not in run_entry_spy


def test_set_bad_value_fails_before_the_form_opens(tmp_path, run_entry_spy, monkeypatch):
    # Upfront --set validation is only observable interactively: the non-interactive
    # path re-validates anyway, but the form must never open on an invalid --set.
    _inject_entry(tmp_path)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def explode(*a, **k):  # pragma: no cover — being called IS the failure
        raise AssertionError("the form must not open for an invalid --set value")

    monkeypatch.setattr(cli, "_collect_values", explode)
    result = runner.invoke(cli.app, ["run", "trip", "--set", "TIMES=abc"])
    assert result.exit_code == 125
    assert "TIMES needs a whole number — you typed 'abc'." in result.output.splitlines()
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

    def fake_collect(entry, plan, prefill, *, plain, runners=None, runner_default=""):
        asked["keys"] = [f.key for f in plan.fields]
        # The form's answer must win over any prefill for the fields it asked.
        return {"CITY": "form-city"}, None, False

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


def test_save_preset_no_fields_refused_before_any_form(tmp_path, monkeypatch):
    """The no-form-fields --save-preset refusal fires BEFORE the interactive collection:
    a refused invocation must not first host the runner picker (an answered question)
    or write last-picked runner state (a fingerprint). Regression pin for the hoist —
    previously a field-less prompt entry opened the picker, saved last-runner, and
    only then exited 2."""
    from skit import config

    p = tmp_path / "plain.prompt.md"
    p.write_text("Just do the thing.\n", encoding="utf-8")
    store.add_prompt(p, name="plainp")
    config.ensure_prompt_runners_seeded()  # runner_names non-empty -> picker would host
    config.save_form("tui")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def boom(*a, **kw):  # the form/picker must never open for a refused --save-preset
        raise AssertionError("collection opened before the refusal")

    monkeypatch.setattr(cli, "_collect_values", boom)
    result = runner.invoke(cli.app, ["run", "plainp", "--save-preset", "x"], env={"TERM": "xterm"})
    assert result.exit_code == 2, result.output
    assert "has no form fields" in result.output
    assert not argstate.load_last_runner()  # no fingerprint


def test_save_preset_persists_when_ctrl_c_ends_an_accepted_run(monkeypatch):
    """Ctrl+C ends the RUNNING script, not the request to keep its values: the launch
    was accepted, so --save-preset persists. Regression pin for the deferral — the
    preset must not be lost to the keystroke that normally ends a server/watch
    script."""
    ent = store.add_command("echo {msg}", name="e")

    def interrupt(*a, **kw):
        raise KeyboardInterrupt

    monkeypatch.setattr(cli.flows, "execute", interrupt)
    result = runner.invoke(
        cli.app, ["run", "e", "--set", "msg=hi", "--save-preset", "prod", "--no-input"]
    )
    assert result.exit_code == 130, result.output
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {"msg": "hi"}
