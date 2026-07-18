"""Behavioural tests targeting mutation-testing survivors in skit/cli.py.

These tests exist to strengthen assertions that were previously too loose to catch mutants
(wrong message text, swapped and/or, dropped kwargs, off-by-one arithmetic, etc.) — not to pad
coverage, which is already at 100%. Style matches tests/test_cli.py and tests/test_config_cmd.py:
CliRunner for the non-interactive path, direct calls + stubs for interactive helpers.
"""

from __future__ import annotations

from pathlib import Path

import pytest
import typer
from typer.testing import CliRunner

from skit import (
    analysis,
    argstate,
    cli,
    config,
    flows,
    i18n,
    launcher,
    pep723,
    promptform,
    store,
)
from skit.i18n import gettext
from skit.langs.python import metawriter, shim
from skit.params import ParamDecl

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")
    return tmp_path


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


@pytest.fixture
def tty(monkeypatch):
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)


def _norm(text: str) -> str:
    """Collapse rich's terminal-width line wrapping so long messages can be matched as one line."""
    return " ".join(text.split())


def _capture_ask(monkeypatch: pytest.MonkeyPatch, module, attr: str, answers: list[object]):
    """Stub Prompt.ask/Confirm.ask, capturing (args, kwargs) of every call in call order."""
    it = iter(answers)
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def fake(*a: object, **kw: object) -> object:
        calls.append((a, kw))
        return next(it)

    monkeypatch.setattr(module, attr, fake)
    return calls


# --------------------------------------------------------------------------
# _resolve_python_metadata
# --------------------------------------------------------------------------


def test_resolve_metadata_existing_block_with_deps_prints_exact_message(capsys):
    text = '# /// script\n# dependencies = ["requests", "rich"]\n# ///\nprint(1)\n'
    deps, py = cli._resolve_python_metadata(text, None, None, no_input=False)
    assert deps == []
    assert py == ""
    out = _norm(capsys.readouterr().out)
    assert "The script declares its own dependencies (PEP 723): requests, rich" in out
    assert "XX" not in out  # catches string-wrapping mutants that keep the substring intact


def test_resolve_metadata_explicit_python_only_no_deps():
    # deps_opt is None but python_opt given: takes the explicit-opts branch (not "and"), deps
    # stays empty (the "" fallback, not some other default).
    deps, py = cli._resolve_python_metadata("print(1)\n", None, ">=3.11", no_input=False)
    assert deps == []
    assert py == ">=3.11"


def test_resolve_metadata_explicit_deps_only_empty_python():
    # python_opt explicitly "" (falsy but not None): falls back to "", not some sentinel.
    deps, py = cli._resolve_python_metadata("print(1)\n", ["requests"], "", no_input=False)
    assert deps == ["requests"]
    assert py == ""


def test_resolve_metadata_no_input_tty_still_non_interactive(tty):
    # no_input=True must short-circuit even when stdin IS a tty (no_input OR not isatty, not AND).
    deps, py = cli._resolve_python_metadata(
        "import requests\nprint(requests)\n", None, None, no_input=True
    )
    assert deps == ["requests"]
    assert py == ""


def test_resolve_metadata_interactive_prompts_exact_text_and_defaults(monkeypatch, tty):
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["requests, rich", ">=3.12"])
    # Two third-party imports so the suggested-deps default exercises the ", " join separator.
    deps, py = cli._resolve_python_metadata(
        "import requests\nimport rich\nprint(requests, rich)\n", None, None, no_input=False
    )
    assert deps == ["requests", "rich"]
    assert py == ">=3.12"
    assert len(calls) == 2
    (msg1, kw1), (msg2, kw2) = calls
    assert msg1[0] == gettext(
        "Dependencies to install (Enter to accept, edit the list, or '-' for none)"
    )
    assert kw1["default"] == "requests, rich"
    assert kw1["console"] is cli.console
    assert msg2[0] == gettext("Python version (leave empty for automatic)")
    assert kw2["default"] == ""
    assert kw2["console"] is cli.console


# --------------------------------------------------------------------------
# _spec_from_candidate
# --------------------------------------------------------------------------


def test_spec_from_candidate_copies_every_field():
    c = analysis.Candidate(
        binding="input",
        name="who",
        type="int",
        default=42,
        prompt="Name: ",
        order=3,
        secret=True,
    )
    spec = ParamDecl.from_candidate(c)
    assert (
        spec.name,
        spec.binding,
        spec.type,
        spec.default,
        spec.prompt,
        spec.order,
        spec.secret,
    ) == (
        "who",
        "input",
        "int",
        42,
        "Name: ",
        3,
        True,
    )


# --------------------------------------------------------------------------
# _parse_selection (equivalent-mutant note lives in the final report, not here)
# --------------------------------------------------------------------------


def test_parse_selection_out_of_range_and_dup_still_ignored():
    assert cli._parse_selection("2,2,5,0", 3) == [1]


# --------------------------------------------------------------------------
# _onboard_params
# --------------------------------------------------------------------------


def test_onboard_params_framework_message_exact(monkeypatch, tty, capsys):
    # Two frameworks so the ", " join separator in the names list is exercised too.
    text = "import argparse\nimport click\np = argparse.ArgumentParser()\n"
    specs = cli._onboard_params(text, "cli-tool", no_input=False)
    assert specs == []
    out = _norm(capsys.readouterr().out)
    assert (
        "This script parses its own arguments (argparse, click); skit couldn't model them "
        "statically, so the run form offers a passthrough-arguments field." in out
    )
    assert "XX" not in out


def test_onboard_params_no_input_short_circuits_even_on_tty(tty):
    # no_input=True must win even when stdin is a tty (no_input OR not isatty, not AND).
    text = 'CITY = "Taipei"\nprint(CITY)\n'
    assert cli._onboard_params(text, "x", no_input=True) == []


def test_onboard_params_candidate_listing_exact_text(monkeypatch, tty, capsys):
    text = 'API_KEY = "shh"\nwho = input("Name: ")\nprint(API_KEY, who)\n'
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["none"])
    specs = cli._onboard_params(text, "x", no_input=False)
    assert specs == []
    out = capsys.readouterr().out
    assert "Found 2 parameter candidates (constants / input() calls):" in out
    # const candidate: secret-marked, exact numbering/format, two-space indent
    assert "  1. API_KEY (str) = 'shh' (secret)" in out
    # input() candidate: ordinal is order+1 (0-based order -> #1), exact prompt repr, indent
    assert "  2. input() #1: 'Name: '" in out
    assert "XX" not in out
    # the "which ones" prompt: exact text + default
    (msg,), kw = calls[0]
    assert msg == gettext("Which ones should skit manage? (e.g. 1,3 / all / none)")
    assert kw["default"] == "all"
    assert kw["console"] is cli.console


def test_onboard_params_singular_count_message(monkeypatch, tty, capsys):
    text = 'CITY = "Taipei"\nprint(CITY)\n'
    _capture_ask(monkeypatch, cli.Prompt, "ask", ["none"])
    cli._onboard_params(text, "x", no_input=False)
    out = capsys.readouterr().out
    assert "Found 1 parameter candidate (constants / input() calls):" in out
    assert "parameter candidates" not in out
    assert "XX" not in out


def test_onboard_params_non_secret_const_has_no_secret_mark(monkeypatch, tty, capsys):
    text = 'CITY = "Taipei"\nprint(CITY)\n'
    _capture_ask(monkeypatch, cli.Prompt, "ask", ["none"])
    cli._onboard_params(text, "x", no_input=False)
    out = capsys.readouterr().out
    assert "  1. CITY (str) = 'Taipei'\n" in out  # exact line end: no secret mark, no junk suffix
    assert "(secret)" not in out
    assert "XX" not in out


# --------------------------------------------------------------------------
# _collect_command_values
# --------------------------------------------------------------------------


def test_command_prefill_unknown_preset_does_not_crash(tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    # No preset named "ghost" was ever saved: must fall back to {} defaults, not crash.
    plan = flows.plan_for_entry(ent)
    assert flows.prefill(plan, ent.slug, preset="ghost") == {}


def test_command_preset_default_offered_when_interactive(monkeypatch, tty):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_preset(ent.slug, "prod", {"msg": "from-preset"})
    plan = flows.plan_for_entry(ent)
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["typed"])
    values = promptform.collect(
        plan, flows.prefill(plan, ent.slug, preset="prod"), console=cli.console
    )
    assert values == {"msg": "typed"}
    (label,), kw = calls[0]
    assert label == "  msg"
    assert kw["default"] == "from-preset"
    assert kw["console"] is cli.console


def test_run_no_input_never_prompts_even_on_tty(tty, monkeypatch):
    # no_input=True must force non-interactive even though stdin looks like a tty
    # (the `not no_input and isatty` conjunction in run()).
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_last(ent.slug, values={"msg": "remembered"})
    captured: dict[str, object] = {}

    def fake_run(
        entry,
        extra,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
        runner=None,
    ):
        captured["values"] = values
        return 0

    monkeypatch.setattr(launcher, "run_entry", fake_run)

    def boom(*_a: object, **_k: object) -> str:
        raise AssertionError("must not prompt under --no-input")

    monkeypatch.setattr(cli.Prompt, "ask", boom)
    result = runner.invoke(cli.app, ["run", "e", "--no-input"])
    assert result.exit_code == 0, result.output
    assert captured["values"] == {"msg": "remembered"}


def test_form_no_default_empty_answer_is_empty_not_sentinel(monkeypatch, tty, tmp_path):
    # An OPTIONAL field (command placeholders are required now and would re-prompt):
    # Rich's Prompt.ask(default=None) returns None on empty input, so `or ""` (not
    # `or "XXXX"`) must decide the stored value.
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    ent = store.add_python(_py(tmp_path, text), name="opt")
    plan = flows.plan_for_entry(ent)
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: None)
    values = promptform.collect(plan, {}, console=cli.console)
    assert values == {"CITY": ""}


# --------------------------------------------------------------------------
# _collect_param_form
# --------------------------------------------------------------------------


def test_run_threads_slug_and_preset_into_prefill(monkeypatch, tmp_path):
    # run() must pass the entry's slug AND the -p preset into flows.prefill (a preset->None
    # or slug->"" mutant would silently drop the preset's values from the launched command).
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_preset(ent.slug, "prod", {"msg": "from-preset"})
    captured: dict[str, object] = {}

    def fake_run(
        entry,
        extra,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
        runner=None,
    ):
        captured["values"] = values
        return 0

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    result = runner.invoke(cli.app, ["run", "e", "-p", "prod", "--no-input"])
    assert result.exit_code == 0, result.output
    assert captured["values"] == {"msg": "from-preset"}


def test_collect_values_header_exact_text(monkeypatch, tty, tmp_path, capsys):
    text = metawriter.write_params(
        'CITY = "Osaka"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Osaka")],
    )
    ent = store.add_python(_py(tmp_path, text), name="widget")
    plan = flows.plan_for_entry(ent)
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "Kyoto")
    cli._collect_values(ent, plan, flows.prefill(plan, ent.slug), plain=True)
    out = _norm(capsys.readouterr().out)
    assert "Parameters for widget (press Enter to keep the value shown):" in out
    assert "XX" not in out


def test_param_form_custom_prompt_label_used(monkeypatch, tty, tmp_path):
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", prompt="Which city?")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["Kyoto"])
    promptform.collect(flows.plan_for_entry(ent), {}, console=cli.console)
    (label,), _kw = calls[0]
    assert label == "  Which city?"


def test_param_form_falls_back_to_name_when_no_prompt(monkeypatch, tty, tmp_path):
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["Kyoto"])
    promptform.collect(flows.plan_for_entry(ent), {}, console=cli.console)
    (label,), _kw = calls[0]
    assert label == "  CITY"


def test_param_form_secret_calls_password_true_and_stays_empty(monkeypatch, tty, tmp_path):
    # v2 intent change: secrets are never prefilled and an empty answer stays empty —
    # a secret's "default" is a value, and values for secrets never echo or persist.
    text = metawriter.write_params(
        'API = "x"\nprint(API)\n',
        [ParamDecl(name="API", binding="const", type="str", secret=True, default="fallback")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", [""])
    values = promptform.collect(flows.plan_for_entry(ent), {}, console=cli.console)
    assert values == {"API": ""}
    (label,), kw = calls[0]
    assert label == "  API"
    assert kw["password"] is True
    assert kw["console"] is cli.console


def test_param_form_secret_no_default_falls_back_to_empty(monkeypatch, tty, tmp_path):
    text = metawriter.write_params(
        'API = "x"\nprint(API)\n',
        [ParamDecl(name="API", binding="const", type="str", secret=True)],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "")
    values = promptform.collect(flows.plan_for_entry(ent), {}, console=cli.console)
    assert values == {"API": ""}


def test_param_form_non_secret_empty_answer_is_empty_not_sentinel(monkeypatch, tty, tmp_path):
    text = metawriter.write_params(
        'CITY = "Osaka"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Osaka")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    plan = flows.plan_for_entry(ent)
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", [""])
    values = promptform.collect(plan, flows.prefill(plan, ent.slug), console=cli.console)
    assert values == {"CITY": ""}
    (label,), kw = calls[0]
    assert label == "  CITY"
    assert kw["default"] == "Osaka"
    assert kw["console"] is cli.console


# --------------------------------------------------------------------------
# _entry_param_specs
# --------------------------------------------------------------------------


def test_plan_for_entry_tolerates_invalid_utf8_bytes(tmp_path):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    # Corrupt the stored copy with invalid UTF-8 bytes; errors="replace" must keep this from
    # raising (an errors="strict"/invalid-errors-mode mutant would blow up here).
    with open(ent.script_path, "r+b") as f:
        data = f.read()
        f.seek(0)
        f.write(data + b"\xff\xfe garbage")
        f.truncate()
    # Must not raise.
    flows.plan_for_entry(ent)


# --------------------------------------------------------------------------
# _validate_preset
# --------------------------------------------------------------------------


def test_validate_preset_unknown_message_exact(tmp_path, capsys):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    argstate.save_preset(ent.slug, "b", {})
    argstate.save_preset(ent.slug, "a", {})
    with pytest.raises(typer.Exit) as exc_info:
        cli._validate_preset(ent, "ghost")
    assert exc_info.value.exit_code == 2
    err = _norm(capsys.readouterr().err)
    assert 'Unknown preset "ghost". Available: a, b' in err
    assert "XX" not in err


def test_validate_preset_unknown_no_presets_shows_dash(tmp_path, capsys):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    with pytest.raises(typer.Exit):
        cli._validate_preset(ent, "ghost")
    err = capsys.readouterr().err
    assert "Available: —" in err


def test_validate_preset_none_is_a_noop(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    cli._validate_preset(ent, None)  # must not raise


# --------------------------------------------------------------------------
# _print_settings
# --------------------------------------------------------------------------


def test_config_list_mirror_off_lines(capsys):
    result = runner.invoke(cli.app, ["config"])
    assert result.exit_code == 0
    out = _norm(result.output)
    assert "lang" in out
    assert "mirror off" in out
    assert "form tui" in out
    assert "XX" not in out


def test_config_list_mirror_on_shows_url(capsys):
    config.save_mirror(config.preset("tsinghua"))
    result = runner.invoke(cli.app, ["config"])
    out = _norm(result.output)
    assert config.PYPI_PRESETS["tsinghua"] in out
    assert "XX" not in out


# --------------------------------------------------------------------------
# _prompt_uv_binary
# --------------------------------------------------------------------------


def test_prompt_uv_binary_prompt_text_and_default(monkeypatch):
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["https://good/uv"])
    result = cli._prompt_uv_binary("https://default/uv")
    assert result == "https://good/uv"
    (msg,), kw = calls[0]
    assert msg == gettext("uv binary mirror URL")
    assert kw["default"] == "https://default/uv"
    assert kw["console"] is cli.console


def test_prompt_uv_binary_rejects_http_with_exact_message(monkeypatch, capsys):
    _capture_ask(monkeypatch, cli.Prompt, "ask", ["http://evil/uv", "https://good/uv"])
    result = cli._prompt_uv_binary("https://default/uv")
    assert result == "https://good/uv"
    err = _norm(capsys.readouterr().err)
    assert (
        "The uv binary is downloaded and executed, so its mirror URL must use https:// "
        "(got: http://evil/uv)." in err
    )
    assert "XX" not in err


# --------------------------------------------------------------------------
# _mirror_wizard
# --------------------------------------------------------------------------


def test_mirror_wizard_choice_prompt_exact_text(monkeypatch):
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["off"])
    cli._mirror_wizard()
    (msg,), kw = calls[0]
    assert msg == gettext("Mirror for faster installs in mainland China")
    assert kw["choices"] == [*config.PYPI_PRESETS, "custom", "off"]
    assert kw["console"] is cli.console


def test_mirror_wizard_custom_computed_defaults_when_disabled(monkeypatch):
    assert not config.load_mirror().enabled
    calls = _capture_ask(
        monkeypatch,
        cli.Prompt,
        "ask",
        ["custom", "https://x/pypi", "https://x/py", "https://x/npm"],
    )
    uv_calls: list[object] = []
    monkeypatch.setattr(
        cli, "_prompt_uv_binary", lambda default: uv_calls.append(default) or "https://x/uv"
    )
    cli._mirror_wizard()
    # calls[1] = PyPI index URL prompt, calls[2] = Python-install mirror prompt
    (msg1,), kw1 = calls[1]
    assert msg1 == gettext("PyPI index URL")
    assert kw1["default"] == config.PYPI_PRESETS["tsinghua"]
    assert kw1["console"] is cli.console
    (msg2,), kw2 = calls[2]
    assert msg2 == gettext("Python-install mirror URL")
    assert kw2["default"] == config.PYTHON_INSTALL_MIRROR
    assert kw2["console"] is cli.console
    assert uv_calls == [config.UV_BINARY_MIRROR]
    # calls[3] = npm registry prompt (the fourth custom URL, mirroring the TUI Preferences set)
    (msg3,), kw3 = calls[3]
    assert msg3 == gettext("npm registry URL")
    assert kw3["default"] == config.NPM_REGISTRY_MIRROR
    m = config.load_mirror()
    assert (m.pypi, m.python_install, m.uv_binary, m.npm) == (
        "https://x/pypi",
        "https://x/py",
        "https://x/uv",
        "https://x/npm",
    )


# --------------------------------------------------------------------------
# _language_wizard
# --------------------------------------------------------------------------


# --------------------------------------------------------------------------
# _set_language_arg / _set_mirror_arg
# --------------------------------------------------------------------------


def test_set_language_arg_unknown_message_exact():
    # NB: must go through `config --lang` — `skit lang` has its own separate error message.
    result = runner.invoke(cli.app, ["config", "lang", "xx-YY"])
    assert result.exit_code == 2
    locales = ", ".join(i18n.available_locales())
    out = _norm(result.output)
    assert f"Unknown language: xx-YY. Available: {locales}" in out
    assert "XX" not in out.replace("xx-YY", "")


def test_set_mirror_arg_unknown_message_exact():
    result = runner.invoke(cli.app, ["config", "mirror", "nope"])
    assert result.exit_code == 2
    choices = ", ".join([*config.PYPI_PRESETS, "off"])
    out = _norm(result.output)
    assert f"Unknown mirror: nope. Choose from: {choices}" in out
    assert "XX" not in out


# --------------------------------------------------------------------------
# _is_interactive / _parse_prompt_opts / _reconciled_specs
# --------------------------------------------------------------------------


def test_is_interactive_requires_both_stdin_and_stdout_tty(monkeypatch):
    monkeypatch.setattr("sys.stdin.isatty", lambda: False, raising=False)
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)
    assert cli._is_interactive() is False  # AND, not OR
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("sys.stdout.isatty", lambda: False, raising=False)
    assert cli._is_interactive() is False
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)
    assert cli._is_interactive() is True


def test_parse_kv_opts_splits_on_first_equals():
    # NAME=text where text itself contains '=': everything after the FIRST '=' is the value.
    pairs, bad = cli._parse_kv_opts(["A=b=c"], "--prompt")
    assert pairs == {"A": "b=c"}
    assert bad == []


def test_run_shim_error_message_exact(tmp_path, monkeypatch):
    # The ShimError message is built by formatting the msgid (which contains the literal
    # "[tool.skit]") and THEN escaping the whole result — regression test for the bug where
    # per-value escape() left the literal brackets to be swallowed as (no-op) rich markup.
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name="widget")
    argstate.save_last(entry.slug, values={"CITY": "Kaohsiung"})

    def boom(*a, **k):
        raise shim.ShimError("nope")

    monkeypatch.setattr(shim, "inject", boom)
    result = runner.invoke(cli.app, ["run", "widget", "--no-input"])
    assert result.exit_code == 125
    out = _norm(result.output)
    assert (
        "The script and its form definitions don't match anymore: nope. "
        "Run `skit params widget --resync` to fix it." in out
    )


def test_run_drift_warning_names_the_entry(tmp_path, monkeypatch):
    from skit import launcher

    monkeypatch.setattr(launcher, "run_entry", lambda *a, **k: 0)  # never actually launch anything
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name="widget")
    script_path = entry.dir / "script.py"
    drifted = script_path.read_text(encoding="utf-8").replace('CITY = "Taipei"', "CITY = 42")
    script_path.write_text(drifted, encoding="utf-8")
    result = runner.invoke(cli.app, ["run", "widget", "--no-input"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    # The drift header must name the entry being run, not something else.
    assert "The parameter definitions for widget have drifted from the script:" in out


# --------------------------------------------------------------------------
# _maybe_first_run_setup
# --------------------------------------------------------------------------


def test_first_run_blocked_message_and_confirm_prompt_exact(monkeypatch, capsys):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(config, "looks_blocked", lambda: True)
    confirm_calls = _capture_ask(monkeypatch, cli.Confirm, "ask", [False])
    cli._maybe_first_run_setup()
    out = _norm(capsys.readouterr().out)
    assert "Network to PyPI / GitHub looks slow or blocked." in out
    assert "XX" not in out
    (msg,), kw = confirm_calls[0]
    assert msg == gettext("Configure mirrors for faster installs (mainland China)?")
    assert kw["default"] is True
    assert kw["console"] is cli.console


# ==========================================================================
# Phase 2 additions: identity prompts, editor create flow, show/edit params.
# Exact-text + branch assertions to kill mutants in the reworked add/edit code.
# ==========================================================================


# --- _prompt_identity ------------------------------------------------------


def test_prompt_identity_exact_prompts_and_defaults(monkeypatch, tty, tmp_path):
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["chosen", "chosen desc"])
    name, desc = cli._prompt_identity(
        tmp_path / "image_stitch.py", '"""First line."""\nprint(1)\n', None, None, no_input=False
    )
    assert (name, desc) == ("chosen", "chosen desc")
    (m1, k1), (m2, k2) = calls
    assert m1[0] == gettext("Name in skit")
    assert k1["default"] == "image_stitch"
    assert k1["console"] is cli.console
    assert m2[0] == gettext("Description (optional)")
    assert k2["default"] == "First line."
    assert k2["console"] is cli.console


def test_prompt_identity_strips_name_to_none_and_strips_description(monkeypatch, tty, tmp_path):
    _capture_ask(monkeypatch, cli.Prompt, "ask", ["   ", "  spaced  "])
    name, desc = cli._prompt_identity(tmp_path / "w.py", "print(1)\n", None, None, no_input=False)
    assert name is None  # whitespace-only name collapses to None so the store derives the stem
    assert desc == "spaced"  # description is stripped


def test_prompt_identity_no_input_passes_through(tmp_path):
    assert cli._prompt_identity(tmp_path / "w.py", "x", "given", "d", no_input=True) == (
        "given",
        "d",
    )


def test_prompt_identity_non_tty_passes_through(monkeypatch, tmp_path):
    monkeypatch.setattr("sys.stdin.isatty", lambda: False, raising=False)

    def _boom(*_a, **_k):
        raise AssertionError("must not prompt on a non-tty")

    monkeypatch.setattr(cli.Prompt, "ask", _boom)
    assert cli._prompt_identity(tmp_path / "w.py", "x", None, None, no_input=False) == (None, None)


def test_prompt_identity_explicit_values_never_prompt(monkeypatch, tty, tmp_path):
    def _boom(*_a, **_k):
        raise AssertionError("must not prompt when both are supplied")

    monkeypatch.setattr(cli.Prompt, "ask", _boom)
    assert cli._prompt_identity(tmp_path / "w.py", "x", "n", "d", no_input=False) == ("n", "d")


# --- _print_add_summary ----------------------------------------------------


def test_print_add_summary_full_block_exact(tmp_path, capsys):
    entry = store.add_python(
        _py(tmp_path, '"""Doc."""\nprint(1)\n'), name="job", description="Doc."
    )
    # Two deps / managed / secrets so every ", " join separator is exercised.
    cli._print_add_summary(entry, ["Pillow", "rich"], ["CITY", "PORT"], ["API", "TOKEN"])
    out = _norm(capsys.readouterr().out)
    assert "Added: job (copy mode)" in out
    assert "Description: Doc." in out
    assert "Dependencies: Pillow, rich" in out
    assert "Managed parameters: CITY, PORT" in out
    assert "Run it: skit run job" in out
    assert "Secret parameter values are never saved to disk: API, TOKEN" in out
    assert "XX" not in out  # kills every whole-string-wrap and "XX, XX"-join mutant


def test_print_add_summary_omits_optional_lines(tmp_path, capsys):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="bare")
    cli._print_add_summary(entry, [], [], [])
    out = _norm(capsys.readouterr().out)
    assert "Added: bare (copy mode)" in out
    assert "Run it: skit run bare" in out
    assert "Description:" not in out
    assert "Dependencies:" not in out
    assert "Managed parameters:" not in out
    assert "never saved to disk" not in out


def test_print_add_summary_command_entry_has_no_mode_note(capsys):
    entry = store.add_command("echo hi", name="c")
    cli._print_add_summary(entry, [], [], [])
    out = _norm(capsys.readouterr().out)
    assert "Added: c" in out
    assert "mode)" not in out  # kind != python -> no "(... mode)" suffix
    assert "XX" not in out  # kills the else "XXXX" mode_note mutant


# --- _onboard_python -------------------------------------------------------


def test_onboard_python_copy_uses_explicit_deps_python_and_writes(monkeypatch, tmp_path):
    monkeypatch.setattr(
        cli,
        "_onboard_params",
        lambda text, name, no_input: [
            ParamDecl(name="CITY", binding="const", type="str", default="x")
        ],
    )
    p = _py(tmp_path, 'import rich\nCITY = "x"\nprint(rich, CITY)\n')
    # Explicit deps_opt / python_opt are threaded to _resolve_python_metadata (kills passing None
    # for either): "requests" overrides the detected "rich", and the python constraint is recorded.
    entry, deps, managed, secrets = cli._onboard_python(
        p,
        p.read_text(),
        name="job",
        description="D",
        ref=False,
        deps_opt=["requests"],
        python_opt=">=3.11",
        no_input=True,
    )
    assert entry.meta.name == "job"
    assert entry.meta.mode == "copy"
    assert entry.meta.description == "D"
    assert deps == ["requests"]  # explicit deps_opt won, not the detected "rich"
    assert managed == ["CITY"]
    assert secrets == []
    stored = (entry.dir / "script.py").read_text(encoding="utf-8")
    block = pep723.parse_block(stored)
    assert block is not None
    assert block["dependencies"] == ["requests"]  # kills dependencies=None at the add_python call
    assert block["requires-python"] == ">=3.11"
    assert [s.name for s in metawriter.read_params(stored)] == ["CITY"]


def _no_prompt(monkeypatch):
    def _boom(*_a, **_k):
        raise AssertionError("no prompt expected on the non-interactive path")

    monkeypatch.setattr(cli.Prompt, "ask", _boom)


def test_onboard_python_no_input_true_never_prompts(monkeypatch, tty, tmp_path):
    # no_input=True must thread through every sub-step (identity, deps, param onboarding) even on a
    # tty; a mutant flipping any of those no_input args to None would trigger a real prompt -> boom.
    _no_prompt(monkeypatch)
    p = _py(tmp_path, "import rich\nCITY = 'x'\nprint(rich, CITY)\n", "widget.py")
    entry, deps, managed, _s = cli._onboard_python(
        p,
        p.read_text(),
        name=None,
        description=None,
        ref=False,
        deps_opt=None,
        python_opt=None,
        no_input=True,
    )
    assert entry.meta.name == "widget"  # name derived from the stem, not prompted
    assert deps == ["rich"]  # suggestion accepted without prompting
    assert managed == []  # onboarding returns nothing under no_input


def test_onboard_python_default_no_input_is_false(monkeypatch, tty, tmp_path):
    # Called without no_input (as the create flow does), onboarding is interactive: the description
    # prompt fires. A default of True (the mutant) would skip it.
    called: list[int] = []
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: called.append(1) or "")
    monkeypatch.setattr(cli, "_onboard_params", lambda text, name, no_input: [])
    p = _py(tmp_path, "print(1)\n")  # no imports -> no deps prompt; name given -> no name prompt
    cli._onboard_python(p, p.read_text(), name="x", description=None)
    assert called  # the description prompt fired, proving no_input defaulted to False


def test_onboard_python_filename_hint_uses_entry_name(tty, tmp_path, capsys, monkeypatch):
    # The onboarding delegates to _onboard_params with entry.meta.name; a name->None mutant
    # would print "skit edit None" in the extract-a-constant hint.
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "none")
    p = _py(tmp_path, "save('out.jpg')\n")
    cli._onboard_python(
        p,
        p.read_text(),
        name="job",
        description="D",
        ref=False,
        deps_opt=None,
        python_opt=None,
        no_input=False,
    )
    out = _norm(capsys.readouterr().out)
    assert "skit edit job" in out


def test_onboard_python_interactive_prompts_name_from_path_stem(monkeypatch, tty, tmp_path):
    # name=None + tty: _prompt_identity is reached and prompts using p.stem as the default,
    # exercising the p argument (a p->None mutant would break the default).
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["chosen", "", "", ""])
    monkeypatch.setattr(cli, "_onboard_params", lambda text, name, no_input: [])
    p = _py(tmp_path, "import rich\nprint(rich)\n", "worker.py")
    entry, _deps, _m, _s = cli._onboard_python(
        p,
        p.read_text(),
        name=None,
        description=None,
        ref=False,
        deps_opt=None,
        python_opt=None,
        no_input=False,
    )
    assert entry.meta.name == "chosen"
    _args, kw = calls[0]  # the name prompt
    assert kw["default"] == "worker"  # p.stem


def test_onboard_python_reference_prints_skip_and_manages_nothing(tmp_path, capsys):
    p = _py(tmp_path, 'CITY = "x"\nprint(CITY)\n')
    entry, _deps, managed, secrets = cli._onboard_python(
        p,
        p.read_text(),
        name="r",
        description=None,
        ref=True,
        deps_opt=None,
        python_opt=None,
        no_input=True,
    )
    assert entry.meta.mode == "reference"
    assert managed == []
    assert secrets == []
    out = _norm(capsys.readouterr().out)
    assert "Reference mode never touches the original file, so parameter setup was skipped." in out
    assert "XX" not in out


def test_onboard_python_collects_secret_names(monkeypatch, tmp_path):
    monkeypatch.setattr(
        cli,
        "_onboard_params",
        lambda text, name, no_input: [
            ParamDecl(name="API_KEY", binding="const", type="str", default="x", secret=True)
        ],
    )
    p = _py(tmp_path, 'API_KEY = "x"\nprint(API_KEY)\n')
    _entry, _deps, managed, secrets = cli._onboard_python(
        p,
        p.read_text(),
        name="j",
        description=None,
        ref=False,
        deps_opt=None,
        python_opt=None,
        no_input=True,
    )
    assert managed == ["API_KEY"]
    assert secrets == ["API_KEY"]


# --- _create_python_in_editor (via `add -e`) -------------------------------


def test_create_in_editor_non_interactive_message(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: False)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "x"])
    assert result.exit_code == 2
    out = _norm(result.output)
    assert "Writing a new script in an editor needs an interactive terminal." in out
    assert "XX" not in out


def test_create_in_editor_blank_name_prompt_exact_and_message(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    calls = _capture_ask(monkeypatch, cli.Prompt, "ask", ["  "])  # whitespace -> no name

    def _boom(_p):
        raise AssertionError("editor must not open without a name")

    monkeypatch.setattr(cli.editor, "open_in_editor", _boom)
    result = runner.invoke(cli.app, ["add", "-e"])
    assert result.exit_code == 2
    out = _norm(result.output)
    assert "A name is required." in out
    assert "XX" not in out
    (msg,), kw = calls[0]
    assert msg == gettext("Name in skit")  # exact prompt text + case
    assert kw["console"] is cli.console


def test_create_in_editor_opening_and_added_message(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def write(p):
        assert p.suffix == ".py"  # temp file is a .py so the analyzer/PEP723 treat it as a script
        p.write_text("import rich\nprint(1)\n", encoding="utf-8")
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", write)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "fresh"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Opening your editor…" in out
    assert "Added: fresh" in out
    assert (
        "Dependencies: rich" in out
    )  # kills _print_add_summary(entry, None, ...) at the call site
    assert "XX" not in out


def test_create_in_editor_unchanged_starter_adds_nothing(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda p: 0)  # leaves the starter untouched
    result = runner.invoke(cli.app, ["add", "-e", "--name", "ghost"])
    out = _norm(result.output)
    assert "Nothing was written, so no script was added." in out
    assert "XX" not in out
    with pytest.raises(store.NotFoundError):
        store.resolve("ghost")


def test_create_in_editor_emptied_file_adds_nothing(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def empty(p):
        p.write_text("", encoding="utf-8")
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", empty)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "e"])
    assert "Nothing was written" in _norm(result.output)
    with pytest.raises(store.NotFoundError):
        store.resolve("e")


# --- _show_params ----------------------------------------------------------


def test_show_params_command_with_placeholders_exact(tmp_path, capsys):
    entry = store.add_command("echo {name}", name="c")
    argstate.save_last(entry.slug, values={"name": "Ada"})
    cli._show_params(entry, as_json=False)
    out = capsys.readouterr().out
    assert "Command template placeholders (the run form asks for them):" in out
    assert "  name = Ada" in out
    assert "XX" not in out


def test_show_params_command_placeholder_without_value_shows_dash(capsys):
    entry = store.add_command("echo {name}", name="c3")  # no recorded value
    cli._show_params(entry, as_json=False)
    out = _norm(capsys.readouterr().out)
    assert "name = —" in out  # kills last.get(p, "—") -> None / omitted / wrapped
    assert "None" not in out
    assert "XX" not in out


def test_show_params_command_no_placeholders_exact(capsys):
    entry = store.add_command("echo hi", name="c2")
    cli._show_params(entry, as_json=False)
    out = _norm(capsys.readouterr().out)
    assert "c2 has no managed parameters." in out
    assert "XX" not in out


def test_show_params_python_no_managed_exact(tmp_path, capsys):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="p")
    cli._show_params(entry, as_json=False)
    out = _norm(capsys.readouterr().out)
    assert (
        "p has no managed parameters. Use --manage to bring a detected candidate under"
        " management." in out
    )
    assert "XX" not in out


def test_show_params_python_argparse_does_not_advertise_manage(tmp_path, capsys):
    """A python entry that parses its own arguments (argparse) is reader-driven exactly like
    every other kind: its own parser IS the run form, so managed params REPLACE it rather than
    ride alongside — plan_for_entry prefers them. The read view must NOT advertise --manage
    (following it would silently shadow the argparse form); the round-6 "python manages
    constants alongside argparse" rationale was false. Plain "has no managed parameters." with
    no --manage advice, and --json reports unmanaged == [] (no candidate offered)."""
    import json

    text = "import argparse\nOUT = 'hi'\np = argparse.ArgumentParser()\np.add_argument('--n')\np.parse_args()\nprint(OUT)\n"
    entry = store.add_python(_py(tmp_path, text), name="gpy")
    cli._show_params(entry, as_json=False)
    out = _norm(capsys.readouterr().out)
    assert "gpy has no managed parameters." in out
    assert "--manage" not in out  # reader-driven: --manage would shadow the argparse form
    assert "OUT" not in out  # the constant is NOT offered as a candidate here
    # --json agrees: no unmanaged candidate is advertised for a reader-driven python entry.
    cli._show_params(entry, as_json=True)
    payload = json.loads(capsys.readouterr().out)
    assert payload["unmanaged"] == []


def test_show_params_python_table_all_cells_and_hint(tmp_path, capsys):
    specs = [
        ParamDecl(name="CITY", binding="const", type="str", default="Taipei"),
        ParamDecl(name="PORT", binding="input", type="str", default=None),
        ParamDecl(name="API", binding="const", type="str", default="x", secret=True),
    ]
    # Two unmanaged candidates (RETRIES, TIMEOUT) so the hint's ", " join separator is exercised.
    text = metawriter.write_params(
        "CITY = 'Taipei'\nRETRIES = 3\nTIMEOUT = 5\nprint(CITY, RETRIES, TIMEOUT)\n", specs
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    argstate.save_last(entry.slug, values={"CITY": "Tokyo"})  # only CITY has a last value
    cli._show_params(entry, as_json=False)
    out = _norm(capsys.readouterr().out)
    for header in ("Parameter", "Kind", "Type", "Default", "Secret", "Last value"):
        assert header in out
    assert "CITY" in out  # Parameter cell
    assert "const" in out  # const Kind cell
    assert "input" in out  # input Kind cell
    assert "str" in out  # Type cells (kills s.type -> None / dropped arg)
    assert "Taipei" in out
    assert "Tokyo" in out
    assert "yes" in out  # API's Secret cell
    assert "•••" in out  # API's masked default
    assert "—" in out  # PORT/API have no last value; PORT has no default
    assert "None" not in out  # kills last.get(.., None) / str(None) mutations
    assert "XX" not in out
    assert "Detected but not yet managed: RETRIES, TIMEOUT (use --manage to manage them)" in out


def test_show_params_missing_copy_reports_no_managed(tmp_path, capsys):
    # A python entry whose stored copy is gone: the `kind == python AND exists()` guard must skip
    # the read (an `or` mutant would try to read the missing file and crash).
    ent = store.add_python(_py(tmp_path, "CITY = 'x'\nprint(CITY)\n"), name="a")
    ent.script_path.unlink()
    cli._show_params(ent, as_json=False)
    out = _norm(capsys.readouterr().out)
    assert "a has no managed parameters. Use --manage" in out


def test_show_params_secret_masks_recorded_last_value(tmp_path, capsys):
    # Secret param with no default (default cell is "—"), so the only "•••" in the render is the
    # masked *last value* — isolating the mask so the last-value branch's mutants are observable.
    text = metawriter.write_params(
        "print('x')\n",
        [ParamDecl(name="API", binding="input", type="str", default=None, secret=True)],
    )
    entry = store.add_python(_py(tmp_path, text), name="s")
    # Recorded before it was secret (no secret_names), so a value exists in state; the now-secret
    # definition must display it masked rather than in the clear.
    argstate.save_last(entry.slug, values={"API": "sekret"})
    cli._show_params(entry, as_json=False)
    out = _norm(capsys.readouterr().out)
    assert "sekret" not in out  # a recorded value under a secret definition is shown masked
    assert "•••" in out  # the mask (from the last value; the default cell is "—")
    assert "XX" not in out


def test_show_params_reference_entry_suppresses_add_hint(tmp_path, capsys):
    # A reference entry's source can hold unmanaged candidates, but you can't --add to it, so the
    # hint must be suppressed (kills the `mode == "copy"` guard and its and/or mutants).
    text = metawriter.write_params(
        "CITY = 'x'\nRETRIES = 3\nprint(CITY, RETRIES)\n",
        [ParamDecl(name="CITY", binding="const", type="str", default="x")],
    )
    store.add_python(_py(tmp_path, text, "ref.py"), name="r", mode="reference")
    cli._show_params(store.resolve("r"), as_json=False)
    out = _norm(capsys.readouterr().out)
    assert "Detected but not yet managed" not in out  # reference mode: no --manage hint


# --- _edit_params (via `params --...`) -------------------------------------


def test_edit_params_updated_message_and_written_back(tmp_path):
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["params", "j", "--secret", "CITY"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Updated j. Managed parameters: CITY" in out
    assert "XX" not in out
    # The change must actually hit the file (kills write_params(text, None) at the write site).
    written = metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8"))
    assert written[0].name == "CITY"
    assert written[0].secret is True


def test_edit_params_resync_and_add_thread_flags(tmp_path):
    # GONE is defined but absent from the script (drift); RETRIES is a detected-but-unmanaged
    # candidate. --resync prunes GONE; --add manages RETRIES. Kills resync=None / add=None at the
    # edit_specs call and remaining="XX, XX".join (two names remain).
    text = metawriter.write_params(
        "CITY = 'x'\nRETRIES = 3\nprint(CITY, RETRIES)\n",
        [
            ParamDecl(name="CITY", binding="const", type="str"),
            ParamDecl(name="GONE", binding="const", type="str"),
        ],
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["params", "j", "--resync", "--manage", "RETRIES"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Updated j. Managed parameters: CITY, RETRIES" in out
    assert "XX" not in out
    names = {s.name for s in metawriter.read_params((entry.dir / "script.py").read_text())}
    assert names == {"CITY", "RETRIES"}  # GONE pruned by --resync, RETRIES added


def test_edit_params_remove_all_shows_dash(tmp_path):
    # Removing the sole managed param leaves nothing; the "remaining" list falls back to "—".
    text = metawriter.write_params(
        "CITY = 'x'\nprint(CITY)\n", [ParamDecl(name="CITY", binding="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["params", "j", "--unmanage", "CITY"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Updated j. Managed parameters: —" in out  # kills the `or "—"` -> "XX—XX" mutant
    assert "XX" not in out


def test_edit_params_prompt_written_back(tmp_path):
    # A --prompt value must reach the file (kills prompts=None / a dropped prompts arg at the
    # edit_specs call).
    text = metawriter.write_params(
        "CITY = 'x'\nprint(CITY)\n", [ParamDecl(name="CITY", binding="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["params", "j", "--prompt", "CITY=Where? "])
    assert result.exit_code == 0, result.output
    written = metawriter.read_params((entry.dir / "script.py").read_text())
    assert written[0].prompt == "Where? "


def test_edit_params_no_secret_unsets(tmp_path):
    # --no-secret must reach edit_specs (kills a dropped no_secret arg): a secret param becomes
    # non-secret.
    text = metawriter.write_params(
        "CITY = 'x'\nprint(CITY)\n",
        [ParamDecl(name="CITY", binding="const", type="str", secret=True)],
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["params", "j", "--no-secret", "CITY"])
    assert result.exit_code == 0, result.output
    written = metawriter.read_params((entry.dir / "script.py").read_text())
    assert written[0].secret is False


def test_edit_params_renders_reconcile_warning(tmp_path):
    # --secret on a name that isn't managed produces a reconcile warning that must be rendered
    # (kills escape(render_warning(None)) / err_console.print(None) in the warnings loop).
    text = metawriter.write_params(
        "CITY = 'x'\nprint(CITY)\n", [ParamDecl(name="CITY", binding="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["params", "j", "--secret", "GHOST"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "GHOST" in out  # the offending name
    assert "skipped" in out  # the warning surfaced
    assert "XX" not in out


def test_edit_params_non_python_message_exact():
    store.add_command("echo hi", name="c")
    result = runner.invoke(cli.app, ["params", "c", "--resync"])
    assert result.exit_code == 1
    out = _norm(result.output)
    assert "c has no managed parameters — its kind has no analyzer to read them from." in out
    assert "XX" not in out


def test_edit_params_reference_message_exact(tmp_path):
    store.add_python(_py(tmp_path, 'CITY = "x"\nprint(CITY)\n'), name="r", mode="reference")
    result = runner.invoke(cli.app, ["params", "r", "--resync"])
    assert result.exit_code == 1
    out = _norm(result.output)
    assert (
        "r is in reference mode, and skit never writes the original file. Edit the [tool.skit] "
        "block in the source directly." in out
    )
    assert "XX" not in out


def test_edit_params_reference_regression_literal_tool_skit_visible(tmp_path):
    # Regression test: the reference-mode message must render the literal "[tool.skit]" text,
    # not have it swallowed as (no-op) rich markup.
    store.add_python(_py(tmp_path, 'CITY = "x"\nprint(CITY)\n'), name="s", mode="reference")
    result = runner.invoke(cli.app, ["params", "s", "--resync"])
    assert result.exit_code == 1
    out = _norm(result.output)
    assert "[tool.skit]" in out


def test_edit_params_missing_copy_message_exact(tmp_path):
    ent = store.add_python(_py(tmp_path, 'CITY = "x"\nprint(CITY)\n'), name="a")
    ent.script_path.unlink()
    result = runner.invoke(cli.app, ["params", "a", "--resync"])
    assert result.exit_code == 1
    out = _norm(result.output)
    assert "a has no stored copy to edit." in out
    assert "XX" not in out


def test_edit_params_bad_prompt_warned_exact(tmp_path):
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["params", "j", "--prompt", "no-equals-sign"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Ignored a malformed value: --prompt: no-equals-sign (expected NAME=text)." in out
    assert "XX" not in out


# --- edit (open source / create) -------------------------------------------


def test_edit_saved_and_reconcile_hint_exact(monkeypatch, tmp_path):
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda p: 0)
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["edit", "a"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Saved a." in out
    assert "skit reconciles parameter drift at run time; review managed parameters with:" in out
    assert "skit params a" in out
    assert "XX" not in out


def test_edit_reference_editing_original_message(monkeypatch, tmp_path):
    src = _py(tmp_path, "print(1)\n", "orig.py")
    store.add_python(src, name="r", mode="reference")
    opened = {}
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda p: opened.setdefault("p", p) or 0)
    result = runner.invoke(cli.app, ["edit", "r"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Editing the original file (reference mode):" in out
    assert "XX" not in out
    assert opened["p"] == src.resolve()


def test_edit_non_python_message_exact():
    store.add_command("echo hi", name="c")
    result = runner.invoke(cli.app, ["edit", "c"])
    assert result.exit_code == 1
    out = _norm(result.output)
    # Kind-neutral now: shell/js ARE editable, so the refusal can't claim "Python only".
    assert "c has no editable source (programs and command templates run as-is)." in out
    assert "XX" not in out


def test_offer_create_non_interactive_message(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: False)
    result = runner.invoke(cli.app, ["edit", "ghost"])
    assert result.exit_code == 1
    out = _norm(result.output)
    assert "No script named ghost." in out
    assert "XX" not in out


def test_offer_create_confirm_prompt_exact_and_declined(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    calls = _capture_ask(monkeypatch, cli.Confirm, "ask", [False])

    def _boom(_p):
        raise AssertionError("declining must not open the editor")

    monkeypatch.setattr(cli.editor, "open_in_editor", _boom)
    result = runner.invoke(cli.app, ["edit", "newname"])
    assert result.exit_code == 0
    (msg,), kw = calls[0]
    assert msg == gettext('No script named "%(name)s". Create it now?') % {"name": "newname"}
    assert kw["default"] is True
    assert kw["console"] is cli.console
