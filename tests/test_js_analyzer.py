"""JS/TS analyzer + parseArgs reader unit pins: const detection over both grammars, the literal
domain (number/string/bool, template/object/array/destructuring excluded), type inference edges,
let/var + reassignment demotion, last-write-wins, the has_error degradation path, the reconcile
matrix, the `// ///` block round-trip + shebang skip, the parseArgs reader (every type/short/default/
multiple + every degrade path), the tsx grammar branch, and the registry import guard.
"""

from __future__ import annotations

import sys

import pytest
from typer.testing import CliRunner

from skit import analysis, cli, flows, store
from skit.langs import registry
from skit.langs.javascript import analyzer as js
from skit.langs.javascript import io as js_io
from skit.params import ParamDecl

runner = CliRunner()


def cands(src: str, *, lang: str = "js") -> list[analysis.Candidate]:
    return js.analyze(src, lang=lang).candidates


def by_name(src: str, *, lang: str = "js") -> dict[str, analysis.Candidate]:
    return {c.name: c for c in cands(src, lang=lang)}


# ---------------------------------------------------------------- const detection


def test_const_number_string_bool():
    b = by_name(
        'const W = 800;\nconst N = "hi";\nconst T = true;\nconst F = false;\nconst R = 2.5;\n'
    )
    assert (b["W"].type, b["W"].default) == ("int", 800)
    assert (b["N"].type, b["N"].default) == ("str", "hi")
    assert (b["T"].type, b["T"].default) == ("bool", True)
    assert (b["F"].type, b["F"].default) == ("bool", False)
    assert (b["R"].type, b["R"].default) == ("float", 2.5)


def test_template_string_excluded():
    # A template string may interpolate — never a candidate, even without a `${...}`.
    assert cands("const A = `hi ${x}`;\nconst B = `plain`;\n") == []


def test_object_and_array_excluded():
    assert cands("const O = {a: 1};\nconst A = [1, 2];\n") == []


def test_destructuring_excluded():
    # object_pattern / array_pattern names are not plain identifiers.
    assert cands("const {p, q} = obj;\nconst [m, n] = arr;\n") == []


def test_bare_declaration_without_value_skipped():
    # `let x;` has a declarator but no value node — skipped; the const still lands.
    assert [c.name for c in cands("let x;\nconst Y = 5;\n")] == ["Y"]


def test_leading_underscore_skipped():
    assert [c.name for c in cands("const _HIDDEN = 1;\nconst SHOWN = 2;\n")] == ["SHOWN"]


def test_last_write_wins_keeps_first_slot():
    b = by_name("const X = 1;\nconst Y = 5;\nconst X = 2;\n")
    assert b["X"].default == 2
    names = [c.name for c in cands("const X = 1;\nconst Y = 5;\nconst X = 2;\n")]
    assert names.index("X") < names.index("Y")


def test_multiple_declarators_in_one_statement():
    b = by_name("const A = 1, B = 2;\n")
    assert (b["A"].default, b["B"].default) == (1, 2)


def test_comment_between_keyword_and_declarator_is_skipped():
    # A comment is a named child of the declaration but not a declarator — skipped, const still lands.
    assert [c.name for c in cands("const /* note */ X = 5;\n")] == ["X"]


def test_secret_by_name():
    assert by_name('const API_KEY = "x";\n')["API_KEY"].secret is True


def test_lineno_recorded():
    (c,) = cands("\n\nconst X = 5;\n")
    assert c.lineno == 3


# ---------------------------------------------------------------- demotions


def _demoted(src: str) -> set[str]:
    return {c.name for c in cands(src) if c.demoted}


def test_let_and_var_demoted():
    b = by_name("let A = 1;\nvar B = 2;\n")
    assert (b["A"].demoted, b["A"].demotion) == (True, "accumulator")
    assert (b["B"].demoted, b["B"].demotion) == (True, "accumulator")


def test_const_reassigned_is_demoted():
    assert _demoted("const C = 1;\nC = 2;\n") == {"C"}


def test_const_augmented_assign_is_demoted():
    assert _demoted("const N = 0;\nN += 5;\n") == {"N"}


def test_const_update_expression_is_demoted():
    assert _demoted("const N = 0;\nN++;\n") == {"N"}


def test_plain_const_not_demoted():
    (c,) = cands("const STABLE = 7;\n")
    assert (c.demoted, c.demotion) == (False, "")


def test_member_reassignment_does_not_demote():
    # `obj.x = …` reassigns a property, not the top-level binding.
    assert _demoted("const CFG = 1;\nglobalThis.CFG = 2;\n") == set()


# ---------------------------------------------------------------- type inference


def test_negative_int_is_a_unary_expression_not_a_number_literal():
    # A leading `-` makes the value a unary_expression, which is NOT in the literal node set — so a
    # negative numeric const is (deliberately, per the design) not offered. Documented limitation.
    assert cands("const N = -3;\n") == []


def test_exotic_number_literals_are_float_with_source_text_default():
    b = by_name("const H = 0xFF;\nconst E = 1e3;\nconst G = 100n;\n")
    assert (b["H"].type, b["H"].default) == ("float", "0xFF")
    assert (b["E"].type, b["E"].default) == ("float", "1e3")
    assert (b["G"].type, b["G"].default) == ("float", "100n")


def test_simple_decimal_float():
    (c,) = cands("const R = 3.25;\n")
    assert (c.type, c.default) == ("float", 3.25)


def test_empty_and_escaped_string_values():
    b = by_name('const E = "";\nconst X = "a\\"b\\n";\n')
    assert b["E"].default == ""
    assert b["X"].default == 'a\\"b\\n'  # fragments + escape sequences, raw


# ---------------------------------------------------------------- TypeScript grammar


def test_ts_annotation_value_still_found():
    b = by_name('const N: number = 5;\nconst S: string = "x";\n', lang="ts")
    assert (b["N"].type, b["N"].default) == ("int", 5)
    assert (b["S"].type, b["S"].default) == ("str", "x")


def test_ts_only_constructs_parse_under_the_typescript_grammar():
    src = "interface I { a: number }\ntype T = number;\nenum E { A }\nconst X: number = 5;\n"
    assert [c.name for c in cands(src, lang="ts")] == ["X"]


def test_js_grammar_errors_on_typescript_only_syntax():
    # The js kind must NOT silently parse TS-only syntax — it degrades honestly.
    assert js.analyze("enum E { A }\nconst X = 5;\n", lang="js").syntax_error is True


def test_tsx_grammar_branch():
    # The tsx dialect parses JSX; injected here only to exercise the language resolver's tsx branch.
    result = js.analyze("const X = 5;\nconst e = <div/>;\n", lang="tsx")
    assert result.syntax_error is False
    assert [c.name for c in result.candidates] == ["X"]


def test_unknown_lang_falls_back_to_javascript():
    assert [c.name for c in cands("const X = 5;\n", lang="brainfuck")] == ["X"]


# ---------------------------------------------------------------- degradation


def test_has_error_returns_empty_syntax_error():
    result = js.analyze("const X = ;\n")
    assert result.syntax_error is True
    assert result.candidates == []


def test_empty_script():
    result = js.analyze("")
    assert result.candidates == []
    assert result.syntax_error is False


# ---------------------------------------------------------------- reconcile


def test_reconcile_const_ok():
    report = js.reconcile(
        "const CITY = 800;\n", [ParamDecl(name="CITY", binding="const", type="int")]
    )
    assert not report.has_drift
    assert [s.name for s in report.ok] == ["CITY"]


def test_reconcile_const_gone_is_missing():
    report = js.reconcile(
        "const OTHER = 1;\n", [ParamDecl(name="CITY", binding="const", type="int")]
    )
    assert report.has_drift
    assert [s.name for s in report.missing] == ["CITY"]


def test_reconcile_type_change_is_flagged():
    report = js.reconcile('const N = "text";\n', [ParamDecl(name="N", binding="const", type="int")])
    assert [spec.name for spec, _ in report.changed] == ["N"]


def test_reconcile_ts_lang_threaded():
    # A TS-only file must reconcile under the TS grammar (js grammar would report a syntax error and
    # mark everything missing).
    src = "interface I { a: number }\nconst X: number = 5;\n"
    report = js.reconcile(src, [ParamDecl(name="X", binding="const", type="int")], lang="ts")
    assert not report.has_drift


# ---------------------------------------------------------------- the // block engine


def test_block_roundtrip_on_ts_file():
    specs = [ParamDecl(name="N", binding="const", delivery="inject", type="int", default=5)]
    src = "const N: number = 5;\nconsole.log(N);\n"
    out = js_io.write_params(src, specs)
    assert "// [tool.skit]" in out
    assert 'name = "N"' in out
    assert js_io.read_params(out) == specs
    assert "const N: number = 5;\nconsole.log(N);\n" in out  # code bytes untouched


def test_block_lands_after_a_node_shebang():
    specs = [ParamDecl(name="P", binding="const", delivery="inject", type="int", default=1)]
    src = "#!/usr/bin/env node\nconst P = 1;\n"
    out = js_io.write_params(src, specs)
    assert out.startswith("#!/usr/bin/env node\n")
    assert out.index("#!") < out.index("// /// script")


def test_block_at_top_when_no_shebang():
    specs = [ParamDecl(name="P", binding="const", delivery="inject", type="int", default=1)]
    out = js_io.write_params("const P = 1;\n", specs)
    assert out.startswith("// /// script\n")


def test_write_empty_params_is_identity():
    assert js_io.write_params("const P = 1;\n", []) == "const P = 1;\n"


# ---------------------------------------------------------------- parseArgs reader


def read(src: str, *, lang: str = "js"):
    from skit.langs.javascript import cli_reader

    return cli_reader.read_cli(src, lang=lang)


def test_parseargs_util_member_inline_options():
    src = 'const {values} = util.parseArgs({options:{name:{type:"string"}}});\n'
    spec = read(src)
    assert spec is not None
    assert spec.ok
    (f,) = spec.fields
    assert (f.name, f.flag, f.type) == ("name", "--name", "str")


def test_parseargs_bare_call():
    (f,) = read('parseArgs({options:{x:{type:"boolean"}}});\n').fields
    assert (f.type, f.action, f.default) == ("bool", "store_true", False)


def test_parseargs_nested_member():
    spec = read('a.b.parseArgs({options:{x:{type:"string"}}});\n')
    assert spec is not None
    assert [f.name for f in spec.fields] == ["x"]


def test_parseargs_all_option_features():
    src = (
        "parseArgs({options:{"
        'name:{type:"string",short:"n",default:"world"},'
        'verbose:{type:"boolean"},'
        'tag:{type:"string",multiple:true},'
        '"dry-run":{type:"boolean",default:false}'
        "}});\n"
    )
    fields = {f.name: f for f in read(src).fields}
    assert fields["name"].default == "world"  # short is display-only; the long flag is assembled
    assert fields["name"].flag == "--name"
    assert (fields["verbose"].type, fields["verbose"].action) == ("bool", "store_true")
    assert fields["tag"].multiple is True
    assert fields["dry-run"].default is False


def test_parseargs_boolean_default_true_applies_literally():
    (f,) = read('parseArgs({options:{force:{type:"boolean",default:true}}});\n').fields
    assert (f.type, f.default) == ("bool", True)


def test_parseargs_string_key_option():
    (f,) = read('parseArgs({options:{"dry-run":{type:"boolean"}}});\n').fields
    assert (f.name, f.flag) == ("dry-run", "--dry-run")


def test_parseargs_secret_option_name():
    (f,) = read('parseArgs({options:{token:{type:"string"}}});\n').fields
    assert f.secret is True


# ---- degrade / skip paths -------------------------------------------------------


def test_parseargs_identifier_options_whole_spec_degrade():
    spec = read("parseArgs({options: opts});\n")
    assert spec is not None
    assert (spec.ok, spec.reason) == (False, "dynamic")


def test_parseargs_spread_in_options_whole_spec_degrade():
    spec = read('parseArgs({options:{...common, name:{type:"string"}}});\n')
    assert (spec.ok, spec.reason) == (False, "dynamic")


def test_parseargs_computed_key_skips_just_that_field():
    src = 'parseArgs({options:{[dyn]:{type:"string"}, name:{type:"string"}}});\n'
    assert [f.name for f in read(src).fields] == ["name"]


def test_parseargs_empty_string_key_is_skipped():
    src = 'parseArgs({options:{"":{type:"string"}, ok:{type:"string"}}});\n'
    assert [f.name for f in read(src).fields] == ["ok"]


def test_parseargs_non_object_option_value_degrades_field():
    (f,) = read("parseArgs({options:{name: someVar}});\n").fields
    assert (f.name, f.degraded) == ("name", True)


def test_parseargs_unknown_type_string_degrades_field():
    (f,) = read('parseArgs({options:{n:{type:"integer"}}});\n').fields
    assert f.degraded is True


def test_parseargs_non_literal_type_value_degrades_field():
    (f,) = read("parseArgs({options:{n:{type: someType}}});\n").fields
    assert f.degraded is True


def test_parseargs_non_literal_default_degrades_field():
    (f,) = read('parseArgs({options:{n:{type:"string", default: fallback}}});\n').fields
    assert f.degraded is True


def test_parseargs_ignores_spread_computed_and_numeric_keys_in_spec():
    # A spread, a computed key, and a numeric key inside the option-spec object are read and skipped,
    # not crashed on — only the real `type` pair is applied.
    (f,) = read('parseArgs({options:{n:{type:"string", [dyn]: 1, 0: 2, ...rest}}});\n').fields
    assert (f.name, f.type) == ("n", "str")


def test_parseargs_option_spec_without_type_keeps_str_and_reads_default():
    # No `type` key: the reader must skip the type application and still read a default.
    (f,) = read('parseArgs({options:{n:{default:"hi"}}});\n').fields
    assert (f.type, f.default) == ("str", "hi")


def test_parseargs_shorthand_property_in_options_is_skipped():
    # A shorthand property (`{name, real:{...}}`) isn't a pair — skipped, not crashed on.
    (f,) = read('parseArgs({options:{shorthand, real:{type:"string"}}});\n').fields
    assert f.name == "real"


def test_parseargs_finds_options_past_a_spread_and_another_key():
    # A spread and a non-"options" pair sit before `options`: the reader scans past both.
    src = 'parseArgs({...base, allowPositionals: true, options:{n:{type:"string"}}});\n'
    assert [f.name for f in read(src).fields] == ["n"]


def test_parseargs_empty_options_object_is_a_readable_zero_field_surface():
    spec = read("parseArgs({options:{}});\n")
    assert spec is not None
    assert spec.ok
    assert spec.fields == []


def test_no_parseargs_surface_returns_none():
    # A plain identifier call that isn't parseArgs (not an identifier match, not a member call).
    assert read("const x = 5;\nfoo(x);\n") is None


def test_parseargs_member_call_that_is_not_parseargs_is_ignored():
    assert read('console.log("x");\nconst y = 5;\n') is None


def test_parseargs_with_no_config_object_returns_none():
    assert read("parseArgs();\n") is None


def test_parseargs_non_object_config_returns_none():
    assert read("parseArgs(config);\n") is None


def test_parseargs_config_without_options_key_returns_none():
    assert read("parseArgs({allowPositionals: true});\n") is None


def test_reader_on_syntax_error_returns_none():
    assert read("const x = ;\n") is None


def test_reader_threads_lang_for_typescript():
    src = 'interface I {}\nparseArgs({options:{n:{type:"string"}}});\n'
    spec = read(src, lang="ts")
    assert spec is not None
    assert [f.name for f in spec.fields] == ["n"]


# ---------------------------------------------------------------- registry import guard


def _break_js_import(monkeypatch: pytest.MonkeyPatch) -> None:
    import skit.langs.javascript as js_pkg

    registry.spec_for.cache_clear()
    monkeypatch.delattr(js_pkg, "analyzer", raising=False)
    monkeypatch.setitem(sys.modules, "skit.langs.javascript.analyzer", None)  # type: ignore[arg-type]


def test_import_guard_degrades_analysis_capabilities_to_none(monkeypatch):
    try:
        _break_js_import(monkeypatch)
        spec = registry.spec_for("js")
        assert spec is not None
        assert spec.analyzer is None  # degraded
        assert spec.cli_reader is None
        assert spec.injector is None
        assert spec.params_io is not None  # the '//' block engine has no grammar dependency
    finally:
        registry.spec_for.cache_clear()


def test_plan_degrades_to_none_when_analyzer_missing(monkeypatch, tmp_path):
    src = tmp_path / "s.js"
    src.write_text("const CITY = 5;\nconsole.log(CITY);\n")
    entry = store.add_script(src, kind="js", name="jsdeg")
    try:
        _break_js_import(monkeypatch)
        plan = flows.plan_for_entry(entry)
        assert plan.source == "none"  # no analyzer -> no inject plan, entry still launchable
    finally:
        registry.spec_for.cache_clear()


# ---------------------------------------------------------------- `skit params` integration


def test_params_manage_writes_block_into_js_copy(tmp_path):
    src = tmp_path / "deploy.js"
    src.write_text("#!/usr/bin/env node\nconst CITY = 800;\nconsole.log(CITY);\n")
    entry = store.add_script(src, kind="js", name="jsp1")
    result = runner.invoke(cli.app, ["params", "jsp1", "--manage", "CITY"])
    assert result.exit_code == 0, result.output
    copy_text = entry.script_path.read_text(encoding="utf-8")
    assert "// [tool.skit]" in copy_text
    assert 'name = "CITY"' in copy_text
    assert copy_text.startswith("#!/usr/bin/env node\n")
    assert copy_text.index("#!") < copy_text.index("// /// script")


def test_params_show_lists_ts_const(tmp_path):
    src = tmp_path / "show.ts"
    src.write_text('const CITY: string = "Taipei";\n')
    store.add_script(src, kind="ts", name="tsp1")
    result = runner.invoke(cli.app, ["params", "tsp1"])
    assert result.exit_code == 0, result.output
    assert "CITY" in result.output
