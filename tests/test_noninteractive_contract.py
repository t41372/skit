"""The CLI's complete, machine-checkable non-interactive surface."""

from __future__ import annotations

import ast
import errno
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
import typer.main
from typer.testing import CliRunner

from skit import agentskill, argstate, cli, interaction, store
from skit.paths import values_dir

runner = CliRunner()


@pytest.fixture(autouse=True)
def isolated_roots(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    monkeypatch.setenv("USERPROFILE", str(tmp_path / "home"))


def _leaf_commands() -> dict[tuple[str, ...], Any]:
    root = typer.main.get_command(cli.app)
    pending: list[tuple[tuple[str, ...], Any]] = [((), root)]
    leaves: dict[tuple[str, ...], Any] = {}
    while pending:
        path, command = pending.pop()
        children = getattr(command, "commands", None)
        if children is None:
            leaves[path] = command
            continue
        pending.extend(((*path, name), child) for name, child in children.items())
    return leaves


# A closed list, with a concrete reason for every leaf that does not take --no-input.
# Adding a command forces its author to classify the new surface here.
_NON_PROMPTING_COMMANDS = {
    ("list",): "read-only library projection",
    ("show",): "read-only entry detail",
    ("rename",): "all input is positional",
    ("describe",): "all input is positional",
    ("params",): "reads or applies explicit flags only",
    ("deps",): "reads or applies explicit flags only",
    ("doctor",): "read/explicit repair; never asks",
    ("config",): "git-config grammar; never asks",
    ("runner", "list"): "read-only runner projection",
    ("runner", "add"): "all input is positional or flagged",
    ("preset", "list"): "read-only preset projection",
}


def _option_names(command: Any) -> set[str]:
    return {
        option
        for parameter in command.params
        for option in (*parameter.opts, *parameter.secondary_opts)
    }


def test_every_cli_leaf_either_takes_no_input_or_is_proven_non_prompting() -> None:
    leaves = _leaf_commands()
    without_flag = {
        path for path, command in leaves.items() if "--no-input" not in _option_names(command)
    }
    assert without_flag == set(_NON_PROMPTING_COMMANDS), {
        "unclassified": sorted(without_flag - _NON_PROMPTING_COMMANDS.keys()),
        "stale_allowlist": sorted(_NON_PROMPTING_COMMANDS.keys() - without_flag),
    }
    assert all(_NON_PROMPTING_COMMANDS.values())


def test_every_no_input_command_records_the_verdict_before_any_other_action() -> None:
    tree = ast.parse(Path(cli.__file__).read_text(encoding="utf-8"))
    functions = {
        node.name: node
        for node in tree.body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }
    for path, command in _leaf_commands().items():
        if "--no-input" not in _option_names(command):
            continue
        function = functions[command.callback.__name__]
        body = list(function.body)
        if (
            body
            and isinstance(body[0], ast.Expr)
            and isinstance(body[0].value, ast.Constant)
            and isinstance(body[0].value.value, str)
        ):
            body.pop(0)
        first = body[0]
        assert isinstance(first, ast.Expr), path
        assert isinstance(first.value, ast.Call), path
        assert isinstance(first.value.func, ast.Name), path
        assert first.value.func.id == "_forbid_interaction", path
        assert len(first.value.args) == 1, path
        argument = first.value.args[0]
        assert isinstance(argument, ast.Name), path
        assert argument.id == "no_input", path


def test_preset_save_in_a_pipe_refuses_instead_of_inventing_current_values() -> None:
    entry = store.add_command("echo {CITY}", name="say")
    result = runner.invoke(cli.app, ["preset", "save", "say", "prod"])
    assert result.exit_code == 2
    assert "--from-last" in result.output
    assert argstate.load_state(entry.slug)["presets"] == {}


def test_preset_save_pipe_refusal_preserves_existing_state_byte_for_byte() -> None:
    entry = store.add_command("echo {CITY}", name="say")
    argstate.save_last(entry.slug, values={"CITY": "Taipei"})
    argstate.save_preset(entry.slug, "existing", {"CITY": "Kyoto"})
    state_path = values_dir() / f"{entry.slug}.toml"
    before = state_path.read_bytes()

    result = runner.invoke(cli.app, ["preset", "save", "say", "new"])

    assert result.exit_code == 2
    assert state_path.read_bytes() == before


def test_preset_save_no_input_records_the_verdict_and_refuses() -> None:
    entry = store.add_command("echo {CITY}", name="say")
    result = runner.invoke(cli.app, ["preset", "save", "say", "prod", "--no-input"])
    assert result.exit_code == 2
    assert interaction.allowed() is False
    assert argstate.load_state(entry.slug)["presets"] == {}


def test_preset_from_last_remains_automation_safe_under_no_input() -> None:
    entry = store.add_command("echo {CITY}", name="say")
    argstate.record_run(
        entry.slug,
        0,
        at="2026-07-27T00:00:00+00:00",
        values={"CITY": "Taipei"},
    )
    result = runner.invoke(
        cli.app,
        ["preset", "save", "say", "prod", "--from-last", "--no-input"],
    )
    assert result.exit_code == 0, result.output
    assert argstate.load_state(entry.slug)["presets"] == {"prod": {"CITY": "Taipei"}}


def test_agent_install_no_input_refuses_the_bare_picker(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        cli,
        "_agent_pick_target",
        lambda _targets: pytest.fail("the picker opened under --no-input"),
    )
    result = runner.invoke(cli.app, ["agent", "install", "--no-input"])
    assert result.exit_code == 2
    assert interaction.allowed() is False


def test_agent_install_project_limits_bare_detection_to_project_targets(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    user_target = agentskill.Target("claude", "user", tmp_path / "home" / ".claude")
    project_target = agentskill.Target("agents", "project", tmp_path / "repo" / ".agents")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(agentskill, "default_roots", lambda: (tmp_path / "home", tmp_path / "repo"))
    monkeypatch.setattr(
        agentskill,
        "detect_targets",
        lambda **_kwargs: [user_target, project_target],
    )
    seen: list[list[agentskill.Target]] = []
    monkeypatch.setattr(cli, "_agent_pick_target", lambda targets: seen.append(targets) or None)

    result = runner.invoke(cli.app, ["agent", "install", "--project"])

    assert result.exit_code == 0, result.output
    assert seen == [[project_target]]


def test_agent_install_project_never_falls_back_to_an_existing_home_target(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    home = tmp_path / "home"
    cwd = tmp_path / "repo"
    (home / ".claude").mkdir(parents=True)
    cwd.mkdir()
    monkeypatch.setattr(agentskill, "default_roots", lambda: (home, cwd))
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(
        cli,
        "_agent_pick_target",
        lambda _targets: pytest.fail("a home target escaped the --project scope"),
    )

    result = runner.invoke(cli.app, ["agent", "install", "--project"])

    assert result.exit_code == 126
    assert not (home / ".claude" / "skills").exists()


def _run_on_unanswered_pty(args: list[str], env: dict[str, str]) -> tuple[int, str]:
    master, slave = os.openpty()  # ty: ignore[possibly-missing-attribute]
    process = subprocess.Popen(
        [str(Path(sys.executable).with_name("skit")), *args],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=env,
        close_fds=True,
    )
    os.close(slave)
    try:
        try:
            returncode = process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            pytest.fail(f"command waited for input: {' '.join(args)}")
        output = bytearray()
        while True:
            try:
                chunk = os.read(master, 4096)
            except OSError as exc:
                if exc.errno == errno.EIO:
                    break
                raise
            if not chunk:
                break
            output.extend(chunk)
        return returncode, output.decode(errors="replace")
    finally:
        os.close(master)


@pytest.mark.skipif(sys.platform == "win32", reason="the stdlib pty module is POSIX-only")
@pytest.mark.parametrize(
    "args",
    [
        ["preset", "save", "say", "prod", "--no-input"],
        ["agent", "install", "--no-input"],
    ],
)
def test_no_input_commands_terminate_on_a_real_unanswered_pty(
    args: list[str],
) -> None:
    store.add_command("echo {CITY}", name="say")
    returncode, output = _run_on_unanswered_pty(args, os.environ.copy())
    assert returncode == 2, output
    assert "Traceback" not in output
