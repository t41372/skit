"""Mutation-kill tests for the JS/TS registry builders (`_javascript_spec` + `_js_analysis`).

Both js and ts resolve through `_javascript_spec`, which bakes the Tier-0 interpreted base
(RunnerLaunch, `//` comment, no interpreter) plus the four tree-sitter capabilities that
`_js_analysis` threads the kind's `lang` into (so ts parses under the TypeScript grammar and js
under JavaScript). Nothing else pins these exact field values or the lang wiring, so each is nailed
here through the real `registry.spec_for` resolution path and the real capability calls. The lang
threading is proved with TypeScript-only syntax (`interface …`), which the TS grammar accepts and
the JS grammar rejects — any mutant that drops/forces the `lang` argument silently falls back to the
JS grammar and is caught. English source values are the source of truth.
"""

from __future__ import annotations

from skit.langs import launch, registry
from skit.langs.base import InjectRequest
from skit.params import ParamDecl

# A TypeScript-only construct: `interface` is a syntax error under the JavaScript grammar, so any
# capability that parses this text under the wrong grammar degrades observably (syntax error / None /
# empty / raised InjectError). Used to pin the `lang="ts"` threading in `_js_analysis`.
_TS_ONLY_CONST = 'interface I { a: number }\nconst CITY = "Taipei";\n'
_TS_ONLY_IMPORT = 'import { foo } from "cowsay";\ninterface I { a: number }\n'
_TS_ONLY_PARSEARGS = 'interface I {}\nconst r = parseArgs({options:{name:{type:"string"}}});\n'


# ---------------------------------------------------------------- _javascript_spec fields (js)


def test_js_spec_scalar_fields():
    spec = registry.spec_for("js")
    assert spec is not None
    assert spec.kind == "js"  # first _interpreted arg
    assert spec.glyph == "✦"  # ✦ — the js badge glyph (second _interpreted arg)
    # `_interpreted(..., "", ...)`: js/ts launch through a resolved runner, so the base kind carries
    # no default interpreter name — the empty string, not None and not a stray literal.
    assert spec.default_interpreter == ""
    # The `//` line-comment prefix carries the in-file [tool.skit] block for js/ts.
    assert spec.comment is not None
    assert spec.comment.prefix == "//"


def test_js_spec_shebang_row_is_the_full_runner_triple():
    # `("node", "deno", "bun") if lang == "js" else ()` — the js kind advertises exactly these three
    # runner basenames for #! inference; the exact tuple pins every element and the `lang == "js"`
    # guard (any element/operator/literal mutation reshapes this tuple).
    spec = registry.spec_for("js")
    assert spec is not None
    assert spec.shebangs == ("node", "deno", "bun")


def test_js_spec_launches_through_a_runner_not_an_interpreter():
    # launch_strategy=launch.RunnerLaunch(): js/ts resolve deno>bun>node at run time rather than a
    # fixed interpreter, so the base `launch_strategy or InterpreterLaunch(...)` must land on
    # RunnerLaunch. Dropping/nulling the kwarg falls back to InterpreterLaunch — a different class.
    spec = registry.spec_for("js")
    assert spec is not None
    assert isinstance(spec.launch, launch.RunnerLaunch)
    assert not isinstance(spec.launch, launch.InterpreterLaunch)


def test_js_spec_declares_npm_dependency_flavor():
    # deps_flavor="npm": js/ts manage per-script package.json deps. Pins the literal against
    # nulling, casing, and the redundant-kwarg drop (which would revert to the "" base default).
    spec = registry.spec_for("js")
    assert spec is not None
    assert spec.deps_flavor == "npm"
    assert spec.supports_deps is True


# ---------------------------------------------------------------- _javascript_spec fields (ts)


def test_ts_spec_scalar_fields_and_empty_shebangs():
    # The ts kind reuses `_javascript_spec` with kind/glyph "ts"/"✧" and — because `lang != "js"` —
    # an EMPTY shebang row (no `.ts` #! convention). Pins the else-branch of the `lang == "js"`
    # conditional: a flipped/mutated operator would hand ts the js runner triple.
    spec = registry.spec_for("ts")
    assert spec is not None
    assert spec.kind == "ts"
    assert spec.glyph == "✧"  # ✧ — the ts badge glyph
    assert spec.shebangs == ()
    assert spec.deps_flavor == "npm"


# ------------------------------------------- _javascript_spec capability wiring (js, functional)


def test_js_spec_cli_reader_is_wired_and_reads_parseargs():
    # cli_reader=cli_reader_cap: the parseArgs reader must be present AND functional. Nulling or
    # dropping the kwarg leaves cli_reader=None (the base default), so this real read would crash.
    spec = registry.spec_for("js")
    assert spec is not None
    assert spec.cli_reader is not None
    argspec = spec.cli_reader.read_cli('const r = parseArgs({options:{name:{type:"string"}}});\n')
    assert argspec is not None
    assert argspec.ok is True
    assert [f.name for f in argspec.fields] == ["name"]


def test_js_spec_injector_is_wired_and_rewrites_a_const(tmp_path):
    # injector=injector_cap: the const injector must be present AND functional. Nulling/dropping it
    # leaves injector=None (base default) → the call below would crash instead of rewriting.
    spec = registry.spec_for("js")
    assert spec is not None
    assert spec.injector is not None
    request = InjectRequest(
        text='const CITY = "Taipei";\n',
        specs=[ParamDecl(name="CITY", binding="const", type="str")],
        values={"CITY": "Tokyo"},
        entry_dir=tmp_path,
    )
    result = spec.injector.inject(request)
    assert result.path is not None
    assert 'const CITY = "Tokyo";' in result.path.read_text(encoding="utf-8")


def test_js_spec_dep_scanner_is_wired_and_reports_imports():
    # dep_scanner=dep_scanner: the npm import scanner must be present AND functional. Nulling/dropping
    # it leaves dep_scanner=None (base default) → not callable.
    spec = registry.spec_for("js")
    assert spec is not None
    assert spec.dep_scanner is not None
    assert spec.dep_scanner('import x from "cowsay";\nimport y from "left-pad";\n') == [
        "cowsay",
        "left-pad",
    ]


# ---------------------------------------------------------------- _js_analysis lang threading (ts)
#
# Each capability's lambda threads the kind's `lang` into the underlying JS/TS function. Under the ts
# spec that lang is "ts"; any mutant that forces it to None, drops it (default "js"), passes the
# wrong positional, or nulls the callable makes the capability parse TS-only syntax under the JS
# grammar (syntax error) — or crash outright. Each assertion below only holds when "ts" is threaded.


def test_ts_analyzer_parses_typescript_only_syntax():
    # analyze=lambda text: analyzer.analyze(text, lang=lang): under ts the TypeScript grammar accepts
    # `interface …` and still finds the const; forced to js/None it reports a syntax error instead.
    spec = registry.spec_for("ts")
    assert spec is not None
    assert spec.analyzer is not None
    result = spec.analyzer.analyze(_TS_ONLY_CONST)
    assert result.syntax_error is False
    assert [c.name for c in result.candidates] == ["CITY"]


def test_ts_cli_reader_threads_the_typescript_grammar():
    # read_cli=lambda text: cli_reader.read_cli(text, lang=lang): the parseArgs surface sits after a
    # TS-only `interface {}`; only the TS grammar reads it. Under the JS grammar (any lang mutation)
    # the parse errors and read_cli returns None; nulling/wrong-positional mutants raise.
    spec = registry.spec_for("ts")
    assert spec is not None
    assert spec.cli_reader is not None
    argspec = spec.cli_reader.read_cli(_TS_ONLY_PARSEARGS)
    assert argspec is not None
    assert [f.name for f in argspec.fields] == ["name"]


def test_ts_injector_threads_the_typescript_grammar(tmp_path):
    # inject=lambda request: inject.inject(request, lang=lang): injecting into a TS-only script only
    # succeeds under the TS grammar — under JS the post-injection re-parse gate rejects the copy
    # (InjectError). Any lang mutation, or a nulled/wrong-positional callable, breaks this success.
    spec = registry.spec_for("ts")
    assert spec is not None
    assert spec.injector is not None
    request = InjectRequest(
        text=_TS_ONLY_CONST,
        specs=[ParamDecl(name="CITY", binding="const", type="str")],
        values={"CITY": "Tokyo"},
        entry_dir=tmp_path,
    )
    result = spec.injector.inject(request)
    assert result.path is not None
    assert 'const CITY = "Tokyo";' in result.path.read_text(encoding="utf-8")


def test_ts_dep_scanner_threads_the_typescript_grammar():
    # dep_scanner=lambda text: analyzer.external_imports(text, lang=lang): the import sits before a
    # TS-only `interface`; only the TS grammar keeps the file parseable enough to report it. Under
    # the JS grammar external_imports returns []; nulling/wrong-positional mutants return None/raise.
    spec = registry.spec_for("ts")
    assert spec is not None
    assert spec.dep_scanner is not None
    assert spec.dep_scanner(_TS_ONLY_IMPORT) == ["cowsay"]
