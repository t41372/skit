"""Mutation-hardening tests for skit.flows: each test pins a real, observable contract of
the form layer (plans, declared-rider merge, reader plans, split/gap/syntax injection error
messages, the injector request forwarding, and transparency env prefixing).

Companion to tests/test_flows.py — same public surface (flows.plan_for_entry / assemble /
execute / transparency_lines and a few pure helpers), driven so a single mutated line changes
an assertion's outcome.
"""

from __future__ import annotations

import dataclasses
from datetime import datetime
from pathlib import Path

from skit import flows
from skit.langs.base import (
    CliReader,
    InjectGapError,
    Injector,
    InjectRequest,
    InjectResult,
    InjectSplitError,
    InjectSyntaxError,
)
from skit.langs.python.argspec import ArgSpec
from skit.models import Entry, ScriptMeta
from skit.params import ParamDecl

NOW = datetime(2026, 7, 9, 14, 30, 5)

MANAGED_SCRIPT = """# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "OUTPUT"
# kind = "const"
# type = "str"
# default = "out.jpg"
#
# [[tool.skit.params]]
# name = "WIDTH"
# kind = "const"
# type = "int"
# default = 800
#
# [[tool.skit.params]]
# name = "API_KEY"
# kind = "const"
# type = "str"
# default = "xxx"
# secret = true
# env_source = "MY_API_KEY"
# ///
OUTPUT = 'out.jpg'
WIDTH = 800
API_KEY = 'xxx'
print(OUTPUT, WIDTH, API_KEY)
"""


def _python_entry(tmp_path: Path, text: str, slug: str = "s") -> Entry:
    d = tmp_path / "store" / slug
    d.mkdir(parents=True)
    (d / "script.py").write_text(text, encoding="utf-8")
    meta = ScriptMeta(name=slug, kind="python", mode="copy", source=str(tmp_path / "orig.py"))
    return Entry(slug=slug, meta=meta, dir=d)


def _emit_sink():
    lines: list[str] = []
    return lines, lines.append


# --------------------------------------------------------------------------
# _assemble_flags: a non-flag field is skipped, not a hard stop (continue, not break)
# --------------------------------------------------------------------------


def test_assemble_flags_skips_non_flag_field_without_dropping_later_flags(tmp_path):
    # A mixed-delivery plan can put an env field BEFORE a flag field. The flag field must still
    # reach argv: the loop skips (continue) the env row, it does not stop (break) at it.
    plan = flows.FormPlan(
        source="declared",
        fields=[
            flows.FormField(key="E", label="E", source="env"),
            flows.FormField(key="x", label="x", source="flag", flag="--x"),
        ],
    )
    result = flows._assemble_flags(plan, {"E": "e", "x": "val"}, tmp_path)
    assert result == ["--x", "val"]  # break would lose --x entirely (result == [])


# --------------------------------------------------------------------------
# _declared_riders: only flag/env deliveries, and only names not already taken
# --------------------------------------------------------------------------


def test_declared_riders_keeps_flag_env_and_drops_taken(tmp_path):
    meta = ScriptMeta(
        name="dr",
        kind="python",
        parameters=[
            {"name": "FLAGP", "delivery": "flag", "flag": "--flagp"},  # flag + free -> kept
            {"name": "TAKEN", "delivery": "flag", "flag": "--taken"},  # flag but taken -> dropped
            {"name": "PH", "delivery": "placeholder"},  # not flag/env -> dropped
        ],
    )
    entry = Entry(slug="dr", meta=meta, dir=tmp_path)
    riders = flows._declared_riders(entry, {"TAKEN"})
    # `and`->`or` would let TAKEN and PH through; "flag"->"XXflagXX"/"FLAG" would drop FLAGP.
    assert [f.key for f in riders] == ["FLAGP"]
    assert riders[0].source == "flag"


# --------------------------------------------------------------------------
# _reader_plan (PowerShell tier): the plan carries the script text for the delivery layer
# --------------------------------------------------------------------------


def test_reader_plan_carries_the_script_text(tmp_path):
    entry = _python_entry(tmp_path, "param($Name)\nWrite-Host $Name\n", slug="rp")
    spec = ArgSpec(fields=[ParamDecl(name="Name", delivery="flag", flag="-Name")])
    reader = CliReader(read_cli=lambda _text: spec)
    plan = flows._reader_plan(entry, reader)
    assert plan is not None
    assert plan.source == "argparse"
    assert [f.key for f in plan.fields] == ["Name"]
    # The reader plan must forward the exact script text (kills text=None and the dropped kwarg).
    assert plan.text == "param($Name)\nWrite-Host $Name\n"


# --------------------------------------------------------------------------
# _split_message: the exact wording for each `read`-mangling reason
# --------------------------------------------------------------------------


def test_split_message_exact_for_each_reason():
    line_break = flows._split_message(InjectSplitError("FIRST", "line-break"))
    assert line_break == (
        "FIRST can't contain a line break: a shell `read` takes ONE line, so everything "
        "after the break would be thrown away."
    )
    field_split = flows._split_message(InjectSplitError("MIDDLE", "field-split"))
    assert field_split == (
        "MIDDLE is read on the same line as other values, so its value can't contain spaces "
        "or tabs — the shell would split it across the other fields. Only the LAST value on a "
        "`read` line may contain spaces."
    )
    edge_space = flows._split_message(InjectSplitError("PAD", "edge-space"))
    assert edge_space == (
        "PAD starts or ends with a space or tab, which a shell `read` strips off the "
        "line — the script would receive it trimmed. Remove the surrounding whitespace."
    )


# --------------------------------------------------------------------------
# execute: the injector request forwards interpreter + original source path
# --------------------------------------------------------------------------


def test_execute_forwards_interpreter_and_source_to_injector(tmp_path, monkeypatch):
    from skit import launcher

    entry = _python_entry(tmp_path, MANAGED_SCRIPT, slug="req")
    entry.meta.interpreter = "python3.11"  # a truthy interpreter distinct from the "" default
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(
        plan, {"OUTPUT": "o", "WIDTH": "1", "API_KEY": "k"}, [], cwd=tmp_path, env={}, now=NOW
    )

    captured: dict[str, InjectRequest] = {}

    def fake_inject(request: InjectRequest) -> InjectResult:
        captured["req"] = request
        return InjectResult()

    real = flows.spec_for("python")
    assert real is not None
    fake_spec = dataclasses.replace(real, injector=Injector(inject=fake_inject))
    monkeypatch.setattr(flows, "spec_for", lambda _kind: fake_spec)
    monkeypatch.setattr(launcher, "run_entry", lambda *a, **k: 0)

    _lines, emit = _emit_sink()
    outcome = flows.execute(entry, plan, asm, emit=emit)
    assert outcome.code == 0
    req = captured["req"]
    # `or`->`and` and the dropped interpreter kwarg both collapse this to "" (python's default
    # interpreter is empty), so the truthy `entry.meta.interpreter` proves the `or` path ran.
    assert req.interpreter == "python3.11"
    # source=None and the dropped source kwarg both blank this out.
    assert req.source == entry.meta.source
    assert req.source != ""


# --------------------------------------------------------------------------
# execute: injection-failure messages (gap / syntax) are exact and correctly classified
# --------------------------------------------------------------------------


def test_execute_gap_error_message_is_exact(tmp_path, monkeypatch):
    from skit.langs.python import shim

    def boom(*_a, **_k):
        raise InjectGapError(empty="LAST", filled="FIRST")

    monkeypatch.setattr(shim, "inject", boom)
    entry = _python_entry(tmp_path, MANAGED_SCRIPT, slug="gap")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(
        plan, {"OUTPUT": "o", "WIDTH": "1", "API_KEY": "k"}, [], cwd=tmp_path, env={}, now=NOW
    )
    _lines, emit = _emit_sink()
    outcome = flows.execute(entry, plan, asm, emit=emit)
    assert outcome.code is None
    assert outcome.failure == flows.FAIL_BAD_VALUE
    assert outcome.message == (
        "LAST is empty, but FIRST is filled and they are read on the same line — a shell "
        "`read` would hand your value to LAST. Fill LAST in, or clear FIRST."
    )


def test_execute_syntax_error_message_is_exact_and_carries_no_resync(tmp_path, monkeypatch):
    from skit.langs.python import shim

    def boom(*_a, **_k):
        raise InjectSyntaxError("BOOM")

    monkeypatch.setattr(shim, "inject", boom)
    entry = _python_entry(tmp_path, MANAGED_SCRIPT, slug="syn")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(
        plan, {"OUTPUT": "o", "WIDTH": "1", "API_KEY": "k"}, [], cwd=tmp_path, env={}, now=NOW
    )
    _lines, emit = _emit_sink()
    outcome = flows.execute(entry, plan, asm, emit=emit)
    assert outcome.code is None
    assert outcome.failure == flows.FAIL_DRIFT
    # Exact detail (str(exc), not str(None)) and no dropped message; a corruption is NOT a resync.
    assert outcome.message == "skit refused to run its own injected copy: BOOM"
    assert "resync" not in outcome.message


# --------------------------------------------------------------------------
# plan_for_entry: guard, declared-rider merge, declared-only source/text
# --------------------------------------------------------------------------


def test_plan_degraded_analyzer_falls_back_to_none(tmp_path, monkeypatch):
    # A kind whose analyzer degraded to None (broken grammar wheel) but whose params_io still
    # reads the in-file block must NOT crash reconciling with a missing analyzer — the guard
    # returns the "none" plan (extra-args escape). `params_io is None OR analyzer is None`
    # must stay an OR: turning it into AND would fall through into `analyzer.reconcile` and crash.
    real = flows.spec_for("python")
    assert real is not None
    degraded = dataclasses.replace(
        real, analyzer=None, cli_reader=None, injector=None, normalizer=None
    )
    monkeypatch.setattr(flows, "spec_for", lambda _kind: degraded)
    entry = _python_entry(tmp_path, MANAGED_SCRIPT, slug="degr")
    plan = flows.plan_for_entry(entry)
    assert plan.source == "none"


def test_plan_inject_merges_a_declared_flag_rider(tmp_path):
    # A managed (inject) script that ALSO hand-declares a flag param: the rider merges after the
    # in-file fields, keyed against the taken set. A None `taken` (the mutant) raises on `not in`.
    entry = _python_entry(tmp_path, MANAGED_SCRIPT, slug="merge")
    entry.meta.parameters = [{"name": "EXTRA", "delivery": "flag", "flag": "--extra"}]
    plan = flows.plan_for_entry(entry)
    assert plan.source == "inject"
    assert [f.key for f in plan.fields] == ["OUTPUT", "WIDTH", "API_KEY", "EXTRA"]
    assert plan.fields[-1].source == "flag"


def test_plan_declared_only_on_analyzable_kind_carries_source_and_text(tmp_path):
    # A plain (no in-file block) python script with a declared env row forms a "declared" plan
    # on its own, carrying the script text through for delivery.
    entry = _python_entry(tmp_path, "print('hi')\n", slug="declonly")
    entry.meta.parameters = [{"name": "TOKEN", "delivery": "env", "type": "str"}]
    plan = flows.plan_for_entry(entry)
    assert plan.source == "declared"  # kills source=None / "XXdeclaredXX" / "DECLARED"
    assert plan.text == "print('hi')\n"  # kills text=None and the dropped text kwarg
    assert [f.key for f in plan.fields] == ["TOKEN"]
    assert plan.fields[0].source == "env"


# --------------------------------------------------------------------------
# transparency_lines: the env prefix concatenates pairs with no separator
# --------------------------------------------------------------------------


def test_transparency_env_prefix_joins_pairs_with_no_separator(tmp_path):
    entry = _python_entry(tmp_path, "print(1)\n", slug="envp")
    asm = flows.Assembly(masked_env={"A": "1", "B": "2"})
    line = flows.transparency_lines(entry, asm, None)[-1]
    assert "A=1 B=2 " in line  # "".join, not "XXXX".join
    assert "XXXX" not in line
