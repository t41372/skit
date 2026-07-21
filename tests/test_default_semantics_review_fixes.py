"""Regression pins for the review fixes on the source-default change set.

tests/test_source_default_semantics.py pins the *feature* (the script's current literal
beats the stored block cache). This file pins the seven defects a code review found in
that change and the fixes that landed for them — each one a case where the "don't
re-deliver a value equal to the default" idea, taken one step too far, dropped a value
the form had already shown:

1. a SECRET whose source literal is empty still delivers its env-sourced value;
2. an input()-binding with a default still injects, so --no-input can't block on stdin;
3. a main-guard override still receives the injected value — in BOTH occurrences;
4. an envdefault default the declared type can no longer hold is not published
   (analysis._record_default's coercibility gate), so the form keeps a valid prefill;
5. a secret's source literal never reaches params/show --json (C3);
6. `preset save --from-last` works after a run that accepted every default;
7. presets store the run's values verbatim while last-used filters the defaults out;
8. a public-to-secret edit cannot copy the source literal into the secret block;
9. --from-last reads the exact historical snapshot, never today's source defaults;
10. shell colon env-default operators do not promise an empty value they cannot deliver.

Style mirrors tests/test_source_default_semantics.py (local _python_entry/_shell_entry
over a `# /// script` block, NOW datetime) and tests/test_show.py (typer CliRunner) for
the CLI-facing ones. Store/argstate isolation rides on conftest's autouse SKIT_*_DIR
fixture — these never touch real user directories and never chdir.
"""

from __future__ import annotations

import json
from datetime import datetime
from pathlib import Path
from typing import Any

from typer.testing import CliRunner

from skit import analysis, argstate, cli, flows, store
from skit.langs.python import metawriter, shim
from skit.langs.python.analyzer import analyze as py_analyze
from skit.langs.shell import analyzer as shell
from skit.models import Entry, ScriptMeta
from skit.params import ParamDecl

runner = CliRunner()

NOW = datetime(2026, 7, 9, 14, 30, 5)

# A secret const whose source literal is the empty string — the shape that made the
# "skip a value equal to its default" shortcut lose an env-sourced secret entirely.
SECRET_SCRIPT = """# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "API_KEY"
# kind = "const"
# type = "str"
# default = ""
# secret = true
# env_source = "MY_KEY"
# ///
API_KEY = ""
print(API_KEY)
"""  # noqa: S105

# An input() binding with a stored default: the value must be intercepted, or a
# --no-input run hangs on stdin forever.
INPUT_SCRIPT = """# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "input-1"
# kind = "input"
# type = "str"
# default = "Tim"
# order = 0
# prompt = "Your name? "
# ///
name = input("Your name? ")
print(name)
"""

# The top-level constant and its main-guard override. Injecting "localhost" over
# "localhost" looks like a no-op — it is not: the guard body says 127.0.0.1.
MAIN_GUARD_SCRIPT = """# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "HOST"
# kind = "const"
# type = "str"
# default = "localhost"
# ///
HOST = "localhost"

if __name__ == "__main__":
    HOST = "127.0.0.1"
    print(HOST)
"""

SHELL_ENVDEFAULT_BLOCK = """#!/usr/bin/env bash
# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "PORT"
# kind = "envdefault"
# type = "int"
# default = 8080
# ///
PORT=${PORT:-8080}
echo "$PORT"
"""

SECRET_LITERAL = "sk-live-SUPERSECRET"  # noqa: S105


def _python_entry(tmp_path: Path, text: str, slug: str = "s") -> Entry:
    d = tmp_path / "store" / slug
    d.mkdir(parents=True)
    (d / "script.py").write_text(text, encoding="utf-8")
    meta = ScriptMeta(name=slug, kind="python", mode="copy", source=str(tmp_path / "orig.py"))
    return Entry(slug=slug, meta=meta, dir=d)


def _shell_entry(tmp_path: Path, text: str, slug: str = "sh") -> Entry:
    d = tmp_path / "store" / slug
    d.mkdir(parents=True)
    (d / "script.sh").write_text(text, encoding="utf-8")
    meta = ScriptMeta(name=slug, kind="shell", mode="copy", source=str(tmp_path / "orig.sh"))
    return Entry(slug=slug, meta=meta, dir=d)


def _envdefault(name: str, type_: str = "str", default: object = None) -> ParamDecl:
    # ParamDecl.type/default are closed-literal typed; the test drives them by value.
    return ParamDecl(
        name=name,
        binding="envdefault",
        delivery="env",
        type=type_,  # ty: ignore[invalid-argument-type]
        default=default,  # ty: ignore[invalid-argument-type]
    )


def _managed(tmp_path: Path, body: str, specs: list[ParamDecl], name: str) -> Entry:
    """Add a real store entry whose stored copy carries a [tool.skit] block."""
    path = tmp_path / f"{name}.py"
    path.write_text(metawriter.write_params(body, specs), encoding="utf-8")
    return store.add_python(path, name=name)


def _json(args: list[str]) -> tuple[str, dict[str, Any]]:
    result = runner.invoke(cli.app, args)
    assert result.exit_code == 0, result.output
    return result.output, json.loads(result.output)


# --------------------------------------------------------------------------
# 1) a secret's empty source literal must not cancel its delivery
# --------------------------------------------------------------------------


def test_secret_with_an_empty_source_literal_is_still_delivered(tmp_path: Path):
    # `API_KEY = ""` is the canonical secret placeholder: the literal is empty ON PURPOSE
    # and the real value arrives from $MY_KEY. A secret's field is never prefilled, so its
    # raw text is always "" — exactly equal to the recorded default. An earlier revision
    # skipped delivery whenever raw == default, which silently dropped every env-sourced
    # secret and ran the script with its empty placeholder.
    entry = _python_entry(tmp_path, SECRET_SCRIPT, slug="sec")
    plan = flows.plan_for_entry(entry)
    (field,) = plan.fields
    assert (field.key, field.secret, field.env_source) == ("API_KEY", True, "MY_KEY")
    assert field.default == ""  # the block's own empty placeholder; secrets are not prefilled
    values = flows.prefill(plan, entry.slug)
    assert values == {}  # C3: a secret is never prefilled
    asm = flows.assemble(plan, values, [], cwd=tmp_path, env={"MY_KEY": "sk-live-XYZ"}, now=NOW)
    assert asm.inject_values == {"API_KEY": "sk-live-XYZ"}
    # ... and the transparency line still masks it.
    assert asm.display == [("API_KEY", "•••")]


def test_secret_field_never_delivers_empty(tmp_path: Path):
    # The companion rule that makes the case above safe: a secret is NOT a delivers-empty
    # field, so an unset env source is a named error rather than an injected ''.
    plan = flows.plan_for_entry(_python_entry(tmp_path, SECRET_SCRIPT, slug="sec2"))
    (field,) = plan.fields
    assert field.delivers_empty is False


# --------------------------------------------------------------------------
# 2) an input() binding with a default must still be intercepted
# --------------------------------------------------------------------------


def test_input_binding_with_a_default_is_delivered(tmp_path: Path):
    # An input() binding's value is what REPLACES the interactive question. If a value
    # equal to the default were skipped, the injected copy would keep the real input()
    # call and the script would block on stdin — under `--no-input`, forever.
    entry = _python_entry(tmp_path, INPUT_SCRIPT, slug="inp")
    plan = flows.plan_for_entry(entry)
    (field,) = plan.fields
    assert (field.key, field.input_binding, field.has_default) == ("input-1", True, True)
    assert plan.drift_lines == []  # the stored prompt/order still resolve
    values = flows.prefill(plan, entry.slug)
    assert values == {"input-1": "Tim"}
    asm = flows.assemble(plan, values, [], cwd=tmp_path, env={}, now=NOW)
    assert asm.inject_values == {"input-1": "Tim"}
    # The shim actually rewrites the call site, so nothing is left to read stdin.
    injected = shim.inject(plan.text, plan.specs, asm.inject_values)
    assert "_skit_i[0](" in injected
    assert 'input("Your name? ")' not in injected


# --------------------------------------------------------------------------
# 3) a main-guard override must receive the injected value too
# --------------------------------------------------------------------------


def test_main_guard_override_receives_the_unchanged_default(tmp_path: Path):
    # The form shows HOST = localhost. Submitting it unchanged still injects — and the
    # point of injecting is the SECOND occurrence: the main-guard body reassigns HOST to
    # 127.0.0.1, so "skip a value equal to the default" would have run the script on a
    # host the form (and the transparency line) denied.
    entry = _python_entry(tmp_path, MAIN_GUARD_SCRIPT, slug="guard")
    plan = flows.plan_for_entry(entry)
    values = flows.prefill(plan, entry.slug)
    assert values == {"HOST": "localhost"}
    asm = flows.assemble(plan, values, [], cwd=tmp_path, env={}, now=NOW)
    assert asm.inject_values == {"HOST": "localhost"}
    injected = shim.inject(MAIN_GUARD_SCRIPT, plan.specs, {"HOST": "localhost"})
    assert injected.count("HOST = 'localhost'") == 2  # top level AND the guard body
    assert "127.0.0.1" not in injected  # the override is gone; the run matches the form


# --------------------------------------------------------------------------
# 4) an unfit envdefault default is not published (the coercibility gate)
# --------------------------------------------------------------------------


def test_envdefault_default_that_no_longer_fits_the_type_is_not_published(tmp_path: Path):
    # An envdefault stays `ok` through a type change (the value arrives by environment
    # either way), so reconcile keeps delivering it. But its SOURCE default may now be
    # text an int param cannot hold: `${PORT:-$FALLBACK}` reads back as the str
    # "$FALLBACK". Publishing that would prefill an int field with "$FALLBACK" — the form
    # opens in error and `--no-input` exits 125 on a script nobody changed the type of.
    text = SHELL_ENVDEFAULT_BLOCK.replace("PORT=${PORT:-8080}", "PORT=${PORT:-$FALLBACK}")
    entry = _shell_entry(tmp_path, text, slug="unfit")
    specs = [_envdefault("PORT", "int", 8080)]
    report = shell.reconcile(text, specs)
    assert [s.name for s in report.ok] == ["PORT"]  # env delivery survives the type change
    assert report.current_defaults == {}  # ... but the unfit default is withheld

    # Consequence: the form keeps the block's own int-valid prefill and opens clean.
    plan = flows.plan_for_entry(entry)
    (field,) = plan.fields
    assert (field.key, field.kind, field.default) == ("PORT", "int", "8080")
    assert flows.validate(plan, flows.prefill(plan, entry.slug)) == {}


def test_int_shaped_literal_still_refreshes_a_str_envdefault():
    # The positive twin: fitness is COERCIBILITY, not type equality. The analyzers type a
    # literal by its shape, so a `str` param defaulting to 8080 reads back as an int
    # candidate — and must still refresh, because its value is text either way.
    report = shell.reconcile('PORT=${PORT:-8080}\necho "$PORT"\n', [_envdefault("PORT", "str")])
    assert [s.name for s in report.ok] == ["PORT"]
    assert report.current_defaults == {"PORT": 8080}


def test_const_default_that_no_longer_fits_the_declared_type_is_not_published():
    # The const lane shares the gate, even though a real analyzer can't reach it: a const
    # whose derived type stops matching lands in report.changed (drift), which never calls
    # _record_default at all. So drive the residual case through a synthetic analyze — types
    # that agree (int/int) over a literal the type still cannot hold.
    def analyze(_text: str) -> analysis.Analysis:
        return analysis.Analysis(
            candidates=[analysis.Candidate(binding="const", name="N", type="int", default="three")]
        )

    report = analysis.reconcile(
        "_\n", [ParamDecl(name="N", binding="const", type="int", default=3)], analyze=analyze
    )
    assert [s.name for s in report.ok] == ["N"]  # types agree, so this is not drift
    assert report.current_defaults == {}  # ... but "three" is not an int


# --------------------------------------------------------------------------
# 5) C3: a secret's source literal never reaches a machine-facing surface
# --------------------------------------------------------------------------


def test_secret_source_literal_is_absent_from_reconcile_and_json(tmp_path: Path):
    # current_defaults feeds `params --json`, `show --json` and the settings pane — none of
    # which mask anything. Publishing a secret's literal there would take a hardcoded
    # `TOKEN = "sk-live-…"` out of the script's own text for the first time.
    body = f'TOKEN = "{SECRET_LITERAL}"\nprint(TOKEN)\n'
    specs = [ParamDecl(name="TOKEN", binding="const", type="str", secret=True)]
    assert (
        analysis.reconcile(
            metawriter.write_params(body, specs), specs, analyze=py_analyze
        ).current_defaults
        == {}
    )

    _managed(tmp_path, body, specs, name="tok")
    raw_params, params_payload = _json(["params", "tok", "--json"])
    assert params_payload["current_defaults"] == {}
    (param,) = params_payload["params"]
    assert param["name"] == "TOKEN"
    assert param["secret"] is True
    assert "default" not in param  # no block default was ever declared
    assert SECRET_LITERAL not in raw_params

    raw_show, show_payload = _json(["show", "tok", "--json"])
    (shown,) = show_payload["fields"]
    assert (shown["key"], shown["secret"]) == ("TOKEN", True)
    assert shown["default"] is None
    assert SECRET_LITERAL not in raw_show


# --------------------------------------------------------------------------
# 6) preset save --from-last after a run that accepted every default
# --------------------------------------------------------------------------


def test_preset_from_last_saves_effective_values_after_an_all_defaults_run(tmp_path: Path):
    # Last-used deliberately stores only what DIFFERED from the defaults, so a run that
    # accepted everything leaves the [values] table empty. Reading that table directly made
    # --from-last refuse a perfectly good preset ("no remembered values yet") right after a
    # successful run. The gate now asks the honest question — has this entry anything to
    # remember at all — and saves the EFFECTIVE values (definition default < last-used).
    body = 'GREETING = "bonjour"\nprint(GREETING)\n'
    specs = [ParamDecl(name="GREETING", binding="const", type="str", default="bonjour")]
    entry = _managed(tmp_path, body, specs, name="greet")
    plan = flows.plan_for_entry(entry)
    values = flows.prefill(plan, entry.slug)
    assert values == {"GREETING": "bonjour"}
    flows.save_after_run(entry.slug, plan, values, [], 0, at="2026-07-09T14:30:05+00:00")
    state = argstate.load_state(entry.slug)
    assert state["values"] == {}  # nothing differed from the default
    assert state["last_run"]["exit"] == 0  # ... but the run really happened

    result = runner.invoke(cli.app, ["preset", "save", "greet", "p", "--from-last"])
    assert result.exit_code == 0, result.output
    assert argstate.load_state(entry.slug)["presets"] == {"p": {"GREETING": "bonjour"}}


def test_preset_from_last_still_refuses_an_entry_that_never_ran(tmp_path: Path):
    # The honest refusal survives the fix: no last_run AND no remembered values means there
    # is genuinely nothing to save, and the message says exactly that.
    body = 'GREETING = "bonjour"\nprint(GREETING)\n'
    specs = [ParamDecl(name="GREETING", binding="const", type="str", default="bonjour")]
    entry = _managed(tmp_path, body, specs, name="fresh")
    result = runner.invoke(cli.app, ["preset", "save", "fresh", "p", "--from-last"])
    assert result.exit_code == 1, result.output
    assert "no remembered values yet" in result.output
    assert argstate.load_state(entry.slug)["presets"] == {}


def test_preset_from_last_pins_the_default_that_actually_ran(tmp_path: Path):
    body = 'GREETING = "A"\nprint(GREETING)\n'
    specs = [ParamDecl(name="GREETING", binding="const", type="str", default="A")]
    entry = _managed(tmp_path, body, specs, name="history")
    plan = flows.plan_for_entry(entry)
    values = flows.prefill(plan, entry.slug)
    flows.save_after_run(entry.slug, plan, values, [], 0, at="2026-07-09T14:30:05+00:00")

    current = entry.script_path.read_text(encoding="utf-8")
    entry.script_path.write_text(current.replace('GREETING = "A"', 'GREETING = "B"'))
    assert flows.prefill(flows.plan_for_entry(entry), entry.slug) == {"GREETING": "B"}

    result = runner.invoke(cli.app, ["preset", "save", "history", "p", "--from-last"])
    assert result.exit_code == 0, result.output
    assert argstate.load_state(entry.slug)["presets"] == {"p": {"GREETING": "A"}}


def test_preset_from_legacy_run_without_snapshot_refuses_to_guess(tmp_path: Path):
    entry = _managed(
        tmp_path,
        'GREETING = "B"\nprint(GREETING)\n',
        [ParamDecl(name="GREETING", binding="const", type="str", default="B")],
        name="legacy-history",
    )
    argstate.record_run(entry.slug, 0, at="2026-07-09T14:30:05+00:00")
    result = runner.invoke(cli.app, ["preset", "save", "legacy-history", "p", "--from-last"])
    assert result.exit_code == 1
    assert "run it once first" in result.output
    assert argstate.load_state(entry.slug)["presets"] == {}


# --------------------------------------------------------------------------
# 7) presets pin what ran; last-used filters
# --------------------------------------------------------------------------


def _defaulted_plan() -> flows.FormPlan:
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
            )
        ],
    )


def test_last_used_filters_the_default_but_keeps_a_delivered_empty():
    plan = _defaulted_plan()
    # Accepting a default is not a choice: remembering it would freeze today's default and
    # hide tomorrow's edit to the script.
    assert flows.remembered_values(plan, {"GREETING": "bonjour"}) == {}
    # A cleared delivers-empty field, by contrast, WAS delivered as '' and must replay.
    assert flows.remembered_values(plan, {"GREETING": ""}) == {"GREETING": ""}
    assert plan.fields[0].delivers_empty is True


def test_run_save_preset_stores_a_default_equal_value_verbatim(tmp_path: Path):
    # The deliberate counterpart of the filter above: a preset is the named way to PIN a
    # value, so it stores the run's values verbatim — including one that happens to equal
    # today's default. (--dry-run keeps this hermetic; the preset write is on the same path.)
    body = 'GREETING = "bonjour"\nprint(GREETING)\n'
    specs = [ParamDecl(name="GREETING", binding="const", type="str", default="bonjour")]
    entry = _managed(tmp_path, body, specs, name="pinned")
    result = runner.invoke(
        cli.app,
        [
            "run",
            "pinned",
            "--set",
            "GREETING=bonjour",
            "--save-preset",
            "p",
            "--no-input",
            "--dry-run",
        ],
    )
    assert result.exit_code == 0, result.output
    state = argstate.load_state(entry.slug)
    assert state["presets"] == {"p": {"GREETING": "bonjour"}}  # verbatim: the default is pinned
    assert state["values"] == {}  # last-used still filtered it out


# --------------------------------------------------------------------------
# 8) public -> secret never caches the source literal
# --------------------------------------------------------------------------


def test_resync_and_secret_in_one_edit_drops_the_refreshed_literal():
    spec = ParamDecl(name="CITY", binding="const", type="str", default="old")
    result = analysis.edit_specs(
        'CITY = "sk-live-source"\n',
        [spec],
        resync=True,
        secret=["CITY"],
        analyze=py_analyze,
    )
    (edited,) = result.specs
    assert edited.secret is True
    assert edited.default is None
    assert "default" not in edited.to_block_dict()
    assert "sk-live-source" not in repr(edited.to_block_dict())


def test_final_no_secret_in_same_edit_keeps_the_public_default():
    spec = ParamDecl(name="CITY", binding="const", type="str", default="old")
    result = analysis.edit_specs(
        'CITY = "new"\n',
        [spec],
        resync=True,
        secret=["CITY"],
        no_secret=["CITY"],
        analyze=py_analyze,
    )
    (edited,) = result.specs
    assert edited.secret is False
    assert edited.default == "new"


# --------------------------------------------------------------------------
# 10) shell colon operators treat empty as unset
# --------------------------------------------------------------------------


def _shell_envdefault_text(operator: str) -> str:
    return f"""#!/usr/bin/env bash
# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "CITY"
# kind = "envdefault"
# type = "str"
# default = "Taipei"
# ///
echo "${{CITY{operator}Taipei}}"
"""


def test_shell_colon_envdefaults_do_not_claim_to_deliver_empty(tmp_path: Path):
    for i, operator in enumerate((":-", ":=")):
        plan = flows.plan_for_entry(
            _shell_entry(tmp_path, _shell_envdefault_text(operator), slug=f"colon-{i}")
        )
        (field,) = plan.fields
        assert field.empty_uses_default is True
        assert field.delivers_empty is False
        asm = flows.assemble(plan, {"CITY": ""}, [], cwd=tmp_path, env={})
        assert asm.env_values == {}


def test_shell_noncolon_envdefaults_genuinely_deliver_empty(tmp_path: Path):
    for i, operator in enumerate(("-", "=")):
        plan = flows.plan_for_entry(
            _shell_entry(tmp_path, _shell_envdefault_text(operator), slug=f"plain-{i}")
        )
        (field,) = plan.fields
        assert field.empty_uses_default is False
        assert field.delivers_empty is True
        asm = flows.assemble(plan, {"CITY": ""}, [], cwd=tmp_path, env={})
        assert asm.env_values == {"CITY": ""}
