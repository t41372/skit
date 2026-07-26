"""The console-script dispatcher (src/skit/__main__.py).

`skit --version` is answered before `skit.cli` is imported, which is the only reason
the version flag costs ~150 modules instead of ~290. That is a contract, not an
accident: the subprocess tests below fail the moment something drags typer or rich back
onto the fast path, and the pyproject test fails if the entry point stops pointing here.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

from skit import __main__ as entry

ROOT = Path(__file__).resolve().parent.parent
PYPROJECT = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))


def _run(argv: list[str]) -> subprocess.CompletedProcess[str]:
    """The dispatcher in a fresh interpreter, reporting its own import graph on stderr."""
    code = (
        f"import sys; sys.argv = {['skit', *argv]!r}\n"
        "from skit.__main__ import main\n"
        "try:\n"
        "    main()\n"
        "except SystemExit as exc:\n"
        "    code = exc.code\n"
        "else:\n"
        "    code = 0\n"
        "import json\n"
        "print(json.dumps({'code': code, 'modules': sorted(sys.modules)}), file=sys.stderr)\n"
    )
    return subprocess.run(
        [sys.executable, "-c", code], capture_output=True, text=True, check=False, timeout=120
    )


def _report(proc: subprocess.CompletedProcess[str]) -> tuple[object, set[str]]:
    """The dispatcher's exit code and the set of modules its interpreter loaded."""
    doc = json.loads(proc.stderr.strip().splitlines()[-1])
    return doc["code"], {str(name) for name in doc["modules"]}


def _loaded(modules: set[str], top: str) -> list[str]:
    """Every loaded module belonging to the `top` package."""
    return sorted(m for m in modules if m == top or m.startswith(f"{top}."))


@pytest.mark.parametrize("flag", ["--version", "-V"])
def test_version_flag_is_answered_without_building_the_cli(flag: str) -> None:
    """Both spellings answer from the dispatcher, and neither imports typer, rich or
    the CLI module itself."""
    proc = _run([flag])
    import skit

    assert proc.stdout == f"skit {skit.__version__}\n"
    code, loaded = _report(proc)
    assert code == 0
    assert "skit.cli" not in loaded
    assert _loaded(loaded, "typer") == []
    assert _loaded(loaded, "rich") == []
    assert _loaded(loaded, "textual") == []
    assert _loaded(loaded, "tree_sitter") == []


def test_version_is_plain_text_not_rich_markup() -> None:
    """rich's number highlighter splits a PEP 440 version into colored fragments
    ("0.4" cyan, ".", "1." cyan, "dev0"). `--version` is a machine-facing answer, so it
    is printed plainly — no escape sequences, whatever the terminal."""
    proc = _run(["--version"])
    assert "\x1b" not in proc.stdout


@pytest.mark.parametrize("flag", ["--version", "-V"])
def test_version_flag_answers_in_process_too(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], flag: str
) -> None:
    """The same branch as the subprocess tests above, called directly — they prove
    what the fast path does NOT import, this proves what it prints and that it returns
    rather than falling through to Typer."""
    import skit

    monkeypatch.setattr(sys, "argv", ["skit", flag])
    monkeypatch.setattr("skit.cli.app", _must_not_run)
    entry.main()
    assert capsys.readouterr().out == f"skit {skit.__version__}\n"


def _must_not_run() -> None:
    raise AssertionError("the CLI was built for a flag the dispatcher should have answered")


def test_a_real_command_still_reaches_the_cli(monkeypatch: pytest.MonkeyPatch) -> None:
    """Everything that is not the leading version flag falls through to Typer."""
    called: list[bool] = []
    monkeypatch.setattr(sys, "argv", ["skit", "list"])
    monkeypatch.setattr("skit.cli.app", lambda: called.append(True))
    entry.main()
    assert called == [True]


def test_no_arguments_reaches_the_cli(monkeypatch: pytest.MonkeyPatch) -> None:
    """A bare `skit` opens the TUI, which is the CLI callback's job — the dispatcher
    must not shortcut it."""
    called: list[bool] = []
    monkeypatch.setattr(sys, "argv", ["skit"])
    monkeypatch.setattr("skit.cli.app", lambda: called.append(True))
    entry.main()
    assert called == [True]


@pytest.mark.parametrize(
    "argv",
    [
        ["list", "--version"],
        ["--install-completion", "--version"],
        ["--version", "foo"],
        ["--version", "list"],
        ["-V", "bar", "baz"],
    ],
)
def test_the_flag_is_claimed_only_as_the_whole_command_line(
    monkeypatch: pytest.MonkeyPatch, argv: list[str]
) -> None:
    """`skit --version` is the one invocation whose answer cannot depend on anything
    Typer would have parsed. `skit --version foo` is a usage error Typer reports ("No
    such command 'foo'"); `skit --version list` prints the version through the
    callback; `skit --install-completion --version` exits on the eager option first.
    A dispatcher that claimed a LEADING flag and ignored the rest of argv would turn
    all of those into a silent exit 0."""
    called: list[bool] = []
    monkeypatch.setattr(sys, "argv", ["skit", *argv])
    monkeypatch.setattr("skit.cli.app", lambda: called.append(True))
    entry.main()
    assert called == [True]


def test_python_dash_m_skit_is_the_same_entry() -> None:
    """`python -m skit` and the console script run the same dispatcher."""
    proc = subprocess.run(
        [sys.executable, "-m", "skit", "--version"],
        capture_output=True,
        text=True,
        check=False,
        timeout=120,
    )
    import skit

    assert proc.returncode == 0
    assert proc.stdout == f"skit {skit.__version__}\n"


def test_the_console_script_points_at_the_dispatcher() -> None:
    """If the entry point goes back to `skit.cli:app`, every test above still passes
    while the installed `skit` command quietly pays the full import again."""
    assert PYPROJECT["project"]["scripts"] == {"skit": "skit.__main__:main"}


@pytest.mark.parametrize("argv", [["--version", "foo"], ["-V", "bar", "baz"]])
def test_a_bad_invocation_still_fails_through_the_dispatcher(argv: list[str]) -> None:
    """The behavioral half of the test above: these command lines exited non-zero
    before the dispatcher existed, and must still. Run end to end so the assertion is
    about what a user's shell sees, not about which function was called."""
    proc = _run(argv)
    code, _ = _report(proc)
    assert code not in (None, 0)
    assert "skit " not in proc.stdout
