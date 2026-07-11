"""Coverage-driven behavioral tests for skit.store and skit.atomic.

These target the currently-uncovered lines/branches in store.py (189-191, 204-205,
218, 253->261, 272->297) and atomic.py (line 30), plus adjacent edge cases found
while reading the code. Every test asserts an observable outcome (raised
exception, file contents, or returned model) rather than merely executing a line.
"""

from __future__ import annotations

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
    p.write_text('"""A greeter."""\nprint("hi")\n', encoding="utf-8")
    return p


# ---------- atomic.atomic_write_text (line 30) ----------


def test_atomic_write_text_roundtrip(tmp_path: Path):
    """atomic_write_text is currently unused by any caller in src/ (grep confirms it), but it is
    public API in atomic.py, so it must be verified directly: it writes UTF-8 text via the same
    tmp+replace path as atomic_write_bytes, and the final file contains the exact text."""
    from skit.atomic import atomic_write_text

    target = tmp_path / "nested" / "note.txt"
    atomic_write_text(target, "héllo 世界\n")
    assert target.read_text(encoding="utf-8") == "héllo 世界\n"
    # no leftover tmp files in the parent dir
    leftovers = [p for p in target.parent.iterdir() if p.name != target.name]
    assert leftovers == []


# ---------- store._add_entry: cleanup-and-reraise on write failure (189-191) ----------


def test_add_entry_cleans_up_dir_when_meta_write_fails(sample_script: Path, monkeypatch):
    """If writing meta.toml fails partway through _add_entry (e.g. disk error), the freshly
    created entry_dir must be removed and the original exception must propagate — no half-written
    entries should survive, and the registry must not be updated with a slug that has no dir."""
    from skit import store

    def boom(path, doc):
        raise OSError("simulated disk failure")

    monkeypatch.setattr(store, "atomic_write_toml", boom)

    with pytest.raises(OSError, match="simulated disk failure"):
        store.add_python(sample_script, name="willfail")

    # No entry directory should remain on disk
    from skit.paths import scripts_dir

    assert list(scripts_dir().iterdir()) == []
    # Registry must not have been touched (still empty / absent)
    assert store.list_entries() == []


def test_add_entry_cleans_up_dir_when_copy_fails(tmp_path: Path, monkeypatch):
    """Same cleanup guarantee, but triggered by shutil.copy2 raising (e.g. permission denied while
    copying the payload) rather than the meta write."""
    from skit import store

    src = tmp_path / "s.py"
    src.write_text("print(1)\n", encoding="utf-8")

    def boom(*a, **k):
        raise OSError("simulated copy failure")

    monkeypatch.setattr(store.shutil, "copy2", boom)

    with pytest.raises(OSError, match="simulated copy failure"):
        store.add_python(src, name="copyfail")

    from skit.paths import scripts_dir

    assert list(scripts_dir().iterdir()) == []
    assert store.list_entries() == []


# ---------- store.list_entries: OSError branch distinct from TOMLDecodeError (204-205) ----------


def test_list_entries_skips_entry_with_unreadable_meta(tmp_path: Path):
    """list_entries must silently skip an entry whose meta.toml raises OSError (not just
    TOMLDecodeError) on read — e.g. meta.toml is actually a directory, so open() raises
    IsADirectoryError, a subclass of OSError.

    Note: list_entries only ever visits slugs that are present in registry.toml (it iterates
    _load_registry(), not the scripts/ directory), so the broken entry must be registered too -
    merely dropping a stray directory under scripts/ (as the existing
    test_list_entries_skips_corrupt_meta in test_store.py does) never reaches the except branch at
    all, since that slug is absent from the registry and the loop never visits it."""
    from skit import store
    from skit.atomic import atomic_write_toml
    from skit.paths import registry_path, scripts_dir

    store.add_command("echo hi", name="good")
    broken_dir = scripts_dir() / "broken-slug"
    broken_dir.mkdir(parents=True, exist_ok=True)
    # meta.toml as a directory instead of a file -> open() raises IsADirectoryError (OSError)
    (broken_dir / "meta.toml").mkdir()
    entries = store._load_registry()
    entries["broken-slug"] = {"name": "broken", "kind": "command", "description": ""}
    atomic_write_toml(registry_path(), {"entries": entries})

    result = store.list_entries()
    assert [e.meta.name for e in result] == ["good"]


# ---------- store.resolve: name-lookup branch and ambiguity (218) ----------


def test_resolve_by_name_when_slug_differs_from_name(sample_script: Path):
    """When name_or_slug is not itself a registry key, resolve() must fall back to matching by
    stored name and succeed if exactly one entry has that name. Using a name that slugifies to a
    different string than the name itself exercises the real fallback path (not the direct-slug
    hit)."""
    from skit import store

    entry = store.add_python(sample_script, name="任務 A!!")
    assert entry.slug != "任務 A!!"  # slugified, so direct-slug lookup would miss
    resolved = store.resolve("任務 A!!")
    assert resolved.slug == entry.slug
    assert resolved.meta.name == "任務 A!!"


def test_resolve_ambiguous_name_raises_not_found(tmp_path: Path):
    """If the registry (e.g. hand-edited or corrupted) contains two entries sharing the same
    `name` field, resolving by that name must not silently pick one - it should raise
    NotFoundError, since len(matches) != 1."""
    from skit import store
    from skit.atomic import atomic_write_toml
    from skit.paths import registry_path, scripts_dir

    # Build two valid entries normally, then hand-edit the registry so both share one name.
    store.add_command("echo one", name="first")
    store.add_command("echo two", name="second")
    entries = {
        p.name: {"name": "dup", "kind": "command", "description": ""}
        for p in scripts_dir().iterdir()
    }
    atomic_write_toml(registry_path(), {"entries": entries})

    with pytest.raises(store.NotFoundError):
        store.resolve("dup")


# ---------- store.update_dependencies: copy-mode script missing (253->261) ----------


def test_update_dependencies_copy_mode_missing_script_skips_pep723_sync(sample_script: Path):
    """In copy mode, update_dependencies normally rewrites the copy's PEP 723 block. But if the
    copy's script.py has been removed out-of-band (e.g. manually deleted), the function must not
    crash - it should skip the sync and still persist the dependency list to meta.toml."""
    from skit import store

    entry = store.add_python(sample_script, name="nofile")
    entry.script_path.unlink()
    assert not entry.script_path.exists()

    updated = store.update_dependencies("nofile", ["requests"], ">=3.12")
    assert updated.meta.dependencies == ["requests"]
    assert updated.meta.requires_python == ">=3.12"
    assert not (entry.dir / "script.py").exists()  # still absent, no crash


# ---------- store.doctor_rebuild: scripts_dir does not exist yet (272->297) ----------


def test_doctor_rebuild_when_scripts_dir_absent(tmp_path: Path):
    """doctor_rebuild must handle a brand-new store where scripts_dir() has never been created
    (no scripts added yet) without raising, producing an empty rebuilt registry."""
    from skit import store
    from skit.paths import scripts_dir

    assert not scripts_dir().exists()
    count, problems = store.doctor_rebuild()
    assert count == 0
    assert problems == []
    # doctor_rebuild always (re)writes registry.toml, even when nothing was found
    from skit.paths import registry_path

    assert registry_path().exists()


# ---------- adversarial edge cases beyond the coverage report ----------


def test_add_python_unicode_path(tmp_path: Path):
    """Unicode-named source files and script directories must round-trip correctly."""
    from skit import store

    src_dir = tmp_path / "腳本目錄"
    src_dir.mkdir()
    src = src_dir / "問候.py"
    src.write_text("print('嗨')\n", encoding="utf-8")

    entry = store.add_python(src, name="問候腳本")
    assert entry.meta.source == str(src.resolve())
    assert entry.script_path.read_text(encoding="utf-8") == "print('嗨')\n"


def test_list_entries_empty_store_returns_empty_list(tmp_path: Path):
    """A freshly-initialized store (no registry.toml at all) must report zero entries, not
    raise."""
    from skit import store
    from skit.paths import registry_path

    assert not registry_path().exists()
    assert store.list_entries() == []


def test_extract_placeholders_dedupes_and_ignores_escaped_braces():
    from skit.store import extract_placeholders

    result = extract_placeholders("{a} {{literal}} {b} {a}")
    assert result == ["a", "b"]


def test_add_command_records_deduped_params_in_order(tmp_path: Path):
    from skit import store

    entry = store.add_command("cp {src} {dst} {src}", name="copycmd")
    assert entry.meta.params == ["src", "dst"]


def test_remove_unknown_raises_not_found(tmp_path: Path):
    from skit import store

    with pytest.raises(store.NotFoundError):
        store.remove("does-not-exist")
