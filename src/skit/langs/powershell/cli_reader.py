"""Static PowerShell `param()` reader: turn a script's parameter block into form fields.

The PowerShell analogue of the argparse / parseArgs readers (`langs/python/argspec.py`,
`langs/javascript/cli_reader.py`). A PowerShell script's `param(...)` block IS its CLI
surface — named flags (`-Name value`), `[switch]` toggles, `[ValidateSet(...)]` choices —
so skit does not inject: it reads the declarations statically and assembles real flags.

Unlike the python/js readers, PowerShell is not a language skit can parse itself, so the
read uses PowerShell's OWN parser via a subprocess (no hand-rolled grammar, no new pip
dependency):

- `read_cli` writes the text to a temp file (0600, removed in a `finally`), then runs
  `pwsh -NoProfile -NonInteractive -Command <extractor>` (falling back to `powershell.exe`
  on Windows when pwsh is absent — both expose the identical
  `System.Management.Automation.Language` Parser API). The extractor calls
  `[Parser]::ParseFile`, which builds a STATIC AST and **executes nothing**, walks
  `$ast.ParamBlock.Parameters`, and emits one JSON row per parameter.

Honesty rules (mirrors argspec's A4/C4 stance — never execute the user's script):
- No `pwsh`/`powershell.exe` at all ⇒ None (Tier-0; the kind still launches).
- Subprocess failure / timeout / non-zero exit / unparseable JSON ⇒ None.
- The script does not parse (the Parser reports errors) ⇒ None — a script that doesn't
  parse simply has no readable surface (not a whole-spec `ok=False` degrade).
- No `param()` block ⇒ None (falls through to the declared/none plan). An *empty* param
  block is a readable zero-field surface (`ArgSpec(fields=[])`), like an empty parseArgs.
- A parameter whose default is a non-constant expression (`$env:X`, `(Get-Date)`) can't be
  read — the extractor catches that PowerShell-side and the field is degraded (free-text
  fallback, omitted when left empty so the script's own default applies).

Headless; stdlib subprocess only.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import TYPE_CHECKING, Any

from ...params import ParamType, is_secret_name
from ..python.argspec import ArgSpec

if TYPE_CHECKING:
    from ...params import ParamDecl

# A parse-only static read; the timeout is a liveness guard (a hung subprocess must never
# wedge an add / params call), not a policy.
_TIMEOUT = 10.0

# PowerShell StaticType full names → the form's closed type axis. A SwitchParameter is
# handled separately (it is a bool toggle, not a value flag); anything not in this table
# (an untyped `[object]` param, a custom class) degrades to a free-text field.
_STATIC_TYPES: dict[str, ParamType] = {
    "System.String": "str",
    "System.Int32": "int",
    "System.Int64": "int",
    "System.Double": "float",
    "System.Single": "float",
}

# The extractor. It reads the target path from an environment variable (so a temp path can
# never be mis-quoted or injected into the script text), STATICALLY parses it with
# ParseFile (which executes nothing), and prints ONE JSON object describing the param block:
#   {"status": "parse-error"}                 the script does not parse
#   {"status": "no-params"}                   the script has no param() block
#   {"status": "ok", "params": [ {row}, ... ]} the param block, possibly empty
# Each row carries name / staticType / switch / hasDefault / defaultReadable / defaultConst
# (SafeGetValue, which THROWS for a non-constant default — caught here, emitting readable
# false) / mandatory (from [Parameter(...)] NamedArguments, ExpressionOmitted ⇒ bare
# `Mandatory`) / validateSet (from [ValidateSet(...)] PositionalArguments) / helpText (from
# comment-based help via GetHelpContent when available).
_EXTRACTOR = r"""
$ErrorActionPreference = 'Stop'
$path = $env:SKIT_PS_TARGET
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
if ($errors -and $errors.Count -gt 0) { '{"status":"parse-error"}'; exit 0 }
if ($null -eq $ast.ParamBlock) { '{"status":"no-params"}'; exit 0 }
try { $help = $ast.GetHelpContent() } catch { $help = $null }
$rows = New-Object System.Collections.ArrayList
foreach ($p in $ast.ParamBlock.Parameters) {
  $name = $p.Name.VariablePath.UserPath
  $isSwitch = $p.StaticType.Name -eq 'SwitchParameter'
  $hasDefault = $false
  $defaultReadable = $false
  $defaultConst = $null
  if ($null -ne $p.DefaultValue) {
    $hasDefault = $true
    try { $defaultConst = $p.DefaultValue.SafeGetValue(); $defaultReadable = $true }
    catch { $defaultReadable = $false }
  }
  $mandatory = $false
  $validateSet = $null
  foreach ($attr in $p.Attributes) {
    if ($attr -isnot [System.Management.Automation.Language.AttributeAst]) { continue }
    $an = $attr.TypeName.Name
    if ($an -eq 'Parameter') {
      foreach ($na in $attr.NamedArguments) {
        if ($na.ArgumentName -eq 'Mandatory') {
          if ($na.ExpressionOmitted) { $mandatory = $true }
          else { try { $mandatory = [bool]$na.Argument.SafeGetValue() } catch { $mandatory = $false } }
        }
      }
    } elseif ($an -eq 'ValidateSet') {
      $vs = New-Object System.Collections.ArrayList
      foreach ($pa in $attr.PositionalArguments) { try { [void]$vs.Add([string]$pa.SafeGetValue()) } catch { } }
      $validateSet = $vs
    }
  }
  $helpText = $null
  $key = $name.ToUpperInvariant()
  if ($help -and $help.Parameters -and $help.Parameters.ContainsKey($key)) { $helpText = $help.Parameters[$key] }
  $row = [ordered]@{
    name = $name; staticType = $p.StaticType.FullName; switch = $isSwitch
    hasDefault = $hasDefault; defaultReadable = $defaultReadable; defaultConst = $defaultConst
    mandatory = $mandatory; validateSet = $validateSet; helpText = $helpText
  }
  [void]$rows.Add($row)
}
ConvertTo-Json ([ordered]@{ status = 'ok'; params = @($rows) }) -Compress -Depth 8
"""


def read_cli(text: str) -> ArgSpec | None:
    """Read the script's `param()` surface through PowerShell's own parser. None when there
    is no PowerShell to run, when the read fails/times out, when the script doesn't parse,
    or when there is no param block at all — every one of those is a "no readable surface"
    case that lets callers fall back to the other form sources."""
    executable = _find_powershell()
    if executable is None:
        return None
    fd, name = tempfile.mkstemp(suffix=".ps1")  # mkstemp creates the file 0600 (owner-only)
    tmp = Path(name)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(text.encode("utf-8"))
        return _extract(executable, tmp)
    finally:
        tmp.unlink(missing_ok=True)


def _find_powershell() -> str | None:
    """`pwsh` anywhere on PATH; on Windows, `powershell.exe` as the fallback (Windows
    PowerShell exposes the identical Parser API). None otherwise — the reader degrades."""
    pwsh = shutil.which("pwsh")
    if pwsh is not None:
        return pwsh
    if sys.platform != "win32":
        return None
    return shutil.which("powershell.exe")


def _extract(executable: str, tmp: Path) -> ArgSpec | None:
    """Run the extractor over the temp file and map its JSON to an ArgSpec, or None on any
    failure (the reader is best-effort: a broken read must never break `add`/`params`)."""
    child_env = {**os.environ, "SKIT_PS_TARGET": str(tmp)}
    try:
        proc = subprocess.run(  # noqa: S603 — argv list, executable resolved from PATH; ParseFile parses only
            [executable, "-NoProfile", "-NonInteractive", "-Command", _EXTRACTOR],
            capture_output=True,
            check=False,
            timeout=_TIMEOUT,
            env=child_env,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if proc.returncode != 0:
        return None
    try:
        payload = json.loads(proc.stdout.decode("utf-8", errors="replace"))
    except json.JSONDecodeError:
        return None
    return _payload_to_spec(payload)


def _payload_to_spec(payload: object) -> ArgSpec | None:
    """Map the extractor's JSON envelope to an ArgSpec. Anything but a `status: "ok"` object
    (a parse error, no param block, or a malformed payload) is a no-readable-surface None."""
    if not isinstance(payload, dict) or payload.get("status") != "ok":
        return None
    raw = payload.get("params")
    rows = raw if isinstance(raw, list) else []
    fields: list[ParamDecl] = []
    for row in rows:
        if isinstance(row, dict):
            decl = _row_to_decl(row)
            if decl is not None:
                fields.append(decl)
    return ArgSpec(fields=fields)


def _row_to_decl(row: dict[Any, Any]) -> ParamDecl | None:
    """One param-block row → a flag-delivery ParamDecl. binding="none" (the script owns the
    parser, skit only reflects it); the flag is single-dash PascalCase (`-Name`), which is
    how PowerShell spells its named parameters and how the flag machinery assembles them."""
    from ...params import ParamDecl

    name = str(row.get("name") or "")
    if not name:
        return None  # a nameless row can't label a field
    decl = ParamDecl(
        name=name,
        binding="none",
        delivery="flag",
        flag=f"-{name}",  # PowerShell flags are single-dash PascalCase: `-Name value`
        required=bool(row.get("mandatory")),
        help=str(row.get("helpText") or ""),
        secret=is_secret_name(name),
    )
    if row.get("switch"):
        # `[switch]$Verbose` ⇒ a store_true toggle, fired bare (`-Verbose`), never a value flag.
        decl.type = "bool"
        decl.action = "store_true"
        decl.default = False
        return decl
    validate_set = row.get("validateSet")
    if isinstance(validate_set, list) and validate_set:
        # `[ValidateSet('a','b')]` constrains the value — the selector wins over the static type.
        decl.type = "choice"
        decl.choices = tuple(str(v) for v in validate_set)
    else:
        _apply_static_type(decl, str(row.get("staticType") or ""))
    _apply_default(decl, row)
    return decl


def _apply_static_type(decl: ParamDecl, static_type: str) -> None:
    """Map `[string]`/`[int]`/`[double]` etc. onto the form's type axis; anything else (an
    untyped `[object]` param, a custom class) degrades to a free-text field."""
    mapped = _STATIC_TYPES.get(static_type)
    if mapped is None:
        decl.degraded = True
    else:
        decl.type = mapped


def _apply_default(decl: ParamDecl, row: dict[Any, Any]) -> None:
    """Apply a parameter's default value. A non-constant default (the extractor couldn't
    SafeGetValue it) degrades the field; a constant default is carried through the injectable
    scalar domain (a non-scalar default — an array literal — is left unset)."""
    if not row.get("hasDefault"):
        return
    if not row.get("defaultReadable"):
        decl.degraded = True
        return
    value = row.get("defaultConst")
    if isinstance(value, (str, int, float, bool)):
        decl.default = value
