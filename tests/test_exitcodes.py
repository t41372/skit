"""One machine-facing exit taxonomy for every CLI front door."""

from __future__ import annotations

import ast
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, exitcodes, flows, store

runner = CliRunner()


@pytest.mark.parametrize(
    ("reason", "code"),
    [
        (exitcodes.FailureReason.BAD_VALUE, 125),
        (exitcodes.FailureReason.DRIFT, 125),
        (exitcodes.FailureReason.LAUNCH, 125),
        (exitcodes.FailureReason.NOT_EXECUTABLE, 126),
        (exitcodes.FailureReason.MISSING, 127),
        ("newer-extension-failure", 125),
    ],
)
def test_launch_failure_reason_has_one_exit_code(reason: exitcodes.FailureReason | str, code: int):
    assert exitcodes.exit_code_for_failure(reason) == code


def test_cli_has_no_unclassified_literal_exit_one() -> None:
    """Doctor names its one through EXIT_DOCTOR_UNHEALTHY; no literal can spread."""
    path = Path(cli.__file__)
    tree = ast.parse(path.read_text(encoding="utf-8"))
    offenders: list[int] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        if isinstance(node.func, ast.Name) and node.func.id == "_fail":
            args = node.args[1:2]
        elif (
            isinstance(node.func, ast.Attribute)
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == "typer"
            and node.func.attr == "Exit"
        ):
            args = node.args[:1]
        else:
            continue
        if args and isinstance(args[0], ast.Constant) and args[0].value == 1:
            offenders.append(node.lineno)
    assert offenders == []


def test_params_wrong_kind_is_usage_and_missing_copy_is_not_found(tmp_path: Path) -> None:
    script = tmp_path / "job.py"
    script.write_text("VALUE = 1\n", encoding="utf-8")
    store.add_python(script, name="job")

    wrong_kind = runner.invoke(cli.app, ["params", "job", "--runner", "nope"])
    assert wrong_kind.exit_code == exitcodes.EXIT_USAGE

    entry = store.resolve("job")
    entry.script_path.unlink()
    missing_copy = runner.invoke(cli.app, ["params", "job", "--manage", "VALUE"])
    assert missing_copy.exit_code == exitcodes.EXIT_NOT_FOUND


def test_destructive_no_is_clean_but_eof_is_abort(tmp_path: Path, monkeypatch) -> None:
    script = tmp_path / "job.py"
    script.write_text("print(1)\n", encoding="utf-8")
    store.add_python(script, name="job")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    monkeypatch.setattr(cli.typer, "confirm", lambda *_a, **_k: False)
    declined = runner.invoke(cli.app, ["remove", "job"])
    assert declined.exit_code == exitcodes.EXIT_SUCCESS
    assert store.resolve("job").meta.name == "job"

    def abort(*_a: object, **_k: object) -> bool:
        raise cli.click.exceptions.Abort

    monkeypatch.setattr(cli.typer, "confirm", abort)
    aborted = runner.invoke(cli.app, ["remove", "job"])
    assert aborted.exit_code == exitcodes.EXIT_ABORTED
    assert store.resolve("job").meta.name == "job"


@pytest.mark.parametrize("child_code", [1, 2, 125, 126, 127, 130])
def test_launched_child_owns_even_codes_that_overlap_skit_contract(
    child_code: int, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    store.add_command("echo ready", name="child")
    monkeypatch.setattr(
        flows,
        "execute",
        lambda *_a, **_k: flows.RunOutcome(child_code),
    )

    result = runner.invoke(cli.app, ["run", "child", "--no-input"])

    assert result.exit_code == child_code
