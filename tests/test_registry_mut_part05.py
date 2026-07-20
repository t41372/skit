"""Mutation pins for ``registry._powershell_spec`` (langs/registry.py, chunk 5/9).

The PowerShell LangSpec is one data row plus a launch prefix and the cli_reader wiring. This
file pins every field that row must carry — the glyph badge, the ``pwsh`` default interpreter,
the ``.ps1`` extension, the two recognized shebang programs, the ``#`` comment prefix — and the
two behaviour-bearing bits the row feeds: the ``-File`` argv prefix the launch strategy places
between ``pwsh`` and the script (PowerShell file semantics), and the real PowerShell ``read_cli``
the spec exposes as its static CLI surface. Every assertion is the observable registry contract a
consumer (launcher, forms, `skit params`) depends on.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit.langs import registry
from skit.langs.base import ArgvLaunch
from skit.langs.powershell import cli_reader as ps_cli_reader
from skit.models import Entry, Mode, ScriptMeta


def _ps_entry(tmp_path: Path, *, interpreter: str = "", mode: Mode = "copy") -> Entry:
    """A stored PowerShell entry whose script file exists on disk (so the launch strategy's
    existence check passes and it can assemble a real argv)."""
    d = tmp_path / "e"
    d.mkdir(exist_ok=True)
    meta = ScriptMeta(name="e", kind="powershell", mode=mode, interpreter=interpreter)
    entry = Entry(slug="e", meta=meta, dir=d)
    entry.script_path.write_text("Write-Host hi\n", encoding="utf-8")
    return entry


def test_powershell_spec_data_row():
    spec = registry.spec_for("powershell")
    assert spec is not None
    assert spec.kind == "powershell"
    assert spec.glyph == "»"  # the badge glyph
    assert spec.default_interpreter == "pwsh"  # `pwsh -File` by default
    assert spec.extensions == (".ps1",)
    assert spec.stored_name == "script.ps1"  # derived from extensions[0]
    assert spec.shebangs == ("pwsh", "powershell")  # both #! programs map to this kind
    assert spec.comment is not None
    assert spec.comment.prefix == "#"  # PowerShell line comments carry the [tool.skit] block


def test_powershell_shebangs_infer_the_kind(tmp_path: Path):
    # The shebang tuple is the real add-time signal: a `#!/usr/bin/env pwsh` (or powershell)
    # script with no .ps1 extension must still infer as powershell.
    for program in ("pwsh", "powershell"):
        p = tmp_path / f"deploy-{program}"
        p.write_bytes(f"#!/usr/bin/env {program}\nWrite-Host hi\n".encode())
        assert registry.infer_kind(p) == "powershell"


def test_powershell_extension_infers_the_kind(tmp_path: Path):
    p = tmp_path / "deploy.ps1"
    p.write_bytes(b"Write-Host hi\n")
    assert registry.infer_kind(p) == "powershell"  # .ps1 -> powershell


def test_powershell_launch_places_file_prefix(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    # The `-File` prefix sits between the interpreter and the script (explicit-file semantics);
    # the spec's launch strategy must assemble exactly `pwsh -File <script>`.
    monkeypatch.setattr(
        "skit.langs.launch._which", lambda name: "/usr/bin/pwsh" if name == "pwsh" else None
    )
    spec = registry.spec_for("powershell")
    assert spec is not None
    entry = _ps_entry(tmp_path)
    payload = spec.launch.build(entry, [], None, None)
    assert isinstance(payload, ArgvLaunch)
    assert payload.argv == ["/usr/bin/pwsh", "-File", str(entry.script_path)]


def test_powershell_spec_wires_the_real_cli_reader():
    # The param() block IS the CLI surface: the spec must expose the PowerShell reader (not None,
    # and not a reader whose read_cli was nulled out).
    spec = registry.spec_for("powershell")
    assert spec is not None
    assert spec.cli_reader is not None
    assert spec.cli_reader.read_cli is ps_cli_reader.read_cli
