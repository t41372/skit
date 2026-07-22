"""Behavioral tests for metadata validation, registry recovery, copy-mode defaults and locking.

Each test asserts user-observable behavior rather than merely executing a branch.
"""

from __future__ import annotations

import os
import threading
import time
from pathlib import Path

import pytest


@pytest.fixture(autouse=True)
def isolated_dirs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


@pytest.fixture
def sample_script(tmp_path: Path) -> Path:
    p = tmp_path / "hello.py"
    p.write_text('"""A greeter."""\nprint("hi")\n', encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# Required metadata keys in otherwise valid TOML
# ---------------------------------------------------------------------------


def test_from_toml_dict_missing_name_raises_scriptmetaerror_not_keyerror():
    from skit.models import ScriptMeta, ScriptMetaError

    with pytest.raises(ScriptMetaError):
        ScriptMeta.from_toml_dict({"kind": "python"})


def test_from_toml_dict_missing_kind_raises_scriptmetaerror_not_keyerror():
    from skit.models import ScriptMeta, ScriptMetaError

    with pytest.raises(ScriptMetaError):
        ScriptMeta.from_toml_dict({"name": "x"})


def _write_missing_key_meta(tmp_path: Path) -> Path:
    from skit.paths import scripts_dir

    bad_dir = scripts_dir() / "bad-slug"
    bad_dir.mkdir(parents=True, exist_ok=True)
    # Valid TOML, but missing the required "name" key.
    (bad_dir / "meta.toml").write_text('schema = 1\nkind = "python"\n', encoding="utf-8")
    return bad_dir


def test_list_entries_skips_valid_toml_missing_name_key(sample_script, tmp_path):
    from skit import store
    from skit.atomic import atomic_write_toml
    from skit.paths import registry_path

    store.add_command("echo hi", name="good")
    _write_missing_key_meta(tmp_path)
    entries = store._load_registry()
    entries["bad-slug"] = {"name": "bad", "kind": "python", "description": ""}
    atomic_write_toml(registry_path(), {"entries": entries})

    result = store.list_entries()  # must not raise KeyError
    assert [e.meta.name for e in result] == ["good"]


def test_doctor_rebuild_reports_missing_key_instead_of_crashing(sample_script, tmp_path):
    from skit import store

    store.add_python(sample_script, name="good")
    _write_missing_key_meta(tmp_path)

    count, problems = store.doctor_rebuild()  # must not raise KeyError
    assert count == 1
    assert any("bad-slug" in p for p in problems)


def test_resolve_corrupt_missing_key_meta_raises_notfounderror_not_keyerror(tmp_path):
    from skit import store
    from skit.atomic import atomic_write_toml
    from skit.paths import registry_path

    _write_missing_key_meta(tmp_path)
    atomic_write_toml(
        registry_path(),
        {"entries": {"bad-slug": {"name": "bad", "kind": "python", "description": ""}}},
    )

    with pytest.raises(store.NotFoundError):
        store.resolve("bad-slug")


# ---------------------------------------------------------------------------
# Fix-beta follow-up (models.py from_toml_dict): a wrong-typed dependencies/params
# field (e.g. `dependencies = 5`, valid TOML) used to raise a raw TypeError past the
# missing-key guard, escaping _META_CORRUPTION and crashing list/resolve/doctor. Also
# covers the ScriptMetaError message now being gettext-wrapped like every other
# skit-authored user-facing string.
# ---------------------------------------------------------------------------


def test_from_toml_dict_scalar_dependencies_raises_scriptmetaerror_not_typeerror():
    from skit.models import ScriptMeta, ScriptMetaError

    with pytest.raises(ScriptMetaError):
        ScriptMeta.from_toml_dict({"name": "x", "kind": "python", "dependencies": 5})


def test_from_toml_dict_scalar_params_raises_scriptmetaerror_not_typeerror():
    from skit.models import ScriptMeta, ScriptMetaError

    with pytest.raises(ScriptMetaError):
        ScriptMeta.from_toml_dict({"name": "x", "kind": "command", "params": 5})


def _write_bad_type_meta(tmp_path: Path, field: str) -> Path:
    from skit.paths import scripts_dir

    bad_dir = scripts_dir() / "bad-type-slug"
    bad_dir.mkdir(parents=True, exist_ok=True)
    # Valid TOML: name/kind both present, but `field` is a truthy non-iterable scalar.
    (bad_dir / "meta.toml").write_text(
        f'schema = 1\nname = "bad"\nkind = "python"\n{field} = 5\n', encoding="utf-8"
    )
    return bad_dir


def test_list_entries_skips_scalar_dependencies_meta(sample_script, tmp_path):
    from skit import store
    from skit.atomic import atomic_write_toml
    from skit.paths import registry_path

    store.add_command("echo hi", name="good")
    _write_bad_type_meta(tmp_path, "dependencies")
    entries = store._load_registry()
    entries["bad-type-slug"] = {"name": "bad", "kind": "python", "description": ""}
    atomic_write_toml(registry_path(), {"entries": entries})

    result = store.list_entries()  # must not raise TypeError
    assert [e.meta.name for e in result] == ["good"]


def test_doctor_rebuild_reports_scalar_params_instead_of_crashing(sample_script, tmp_path):
    from skit import store

    store.add_python(sample_script, name="good")
    _write_bad_type_meta(tmp_path, "params")

    count, problems = store.doctor_rebuild()  # must not raise TypeError
    assert count == 1
    assert any("bad-type-slug" in p for p in problems)


def test_resolve_scalar_dependencies_meta_raises_notfounderror_not_typeerror(tmp_path):
    from skit import store
    from skit.atomic import atomic_write_toml
    from skit.paths import registry_path

    _write_bad_type_meta(tmp_path, "dependencies")
    atomic_write_toml(
        registry_path(),
        {"entries": {"bad-type-slug": {"name": "bad", "kind": "python", "description": ""}}},
    )

    with pytest.raises(store.NotFoundError):
        store.resolve("bad-type-slug")


def test_from_toml_dict_missing_key_message_is_gettext_wrapped():
    """The ScriptMetaError message must flow through i18n.gettext (not a raw f-string) so it
    participates in translation/pseudo-localization like every other skit-authored user-facing
    message."""
    from skit import i18n
    from skit.models import ScriptMeta, ScriptMetaError

    i18n.init("x-pseudo")
    try:
        with pytest.raises(ScriptMetaError) as excinfo:
            ScriptMeta.from_toml_dict({"kind": "python"})
        message = str(excinfo.value)
        assert message.startswith("⟦")
        assert message.endswith("⟧")
        assert "name" in message  # the %(keys)s placeholder value survives pseudoization
    finally:
        i18n.init("en")


def test_from_toml_dict_invalid_type_message_is_gettext_wrapped():
    from skit import i18n
    from skit.models import ScriptMeta, ScriptMetaError

    i18n.init("x-pseudo")
    try:
        with pytest.raises(ScriptMetaError) as excinfo:
            ScriptMeta.from_toml_dict({"name": "x", "kind": "python", "dependencies": 5})
        message = str(excinfo.value)
        assert message.startswith("⟦")
        assert message.endswith("⟧")
        assert "dependencies" in message
    finally:
        i18n.init("en")


# ---------------------------------------------------------------------------
# Lost-registry name collisions must not overwrite stored entries
# ---------------------------------------------------------------------------


def test_lost_registry_name_collision_does_not_clobber_existing_script(sample_script, tmp_path):
    from skit import store
    from skit.paths import registry_path

    entry = store.add_python(sample_script, name="Deploy")
    original_bytes = entry.script_path.read_bytes()
    os.unlink(registry_path())  # simulate a lost/corrupt registry (registry.toml is only an index)

    other = tmp_path / "other.py"
    other.write_text("print('a completely different script')\n", encoding="utf-8")
    with pytest.raises(store.NameConflictError):
        store.add_python(other, name="Deploy")

    # The original stored copy must survive untouched, never silently overwritten.
    assert entry.script_path.read_bytes() == original_bytes


def test_lost_registry_slug_collision_gets_deduped_not_overwritten(sample_script, tmp_path):
    """Different names that slugify to the same base slug must not clobber each other even when
    registry.toml (the pre-fix only source of "taken slugs") is lost."""
    from skit import store
    from skit.paths import registry_path

    entry = store.add_python(sample_script, name="deploy")
    original_bytes = entry.script_path.read_bytes()
    os.unlink(registry_path())

    other = tmp_path / "other.py"
    other.write_text("print('different')\n", encoding="utf-8")
    new_entry = store.add_python(other, name="DEPLOY")  # slugifies to the same base as "deploy"

    assert new_entry.slug != entry.slug
    assert entry.script_path.read_bytes() == original_bytes


def test_add_entry_refuses_to_reuse_an_existing_nonempty_directory(
    monkeypatch, sample_script, tmp_path
):
    """Defense in depth: even if slug allocation were ever wrong (a future bug, a race), _add_entry
    must not silently reuse — and overwrite — a directory that already holds a stored script."""
    from skit import store

    entry = store.add_python(sample_script, name="first")
    original_bytes = entry.script_path.read_bytes()
    monkeypatch.setattr(store, "_unique_slug", lambda base, existing: entry.slug)

    other = tmp_path / "other.py"
    other.write_text("print('clobber me not')\n", encoding="utf-8")
    with pytest.raises(store.StoreError):
        store.add_python(other, name="second")

    assert entry.script_path.read_bytes() == original_bytes


def test_add_entry_still_reuses_preexisting_empty_slug_dir(tmp_path):
    """Preserves the existing, intentional behavior: an empty leftover directory (nothing to
    protect) is still safe to reuse, matching the pre-fix mkdir(exist_ok=True) semantics."""
    from skit import store
    from skit.paths import scripts_dir

    pre = scripts_dir() / "myname"
    pre.mkdir(parents=True)
    entry = store.add_command("echo hi", name="myname")
    assert entry.slug == "myname"


def test_fs_truth_ignores_stray_non_directory_entries_in_scripts_dir(sample_script):
    """scripts_dir() should only ever contain entry directories, but _fs_truth must not choke if a
    stray non-directory file ends up there (e.g. a leftover .DS_Store-style file)."""
    from skit import store
    from skit.paths import scripts_dir

    scripts_dir().mkdir(parents=True, exist_ok=True)
    (scripts_dir() / "stray-file.txt").write_text("not a script dir", encoding="utf-8")

    entry = store.add_python(sample_script, name="ok")
    assert entry.meta.name == "ok"


def test_fs_truth_skips_unreadable_meta_in_unregistered_orphan_directory(sample_script, tmp_path):
    """An unregistered, non-empty orphan directory whose meta.toml can't be read must not crash
    _fs_truth's name-conflict scan — it's excluded from the name set (doctor --rebuild will report
    it), but its slug is still protected from reuse."""
    from skit import store
    from skit.paths import scripts_dir

    orphan = scripts_dir() / "orphan"
    orphan.mkdir(parents=True, exist_ok=True)
    (orphan / "meta.toml").write_text("not valid toml [[[", encoding="utf-8")
    (orphan / "script.py").write_text("print(1)\n", encoding="utf-8")

    entry = store.add_python(sample_script, name="ok")  # must not raise
    assert entry.meta.name == "ok"
    assert entry.slug != "orphan"


# ---------------------------------------------------------------------------
# Gap: copy-mode workdir default + launcher fallback (store side)
# ---------------------------------------------------------------------------


def test_add_python_copy_mode_defaults_workdir_to_invoke(sample_script):
    from skit import store

    entry = store.add_python(sample_script)
    assert entry.meta.workdir == "invoke"


def test_add_python_reference_mode_still_defaults_workdir_to_origin(sample_script):
    from skit import store

    entry = store.add_python(sample_script, mode="reference")
    assert entry.meta.workdir == "origin"


def test_add_python_copy_mode_explicit_workdir_override_still_respected(sample_script):
    from skit import store

    entry = store.add_python(sample_script, workdir="store")
    assert entry.meta.workdir == "store"


# ---------------------------------------------------------------------------
# Corrupt registry quarantine and reconstruction
# ---------------------------------------------------------------------------


def test_corrupt_registry_is_backed_up_and_degrades_to_empty(tmp_path):
    from skit import store
    from skit.paths import registry_path

    store.add_command("echo hi", name="a")
    path = registry_path()
    corrupt_bytes = b"not valid toml [[["
    path.write_bytes(corrupt_bytes)

    assert store.list_entries() == []  # degrades gracefully, does not raise

    backup = path.with_name(f"{path.name}.corrupt")
    assert backup.exists()
    assert backup.read_bytes() == corrupt_bytes
    assert not path.exists()  # renamed away, not left in place to re-trigger the branch


def test_corrupt_registry_recovers_fully_via_doctor_rebuild(tmp_path):
    """The real data (scripts/<slug> metas) is untouched by a corrupt registry.toml, so
    doctor --rebuild fully recovers it despite the corruption."""
    from skit import store
    from skit.paths import registry_path

    store.add_command("echo hi", name="a")
    registry_path().write_text("not valid toml [[[", encoding="utf-8")

    count, problems = store.doctor_rebuild()
    assert count == 1
    assert problems == []
    assert {e.meta.name for e in store.list_entries()} == {"a"}


# ---------------------------------------------------------------------------
# Non-UTF-8 copy fidelity when dependency injection is unavailable
# ---------------------------------------------------------------------------


def test_add_python_non_utf8_source_skips_injection_keeps_deps_in_meta(tmp_path):
    from skit import store

    src = tmp_path / "latin1.py"
    src.write_bytes(b'# -*- coding: latin-1 -*-\nX = 1\nS = "caf\xe9"\nimport requests\n')

    entry = store.add_python(src, dependencies=["requests"])

    # The copy must be byte-exact: no lossy utf-8 round-trip corrupting the latin-1 byte.
    assert entry.script_path.read_bytes() == src.read_bytes()
    # Injection was skipped, so the deps must be recoverable from meta instead (like reference
    # mode), so the launcher can still supply them via --with at run time.
    assert entry.meta.dependencies == ["requests"]


def test_add_python_utf8_source_still_injects_normally(tmp_path):
    """Control case: a strict-UTF-8 source with no existing block still gets the PEP 723 block
    injected into the copy, and meta.dependencies is cleared (single source of truth)."""
    from skit import store

    src = tmp_path / "plain.py"
    src.write_text("print(1)\n", encoding="utf-8")

    entry = store.add_python(src, dependencies=["httpx"])
    assert "httpx" in entry.script_path.read_text(encoding="utf-8")
    assert entry.meta.dependencies is None


def test_add_python_injected_write_failure_rolls_back_entire_entry(tmp_path, monkeypatch):
    """If the injected write fails, nothing may survive half-committed: no registered entry, no
    leftover directory, and therefore no scenario where deps were silently dropped."""
    from skit import store
    from skit.paths import scripts_dir

    src = tmp_path / "nodoc.py"
    src.write_text("print(1)\n", encoding="utf-8")

    real_write_text = Path.write_text

    def boom(self, *a, **kw):
        if self.name == "script.py":
            raise OSError("disk full (simulated)")
        return real_write_text(self, *a, **kw)

    monkeypatch.setattr(Path, "write_text", boom)
    with pytest.raises(OSError, match="disk full"):
        store.add_python(src, dependencies=["requests"])

    assert store.list_entries() == []
    root = scripts_dir()
    remaining = list(root.iterdir()) if root.exists() else []
    assert remaining == []


# ---------------------------------------------------------------------------
# Registry mutation serialization
# ---------------------------------------------------------------------------


def test_registry_lock_serializes_concurrent_holders():
    from skit import store

    order: list[str] = []

    def hold_lock() -> None:
        with store._registry_lock():
            order.append("A-enter")
            time.sleep(0.3)
            order.append("A-exit")

    t = threading.Thread(target=hold_lock)
    t.start()
    time.sleep(0.1)  # give thread A time to acquire the lock first
    with store._registry_lock():
        order.append("B-enter")
    t.join()

    # B must not enter until A has fully exited: no interleaving of the critical section.
    assert order.index("A-exit") < order.index("B-enter")


def test_registry_lock_uses_a_versioned_persistent_native_inode(tmp_path):
    from skit import store
    from skit.paths import registry_path

    old_protocol = registry_path().with_suffix(".lock")
    native = registry_path().with_suffix(".native.lock")
    with store._registry_lock():
        assert native.is_file()
        first_inode = native.stat().st_ino
    assert native.is_file()
    assert not old_protocol.exists()  # released O_EXCL builds must not stall on our inode
    with store._registry_lock():
        assert native.stat().st_ino == first_inode


def test_concurrent_add_python_both_succeed_with_distinct_slugs(tmp_path):
    """Regression test for the read-modify-write race: without the lock, concurrent adds can
    silently lose an entry (last writer wins on registry.toml). With the lock, all of them land."""
    import threading

    from skit import store

    errors: list[BaseException] = []

    def add(idx: int) -> None:
        try:
            p = tmp_path / f"s{idx}.py"
            p.write_text(f"print({idx})\n", encoding="utf-8")
            store.add_python(p, name=f"script-{idx}")
        except BaseException as exc:  # captured for the assertion below, not swallowed
            errors.append(exc)

    threads = [threading.Thread(target=add, args=(i,)) for i in range(8)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    assert errors == []
    entries = store.list_entries()
    assert len(entries) == 8
    assert len({e.slug for e in entries}) == 8


# ---------------------------------------------------------------------------
# _sync_python_block: the PEP 723 sync path's own strict-decode gate
# (the copy-mode twin of add_python's non-UTF-8 fidelity rule)
# ---------------------------------------------------------------------------


def test_update_dependencies_copy_non_utf8_leaves_stored_copy_byte_identical(tmp_path):
    """`skit deps` on a copy-mode python entry syncs the copy's PEP 723 block — but only after a
    STRICT re-decode (read_bytes().decode('utf-8')). A copy that isn't valid UTF-8 makes that
    decode raise, and the sync RETURNS silently, leaving the copy byte-exact. The previous
    errors='replace' round-trip would have rewritten every non-UTF-8 byte as U+FFFD (real
    corruption). The dependency edit still lands in meta (delivered via --with at run time)."""
    from skit import store

    src = tmp_path / "latin1.py"
    src.write_bytes(b"# -*- coding: latin-1 -*-\nTEXT = 'caf\xe9'\n")
    entry = store.add_python(src)  # non-UTF-8 copy, no injection at add
    before = entry.script_path.read_bytes()

    updated = store.update_dependencies(entry.slug, ["requests"])

    assert entry.script_path.read_bytes() == before  # byte-identical: no U+FFFD corruption
    assert updated.meta.dependencies == ["requests"]  # recorded in meta instead


def test_update_dependencies_copy_utf8_syncs_block_and_stays_utf8(tmp_path):
    """The happy path is unchanged: a strict-UTF-8 copy still gets its PEP 723 block updated
    (written atomically), and the copy round-trips as UTF-8 with the edit applied."""
    from skit import store

    src = tmp_path / "plain.py"
    src.write_text("print(1)\n", encoding="utf-8")
    entry = store.add_python(src)

    updated = store.update_dependencies(entry.slug, ["httpx"], ">=3.11")

    text = updated.script_path.read_text(encoding="utf-8")
    assert "httpx" in text
    assert ">=3.11" in text


def test_update_dependencies_copy_sync_swallows_read_oserror(tmp_path, monkeypatch):
    """The sync's guard covers OSError too, not just decode failures: if the stored copy can't be
    read back at all, the edit still lands in meta and the sync degrades silently rather than
    crashing the whole update."""
    from skit import store

    src = tmp_path / "plain.py"
    src.write_text("print(1)\n", encoding="utf-8")
    entry = store.add_python(src)

    real_read_bytes = Path.read_bytes

    def boom(self):
        if self.name == "script.py":
            raise OSError("simulated: unreadable stored copy")
        return real_read_bytes(self)

    monkeypatch.setattr(Path, "read_bytes", boom)
    updated = store.update_dependencies(entry.slug, ["httpx"])  # must not crash

    assert updated.meta.dependencies == ["httpx"]
