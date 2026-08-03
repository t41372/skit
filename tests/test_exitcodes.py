"""One machine-facing exit taxonomy for every CLI front door."""

from __future__ import annotations

import ast
import contextlib
import errno
import os
import select
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, NoReturn, cast, override

import pytest
from typer.testing import CliRunner

from conftest import real_repo_root
from skit import cli, config, exitcodes, flows, store

runner = CliRunner()


@pytest.mark.parametrize(
    ("reason", "code"),
    [
        (exitcodes.FailureReason.BAD_VALUE, 125),
        (exitcodes.FailureReason.DRIFT, 125),
        (exitcodes.FailureReason.LAUNCH, 125),
        (exitcodes.FailureReason.NOT_EXECUTABLE, 126),
        (exitcodes.FailureReason.MISSING, 127),
    ],
)
def test_launch_failure_reason_has_one_exit_code(
    reason: exitcodes.FailureReason, code: int
) -> None:
    assert exitcodes.exit_code_for_failure(reason) == code


def test_launch_failure_reason_is_a_closed_set() -> None:
    with pytest.raises(AssertionError):
        exitcodes.exit_code_for_failure("misspelled")  # ty: ignore[invalid-argument-type]
    with pytest.raises(AssertionError):
        flows.failure_reason(flows.RunOutcome(None))


def test_cli_exit_routes_are_structurally_closed() -> None:
    """Only typed skit helpers, child passthrough, and doctor's named exception vary status.

    Read from the REAL tree, never cli.__file__: under mutmut's baseline that module is
    the trampoline rewrite, whose x_-variants of _fail would trip this whitelist."""
    path = real_repo_root() / "src" / "skit" / "cli.py"
    tree = ast.parse(path.read_text(encoding="utf-8"))
    arbitrary_exit_helpers = {"_fail", "_exit_passthrough", "_exit_doctor_health"}
    fixed_names = {
        "EXIT_ABORTED",
        "EXIT_CANCELLED",
        "EXIT_NOT_EXECUTABLE",
        "EXIT_NOT_FOUND",
        "EXIT_SKIT",
        "EXIT_USAGE",
    }
    arbitrary_exits: list[tuple[str | None, int]] = []
    doctor_constant_refs: list[tuple[str | None, int]] = []
    passthrough_calls: list[tuple[str | None, int]] = []
    doctor_exit_calls: list[tuple[str | None, int]] = []

    class Visitor(ast.NodeVisitor):
        current: str | None = None

        @override
        def visit_FunctionDef(self, node: ast.FunctionDef) -> None:
            previous = self.current
            self.current = node.name
            self.generic_visit(node)
            self.current = previous

        @override
        def visit_Attribute(self, node: ast.Attribute) -> None:
            if node.attr == "EXIT_DOCTOR_UNHEALTHY":
                doctor_constant_refs.append((self.current, node.lineno))
            self.generic_visit(node)

        @override
        def visit_Call(self, node: ast.Call) -> None:
            if isinstance(node.func, ast.Name):
                if node.func.id == "_exit_passthrough":
                    passthrough_calls.append((self.current, node.lineno))
                elif node.func.id == "_exit_doctor_health":
                    doctor_exit_calls.append((self.current, node.lineno))
            if (
                isinstance(node.func, ast.Attribute)
                and isinstance(node.func.value, ast.Name)
                and node.func.value.id == "typer"
                and node.func.attr == "Exit"
                and node.args
            ):
                arg = node.args[0]
                fixed = (
                    (isinstance(arg, ast.Constant) and arg.value == 0)
                    or (isinstance(arg, ast.Name) and arg.id in fixed_names)
                    or (
                        isinstance(arg, ast.Attribute)
                        and isinstance(arg.value, ast.Name)
                        and arg.value.id == "exitcodes"
                        and arg.attr == "EXIT_SUCCESS"
                    )
                )
                if not fixed and self.current not in arbitrary_exit_helpers:
                    arbitrary_exits.append((self.current, node.lineno))
            self.generic_visit(node)

    Visitor().visit(tree)

    assert arbitrary_exits == []
    assert len(doctor_constant_refs) == 1
    assert doctor_constant_refs[0][0] == "_doctor_exit_code"
    assert {name for name, _line in passthrough_calls} == {"main", "run"}
    assert {name for name, _line in doctor_exit_calls} == {"doctor"}


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


def test_plain_form_eof_uses_the_shared_abort_status(tmp_path: Path, monkeypatch) -> None:
    store.add_command("echo {value}", name="job")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    aborted = runner.invoke(cli.app, ["run", "job", "--plain"], input="")

    assert aborted.exit_code == exitcodes.EXIT_ABORTED
    assert "Cancelled." in aborted.output


@pytest.mark.skipif(sys.platform == "win32", reason="PTY EOF is a POSIX terminal contract")
def test_rich_prompt_eof_on_a_real_pty_exits_130() -> None:
    store.add_command("echo {value}", name="job")
    master, slave = os.openpty()  # ty: ignore[possibly-missing-attribute]
    process = subprocess.Popen(
        [str(Path(sys.executable).with_name("skit")), "run", "job", "--plain"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=os.environ.copy(),
        close_fds=True,
    )
    os.close(slave)
    output = bytearray()
    try:
        deadline = time.monotonic() + 5
        last_output = time.monotonic()
        while time.monotonic() < deadline:
            readable, _, _ = select.select([master], [], [], 0.1)
            if readable:
                output.extend(os.read(master, 4096))
                last_output = time.monotonic()
            elif output and time.monotonic() - last_output >= 0.3:
                break
        assert b"value" in output.lower(), output.decode(errors="replace")
        os.write(master, b"\x04")
        try:
            returncode = process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            pytest.fail("Rich Prompt waited after terminal EOF")
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
    finally:
        os.close(master)

    rendered = output.decode(errors="replace")
    assert returncode == exitcodes.EXIT_ABORTED, rendered
    assert "Traceback" not in rendered


def test_declining_optional_directory_offer_is_clean_success(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    directory = tmp_path / "tool"
    directory.mkdir()
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli, "_wants_tui_form", lambda: False)

    declined = runner.invoke(cli.app, ["add", str(directory)], input="n\n")

    assert declined.exit_code == exitcodes.EXIT_SUCCESS
    assert store.list_entries() == []


def test_rich_confirm_eof_reaches_the_shared_abort_boundary(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    directory = tmp_path / "tool"
    directory.mkdir()
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli, "_wants_tui_form", lambda: False)

    aborted = runner.invoke(cli.app, ["add", str(directory)], input="")

    assert aborted.exit_code == exitcodes.EXIT_ABORTED
    assert "Cancelled." in aborted.output
    assert store.list_entries() == []


def test_click_usage_error_does_not_enter_the_interaction_abort_boundary() -> None:
    result = runner.invoke(cli.app, ["show", "--not-a-real-option"])

    assert result.exit_code == exitcodes.EXIT_USAGE
    assert "Cancelled." not in result.output


@pytest.mark.parametrize(
    "error",
    [
        PermissionError(13, "Permission denied", "config.toml"),
        config.ConfigWriteError(13, "Permission denied", "config.toml"),
    ],
)
def test_root_boundary_classifies_expected_config_write_failures(
    monkeypatch: pytest.MonkeyPatch,
    error: OSError,
) -> None:
    @contextlib.contextmanager
    def denied(*_args: object, **_kwargs: object):
        raise error
        yield

    monkeypatch.setattr(config, "advisory_file_lock", denied)
    result = runner.invoke(cli.app, ["runner", "add", "mine", "tool", "{{prompt}}"])

    assert result.exit_code == exitcodes.EXIT_SKIT
    assert "filesystem operation" in result.output
    assert "Traceback" not in result.output


def test_unreadable_executable_source_is_a_clean_store_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = tmp_path / "blocked.bin"
    source.write_bytes(b"program")
    resolved = source.resolve()
    real_open = Path.open

    def denied(path: Path, *args: object, **kwargs: object):
        if path == resolved:
            raise PermissionError(13, "Permission denied", str(path))
        return cast(Any, real_open)(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", denied)
    result = runner.invoke(
        cli.app,
        ["add", str(source), "--exe", "--name", "blocked", "--no-input"],
    )

    assert result.exit_code == exitcodes.EXIT_SKIT
    assert "Can't read" in result.output
    assert "Traceback" not in result.output
    assert store.list_entries() == []


def test_corrupt_metadata_is_not_reported_as_a_missing_entry() -> None:
    entry = store.add_command("echo ready", name="one")
    (entry.dir / "meta.toml").write_text("broken = [", encoding="utf-8")

    result = runner.invoke(cli.app, ["show", "one"])

    assert result.exit_code == exitcodes.EXIT_SKIT
    assert "metadata is corrupt" in result.output


def test_tui_child_status_uses_the_same_passthrough_route(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from skit import tui

    monkeypatch.setattr(cli, "_maybe_first_run_setup", lambda: None)
    monkeypatch.setattr(tui, "run_menu", lambda: 42)

    assert runner.invoke(cli.app, []).exit_code == 42


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


@pytest.mark.skipif(sys.platform == "win32", reason="the real child uses a POSIX shell command")
def test_post_run_state_failure_warns_without_stealing_real_child_status(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store.add_command("sh -c 'exit 42'", name="child")

    def denied(*_args: object, **_kwargs: object) -> None:
        raise PermissionError(13, "Permission denied", "values/child.toml")

    monkeypatch.setattr(flows, "save_after_run", denied)
    result = runner.invoke(cli.app, ["run", "child", "--no-input"])

    assert result.exit_code == 42
    assert "couldn't save its state" in result.output
    assert "Traceback" not in result.output


def test_interrupted_run_warns_when_accepted_values_cannot_be_persisted(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    store.add_command("echo ok", name="job")

    def interrupted(*_args: object, **_kwargs: object) -> NoReturn:
        raise KeyboardInterrupt

    monkeypatch.setattr(flows, "execute", interrupted)
    monkeypatch.setattr(
        flows,
        "post_run_persistence_error",
        lambda _action: "skit couldn't save its state: disk is read-only",
    )

    result = runner.invoke(cli.app, ["run", "job", "--no-input"])

    assert result.exit_code == exitcodes.EXIT_ABORTED
    assert "couldn't save its state" in result.output
    assert "Cancelled." in result.output


@pytest.mark.skipif(sys.platform == "win32", reason="the real child uses a POSIX shell command")
def test_tui_post_run_state_failure_preserves_real_child_status(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from skit import tui

    entry = store.add_command("sh -c 'exit 42'", name="child")
    plan = flows.plan_for_entry(entry)
    assembly = flows.assemble(plan, {}, [], cwd=tmp_path)

    def denied(*_args: object, **_kwargs: object) -> None:
        raise PermissionError(13, "Permission denied", "values/child.toml")

    printed: list[str] = []
    monkeypatch.setattr(flows, "save_after_run", denied)
    monkeypatch.setattr(
        "builtins.print",
        lambda *args, **_kwargs: printed.append(" ".join(str(arg) for arg in args)),
    )

    code = tui._finish_run(
        tui.PendingRun(entry, plan, assembly, {}, [], extra_raw=False, show_drift=False)
    )

    assert code == 42
    assert any("couldn't save its state" in line for line in printed)
