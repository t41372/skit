"""Mutation-kill pins for src/skit/langs/registry.py, chunk 8/9.

Covers the two data-heavy builders in this chunk — ``_shell_spec`` and ``_ts_spec`` — plus
the shell kind's four tree-sitter-backed capabilities (analyzer/reconcile, injector,
normalizer, cli_reader) and its import-guard degradation branch. Every capability assertion
drives the real function through the registry's public ``spec_for`` surface, so a wired
capability that has been nulled out (or a builder field silently altered) is observable.
English catalog is assumed for any message text.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from skit.langs import registry
from skit.langs.base import InjectRequest
from skit.params import ParamDecl

# ---------------------------------------------------------------- _shell_spec: identity data


def test_shell_spec_identity_fields():
    # The Tier-0 data row for shell, threaded through _interpreted(...): kind, badge glyph,
    # default interpreter, the recognized extensions/shebangs, and the '#'-comment prefix.
    spec = registry.spec_for("shell")
    assert spec is not None
    assert spec.kind == "shell"
    assert spec.glyph == "#"
    assert spec.default_interpreter == "bash"
    assert spec.extensions == (".sh", ".bash", ".zsh")
    assert spec.shebangs == ("bash", "sh", "zsh", "dash", "ash", "ksh")
    assert spec.comment is not None
    assert spec.comment.prefix == "#"
    # stored copy name is derived from the first extension.
    assert spec.stored_name == "script.sh"


def test_shell_extension_inference_maps_each_extension_to_shell():
    # The extensions tuple is the add-time inference table: each recognized suffix names "shell".
    for name in ("deploy.sh", "lib.bash", "prompt.zsh"):
        assert registry.infer_kind(Path(name)) == "shell"


def test_shell_shebang_inference(tmp_path: Path):
    # Each recognized #! program basename is inferred as shell (extension-less files).
    for program in ("bash", "sh", "zsh", "dash", "ash", "ksh"):
        script = tmp_path / f"tool_{program}"
        script.write_bytes(f"#!/usr/bin/env {program}\necho hi\n".encode())
        assert registry.infer_kind(script) == "shell"


# ---------------------------------------------------------------- _shell_spec: wired capabilities

_SHELL_SRC = 'NAME=world\necho "hi $NAME"\n'


def _shell_specs():
    spec = registry.spec_for("shell")
    assert spec is not None
    assert spec.analyzer is not None
    return spec, [ParamDecl.from_candidate(c) for c in spec.analyzer.analyze(_SHELL_SRC).candidates]


def test_shell_analyzer_reconcile_is_wired():
    # spec.analyzer.reconcile must be the real drift reconciler: a matching const reconciles
    # clean, while a spec with no source anchor is reported as drift.
    spec, specs = _shell_specs()
    assert spec.analyzer is not None
    ok_report = spec.analyzer.reconcile(_SHELL_SRC, specs)
    assert [p.name for p in ok_report.ok] == ["NAME"]
    assert ok_report.has_drift is False

    gone = [ParamDecl(name="GONE", binding="const", delivery="inject", type="str", default="x")]
    drift_report = spec.analyzer.reconcile(_SHELL_SRC, gone)
    assert [p.name for p in drift_report.missing] == ["GONE"]
    assert drift_report.has_drift is True


def test_shell_injector_is_wired(tmp_path: Path):
    # spec.injector.inject must rewrite the stored const to the supplied value in a temp copy.
    spec, specs = _shell_specs()
    assert spec.injector is not None
    request = InjectRequest(
        text=_SHELL_SRC,
        specs=specs,
        values={"NAME": "bob"},
        entry_dir=tmp_path,
        interpreter="",  # skip the interpreter syntax gate (Windows may have no bash)
    )
    result = spec.injector.inject(request)
    assert result.path is not None
    injected = Path(result.path)
    try:
        assert injected.exists()
        assert "bob" in injected.read_text(encoding="utf-8")  # the value was delivered
    finally:
        injected.unlink(missing_ok=True)


def test_shell_normalizer_is_wired():
    # spec.normalizer.normalize must rewrite a bare const into the ${NAME:-value} env-default idiom.
    spec, _ = _shell_specs()
    assert spec.normalizer is not None
    result = spec.normalizer.normalize(_SHELL_SRC, ["NAME"])
    assert result.normalized == ["NAME"]
    assert result.refused == []
    assert "${NAME:-world}" in result.text


def test_shell_cli_reader_is_wired():
    # spec.cli_reader.read_cli must parse the script's own getopts surface into an ArgSpec.
    spec, _ = _shell_specs()
    assert spec.cli_reader is not None
    argspec = spec.cli_reader.read_cli(
        'while getopts "f:v" opt; do case $opt in f) FILE=$OPTARG;; v) V=1;; esac; done\n'
    )
    assert argspec is not None
    assert {f.flag for f in argspec.fields} == {"-f", "-v"}


def test_shell_spec_degrades_to_none_when_grammar_import_fails(monkeypatch: pytest.MonkeyPatch):
    # The single import guard: a broken/absent tree-sitter-bash wheel must leave every one of the
    # four grammar-backed capabilities at None (never "" or a half-record), while the grammar-free
    # metawriter params_io still works. Forcing `from .shell import ...` to fail exercises that
    # branch (normally unreachable — the grammar is a hard dependency in the test env).
    import skit.langs.shell  # ensure the real module is the one monkeypatch restores  # noqa: F401

    monkeypatch.setitem(sys.modules, "skit.langs.shell", None)
    spec = registry._shell_spec()
    assert spec.analyzer is None
    assert spec.injector is None
    assert spec.normalizer is None
    assert spec.cli_reader is None
    assert spec.params_io is not None  # metawriter is grammar-free and survives the degradation


# ---------------------------------------------------------------- _ts_spec: identity data


def test_ts_spec_identity_fields():
    spec = registry.spec_for("ts")
    assert spec is not None
    assert spec.kind == "ts"
    assert spec.glyph == "✧"
    assert spec.extensions == (".ts", ".mts", ".cts")


def test_ts_extension_inference_maps_each_extension_to_ts():
    for name in ("app.ts", "mod.mts", "cfg.cts"):
        assert registry.infer_kind(Path(name)) == "ts"
