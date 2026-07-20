"""Behavioural tests targeting mutation survivors in skit/cli.py (chunk 6/6).

Scope: `_show_params` (the `skit params` read view, human + --json) and
`_validate_forced_kind` (the `skit add --kind` guard). Real CLI via CliRunner, English
catalog, assertions on observable output / exit codes — style matches tests/test_cli_mut.py.

A few init-line mutants in `_show_params` are provable equivalents and are resolved by
`# pragma: no mutate` in the source rather than a test (see the module for justifications):
  * `text = ""`          -> None / "XXXX": the sentinel is read only via `and text`, and is
                            overwritten whenever a file is actually read.
  * `self_locating = ""` -> None: read only via `if self_locating`. The observable `= True`
                            variant IS pinned here, by
                            test_params_reference_mode_shell_omits_self_location_hint.
  * `ensure_ascii=False` -> True / None / dropped: inert, because `console.print_json`
                            re-parses then re-serializes the string, normalizing any escaping.
"""

from __future__ import annotations

import json
import stat
from pathlib import Path
from typing import Literal

import pytest
from typer.testing import CliRunner

from skit import cli, i18n, store

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")
    return tmp_path


def _norm(text: str) -> str:
    """Collapse rich's terminal-width line wrapping so long messages match as one line."""
    return " ".join(text.split())


def _shell(
    tmp_path: Path, body: str, name: str, *, mode: Literal["copy", "reference"] = "copy"
) -> store.Entry:
    p = tmp_path / f"{name}.sh"
    p.write_text(body, encoding="utf-8")
    return store.add_script(p, kind="shell", name=name, mode=mode)


_SELF_LOCATING = '#!/usr/bin/env bash\nHERE=$(dirname "$0")\nWIDTH=800\necho "$HERE $WIDTH"\n'


# --------------------------------------------------------------------------
# _validate_forced_kind  (mutants 6, 7, 15)
# --------------------------------------------------------------------------


def test_add_unknown_kind_error_lists_valid_kinds_joined_by_comma(tmp_path):
    """`skit add x --kind bogus` refuses (exit 2) and names the valid kinds, comma-joined."""
    f = tmp_path / "extensionless"
    f.write_text("hello\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(f), "--kind", "bogus"])
    assert result.exit_code == 2, result.output
    out = _norm(result.output)
    # exact-case English msgid — kills the lowercased-msgid mutant.
    assert "Unknown kind: bogus. Choose from:" in out
    # the forceable kinds are joined by ", " — kills the 'XX, XX'.join separator mutant.
    assert ", ".join(cli._forceable_kinds()) in out
    # no i18n mangling reaches the user — kills the XX-wrapped-msgid / XX-separator mutants.
    assert "XX" not in result.output


# --------------------------------------------------------------------------
# _show_params — human view (mutants 82, 178, 179, 18)
# --------------------------------------------------------------------------


def test_params_exe_reports_no_managed_parameters_in_clean_english(tmp_path):
    """An exe has no analyzer -> the exact 'no managed parameters' line (kills the XX-wrap msgid)."""
    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    exe.chmod(exe.stat().st_mode | stat.S_IXUSR)
    store.add_exe(exe, name="mytool", description="")
    result = runner.invoke(cli.app, ["params", "mytool"])
    assert result.exit_code == 0, result.output
    # Exact whole-message match: the mutant renders "XXmytool has no managed parameters.XX".
    assert _norm(result.output) == "mytool has no managed parameters."
    assert "XX" not in result.output


def test_params_self_location_hint_exact_text(tmp_path):
    """A copy-mode self-locating shell shows the $0/BASH_SOURCE hint with its exact wording."""
    _shell(tmp_path, _SELF_LOCATING, "loc")
    result = runner.invoke(cli.app, ["params", "loc"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    # Exact-case, exact tokens — kills the lowercased-msgid mutant.
    assert "This script locates itself ($0 / BASH_SOURCE)." in out
    assert "skit params loc --normalize NAME" in out
    assert 'NAME="${NAME:-value}"' in out
    # Clean i18n rendering — kills the XX-wrapped-msgid mutant.
    assert "XX" not in result.output


def test_params_reference_mode_shell_omits_self_location_hint(tmp_path):
    """A reference entry gets the same honest read, but never the --normalize hint.

    Since #14 both modes run the analyzer, so `self_locating` is COMPUTED here:
    `not ref_mode and injector and uses_self_location`. The hint must NOT appear —
    it would if the `not ref_mode` arm flipped or dropped. The same shell in copy
    mode DOES show the hint (test above), so this pins that arm, not the absence
    of self-location in the script.
    """
    _shell(tmp_path, _SELF_LOCATING, "refloc", mode="reference")
    result = runner.invoke(cli.app, ["params", "refloc"])
    assert result.exit_code == 0, result.output
    assert "locates itself" not in result.output


# --------------------------------------------------------------------------
# _show_params — --json view (mutants 16, 46)
# --------------------------------------------------------------------------


def test_params_json_placeholders_reflect_command_template(tmp_path):
    """`placeholders` echoes the template's own placeholders — kills the `and []` mutant."""
    store.add_command("echo {msg}", name="cmd1")
    result = runner.invoke(cli.app, ["params", "cmd1", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    # `entry.meta.params or []` -> ["msg"]; the `and []` mutant would yield [].
    assert data["placeholders"] == ["msg"]


def test_params_json_unmanaged_is_empty_list_not_null(tmp_path):
    """`unmanaged` is an empty list, never null — kills the `unmanaged = None` init mutant."""
    store.add_command("echo hi", name="cmd2")  # no analyzer -> reconcile skipped, list stays []
    result = runner.invoke(cli.app, ["params", "cmd2", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["unmanaged"] == []
