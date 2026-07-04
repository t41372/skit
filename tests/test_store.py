"""Headless tests for Store / Registry / doctor."""

from __future__ import annotations

import os
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


def test_add_python_missing_file_raises(tmp_path: Path):
    from skit import store

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
