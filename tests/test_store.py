"""Headless tests for Store / Registry / doctor."""

from __future__ import annotations

import os
import sys
from pathlib import Path

import pytest


@pytest.fixture(autouse=True)
def isolated_dirs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    return tmp_path


@pytest.fixture
def sample_script(tmp_path: Path) -> Path:
    p = tmp_path / "hello.py"
    p.write_text(
        '"""打招呼腳本。\n\n多行 docstring。"""\nNAME = "world"\nprint(f"hi {NAME}")\n',
        encoding="utf-8",
    )
    return p


def test_add_copy_preserves_original_verbatim(sample_script: Path):
    from skit import store

    entry = store.add_python(sample_script)
    assert entry.meta.kind == "python"
    assert entry.meta.mode == "copy"
    # A5: the store copy must be byte-for-byte identical to the original file
    assert entry.script_path.read_bytes() == sample_script.read_bytes()
    # Description is taken from the first line of the docstring
    assert entry.meta.description == "打招呼腳本。"
    # Provenance fields
    assert entry.meta.source == str(sample_script.resolve())
    assert entry.meta.source_hash.startswith("sha256:")


def test_add_reference_points_to_origin(sample_script: Path):
    from skit import store

    entry = store.add_python(sample_script, mode="reference")
    assert entry.script_path == sample_script.resolve()
    assert not (entry.dir / "script.py").exists()


def test_name_conflict_rejected(sample_script: Path):
    from skit import store

    store.add_python(sample_script)
    with pytest.raises(store.NameConflictError):
        store.add_python(sample_script)


def test_slug_dedup(sample_script: Path, tmp_path: Path):
    from skit import store

    store.add_python(sample_script, name="任務A")  # slugified names may collide
    other = tmp_path / "hello2.py"
    other.write_text("print(1)\n", encoding="utf-8")
    e2 = store.add_python(other, name="任務B")
    entries = store.list_entries()
    assert len(entries) == 2
    assert len({e.slug for e in entries}) == 2
    assert e2.slug  # non-empty


def test_resolve_and_remove(sample_script: Path):
    from skit import store

    entry = store.add_python(sample_script, name="hi")
    assert store.resolve("hi").slug == entry.slug
    assert store.resolve(entry.slug).meta.name == "hi"
    store.remove("hi")
    with pytest.raises(store.NotFoundError):
        store.resolve("hi")
    assert not entry.dir.exists()


def test_remove_copy_does_not_touch_original(sample_script: Path):
    from skit import store

    store.add_python(sample_script, name="hi")
    store.remove("hi")
    assert sample_script.exists()


def test_add_command_entry():
    from skit import store

    entry = store.add_command("echo {msg}", name="回聲")
    assert entry.meta.kind == "command"
    assert entry.meta.template == "echo {msg}"
    assert entry.meta.workdir == "invoke"


def test_command_requires_nonempty_template():
    from skit import store

    with pytest.raises(store.StoreError):
        store.add_command("   ", name="空")


def test_doctor_rebuild_from_meta(sample_script: Path):
    from skit import store
    from skit.paths import registry_path

    store.add_python(sample_script, name="a")
    store.add_command("echo hi", name="b")
    # Simulate a corrupted registry
    os.unlink(registry_path())
    assert store.list_entries() == []
    count, problems = store.doctor_rebuild()
    assert count == 2
    assert problems == []
    names = {e.meta.name for e in store.list_entries()}
    assert names == {"a", "b"}


def test_doctor_reports_missing_reference(sample_script: Path):
    from skit import store

    store.add_python(sample_script, mode="reference", name="ref")
    sample_script.unlink()
    _, problems = store.doctor_rebuild()
    # Assert behaviour (slug + original path appear in the problem list), not locale copy
    assert any("ref" in p and str(sample_script) in p for p in problems)


def test_syntax_error_script_still_addable(tmp_path: Path):
    """A script with a syntax error must still be addable (description is left empty, no crash)."""
    from skit import store

    bad = tmp_path / "bad.py"
    bad.write_text("def broken(:\n", encoding="utf-8")
    entry = store.add_python(bad)
    assert entry.meta.description == ""


# ---------- add_python: file not found ----------


def test_add_python_missing_file_raises(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    from skit import store

    monkeypatch.setenv("SKIT_LANG", "en")
    with pytest.raises(store.StoreError, match="not found"):
        store.add_python(tmp_path / "ghost.py")


# ---------- add_exe ----------


def test_add_exe_roundtrip(tmp_path: Path):
    from skit import store

    exe = tmp_path / "mytool"
    exe.touch()
    entry = store.add_exe(exe, description="a tool")
    assert entry.meta.kind == "exe"
    assert entry.meta.mode == "reference"
    assert entry.meta.description == "a tool"


def test_add_exe_missing_file_raises(tmp_path: Path):
    from skit import store

    with pytest.raises(store.StoreError):
        store.add_exe(tmp_path / "no_such_tool")


# ---------- list_entries: skips corrupt meta silently ----------


def test_list_entries_skips_corrupt_meta(tmp_path: Path):
    from skit import store
    from skit.paths import scripts_dir

    store.add_command("echo hi", name="good")
    # Inject a directory with a corrupt meta.toml manually
    bad_dir = scripts_dir() / "bad-slug"
    bad_dir.mkdir(parents=True, exist_ok=True)
    (bad_dir / "meta.toml").write_text("not valid toml [[[", encoding="utf-8")
    entries = store.list_entries()
    # "bad-slug" is silently skipped; only "good" remains
    assert len(entries) == 1
    assert entries[0].meta.name == "good"


# ---------- doctor_rebuild: corrupt meta + missing-meta branches ----------


def test_doctor_rebuild_corrupt_meta(tmp_path: Path):
    from skit import store
    from skit.paths import scripts_dir

    # Inject a dir with no meta.toml (missing-meta branch)
    missing_dir = scripts_dir() / "orphan"
    missing_dir.mkdir(parents=True, exist_ok=True)
    # Inject a dir with corrupt meta.toml (corrupt-meta branch)
    corrupt_dir = scripts_dir() / "corrupt"
    corrupt_dir.mkdir(parents=True, exist_ok=True)
    (corrupt_dir / "meta.toml").write_text("[[[bad", encoding="utf-8")

    count, problems = store.doctor_rebuild()
    assert count == 0
    problem_text = "\n".join(problems)
    assert "orphan" in problem_text
    assert "corrupt" in problem_text


# ---------- update_dependencies: copy mode syncs PEP 723 block ----------


def test_update_dependencies_copy_mode(sample_script: Path):
    from skit import store

    entry = store.add_python(sample_script)
    updated = store.update_dependencies(entry.slug, ["httpx"], ">=3.11")
    script_text = updated.script_path.read_text(encoding="utf-8")
    assert "httpx" in script_text
    assert ">=3.11" in script_text


# ---------- resolve: ambiguous slug vs name handling ----------


def test_resolve_not_found_raises(tmp_path: Path):
    from skit import store

    with pytest.raises(store.NotFoundError):
        store.resolve("nonexistent")


# ---------------------------------------------------------------------------
# disk-usage helpers (public store API; shared by `doctor` and the TUI health check)
# ---------------------------------------------------------------------------


def test_dir_size_sums_only_files_recursively(tmp_path):
    from skit import store

    root = tmp_path / "lib"
    (root / "a").mkdir(parents=True)
    (root / "a" / "one.txt").write_bytes(b"x" * 100)
    (root / "two.txt").write_bytes(b"y" * 50)
    (root / "empty-dir").mkdir()  # directories themselves contribute nothing
    assert store.dir_size(root) == 150


def test_dir_size_missing_dir_is_zero(tmp_path):
    from skit import store

    assert store.dir_size(tmp_path / "nope") == 0


def test_dir_size_on_a_file_is_zero(tmp_path):
    from skit import store

    f = tmp_path / "f.txt"
    f.write_bytes(b"data")
    assert store.dir_size(f) == 0  # not a directory


def test_human_size_units_and_thresholds():
    from skit import store

    assert store.human_size(0) == "0 B"
    assert store.human_size(512) == "512 B"  # bytes stay integer, no decimal
    assert store.human_size(1024) == "1.0 KB"  # exactly at the boundary rolls up
    assert store.human_size(1536) == "1.5 KB"
    assert store.human_size(1024 * 1024) == "1.0 MB"
    assert store.human_size(3 * 1024 * 1024 * 1024) == "3.0 GB"
    assert store.human_size(5 * 1024**4) == "5120.0 GB"  # never rolls past GB


# ---- infer_kind: platform-correct executable detection --------------------------------------


def test_infer_kind_python_and_forced_exe(tmp_path: Path):
    from skit import store

    py = tmp_path / "a.py"
    py.write_text("print(1)\n", encoding="utf-8")
    assert store.infer_kind(py) == "python"
    # A .PY suffix is still python regardless of case; --exe forces exe even for a .py file.
    assert store.infer_kind(tmp_path / "B.PY") == "python"
    assert store.infer_kind(py, force_exe=True) == "exe"


@pytest.mark.skipif(
    sys.platform == "win32",
    reason="POSIX execute-bit semantics — os.access(X_OK) is always True on Windows, so the real "
    "os.access branch can't be exercised there (monkeypatching sys.platform doesn't change it).",
)
def test_infer_kind_posix_uses_execute_bit(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """On POSIX, an executable file is one with the execute bit set; a plain file is 'unknown'."""
    from skit import store

    monkeypatch.setattr("sys.platform", "linux")
    prog = tmp_path / "prog"
    prog.write_text("just bytes, no shebang\n", encoding="utf-8")
    assert store.infer_kind(prog) == "unknown"  # no +x yet
    prog.chmod(prog.stat().st_mode | 0o755)
    assert store.infer_kind(prog) == "exe"
    # a recognized shebang outranks the execute bit: this is a shell script, not an
    # opaque program, even without +x (approved inference change — multilang design)
    scripty = tmp_path / "deploy"
    scripty.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    assert store.infer_kind(scripty) == "shell"
    scripty.chmod(scripty.stat().st_mode | 0o755)
    assert store.infer_kind(scripty) == "shell"


def test_infer_kind_windows_uses_pathext_not_execute_bit(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """On Windows there is no execute bit (os.access(X_OK) is True for every file), so a runnable
    file is identified by its extension being in PATHEXT — a plain .txt must stay 'unknown'."""
    from skit import store

    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.setenv("PATHEXT", ".COM;.EXE;.BAT;.CMD")
    exe = tmp_path / "tool.exe"
    exe.write_bytes(b"MZ")
    txt = tmp_path / "notes.txt"
    txt.write_text("hi", encoding="utf-8")
    assert store.infer_kind(exe) == "exe"  # .EXE is in PATHEXT
    assert store.infer_kind(tmp_path / "run.BAT") == "unknown"  # not a file (missing) → unknown
    (tmp_path / "run.BAT").write_text("echo", encoding="utf-8")
    assert store.infer_kind(tmp_path / "run.BAT") == "exe"  # case-insensitive PATHEXT match
    assert store.infer_kind(txt) == "unknown"  # .txt is NOT in PATHEXT — the whole point


def test_infer_kind_windows_reads_pathext_env(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """The runnable set comes from PATHEXT itself, not a hardcoded list: a custom PATHEXT makes an
    otherwise-unknown extension runnable, and drops .exe when PATHEXT omits it."""
    from skit import store

    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.setenv("PATHEXT", ".PY1;.FOO")
    foo = tmp_path / "thing.foo"
    foo.write_text("x", encoding="utf-8")
    exe = tmp_path / "thing.exe"
    exe.write_bytes(b"MZ")
    assert store.infer_kind(foo) == "exe"  # honoured from the custom PATHEXT
    assert store.infer_kind(exe) == "unknown"  # .exe dropped because PATHEXT no longer lists it


def test_infer_kind_windows_falls_back_to_default_pathext(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """With PATHEXT unset (or empty), fall back to the built-in default so common programs still
    register as executables."""
    from skit import store

    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.delenv("PATHEXT", raising=False)
    bat = tmp_path / "go.bat"
    bat.write_text("echo hi", encoding="utf-8")
    assert store.infer_kind(bat) == "exe"  # .BAT is in the default fallback set
