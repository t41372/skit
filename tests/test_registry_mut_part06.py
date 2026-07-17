"""Mutation-kill tests for registry._python_spec (chunk 6/9).

Every assertion pins a real, observable property of the Python LangSpec the registry
builds: the data fields consumed by infer_kind / stored_name / the launcher, and the four
capability records (params_io, analyzer, cli_reader, injector) driven through their real
functions on real Python source. A None/renamed/dropped field either changes one of these
observable answers or makes the spec fail to build at all — both are caught here.
"""

from __future__ import annotations

from pathlib import Path

from skit.langs import launch, registry
from skit.langs.base import InjectRequest
from skit.models import Entry, ScriptMeta
from skit.params import ParamDecl


def _python_spec():
    spec = registry.spec_for("python")
    assert spec is not None
    return spec


# ---- data fields (identity, badge, extension/shebang/store wiring) ---------------------------


def test_python_spec_data_fields_exact():
    spec = _python_spec()
    assert spec.kind == "python"
    assert spec.family == "interpreted"
    assert spec.glyph == "⬡"  # ⬡ — the badge glyph
    assert spec.extensions == (".py",)
    assert spec.shebangs == ("python", "python3")
    assert spec.stored_name == "script.py"
    assert spec.comment is not None
    assert spec.comment.prefix == "#"
    assert spec.supports_modes is True
    assert spec.deps_flavor == "uv"
    assert spec.supports_deps is True  # deps_flavor is truthy -> managed deps


def test_python_extension_infers_python_kind(tmp_path: Path):
    # infer_kind consumes spec.extensions via _extension_map — the authoritative signal.
    p = tmp_path / "tool.py"
    p.write_bytes(b"print('hi')\n")
    assert registry.infer_kind(p) == "python"
    # a non-registered extension must NOT resolve to python (pins the extension is EXACTLY ".py").
    other = tmp_path / "tool.PYTHON"
    other.write_bytes(b"print('hi')\n")
    assert registry.infer_kind(other) != "python"


def test_python_shebang_infers_python_kind(tmp_path: Path):
    # infer_kind consumes spec.shebangs via _shebang_map for extension-less files.
    plain = tmp_path / "tool"
    plain.write_bytes(b"#!/usr/bin/python\nprint('hi')\n")
    assert registry.infer_kind(plain) == "python"  # "python" shebang entry
    env3 = tmp_path / "tool3"
    env3.write_bytes(b"#!/usr/bin/env python3\nprint('hi')\n")
    assert registry.infer_kind(env3) == "python"  # "python3" shebang entry


def test_python_stored_name_is_pinned():
    # stored_name() reads spec.stored_name; existing stores carry "script.py" on disk.
    assert registry.stored_name("python") == "script.py"


# ---- launch wiring ---------------------------------------------------------------------------


def test_python_launch_is_the_uv_strategy(tmp_path: Path):
    spec = _python_spec()
    assert isinstance(spec.launch, launch.UvLaunch)
    entry = Entry(
        slug="thing", meta=ScriptMeta(name="thing", kind="python"), dir=tmp_path / "thing"
    )
    described = spec.launch.describe(entry, [], None, None)
    # The uv --script contract python scripts launch under.
    assert "run" in described
    assert "--no-project" in described
    assert "--script" in described


# ---- params_io capability (read/write [tool.skit]) -------------------------------------------


def test_python_params_io_round_trips_declarations():
    spec = _python_spec()
    assert spec.params_io is not None
    decl = ParamDecl(name="COUNT", binding="const", delivery="inject", type="int", default=5)
    written = spec.params_io.write("print('hi')\n", [decl])
    assert "COUNT" in written  # write embedded the declaration in a [tool.skit] block
    read_back = spec.params_io.read(written)
    assert [p.name for p in read_back] == ["COUNT"]
    assert read_back[0].binding == "const"
    assert read_back[0].type == "int"
    assert read_back[0].default == 5


# ---- analyzer capability (analyze + reconcile) -----------------------------------------------


def test_python_analyzer_detects_and_reconciles_a_constant():
    spec = _python_spec()
    assert spec.analyzer is not None
    text = "COUNT = 5\nprint(COUNT)\n"
    analysis = spec.analyzer.analyze(text)
    assert "COUNT" in [c.name for c in analysis.candidates]
    decl = ParamDecl(name="COUNT", binding="const", delivery="inject", type="int", default=5)
    report = spec.analyzer.reconcile(text, [decl])
    assert report.syntax_error is False
    assert report.missing == []  # the declared const still anchors in the source
    assert "COUNT" in [p.name for p in report.ok]


# ---- cli_reader capability (static argparse surface) -----------------------------------------


def test_python_cli_reader_reads_argparse_flags():
    spec = _python_spec()
    assert spec.cli_reader is not None
    text = "import argparse\np = argparse.ArgumentParser()\np.add_argument('--width')\n"
    arg_spec = spec.cli_reader.read_cli(text)
    assert arg_spec is not None
    assert [f.flag for f in arg_spec.fields] == ["--width"]


# ---- injector capability (const rewrite in a temp copy) --------------------------------------


def test_python_injector_rewrites_a_constant_value(tmp_path: Path):
    spec = _python_spec()
    assert spec.injector is not None
    text = "COUNT = 5\nprint(COUNT)\n"
    decl = ParamDecl(name="COUNT", binding="const", delivery="inject", type="int", default=5)
    request = InjectRequest(text=text, specs=[decl], values={"COUNT": "42"}, entry_dir=tmp_path)
    result = spec.injector.inject(request)
    assert result.path is not None
    try:
        injected = result.path.read_text(encoding="utf-8")
    finally:
        result.path.unlink(missing_ok=True)
    assert "COUNT = 42" in injected  # the injected copy carries the new value
