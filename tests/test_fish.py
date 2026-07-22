"""fish analyzer (env-idiom only) + argparse reader unit pins.

Covers the hand scanner (tokenizer, quoting edges, line continuation, block-depth tracking),
the env-default idiom (one-line and newline `or`, suppression by a plain clobber, the -P-vs-p
distinction is N/A here since reads are deferred), the argparse spec-string matrix, the registry
capabilities, and an env-default e2e (offline plan/assemble unconditional; the real fish run is
SKIP-gated). The corpus (tests/corpus/fish/*.fish) is swept for totality + expected detections.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, flows, store
from skit.langs import registry
from skit.langs.fish import analyzer as fa
from skit.langs.fish import cli_reader as fc
from skit.params import ParamDecl

runner = CliRunner()
FISH_CORPUS = sorted((Path(__file__).parent / "corpus" / "fish").glob("*.fish"))


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    return tmp_path


def cands(src):
    return {c.name: c for c in fa.analyze(src).candidates}


# ---------------------------------------------------------------- env-default idiom


def test_oneline_idiom_int():
    c = cands("set -q PORT; or set PORT 8080\n")["PORT"]
    assert (c.binding, c.type, c.default, c.env_name) == ("envdefault", "int", 8080, "PORT")


def test_newline_continued_or():
    # fish continues an `or` at the start of the next line — the same idiom, two lines.
    c = cands("set -q PORT\nor set PORT 8080\n")
    assert c["PORT"].default == 8080


def test_float_and_string_defaults():
    c = cands("set -q RATE; or set RATE 2.5\nset -q REGION; or set REGION us-east-1\n")
    assert (c["RATE"].type, c["RATE"].default) == ("float", 2.5)
    assert (c["REGION"].type, c["REGION"].default) == ("str", "us-east-1")


def test_guarded_set_may_carry_scope_flags():
    # `or set -gx NAME v` still preserves an inherited value (the `or` only fires when unset).
    assert cands("set -q LOG; or set -gx LOG /var/log\n")["LOG"].default == "/var/log"


def test_secret_name_flagged():
    assert cands("set -q API_TOKEN; or set API_TOKEN x\n")["API_TOKEN"].secret is True


def test_suppressed_by_plain_clobber_anywhere():
    # A later unconditional `set PORT 9090` clobbers the inherited value → env would no-op.
    assert cands("set -q PORT; or set PORT 8080\nset PORT 9090\n") == {}


def test_clobber_before_the_idiom_also_suppresses():
    assert cands("set PORT 9090\nset -q PORT; or set PORT 8080\n") == {}


def test_unrelated_clobber_does_not_suppress():
    c = cands("set OTHER 1\nset -q PORT; or set PORT 8080\n")
    assert "PORT" in c
    assert "OTHER" not in c


def test_underscore_name_skipped():
    assert cands("set -q _P; or set _P 1\n") == {}


def test_first_occurrence_wins_on_duplicate_idiom():
    c = cands("set -q PORT; or set PORT 8080\nset -q PORT; or set PORT 1\n")
    assert c["PORT"].default == 8080  # first occurrence's default


def test_query_without_following_set_is_not_a_candidate():
    assert cands("set -q PORT\necho done\n") == {}


def test_query_with_no_name_is_ignored():
    assert cands("set -q; or set PORT 8080\n") == {}


def test_conditional_set_without_value_is_not_a_candidate():
    assert cands("set -q PORT; or set PORT\n") == {}


def test_mismatched_names_are_not_an_idiom():
    assert cands("set -q PORT; or set OTHER 8080\n") == {}


def test_unconditional_set_after_query_is_not_an_idiom():
    # `set -q X; set X 1` (no `or`) is a query then a plain clobber — not an env-default.
    assert cands("set -q X; set X 1\n") == {}


# ---------------------------------------------------------------- block depth


def test_idiom_inside_function_is_not_toplevel():
    assert cands("function f\n  set -q P; or set P 1\nend\n") == {}


@pytest.mark.parametrize("opener", ["if true", "while true", "for x in 1", "begin", "switch $x"])
def test_idiom_inside_every_block_kind_is_ignored(opener):
    assert cands(f"{opener}\n  set -q P; or set P 1\nend\n") == {}


def test_toplevel_after_a_closed_block_is_detected():
    src = "function f\n  echo hi\nend\nset -q P; or set P 1\n"
    assert cands(src)["P"].default == 1


def test_nested_clobber_does_not_suppress_toplevel_idiom():
    # A plain `set P 9` inside a function must not suppress a top-level P env-default.
    src = "set -q P; or set P 1\nfunction f\n  set P 9\nend\n"
    assert cands(src)["P"].default == 1


def test_stray_end_clamps_depth_at_zero():
    # A leading `end` must not drive depth negative and hide a following top-level idiom.
    assert cands("end\nset -q P; or set P 1\n")["P"].default == 1


# ---------------------------------------------------------------- hints


def test_argv_hint():
    assert fa.analyze("echo $argv\n").uses_argv is True


def test_self_location_hints():
    assert fa.analyze("set d (status dirname)\n").uses_self_location is True
    assert fa.analyze("set f (status filename)\n").uses_self_location is True
    assert fa.analyze("echo hi\n").uses_self_location is False


def test_hint_ignores_commented_argv():
    assert fa.analyze("# uses $argv here\necho hi\n").uses_argv is False


# ---------------------------------------------------------------- reconcile


def test_reconcile_ok_then_drift():
    specs = [ParamDecl(name="PORT", binding="envdefault", delivery="env")]
    ok = fa.reconcile("set -q PORT; or set PORT 8080\n", specs)
    assert ok.ok
    assert not ok.has_drift
    gone = fa.reconcile("echo hi\n", specs)
    assert [s.name for s in gone.missing] == ["PORT"]


# ---------------------------------------------------------------- tokenizer internals


def test_tokenize_semicolon_and_words():
    assert fa._tokenize("a;b") == ["a", ";", "b"]
    assert fa._tokenize(";a") == [";", "a"]  # leading semicolon, empty current
    assert fa._tokenize("a;;b") == ["a", ";", ";", "b"]


def test_tokenize_quotes_hold_separators():
    assert fa._tokenize("set X 'a;b c'") == ["set", "X", "'a;b c'"]
    assert fa._tokenize('set X "a b"') == ["set", "X", '"a b"']


def test_tokenize_escaped_quote_does_not_close():
    assert fa._tokenize(r"echo 'a\'b'") == ["echo", r"'a\'b'"]


def test_tokenize_comment_ends_line():
    assert fa._tokenize("echo hi # tail") == ["echo", "hi"]


def test_tokenize_hash_midword_is_literal():
    assert fa._tokenize("echo a#b") == ["echo", "a#b"]


def test_tokenize_backslash_escape_outside_quote():
    assert fa._tokenize(r"echo a\ b") == ["echo", "a\\ b"]


def test_tokenize_unterminated_quote_is_total():
    assert fa._tokenize("echo 'oops") == ["echo", "'oops"]


def test_statements_drop_empty_runs_between_semicolons():
    # `a;;b` yields two statements with the empty middle run dropped (the ;-with-no-current path).
    assert fa._statements("a;;b") == [(["a"], 1), (["b"], 1)]


# ---------------------------------------------------------------- line continuation


def test_logical_lines_join_continuation():
    assert fa._logical_lines("a\\\nb") == [("ab", 1)]


def test_logical_lines_even_backslashes_are_not_a_continuation():
    lines = fa._logical_lines("a\\\\\nb")
    assert lines == [("a\\\\", 1), ("b", 2)]


def test_logical_lines_trailing_continuation_flushes():
    assert fa._logical_lines("a\\") == [("a", 1)]


# ---------------------------------------------------------------- dequote internals


def test_dequote_single_quote_escapes():
    assert fa._dequote(r"'a\'b'") == "a'b"
    assert fa._dequote(r"'a\\b'") == "a\\b"
    assert fa._dequote(r"'a\nb'") == "a\\nb"  # \n is not an escape in single quotes


def test_dequote_double_quote_escapes():
    assert fa._dequote(r'"a\"b"') == 'a"b'
    assert fa._dequote(r'"a\$b"') == "a$b"
    assert fa._dequote(r'"a\nb"') == "a\\nb"  # \n is not an escape in double quotes


def test_dequote_backslash_outside_and_at_end():
    assert fa._dequote(r"a\ b") == "a b"
    assert fa._dequote("a\\") == "a\\"  # a trailing backslash with nothing to escape is literal


def test_dequote_unterminated_quotes_are_total():
    assert fa._dequote("'abc") == "abc"
    assert fa._dequote('"abc') == "abc"


# ---------------------------------------------------------------- strip_comment internals


def test_strip_comment_paths():
    assert fa._strip_comment("echo hi # c") == "echo hi "
    assert fa._strip_comment("echo hi") == "echo hi"
    assert fa._strip_comment("# whole") == ""
    assert fa._strip_comment("echo '# not'") == "echo '# not'"
    assert fa._strip_comment("echo a#b") == "echo a#b"  # # not at a word boundary
    assert fa._strip_comment(r"echo a\#b") == r"echo a\#b"  # escaped # outside quotes
    assert fa._strip_comment(r"echo 'a\'b' # c") == r"echo 'a\'b' "


# ---------------------------------------------------------------- classify_set / is_query


def test_classify_set_matrix():
    assert fa._classify_set(["echo", "hi"]) is None
    assert fa._classify_set([]) is None
    st = fa._classify_set(["or", "set", "X", "1"])
    assert st == fa._SetStmt(conditional=True, is_query=False, name="X", value=["1"])
    flagged = fa._classify_set(["set", "-gx", "X", "1"])
    assert flagged == fa._SetStmt(conditional=False, is_query=False, name="X", value=["1"])
    end_opts = fa._classify_set(["set", "--", "-x", "1"])
    assert end_opts == fa._SetStmt(conditional=False, is_query=False, name="-x", value=["1"])
    dashval = fa._classify_set(["set", "X", "-1"])
    assert dashval == fa._SetStmt(conditional=False, is_query=False, name="X", value=["-1"])
    noname = fa._classify_set(["set", "-q"])
    assert noname == fa._SetStmt(conditional=False, is_query=True, name=None, value=[])


def test_is_query_matrix():
    assert fa._is_query(["--query"]) is True
    assert fa._is_query(["-q"]) is True
    assert fa._is_query(["-gq"]) is True
    assert fa._is_query(["-gx"]) is False
    assert fa._is_query(["--global"]) is False
    assert fa._is_query([]) is False


# ---------------------------------------------------------------- argparse reader


def read(src):
    spec = fc.read_cli(src)
    assert spec is not None
    return {f.name: f for f in spec.fields}, spec


def test_argparse_short_long_and_valueless_bool():
    fields, _ = read("argparse 'h/help' 'v/verbose' -- $argv\n")
    assert (fields["help"].flag, fields["help"].type) == ("--help", "bool")
    assert fields["help"].action == "store_true"
    assert fields["verbose"].type == "bool"


def test_argparse_value_suffixes():
    fields, _ = read("argparse 'n/name=' 'r/retries=?' 'f/file=+' 'g/glob=*' -- $argv\n")
    assert fields["name"].type == "str"
    assert not fields["name"].multiple
    assert fields["retries"].type == "str"  # optional attached value
    assert fields["file"].multiple is True
    assert fields["glob"].multiple is True
    # `=+`/`=*` are REPEAT grammar: fish's argparse wants `--file a --file b`, and the
    # one-flag-many-values shape would leave `b` as a stray positional.
    assert (fields["file"].repeat, fields["glob"].repeat) == (True, True)
    assert fields["name"].repeat is False


def test_argparse_long_only_and_short_only():
    fields, _ = read("argparse 'dry-run' 'x' -- $argv\n")
    assert fields["dry-run"].flag == "--dry-run"  # long name that contains a hyphen
    assert fields["x"].flag == "-x"  # single-char short-only


def test_argparse_dummy_short_yields_long_only():
    fields, _ = read("argparse 'x-long' -- $argv\n")
    assert fields["long"].flag == "--long"


def test_argparse_numeric_hash_degrades():
    fields, _ = read("argparse 'm#max' -- $argv\n")
    assert fields["max"].flag == "--max"
    assert fields["max"].degraded


def test_argparse_validator_is_stripped():
    fields, _ = read("argparse 'v/verbose!_check_it' -- $argv\n")
    assert fields["verbose"].type == "bool"
    assert not fields["verbose"].degraded


def test_argparse_secret_name():
    fields, _ = read("argparse 'token=' -- $argv\n")
    assert fields["token"].secret is True


def test_argparse_skips_own_options():
    fields, _ = read("argparse -n tool -x 'h,help' -i 'c/city=' -- $argv\n")
    # -n consumes `tool`, -x consumes `'h,help'`, -i takes no value; only c/city is a spec.
    assert list(fields) == ["city"]


def test_argparse_attached_own_option_does_not_consume():
    fields, _ = read("argparse --name=tool 'c/city=' -- $argv\n")
    assert list(fields) == ["city"]


def test_argparse_after_conditional_prefix_is_found():
    fields, _ = read("or argparse 'h/help' -- $argv\n")
    assert list(fields) == ["help"]


def test_argparse_empty_specs_is_zero_field_surface():
    spec = fc.read_cli("argparse -- $argv\n")
    assert spec is not None
    assert spec.fields == []


def test_no_argparse_returns_none():
    assert fc.read_cli("echo hello\n") is None


def test_argparse_variable_specs_degrade_to_dynamic():
    """A variable spec list (`argparse $specs -- $argv`) is DETECTED but unmodelable: the
    reader degrades to ok=False 'dynamic' (the python/JS rule) instead of fabricating a
    phantom `$specs` flag out of the variable name."""
    spec = fc.read_cli("argparse $specs -- $argv\n")
    assert spec is not None
    assert spec.ok is False
    assert spec.reason == "dynamic"
    assert spec.fields == []


def test_argparse_command_substitution_specs_degrade_to_dynamic():
    """Command substitution (`argparse (make_specs) -- $argv`) is dynamic too — the option
    set is unknowable statically."""
    spec = fc.read_cli("argparse (make_specs) -- $argv\n")
    assert spec is not None
    assert spec.ok is False
    assert spec.reason == "dynamic"
    assert spec.fields == []


def test_argparse_garbage_specs_are_skipped():
    # Empty spec, a value-suffix with no name (`=`), validator-only, bare and leading separators.
    spec = fc.read_cli("argparse '' '=' '!v' '#' '/x' 'ok' -- $argv\n")
    assert spec is not None
    assert [f.name for f in spec.fields] == ["ok"]


def test_spec_tokens_all_own_options_no_specs():
    # Own options exhaust the token list with no spec and no `--` (the loop-condition exit).
    assert fc._spec_tokens(["-n", "tool"]) == []


def test_argparse_empty_long_falls_back_to_short():
    fields, _ = read("argparse 'x/' -- $argv\n")
    assert fields["x"].flag == "-x"


# ---------------------------------------------------------------- registry wiring


def test_registry_capabilities():
    spec = registry.spec_for("fish")
    assert spec is not None
    assert spec.analyzer is not None  # env-idiom detection
    assert spec.cli_reader is not None  # argparse reading
    assert spec.params_io is not None  # in-file [tool.skit] via the '#' block engine
    assert spec.injector is None  # no injector in v1 (const/read injection deferred)


# ---------------------------------------------------------------- corpus sweep


@pytest.mark.parametrize("path", FISH_CORPUS, ids=lambda p: p.name)
def test_corpus_analyze_is_total_and_reads_back(path):
    from skit.langs.python import metawriter

    text = path.read_text(encoding="utf-8")
    result = fa.analyze(text)  # never raises
    # Every emitted candidate is an env-default (v1 scope); the block writer round-trips them.
    assert all(c.binding == "envdefault" for c in result.candidates)
    specs = [ParamDecl.from_candidate(c) for c in result.candidates]
    written = metawriter.write_params(text, specs)
    read_back = {d.name for d in metawriter.read_params(written)}
    assert read_back == {c.name for c in result.candidates}


def test_corpus_expected_detections():
    detected = {p.name: set(cands(p.read_text(encoding="utf-8"))) for p in FISH_CORPUS}
    assert detected["01_env_idioms.fish"] == {"PORT", "RATE", "REGION", "LOG_DIR"}
    assert detected["04_block_nesting.fish"] == {"TOP", "ALSO_TOP"}
    assert detected["05_reads_and_consts.fish"] == {"RETRIES"}
    assert detected["06_cjk.fish"] == {"問候", "EMOJI", "CITY"}


# ---------------------------------------------------------------- env-default e2e


def _add_port_entry(tmp_path, name="cfg"):
    src = tmp_path / f"{name}.fish"
    src.write_text("#!/usr/bin/env fish\nset -q PORT; or set PORT 8080\necho $PORT\n")
    return store.add_script(src, kind="fish", name=name)


def test_manage_then_plan_and_assemble_env_delivery(tmp_path):
    entry = _add_port_entry(tmp_path)
    result = runner.invoke(cli.app, ["params", entry.meta.name, "--manage", "PORT"])
    assert result.exit_code == 0, result.output
    block = entry.script_path.read_text(encoding="utf-8")
    assert "# [tool.skit]" in block
    assert 'name = "PORT"' in block
    assert 'kind = "envdefault"' in block

    plan = flows.plan_for_entry(entry)
    assert plan.source == "inject"
    (field,) = [f for f in plan.fields if f.key == "PORT"]
    assert field.source == "env"  # env delivery, no injector needed
    asm = flows.assemble(plan, {"PORT": "9090"}, [], cwd=tmp_path)
    assert asm.env_values == {"PORT": "9090"}


@pytest.mark.skipif(shutil.which("fish") is None, reason="fish not installed")
def test_env_overlay_overrides_default_in_real_fish(tmp_path):
    fish = shutil.which("fish")
    assert fish is not None
    entry = _add_port_entry(tmp_path, name="realcfg")
    runner.invoke(cli.app, ["params", entry.meta.name, "--manage", "PORT"])
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {"PORT": "9090"}, [], cwd=tmp_path)
    proc = subprocess.run(
        [fish, str(entry.script_path)],
        env={**os.environ, **asm.env_values},
        capture_output=True,
        text=True,
        check=False,
    )
    assert proc.stdout.strip() == "9090"  # the env overlay beat the script's 8080 default
