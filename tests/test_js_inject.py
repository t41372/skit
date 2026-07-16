"""JS/TS injector contract: const value delivery, quoting normalization (number/string/bool +
json.dumps escaping), same-name multi-occurrence, drift vs bad-value, the mandatory offline
re-parse gate and the best-effort `node --check` gate, plus flows.execute integration and
runner-gated end-to-end execution.

The offline (analysis/injection) tests run everywhere and cover every code line without assuming a
JS runtime. The execution tests are SKIP-gated on a runner (node/deno/bun) being present — when one
is, the injected value is proven to actually reach the child process.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, flows, store
from skit.langs.base import (
    InjectError,
    InjectRequest,
    InjectResult,
    InjectSyntaxError,
    InjectValueError,
)
from skit.langs.javascript import analyzer, inject
from skit.params import ParamDecl

runner = CliRunner()

_RUNNER = shutil.which("node") or shutil.which("deno") or shutil.which("bun")
runner_present = pytest.mark.skipif(_RUNNER is None, reason="no JS runtime installed")
posix_or_win = pytest.mark.skipif(sys.platform == "win32", reason="POSIX file-mode assertion")


def specs_of(src: str, *, lang: str = "js") -> list[ParamDecl]:
    return [ParamDecl.from_candidate(c) for c in analyzer.analyze(src, lang=lang).candidates]


def inject_src(
    src: str,
    values: dict[str, str],
    tmp_path: Path,
    *,
    specs: list[ParamDecl] | None = None,
    lang: str = "js",
    interpreter: str = "",
    source: str = "",
) -> InjectResult:
    return inject.inject(
        InjectRequest(
            text=src,
            specs=specs_of(src, lang=lang) if specs is None else specs,
            values=values,
            entry_dir=tmp_path,
            interpreter=interpreter,
            source=source,
        ),
        lang=lang,
    )


def temp_files(tmp_path: Path) -> list[Path]:
    return list(tmp_path.glob(".injected-*"))


def run_js(path: Path, cwd: Path) -> str:
    """Run the injected copy under whatever runner is installed, returning its stdout."""
    from skit.langs.launch import RunnerLaunch

    for name in ("node", "deno", "bun"):
        program = shutil.which(name)
        if program is None:  # pragma: no cover — depends on the host's installed runtimes
            continue
        import subprocess

        argv = [program, *RunnerLaunch._INVOKE.get(name, ()), str(path)]
        return subprocess.run(argv, capture_output=True, text=True, cwd=cwd, check=False).stdout
    pytest.skip("no runner")  # pragma: no cover — guarded by runner_present


# ---------------------------------------------------------------- const quoting


def test_int_injects_a_bare_number(tmp_path):
    result = inject_src("const W = 800;\n", {"W": "1200"}, tmp_path)
    assert result.path is not None
    assert "const W = 1200;" in result.path.read_text(encoding="utf-8")


def test_float_injects_a_bare_number(tmp_path):
    result = inject_src("const R = 0.5;\n", {"R": "2.75"}, tmp_path)
    assert result.path is not None
    assert "const R = 2.75;" in result.path.read_text(encoding="utf-8")


def test_string_injects_a_json_dumps_literal(tmp_path):
    result = inject_src('const C = "x";\n', {"C": "New York"}, tmp_path)
    assert result.path is not None
    assert 'const C = "New York";' in result.path.read_text(encoding="utf-8")


def test_string_json_escapes_quote_backslash_newline(tmp_path):
    result = inject_src('const M = "x";\n', {"M": 'a"b\\c\nd'}, tmp_path)
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert 'const M = "a\\"b\\\\c\\nd";' in text  # quote, backslash, newline all escaped
    assert not analyzer.analyze(text).syntax_error


def test_cjk_and_emoji_escape_to_valid_js(tmp_path):
    result = inject_src('const C = "x";\n', {"C": "高雄 🚀"}, tmp_path)
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert "\\u9ad8" in text  # json.dumps escapes non-ASCII (ensure_ascii)
    assert not analyzer.analyze(text).syntax_error


def test_bool_injects_true_or_false_lowercase(tmp_path):
    on = inject_src("const B = false;\n", {"B": "yes"}, tmp_path)
    assert on.path is not None
    assert "const B = true;" in on.path.read_text(encoding="utf-8")
    off = inject_src("const B = true;\n", {"B": "0"}, tmp_path)
    assert off.path is not None
    assert "const B = false;" in off.path.read_text(encoding="utf-8")


def test_rewrites_every_same_name_occurrence(tmp_path):
    src = "var M = 1;\nvar M = 2;\nconsole.log(M);\n"
    result = inject_src(src, {"M": "9"}, tmp_path)
    assert result.path is not None
    assert result.path.read_text(encoding="utf-8").count("= 9;") == 2


def test_same_name_nonliteral_declaration_is_not_a_target(tmp_path):
    # A later `var M = compute()` (non-literal) is not a rewrite target — only literal ones are.
    src = "var M = 1;\nvar M = compute();\n"
    result = inject_src(src, {"M": "9"}, tmp_path)
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert "var M = 9;" in text
    assert "var M = compute();" in text  # untouched


def test_ts_temp_copy_has_ts_suffix(tmp_path):
    result = inject_src("const N: number = 5;\n", {"N": "7"}, tmp_path, lang="ts")
    assert result.path is not None
    assert result.path.suffix == ".ts"
    assert "const N: number = 7;" in result.path.read_text(encoding="utf-8")


@pytest.mark.parametrize(
    ("source", "lang", "expected"),
    [
        ("tool.mjs", "js", ".mjs"),  # ESM origin → node reads the copy as ESM by extension
        ("tool.cjs", "js", ".cjs"),  # CommonJS origin
        ("plain.js", "js", ".js"),  # no explicit flavor → plain extension
        ("", "js", ".js"),  # no origin recorded
        ("tool.mts", "ts", ".mts"),
        ("tool.cts", "ts", ".cts"),
        ("plain.ts", "ts", ".ts"),
    ],
)
def test_injected_copy_carries_the_origins_module_flavor(tmp_path, source, lang, expected):
    # The store flattens the stored copy to script.js/script.ts; the temp copy re-encodes the
    # origin's .mjs/.cjs flavor so node resolves the module type from the extension — before any
    # package.json exists and independent of node's version.
    src = "const N = 5;\n" if lang == "js" else "const N: number = 5;\n"
    result = inject_src(src, {"N": "7"}, tmp_path, lang=lang, source=source)
    assert result.path is not None
    assert result.path.suffix == expected


# ---------------------------------------------------------------- drift / bad value


def test_missing_target_is_drift_not_value_error(tmp_path):
    spec = ParamDecl(name="GONE", binding="const", delivery="inject", type="str")
    with pytest.raises(InjectError) as exc_info:
        inject_src("const W = 800;\n", {"GONE": "x"}, tmp_path, specs=[spec])
    assert "GONE" in str(exc_info.value)
    assert not isinstance(exc_info.value, InjectValueError)
    assert not temp_files(tmp_path)


def test_bad_int_value_raises_value_error(tmp_path):
    with pytest.raises(InjectValueError) as exc_info:
        inject_src("const W = 800;\n", {"W": "not-a-number"}, tmp_path)
    assert exc_info.value.param_name == "W"
    assert not temp_files(tmp_path)


def test_bad_float_and_non_finite_are_refused(tmp_path):
    for bad in ("abc", "inf", "-inf", "nan"):
        with pytest.raises(InjectValueError):
            inject_src("const R = 0.5;\n", {"R": bad}, tmp_path)


def test_bad_bool_value_raises_value_error(tmp_path):
    with pytest.raises(InjectValueError):
        inject_src("const B = true;\n", {"B": "maybe"}, tmp_path)


def test_no_values_writes_nothing(tmp_path):
    result = inject_src("const W = 800;\n", {}, tmp_path)
    assert result.path is None
    assert result.env == {}
    assert not temp_files(tmp_path)


def test_value_for_unmanaged_name_is_ignored(tmp_path):
    # A value whose key isn't a managed spec never produces a span.
    result = inject_src("const W = 800;\n", {"OTHER": "x"}, tmp_path, specs=[])
    assert result.path is None


# ---------------------------------------------------------------- gate 1 (offline)


def test_offline_gate_refuses_a_corrupted_injection(tmp_path, monkeypatch):
    monkeypatch.setattr(inject, "escape_string", lambda value: f'"{value}')  # unterminated string
    with pytest.raises(InjectSyntaxError):
        inject_src('const T = "hi";\n', {"T": "x"}, tmp_path)
    assert not temp_files(tmp_path)


# ---------------------------------------------------------------- gate 2 (node --check)


def test_resolve_runner_finds_first_installed(monkeypatch):
    monkeypatch.setattr(inject.shutil, "which", lambda n: "/x/node" if n == "node" else None)
    assert inject._resolve_runner("") == ("node", "/x/node")  # deno/bun absent, node found


def test_resolve_runner_none_when_nothing_installed(monkeypatch):
    monkeypatch.setattr(inject.shutil, "which", lambda _n: None)
    assert inject._resolve_runner("") == (None, None)


def test_resolve_runner_respects_pinned_interpreter_and_normalizes(monkeypatch):
    monkeypatch.setattr(inject.shutil, "which", lambda n: "/abs/" + n)
    assert inject._resolve_runner("node") == ("node", "/abs/node")
    # a path/.exe interpreter normalizes to a bare runner name
    assert inject._resolve_runner("dir/node.exe") == ("node", "/abs/dir/node.exe")


def test_gate_node_skips_ts_suffix(tmp_path, monkeypatch):
    called = {"run": False}
    monkeypatch.setattr(inject.subprocess, "run", lambda *a, **k: called.__setitem__("run", True))
    inject._gate_node("node", tmp_path / "x.ts", ".ts")  # .ts is not node-checkable
    assert called["run"] is False


@pytest.mark.skipif(shutil.which("node") is None, reason="node required for gate 2")
def test_mjs_origin_esm_copy_survives_gate2_before_any_package_json(tmp_path, monkeypatch):
    """Regression for the bricking bug: an ESM .mjs-origin entry is injected and gate 2
    (`node --check`) runs BEFORE the deps install writes a "type": "module" package.json. The .mjs
    temp-copy extension makes node read the copy as ESM even with auto-detect off (node <22.7); with
    the pre-fix .js extension node would reject the `import` as CommonJS → InjectSyntaxError."""
    node = shutil.which("node")
    assert node is not None
    if (
        subprocess.run(  # skip if this node predates the auto-detect flag entirely
            [node, "--no-experimental-detect-module", "-e", ""], capture_output=True, check=False
        ).returncode
        != 0
    ):
        pytest.skip("this node predates --no-experimental-detect-module")
    monkeypatch.setenv("NODE_OPTIONS", "--no-experimental-detect-module")  # simulate node <22.7
    src = 'import assert from "node:assert";\nconst N = 5;\nassert.ok(N);\n'
    result = inject_src(src, {"N": "7"}, tmp_path, interpreter="node", source="orig.mjs")
    assert result.path is not None
    assert result.path.suffix == ".mjs"  # gate 2 accepted the ESM copy, no InjectSyntaxError


def test_gate_node_skips_when_runner_is_not_node(tmp_path, monkeypatch):
    monkeypatch.setattr(inject, "_resolve_runner", lambda _i: ("deno", "/x/deno"))
    monkeypatch.setattr(
        inject.subprocess, "run", lambda *a, **k: pytest.fail("node --check must not run")
    )
    inject._gate_node("", tmp_path / "x.js", ".js")  # returns without spawning


def test_gate_node_skips_when_no_runner_installed(tmp_path, monkeypatch):
    monkeypatch.setattr(inject, "_resolve_runner", lambda _i: (None, None))
    inject._gate_node("", tmp_path / "x.js", ".js")  # program is None -> return


def _fake_node(monkeypatch, returncode: int, stderr: bytes = b""):
    monkeypatch.setattr(inject, "_resolve_runner", lambda _i: ("node", "/fake/node"))

    class _Proc:
        def __init__(self) -> None:
            self.returncode = returncode
            self.stderr = stderr

    monkeypatch.setattr(inject.subprocess, "run", lambda *a, **k: _Proc())


def test_gate_node_passes_on_returncode_zero(tmp_path, monkeypatch):
    _fake_node(monkeypatch, 0)
    inject._gate_node("node", tmp_path / "x.js", ".js")  # no raise


def test_gate_node_raises_on_nonzero(tmp_path, monkeypatch):
    _fake_node(monkeypatch, 1, b"SyntaxError: boom\n")
    with pytest.raises(InjectSyntaxError) as exc_info:
        inject._gate_node("node", tmp_path / "x.js", ".js")
    assert "boom" in str(exc_info.value)


def test_gate_node_raises_on_nonzero_with_empty_stderr(tmp_path, monkeypatch):
    _fake_node(monkeypatch, 1, b"")
    with pytest.raises(InjectSyntaxError):
        inject._gate_node("node", tmp_path / "x.js", ".js")


def test_gate_node_survives_a_spawn_failure(tmp_path, monkeypatch):
    monkeypatch.setattr(inject, "_resolve_runner", lambda _i: ("node", "/fake/node"))

    def boom(*_a, **_k):
        raise OSError("no fork")

    monkeypatch.setattr(inject.subprocess, "run", boom)
    inject._gate_node("node", tmp_path / "x.js", ".js")  # returns; gate 1 already vouched


def test_gate2_failure_removes_the_temp_copy(tmp_path, monkeypatch):
    # Skip gate 1, force a node-check rejection: the written copy must not survive on disk.
    monkeypatch.setattr(inject, "_gate_reparse", lambda out, lang: None)
    _fake_node(monkeypatch, 1, b"boom")
    with pytest.raises(InjectSyntaxError):
        inject_src('const T = "hi";\n', {"T": "x"}, tmp_path, interpreter="node")
    assert not temp_files(tmp_path)


# ---------------------------------------------------------------- secret handling


@posix_or_win
def test_injected_copy_is_0600(tmp_path):
    result = inject_src('const API_KEY = "changeme";\n', {"API_KEY": "s3cr3t"}, tmp_path)
    assert result.path is not None
    assert result.path.stat().st_mode & 0o777 == 0o600
    result.path.unlink()


# ---------------------------------------------------------------- execution (runner-gated)


def _js_entry(tmp_path: Path, text: str, *, name: str, ext: str = "js") -> store.Entry:
    src = tmp_path / f"{name}.{ext}"
    src.write_text(text, encoding="utf-8")
    kind = "ts" if ext == "ts" else "js"
    return store.add_script(src, kind=kind, name=name)


@runner_present
def test_injected_const_reaches_the_child(tmp_path):
    result = inject_src(
        'const WIDTH = 800;\nconsole.log("w=" + WIDTH);\n', {"WIDTH": "1200"}, tmp_path
    )
    assert result.path is not None
    assert run_js(result.path, tmp_path).strip() == "w=1200"


@runner_present
def test_injected_string_reaches_the_child(tmp_path):
    result = inject_src('const CITY = "here";\nconsole.log(CITY);\n', {"CITY": "台北 🚀"}, tmp_path)
    assert result.path is not None
    assert run_js(result.path, tmp_path).strip() == "台北 🚀"


@runner_present
def test_run_injects_and_executes_end_to_end(tmp_path, capfd):
    _js_entry(tmp_path, 'const WIDTH = 800;\nconsole.log("w=" + WIDTH);\n', name="jsrun1")
    assert runner.invoke(cli.app, ["params", "jsrun1", "--manage", "WIDTH"]).exit_code == 0
    result = runner.invoke(cli.app, ["run", "jsrun1", "--set", "WIDTH=1200", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "w=1200" in capfd.readouterr().out


# ---------------------------------------------------------------- flows.execute integration


def test_execute_runs_a_js_entry_offline_plan(tmp_path):
    # Even without a runner, plan + inject + gate must succeed up to the launch boundary.
    entry = _js_entry(tmp_path, "const WIDTH = 800;\nconsole.log(WIDTH);\n", name="jsx1")
    assert runner.invoke(cli.app, ["params", "jsx1", "--manage", "WIDTH"]).exit_code == 0
    entry = store.resolve("jsx1")
    plan = flows.plan_for_entry(entry)
    assert plan.source == "inject"


def test_execute_maps_a_drifted_js_definition_to_drift(tmp_path):
    entry = _js_entry(tmp_path, "const TALL = 800;\n", name="jsx2")
    plan = flows.FormPlan(
        source="inject",
        fields=[flows.FormField(key="WIDTH", label="WIDTH")],
        specs=[ParamDecl(name="WIDTH", binding="const", delivery="inject", type="int")],
        text=entry.script_path.read_text(encoding="utf-8"),
    )
    outcome = flows.execute(
        entry, plan, flows.Assembly(inject_values={"WIDTH": "1200"}), emit=lambda _line: None
    )
    assert outcome.failure == flows.FAIL_DRIFT
    assert "--resync" in outcome.message
    assert not temp_files(entry.dir)


def test_execute_refuses_a_bad_value_before_launch(tmp_path):
    _js_entry(tmp_path, "const WIDTH = 800;\n", name="jsx3")
    assert runner.invoke(cli.app, ["params", "jsx3", "--manage", "WIDTH"]).exit_code == 0
    bad = runner.invoke(cli.app, ["run", "jsx3", "--set", "WIDTH=abc", "--no-input"])
    assert bad.exit_code == flows.FAILURE_EXIT_CODES[flows.FAIL_BAD_VALUE]


def test_execute_syntax_gate_failure_never_launches(tmp_path, monkeypatch):
    from skit import launcher

    monkeypatch.setattr(inject, "escape_string", lambda value: f'"{value}')
    monkeypatch.setattr(
        launcher, "run_entry", lambda *a, **k: pytest.fail("the script must not launch")
    )
    entry = _js_entry(tmp_path, 'const TITLE = "hello";\n', name="jsx4")
    assert runner.invoke(cli.app, ["params", "jsx4", "--manage", "TITLE"]).exit_code == 0
    entry = store.resolve("jsx4")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {"TITLE": "x"}, [], cwd=tmp_path)
    outcome = flows.execute(entry, plan, asm, emit=lambda _line: None)
    assert outcome.failure == flows.FAIL_DRIFT
    assert "--resync" not in outcome.message  # a resync cannot fix skit's own corruption
    assert not temp_files(entry.dir)
