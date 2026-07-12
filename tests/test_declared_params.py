"""Declared parameter schema ([[parameters]]) + env delivery (P1, docs/design/multilang.md).

The declared schema is what gives exe/command entries a real form (types, defaults,
optional, secret OVERRIDE — fixing the {token_file} auto-secret-no-override defect),
and env delivery is the zero-rewrite value channel every kind can use.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from skit import flows, launcher, store
from skit.models import Entry, ScriptMeta
from skit.params import (
    ParamDecl,
    declared_for_template,
    declared_from_meta,
    synthesized_placeholder,
)

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
        entry, extra, *, values=None, invoke_cwd=None, script_override=None, env_overlay=None
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
