"""store's launch-policy writers: write_workdir, write_interpreter, update_template.

These three persist the policies the CLI's `skit params --workdir/--interpreter/--template`
and the TUI's Script-settings launch section edit. Each test asserts the stored meta (or
the raised refusal), never merely that a line ran.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import store


@pytest.fixture(autouse=True)
def _isolated(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _shell(tmp_path: Path, name: str = "sh") -> store.Entry:
    src = tmp_path / f"{name}.sh"
    src.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    return store.add_script(src, kind="shell", name=name)


def _js(tmp_path: Path, name: str = "j") -> store.Entry:
    src = tmp_path / f"{name}.js"
    src.write_text("console.log('hi')\n", encoding="utf-8")
    return store.add_script(src, kind="js", name=name)


def _python(tmp_path: Path, name: str = "py") -> store.Entry:
    src = tmp_path / f"{name}.py"
    src.write_text("print('hi')\n", encoding="utf-8")
    return store.add_python(src, name=name)


# ---------------------------------------------------------------- write_workdir


@pytest.mark.parametrize("literal", ["origin", "store", "invoke"])
def test_write_workdir_accepts_each_literal(tmp_path, literal):
    _shell(tmp_path)
    entry = store.write_workdir("sh", literal)
    assert entry.meta.workdir == literal
    assert store.resolve("sh").meta.workdir == literal  # persisted


def test_write_workdir_accepts_an_absolute_path(tmp_path):
    _shell(tmp_path)
    entry = store.write_workdir("sh", "/opt/data")
    assert entry.meta.workdir == "/opt/data"


def test_write_workdir_expands_a_leading_tilde_to_absolute(tmp_path):
    _shell(tmp_path)
    entry = store.write_workdir("sh", "~/work")
    assert entry.meta.workdir == str(Path("~/work").expanduser())
    assert Path(entry.meta.workdir).is_absolute()


def test_write_workdir_rejects_a_relative_path(tmp_path):
    before = _shell(tmp_path).meta.workdir
    with pytest.raises(store.StoreUsageError, match="origin, store, invoke, or an absolute path"):
        store.write_workdir("sh", "some/relative")
    assert store.resolve("sh").meta.workdir == before  # unchanged


def test_write_workdir_rejects_empty(tmp_path):
    _shell(tmp_path)
    with pytest.raises(store.StoreUsageError):
        store.write_workdir("sh", "   ")


# ---------------------------------------------------------------- write_interpreter


def test_write_interpreter_sets_then_clears_on_shell(tmp_path):
    _shell(tmp_path)
    assert store.write_interpreter("sh", "zsh").meta.interpreter == "zsh"
    assert store.resolve("sh").meta.interpreter == "zsh"
    # Empty value returns the entry to automatic detection.
    assert store.write_interpreter("sh", "").meta.interpreter == ""
    assert store.resolve("sh").meta.interpreter == ""


def test_write_interpreter_sets_on_js(tmp_path):
    _js(tmp_path)
    assert store.write_interpreter("j", "bun").meta.interpreter == "bun"


@pytest.mark.parametrize("maker", [_python, _shell, _js])
def test_write_interpreter_pins_only_where_it_is_read(tmp_path, maker):
    # python and prompt launch through machinery that never reads meta.interpreter, so
    # only shell/js accept the pin. This parametrization proves the accept side; the
    # refusals are asserted below.
    entry = maker(tmp_path)
    if entry.meta.kind == "python":
        with pytest.raises(store.StoreUsageError, match="pinnable interpreter"):
            store.write_interpreter(entry.meta.name, "x")
    else:
        assert store.write_interpreter(entry.meta.name, "x").meta.interpreter == "x"


def test_write_interpreter_refused_on_prompt(tmp_path):
    src = tmp_path / "p.prompt.md"
    src.write_text("Do {{a}}\n", encoding="utf-8")
    store.add_prompt(src, name="pr")
    with pytest.raises(store.StoreUsageError, match="pinnable interpreter"):
        store.write_interpreter("pr", "zsh")


def test_write_interpreter_refused_on_exe_and_command(tmp_path):
    prog = tmp_path / "tool"
    prog.write_text("#!/bin/sh\necho\n", encoding="utf-8")
    prog.chmod(0o755)
    store.add_exe(prog, name="ex")
    store.add_command("echo {m}", name="cmd")
    for name in ("ex", "cmd"):
        with pytest.raises(store.StoreUsageError, match="pinnable interpreter"):
            store.write_interpreter(name, "zsh")


# ---------------------------------------------------------------- update_template


def test_update_template_rewrites_and_reextracts_placeholders(tmp_path):
    store.add_command("echo {old}", name="cmd")
    entry = store.update_template("cmd", "ffmpeg -i {input} -s {size} {output}")
    assert entry.meta.template == "ffmpeg -i {input} -s {size} {output}"
    assert entry.meta.params == ["input", "size", "output"]  # re-read from the new text
    assert store.resolve("cmd").meta.params == ["input", "size", "output"]


def test_update_template_no_placeholders_clears_params(tmp_path):
    store.add_command("echo {x}", name="cmd")
    entry = store.update_template("cmd", "date")
    assert entry.meta.params is None


def test_update_template_refused_on_non_command(tmp_path):
    _python(tmp_path)
    with pytest.raises(store.StoreUsageError, match="isn't a command entry"):
        store.update_template("py", "echo {x}")


def test_update_template_refuses_empty(tmp_path):
    store.add_command("echo {x}", name="cmd")
    with pytest.raises(store.StoreError, match="must not be empty"):
        store.update_template("cmd", "   ")
    assert store.resolve("cmd").meta.template == "echo {x}"  # unchanged
