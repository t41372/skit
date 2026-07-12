"""Unified form layer: plans, prefill order, validation, assembly, run recording."""

from __future__ import annotations

import os
from datetime import datetime
from pathlib import Path

import pytest

from skit import argstate, flows
from skit.langs.python import metawriter as _metawriter
from skit.models import Entry, ScriptMeta
from skit.params import ParamDecl, ParamType


def metawriter_write(text: str, params: list[tuple[str, ParamType]]) -> str:
    specs = [ParamDecl(name=n, binding="const", type=t) for n, t in params]
    return _metawriter.write_params(text, specs)


NOW = datetime(2026, 7, 9, 14, 30, 5)

ARGPARSE_SCRIPT = """
import argparse
ap = argparse.ArgumentParser()
ap.add_argument("inputs", nargs="+", help="input files")
ap.add_argument("-o", "--output", required=True, help="output path")
ap.add_argument("--gap", type=int, default=0)
ap.add_argument("--mode", choices=["a", "b"], default="a")
ap.add_argument("--fast", action="store_true")
ap.add_argument("--bg", type=custom, help="color")
args = ap.parse_args()
"""

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


def _command_entry(slug: str = "c") -> Entry:
    meta = ScriptMeta(name=slug, kind="command", template="echo {msg}", params=["msg"])
    return Entry(slug=slug, meta=meta, dir=Path("/nonexistent"))


# --------------------------------------------------------------------------
# plans
# --------------------------------------------------------------------------


def test_plan_managed_script_is_inject(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT))
    assert plan.source == "inject"
    assert [f.key for f in plan.fields] == ["OUTPUT", "WIDTH", "API_KEY"]
    assert plan.fields[1].kind == "int"
    assert plan.fields[2].secret is True
    assert plan.fields[2].env_source == "MY_API_KEY"


def test_plan_argparse_script(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    assert plan.source == "argparse"
    assert [f.key for f in plan.fields] == ["inputs", "output", "gap", "mode", "fast", "bg"]
    assert plan.fields[0].multiple is True
    assert plan.fields[5].degraded is True
    assert plan.text == ARGPARSE_SCRIPT  # the delivery layer reuses exactly this script text


def test_plan_plain_script_is_none(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, "print('hi')\n"))
    assert plan.source == "none"
    assert plan.fields == []
    assert plan.text == "print('hi')\n"  # a readable-but-fieldless script still carries its text


def test_plan_command_entry_placeholders():
    plan = flows.plan_for_entry(_command_entry())
    assert plan.source == "command"
    assert [f.key for f in plan.fields] == ["msg"]
    assert [f.label for f in plan.fields] == ["msg"]  # a placeholder's label is its own name


def test_plan_managed_wins_over_argparse(tmp_path):
    text = (
        MANAGED_SCRIPT
        + "\nimport argparse\nap = argparse.ArgumentParser()\nap.add_argument('--x')\nap.parse_args()\n"
    )
    plan = flows.plan_for_entry(_python_entry(tmp_path, text))
    assert plan.source == "inject"


def test_plan_missing_script_is_none(tmp_path):
    entry = _python_entry(tmp_path, "print(1)\n")
    entry.script_path.unlink()
    assert flows.plan_for_entry(entry).source == "none"


# --------------------------------------------------------------------------
# prefill
# --------------------------------------------------------------------------


def test_prefill_default_then_last_then_preset(tmp_path):
    entry = _python_entry(tmp_path, MANAGED_SCRIPT)
    plan = flows.plan_for_entry(entry)
    assert flows.prefill(plan, "s")["OUTPUT"] == "out.jpg"  # definition default
    argstate.save_last("s", values={"OUTPUT": "last.jpg"})
    assert flows.prefill(plan, "s")["OUTPUT"] == "last.jpg"  # last wins over default
    argstate.save_preset("s", "web", {"OUTPUT": "web.jpg"})
    assert flows.prefill(plan, "s", preset="web")["OUTPUT"] == "web.jpg"  # preset wins
    assert flows.prefill(plan, "s")["OUTPUT"] == "last.jpg"  # no preset asked -> last


def test_prefill_never_offers_secrets(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT))
    values = flows.prefill(plan, "s")
    assert "API_KEY" not in values  # even though the definition has a default


# --------------------------------------------------------------------------
# validation
# --------------------------------------------------------------------------


def test_validate_required_empty(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    errors = flows.validate(plan, {})
    assert set(errors) == {"inputs", "output"}


def test_validate_int_error_names_field_and_value(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    errors = flows.validate(plan, {"inputs": "a", "output": "o", "gap": "abc"})
    assert "gap" in errors
    assert "abc" in errors["gap"]


def test_validate_choice(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    errors = flows.validate(plan, {"inputs": "a", "output": "o", "mode": "zzz"})
    assert "mode" in errors
    assert "a, b" in errors["mode"]


def test_validate_token_values_deferred(tmp_path):
    # "{env:N}" can't be type-checked before expansion; validate defers to assembly.
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    errors = flows.validate(plan, {"inputs": "a", "output": "o", "gap": "{env:GAP}"})
    assert "gap" not in errors


# --------------------------------------------------------------------------
# assembly
# --------------------------------------------------------------------------


def _values_ok() -> dict[str, str]:
    return {"inputs": "a.png", "output": "o.png", "gap": "0", "mode": "a", "fast": "false"}


def test_assemble_argparse_positionals_then_flags(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    asm = flows.assemble(
        plan,
        {"inputs": "a.png b.png", "output": "o.png", "gap": "4", "mode": "b", "fast": "true"},
        [],
        cwd=tmp_path,
        env={},
        now=NOW,
    )
    assert asm.args == [
        "a.png",
        "b.png",
        "--output",
        "o.png",
        "--gap",
        "4",
        "--mode",
        "b",
        "--fast",
    ]


def test_assemble_unchecked_store_true_omits_flag(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    asm = flows.assemble(plan, _values_ok(), [], cwd=tmp_path, env={}, now=NOW)
    assert "--fast" not in asm.args


def test_assemble_degraded_empty_omitted_filled_passed(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    asm = flows.assemble(plan, _values_ok(), [], cwd=tmp_path, env={}, now=NOW)
    assert "--bg" not in asm.args
    asm2 = flows.assemble(plan, {**_values_ok(), "bg": "#fff"}, [], cwd=tmp_path, env={}, now=NOW)
    assert asm2.args[-2:] == ["--bg", "#fff"]


def test_assemble_glob_expands_multiple_fields_against_cwd(tmp_path):
    (tmp_path / "shots").mkdir()
    for n in ("2.png", "1.png"):
        (tmp_path / "shots" / n).touch()
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    asm = flows.assemble(
        plan, {**_values_ok(), "inputs": "shots/*.png"}, [], cwd=tmp_path, env={}, now=NOW
    )
    # glob returns platform-native separators (backslashes on Windows); build the expectation
    # with os.path.join so it matches on every OS.
    assert asm.args[:2] == [os.path.join("shots", "1.png"), os.path.join("shots", "2.png")]


def test_assemble_glob_without_match_keeps_literal(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    asm = flows.assemble(
        plan, {**_values_ok(), "inputs": "none/*.xyz"}, [], cwd=tmp_path, env={}, now=NOW
    )
    assert asm.args[0] == "none/*.xyz"


def test_assemble_tokens_expand_and_type_check_after_expansion(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    asm = flows.assemble(
        plan,
        {**_values_ok(), "output": "out_{today}.png", "gap": "{env:GAP}"},
        [],
        cwd=tmp_path,
        env={"GAP": "8"},
        now=NOW,
    )
    assert "out_2026-07-09.png" in asm.args
    assert "8" in asm.args
    with pytest.raises(flows.FormError) as exc:
        flows.assemble(
            plan,
            {**_values_ok(), "gap": "{env:GAP}"},
            [],
            cwd=tmp_path,
            env={"GAP": "not-a-number"},
            now=NOW,
        )
    assert "not-a-number" in str(exc.value)


def test_assemble_missing_env_token_is_named_error(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    with pytest.raises(flows.FormError) as exc:
        flows.assemble(
            plan, {**_values_ok(), "output": "{env:NOPE}"}, [], cwd=tmp_path, env={}, now=NOW
        )
    assert "NOPE" in str(exc.value)


def test_assemble_inject_values_expanded_and_masked_display(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT))
    asm = flows.assemble(
        plan,
        {"OUTPUT": "long_{today}.jpg", "WIDTH": "800", "API_KEY": "typed-secret"},
        [],
        cwd=tmp_path,
        env={},
        now=NOW,
    )
    assert asm.inject_values["OUTPUT"] == "long_2026-07-09.jpg"
    assert ("API_KEY", "•••") in asm.display
    assert all(v != "typed-secret" for _k, v in asm.display)


def test_assemble_secret_env_source_reads_environment(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT))
    asm = flows.assemble(
        plan,
        {"OUTPUT": "o.jpg", "WIDTH": "1", "API_KEY": ""},
        [],
        cwd=tmp_path,
        env={"MY_API_KEY": "from-env"},
        now=NOW,
    )
    assert asm.inject_values["API_KEY"] == "from-env"


def test_assemble_secret_env_source_missing_is_named_error(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT))
    with pytest.raises(flows.FormError) as exc:
        flows.assemble(
            plan, {"OUTPUT": "o", "WIDTH": "1", "API_KEY": ""}, [], cwd=tmp_path, env={}, now=NOW
        )
    # Pin the exact sentence (both the field label and the env-var name), so a corrupted message
    # string can't survive behind a bare substring check.
    assert str(exc.value) == (
        "API_KEY reads from the environment variable MY_API_KEY, but it isn't set."
    )


def test_assemble_typed_secret_beats_env_source(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT))
    asm = flows.assemble(
        plan,
        {"OUTPUT": "o", "WIDTH": "1", "API_KEY": "typed"},
        [],
        cwd=tmp_path,
        env={"MY_API_KEY": "env"},
        now=NOW,
    )
    assert asm.inject_values["API_KEY"] == "typed"


def test_assemble_command_values_and_extra_args(tmp_path):
    plan = flows.plan_for_entry(_command_entry())
    asm = flows.assemble(plan, {"msg": "hi {today}"}, ["--verbose"], cwd=tmp_path, env={}, now=NOW)
    assert asm.command_values == {"msg": "hi 2026-07-09"}
    assert asm.args == ["--verbose"]
    assert asm.masked_args == ["--verbose"]


def test_assemble_extra_args_expand_tokens_and_globs(tmp_path):
    (tmp_path / "x1.txt").touch()
    (tmp_path / "x2.txt").touch()
    plan = flows.FormPlan(source="none")
    # Every token forwarded into the extra-args expansion is pinned here: glob, {today}, {now}
    # (not the wall clock), {cwd} (not "None"), and {env:...} (from the passed env, not os.environ).
    asm = flows.assemble(
        plan,
        {},
        ["x*.txt", "{today}", "{now}", "{cwd}", "{env:XV}"],
        cwd=tmp_path,
        env={"XV": "envval"},
        now=NOW,
    )
    assert asm.args == [
        "x1.txt",
        "x2.txt",
        "2026-07-09",
        "14-30-05",
        str(tmp_path),
        "envval",
    ]


def test_assemble_extra_arg_token_error_forwards_the_token_message(tmp_path):
    # A failed token in an extra arg surfaces the TokenError's own message, not a dropped/None one.
    plan = flows.FormPlan(source="none")
    with pytest.raises(flows.FormError) as exc:
        flows.assemble(plan, {}, ["{env:NOPE_EXTRA}"], cwd=tmp_path, env={}, now=NOW)
    assert "NOPE_EXTRA" in str(exc.value)


def test_assemble_inject_source_forwards_extra_args(tmp_path):
    # The inject delivery still carries the extra-args escape hatch through to argv.
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="ie"))
    asm = flows.assemble(
        plan,
        {"OUTPUT": "o", "WIDTH": "1", "API_KEY": "k"},
        ["--flag", "v"],
        cwd=tmp_path,
        env={},
        now=NOW,
    )
    assert asm.args == ["--flag", "v"]


def test_assemble_field_expands_cwd_and_now_tokens(tmp_path):
    # A field value's {cwd}/{now} tokens must expand against the RUN's cwd/now (forwarded all the
    # way through assemble -> _final_value -> tokens.expand), not None or the wall clock.
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT, slug="cn"))
    asm = flows.assemble(
        plan, {**_values_ok(), "output": "{cwd}/{now}.png"}, [], cwd=tmp_path, env={}, now=NOW
    )
    assert f"{tmp_path}/14-30-05.png" in asm.args


def test_assemble_does_not_retypecheck_plain_values(tmp_path):
    # Plain (token-free) values were already checked by pre-submit validate(); assemble only
    # re-checks values that carried tokens. A plain value that would fail a type check therefore
    # passes straight through instead of raising (kills the `and`->`or` gate).
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT, slug="pv"))
    asm = flows.assemble(plan, {**_values_ok(), "gap": "abc"}, [], cwd=tmp_path, env={}, now=NOW)
    assert asm.args[asm.args.index("--gap") + 1] == "abc"


def test_assemble_defaults_env_to_os_environ(monkeypatch, tmp_path):
    # With no env kwarg, assemble reads the process environment. A genuinely unset secret source
    # then raises the named FormError — never a TypeError from a None environment.
    monkeypatch.delenv("MY_API_KEY", raising=False)
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="oe"))
    with pytest.raises(flows.FormError):
        flows.assemble(
            plan, {"OUTPUT": "o", "WIDTH": "1", "API_KEY": ""}, [], cwd=tmp_path, now=NOW
        )


def test_assemble_flags_tolerates_missing_keys(tmp_path):
    # _assemble_flags defends against a `final` lacking keys (a preset saved before a field
    # existed): a missing bool reads as unchecked (not a crash), a missing value field is omitted
    # (not injected as a sentinel). Exercised directly since assemble always fills every key.
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT, slug="mk"))
    result = flows._assemble_flags(plan, {"inputs": "a", "output": "o"}, tmp_path)
    assert result == ["a", "--output", "o"]


def test_assemble_empty_field_does_not_stop_later_flags(tmp_path):
    # An empty optional field is skipped, not a hard stop: a later filled field must still be
    # assembled (kills continue->break).
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT, slug="cont"))
    asm = flows.assemble(plan, {**_values_ok(), "gap": ""}, [], cwd=tmp_path, env={}, now=NOW)
    assert "--mode" in asm.args  # gap (earlier) empty must not drop mode (later)


def test_split_multi_falls_back_on_unbalanced_quote(tmp_path):
    # A multi-value field whose text can't be shlex-split (unbalanced quote) falls back to the
    # whole raw value instead of crashing.
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT, slug="sm"))
    asm = flows.assemble(plan, {**_values_ok(), "inputs": 'a"b'}, [], cwd=tmp_path, env={}, now=NOW)
    assert asm.args[0] == 'a"b'


def test_resolve_secret_empty_when_no_input_and_no_env_source():
    # A secret with neither a typed value nor an env_source resolves to "" (nothing delivered),
    # never a placeholder string.
    f = flows.FormField(key="k", label="k", secret=True)
    assert flows._resolve_secret(f, "", {}) == ""


def test_validate_value_accepts_a_valid_choice():
    # A value that IS among the choices validates clean: the choice guard fires only on mismatch
    # (kills the `or`-flattened guard that would reject every choice value).
    choice_field = flows.FormField(key="m", label="mode", kind="choice", choices=["a", "b"])
    assert flows.validate_value(choice_field, "a") is None
    assert flows.validate_value(choice_field, "b") is None


def test_prefill_drops_a_secret_that_leaked_into_saved_values(tmp_path):
    # Defense in depth: even if a secret key sits in saved values, prefill must not surface it
    # (the `k not in secret` half of the filter). save_last without secret_names lets it reach disk.
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="ps"))
    argstate.save_last("ps", values={"OUTPUT": "o.jpg", "API_KEY": "leaked"})
    values = flows.prefill(plan, "ps")
    assert values["OUTPUT"] == "o.jpg"
    assert "API_KEY" not in values


def test_prefill_preset_drops_leaked_secret(tmp_path):
    # Same guard, preset branch.
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="pp"))
    argstate.save_preset("pp", "web", {"OUTPUT": "web.jpg", "API_KEY": "leaked"})
    values = flows.prefill(plan, "pp", preset="web")
    assert values["OUTPUT"] == "web.jpg"
    assert "API_KEY" not in values


def test_prefill_unknown_preset_is_no_op_not_a_crash(tmp_path):
    # A never-saved preset yields an empty overlay, not a crash: pins the {} default on the presets
    # lookup (None would blow up on .items()).
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="up"))
    values = flows.prefill(plan, "up", preset="ghost")
    assert values["OUTPUT"] == "out.jpg"


# --------------------------------------------------------------------------
# glob feedback + run recording
# --------------------------------------------------------------------------


def test_glob_feedback_counts(tmp_path):
    (tmp_path / "a.png").touch()
    (tmp_path / "b.png").touch()
    assert flows.glob_feedback("*.png", tmp_path) == 2
    assert flows.glob_feedback("*.png extra.txt", tmp_path) == 3  # non-glob piece counts as 1
    # Two *glob* pieces: the running total must ACCUMULATE, not overwrite (2 + 2 = 4). A single
    # glob piece can't tell `count += n` from `count = n`; two of them can.
    assert flows.glob_feedback("*.png ?.png", tmp_path) == 4
    assert flows.glob_feedback("plain.txt", tmp_path) is None


def test_save_after_run_persists_intent_and_stamps_run(tmp_path):
    entry = _python_entry(tmp_path, MANAGED_SCRIPT)
    plan = flows.plan_for_entry(entry)
    flows.save_after_run(
        "s",
        plan,
        {"OUTPUT": "long_{today}.jpg", "WIDTH": "800", "API_KEY": "secret!"},
        ["--fast"],
        0,
        at="2026-07-09T14:30:05+00:00",
    )
    state = argstate.load_state("s")
    assert state["values"]["OUTPUT"] == "long_{today}.jpg"  # raw token text, not expansion
    assert "API_KEY" not in state["values"]  # C3
    assert state["extra_args"] == ["--fast"]
    assert state["last_run"] == {"at": "2026-07-09T14:30:05+00:00", "exit": 0}


def test_record_run_zero_exit_survives_save(tmp_path):
    argstate.record_run("z", 0, at="2026-07-09T00:00:00+00:00")
    assert argstate.load_state("z")["last_run"]["exit"] == 0


# --------------------------------------------------------------------------
# mutation hardening: pin the exact contracts of the small helpers
# --------------------------------------------------------------------------


def test_coerce_bool_lenient_accepts_every_truthy_spelling():
    for spelling in ("true", "1", "yes", "y", "on", " TRUE ", "On"):
        assert flows._coerce_bool_lenient(spelling) is True, spelling
    for spelling in ("false", "0", "no", "n", "off", "", "garbage"):
        assert flows._coerce_bool_lenient(spelling) is False, spelling


def test_expand_glob_piece_globs_only_when_glob_chars_present(tmp_path):
    (tmp_path / "ax").touch()
    (tmp_path / "bx").touch()
    # A piece containing every glob char must expand (kills the inverted-any mutant,
    # which only misbehaves when *, ?, and [ are all present).
    assert flows._expand_glob_piece("[ab]?*", tmp_path) == ["ax", "bx"]
    # A plain filename passes through untouched even when a same-named file exists.
    assert flows._expand_glob_piece("ax", tmp_path) == ["ax"]


def test_expand_glob_piece_supports_recursive_doublestar(tmp_path):
    nested = tmp_path / "deep" / "deeper"
    nested.mkdir(parents=True)
    (nested / "x.txt").touch()
    assert flows._expand_glob_piece("**/x.txt", tmp_path) == [
        os.path.join("deep", "deeper", "x.txt")
    ]


def test_assemble_tolerates_a_bool_field_missing_from_values(tmp_path):
    # A values dict that never mentions the checkbox (e.g. a preset saved before the
    # flag existed) must behave as unchecked, not crash.
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT))
    values = {k: v for k, v in _values_ok().items() if k != "fast"}
    asm = flows.assemble(plan, values, [], cwd=tmp_path, env={}, now=NOW)
    assert "--fast" not in asm.args


def test_assemble_store_false_fires_flag_when_unchecked(tmp_path):
    script = (
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--color', action='store_false')\nap.parse_args()\n"
    )
    plan = flows.plan_for_entry(_python_entry(tmp_path, script, slug="sf"))
    checked = flows.assemble(plan, {"color": "true"}, [], cwd=tmp_path, env={}, now=NOW)
    assert checked.args == []  # matches the script default: no flag
    unchecked = flows.assemble(plan, {"color": "false"}, [], cwd=tmp_path, env={}, now=NOW)
    assert unchecked.args == ["--color"]


def test_field_from_arg_maps_every_field(tmp_path):
    a = ParamDecl(
        name="mode",
        binding="none",
        delivery="flag",
        flag="--mode",
        required=True,
        type="choice",
        choices=("a", "b"),
        default="a",
        help="pick one",
        multiple=True,
        degraded=False,
        secret=True,
        action="",
    )
    f = flows.FormField.from_decl(a)
    assert (f.key, f.label, f.kind, f.source) == ("mode", "mode", "choice", "flag")
    assert f.choices == ["a", "b"]
    assert (f.default, f.has_default) == ("a", True)
    assert (f.help, f.required, f.secret) == ("pick one", True, True)
    assert (f.multiple, f.degraded, f.flag, f.action) == (True, False, "--mode", "")


def test_field_from_arg_degraded_renders_as_text(tmp_path):
    a = ParamDecl(
        name="bg", binding="none", delivery="flag", flag="--bg", type="int", degraded=True
    )
    f = flows.FormField.from_decl(a)
    assert f.kind == "str"  # a degraded field is a free-text field, whatever its type said
    assert f.default == ""  # a None flag default renders as the empty string, not a sentinel
    assert f.has_default is False


def test_render_default_spells_booleans_lowercase():
    assert flows._render_default(True) == "true"
    assert flows._render_default(False) == "false"
    assert flows._render_default(8) == "8"
    assert flows._render_default("x") == "x"


def test_plan_sources_are_exact_per_field():
    plan = flows.plan_for_entry(_command_entry())
    assert [f.source for f in plan.fields] == ["placeholder"]


def test_plan_field_sources_inject_and_flag(tmp_path):
    inject_plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="i1"))
    assert {f.source for f in inject_plan.fields} == {"inject"}
    flag_plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT, slug="f1"))
    assert {f.source for f in flag_plan.fields} == {"flag"}


DRIFTED_SCRIPT = """# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "CITY"
# kind = "const"
# type = "str"
#
# [[tool.skit.params]]
# name = "GONE"
# kind = "const"
# type = "str"
# ///
CITY = 'x'
print(CITY)
"""


def test_plan_drift_names_entry_and_keeps_usable_specs(tmp_path):
    # Use a slug that is NOT a substring of the word "drifted" — otherwise a dropped entry name
    # (drift_lines called with None) would still leave "drift" in the banner via "have drifted".
    plan = flows.plan_for_entry(_python_entry(tmp_path, DRIFTED_SCRIPT, slug="poster"))
    assert plan.source == "inject"
    assert plan.drift_lines  # GONE no longer exists in the script
    assert any("poster" in line for line in plan.drift_lines)  # names the entry
    assert [s.name for s in plan.specs] == ["CITY"]  # missing definitions dropped
    assert [f.key for f in plan.fields] == ["CITY"]
    assert plan.text == DRIFTED_SCRIPT  # the delivery layer reuses exactly this text


def test_plan_subparsers_degrades_with_reason(tmp_path):
    script = (
        "import argparse\nap = argparse.ArgumentParser()\nsub = ap.add_subparsers()\n"
        "p = sub.add_parser('x')\np.add_argument('--y')\n"
    )
    plan = flows.plan_for_entry(_python_entry(tmp_path, script, slug="sp"))
    assert plan.source == "argparse"
    assert plan.degraded_reason == "subparsers"
    assert plan.fields == []
    assert plan.text == script  # even a whole-parser degradation carries the text for delivery


def test_field_from_spec_maps_every_field(tmp_path):
    spec = ParamDecl(
        name="API",
        binding="const",
        delivery="inject",
        type="int",
        default=7,
        prompt="How many?",
        secret=True,
        env_source="API_N",
    )
    f = flows.FormField.from_decl(spec)
    assert (f.key, f.label, f.kind, f.source) == ("API", "How many?", "int", "inject")
    assert (f.default, f.has_default) == ("7", True)
    assert (f.secret, f.env_source) == (True, "API_N")


def test_field_from_spec_unknown_type_falls_back_to_text():
    # A non-numeric, non-bool type (choice here) is not in the inject whitelist, so the form
    # field collapses to free text — the fallback branch of the inject projection.
    spec = ParamDecl(name="X", binding="const", delivery="inject", type="choice")
    f = flows.FormField.from_decl(spec)
    assert f.kind == "str"
    assert f.default == ""  # a None spec default renders as "", not a sentinel string


def test_field_from_spec_maps_numeric_and_bool_kinds():
    # int is pinned elsewhere (WIDTH); float and bool need their own coverage so a corrupted
    # kind-whitelist entry can't quietly collapse them to free text.
    def _inject(name: str, type: ParamType) -> flows.FormField:
        return flows.FormField.from_decl(
            ParamDecl(name=name, binding="const", delivery="inject", type=type)
        )

    assert _inject("R", "float").kind == "float"
    assert _inject("B", "bool").kind == "bool"
    assert _inject("I", "int").kind == "int"


def test_type_error_messages_exact(tmp_path):
    int_field = flows.FormField(key="gap", label="gap", kind="int")
    assert flows.validate_value(int_field, "abc") == "gap needs a whole number — you typed 'abc'."
    float_field = flows.FormField(key="r", label="ratio", kind="float")
    assert flows.validate_value(float_field, "x") == "ratio needs a number — you typed 'x'."
    assert flows.validate_value(float_field, "1.5") is None
    bool_field = flows.FormField(key="b", label="fast", kind="bool")
    assert flows.validate_value(bool_field, "maybe") == "fast needs on or off — you typed 'maybe'."
    assert flows.validate_value(bool_field, "yes") is None
    choice_field = flows.FormField(key="m", label="mode", kind="choice", choices=["a", "b"])
    assert flows.validate_value(choice_field, "z") == "mode must be one of: a, b"
    required_field = flows.FormField(key="o", label="output", kind="str", required=True)
    assert flows.validate_value(required_field, "  ") == "output is required."


def test_assemble_display_order_and_masking(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="disp"))
    asm = flows.assemble(
        plan,
        {"OUTPUT": "long_{today}.jpg", "WIDTH": "800", "API_KEY": "sekret"},
        [],
        cwd=tmp_path,
        env={},
        now=NOW,
    )
    assert asm.display == [
        ("OUTPUT", "long_2026-07-09.jpg"),
        ("WIDTH", "800"),
        ("API_KEY", "•••"),
    ]
    assert asm.inject_values == {
        "OUTPUT": "long_2026-07-09.jpg",
        "WIDTH": "800",
        "API_KEY": "sekret",
    }
    assert asm.masked_args == asm.args  # inject: values aren't in argv, nothing to mask


def test_assemble_none_plan_only_carries_extras(tmp_path):
    asm = flows.assemble(flows.FormPlan(source="none"), {}, ["-v"], cwd=tmp_path, env={}, now=NOW)
    assert asm.args == ["-v"]
    assert asm.masked_args == ["-v"]
    assert asm.inject_values == {}
    assert asm.command_values == {}
    assert asm.display == []


# --------------------------------------------------------------------------
# review fixes (B3/B4/B5/B6 + transparency masking)
# --------------------------------------------------------------------------


def test_command_placeholders_are_required_and_secret_prechecked():
    from skit.models import ScriptMeta

    meta = ScriptMeta(
        name="c2", kind="command", template="curl -H {api_key} {url}", params=["api_key", "url"]
    )
    plan = flows.plan_for_entry(Entry(slug="c2", meta=meta, dir=Path("/nonexistent")))
    by = {f.key: f for f in plan.fields}
    assert by["api_key"].secret is True  # C3 applies to every source
    assert by["url"].secret is False
    assert all(f.required for f in plan.fields)  # empty values must not assemble silently
    errors = flows.validate(plan, {"api_key": "", "url": ""})
    assert set(errors) == {"api_key", "url"}


def test_save_after_run_clears_cleared_extra_args(tmp_path):
    entry = _python_entry(tmp_path, MANAGED_SCRIPT, slug="clr")
    plan = flows.plan_for_entry(entry)
    flows.save_after_run(
        "clr", plan, {"OUTPUT": "a"}, ["--fast"], 0, at="2026-01-01T00:00:00+00:00"
    )
    assert argstate.load_state("clr")["extra_args"] == ["--fast"]
    # The user emptied the extra-args field: the cleared state must PERSIST (the old
    # falsy-merge resurrected it forever).
    flows.save_after_run("clr", plan, {"OUTPUT": "a"}, [], 0, at="2026-01-01T00:00:01+00:00")
    assert argstate.load_state("clr")["extra_args"] == []


def test_save_after_run_purges_secret_placeholder_from_presets(tmp_path):
    from skit.models import ScriptMeta

    meta = ScriptMeta(name="c3", kind="command", template="x {api_key}", params=["api_key"])
    entry = Entry(slug="c3", meta=meta, dir=Path("/nonexistent"))
    # Plaintext saved back when the placeholder wasn't treated as secret yet.
    argstate.save_preset("c3", "old", {"api_key": "sk-123"})
    argstate.save_last("c3", values={"api_key": "sk-123"})
    plan = flows.plan_for_entry(entry)
    flows.save_after_run("c3", plan, {"api_key": "sk-456"}, [], 0, at="2026-01-01T00:00:00+00:00")
    state = argstate.load_state("c3")
    assert "api_key" not in state["values"]
    assert all("api_key" not in p for p in state["presets"].values())


def test_assemble_expand_extra_false_passes_argv_untouched(tmp_path):
    (tmp_path / "x1.txt").touch()
    plan = flows.FormPlan(source="none")
    asm = flows.assemble(
        plan, {}, ["x*.txt", "{env:UNSET_VAR}"], cwd=tmp_path, env={}, now=NOW, expand_extra=False
    )
    # The CLI's argv already went through the user's shell: no re-glob, no token pass,
    # and an unset {env:...} is NOT an error — it's just text the script will receive.
    assert asm.args == ["x*.txt", "{env:UNSET_VAR}"]


def test_masked_args_hide_flag_source_secret_values(tmp_path):
    script = (
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--api-key')\nap.add_argument('--name')\nap.parse_args()\n"
    )
    plan = flows.plan_for_entry(_python_entry(tmp_path, script, slug="mask"))
    asm = flows.assemble(
        plan, {"api_key": "sk-secret", "name": "ada"}, [], cwd=tmp_path, env={}, now=NOW
    )
    assert asm.args == ["--api-key", "sk-secret", "--name", "ada"]  # the real command
    assert asm.masked_args == ["--api-key", "•••", "--name", "ada"]  # what gets printed


def test_masked_args_still_glob_expand_multiple_fields(tmp_path):
    # The masked mirror runs through the same assembly (cwd and all): a multiple field
    # must glob-expand identically on both sides while the secret stays masked.
    (tmp_path / "a.png").touch()
    script = (
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('inputs', nargs='+')\nap.add_argument('--api-key')\nap.parse_args()\n"
    )
    plan = flows.plan_for_entry(_python_entry(tmp_path, script, slug="mg"))
    asm = flows.assemble(
        plan, {"inputs": "*.png", "api_key": "sk-1"}, [], cwd=tmp_path, env={}, now=NOW
    )
    assert asm.args == ["a.png", "--api-key", "sk-1"]
    assert asm.masked_args == ["a.png", "--api-key", "•••"]


# --------------------------------------------------------------------------
# execute — the unified delivery pipeline (A1)
# --------------------------------------------------------------------------


def _emit_sink():
    """A (captured-lines, emit) pair for driving flows.execute in tests."""
    lines: list[str] = []
    return lines, lines.append


def test_transparency_lines_inject_source_shows_masked_and_temp_note(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="tl"))
    asm = flows.assemble(
        plan,
        {"OUTPUT": "out.jpg", "WIDTH": "800", "API_KEY": "sekret"},
        [],
        cwd=tmp_path,
        env={},
        now=NOW,
    )
    lines = flows.transparency_lines(
        plan.plan_entry if False else _python_entry(tmp_path, MANAGED_SCRIPT, slug="tl2"), asm, None
    )
    joined = "\n".join(lines)
    assert "→ inject:" in joined
    assert "OUTPUT = out.jpg" in joined  # plain `k = v`, no repr quotes (the old CLI/TUI drift)
    assert "temporary copy" in joined
    assert "sekret" not in joined  # the secret value never appears
    assert "•••" in joined


def test_transparency_lines_flag_source_is_single_command_line(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, ARGPARSE_SCRIPT, slug="tlf"))
    asm = flows.assemble(plan, _values_ok(), [], cwd=tmp_path, env={}, now=NOW)
    lines = flows.transparency_lines(
        _python_entry(tmp_path, ARGPARSE_SCRIPT, slug="tlf2"), asm, None
    )
    assert len(lines) == 1  # no inject note for a flag-source run
    assert lines[0].startswith("→ ")


def test_execute_runs_and_returns_the_scripts_exit_code(tmp_path, monkeypatch):
    from skit import launcher

    entry = _python_entry(tmp_path, "print(1)\n", slug="ex")
    monkeypatch.setattr(launcher, "run_entry", lambda *a, **k: 7)
    lines, emit = _emit_sink()
    outcome = flows.execute(entry, flows.FormPlan(source="none"), flows.Assembly(), emit=emit)
    assert outcome.code == 7
    assert outcome.launched is True
    assert outcome.failure == ""
    assert lines  # the command line was emitted


def test_execute_injects_then_cleans_up_the_temp_copy(tmp_path, monkeypatch):
    from skit import launcher

    captured: dict[str, object] = {}

    def fake_run(
        entry, extra, *, values=None, invoke_cwd=None, script_override=None, env_overlay=None
    ):
        assert script_override is not None
        captured["override"] = script_override
        captured["existed_during_run"] = script_override.exists()
        return 0

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    entry = _python_entry(tmp_path, MANAGED_SCRIPT, slug="exi")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(
        plan, {"OUTPUT": "o", "WIDTH": "1", "API_KEY": "k"}, [], cwd=tmp_path, env={}, now=NOW
    )
    lines, emit = _emit_sink()
    outcome = flows.execute(entry, plan, asm, emit=emit)
    assert outcome.code == 0
    assert captured["existed_during_run"] is True  # the temp copy was there for the run
    override = captured["override"]
    assert isinstance(override, Path)
    assert not override.exists()  # ...and gone afterwards
    # execute passes the REAL temp path into the transparency line, not None:
    assert any(".injected-" in line for line in lines)


def test_execute_classifies_missing_target(tmp_path, monkeypatch):
    from skit import launcher

    def boom(*a, **k):
        raise launcher.TargetMissingError("gone")

    monkeypatch.setattr(launcher, "run_entry", boom)
    _lines, emit = _emit_sink()
    outcome = flows.execute(
        _python_entry(tmp_path, "print(1)\n", slug="exm"),
        flows.FormPlan(source="none"),
        flows.Assembly(),
        emit=emit,
    )
    assert outcome.code is None
    assert outcome.launched is False
    assert outcome.failure == flows.FAIL_MISSING
    assert "gone" in outcome.message


def test_execute_classifies_not_executable(tmp_path, monkeypatch):
    from skit import launcher

    def boom(*a, **k):
        raise launcher.NotExecutableError("not +x")

    monkeypatch.setattr(launcher, "run_entry", boom)
    _lines, emit = _emit_sink()
    outcome = flows.execute(
        _python_entry(tmp_path, "print(1)\n", slug="exx"),
        flows.FormPlan(source="none"),
        flows.Assembly(),
        emit=emit,
    )
    assert outcome.failure == flows.FAIL_NOT_EXECUTABLE


def test_execute_classifies_injection_drift(tmp_path, monkeypatch):
    from skit.langs.python import shim

    def boom(*a, **k):
        raise shim.ShimError("target vanished")

    monkeypatch.setattr(shim, "inject", boom)
    entry = _python_entry(tmp_path, MANAGED_SCRIPT, slug="exd")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(
        plan, {"OUTPUT": "o", "WIDTH": "1", "API_KEY": "k"}, [], cwd=tmp_path, env={}, now=NOW
    )
    _lines, emit = _emit_sink()
    outcome = flows.execute(entry, plan, asm, emit=emit)
    assert outcome.code is None
    assert outcome.failure == flows.FAIL_DRIFT
    assert "resync" in outcome.message  # points at the fix


def test_execute_bad_value_reports_value_not_drift(tmp_path, monkeypatch):
    # A value that can't coerce to its declared int type: the target was found fine, so
    # this is a value error (not drift) — a distinct classification and message.
    text = metawriter_write(
        "RETRIES = 3\nprint(RETRIES)\n",
        [("RETRIES", "int")],
    )
    entry = _python_entry(tmp_path, text, slug="exv")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {"RETRIES": "not-a-number"}, [], cwd=tmp_path, env={}, now=NOW)
    _lines, emit = _emit_sink()
    outcome = flows.execute(entry, plan, asm, emit=emit)
    assert outcome.code is None
    assert outcome.failure == flows.FAIL_BAD_VALUE
    assert outcome.message == "'not-a-number' isn't a valid int for RETRIES."
    assert "resync" not in outcome.message


def test_transparency_inject_lines_are_exact(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="tex"))
    asm = flows.assemble(
        plan,
        {"OUTPUT": "out.jpg", "WIDTH": "800", "API_KEY": "s"},
        [],
        cwd=tmp_path,
        env={},
        now=NOW,
    )
    lines = flows.transparency_lines(
        _python_entry(tmp_path, MANAGED_SCRIPT, slug="tex2"), asm, None
    )
    # Exact first line (kills the ", " separator and the "→ inject: " string mutants):
    assert lines[0] == "→ inject: OUTPUT = out.jpg, WIDTH = 800, API_KEY = •••"
    assert lines[1].startswith("  (written to a temporary copy")


def test_transparency_shows_the_injected_temp_path(tmp_path):
    plan = flows.plan_for_entry(_python_entry(tmp_path, MANAGED_SCRIPT, slug="ttp"))
    asm = flows.assemble(
        plan, {"OUTPUT": "o", "WIDTH": "1", "API_KEY": "s"}, [], cwd=tmp_path, env={}, now=NOW
    )
    fake_temp = tmp_path / ".injected-abc.py"
    lines = flows.transparency_lines(
        _python_entry(tmp_path, MANAGED_SCRIPT, slug="ttp2"), asm, fake_temp
    )
    # The command line names the temp copy that will actually run, not the stored script.
    assert ".injected-abc.py" in lines[-1]


def test_transparency_flag_source_masks_secret_in_command(tmp_path):
    script = (
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--api-key')\nap.add_argument('--name')\nap.parse_args()\n"
    )
    plan = flows.plan_for_entry(_python_entry(tmp_path, script, slug="tfs"))
    asm = flows.assemble(
        plan, {"api_key": "sk-SECRET", "name": "ada"}, [], cwd=tmp_path, env={}, now=NOW
    )
    line = flows.transparency_lines(_python_entry(tmp_path, script, slug="tfs2"), asm, None)[-1]
    assert "sk-SECRET" not in line  # masked_args, not args
    assert "•••" in line
    assert "--name ada" in line  # the non-secret flags still show (masked_args isn't dropped)


def test_transparency_command_source_shows_filled_template(tmp_path):
    entry = _command_entry("tcs")
    asm = flows.assemble(
        flows.plan_for_entry(entry), {"msg": "hello"}, [], cwd=tmp_path, env={}, now=NOW
    )
    line = flows.transparency_lines(entry, asm, None)[-1]
    assert "echo hello" in line  # command_values were substituted into the template


def test_transparency_command_source_masks_secret_placeholder(tmp_path):
    """A secret-named command placeholder ({api_key}) must be masked in the shown command
    line, exactly like a secret flag — the plaintext key must never reach the scrollback or
    `skit run --dry-run` output. The real value still substitutes into the process."""
    meta = ScriptMeta(
        name="fetch",
        kind="command",
        template='curl -H "Authorization: Bearer {api_key}" https://api.example.com',
        params=["api_key"],
    )
    entry = Entry(slug="fetch", meta=meta, dir=Path("/nonexistent"))
    plan = flows.plan_for_entry(entry)
    assert any(f.secret for f in plan.fields)  # api_key is treated as a secret
    asm = flows.assemble(plan, {"api_key": "sk-SUPERSECRET-123"}, [], cwd=tmp_path, env={}, now=NOW)
    line = flows.transparency_lines(entry, asm, None)[-1]
    assert "sk-SUPERSECRET-123" not in line  # not leaked to the shown command line
    assert "•••" in line
    assert asm.command_values["api_key"] == "sk-SUPERSECRET-123"  # real value still runs


def test_execute_not_executable_message_carries_the_error(tmp_path, monkeypatch):
    from skit import launcher

    monkeypatch.setattr(
        launcher,
        "run_entry",
        lambda *a, **k: (_ for _ in ()).throw(launcher.NotExecutableError("chmod +x it")),
    )
    _lines, emit = _emit_sink()
    outcome = flows.execute(
        _python_entry(tmp_path, "print(1)\n", slug="exne"),
        flows.FormPlan(source="none"),
        flows.Assembly(),
        emit=emit,
    )
    assert "chmod +x it" in outcome.message


def test_execute_launch_error_message_carries_the_error(tmp_path, monkeypatch):
    from skit import launcher

    monkeypatch.setattr(
        launcher,
        "run_entry",
        lambda *a, **k: (_ for _ in ()).throw(launcher.LaunchError("workdir gone")),
    )
    _lines, emit = _emit_sink()
    outcome = flows.execute(
        _python_entry(tmp_path, "print(1)\n", slug="exle"),
        flows.FormPlan(source="none"),
        flows.Assembly(),
        emit=emit,
    )
    assert outcome.failure == flows.FAIL_LAUNCH
    assert "workdir gone" in outcome.message


def test_execute_forwards_invoke_cwd(tmp_path, monkeypatch):
    from skit import launcher

    captured: dict[str, object] = {}

    def fake_run(
        entry, extra, *, values=None, invoke_cwd=None, script_override=None, env_overlay=None
    ):
        captured["cwd"] = invoke_cwd
        return 0

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    _lines, emit = _emit_sink()
    where = tmp_path / "run-here"
    where.mkdir()
    flows.execute(
        _python_entry(tmp_path, "print(1)\n", slug="excwd"),
        flows.FormPlan(source="none"),
        flows.Assembly(),
        emit=emit,
        invoke_cwd=where,
    )
    assert captured["cwd"] == where


def test_execute_inject_falls_back_to_entry_dir(tmp_path, monkeypatch):
    # write_injected writes to the OS temp dir, falling back to entry.dir when that
    # fails. execute must pass entry.dir as the fallback (not None) — force the primary
    # mkstemp to fail and assert the temp copy lands next to the stored script.
    import tempfile

    from skit import launcher

    real_mkstemp = tempfile.mkstemp

    def flaky_mkstemp(*args, **kwargs):
        if "dir" not in kwargs:  # the primary (OS-temp) attempt
            raise OSError("no temp dir")
        return real_mkstemp(*args, **kwargs)

    monkeypatch.setattr(tempfile, "mkstemp", flaky_mkstemp)

    landed: dict[str, object] = {}

    def fake_run(
        entry, extra, *, values=None, invoke_cwd=None, script_override=None, env_overlay=None
    ):
        landed["path"] = script_override
        return 0

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    entry = _python_entry(tmp_path, MANAGED_SCRIPT, slug="exfb")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(
        plan, {"OUTPUT": "o", "WIDTH": "1", "API_KEY": "s"}, [], cwd=tmp_path, env={}, now=NOW
    )
    _lines, emit = _emit_sink()
    flows.execute(entry, plan, asm, emit=emit)
    path = landed["path"]
    assert isinstance(path, Path)
    assert path.parent == entry.dir  # the fallback dir was entry.dir, not None
