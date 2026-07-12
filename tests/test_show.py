"""`skit show` — the full read view of one script (identity + unified schema + presets).

show is the agent-facing discovery surface (issue #2): the one command that exposes the
complete FormPlan field schema across all three sources (inject / argparse / command),
which `params` deliberately does not (it owns the managed-definition view). The --json
shape is a stable contract, so these tests pin it key-by-key.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, metawriter, store
from skit.metawriter import ParamSpec

runner = CliRunner()

# Every key the payload must always carry — the stable-shape contract.
PAYLOAD_KEYS = {
    "name",
    "slug",
    "kind",
    "mode",
    "description",
    "source",
    "workdir",
    "missing",
    "dependencies",
    "requires_python",
    "template",
    "param_source",
    "degraded_reason",
    "drift",
    "fields",
    "presets",
    "last_run_at",
    "last_exit",
}
FIELD_KEYS = {
    "key",
    "label",
    "type",
    "source",
    "required",
    "secret",
    "multiple",
    "degraded",
    "choices",
    "default",
    "help",
    "flag",
    "action",
    "env_source",
}


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    return tmp_path


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _show_json(name: str) -> dict[str, Any]:
    result = runner.invoke(cli.app, ["show", name, "--json"])
    assert result.exit_code == 0, result.output
    return json.loads(result.output)


ARGPARSE = (
    "import argparse\n"
    "ap = argparse.ArgumentParser()\n"
    "ap.add_argument('src')\n"
    "ap.add_argument('--width', type=int, default=800, help='target width')\n"
    "ap.add_argument('--fmt', choices=['png', 'jpg'], default='png')\n"
    "ap.add_argument('--force', action='store_true')\n"
    "ap.parse_args()\n"
)


# --------------------------------------------------------------------------
# --json: the stable contract
# --------------------------------------------------------------------------


def test_show_json_argparse_full_schema(tmp_path):
    entry = store.add_python(_py(tmp_path, ARGPARSE), name="resize")
    payload = _show_json("resize")
    assert payload["name"] == "resize"
    assert payload["slug"] == entry.slug
    assert payload["kind"] == "python"
    assert payload["mode"] == "copy"
    assert payload["source"] == str(tmp_path / "job.py")
    assert payload["workdir"] == str(entry.meta.workdir)
    assert payload["missing"] is False
    assert payload["template"] is None
    assert payload["param_source"] == "argparse"
    assert payload["degraded_reason"] == ""
    assert payload["drift"] is False
    assert payload["presets"] == []
    assert payload["last_run_at"] is None
    assert payload["last_exit"] is None

    fields = {f["key"]: f for f in payload["fields"]}
    assert list(fields) == ["src", "width", "fmt", "force"]
    src = fields["src"]
    assert src == {
        "key": "src",
        "label": "src",
        "type": "str",
        "source": "flag",
        "required": True,
        "secret": False,
        "multiple": False,
        "degraded": False,
        "choices": [],
        "default": None,
        "help": "",
        "flag": "",
        "action": "",
        "env_source": "",
    }
    assert fields["width"]["type"] == "int"
    assert fields["width"]["default"] == "800"
    assert fields["width"]["help"] == "target width"
    assert fields["width"]["flag"] == "--width"
    assert fields["width"]["required"] is False
    assert fields["fmt"]["type"] == "choice"
    assert fields["fmt"]["choices"] == ["png", "jpg"]
    assert fields["force"]["type"] == "bool"
    assert fields["force"]["action"] == "store_true"
    assert fields["force"]["default"] == "false"


def test_show_json_stable_shape(tmp_path):
    store.add_python(_py(tmp_path, ARGPARSE), name="shape")
    payload = _show_json("shape")
    assert set(payload) == PAYLOAD_KEYS
    for f in payload["fields"]:
        assert set(f) == FIELD_KEYS


def test_show_json_inject_secret_and_state(tmp_path):
    text = metawriter.write_params(
        'KEY = "abc"\nCITY = "Taipei"\nprint(KEY, CITY)\n',
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
        ],
    )
    entry = store.add_python(_py(tmp_path, text), name="api")
    argstate.save_preset(entry.slug, "fast", {"CITY": "Tainan"})
    argstate.record_run(entry.slug, 3, at="2026-07-11T00:00:00+00:00")
    payload = _show_json("api")
    assert payload["param_source"] == "inject"
    assert payload["presets"] == ["fast"]
    assert payload["last_run_at"] == "2026-07-11T00:00:00+00:00"
    assert payload["last_exit"] == 3
    fields = {f["key"]: f for f in payload["fields"]}
    key = fields["KEY"]
    assert key["source"] == "inject"
    assert key["secret"] is True
    assert key["env_source"] == "API_KEY"
    # params --json parity: a secret's declared default already lives in the script's own
    # text, so the JSON carries it as-is (the human table masks it instead).
    assert key["default"] == "abc"
    assert fields["CITY"]["label"] == "Which city?"


def test_show_json_command_kind(tmp_path):
    result = runner.invoke(
        cli.app, ["add", "--cmd", "echo {target} {level}", "--name", "deploy", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    payload = _show_json("deploy")
    assert payload["kind"] == "command"
    assert payload["template"] == "echo {target} {level}"
    assert payload["param_source"] == "command"
    fields = {f["key"]: f for f in payload["fields"]}
    assert set(fields) == {"target", "level"}
    assert fields["target"]["source"] == "placeholder"
    assert fields["target"]["required"] is True


def test_show_json_deps_and_missing_reference(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    entry = store.add_python(p, name="ref", mode="reference")
    store.update_dependencies(entry.slug, ["requests>=2,<3"], requires_python=">=3.12")
    p.unlink()
    payload = _show_json("ref")
    assert payload["mode"] == "reference"
    assert payload["missing"] is True
    assert payload["dependencies"] == ["requests>=2,<3"]
    assert payload["requires_python"] == ">=3.12"


def test_show_json_degraded_parser(tmp_path):
    text = (
        "import argparse\n"
        "ap = argparse.ArgumentParser()\n"
        "sub = ap.add_subparsers()\n"
        "p = sub.add_parser('x')\n"
        "p.add_argument('--y')\n"
        "ap.parse_args()\n"
    )
    store.add_python(_py(tmp_path, text), name="multi")
    payload = _show_json("multi")
    assert payload["param_source"] == "argparse"
    assert payload["degraded_reason"] == "subparsers"
    assert payload["fields"] == []


def test_show_json_drift(tmp_path):
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n', [ParamSpec(name="CITY", kind="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name="stale")
    moved = entry.script_path.read_text(encoding="utf-8").replace('CITY = "x"', 'TOWN = "x"')
    entry.script_path.write_text(moved, encoding="utf-8")
    payload = _show_json("stale")
    assert payload["drift"] is True


# --------------------------------------------------------------------------
# human view
# --------------------------------------------------------------------------


def test_show_human_argparse_table(tmp_path):
    store.add_python(_py(tmp_path, ARGPARSE), name="resize")
    result = runner.invoke(cli.app, ["show", "resize"])
    assert result.exit_code == 0, result.output
    assert "resize" in result.output
    assert "width" in result.output
    assert "target width" in result.output
    assert "png, jpg" in result.output
    assert "yes" in result.output  # src is required
    assert "Source:" in result.output
    assert "Run it: skit run resize" in result.output


def test_show_human_masks_secret_default_and_names_env_source(tmp_path):
    text = metawriter.write_params(
        'KEY = "s3cret"\nprint(KEY)\n',
        [
            ParamSpec(
                name="KEY",
                kind="const",
                type="str",
                default="s3cret",
                secret=True,
                env_source="API_KEY",
            )
        ],
    )
    store.add_python(_py(tmp_path, text), name="api")
    result = runner.invoke(cli.app, ["show", "api"])
    assert result.exit_code == 0, result.output
    assert "s3cret" not in result.output
    assert "•••" in result.output
    assert "$API_KEY" in result.output


def test_show_human_secret_without_env_source(tmp_path):
    text = metawriter.write_params(
        'TOKEN = "t"\nprint(TOKEN)\n',
        [ParamSpec(name="TOKEN", kind="const", type="str", secret=True)],
    )
    store.add_python(_py(tmp_path, text), name="tok")
    result = runner.invoke(cli.app, ["show", "tok"])
    assert result.exit_code == 0, result.output
    # TOKEN is optional and has no default, so the single "yes" is the Secret cell.
    assert result.output.count("yes") == 1
    assert "←" not in result.output  # no env-source arrow without an env source


def test_show_human_command_kind(tmp_path):
    runner.invoke(cli.app, ["add", "--cmd", "echo {a}", "--name", "c1", "--no-input"])
    result = runner.invoke(cli.app, ["show", "c1"])
    assert result.exit_code == 0, result.output
    assert "Command template: echo {a}" in result.output
    assert "Source:" not in result.output  # a command entry has no file source to show
    assert "a" in result.output


def test_show_human_no_fields_exe(tmp_path):
    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    exe.chmod(0o755)
    result = runner.invoke(cli.app, ["add", "--exe", str(exe), "--name", "tool", "--no-input"])
    assert result.exit_code == 0, result.output
    result = runner.invoke(cli.app, ["show", "tool"])
    assert result.exit_code == 0, result.output
    assert "No form fields" in result.output


def test_show_human_description_deps_presets_and_drift(tmp_path):
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n', [ParamSpec(name="CITY", kind="const", type="str")]
    )
    result = runner.invoke(
        cli.app,
        ["add", str(_py(tmp_path, text)), "--name", "trip", "-d", "plan a trip", "--no-input"],
    )
    assert result.exit_code == 0, result.output
    entry = store.resolve("trip")
    store.update_dependencies(entry.slug, ["rich>=15"], requires_python=">=3.12")
    argstate.save_preset(entry.slug, "quick", {"CITY": "Tainan"})
    entry = store.resolve("trip")
    moved = entry.script_path.read_text(encoding="utf-8").replace('CITY = "x"', 'TOWN = "x"')
    entry.script_path.write_text(moved, encoding="utf-8")
    result = runner.invoke(cli.app, ["show", "trip"])
    assert result.exit_code == 0, result.output
    assert "plan a trip" in result.output
    assert "rich>=15" in result.output
    assert ">=3.12" in result.output
    assert "Presets: quick" in result.output
    assert "drifted from the script" in result.output  # the drift banner is shown


def test_show_human_degraded_parser_notice(tmp_path):
    text = (
        "import argparse\n"
        "ap = argparse.ArgumentParser()\n"
        "sub = ap.add_subparsers()\n"
        "p = sub.add_parser('x')\n"
        "p.add_argument('--y')\n"
        "ap.parse_args()\n"
    )
    store.add_python(_py(tmp_path, text), name="multi")
    result = runner.invoke(cli.app, ["show", "multi"])
    assert result.exit_code == 0, result.output
    # Line-exact: an XX-wrapped msgid mutant still contains the substring.
    assert (
        "skit could not model this script's own arguments; pass them after -- instead."
        in result.output.splitlines()
    )


def test_show_human_missing_marker(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    store.add_python(p, name="gone", mode="reference")
    p.unlink()
    result = runner.invoke(cli.app, ["show", "gone"])
    assert result.exit_code == 0, result.output
    # The glyph-prefixed marker, not a bare "missing" — pytest's tmp dir is named
    # after this test, so the *source path* printed above already contains "missing".
    assert "⚠ missing:" in result.output


def test_show_not_found_exits_1():
    result = runner.invoke(cli.app, ["show", "ghost"])
    assert result.exit_code == 1
