"""Residual coverage gaps across headless modules.

Each test here closes a tiny leftover gap (an error path, an edge case, or a
rarely-hit branch) in analyzer / argspec / flows / launcher / metawriter /
pep723 / store. Every test asserts OBSERVABLE behavior — a returned value, a
raised error + message, or a file/registry mutation — not "run this line".

All modules are headless (no Textual); the autouse conftest fixture isolates
SKIT_* dirs + HOME per tmp_path and pins SKIT_LANG=en for exact-message asserts.
"""

from __future__ import annotations

import contextlib
import os
import subprocess
import sys
from pathlib import Path

import pytest

from skit import flows, launcher, pep723, store
from skit.langs import launch
from skit.langs.python import analyzer, argspec, metawriter
from skit.params import ParamDecl

# --------------------------------------------------------------------------
# analyzer._match_inputs — the equal-count duplicate-prompt multiset pass
# --------------------------------------------------------------------------


def test_match_inputs_binds_equal_count_duplicate_prompts_positionally() -> None:
    # A retry pattern: two managed input("Go? ") calls, both still present. Stored and current
    # have the SAME number of call sites for the identical prompt, so the multiset pass pairs
    # them in positional order (analyzer.py:363-367) — a stable shape that must NOT be flagged
    # as a rebind on every run. Neither is ambiguous (both resolve cleanly).
    stored = [(0, "Go? "), (1, "Go? ")]
    current = [(0, "Go? "), (1, "Go? ")]
    assert analyzer._match_inputs(stored, current) == {0: (0, False), 1: (1, False)}


# --------------------------------------------------------------------------
# argspec — argparse/typer edge branches
# --------------------------------------------------------------------------


def test_argparse_loop_without_add_argument_stays_static() -> None:
    # A for/while loop that contains NO add_argument call must not trip the "dynamic" degrade:
    # _any_call_inside_loop walks the loop, finds nothing, and falls back to the outer walk
    # (argspec.py:108->106). The parser is still read statically.
    src = (
        "import argparse\n"
        "p = argparse.ArgumentParser()\n"
        "p.add_argument('--x')\n"
        "for i in range(3):\n"
        "    print(i)\n"
    )
    spec = argspec.read_argparse(src)
    assert spec is not None
    assert spec.ok is True
    assert spec.reason == ""
    assert [f.flag for f in spec.fields] == ["--x"]


def test_typer_import_without_any_command_has_no_cli_surface() -> None:
    # typer is imported but there is neither an @app.command() function nor a typer.run(fn)
    # target, so _read_typer finds zero commands and returns None (argspec.py:376) — the script
    # has no readable CLI surface at all.
    assert argspec.read_cli("import typer\napp = typer.Typer()\n") is None


def test_argparse_choices_with_non_literal_element_degrades_to_free_text() -> None:
    # choices built from a non-literal (a bare Name we can't read statically): _literal_str_list
    # bails on the opaque element (argspec.py:528), so the field degrades to free text instead
    # of pretending to be a choice.
    src = (
        "import argparse\n"
        "p = argparse.ArgumentParser()\n"
        "p.add_argument('--color', choices=[PALETTE])\n"
    )
    spec = argspec.read_argparse(src)
    assert spec is not None
    field = spec.fields[0]
    assert field.degraded is True
    assert field.type == "str"
    assert field.choices == ()


# --------------------------------------------------------------------------
# flows.glob_feedback — unparsable value
# --------------------------------------------------------------------------


def test_glob_feedback_returns_none_on_unbalanced_quotes(tmp_path: Path) -> None:
    # The value has glob characters (so there's something to report) but can't be tokenized —
    # an unterminated quote makes shlex.split raise ValueError, and glob_feedback reports None
    # rather than crashing (flows.py:271-272).
    assert flows.glob_feedback('*.txt "unclosed', tmp_path) is None


# --------------------------------------------------------------------------
# launcher — exe executability + describe_command branches + win32 join
# --------------------------------------------------------------------------


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX-only executable-bit check")
def test_build_command_exe_not_executable_raises(tmp_path: Path) -> None:
    # An exe target that exists on disk but lacks the +x bit: _check_exe_exists raises
    # NotExecutableError with an actionable hint (launcher.py:109).
    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    os.chmod(exe, 0o644)
    entry = store.add_exe(exe)
    with pytest.raises(launcher.NotExecutableError) as excinfo:
        launcher.build_command(entry)
    assert "chmod +x" in str(excinfo.value)


def test_describe_command_python_includes_python_and_with_flags(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The transparency line for a python entry mirrors build_command: it echoes --python for a
    # pinned interpreter (launcher.py:233) and one --with per declared dependency
    # (launcher.py:235).
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/fake/uv")
    p = tmp_path / "s.py"
    p.write_text("print(1)\n", encoding="utf-8")
    entry = store.add_python(p)
    entry.meta.requires_python = ">=3.12"
    entry.meta.dependencies = ["httpx", "rich"]
    line = launcher.describe_command(entry)
    assert "--python" in line
    assert ">=3.12" in line
    assert line.count("--with") == 2
    assert "httpx" in line
    assert "rich" in line


def test_describe_command_exe_joins_source_and_extra(tmp_path: Path) -> None:
    # An exe entry's transparency line is just its source followed by the extra args
    # (launcher.py:239) — no uv, no isolation flags.
    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\n", encoding="utf-8")
    entry = store.add_exe(exe)
    line = launcher.describe_command(entry, ["--flag", "value"])
    assert entry.meta.source in line
    assert "--flag" in line
    assert "value" in line


def test_describe_command_falls_back_to_raw_template_when_params_unfilled(
    tmp_path: Path,
) -> None:
    # A command entry with an unfilled placeholder: _build_shell raises LaunchError (missing
    # value), and describe_command falls back to showing the raw template rather than failing
    # (launcher.py:242-243).
    entry = store.add_command("deploy {env} --force", name="deployer")
    assert launcher.describe_command(entry) == "deploy {env} --force"


def test_join_for_display_uses_list2cmdline_on_windows(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # On Windows the display join uses subprocess.list2cmdline, not shlex.join
    # (launcher.py:249).
    monkeypatch.setattr("sys.platform", "win32")
    argv = ["C:/tools/tool.exe", "a b", "c"]
    assert launch.join_for_display(argv) == subprocess.list2cmdline(argv)


# --------------------------------------------------------------------------
# ParamDecl.to_block_dict — env_source round-trip
# --------------------------------------------------------------------------


def test_param_env_source_survives_write_read_round_trip() -> None:
    # A secret sourced from an environment variable: to_block_dict serializes env_source
    # and from_block_dict reads it back, so the binding survives a full
    # write_params -> read_params round-trip.
    params = [
        ParamDecl(
            name="API_KEY",
            binding="const",
            type="str",
            secret=True,
            env_source="MY_API_KEY",
        )
    ]
    out = metawriter.write_params("x = 1\n", params)
    got = metawriter.read_params(out)
    assert len(got) == 1
    assert got[0].env_source == "MY_API_KEY"


# --------------------------------------------------------------------------
# pep723._toml_str — control-character escaping
# --------------------------------------------------------------------------


def test_build_block_escapes_control_char_in_dependency() -> None:
    # A control character embedded in a dependency string would break out of the single comment
    # line the PEP 723 block is built from; _toml_str escapes it to a \uXXXX sequence
    # (pep723.py:166) so the block re-parses.
    block = pep723.build_block(["pkg\x0bname"])
    assert "\\u000B" in block
    assert "\x0b" not in block


# --------------------------------------------------------------------------
# store.rename / store.update_description
# --------------------------------------------------------------------------


def _add_py(tmp_path: Path, name: str) -> store.Entry:
    p = tmp_path / f"{name}.py"
    p.write_text("print(1)\n", encoding="utf-8")
    return store.add_python(p, name=name)


def test_rename_tolerates_registry_row_vanishing_under_the_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # TOCTOU defense: resolve() found the entry, but by the time rename re-reads the registry
    # under the lock the index row is gone (a concurrent removal). rename must still update
    # meta.toml and return the renamed Entry, silently skipping the registry write
    # (store.py:467->470).
    entry = _add_py(tmp_path, "orig")
    orig_lock = store._registry_lock

    @contextlib.contextmanager
    def dropping_lock():  # type: ignore[no-untyped-def]
        with orig_lock():
            # simulate a concurrent removal of just this slug's index row
            entries = store._load_registry()
            entries.pop(entry.slug, None)
            store._save_registry(entries)
            yield

    monkeypatch.setattr(store, "_registry_lock", dropping_lock)
    renamed = store.rename("orig", "renamed")
    assert renamed.meta.name == "renamed"  # meta.toml is updated regardless of the index
    assert renamed.slug == entry.slug
    # The row stayed absent — rename did not resurrect it (there was no row to touch).
    assert entry.slug not in store._load_registry()


def test_update_description_updates_meta_and_registry_row(tmp_path: Path) -> None:
    # The happy path (store.py:476-485, row present): meta.toml is the truth and the registry
    # index row is refreshed too, so `list` reflects the new description without a rebuild.
    entry = _add_py(tmp_path, "doc")
    updated = store.update_description("doc", "a fresh description")
    assert updated.meta.description == "a fresh description"
    assert updated.slug == entry.slug
    assert store.resolve("doc").meta.description == "a fresh description"
    assert store._load_registry()[entry.slug]["description"] == "a fresh description"


def test_update_description_tolerates_registry_row_vanishing_under_the_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Same TOCTOU defense as rename, for update_description: the index row is gone by the time
    # the lock re-reads it, so the registry write is skipped while meta.toml still updates
    # (store.py:483->486).
    entry = _add_py(tmp_path, "doc")
    orig_lock = store._registry_lock

    @contextlib.contextmanager
    def dropping_lock():  # type: ignore[no-untyped-def]
        with orig_lock():
            entries = store._load_registry()
            entries.pop(entry.slug, None)
            store._save_registry(entries)
            yield

    monkeypatch.setattr(store, "_registry_lock", dropping_lock)
    updated = store.update_description("doc", "index-drifted description")
    assert updated.meta.description == "index-drifted description"
    # meta.toml on disk is the truth and was written regardless of the missing index row.
    assert store._read_meta(entry.dir).description == "index-drifted description"
    assert entry.slug not in store._load_registry()  # row stayed absent
