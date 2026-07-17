"""PowerShell param() reader unit pins.

The JSON→ParamDecl mapping is fully unit-tested by monkeypatching the subprocess result (every
type, mandatory, ValidateSet, switch, degraded/non-constant/non-scalar default, help), together
with the subprocess-plumbing edges (absence, non-zero exit, unparseable JSON, timeout/OSError,
malformed payloads) and the executable-discovery matrix — 100% coverage WITHOUT pwsh installed.
The single SKIP-gated integration test runs the real extractor over a fixture .ps1 when pwsh is
present, exercising the PowerShell-side code (ParseFile, SafeGetValue, both Mandatory spellings).
"""

from __future__ import annotations

import json
import shutil
import subprocess

import pytest

from skit import flows, store
from skit.langs.powershell import cli_reader
from skit.params import ParamDecl

# ---------------------------------------------------------------- helpers


def _fake_subprocess(monkeypatch, *, payload=None, stdout=None, returncode=0, raises=None):
    """Make `read_cli` believe pwsh is on PATH and stub the subprocess result. The script text
    passed to read_cli is irrelevant (the real extractor never runs)."""
    monkeypatch.setattr(
        cli_reader.shutil, "which", lambda name: "/usr/bin/pwsh" if name == "pwsh" else None
    )
    if stdout is None and payload is not None:
        stdout = json.dumps(payload).encode("utf-8")

    def fake_run(argv, **kwargs):
        if raises is not None:
            raise raises
        return subprocess.CompletedProcess(argv, returncode, stdout=stdout or b"", stderr=b"")

    monkeypatch.setattr(cli_reader.subprocess, "run", fake_run)


def _row(name, static="System.String", **over):
    row = {
        "name": name,
        "staticType": static,
        "switch": False,
        "hasDefault": False,
        "defaultReadable": False,
        "defaultConst": None,
        "mandatory": False,
        "validateSet": None,
        "helpText": None,
    }
    row.update(over)
    return row


def _read(monkeypatch, rows):
    _fake_subprocess(monkeypatch, payload={"status": "ok", "params": rows})
    spec = cli_reader.read_cli("param()\n")
    assert spec is not None
    return {f.name: f for f in spec.fields}, spec


# ---------------------------------------------------------------- the type matrix


def test_string_param_with_default_and_help(monkeypatch):
    fields, _ = _read(
        monkeypatch,
        [_row("Name", hasDefault=True, defaultReadable=True, defaultConst="world", helpText="who")],
    )
    f = fields["Name"]
    assert (f.type, f.default, f.flag, f.help) == ("str", "world", "-Name", "who")
    assert (f.binding, f.delivery) == ("none", "flag")
    assert not f.degraded


def test_help_is_stripped_of_surrounding_whitespace(monkeypatch):
    # pwsh's GetHelpContent trails a `.PARAMETER` block with newlines on some versions and not
    # on others; the reader normalizes so the field text is identical whatever version ran.
    fields, _ = _read(monkeypatch, [_row("Name", helpText="The city to deploy to.\n\n")])
    assert fields["Name"].help == "The city to deploy to."


def test_int_and_long_map_to_int(monkeypatch):
    fields, _ = _read(
        monkeypatch,
        [
            _row("A", "System.Int32", hasDefault=True, defaultReadable=True, defaultConst=5),
            _row("B", "System.Int64", hasDefault=True, defaultReadable=True, defaultConst=9),
        ],
    )
    assert (fields["A"].type, fields["A"].default) == ("int", 5)
    assert (fields["B"].type, fields["B"].default) == ("int", 9)


def test_double_and_single_map_to_float(monkeypatch):
    fields, _ = _read(
        monkeypatch,
        [
            _row("R", "System.Double", hasDefault=True, defaultReadable=True, defaultConst=2.5),
            _row("S", "System.Single", hasDefault=True, defaultReadable=True, defaultConst=1.5),
        ],
    )
    assert (fields["R"].type, fields["R"].default) == ("float", 2.5)
    assert (fields["S"].type, fields["S"].default) == ("float", 1.5)


def test_switch_is_a_store_true_flag(monkeypatch):
    fields, _ = _read(
        monkeypatch,
        [_row("Verbose", "System.Management.Automation.SwitchParameter", switch=True)],
    )
    f = fields["Verbose"]
    assert (f.type, f.action, f.default, f.flag) == ("bool", "store_true", False, "-Verbose")


def test_validate_set_becomes_choice(monkeypatch):
    fields, _ = _read(
        monkeypatch,
        [
            _row(
                "Mode",
                validateSet=["dev", "stage", "prod"],
                hasDefault=True,
                defaultReadable=True,
                defaultConst="dev",
            )
        ],
    )
    f = fields["Mode"]
    assert (f.type, f.choices, f.default) == ("choice", ("dev", "stage", "prod"), "dev")


def test_unknown_static_type_degrades(monkeypatch):
    fields, _ = _read(monkeypatch, [_row("Obj", "System.Collections.Hashtable")])
    assert fields["Obj"].degraded
    assert fields["Obj"].type == "str"


def test_mandatory_is_required(monkeypatch):
    fields, _ = _read(monkeypatch, [_row("Target", mandatory=True)])
    assert fields["Target"].required is True


def test_non_constant_default_degrades_field(monkeypatch):
    # `[string]$When = (Get-Date)` — SafeGetValue throws PS-side, so defaultReadable is false.
    fields, _ = _read(monkeypatch, [_row("When", hasDefault=True, defaultReadable=False)])
    f = fields["When"]
    assert f.degraded
    assert f.default is None


def test_non_scalar_default_is_left_unset(monkeypatch):
    # `$Items = @(1, 2)` — a readable but non-scalar default; the type is known (str), so the
    # field is not degraded, but the array default is not carried onto the scalar model.
    fields, _ = _read(
        monkeypatch, [_row("Items", hasDefault=True, defaultReadable=True, defaultConst=[1, 2])]
    )
    assert fields["Items"].default is None
    assert not fields["Items"].degraded


def test_bool_default_is_carried(monkeypatch):
    # A readable bool default on a non-switch typed param survives through the scalar domain.
    fields, _ = _read(
        monkeypatch,
        [_row("On", "System.Boolean", hasDefault=True, defaultReadable=True, defaultConst=True)],
    )
    assert fields["On"].default is True  # System.Boolean is unmapped, so the field also degrades
    assert fields["On"].degraded


def test_secret_name_flagged(monkeypatch):
    fields, _ = _read(monkeypatch, [_row("ApiToken")])
    assert fields["ApiToken"].secret is True


def test_declaration_order_is_preserved(monkeypatch):
    _fake_subprocess(
        monkeypatch, payload={"status": "ok", "params": [_row("First"), _row("Second")]}
    )
    spec = cli_reader.read_cli("param()\n")
    assert spec is not None
    assert [f.name for f in spec.fields] == ["First", "Second"]


# ---------------------------------------------------------------- envelope / degrade paths


def test_empty_param_block_is_a_zero_field_surface(monkeypatch):
    _fake_subprocess(monkeypatch, payload={"status": "ok", "params": []})
    spec = cli_reader.read_cli("param()\n")
    assert spec is not None
    assert spec.fields == []
    assert spec.ok


def test_no_param_block_returns_none(monkeypatch):
    _fake_subprocess(monkeypatch, payload={"status": "no-params"})
    assert cli_reader.read_cli("Write-Host hi\n") is None


def test_parse_error_returns_none(monkeypatch):
    _fake_subprocess(monkeypatch, payload={"status": "parse-error"})
    assert cli_reader.read_cli("param(\n") is None


def test_non_dict_payload_returns_none(monkeypatch):
    _fake_subprocess(monkeypatch, payload=None)  # JSON `null`
    assert cli_reader.read_cli("param()\n") is None


def test_missing_status_returns_none(monkeypatch):
    _fake_subprocess(monkeypatch, payload={"params": []})
    assert cli_reader.read_cli("param()\n") is None


def test_params_not_a_list_yields_zero_fields(monkeypatch):
    _fake_subprocess(monkeypatch, payload={"status": "ok", "params": "oops"})
    spec = cli_reader.read_cli("param()\n")
    assert spec is not None
    assert spec.fields == []


def test_non_dict_row_is_skipped(monkeypatch):
    _fake_subprocess(monkeypatch, payload={"status": "ok", "params": [123, _row("Keep")]})
    spec = cli_reader.read_cli("param()\n")
    assert spec is not None
    assert [f.name for f in spec.fields] == ["Keep"]


def test_nameless_row_is_dropped(monkeypatch):
    _fake_subprocess(monkeypatch, payload={"status": "ok", "params": [_row(""), _row("Keep")]})
    spec = cli_reader.read_cli("param()\n")
    assert spec is not None
    assert [f.name for f in spec.fields] == ["Keep"]


# ---------------------------------------------------------------- subprocess plumbing


def test_no_powershell_at_all_returns_none(monkeypatch):
    monkeypatch.setattr(cli_reader.shutil, "which", lambda name: None)
    # No subprocess is ever spawned: read_cli returns before writing a temp file.
    monkeypatch.setattr(
        cli_reader.subprocess,
        "run",
        lambda *a, **k: pytest.fail("subprocess must not run without pwsh"),
    )
    assert cli_reader.read_cli("param([string]$X)\n") is None


def test_nonzero_exit_returns_none(monkeypatch):
    _fake_subprocess(monkeypatch, stdout=b'{"status":"ok","params":[]}', returncode=1)
    assert cli_reader.read_cli("param()\n") is None


def test_unparseable_json_returns_none(monkeypatch):
    _fake_subprocess(monkeypatch, stdout=b"not json at all")
    assert cli_reader.read_cli("param()\n") is None


def test_timeout_returns_none(monkeypatch):
    _fake_subprocess(
        monkeypatch, raises=subprocess.TimeoutExpired(cmd="pwsh", timeout=cli_reader._TIMEOUT)
    )
    assert cli_reader.read_cli("param()\n") is None


def test_oserror_returns_none(monkeypatch):
    _fake_subprocess(monkeypatch, raises=OSError("boom"))
    assert cli_reader.read_cli("param()\n") is None


# ---------------------------------------------------------------- executable discovery


def test_find_prefers_pwsh(monkeypatch):
    monkeypatch.setattr(
        cli_reader.shutil, "which", lambda name: "/opt/pwsh" if name == "pwsh" else None
    )
    assert cli_reader._find_powershell() == "/opt/pwsh"


def test_find_none_on_non_windows(monkeypatch):
    monkeypatch.setattr(cli_reader.sys, "platform", "linux")
    monkeypatch.setattr(cli_reader.shutil, "which", lambda name: None)
    assert cli_reader._find_powershell() is None


def test_find_falls_back_to_powershell_exe_on_windows(monkeypatch):
    monkeypatch.setattr(cli_reader.sys, "platform", "win32")
    monkeypatch.setattr(
        cli_reader.shutil,
        "which",
        lambda name: r"C:\ps\powershell.exe" if name == "powershell.exe" else None,
    )
    assert cli_reader._find_powershell() == r"C:\ps\powershell.exe"


def test_find_none_on_windows_without_powershell(monkeypatch):
    monkeypatch.setattr(cli_reader.sys, "platform", "win32")
    monkeypatch.setattr(cli_reader.shutil, "which", lambda name: None)
    assert cli_reader._find_powershell() is None


# ---------------------------------------------------------------- flag assembly + plan


def test_single_dash_flags_assemble(tmp_path):
    # PowerShell flags are single-dash PascalCase; the existing flag machinery assembles
    # `-Name value` and fires a `[switch]` bare (`-Verbose`).
    decls = [
        ParamDecl(name="Name", delivery="flag", flag="-Name", type="str"),
        ParamDecl(
            name="Verbose",
            delivery="flag",
            flag="-Verbose",
            type="bool",
            action="store_true",
            default=False,
        ),
    ]
    plan = flows.FormPlan(source="argparse", fields=[flows.FormField.from_decl(d) for d in decls])
    asm = flows.assemble(plan, {"Name": "Ada", "Verbose": "true"}, [], cwd=tmp_path)
    assert asm.args == ["-Name", "Ada", "-Verbose"]


def test_plan_reads_powershell_param_block(monkeypatch, tmp_path):
    _fake_subprocess(
        monkeypatch,
        payload={
            "status": "ok",
            "params": [_row("City", hasDefault=True, defaultReadable=True, defaultConst="Taipei")],
        },
    )
    src = tmp_path / "deploy.ps1"
    src.write_text("param([string]$City = 'Taipei')\n")
    entry = store.add_script(src, kind="powershell", name="psplan")
    plan = flows.plan_for_entry(entry)
    assert plan.source == "argparse"
    assert [f.key for f in plan.fields] == ["City"]
    assert plan.fields[0].flag == "-City"


def test_plan_none_when_reader_finds_no_surface(monkeypatch, tmp_path):
    _fake_subprocess(monkeypatch, payload={"status": "no-params"})
    src = tmp_path / "plain.ps1"
    src.write_text("Write-Host hi\n")
    entry = store.add_script(src, kind="powershell", name="psplain")
    plan = flows.plan_for_entry(entry)
    assert (
        plan.source == "none"
    )  # no param block -> reader returns None -> the entry still launches


# ---------------------------------------------------------------- real pwsh (skip-gated)


@pytest.mark.skipif(shutil.which("pwsh") is None, reason="pwsh not installed")
def test_integration_reads_a_real_param_block(tmp_path):
    text = (
        "<#\n.PARAMETER City\nThe city to deploy to.\n#>\n"
        "param(\n"
        "  [Parameter(Mandatory)][string]$City,\n"  # bare Mandatory (ExpressionOmitted)
        "  [Parameter(Mandatory=$true)][string]$Region,\n"  # explicit Mandatory=$true
        "  [ValidateSet('dev','prod')][string]$Env = 'dev',\n"
        "  [int]$Retries = 3,\n"
        "  [switch]$DryRun\n"
        ")\n"
        "Write-Host $City\n"
    )
    spec = cli_reader.read_cli(text)
    assert spec is not None
    fields = {f.name: f for f in spec.fields}
    assert fields["City"].required is True  # bare Mandatory spelling
    assert fields["Region"].required is True  # explicit Mandatory=$true spelling
    assert fields["City"].help == "The city to deploy to."  # normalized across pwsh versions
    assert fields["Env"].type == "choice"
    assert fields["Env"].choices == ("dev", "prod")
    assert fields["Env"].default == "dev"
    assert (fields["Retries"].type, fields["Retries"].default) == ("int", 3)
    assert fields["DryRun"].type == "bool"
    assert fields["DryRun"].action == "store_true"
