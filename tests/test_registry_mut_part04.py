"""Mutation-kill tests for src/skit/langs/registry.py — chunk 4/9.

Pins the data rows of three interpreted kinds — js, lua, perl — through the public
registry surface. Each ``spec_for`` field assertion is a real registry contract (the
same shape as tests/test_langs.py's completeness gate), and the ``infer_kind`` cases
exercise the add-time inference paths those rows feed (``_extension_map`` /
``_shebang_map``): a user who adds a ``.lua`` file, or a ``#!/usr/bin/env node`` script,
gets exactly the kind the data row promises.
"""

from __future__ import annotations

from pathlib import Path

from skit.langs import registry


def _write(tmp_path: Path, name: str, data: bytes) -> Path:
    p = tmp_path / name
    p.write_bytes(data)
    return p


# ---- js (_javascript_spec("js", "✦", (".js", ".mjs", ".cjs"), "js")) --------------------------


def test_js_spec_identity_glyph_extensions_and_shebangs():
    spec = registry.spec_for("js")
    assert spec is not None
    assert spec.kind == "js"
    assert spec.glyph == "✦"
    assert spec.extensions == (".js", ".mjs", ".cjs")
    # lang == "js" is precisely what turns the node/deno/bun runner shebangs on; any other
    # value (None, "JS", "XXjsXX") takes the `else ()` branch and empties them.
    assert spec.shebangs == ("node", "deno", "bun")


def test_js_extensions_and_node_shebang_infer_js(tmp_path: Path):
    # The three extension rows and the runner-shebang row drive add-time kind inference.
    assert registry.infer_kind(_write(tmp_path, "a.js", b"console.log(1)\n")) == "js"
    assert registry.infer_kind(_write(tmp_path, "b.mjs", b"export {}\n")) == "js"
    assert registry.infer_kind(_write(tmp_path, "c.cjs", b"module.exports = {}\n")) == "js"
    node = _write(tmp_path, "runme", b"#!/usr/bin/node\nconsole.log(1)\n")
    assert registry.infer_kind(node) == "js"


# ---- lua (_interpreted("lua", "○", "lua", (".lua",), ("lua", "luajit"), "--")) -----------------


def test_lua_spec_full_data_row():
    # A dropped positional argument (or extensions=None) can't even build the spec — it
    # raises before returning — so reaching these assertions already pins the call's arity.
    spec = registry.spec_for("lua")
    assert spec is not None
    assert spec.kind == "lua"
    assert spec.glyph == "○"
    assert spec.default_interpreter == "lua"
    assert spec.extensions == (".lua",)
    assert spec.shebangs == ("lua", "luajit")
    assert spec.comment is not None
    assert spec.comment.prefix == "--"


def test_lua_extension_and_shebangs_infer_lua(tmp_path: Path):
    assert registry.infer_kind(_write(tmp_path, "a.lua", b"print('hi')\n")) == "lua"
    for interp in (b"lua", b"luajit"):
        script = _write(tmp_path, "s", b"#!/usr/bin/" + interp + b"\nprint('hi')\n")
        assert registry.infer_kind(script) == "lua"


# ---- perl (_interpreted("perl", "◈", "perl", (".pl",), ("perl",), "#")) ------------------------


def test_perl_spec_data_row():
    spec = registry.spec_for("perl")
    assert spec is not None
    assert spec.glyph == "◈"
    assert spec.default_interpreter == "perl"
    assert spec.extensions == (".pl",)
    assert spec.shebangs == ("perl",)
    assert spec.comment is not None
    assert spec.comment.prefix == "#"


def test_perl_extension_and_shebang_infer_perl(tmp_path: Path):
    assert registry.infer_kind(_write(tmp_path, "a.pl", b"print 1;\n")) == "perl"
    script = _write(tmp_path, "s", b"#!/usr/bin/perl\nprint 1;\n")
    assert registry.infer_kind(script) == "perl"
