"""Mutation-kill tests for src/skit/cli.py — chunk 1/6.

Targets surviving mutants in the stdin-add path (`_add_from_stdin`), the env-source
applier (`_apply_env_sources`), the interactive value collector (`_collect_values`),
completion (`_complete_script`) and the config read/write helpers (`_config_lang_value`,
`_config_set`, `_config_value`).

Convention mirrors the sibling suites: CliRunner for the non-interactive default path,
direct helper calls (with a stubbed tty / Prompt) for interactive branches, and behavioural
assertions (exit code, on-disk results, exact message text) rather than locale-agnostic
substrings — an XX-wrapper or a case-flip mutation only shows up when the whole rendered
string is pinned.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, config, flows, inlineform, store
from skit.params import ParamDecl

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
# _add_from_stdin — messages, forwarded kwargs, failure path, summary args
# --------------------------------------------------------------------------


def test_add_stdin_requires_name_exact_message():
    """No --name: the exact usage message is emitted (case-sensitive, no XX wrapper)."""
    result = runner.invoke(cli.app, ["add", "-"], input="print(1)\n")
    assert result.exit_code == 2
    # Capital 'R' kills the case-flip mutant; the whole rendered line has no XX markers.
    assert "Reading the script from stdin needs an explicit --name." in result.output
    assert "XX" not in result.output


def test_add_stdin_empty_input_exact_message():
    """Empty stdin: the exact 'nothing arrived' message (no XX wrapper)."""
    result = runner.invoke(cli.app, ["add", "-", "--name", "x"], input="")
    assert result.exit_code == 1
    assert "Nothing arrived on stdin, so there is nothing to add." in result.output
    assert "XX" not in result.output


def test_add_stdin_forwards_explicit_description():
    """An explicit --description is recorded verbatim (not dropped / nulled to a derived one)."""
    result = runner.invoke(
        cli.app,
        ["add", "-", "--name", "clip", "--description", "Custom desc"],
        input="print('hi')\n",
    )
    assert result.exit_code == 0, result.output
    # A script with no docstring would derive an empty description; the flag must win.
    assert store.resolve("clip").meta.description == "Custom desc"


def test_add_stdin_stays_noninteractive_even_on_a_tty(monkeypatch):
    """`add -` hardwires no_input=True: even with a real tty stdin it must never prompt."""

    class FakeStdin:
        def read(self) -> str:
            return "CITY = 'x'\nprint(CITY)\n"

        def isatty(self) -> bool:
            return True

    monkeypatch.setattr(sys, "stdin", FakeStdin())

    def boom(*_a: object, **_k: object) -> str:
        raise AssertionError("stdin add must never prompt (no_input must stay True)")

    monkeypatch.setattr(cli.Prompt, "ask", boom)
    cli._add_from_stdin("clip", None)  # description=None would trigger a prompt if no_input flips
    assert store.resolve("clip").meta.kind == "python"


def test_add_stdin_store_error_shows_real_message():
    """A duplicate name surfaces the store's own error text (not None / '1' / a crash)."""
    runner.invoke(cli.app, ["add", "-", "--name", "dup"], input="print(1)\n")
    result = runner.invoke(cli.app, ["add", "-", "--name", "dup"], input="print(2)\n")
    assert result.exit_code == 1
    assert "The name dup is already taken" in result.output


def test_add_stdin_summary_lists_dependencies():
    """An explicit --dep shows up in the post-add summary (deps must reach _print_add_summary)."""
    result = runner.invoke(
        cli.app,
        ["add", "-", "--name", "clip", "--dep", "requests"],
        input="print(1)\n",
    )
    assert result.exit_code == 0, result.output
    assert "Dependencies" in result.output
    assert "requests" in result.output


def test_add_stdin_forwards_summary_args_verbatim(monkeypatch):
    """The summary call forwards exactly what _onboard_python returned — all four slots,
    unfiltered. managed/secrets happen to be empty on this path today (no_input skips
    parameter onboarding), but that is _onboard_python's business: the call site forwards,
    it does not assume, so a mutant nulling or dropping any slot is observable here."""

    class FakeStdin:
        def read(self) -> str:
            return "print('hi')\n"

    monkeypatch.setattr(sys, "stdin", FakeStdin())
    sentinel = object()
    monkeypatch.setattr(
        cli, "_onboard_python", lambda *_a, **_k: (sentinel, ["req"], ["CITY"], ["TOKEN"])
    )
    calls: list[tuple[object, ...]] = []
    monkeypatch.setattr(cli, "_print_add_summary", lambda *a: calls.append(a))
    cli._add_from_stdin("clip", None)
    assert calls == [(sentinel, ["req"], ["CITY"], ["TOKEN"])]


# --------------------------------------------------------------------------
# _apply_env_sources — warning text + continue-not-break control flow
# --------------------------------------------------------------------------


def test_apply_env_sources_unmanaged_warning_exact():
    specs = [ParamDecl(name="CITY", type="str")]
    warnings = cli._apply_env_sources(specs, {"GHOST": "OPENAI"})
    assert warnings == ["GHOST isn't a managed parameter; --env-source skipped."]


def test_apply_env_sources_non_secret_warning_exact():
    specs = [ParamDecl(name="CITY", type="str", secret=False)]
    warnings = cli._apply_env_sources(specs, {"CITY": "OPENAI"})
    assert warnings == [
        "CITY isn't secret; --env-source only applies to secret parameters "
        "(mark it with --secret first)."
    ]


def test_apply_env_sources_skips_but_keeps_processing_later_specs():
    """Both skip branches use `continue`, not `break`: an unmanaged name and a non-secret name
    each warn without stopping the loop, so a later secret spec still gets its env_source set."""
    specs = [
        ParamDecl(name="CITY", type="str", secret=False),
        ParamDecl(name="API", type="str", secret=True),
    ]
    warnings = cli._apply_env_sources(specs, {"GHOST": "X", "CITY": "Y", "API": "OPENAI_KEY"})
    # GHOST is unmanaged (spec is None -> continue); if that were a break, CITY never warns.
    assert any("CITY isn't secret" in w for w in warnings)
    # CITY is non-secret (-> continue); if that were a break, API's env_source stays unset.
    assert specs[1].env_source == "OPENAI_KEY"


# --------------------------------------------------------------------------
# _collect_values — TERM=dumb -> plain, renderer arg forwarding
# --------------------------------------------------------------------------


def test_collect_values_term_dumb_forces_plain_promptform(monkeypatch):
    """TERM=dumb (exact, lowercase key+value) forces the plain line-prompt path even when the
    configured form is 'tui' — the inline renderer must not be reached."""
    monkeypatch.setenv("TERM", "dumb")
    config.save_form("tui")
    ent = store.add_command("echo {msg}", name="e")
    plan = flows.plan_for_entry(ent)
    inline_hit: dict[str, int] = {}
    monkeypatch.setattr(inlineform, "collect", lambda *a, **k: inline_hit.setdefault("x", 1) or {})
    monkeypatch.setattr(
        cli.promptform, "collect", lambda plan, prefill, console: {"msg": "plainval"}
    )
    values, runner, picked = cli._collect_values(ent, plan, {}, plain=False)
    assert values == {"msg": "plainval"}
    assert runner is None  # the line fallback never answers the picker
    assert picked is False
    assert "x" not in inline_hit  # the tui/inline renderer was never entered


def test_collect_values_inline_receives_real_entry_plan_prefill(monkeypatch):
    """The tui path passes the actual entry, plan and prefill through to inlineform.collect."""
    monkeypatch.setenv("TERM", "xterm-256color")
    config.save_form("tui")
    ent = store.add_command("echo {msg}", name="e")
    plan = flows.plan_for_entry(ent)
    seen: dict[str, object] = {}

    def fake_collect(entry, plan_, prefill, runners=None, runner_default="SENTINEL"):
        seen["entry"] = entry
        seen["plan"] = plan_
        seen["prefill"] = prefill
        seen["runner_default"] = runner_default
        return {"msg": "v"}, None, False

    monkeypatch.setattr(inlineform, "collect", fake_collect)
    prefill = {"msg": "pre"}
    values, _runner, _picked = cli._collect_values(ent, plan, prefill, plain=False)
    assert values == {"msg": "v"}
    assert seen["entry"] is ent
    assert seen["plan"] is plan
    assert seen["prefill"] is prefill
    # _collect_values' own runner_default default is the empty string; called without it, the
    # inline renderer must receive "" (kills the `runner_default: str = "XXXX"` default mutant).
    assert seen["runner_default"] == ""


def test_collect_values_plain_forwards_module_console(monkeypatch):
    """The plain fallback hands promptform.collect skit's own console, not None/a default."""
    ent = store.add_command("echo {msg}", name="e")
    plan = flows.plan_for_entry(ent)
    seen: dict[str, object] = {}

    def fake_collect(plan_, prefill, *, console):
        seen["console"] = console
        return {"msg": "v"}

    monkeypatch.setattr(cli.promptform, "collect", fake_collect)
    cli._collect_values(ent, plan, {}, plain=True)
    assert seen["console"] is cli.console


# --------------------------------------------------------------------------
# _complete_script — union (not intersection) of names and slugs
# --------------------------------------------------------------------------


def test_complete_script_unions_names_and_slugs(tmp_path):
    """Completion offers both the display name and the derived slug; when they differ, an
    intersection would offer neither, so the union is observable."""
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="Alpha Script")
    assert ent.meta.name != ent.slug  # "Alpha Script" vs "alpha-script"
    out = cli._complete_script("")
    assert ent.meta.name in out
    assert ent.slug in out


# --------------------------------------------------------------------------
# _config_lang_value — override wins, else the auto label
# --------------------------------------------------------------------------


def test_config_lang_value_returns_override(monkeypatch):
    monkeypatch.setattr(cli.config, "load_config", lambda: {"language": "zh-CN"})
    assert cli._config_lang_value() == "zh-CN"


def test_config_lang_value_auto_when_unset(monkeypatch):
    monkeypatch.setattr(cli.config, "load_config", lambda: {})
    # No override -> the "auto (locale)" fallback, never an empty string.
    assert cli._config_lang_value().startswith("auto")


def test_config_lang_value_empty_override_reads_auto():
    # An explicit `language = ""` in config.toml means "no override": the value face must
    # show the same auto label as a missing key. The guard is isinstance AND truthy — an
    # or-mutant would leak the raw "" straight into the listing.
    config.save_config({"language": ""})
    empty_value = cli._config_lang_value()
    config.save_config({})
    assert empty_value == cli._config_lang_value()  # empty ≡ missing -> the auto label
    assert empty_value.startswith("auto")


# --------------------------------------------------------------------------
# _mirror_master_value — every axis alone flips the master to "on"
# --------------------------------------------------------------------------


def test_mirror_master_value_on_with_only_uv_binary():
    # "on" = enabled AND any single axis URL. A config holding only the uv-binary vector
    # must read "on": each `or` in the any-axis chain is load-bearing, and this axis sits
    # in the associativity spot where an and-mutant collapses the chain to "off".
    config.save_mirror(config.MirrorConfig(enabled=True, uv_binary="https://m.example/uv"))
    assert cli._mirror_master_value() == "on"


# --------------------------------------------------------------------------
# _config_set — each rejection prints its own exact message (exit 2)
# --------------------------------------------------------------------------


def test_config_set_form_unknown_message():
    result = runner.invoke(cli.app, ["config", "form", "fancy"])
    assert result.exit_code == 2
    assert "Unknown form style" in result.output
    assert "XX" not in result.output


def test_config_set_js_runner_unknown_message():
    result = runner.invoke(cli.app, ["config", "js.runner", "carrier-pigeon"])
    assert result.exit_code == 2
    assert "Unknown JS runner" in result.output
    assert "XX" not in result.output  # msgid wrapper AND the ', ' join separator


def test_config_set_after_run_unknown_message():
    result = runner.invoke(cli.app, ["config", "after_run", "loop"])
    assert result.exit_code == 2
    assert "Unknown after-run behavior" in result.output
    assert "XX" not in result.output


def test_config_set_bash_path_missing_message(tmp_path):
    ghost = tmp_path / "nope"  # never created
    result = runner.invoke(cli.app, ["config", "shell.bash_path", str(ghost)])
    assert result.exit_code == 2
    assert "No such file" in result.output
    assert "XX" not in result.output


# --------------------------------------------------------------------------
# _config_value — the human default labels for unset keys
# --------------------------------------------------------------------------


def test_config_value_editor_default_label():
    assert cli._config_value("editor") == "default ($VISUAL / $EDITOR)"


def test_config_value_bash_path_default_label():
    assert cli._config_value("shell.bash_path") == "auto (bash on PATH)"
