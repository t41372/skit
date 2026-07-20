"""Mutation-kill tests for src/skit/langs/registry.py (chunk 2/9).

Pins the fish LangSpec (data row + the four analysis capabilities wired on top of the
Tier-0 interpreted base), the shared `_interpreted` builder's field assignment (exercised
through the ruby row and the powershell `-File` prefix), and `_is_executable_file`'s Windows
PATHEXT fallback. Every assertion drives real registry API through a real code path — the
fish analyzer/cli_reader/metawriter, the launch strategy's own describe, and the executable
sniff — never a mock of the unit under test.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit.langs import registry
from skit.models import Entry, ScriptMeta
from skit.params import ParamDecl


def _entry(tmp_path: Path, kind: str) -> Entry:
    return Entry(slug="e", meta=ScriptMeta(name="e", kind=kind), dir=tmp_path / "e")


# ---- fish: the Tier-0 interpreted data row --------------------------------------------------


def test_fish_spec_data_fields():
    # The exact _interpreted("fish", "∿", "fish", (".fish",), ("fish",), "#") data row survives
    # the replace() overlay: every positional carries through, and a dropped/None arg either
    # crashes the builder or lands the wrong value here.
    spec = registry.spec_for("fish")
    assert spec is not None
    assert spec.kind == "fish"
    assert spec.glyph == "∿"
    assert spec.default_interpreter == "fish"
    assert spec.extensions == (".fish",)
    assert spec.shebangs == ("fish",)
    assert spec.comment is not None
    assert spec.comment.prefix == "#"
    assert spec.stored_name == "script.fish"  # f"script{extensions[0]}"
    assert spec.supports_modes is True


def test_fish_spec_wires_the_four_capabilities():
    # replace() lifts fish from the bare interpreted base to a params_io + analyzer + cli_reader
    # kind; dropping any of those kwargs (or Noneing them) leaves the interpreted base's None.
    spec = registry.spec_for("fish")
    assert spec is not None
    assert spec.params_io is not None
    assert spec.analyzer is not None
    assert spec.cli_reader is not None
    # fish v1 deliberately ships no injector/normalizer (env delivery needs neither).
    assert spec.injector is None
    assert spec.normalizer is None


def test_fish_params_io_round_trips_declared_params():
    # The '#'-block metawriter is wired as fish's params_io: writing a decl embeds it in the
    # [tool.skit] block and reading it back recovers the name.
    spec = registry.spec_for("fish")
    assert spec is not None
    assert spec.params_io is not None
    written = spec.params_io.write("#!/usr/bin/env fish\necho hi\n", [ParamDecl(name="WIDTH")])
    assert "WIDTH" in written
    assert [d.name for d in spec.params_io.read(written)] == ["WIDTH"]


def test_fish_analyzer_detects_env_default_and_reconciles():
    # The fish analyzer's analyze() flags the `set -q NAME; or set NAME value` env idiom, and
    # reconcile() reports the same detection as a new (unmanaged) candidate.
    spec = registry.spec_for("fish")
    assert spec is not None
    assert spec.analyzer is not None
    src = "set -q WIDTH\nor set WIDTH 80\n"
    analysis = spec.analyzer.analyze(src)
    assert any(c.name == "WIDTH" and c.binding == "envdefault" for c in analysis.candidates)
    report = spec.analyzer.reconcile(src, [])
    assert any(c.name == "WIDTH" for c in report.new)


def test_fish_cli_reader_reads_argparse_specs():
    # fish's cli_reader parses the builtin `argparse` spec strings into flag fields.
    spec = registry.spec_for("fish")
    assert spec is not None
    assert spec.cli_reader is not None
    argspec = spec.cli_reader.read_cli("argparse 'n/name=' -- $argv\n")
    assert argspec is not None
    assert any(f.name == "name" and f.flag == "--name" for f in argspec.fields)


# ---- the shared _interpreted builder (exercised through ruby + powershell) -------------------


def test_ruby_row_carries_interpreted_fields():
    # ruby is a straight _interpreted(...) row, so it pins the builder's field assignment:
    # extensions/shebangs/default_interpreter carry the args, supports_modes is fixed True,
    # and none of them fall back to the LangSpec defaults ((), (), "", False).
    spec = registry.spec_for("ruby")
    assert spec is not None
    assert spec.extensions == (".rb",)
    assert spec.shebangs == ("ruby",)
    assert spec.default_interpreter == "ruby"
    assert spec.stored_name == "script.rb"
    assert spec.supports_modes is True


def test_powershell_launch_keeps_the_file_prefix(tmp_path: Path):
    # powershell passes prefix=("-File",) into _interpreted, which threads it to
    # InterpreterLaunch; dropping/None-ing the prefix wiring loses -File from the command.
    spec = registry.spec_for("powershell")
    assert spec is not None
    described = spec.launch.describe(_entry(tmp_path, "powershell"), [], None, None)
    assert "-File" in described


# ---- _is_executable_file: the Windows PATHEXT fallback --------------------------------------


def test_win_pathext_fallback_recognizes_conventional_extensions(monkeypatch: pytest.MonkeyPatch):
    # On Windows with PATHEXT unset, the conventional ".COM;.EXE;.BAT;.CMD" fallback decides
    # runnability by extension (compared case-insensitively): a .com is executable, a .txt is not.
    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.delenv("PATHEXT", raising=False)
    assert registry._is_executable_file(Path("tool.com")) is True
    assert registry._is_executable_file(Path("run.cmd")) is True
    assert registry._is_executable_file(Path("notes.txt")) is False
