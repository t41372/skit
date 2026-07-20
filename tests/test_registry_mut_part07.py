"""Mutation-kill tests for the ruby and R rows of the language registry
(`src/skit/langs/registry.py` — `_ruby_spec` / `_r_spec`).

Both specs are single `_interpreted(...)` data rows, so every one of their fields is a
positional literal a consumer depends on: the badge glyph, the default interpreter that
launch falls back to, the extensions/shebangs that add-time inference maps to the kind,
and the `#` comment prefix the in-file metadata block rides on. Each test pins one of
those through a real path — `spec_for` (the registry's public resolver), `infer_kind`
(the add-time extension/shebang consumer) and the wired-up `InterpreterLaunch` (the real
launch command line). Nothing is mocked; the specs are exercised exactly as skit uses
them.
"""

from __future__ import annotations

from pathlib import Path

from skit.langs import registry as reg
from skit.models import Entry, ScriptMeta


def _no_ext_file(tmp_path: Path, name: str, first_line: str) -> Path:
    """A shebang'd file with NO recognized extension and NO execute bit, so infer_kind must
    reach the shebang branch (not the extension map, not the exec-bit fallback)."""
    p = tmp_path / name
    p.write_bytes(f"{first_line}\n".encode())
    return p


def _entry(tmp_path: Path, kind: str) -> Entry:
    return Entry(slug="t", meta=ScriptMeta(name="t", kind=kind), dir=tmp_path / "t")


# ---- ruby -------------------------------------------------------------------------------------


def test_ruby_spec_has_the_expected_row():
    spec = reg.spec_for("ruby")
    assert spec is not None
    assert spec.kind == "ruby"
    assert spec.glyph == "◆"
    assert spec.default_interpreter == "ruby"  # launch falls back to this when meta has none
    assert spec.extensions == (".rb",)
    assert spec.shebangs == ("ruby",)
    assert spec.comment is not None
    assert spec.comment.prefix == "#"  # the '#' block engine carries in-file metadata
    assert spec.stored_name == "script.rb"  # f"script{extensions[0]}"
    assert spec.family == "interpreted"
    assert spec.supports_modes


def test_ruby_inferred_from_rb_extension(tmp_path: Path):
    p = tmp_path / "widget.rb"
    p.write_text('puts "hi"\n', encoding="utf-8")
    assert reg.infer_kind(p) == "ruby"  # the (".rb",) row reaches _extension_map


def test_ruby_inferred_from_ruby_shebang(tmp_path: Path):
    # No extension + no +x bit: only the ("ruby",) shebang row can name this kind.
    p = _no_ext_file(tmp_path, "widget", "#!/usr/bin/ruby")
    assert reg.infer_kind(p) == "ruby"


def test_ruby_launch_command_names_the_ruby_interpreter(tmp_path: Path):
    spec = reg.spec_for("ruby")
    assert spec is not None
    described = spec.launch.describe(_entry(tmp_path, "ruby"), [], None, None)
    assert described.split()[0] == "ruby"  # the interpreter reached the real launch line


# ---- r ----------------------------------------------------------------------------------------


def test_r_spec_has_the_expected_row():
    spec = reg.spec_for("r")
    assert spec is not None
    assert spec.kind == "r"
    assert spec.glyph == "◇"
    assert spec.default_interpreter == "Rscript"  # capitalized binary name, verbatim
    assert spec.extensions == (".r",)
    assert spec.shebangs == ("Rscript",)
    assert spec.comment is not None
    assert spec.comment.prefix == "#"
    assert spec.stored_name == "script.r"
    assert spec.family == "interpreted"
    assert spec.supports_modes


def test_r_inferred_from_r_extension(tmp_path: Path):
    p = tmp_path / "plot.r"
    p.write_text('cat("hi")\n', encoding="utf-8")
    assert reg.infer_kind(p) == "r"


def test_r_inferred_from_rscript_shebang(tmp_path: Path):
    # The shebang program is "Rscript" verbatim (case-sensitive lookup in _shebang_map).
    p = _no_ext_file(tmp_path, "plot", "#!/usr/bin/Rscript")
    assert reg.infer_kind(p) == "r"


def test_r_launch_command_names_the_rscript_interpreter(tmp_path: Path):
    spec = reg.spec_for("r")
    assert spec is not None
    described = spec.launch.describe(_entry(tmp_path, "r"), [], None, None)
    assert described.split()[0] == "Rscript"
