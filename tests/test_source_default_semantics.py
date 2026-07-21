"""Source-default tracking: the script is the truth, not the [tool.skit] cache.

These pin the change that made a script's CURRENT literal beat a stale block default
everywhere it matters:

- reconcile records the source's current default for every ok-matched const/envdefault
  (Report.current_defaults), but NOT for a type-changed spec;
- edit_specs --resync writes that refreshed default back into the stored record (and a
  type-changed spec takes both type and default from the candidate);
- plan_for_entry overlays those defaults onto the form fields, so the run form prefills
  the script's value, not the manage-time cache;
- a free-text field with a known default (FormField.delivers_empty) delivers '' when
  cleared, and every other value the form shows is delivered as shown;
- last-used (remembered_values) drops a value equal to the default so the next prefill
  can follow the source, while a preset stores the run's values verbatim.

Style mirrors tests/test_flows.py (MANAGED_SCRIPT block pattern, _python_entry helper,
NOW datetime); the store/argstate isolation rides on conftest's autouse SKIT_*_DIR
fixture, so these never touch real user directories and never chdir.
"""

from __future__ import annotations

from datetime import datetime
from pathlib import Path

from skit import analysis, argstate, flows
from skit.langs.python.analyzer import analyze as py_analyze
from skit.langs.shell import analyzer as shell
from skit.params import ParamDecl

NOW = datetime(2026, 7, 9, 14, 30, 5)

# Block default "hello" is a stale manage-time cache; the body now says "bonjour". The
# script is the truth: the run form must prefill "bonjour", and injecting "bonjour" is a
# no-op the delivery path skips.
REFRESH_SCRIPT = """# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "GREETING"
# kind = "const"
# type = "str"
# default = "hello"
# ///
GREETING = 'bonjour'
print(GREETING)
"""

# Shell envdefault whose block default (9999) went stale; the body reads ${PORT:-8080}.
SHELL_ENVDEFAULT_SCRIPT = """#!/usr/bin/env bash
# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "PORT"
# kind = "envdefault"
# type = "int"
# default = 9999
# ///
echo "${PORT:-8080}"
"""


def _python_entry(tmp_path: Path, text: str, slug: str = "s"):
    from skit.models import Entry, ScriptMeta

    d = tmp_path / "store" / slug
    d.mkdir(parents=True)
    (d / "script.py").write_text(text, encoding="utf-8")
    meta = ScriptMeta(name=slug, kind="python", mode="copy", source=str(tmp_path / "orig.py"))
    return Entry(slug=slug, meta=meta, dir=d)


def _shell_entry(tmp_path: Path, text: str, slug: str = "sh"):
    from skit.models import Entry, ScriptMeta

    d = tmp_path / "store" / slug
    d.mkdir(parents=True)
    (d / "script.sh").write_text(text, encoding="utf-8")
    meta = ScriptMeta(name=slug, kind="shell", mode="copy", source=str(tmp_path / "orig.sh"))
    return Entry(slug=slug, meta=meta, dir=d)


def _envdefault(name: str) -> ParamDecl:
    return ParamDecl(name=name, binding="envdefault", delivery="env", type="str")


# --------------------------------------------------------------------------
# 1) plan_for_entry: the SOURCE's current default beats a stale block cache
# --------------------------------------------------------------------------


def test_plan_refreshes_a_stale_block_default_from_the_python_body(tmp_path):
    # Block says default = "hello"; the body assigns "bonjour". The form field must carry
    # the body's value, not the cache.
    plan = flows.plan_for_entry(_python_entry(tmp_path, REFRESH_SCRIPT, slug="ref"))
    assert plan.source == "inject"
    (field,) = plan.fields
    assert field.key == "GREETING"
    assert field.default == "bonjour"  # the script wins over the stale "hello"
    assert field.has_default is True


def test_plan_refreshes_a_stale_shell_envdefault_from_the_body(tmp_path):
    # Same rule through the shell analyzer: block default 9999 is stale, ${PORT:-8080} is
    # the truth. The env-delivered field prefills "8080".
    plan = flows.plan_for_entry(_shell_entry(tmp_path, SHELL_ENVDEFAULT_SCRIPT, slug="port"))
    assert plan.source == "inject"
    (field,) = plan.fields
    assert (field.key, field.source) == ("PORT", "env")
    assert field.default == "8080"  # refreshed from ${PORT:-8080}, not 9999
    assert field.has_default is True


# --------------------------------------------------------------------------
# 2) reconcile: current_defaults for ok const/envdefault, not for type-changed
# --------------------------------------------------------------------------


def test_reconcile_records_current_default_for_an_ok_const(tmp_path):
    report = analysis.reconcile(
        'CITY = "Taipei"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str")],
        analyze=py_analyze,
    )
    assert [s.name for s in report.ok] == ["CITY"]
    assert report.current_defaults == {"CITY": "Taipei"}


def test_reconcile_records_current_default_for_an_ok_envdefault():
    # Shell envdefault: the source's fallback (int 8080) is the recorded current default.
    report = shell.reconcile('echo "${PORT:-8080}"\n', [_envdefault("PORT")])
    assert [s.name for s in report.ok] == ["PORT"]
    assert report.current_defaults == {"PORT": 8080}


def test_reconcile_omits_current_default_for_a_type_changed_const(tmp_path):
    # Block says int, the source now holds a string literal: this is drift (report.changed),
    # so the stale prefill is kept until the user resyncs — NOT tracked in current_defaults.
    report = analysis.reconcile(
        'RETRIES = "three"\nprint(RETRIES)\n',
        [ParamDecl(name="RETRIES", binding="const", type="int")],
        analyze=py_analyze,
    )
    assert [s.name for s, _ in report.changed] == ["RETRIES"]
    assert report.current_defaults == {}  # a type-changed spec is excluded


# --------------------------------------------------------------------------
# 3) edit_specs --resync writes the refreshed default back into the record
# --------------------------------------------------------------------------


def test_resync_writes_source_default_into_ok_and_type_changed_specs():
    # One resync exercises both write paths: an ok const's default follows the source, and
    # a type-changed const takes BOTH its type and its default from the candidate.
    text = 'CITY = "Taipei"\nRETRIES = "three"\nprint(CITY, RETRIES)\n'
    specs = [
        ParamDecl(name="CITY", binding="const", type="str", default="old-city"),
        ParamDecl(name="RETRIES", binding="const", type="int", default=3),
    ]
    result = analysis.edit_specs(text, specs, resync=True, analyze=py_analyze)
    by = {s.name: s for s in result.specs}
    assert (by["CITY"].type, by["CITY"].default) == ("str", "Taipei")  # ok: default refreshed
    assert (by["RETRIES"].type, by["RETRIES"].default) == ("str", "three")  # changed: type+default


def test_resync_current_default_and_rebind_and_untouched_input_share_one_pass():
    # The resync elif chain, exercised end to end in one call:
    #   CITY    -> current_defaults elif (its literal moved)
    #   input-1 -> exact prompt match, falls through the chain untouched
    #   input-2 -> its prompt no longer resolves, re-anchored by position (rebind)
    text = (
        'CITY = "Taipei"\nwho = input("Name: ")\npw = input("New label: ")\nprint(CITY, who, pw)\n'
    )
    specs = [
        ParamDecl(name="CITY", binding="const", type="str", default="old"),
        ParamDecl(name="input-1", binding="input", delivery="inject", order=0, prompt="Name: "),
        ParamDecl(
            name="input-2", binding="input", delivery="inject", order=1, prompt="Old label: "
        ),
    ]
    result = analysis.edit_specs(text, specs, resync=True, analyze=py_analyze)
    by = {s.name: s for s in result.specs}
    assert by["CITY"].default == "Taipei"  # current_defaults elif fired
    assert (by["input-1"].order, by["input-1"].prompt) == (0, "Name: ")  # untouched fall-through
    assert (by["input-2"].order, by["input-2"].prompt) == (1, "New label: ")  # rebound to source
    assert "resync-rebound:input-2" in result.warnings


def test_reconcile_ok_const_without_a_default_is_not_recorded():
    # A matched ok const whose candidate carries no default (default is None) must not be
    # written into current_defaults — the `if cand.default is not None` guard. Real analyzers
    # always give a const a literal, so drive this through a synthetic analyze.
    def analyze(_text: str) -> analysis.Analysis:
        return analysis.Analysis(
            candidates=[analysis.Candidate(binding="const", name="X", type="str", default=None)]
        )

    report = analysis.reconcile(
        "_\n", [ParamDecl(name="X", binding="const", type="str")], analyze=analyze
    )
    assert [s.name for s in report.ok] == ["X"]
    assert report.current_defaults == {}


def test_reconcile_ok_envdefault_without_a_default_is_not_recorded():
    # The envdefault twin of the guard above: an ok env match with a None default records
    # nothing (the value arrives by env either way).
    def analyze(_text: str) -> analysis.Analysis:
        return analysis.Analysis(
            candidates=[
                analysis.Candidate(
                    binding="envdefault", name="PORT", env_name="PORT", type="str", default=None
                )
            ]
        )

    report = analysis.reconcile("_\n", [_envdefault("PORT")], analyze=analyze)
    assert [s.name for s in report.ok] == ["PORT"]
    assert report.current_defaults == {}


# --------------------------------------------------------------------------
# 4) assemble: a value that equals the source default is not injected
# --------------------------------------------------------------------------


def test_assemble_injects_a_value_that_equals_the_source_default(tmp_path):
    # Whatever the form shows IS what the script gets: a value equal to the source's own
    # literal is still injected, so the run matches the form even when a main-guard
    # rebinds the name, the value carries a {token}, or the spec has drifted. (An earlier
    # revision skipped this case to save a temp copy; every one of those situations then
    # ran on a value the form and the transparency line denied.)
    plan = flows.plan_for_entry(_python_entry(tmp_path, REFRESH_SCRIPT, slug="skip"))
    equal = flows.assemble(plan, {"GREETING": "bonjour"}, [], cwd=tmp_path, env={}, now=NOW)
    assert equal.inject_values == {"GREETING": "bonjour"}
    assert equal.display == [("GREETING", "bonjour")]
    changed = flows.assemble(plan, {"GREETING": "other"}, [], cwd=tmp_path, env={}, now=NOW)
    assert changed.inject_values == {"GREETING": "other"}
    assert changed.display == [("GREETING", "other")]


def test_assemble_injects_the_expansion_of_an_untouched_token_default(tmp_path):
    # The token preview the form shows must be what lands: an untouched default that
    # carries {today} delivers the EXPANDED text, never the literal braces.
    text = REFRESH_SCRIPT.replace('default = "hello"', 'default = "out_{today}.csv"').replace(
        "GREETING = 'bonjour'", "GREETING = 'out_{today}.csv'"
    )
    plan = flows.plan_for_entry(_python_entry(tmp_path, text, slug="tok"))
    asm = flows.assemble(plan, {"GREETING": "out_{today}.csv"}, [], cwd=tmp_path, env={}, now=NOW)
    assert asm.inject_values == {"GREETING": "out_2026-07-09.csv"}


# --------------------------------------------------------------------------
# 5) delivers_empty: a cleared free-text field delivers '' across all sources
# --------------------------------------------------------------------------


def test_assemble_inject_delivers_empty_string_when_cleared(tmp_path):
    # A str const with a known default is WYSIWYG: clearing it delivers '' (an empty string
    # is a legitimate value), shown as '' in the transparency display.
    plan = flows.plan_for_entry(_python_entry(tmp_path, REFRESH_SCRIPT, slug="empty"))
    asm = flows.assemble(plan, {"GREETING": ""}, [], cwd=tmp_path, env={}, now=NOW)
    assert asm.inject_values == {"GREETING": ""}
    assert ("GREETING", "''") in asm.display


def test_assemble_env_delivers_empty_string_when_cleared(tmp_path):
    # An env-delivered free-text field with a default exports the variable set to "" when
    # cleared (the ${NAME:-default} script still falls back; a ${NAME-default} one gets '').
    plan = flows.FormPlan(
        source="inject",
        fields=[
            flows.FormField(
                key="CITY",
                label="CITY",
                source="env",
                kind="str",
                has_default=True,
                default="Taipei",
            )
        ],
    )
    asm = flows.assemble(plan, {"CITY": ""}, [], cwd=tmp_path, env={}, now=NOW)
    assert asm.env_values == {"CITY": ""}


def test_assemble_flag_delivers_empty_string_when_cleared(tmp_path):
    # A free-text flag with a default emits `--x ''` when cleared, instead of omitting it.
    plan = flows.FormPlan(
        source="argparse",
        fields=[
            flows.FormField(
                key="x",
                label="x",
                source="flag",
                flag="--x",
                kind="str",
                has_default=True,
                default="def",
            )
        ],
    )
    asm = flows.assemble(plan, {"x": ""}, [], cwd=tmp_path, env={}, now=NOW)
    assert asm.args == ["--x", ""]


# --------------------------------------------------------------------------
# 6) delivers_empty is False everywhere WYSIWYG is unsound
# --------------------------------------------------------------------------


def test_delivers_empty_matrix():
    # WYSIWYG applies to exactly one shape: a non-secret, single-value, free-text (str/path)
    # inject/flag/env field with a known default. Every disqualifier keeps '' meaning "unset".
    def field(
        *,
        kind: str = "str",
        source: str = "inject",
        has_default: bool = True,
        secret: bool = False,
        degraded: bool = False,
        multiple: bool = False,
        input_binding: bool = False,
    ) -> flows.FormField:
        return flows.FormField(
            key="k",
            label="k",
            kind=kind,
            source=source,
            has_default=has_default,
            secret=secret,
            degraded=degraded,
            multiple=multiple,
            input_binding=input_binding,
        )

    # The one true delivers-empty shape (both str and path qualify).
    assert field().delivers_empty is True
    assert field(kind="path").delivers_empty is True
    # Every disqualifier.
    assert field(kind="int").delivers_empty is False
    assert field(kind="float").delivers_empty is False
    assert field(kind="bool").delivers_empty is False
    assert field(kind="choice").delivers_empty is False
    assert field(secret=True).delivers_empty is False
    assert field(degraded=True).delivers_empty is False
    assert field(multiple=True).delivers_empty is False
    assert field(has_default=False).delivers_empty is False  # no default: nothing to clear back to
    # An input binding never carries a default (empty = let the script ask), so it never
    # delivers empty either.
    assert field(input_binding=True, has_default=False).delivers_empty is False


# --------------------------------------------------------------------------
# 7) preset (persistable) keeps the default; last-used (remembered) drops it
# --------------------------------------------------------------------------


def _persist_plan() -> flows.FormPlan:
    return flows.FormPlan(
        source="inject",
        fields=[
            flows.FormField(
                key="GREETING",
                label="G",
                source="inject",
                kind="str",
                has_default=True,
                default="bonjour",
            ),
            flows.FormField(
                key="WIDTH", label="W", source="inject", kind="int", has_default=True, default="800"
            ),
        ],
    )


def test_last_used_drops_values_equal_to_their_default():
    plan = _persist_plan()
    values = {"GREETING": "bonjour", "WIDTH": "800"}  # both equal to their defaults
    # Last-used tracks the source: an untouched default is acceptance, not intent -> dropped,
    # so a later edit to the script's own literal is not shadowed by a value never chosen.
    # (Presets take the values verbatim instead — the deliberate way to pin one.)
    assert flows.remembered_values(plan, values) == {}


def test_last_used_keeps_a_cleared_empty_only_where_it_was_delivered():
    plan = _persist_plan()
    values = {"GREETING": "", "WIDTH": ""}  # both cleared
    # GREETING (str, has_default) delivered '' so it must replay as ''; WIDTH (int) did not
    # — there "" only ever meant "unset", and storing it would shadow a later default.
    assert flows.remembered_values(plan, values) == {"GREETING": ""}


def test_save_after_run_persists_via_the_remembered_rule(tmp_path):
    # save_after_run stores last-used through remembered_values: a value equal to the default
    # is dropped, a changed one is kept.
    plan = _persist_plan()
    flows.save_after_run(
        "rem",
        plan,
        {"GREETING": "bonjour", "WIDTH": "900"},
        [],
        0,
        at="2026-07-09T14:30:05+00:00",
    )
    state = argstate.load_state("rem")
    assert state["values"] == {"WIDTH": "900"}  # GREETING (== default) dropped; WIDTH kept


# --------------------------------------------------------------------------
# 8) FormField.input_binding tracks the ParamDecl binding
# --------------------------------------------------------------------------


def test_input_binding_flag_reflects_the_decl_binding():
    inp = ParamDecl(name="input-1", binding="input", delivery="inject", order=0, prompt="Name: ")
    assert flows.FormField.from_decl(inp).input_binding is True
    const = ParamDecl(name="X", binding="const", delivery="inject", type="str", default="v")
    assert flows.FormField.from_decl(const).input_binding is False
