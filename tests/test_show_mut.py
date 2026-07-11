"""Mutation-hardening for `skit show`'s human view: exact-output pins.

The human view is agent- and human-facing copy rendered from many small string
fragments; substring assertions let dozens of string/format mutants survive. These
tests pin the ENTIRE rendered output for three representative fixtures instead
(console widened so long temp paths don't wrap; the table sizes to content).
"""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, metawriter, store
from skit.metawriter import ParamSpec

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    return tmp_path


@pytest.fixture(autouse=True)
def wide_console(monkeypatch: pytest.MonkeyPatch):
    # Exact-output comparison: stop rich from wrapping the variable-length tmp paths.
    monkeypatch.setattr(cli.console, "_width", 400)
    monkeypatch.setattr(cli.err_console, "_width", 400)


def _py(tmp_path: Path, body: str, name: str) -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def test_show_minimal_output_exact(tmp_path):
    p = _py(tmp_path, "print(1)\n", "job.py")
    store.add_python(p, name="job", workdir="origin")
    result = runner.invoke(cli.app, ["show", "job"])
    assert result.exit_code == 0, result.output
    assert result.output == (
        "job  (python · copy)\n"
        f"  Source: {p}\n"
        "  No form fields — arguments after -- pass straight through to the script.\n"
        "  Run it: skit run job\n"
    )


def test_show_argparse_output_exact(tmp_path):
    body = (
        "import argparse\n"
        "ap = argparse.ArgumentParser()\n"
        "ap.add_argument('src')\n"
        "ap.add_argument('--width', type=int, default=800, help='target width')\n"
        "ap.add_argument('--fmt', choices=['png', 'jpg'], default='png')\n"
        "ap.add_argument('--suffix', default='')\n"
        "ap.add_argument('--force', action='store_true')\n"
        "ap.parse_args()\n"
    )
    p = _py(tmp_path, body, "resize.py")
    entry = store.add_python(p, name="resize", description="resize images", workdir="origin")
    # Two deps and two presets so the ", " join separators are actually exercised.
    store.update_dependencies(
        entry.slug, ["pillow>=10,<12", "rich>=15,<16"], requires_python=">=3.12"
    )
    argstate.save_preset(entry.slug, "fast", {"width": "1200"})
    argstate.save_preset(entry.slug, "web", {"fmt": "jpg"})
    result = runner.invoke(cli.app, ["show", "resize"])
    assert result.exit_code == 0, result.output
    assert result.output == (
        "resize  (python · copy)\n"
        "  resize images\n"
        f"  Source: {p}\n"
        "  Dependencies: pillow>=10,<12, rich>=15,<16\n"
        "  Python constraint: >=3.12\n"
        "┏━━━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━━━━━┓\n"
        "┃ Parameter ┃ Type   ┃ Required ┃ Default ┃ Choices  ┃ Secret ┃ Help         ┃\n"
        "┡━━━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━━━━━┩\n"
        "│ src       │ str    │ yes      │ —       │ —        │ —      │ —            │\n"
        "│ width     │ int    │ —        │ 800     │ —        │ —      │ target width │\n"
        "│ fmt       │ choice │ —        │ png     │ png, jpg │ —      │ —            │\n"
        "│ suffix    │ str    │ —        │ —       │ —        │ —      │ —            │\n"
        "│ force     │ bool   │ —        │ false   │ —        │ —      │ —            │\n"
        "└───────────┴────────┴──────────┴─────────┴──────────┴────────┴──────────────┘\n"
        "  Presets: fast, web\n"
        "  Run it: skit run resize\n"
    )


def test_show_inject_secret_output_exact(tmp_path):
    text = metawriter.write_params(
        'KEY = "abc"\nCITY = "Taipei"\nTOKEN = "t"\nprint(KEY, CITY, TOKEN)\n',
        [
            ParamSpec(
                name="KEY",
                kind="const",
                type="str",
                default="abc",
                secret=True,
                env_source="API_KEY",
            ),
            ParamSpec(
                name="CITY", kind="const", type="str", default="Taipei", prompt="Which city?"
            ),
            ParamSpec(name="TOKEN", kind="const", type="str", secret=True),
        ],
    )
    p = _py(tmp_path, text, "api.py")
    store.add_python(p, name="api", workdir="origin")
    result = runner.invoke(cli.app, ["show", "api"])
    assert result.exit_code == 0, result.output
    # Pins: masked secret default, env-source arrow, bare-secret "yes", prompt as Help.
    assert result.output == (
        "api  (python · copy)\n"
        f"  Source: {p}\n"
        "┏━━━━━━━━━━━┳━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━┓\n"
        "┃ Parameter ┃ Type ┃ Required ┃ Default ┃ Choices ┃ Secret         ┃ Help        ┃\n"
        "┡━━━━━━━━━━━╇━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━┩\n"
        "│ KEY       │ str  │ —        │ •••     │ —       │ yes ← $API_KEY │ —           │\n"
        "│ CITY      │ str  │ —        │ Taipei  │ —       │ —              │ Which city? │\n"
        "│ TOKEN     │ str  │ —        │ —       │ —       │ yes            │ —           │\n"
        "└───────────┴──────┴──────────┴─────────┴─────────┴────────────────┴─────────────┘\n"
        "  Run it: skit run api\n"
    )


def test_show_command_output_exact():
    result = runner.invoke(
        cli.app, ["add", "--cmd", "echo deploy {target}", "--name", "dep", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    result = runner.invoke(cli.app, ["show", "dep"])
    assert result.exit_code == 0, result.output
    assert result.output == (
        "dep  (command · reference)\n"
        "  Working directory: invoke\n"
        "  Command template: echo deploy {target}\n"
        "┏━━━━━━━━━━━┳━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━┳━━━━━━┓\n"
        "┃ Parameter ┃ Type ┃ Required ┃ Default ┃ Choices ┃ Secret ┃ Help ┃\n"
        "┡━━━━━━━━━━━╇━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━╇━━━━━━┩\n"
        "│ target    │ str  │ yes      │ —       │ —       │ —      │ —    │\n"
        "└───────────┴──────┴──────────┴─────────┴─────────┴────────┴──────┘\n"
        "  Run it: skit run dep\n"
    )


def test_show_workdir_line_appears_only_when_not_origin(tmp_path):
    p = _py(tmp_path, "print(1)\n", "job.py")
    store.add_python(p, name="wd", workdir="invoke")
    result = runner.invoke(cli.app, ["show", "wd"])
    assert result.exit_code == 0, result.output
    assert "  Working directory: invoke\n" in result.output
