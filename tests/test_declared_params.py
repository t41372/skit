"""Declared parameter schema ([[parameters]]) + env delivery (P1, docs/design/multilang.md).

The declared schema is what gives exe/command entries a real form (types, defaults,
optional, secret OVERRIDE — fixing the {token_file} auto-secret-no-override defect),
and env delivery is the zero-rewrite value channel every kind can use.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, flows, launcher, store
from skit.models import Entry, ScriptMeta
from skit.params import (
    ParamDecl,
    declared_for_template,
    declared_from_meta,
    synthesized_placeholder,
)

runner = CliRunner()


def _json(result):
    """The --json contract: stdout is EXACTLY one JSON document (SKILL.md's stable
    contract). Parse the WHOLE of stdout — never slice from the first `{`, which would
    mask a human line leaking onto stdout. Warnings ride stderr, which CliRunner keeps
    separate, so this stays pure."""
    return json.loads(result.output)


@pytest.fixture
def run_entry_spy(monkeypatch):
    """Capture the delivery-ready material handed to launcher.run_entry (nothing runs)."""
    calls: dict[str, object] = {}

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
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        calls["env_overlay"] = dict(env_overlay or {})
        return 0

    monkeypatch.setattr(launcher, "run_entry", fake)
    return calls


def _exe(tmp_path: Path, name: str = "prog") -> store.Entry:
    prog = tmp_path / ("t.exe" if sys.platform == "win32" else "t")
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    return store.add_exe(prog, name=name)


# ---- declared_for_template ---------------------------------------------------------------------


def test_undeclared_placeholders_synthesize_the_historical_field():
    decls = declared_for_template(None, ["input", "api_key"])
    assert [d.name for d in decls] == ["input", "api_key"]
    assert all(d.delivery == "placeholder" for d in decls)
    assert all(d.required for d in decls)
    assert decls[0].secret is False
    assert decls[1].secret is True  # KEY heuristic — unchanged historical behavior


def test_declared_row_overrides_placeholder_schema_including_secret():
    # THE defect fix: {token_file} matched the TOKEN heuristic and could never be
    # un-secreted. A declared row now owns the schema outright.
    rows = [
        ParamDecl(
            name="token_file",
            delivery="placeholder",
            type="str",
            required=False,
            default="creds.json",
            secret=False,
        ).to_meta_dict()
    ]
    decls = declared_for_template(rows, ["token_file", "host"])
    assert decls[0].secret is False
    assert decls[0].required is False
    assert decls[0].default == "creds.json"
    # the undeclared one still synthesizes
    assert decls[1].name == "host"
    assert decls[1].required is True


def test_declared_env_param_rides_along_after_placeholders():
    rows = [ParamDecl(name="RETRIES", delivery="env", type="int", default=3).to_meta_dict()]
    decls = declared_for_template(rows, ["file"])
    assert [d.name for d in decls] == ["file", "RETRIES"]
    assert decls[1].delivery == "env"


def test_declared_flag_row_is_dropped_for_templates():
    # argv is not a template's interface (takes_argv=False): a flag row can only be a
    # hand-edit mistake, and dropping beats assembling arguments the template never reads.
    rows = [ParamDecl(name="width", delivery="flag", flag="--width").to_meta_dict()]
    decls = declared_for_template(rows, ["file"])
    assert [d.name for d in decls] == ["file"]


def test_declared_row_with_wrong_delivery_for_its_placeholder_is_replaced_by_synth():
    # A row named like a placeholder but declared env can't fill the {slot}; the
    # placeholder still needs a value, so the synthesized field steps back in.
    rows = [ParamDecl(name="file", delivery="env").to_meta_dict()]
    decls = declared_for_template(rows, ["file"])
    assert len(decls) == 1
    assert decls[0].delivery == "placeholder"
    assert decls[0].required is True


def test_declared_from_meta_drops_nameless_rows():
    rows = [{"delivery": "flag"}, ParamDecl(name="ok").to_meta_dict()]
    assert [d.name for d in declared_from_meta(rows)] == ["ok"]


def test_synthesized_placeholder_shape():
    d = synthesized_placeholder("api_key")
    assert (d.delivery, d.required, d.secret) == ("placeholder", True, True)


# ---- plan_for_entry ------------------------------------------------------------------------------


def test_command_plan_honors_declared_schema(tmp_path: Path):
    entry = store.add_command("convert {size} {api_key}", name="conv")
    store.write_parameters(
        entry.slug,
        [
            ParamDecl(
                name="size",
                delivery="placeholder",
                type="choice",
                choices=("s", "m"),
                default="m",
                required=False,
            ),
        ],
    )
    plan = flows.plan_for_entry(store.resolve(entry.slug))
    assert plan.source == "command"
    size, key = plan.fields
    assert (size.kind, size.choices, size.default, size.required) == (
        "choice",
        ["s", "m"],
        "m",
        False,
    )
    assert key.required is True  # undeclared: synthesized, unchanged behavior
    assert key.secret is True


def test_exe_with_declared_params_gets_a_form(tmp_path: Path):
    prog = tmp_path / ("t.exe" if sys.platform == "win32" else "t")
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    entry = store.add_exe(prog, name="prog")
    store.write_parameters(
        entry.slug,
        [
            ParamDecl(name="width", delivery="flag", flag="--width", type="int", default=800),
            ParamDecl(name="DEBUG", delivery="env", type="bool"),
            ParamDecl(name="slot", delivery="placeholder"),  # meaningless on a binary: dropped
        ],
    )
    plan = flows.plan_for_entry(store.resolve(entry.slug))
    assert plan.source == "declared"
    assert [(f.key, f.source) for f in plan.fields] == [("width", "flag"), ("DEBUG", "env")]


def test_exe_without_declared_params_stays_none_plan(tmp_path: Path):
    prog = tmp_path / ("u.exe" if sys.platform == "win32" else "u")
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    entry = store.add_exe(prog, name="prog2")
    assert flows.plan_for_entry(entry).source == "none"


# ---- assemble: env routing -----------------------------------------------------------------------


def _env_plan() -> flows.FormPlan:
    fields = [
        flows.FormField.from_decl(ParamDecl(name="WIDTH", delivery="env", type="int")),
        flows.FormField.from_decl(
            ParamDecl(name="token", delivery="env", secret=True, env_target="API_TOKEN")
        ),
        flows.FormField.from_decl(ParamDecl(name="UNSET", delivery="env")),
    ]
    return flows.FormPlan(source="declared", fields=fields)


def test_assemble_env_values_masked_and_empty_absent(tmp_path: Path):
    asm = flows.assemble(
        _env_plan(), {"WIDTH": "800", "token": "hunter2"}, [], cwd=tmp_path, env={}
    )
    assert asm.env_values == {"WIDTH": "800", "API_TOKEN": "hunter2"}  # env_target honored
    assert asm.masked_env == {"WIDTH": "800", "API_TOKEN": "•••"}  # secret masked for display
    assert "UNSET" not in asm.env_values  # empty stays ABSENT so script defaults fire
    assert asm.args == []


def test_assemble_mixed_flag_and_env_fields(tmp_path: Path):
    fields = [
        flows.FormField.from_decl(
            ParamDecl(name="width", delivery="flag", flag="--width", type="int")
        ),
        flows.FormField.from_decl(ParamDecl(name="DEBUG", delivery="env")),
    ]
    plan = flows.FormPlan(source="declared", fields=fields)
    asm = flows.assemble(plan, {"width": "800", "DEBUG": "1"}, ["-v"], cwd=tmp_path, env={})
    assert asm.args == ["--width", "800", "-v"]  # env field never enters argv
    assert asm.env_values == {"DEBUG": "1"}


def test_assemble_command_with_env_rider(tmp_path: Path):
    entry = store.add_command("echo {msg}", name="e-env")
    store.write_parameters(entry.slug, [ParamDecl(name="RETRIES", delivery="env")])
    plan = flows.plan_for_entry(store.resolve(entry.slug))
    asm = flows.assemble(plan, {"msg": "hi", "RETRIES": "3"}, [], cwd=tmp_path, env={})
    assert asm.command_values == {"msg": "hi"}  # env rider is NOT a template value
    assert asm.env_values == {"RETRIES": "3"}


# ---- run_entry: overlay order ---------------------------------------------------------------------


def test_run_entry_env_overlay_wins_last(tmp_path: Path, monkeypatch):
    captured: dict[str, str] = {}

    def fake_run(cmd, **kwargs):
        captured.update(kwargs["env"])
        return subprocess.CompletedProcess(cmd, 0)

    monkeypatch.setattr("skit.launcher.subprocess.run", fake_run)
    monkeypatch.setenv("WIDTH", "from-ambient")
    entry = store.add_command("echo hi", name="ov")
    code = launcher.run_entry(entry, invoke_cwd=tmp_path, env_overlay={"WIDTH": "800"})
    assert code == 0
    assert captured["WIDTH"] == "800"  # the explicit parameter beats the ambient value


# ---- transparency ----------------------------------------------------------------------------------


def test_transparency_shows_masked_env_prefix(tmp_path: Path):
    entry = store.add_command("echo hi", name="tr")
    asm = flows.Assembly(
        env_values={"API_TOKEN": "hunter2", "GREETING": "hello world"},
        masked_env={"API_TOKEN": "•••", "GREETING": "hello world"},
    )
    lines = flows.transparency_lines(entry, asm, None)
    arrow = lines[-1]
    assert "API_TOKEN=" in arrow
    assert "hunter2" not in arrow  # the secret value never reaches the scrollback
    assert "•••" in arrow
    # a spaced value is quoted so the shown line is genuinely copy-pasteable
    assert "GREETING='hello world'" in arrow or 'GREETING="hello world"' in arrow


# ---- store round-trip -------------------------------------------------------------------------------


def test_write_read_parameters_roundtrip_and_legacy_params_untouched(tmp_path: Path):
    entry = store.add_command("run {a} {b}", name="rt")
    assert entry.meta.params == ["a", "b"]
    decls = [ParamDecl(name="a", delivery="placeholder", type="int", required=False)]
    store.write_parameters(entry.slug, decls)
    back = store.read_parameters(entry.slug)
    assert back == decls
    # the placeholder-name cache stays the template's truth (downgrade safety),
    # independent of which subset carries declared schema
    assert store.resolve(entry.slug).meta.params == ["a", "b"]
    # clearing works
    store.write_parameters(entry.slug, [])
    assert store.read_parameters(entry.slug) == []
    assert store.resolve(entry.slug).meta.parameters is None


# ---- execute wiring ---------------------------------------------------------------------------------


def test_execute_passes_env_values_to_run_entry(tmp_path: Path, monkeypatch):
    seen: dict[str, object] = {}

    def fake_run_entry(
        entry,
        extra,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
        runner=None,
    ):
        seen["env_overlay"] = env_overlay
        return 0

    monkeypatch.setattr("skit.flows.launcher.run_entry", fake_run_entry)
    entry = store.add_command("echo {m}", name="exec-env")
    store.write_parameters(entry.slug, [ParamDecl(name="N", delivery="env")])
    plan = flows.plan_for_entry(store.resolve(entry.slug))
    asm = flows.assemble(plan, {"m": "x", "N": "5"}, [], cwd=tmp_path, env={})
    outcome = flows.execute(entry, plan, asm, emit=lambda _line: None, invoke_cwd=tmp_path)
    assert outcome.code == 0
    assert seen["env_overlay"] == {"N": "5"}


# ---- meta model -------------------------------------------------------------------------------------


def test_meta_parameters_roundtrip_and_non_dict_rows_dropped():
    meta = ScriptMeta(
        name="x",
        kind="command",
        template="echo {a}",
        parameters=[{"name": "a", "delivery": "placeholder"}],
    )
    d = meta.to_toml_dict()
    assert d["parameters"] == [{"name": "a", "delivery": "placeholder"}]
    back = ScriptMeta.from_toml_dict({**d, "parameters": [{"name": "a"}, "garbage", 5]})
    assert back.parameters == [{"name": "a"}]


def test_declared_plan_secret_placeholder_masks_in_command_values(tmp_path: Path):
    # End-to-end C3 for declared schema: a secret placeholder's value masks in
    # masked_command_values while the real value still runs.
    entry = store.add_command("login {password}", name="c3")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {"password": "s3cret"}, [], cwd=tmp_path, env={})
    assert asm.command_values == {"password": "s3cret"}
    assert asm.masked_command_values == {"password": "•••"}


def test_unknown_kind_entry_still_gets_none_plan(tmp_path: Path):
    meta = ScriptMeta(name="m", kind="martian")
    entry = Entry(slug="m", meta=meta, dir=tmp_path / "m")
    assert flows.plan_for_entry(entry).source == "none"


def test_exe_with_only_placeholder_rows_falls_through_to_none(tmp_path: Path):
    # Every declared row filters out (placeholder means nothing for a binary):
    # the plan must fall through to "none", not produce an empty declared form.
    prog = tmp_path / ("v.exe" if sys.platform == "win32" else "v")
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    entry = store.add_exe(prog, name="prog3")
    store.write_parameters(entry.slug, [ParamDecl(name="slot", delivery="placeholder")])
    assert flows.plan_for_entry(store.resolve(entry.slug)).source == "none"


# ==================================================================================================
# CLI: skit params --add/--rm/--type/... on exe & command (add -> show -> json -> run --set)
# ==================================================================================================


def test_cli_add_flag_param_on_exe_then_run_set(tmp_path: Path, run_entry_spy):
    entry = _exe(tmp_path)
    result = runner.invoke(
        cli.app,
        [
            "params",
            "prog",
            "--add",
            "width",
            "--type",
            "width=int",
            "--deliver",
            "width=flag",
            "--flag",
            "width=--width",
            "--default",
            "width=800",
        ],
    )
    assert result.exit_code == 0, result.output
    assert "Declared parameters: width" in result.output
    decls = store.read_parameters(entry.slug)
    assert (decls[0].name, decls[0].delivery, decls[0].type, decls[0].flag) == (
        "width",
        "flag",
        "int",
        "--width",
    )
    assert decls[0].default == 800  # coerced + stored typed
    # run --set assembles the real flag
    result = runner.invoke(cli.app, ["run", "prog", "--set", "width=1024", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["--width", "1024"]


def test_cli_exe_show_table_and_json(tmp_path: Path):
    entry = _exe(tmp_path)
    store.write_parameters(
        entry.slug,
        [ParamDecl(name="width", delivery="flag", flag="--width", type="int", default=800)],
    )
    human = runner.invoke(cli.app, ["params", "prog"])
    assert human.exit_code == 0
    assert "width" in human.output
    assert "flag" in human.output  # the Delivery column value
    js = runner.invoke(cli.app, ["params", "prog", "--json"])
    assert js.exit_code == 0
    payload = json.loads(js.output)
    assert payload["declared"][0]["name"] == "width"
    assert payload["declared"][0]["delivery"] == "flag"


def test_cli_exe_show_without_declared_is_plain_message(tmp_path: Path):
    _exe(tmp_path)
    result = runner.invoke(cli.app, ["params", "prog"])
    assert result.exit_code == 0
    assert "has no managed parameters" in result.output


def test_cli_declared_edit_with_json_emits_the_final_read_view(tmp_path: Path):
    """A declared edit with --json emits the final read-view JSON as the WHOLE of stdout,
    instead of silently dropping the flag — an explicit --json never no-ops (finding 14),
    and under the purity rule the human summary rides stderr, not stdout."""
    _exe(tmp_path)
    result = runner.invoke(
        cli.app,
        [
            "params",
            "prog",
            "--add",
            "width",
            "--deliver",
            "width=flag",
            "--flag",
            "width=--width",
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    # Under --json stdout is EXACTLY the read-view JSON — the human "Updated prog" summary
    # is silenced (it would break the one-document contract), so parse the whole output.
    payload = _json(result)
    assert payload["declared"][0]["name"] == "width"  # the just-added row is in the JSON
    assert payload["declared"][0]["delivery"] == "flag"


def test_cli_env_source_on_non_secret_declared_param_warns(tmp_path: Path):
    """--env-source on a DECLARED non-secret param warns (it only applies to secrets) instead
    of vanishing — the in-file lane's rule, now on the declared lane too. The warning rides
    stderr, so under --json stdout stays exactly one read-view document."""
    entry = _exe(tmp_path)
    store.write_parameters(entry.slug, [ParamDecl(name="WIDTH", delivery="env")])
    result = runner.invoke(cli.app, ["params", "prog", "--env-source", "WIDTH=COLS"])
    assert result.exit_code == 0, result.output
    assert "WIDTH isn't secret" in result.stderr  # the no-op flag is surfaced, not dropped
    # --json: stdout is exactly one JSON document; the warning stays on stderr, so the
    # STDOUT stream (not the mixed .output) parses whole.
    jr = runner.invoke(cli.app, ["params", "prog", "--env-source", "WIDTH=COLS", "--json"])
    assert jr.exit_code == 0, jr.output
    payload = json.loads(jr.stdout)  # stdout alone is pure JSON
    assert any(p["name"] == "WIDTH" for p in payload["declared"])
    assert "WIDTH isn't secret" in jr.stderr  # the warning rode stderr, not stdout


def test_cli_python_manage_with_json_emits_the_final_read_view(tmp_path: Path):
    """The twin on the analyzer branch: `skit params <py> --manage CITY --json` emits the
    final read-view JSON after managing CITY (finding 14, the analyzer-edit branch)."""
    src = tmp_path / "job.py"
    src.write_text('CITY = "Taipei"\nprint(CITY)\n', encoding="utf-8")
    store.add_python(src, name="job")
    result = runner.invoke(cli.app, ["params", "job", "--manage", "CITY", "--json"])
    assert result.exit_code == 0, result.output
    # --json purity: the human "Updated job" summary is silenced; stdout is the read view.
    payload = _json(result)
    assert [p["name"] for p in payload["params"]] == ["CITY"]  # CITY is now managed in the JSON


def test_cli_add_choice_placeholder_on_command_then_run(tmp_path: Path, run_entry_spy):
    runner.invoke(cli.app, ["add", "--cmd", "convert {size}", "--name", "conv", "--no-input"])
    result = runner.invoke(
        cli.app,
        [
            "params",
            "conv",
            "--add",
            "size",
            "--type",
            "size=choice",
            "--choices",
            "size=s,m,l",
            "--default",
            "size=m",
            "--optional",
            "size",
        ],
    )
    assert result.exit_code == 0, result.output
    decl = store.read_parameters(store.resolve("conv").slug)[0]
    assert decl.delivery == "placeholder"  # add on a placeholder name
    assert decl.type == "choice"
    assert decl.choices == ("s", "m", "l")
    assert decl.default == "m"
    assert decl.required is False
    # run --no-input: the declared default fills the placeholder without prompting
    result = runner.invoke(cli.app, ["run", "conv", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["values"] == {"size": "m"}


def test_cli_command_show_enriched_and_env_rider(tmp_path: Path):
    runner.invoke(cli.app, ["add", "--cmd", "echo {msg}", "--name", "c", "--no-input"])
    runner.invoke(
        cli.app,
        [
            "params",
            "c",
            "--add",
            "msg",
            "--type",
            "msg=str",
            "--default",
            "msg=hi",
            "--optional",
            "msg",
        ],
    )
    runner.invoke(cli.app, ["params", "c", "--add", "RETRIES", "--deliver", "RETRIES=env"])
    human = runner.invoke(cli.app, ["params", "c"])
    assert human.exit_code == 0, human.output
    assert "msg" in human.output
    assert "optional" in human.output  # the schema suffix marker
    assert "RETRIES" in human.output  # the declared env rider is listed
    js = runner.invoke(cli.app, ["params", "c", "--json"])
    names = {d["name"] for d in json.loads(js.output)["declared"]}
    assert names == {"msg", "RETRIES"}


def test_cli_command_env_rider_only_no_placeholders(tmp_path: Path):
    # a template with no placeholders but a declared env rider: the show view still lists it
    runner.invoke(cli.app, ["add", "--cmd", "echo hi", "--name", "noph", "--no-input"])
    runner.invoke(cli.app, ["params", "noph", "--add", "RETRIES", "--deliver", "RETRIES=env"])
    result = runner.invoke(cli.app, ["params", "noph"])
    assert result.exit_code == 0
    assert "RETRIES" in result.output


def test_cli_python_declared_op_is_refused(tmp_path: Path):
    body = 'CITY = "x"\nprint(CITY)\n'
    (tmp_path / "job.py").write_text(body, encoding="utf-8")
    store.add_python(tmp_path / "job.py", name="py")
    result = runner.invoke(cli.app, ["params", "py", "--add", "WIDTH"])
    assert result.exit_code == 1
    assert "manages its parameters from the script itself" in result.output


def test_cli_declared_malformed_value_warns(tmp_path: Path):
    _exe(tmp_path)
    result = runner.invoke(cli.app, ["params", "prog", "--type", "NOEQUALS"])
    assert result.exit_code == 0, result.output
    assert "Ignored a malformed value" in result.output


def test_cli_declared_warning_codes_render(tmp_path: Path):
    # Every closed warning code renders a distinct localized line (via _render_declared_warning).
    for code in (
        "not-declared:x",
        "already-declared:x",
        "bad-delivery:x",
        "not-a-placeholder:x",
        "bad-type:x",
        "bad-default:x",
        "choice-without-choices:x",
    ):
        line = cli._render_declared_warning(code)
        assert "x" in line
        assert ":" not in line.split("x", 1)[0]  # the code prefix isn't leaked into the message


def test_cli_bad_type_warns_and_skips(tmp_path: Path):
    entry = _exe(tmp_path)
    store.write_parameters(entry.slug, [ParamDecl(name="w", delivery="flag", type="str")])
    result = runner.invoke(cli.app, ["params", "prog", "--type", "w=integer"])
    assert result.exit_code == 0, result.output
    assert "unknown type" in result.output
    assert store.read_parameters(entry.slug)[0].type == "str"  # unchanged


def test_cli_secret_override_persists_value_now_that_it_isnt_secret(tmp_path: Path, run_entry_spy):
    # THE defect fix, end to end: {token_file} matches the auto-secret heuristic, so before it
    # could never be un-secreted and its value was never saved. A declared row with --no-secret
    # makes it public, and the run value now persists in argstate.
    runner.invoke(cli.app, ["add", "--cmd", "auth {token_file}", "--name", "auth", "--no-input"])
    entry = store.resolve("auth")
    # First declare token_file as a non-secret placeholder.
    result = runner.invoke(
        cli.app, ["params", "auth", "--add", "token_file", "--no-secret", "token_file"]
    )
    assert result.exit_code == 0, result.output
    decl = store.read_parameters(entry.slug)[0]
    assert decl.secret is False  # overridden away from the auto-secret heuristic
    result = runner.invoke(cli.app, ["run", "auth", "--set", "token_file=creds.json", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["values"] == {"token_file": "creds.json"}
    # Now that it isn't secret, the value IS remembered (the old behavior scrubbed it).
    assert argstate.load_state(entry.slug)["values"]["token_file"] == "creds.json"  # noqa: S105


def test_cli_secret_declared_env_purges_prior_plaintext(tmp_path: Path):
    entry = _exe(tmp_path)
    store.write_parameters(entry.slug, [ParamDecl(name="TOKEN", delivery="env")])
    argstate.save_last(entry.slug, values={"TOKEN": "plaintext"})
    result = runner.invoke(cli.app, ["params", "prog", "--secret", "TOKEN"])
    assert result.exit_code == 0, result.output
    assert "Removed previously stored plaintext" in result.output
    assert "TOKEN" not in argstate.load_state(entry.slug)["values"]


def test_cli_declared_secret_env_source_resolves_without_prompting(
    tmp_path, run_entry_spy, monkeypatch
):
    # A secret env param with an env_source resolves under --no-input with no prompt: the value
    # comes from the environment, never from a form field.
    runner.invoke(cli.app, ["add", "--cmd", "echo hi", "--name", "svc", "--no-input"])
    entry = store.resolve("svc")
    runner.invoke(
        cli.app,
        [
            "params",
            "svc",
            "--add",
            "TOKEN",
            "--deliver",
            "TOKEN=env",
            "--secret",
            "TOKEN",
            "--env-source",
            "TOKEN=SVC_TOKEN",
        ],
    )
    monkeypatch.setenv("SVC_TOKEN", "from-env")
    result = runner.invoke(cli.app, ["run", "svc", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["env_overlay"] == {"TOKEN": "from-env"}
    assert "TOKEN" not in argstate.load_state(entry.slug)["values"]  # C3: never persisted


def test_cli_run_set_env_and_placeholder_dry_run(tmp_path: Path):
    runner.invoke(cli.app, ["add", "--cmd", "echo {msg}", "--name", "dr", "--no-input"])
    runner.invoke(
        cli.app,
        ["params", "dr", "--add", "RETRIES", "--deliver", "RETRIES=env", "--default", "RETRIES=3"],
    )
    result = runner.invoke(cli.app, ["run", "dr", "--set", "msg=hello", "--dry-run", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "RETRIES=3" in result.output  # env overlay shown in the transparency line


def test_cli_rm_declared_param(tmp_path: Path):
    entry = _exe(tmp_path)
    store.write_parameters(
        entry.slug,
        [ParamDecl(name="a", delivery="flag"), ParamDecl(name="b", delivery="flag")],
    )
    result = runner.invoke(cli.app, ["params", "prog", "--rm", "a"])
    assert result.exit_code == 0, result.output
    assert [d.name for d in store.read_parameters(entry.slug)] == ["b"]


def test_cli_exe_declared_show_json_param_origin(tmp_path: Path):
    entry = _exe(tmp_path)
    store.write_parameters(
        entry.slug, [ParamDecl(name="w", delivery="flag", flag="--w", type="int")]
    )
    payload = json.loads(runner.invoke(cli.app, ["show", "prog", "--json"]).output)
    assert payload["param_source"] == "declared"
    assert payload["param_origin"] == "declared"
    js_fields = {f["key"]: f for f in payload["fields"]}
    assert js_fields["w"]["source"] == "flag"


def test_cli_exe_no_declared_show_json_param_origin_none(tmp_path: Path):
    _exe(tmp_path)
    payload = json.loads(runner.invoke(cli.app, ["show", "prog", "--json"]).output)
    assert payload["param_source"] == "none"
    assert payload["param_origin"] == "none"


def test_cli_command_env_show_json_source_env(tmp_path: Path):
    runner.invoke(cli.app, ["add", "--cmd", "echo {m}", "--name", "cj", "--no-input"])
    runner.invoke(cli.app, ["params", "cj", "--add", "N", "--deliver", "N=env"])
    payload = json.loads(runner.invoke(cli.app, ["show", "cj", "--json"]).output)
    fields = {f["key"]: f for f in payload["fields"]}
    assert fields["N"]["source"] == "env"  # env value source flows through show --json


def test_cli_exe_show_masks_secret_default_and_last_value(tmp_path: Path):
    # Covers the read-view secret masking: a secret row with a stored value → •••; a secret row
    # with a default → •••; a secret row with no default → —.
    entry = _exe(tmp_path)
    store.write_parameters(
        entry.slug,
        [
            ParamDecl(name="a", delivery="flag", type="str", secret=True),  # no default
            ParamDecl(name="b", delivery="flag", type="str", default="x", secret=True),
        ],
    )
    argstate.save_last(entry.slug, values={"a": "stale"})  # a value lingering from a public past
    result = runner.invoke(cli.app, ["params", "prog"])
    assert result.exit_code == 0, result.output
    assert "•••" in result.output
    assert "stale" not in result.output  # the secret value is never echoed


def test_cli_command_show_masks_secret_placeholder_and_undeclared(tmp_path: Path):
    # Covers _show_command_params secret masking + an undeclared placeholder's empty schema suffix.
    entry = store.add_command("login {password} {other}", name="lg")
    entry = store.write_parameters(
        entry.slug,
        [
            ParamDecl(
                name="password",
                delivery="placeholder",
                secret=True,
                default="seed",
                required=True,
            )
        ],
    )
    argstate.save_last(entry.slug, values={"password": "stale"})
    result = runner.invoke(cli.app, ["params", "lg"])
    assert result.exit_code == 0, result.output
    assert "•••" in result.output  # secret default + last value masked
    assert "seed" not in result.output
    assert "other" in result.output  # the undeclared placeholder is still listed (bare)


# ---- capability-honesty fixes (review findings) ------------------------------------------------


def _ruby(tmp_path: Path, name: str = "rb") -> store.Entry:
    src = tmp_path / f"{name}.rb"
    src.write_text('#!/usr/bin/env ruby\nputs "hi"\n', encoding="utf-8")
    return store.add_script(src, kind="ruby", name=name)


def test_declared_add_on_interpreted_meta_kind_defaults_to_deliverable_flag(tmp_path: Path):
    # An interpreted kind whose schema home is meta (ruby/perl/lua/r) assembles a real argv, so a
    # bare --add must default to flag delivery — not the placeholder a template gets, which would
    # be a dead row that never reaches the child (the confirmed capability-honesty defect).
    entry = _ruby(tmp_path)
    assert runner.invoke(cli.app, ["params", entry.slug, "--add", "SIZE"]).exit_code == 0
    (decl,) = store.read_parameters(entry.slug)
    assert decl.delivery == "flag"
    plan = flows.plan_for_entry(store.resolve(entry.slug))
    assert plan.source == "declared"
    assert [(f.key, f.source) for f in plan.fields] == [("SIZE", "flag")]


def test_declared_add_on_interpreted_kind_delivers_at_run(tmp_path: Path):
    entry = _ruby(tmp_path, name="rb2")
    runner.invoke(cli.app, ["params", entry.slug, "--add", "SIZE", "--flag", "SIZE=--size"])
    result = runner.invoke(
        cli.app, ["run", entry.slug, "--set", "SIZE=5", "--dry-run", "--no-input"]
    )
    assert result.exit_code == 0
    assert "--size" in result.output.replace("\n", "")
    assert "5" in result.output


def test_reader_kind_declared_env_rider_merges_not_erases(tmp_path: Path, monkeypatch):
    # A PowerShell entry reads its param() block statically; a declared env row must RIDE ALONG
    # after the reader's fields, never short-circuit _declared_plan and erase the whole form.
    import dataclasses

    from skit.langs import registry
    from skit.langs.base import CliReader
    from skit.langs.python.argspec import ArgSpec

    ps = tmp_path / "deploy.ps1"
    ps.write_text("param([string]$Region)\n", encoding="utf-8")
    entry = store.add_script(ps, kind="powershell")
    store.write_parameters(entry.slug, [ParamDecl(name="LOGLEVEL", delivery="env")])
    fake = ArgSpec(fields=[ParamDecl(name="Region", delivery="flag", flag="-Region")])
    spec = dataclasses.replace(
        registry._powershell_spec(), cli_reader=CliReader(read_cli=lambda _t: fake)
    )
    monkeypatch.setattr("skit.flows.spec_for", lambda _kind: spec)
    plan = flows.plan_for_entry(store.resolve(entry.slug))
    assert plan.source == "argparse"
    assert [(f.key, f.source) for f in plan.fields] == [("Region", "flag"), ("LOGLEVEL", "env")]


def test_reader_kind_declared_rows_stand_alone_when_no_readable_surface(
    tmp_path: Path, monkeypatch
):
    # No readable param() (no pwsh, or no param block) but declared rows exist: they still form a
    # plan on their own rather than vanishing into the "none" fall-through.
    import dataclasses

    from skit.langs import registry
    from skit.langs.base import CliReader

    ps = tmp_path / "d2.ps1"
    ps.write_text("Write-Output 'hi'\n", encoding="utf-8")
    entry = store.add_script(ps, kind="powershell", name="d2")
    store.write_parameters(entry.slug, [ParamDecl(name="LOGLEVEL", delivery="env")])
    spec = dataclasses.replace(
        registry._powershell_spec(), cli_reader=CliReader(read_cli=lambda _t: None)
    )
    monkeypatch.setattr("skit.flows.spec_for", lambda _kind: spec)
    plan = flows.plan_for_entry(store.resolve(entry.slug))
    assert plan.source == "declared"
    assert [(f.key, f.source) for f in plan.fields] == [("LOGLEVEL", "env")]


def test_declared_table_is_shown_for_an_interpreted_meta_kind(tmp_path: Path):
    # The read surface must not deny what the write surface created and the run delivers: a ruby
    # entry's declared rows appeared in --json but printed "has no managed parameters" in the
    # human view (gated on family=="binary" instead of "the schema lives in meta").
    entry = _ruby(tmp_path, name="rb3")
    runner.invoke(cli.app, ["params", entry.slug, "--add", "GREETING"])
    out = runner.invoke(cli.app, ["params", entry.slug]).output
    assert "GREETING" in out
    assert "has no managed parameters" not in out


def test_declared_param_on_an_interpreted_kind_actually_delivers(tmp_path: Path, run_entry_spy):
    entry = _ruby(tmp_path, name="rb4")
    runner.invoke(
        cli.app, ["params", entry.slug, "--add", "GREETING", "--flag", "GREETING=--greeting"]
    )
    result = runner.invoke(cli.app, ["run", entry.slug, "--set", "GREETING=world", "--no-input"])
    assert result.exit_code == 0
    assert run_entry_spy["extra"] == ["--greeting", "world"]


def test_template_add_of_a_non_placeholder_name_creates_a_deliverable_env_row(tmp_path: Path):
    # The template is the truth about which {slots} exist, so a name that is not one cannot be a
    # placeholder. Defaulting it to placeholder delivery wrote a row the CLI listed but the run
    # surface refused ("Unknown parameter for --set") — and the TUI created an env row for the very
    # same action. env is the delivery a template can always honour.
    entry = store.add_command("greet {WHO}", name="tpl")
    assert runner.invoke(cli.app, ["params", entry.slug, "--add", "RETRIES"]).exit_code == 0
    decls = {d.name: d.delivery for d in store.read_parameters(entry.slug)}
    assert decls["RETRIES"] == "env"
    # and it really delivers, rather than being denied by --set
    result = runner.invoke(
        cli.app,
        ["run", entry.slug, "--set", "WHO=ada", "--set", "RETRIES=3", "--dry-run", "--no-input"],
    )
    assert result.exit_code == 0
    assert "RETRIES=3" in result.output.replace("\n", "")


def test_template_add_of_a_real_placeholder_name_still_fills_the_slot(tmp_path: Path):
    entry = store.add_command("greet {WHO}", name="tpl2")
    runner.invoke(cli.app, ["params", entry.slug, "--add", "WHO", "--type", "WHO=str"])
    decls = {d.name: d.delivery for d in store.read_parameters(entry.slug)}
    assert decls["WHO"] == "placeholder"
