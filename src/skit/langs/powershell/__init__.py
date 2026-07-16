"""PowerShell support: the `param(...)` block read through PowerShell's OWN parser.

A PowerShell script's `param()` block IS its command-line surface (`-Name value`,
`[switch]` flags), so — unlike python/shell/js — skit does not inject anything: it
statically reads the parameter declarations and assembles real flags, exactly like the
argparse / parseArgs readers. There is deliberately no analyzer and no injector here
(documented in `cli_reader.py`): injection into a PowerShell script is out of scope for v1.

The read spawns `pwsh` (or `powershell.exe` on Windows) running
`[System.Management.Automation.Language.Parser]::ParseFile`, a pure STATIC parse that
executes nothing. No PowerShell on PATH ⇒ the reader degrades to None (Tier-0), and the
kind still launches. The registry wires the reader with no import guard — it is
stdlib-subprocess only, and degrades at run time rather than import time.
"""
