"""Behavioral tests targeting surviving mutmut mutants in store.py and its neighbors
(paths.py, atomic.py, config.py, argstate.py, models.py).

Each test is aimed at killing one or more specific surviving mutants (see SURVIVORS.txt at
the repo root) by asserting on real, user-observable behavior: exact file/dir names,
exact messages, exact ordering, and exact error/edge-case handling. Mutants that turned
out to be semantically equivalent are NOT force-killed here; they're called out in the
final report instead.
"""

from __future__ import annotations

import hashlib
import os
import shutil
import socket
import tempfile
import threading
from datetime import UTC, datetime
from pathlib import Path
from typing import override

import pytest

from skit import argstate, config, store
from skit.atomic import atomic_write_bytes
from skit.models import ScriptMeta, now_iso, slugify
from skit.params import ParamDecl
from skit.paths import (
    config_dir,
    data_dir,
    private_bin_dir,
    registry_path,
    scripts_dir,
    state_dir,
    values_dir,
)


@pytest.fixture(autouse=True)
def _force_english_locale(monkeypatch: pytest.MonkeyPatch) -> None:
    """Pin gettext output to English regardless of the host locale (LC_ALL/LANG).

    Several tests in this file assert on exact English message text; without this,
    they fail under a non-English locale even though the conftest i18n-reset fixture
    doesn't touch LC_*/LANG itself.
    """
    monkeypatch.setenv("SKIT_LANG", "en")


# ---------------------------------------------------------------------------
# paths.py
# ---------------------------------------------------------------------------


def test_data_state_config_dir_fall_back_to_real_platformdirs_app_name(monkeypatch):
    """When the SKIT_*_DIR overrides are unset, each dir must fall back to platformdirs
    resolved with the real app name "skit" — not a directory with no app name at all
    (which is what happens if _APP is dropped, or if the fallback call is replaced
    outright with Path(None))."""
    monkeypatch.delenv("SKIT_DATA_DIR", raising=False)
    monkeypatch.delenv("SKIT_STATE_DIR", raising=False)
    monkeypatch.delenv("SKIT_CONFIG_DIR", raising=False)

    for fn in (data_dir, state_dir, config_dir):
        result = fn()
        assert "skit" in result.parts


def test_data_and_state_dir_honor_their_exact_env_override(tmp_path, monkeypatch):
    """SKIT_DATA_DIR / SKIT_STATE_DIR must be read by exactly those names and win over the
    platformdirs fallback (which conftest redirects under a fake HOME — so a mutant that drops
    or corrupts the env lookup resolves somewhere else and fails the equality)."""
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "override-data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "override-state"))
    assert data_dir() == tmp_path / "override-data"
    assert state_dir() == tmp_path / "override-state"


def test_scripts_dir_registry_path_private_bin_dir_values_dir_exact_names():
    assert scripts_dir().name == "scripts"
    assert registry_path().name == "registry.toml"
    assert private_bin_dir().name == "bin"
    assert values_dir().name == "values"


# ---------------------------------------------------------------------------
# atomic.py
# ---------------------------------------------------------------------------


def test_atomic_write_bytes_temp_file_is_sibling_hidden_and_suffixed(tmp_path, monkeypatch):
    """The tmp file must be created in the SAME directory as the target (required for an
    atomic os.replace across the same filesystem), and named so it's recognizable/hidden
    junk if a crash leaves it behind: ".<name>.<random>.tmp"."""
    calls: dict[str, object] = {}
    original_mkstemp = tempfile.mkstemp

    def spy(*args, **kwargs):
        calls.update(kwargs)
        return original_mkstemp(*args, **kwargs)

    monkeypatch.setattr(tempfile, "mkstemp", spy)
    target = tmp_path / "sub" / "file.txt"
    atomic_write_bytes(target, b"hello")

    assert calls["dir"] == target.parent
    assert calls["prefix"] == ".file.txt."
    assert calls["suffix"] == ".tmp"
    assert target.read_bytes() == b"hello"


def test_atomic_write_bytes_suppresses_only_cleanup_oserror_not_original_error(
    tmp_path, monkeypatch
):
    """On failure, atomic_write_bytes must re-raise the ORIGINAL error, not a secondary
    error from the best-effort tmp-file cleanup. This requires the cleanup to suppress
    OSError specifically -- contextlib.suppress(None) would instead raise a TypeError
    from inside __exit__, masking the real failure."""
    target = tmp_path / "file.txt"

    def bad_replace(_src, _dst):
        raise RuntimeError("replace boom")

    def bad_unlink(_path):
        raise OSError("cleanup also failed")

    monkeypatch.setattr(os, "replace", bad_replace)
    monkeypatch.setattr(os, "unlink", bad_unlink)

    with pytest.raises(RuntimeError, match="replace boom"):
        atomic_write_bytes(target, b"data")


# ---------------------------------------------------------------------------
# models.py
# ---------------------------------------------------------------------------


def test_now_iso_is_utc_with_no_microseconds():
    value = now_iso()
    assert value.endswith("+00:00")
    assert "." not in value
    # sanity: it round-trips and is close to "now"
    parsed = datetime.fromisoformat(value)
    assert abs((datetime.now(UTC) - parsed).total_seconds()) < 10


@pytest.mark.parametrize(
    ("name", "expected"),
    [
        ("Hello World", "hello-world"),
        ("  leading and trailing  ", "leading-and-trailing"),
        ("!!!only-punct!!!", "only-punct"),
        ("###", "script"),
        ("", "script"),
    ],
)
def test_slugify(name, expected):
    assert slugify(name) == expected


# ---------------------------------------------------------------------------
# config.py
# ---------------------------------------------------------------------------


def test_config_path_is_config_toml():
    assert config._config_path().name == "config.toml"


def test_looks_blocked_uses_caller_timeout_and_https_port(monkeypatch):
    calls: list[tuple[tuple[str, int], float | None]] = []

    class _FakeConn:
        def __enter__(self):
            return self

        def __exit__(self, *exc):
            return False

    def fake_create_connection(addr, timeout=None):
        calls.append((addr, timeout))
        return _FakeConn()

    monkeypatch.setattr(socket, "create_connection", fake_create_connection)

    assert config.looks_blocked(timeout=7.25) is False
    assert calls == [(("pypi.org", 443), 7.25), (("github.com", 443), 7.25)]


def test_looks_blocked_default_timeout_is_2_5(monkeypatch):
    calls: list[tuple[tuple[str, int], float | None]] = []

    class _FakeConn:
        def __enter__(self):
            return self

        def __exit__(self, *exc):
            return False

    def fake_create_connection(addr, timeout=None):
        calls.append((addr, timeout))
        return _FakeConn()

    monkeypatch.setattr(socket, "create_connection", fake_create_connection)

    config.looks_blocked()
    assert calls[0] == (("pypi.org", 443), 2.5)


# ---------------------------------------------------------------------------
# argstate.py
# ---------------------------------------------------------------------------


def _spec(name: str, *, default=None) -> ParamDecl:
    return ParamDecl(name=name, binding="const", type="str", default=default, secret=False)


def test_save_preset_preserves_previously_saved_presets_for_same_slug(tmp_path, monkeypatch):
    """save_preset must read-modify-write against the SAME slug's file and under the
    correct "presets" key -- a wrong slug (e.g. None) or a wrong key would silently
    drop every preset saved before the most recent call."""
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    argstate.save_preset("myslug", "p1", {"A": "1"})
    argstate.save_preset("myslug", "p2", {"B": "2"})

    state = argstate.load_state("myslug")
    assert state["presets"] == {"p1": {"A": "1"}, "p2": {"B": "2"}}


def test_delete_preset_only_removes_named_preset_keeps_others(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    argstate.save_preset("myslug", "p1", {"A": "1"})
    argstate.save_preset("myslug", "p2", {"B": "2"})

    assert argstate.delete_preset("myslug", "p1") is True

    state = argstate.load_state("myslug")
    assert state["presets"] == {"p2": {"B": "2"}}


# ---------------------------------------------------------------------------
# store.py
# ---------------------------------------------------------------------------


@pytest.fixture
def sample_script(tmp_path: Path) -> Path:
    p = tmp_path / "hello.py"
    p.write_text(
        '"""打招呼腳本。\n\n多行 docstring。"""\nNAME = "world"\nprint(f"hi {NAME}")\n',
        encoding="utf-8",
    )
    return p


def test_hash_file_reflects_actual_file_content(tmp_path):
    f1 = tmp_path / "a.bin"
    f1.write_bytes(b"hello world")
    f2 = tmp_path / "b.bin"
    f2.write_bytes(b"a totally different payload")

    h1 = store._hash_file(f1)
    h2 = store._hash_file(f2)

    assert h1 == f"sha256:{hashlib.sha256(b'hello world').hexdigest()}"
    assert h1 != h2


def test_read_meta_and_write_meta_use_exact_lowercase_filename(tmp_path, monkeypatch):
    entry_dir = tmp_path / "slug"
    entry_dir.mkdir()

    write_paths: list[Path] = []
    from skit.atomic import atomic_write_toml as real_atomic_write_toml

    def spy_write(path, doc):
        write_paths.append(path)
        real_atomic_write_toml(path, doc)

    monkeypatch.setattr(store, "atomic_write_toml", spy_write)
    store._write_meta(entry_dir, ScriptMeta(name="x", kind="command"))
    assert write_paths[0].name == "meta.toml"

    read_paths: list[Path] = []
    real_open = open

    def spy_open(path, *a, **kw):
        read_paths.append(Path(path))
        return real_open(path, *a, **kw)

    monkeypatch.setattr(store, "open", spy_open, raising=False)
    store._read_meta(entry_dir)
    assert read_paths[0].name == "meta.toml"


def test_load_registry_missing_entries_key_returns_empty_dict(tmp_path):
    reg = registry_path()
    reg.parent.mkdir(parents=True, exist_ok=True)
    reg.write_text("", encoding="utf-8")  # valid empty TOML doc, no [entries] table

    assert store._load_registry() == {}


def test_unique_slug_numbers_from_2():
    assert store._unique_slug("foo", set()) == "foo"
    assert store._unique_slug("foo", {"foo"}) == "foo-2"
    assert store._unique_slug("foo", {"foo", "foo-2"}) == "foo-3"


def test_extract_description_with_no_docstring_is_empty():
    assert store._extract_description("print(1)\n") == ""


def test_add_python_missing_file_exact_message(tmp_path):
    ghost = tmp_path / "ghost.py"
    with pytest.raises(store.StoreError) as exc:
        store.add_python(ghost)
    assert str(exc.value) == f"File not found: {ghost.resolve()}"


def test_add_python_reads_source_as_utf8_replace(sample_script, monkeypatch):
    calls: list[tuple[Path, dict[str, object]]] = []
    real_read_text = Path.read_text

    def spy(self, *a, **kw):
        calls.append((self, kw))
        return real_read_text(self, *a, **kw)

    monkeypatch.setattr(Path, "read_text", spy)
    store.add_python(sample_script)

    matched = [kw for p, kw in calls if p == sample_script.resolve()]
    assert matched, "expected a read_text call on the source file"
    assert matched[0].get("encoding") == "utf-8"
    assert matched[0].get("errors") == "replace"


def test_add_python_injects_pep723_with_only_dependencies(tmp_path):
    script = tmp_path / "nodoc.py"
    script.write_text("print(1)\n", encoding="utf-8")

    entry = store.add_python(script, dependencies=["httpx"])
    text = entry.script_path.read_text(encoding="utf-8")
    assert "httpx" in text


def test_add_python_injects_pep723_with_only_requires_python(tmp_path):
    script = tmp_path / "nodoc2.py"
    script.write_text("print(1)\n", encoding="utf-8")

    entry = store.add_python(script, requires_python=">=3.11")
    text = entry.script_path.read_text(encoding="utf-8")
    assert ">=3.11" in text


def test_add_python_records_added_at_timestamp(sample_script):
    entry = store.add_python(sample_script)
    assert entry.meta.added_at != ""
    datetime.fromisoformat(entry.meta.added_at)  # must parse


def test_add_python_copy_mode_respects_custom_workdir(sample_script):
    entry = store.add_python(sample_script, workdir="store")
    assert entry.meta.workdir == "store"


def test_add_python_reference_mode_forces_origin_workdir(sample_script):
    entry = store.add_python(sample_script, mode="reference", workdir="store")
    assert entry.meta.workdir == "origin"


def test_add_python_writes_injected_script_as_utf8_with_lowercase_name(tmp_path, monkeypatch):
    script = tmp_path / "nodoc3.py"
    script.write_text("print(1)\n", encoding="utf-8")

    calls: list[tuple[Path, dict[str, object]]] = []
    real_write_text = Path.write_text

    def spy(self, *a, **kw):
        calls.append((self, kw))
        return real_write_text(self, *a, **kw)

    monkeypatch.setattr(Path, "write_text", spy)
    store.add_python(script, dependencies=["httpx"])

    matched = [(p, kw) for p, kw in calls if p.name.lower() == "script.py"]
    assert matched, "expected a write_text call on script.py"
    assert matched[0][0].name == "script.py"  # exact case, not SCRIPT.PY
    assert matched[0][1].get("encoding") == "utf-8"


def test_add_exe_roundtrip_full_fields(tmp_path):
    exe = tmp_path / "mytool"
    exe.write_bytes(b"#!/bin/sh\necho hi\n")
    entry = store.add_exe(exe)
    assert entry.meta.description == ""  # default
    assert entry.meta.workdir == "origin"
    assert entry.meta.added_at != ""
    assert entry.meta.source_hash == f"sha256:{hashlib.sha256(exe.read_bytes()).hexdigest()}"


def test_add_exe_directory_source_has_empty_hash(tmp_path):
    a_dir = tmp_path / "adir"
    a_dir.mkdir()
    entry = store.add_exe(a_dir)
    assert entry.meta.source_hash == ""


def test_add_exe_missing_file_exact_message(tmp_path):
    ghost = tmp_path / "ghost_tool"
    with pytest.raises(store.StoreError) as exc:
        store.add_exe(ghost)
    assert str(exc.value) == f"File not found: {ghost.resolve()}"


def test_extract_placeholders_supports_uppercase_names():
    assert store.extract_placeholders("hi {CITY} and {Name}") == ["CITY", "Name"]


def test_add_command_defaults_and_fields():
    entry = store.add_command("echo {msg}", name="hello", description="says hi")
    assert entry.meta.mode == "reference"
    assert entry.meta.added_at != ""
    assert entry.meta.description == "says hi"

    entry2 = store.add_command("echo hi", name="no-desc")
    assert entry2.meta.description == ""


def test_add_command_empty_template_exact_message():
    with pytest.raises(store.StoreError) as exc:
        store.add_command("   ", name="x")
    assert str(exc.value) == "Command template must not be empty"


def test_add_entry_name_conflict_exact_message(sample_script):
    store.add_python(sample_script, name="dup")
    other = sample_script.parent / "other.py"
    other.write_text("print(1)\n", encoding="utf-8")
    with pytest.raises(store.NameConflictError) as exc:
        store.add_python(other, name="dup")
    assert str(exc.value) == "The name dup is already taken — pick another name."


def test_add_entry_reuses_preexisting_empty_slug_dir():
    pre = scripts_dir() / "myname"
    pre.mkdir(parents=True)
    entry = store.add_command("echo hi", name="myname")
    assert entry.slug == "myname"


def test_add_entry_copies_payload_to_exact_lowercase_script_py(sample_script, monkeypatch):
    calls: list[Path] = []
    real_copy2 = shutil.copy2

    def spy(src, dst, *a, **kw):
        calls.append(Path(dst))
        return real_copy2(src, dst, *a, **kw)

    monkeypatch.setattr(shutil, "copy2", spy)
    store.add_python(sample_script)
    assert calls
    assert calls[0].name == "script.py"


def test_add_entry_cleanup_on_failure_ignores_rmtree_errors(sample_script, monkeypatch):
    calls: list[object] = []
    real_rmtree = shutil.rmtree

    def spy(path, *a, **kw):
        calls.append(kw.get("ignore_errors"))
        return real_rmtree(path, *a, **kw)

    monkeypatch.setattr(shutil, "rmtree", spy)
    monkeypatch.setattr(
        store,
        "_write_meta",
        lambda *a, **k: (_ for _ in ()).throw(RuntimeError("boom")),
    )
    with pytest.raises(RuntimeError, match="boom"):
        store.add_python(sample_script)
    assert calls == [True]


def test_add_entry_registry_index_has_correct_keys(sample_script):
    entry = store.add_python(sample_script, name="hi", description="desc here")
    reg = store._load_registry()
    assert reg[entry.slug] == {"name": "hi", "kind": "python", "description": "desc here"}


def test_resolve_not_found_exact_message(tmp_path):
    with pytest.raises(store.NotFoundError) as exc:
        store.resolve("nonexistent")
    assert str(exc.value) == "Script not found: nonexistent"


def test_remove_passes_ignore_errors_true_to_rmtree(sample_script, monkeypatch):
    """remove() must call shutil.rmtree with ignore_errors=True (a best-effort delete): the
    entry is already gone from the registry/meta by the time rmtree runs. But a best-effort
    delete that leaves the directory behind (a Windows held-open file, simulated here with a
    no-op rmtree) must NOT be reported as success — remove() re-checks entry.dir.exists() and
    raises StoreError naming the leftover directory, so a later `doctor --rebuild` can't
    silently resurrect the 'removed' entry from its surviving meta.toml."""
    entry = store.add_python(sample_script, name="hi")
    captured: dict[str, object] = {}

    def fake_rmtree(path, ignore_errors=False, **_kw):
        captured["path"] = Path(path)
        captured["ignore_errors"] = ignore_errors
        # A no-op even under ignore_errors=True: the directory survives, which is exactly
        # the held-open-file case the leftover raise exists to catch.

    monkeypatch.setattr(shutil, "rmtree", fake_rmtree)
    with pytest.raises(store.StoreError) as exc:
        store.remove("hi")
    assert captured["ignore_errors"] is True  # best-effort delete, not a hard rmtree
    assert captured["path"] == entry.dir
    assert str(entry.dir) in str(exc.value)
    # The leftover-dir message is a 3-segment implicit string concat. mutmut mutates each segment
    # independently (an "XX"-wrap and an uppercase variant per literal), and several survive a mere
    # `path in message` check: an XX-wrap of segment 3 keeps every interior substring intact, so
    # even asserting `"doctor --rebuild" in message` misses it. Pin the WHOLE rendered message so a
    # break anywhere in any segment — wrap or uppercase — fails the equality.
    expected = (
        "hi was removed from the library, but its files couldn't be fully "
        f"deleted: {entry.dir} — close any program using them, then delete the folder "
        "(or run `skit doctor --rebuild` to restore the entry and retry)."
    )
    assert str(exc.value) == expected


def test_doctor_rebuild_continues_past_missing_meta_dirs(tmp_path):
    d1 = scripts_dir() / "a_missing"
    d1.mkdir(parents=True)
    d2 = scripts_dir() / "z_good"
    d2.mkdir(parents=True)
    store._write_meta(d2, ScriptMeta(name="good", kind="command"))

    count, problems = store.doctor_rebuild()
    assert count == 1
    assert any("a_missing" in p for p in problems)
    names = {e.meta.name for e in store.list_entries()}
    assert names == {"good"}


def test_doctor_rebuild_no_problem_for_existing_reference(sample_script):
    store.add_python(sample_script, mode="reference", name="ref")
    _, problems = store.doctor_rebuild()
    assert problems == []


def test_doctor_rebuild_missing_meta_and_missing_reference_exact_messages(sample_script, tmp_path):
    orphan = scripts_dir() / "orphan"
    orphan.mkdir(parents=True)

    store.add_python(sample_script, mode="reference", name="ref")
    source_path = sample_script.resolve()
    sample_script.unlink()

    _, problems = store.doctor_rebuild()
    assert "orphan: meta.toml is missing; skipped" in problems
    assert f"ref: the referenced source file is gone: {source_path}" in problems


def test_update_dependencies_clearing_requires_python_to_empty_string(sample_script):
    entry = store.add_python(sample_script, requires_python=">=3.10")
    updated = store.update_dependencies(entry.slug, ["httpx"], requires_python="")
    assert updated.meta.requires_python == ""


def test_update_dependencies_syncs_pep723_block_reads_and_writes_utf8(sample_script):
    """Reads/writes the copy's script.py as utf-8/replace; also exercises the
    kind==python and mode==copy gate (only copy-mode python entries get synced)."""
    entry = store.add_python(sample_script)
    updated = store.update_dependencies(entry.slug, ["httpx"], ">=3.11")
    script_text = updated.script_path.read_text(encoding="utf-8")
    assert "httpx" in script_text
    assert ">=3.11" in script_text


def test_update_dependencies_reference_mode_never_touches_disk(sample_script):
    entry = store.add_python(sample_script, mode="reference", name="ref2")
    before = sample_script.read_bytes()
    store.update_dependencies(entry.slug, ["httpx"], ">=3.11")
    assert sample_script.read_bytes() == before
    assert not (entry.dir / "script.py").exists()


def test_update_dependencies_reads_and_writes_script_py_as_utf8(sample_script, monkeypatch):
    """The copy-mode PEP 723 sync reads the stored copy as BYTES (strict utf-8 decode, so a
    non-utf-8 byte is preserved instead of replaced) and writes it back through
    atomic_write_text_keep_mode (atomic + permission-preserving) — never a plain
    read_text/write_text. Its mutation-killing purpose is retained: the stored copy still
    round-trips as UTF-8 with the edit applied."""
    entry = store.add_python(sample_script)

    read_bytes_paths: list[Path] = []
    real_read_bytes = Path.read_bytes

    def spy_read_bytes(self):
        read_bytes_paths.append(self)
        return real_read_bytes(self)

    keep_mode_writes: list[tuple[Path, str]] = []
    real_keep_mode = store.atomic_write_text_keep_mode

    def spy_keep_mode(path, text):
        keep_mode_writes.append((path, text))
        return real_keep_mode(path, text)

    monkeypatch.setattr(Path, "read_bytes", spy_read_bytes)
    monkeypatch.setattr(store, "atomic_write_text_keep_mode", spy_keep_mode)
    store.update_dependencies(entry.slug, ["httpx"], ">=3.11")

    assert any(p.name == "script.py" for p in read_bytes_paths), (
        "the sync must read the copy as bytes (read_bytes), not read_text"
    )
    script_writes = [(p, t) for p, t in keep_mode_writes if p.name == "script.py"]
    assert script_writes, "the copy must be written via atomic_write_text_keep_mode"
    _, written_text = script_writes[0]
    assert "httpx" in written_text
    assert ">=3.11" in written_text
    # The stored copy round-trips as UTF-8 with the edit applied (no lossy re-encode).
    round_tripped = entry.script_path.read_text(encoding="utf-8")
    assert "httpx" in round_tripped
    assert ">=3.11" in round_tripped


def test_update_dependencies_returns_entry_with_correct_slug(sample_script):
    entry = store.add_python(sample_script)
    updated = store.update_dependencies(entry.slug, ["httpx"])
    assert updated.slug == entry.slug


def test_update_dependencies_without_requires_python_writes_none_into_block(tmp_path):
    """When the entry has no requires_python recorded and none is passed, the synced
    PEP 723 block must not contain a requires-python line at all (a bogus fallback
    string would poison every future `uv run` of the copy)."""
    script = tmp_path / "plain.py"
    script.write_text("print(1)\n", encoding="utf-8")
    entry = store.add_python(script)

    updated = store.update_dependencies(entry.slug, ["httpx"])
    text = updated.script_path.read_text(encoding="utf-8")
    assert "httpx" in text
    assert "requires-python" not in text


def test_update_dependencies_preserves_block_requires_python_when_meta_has_none(tmp_path):
    """A copy-mode python entry whose PEP 723 block carries requires-python but whose meta
    does NOT keeps the block's own constraint across a deps edit — the block is the source
    of truth, so a deps edit must PRESERVE it, not erase it by passing ""."""
    script = tmp_path / "pinned.py"
    script.write_text(
        '# /// script\n# requires-python = ">=3.12"\n# dependencies = []\n# ///\nprint(1)\n',
        encoding="utf-8",
    )
    entry = store.add_python(script)  # no requires_python passed → meta carries none
    assert not entry.meta.requires_python  # meta has no constraint…
    updated = store.update_dependencies(entry.slug, ["httpx"])
    text = updated.script_path.read_text(encoding="utf-8")
    assert "httpx" in text
    assert 'requires-python = ">=3.12"' in text  # …but the block's own constraint survives


def test_doctor_rebuild_corrupt_meta_exact_message(tmp_path):
    import tomllib

    corrupt_dir = scripts_dir() / "corrupt"
    corrupt_dir.mkdir(parents=True)
    bad_toml = "[[[bad"
    (corrupt_dir / "meta.toml").write_text(bad_toml, encoding="utf-8")

    # Reproduce the exact parser error text the user should see in the problem line.
    with pytest.raises(tomllib.TOMLDecodeError) as exc:
        tomllib.loads(bad_toml)
    expected_error = str(exc.value)

    count, problems = store.doctor_rebuild()
    assert count == 0
    assert problems == [f"corrupt: meta.toml is corrupt ({expected_error}); skipped"]


def test_doctor_rebuild_registry_index_has_correct_keys(sample_script):
    """The rebuilt registry rows must use the same schema as _add_entry writes
    ("name"/"kind"/"description"), otherwise resolve() and the index consumers break."""
    entry = store.add_python(sample_script, name="hi", description="desc here")
    os.unlink(registry_path())

    store.doctor_rebuild()
    reg = store._load_registry()
    assert reg[entry.slug] == {"name": "hi", "kind": "python", "description": "desc here"}
    assert store.resolve("hi").slug == entry.slug


# ===========================================================================
# Second wave — additional surviving-mutant kills for store.py:
# _fs_truth scan logic, _add_entry's reuse guard, _load_registry's suppressed
# cleanup, add_python's strict-UTF-8 gate, add_script messages/encoding,
# remove/rename/resolve messages, update_dependencies' npm gating, and the
# Entry-return fields of the meta-updating helpers.
# ===========================================================================


def _force_sorted_root_iterdir(monkeypatch: pytest.MonkeyPatch) -> None:
    """Make scripts_dir().iterdir() deterministic (name-sorted).

    _fs_truth loops over scripts_dir().iterdir(), whose order is filesystem-defined.
    Several mutants turn a `continue` (skip this dir, keep scanning) into a `break`
    (abandon the whole scan). That divergence is only observable when a dir that must
    still be visited is iterated AFTER the one that triggers the branch, so the tests
    below pin the order: the branch-triggering dir is named to sort first, and the dir
    whose name must still be collected sorts last."""
    real_iterdir = Path.iterdir
    root = scripts_dir()

    def ordered(self: Path):
        if self == root:
            return iter(sorted(real_iterdir(self), key=lambda p: p.name))
        return real_iterdir(self)

    monkeypatch.setattr(Path, "iterdir", ordered)


def _orphan_with_name(slug: str, name: str) -> None:
    """A non-empty, UNREGISTERED entry directory under scripts_dir() whose meta names `name`."""
    d = scripts_dir() / slug
    d.mkdir(parents=True)
    store._write_meta(d, ScriptMeta(name=name, kind="python"))


def test_fs_truth_registered_entry_name_comes_from_registry_not_meta():
    """A registered entry's name is accounted for from the registry index, WITHOUT re-reading
    its meta.toml (the `if in_registry: continue` fast path). If that flag collapsed to a falsy
    constant, _fs_truth would re-read every registered meta — so a name that drifted only in
    meta.toml would wrongly count as taken and block an unrelated add."""
    entry = store.add_command("echo hi", name="alpha")
    meta = store._read_meta(entry.dir)
    meta.name = "beta"
    store._write_meta(entry.dir, meta)  # meta now says "beta"; the registry row still says "alpha"

    new = store.add_command("echo bye", name="beta")  # must NOT conflict with the drifted meta
    assert new.meta.name == "beta"


def test_fs_truth_keeps_scanning_past_an_empty_leftover_dir(monkeypatch):
    """An empty, unregistered leftover directory is skipped (continue), never a reason to abandon
    the whole scan (break): a real orphan iterated after it must still have its name protected."""
    (scripts_dir() / "aaa-empty").mkdir(parents=True)  # empty leftover, sorts first
    _orphan_with_name("zzz-orphan", "ghostname")  # real orphan, sorts last
    _force_sorted_root_iterdir(monkeypatch)

    with pytest.raises(store.NameConflictError):
        store.add_command("echo x", name="ghostname")


def test_fs_truth_keeps_scanning_past_a_stray_non_directory(monkeypatch):
    """A stray non-directory entry is skipped (continue), not a reason to abandon the scan."""
    scripts_dir().mkdir(parents=True, exist_ok=True)
    (scripts_dir() / "aaa-stray").write_text("not a dir", encoding="utf-8")  # sorts first
    _orphan_with_name("zzz-orphan", "ghostname")
    _force_sorted_root_iterdir(monkeypatch)

    with pytest.raises(store.NameConflictError):
        store.add_command("echo x", name="ghostname")


def test_fs_truth_keeps_scanning_past_a_registered_dir(monkeypatch):
    """After a registered dir's name is accounted for, the scan continues to later dirs — a
    `break` there would leave an unregistered orphan's name unprotected."""
    store.add_command("echo hi", name="aaa")  # registered, slug "aaa", sorts first
    _orphan_with_name("zzz-orphan", "ghostname")
    _force_sorted_root_iterdir(monkeypatch)

    with pytest.raises(store.NameConflictError):
        store.add_command("echo x", name="ghostname")


def test_fs_truth_keeps_scanning_past_a_corrupt_orphan_meta(monkeypatch):
    """An orphan whose meta.toml is unreadable is skipped (continue), and the scan continues:
    a later orphan's name must still be collected."""
    corrupt = scripts_dir() / "aaa-corrupt"
    corrupt.mkdir(parents=True)
    (corrupt / "meta.toml").write_text("[[[bad", encoding="utf-8")  # unreadable, sorts first
    _orphan_with_name("zzz-orphan", "ghostname")
    _force_sorted_root_iterdir(monkeypatch)

    with pytest.raises(store.NameConflictError):
        store.add_command("echo x", name="ghostname")


def test_add_entry_refuse_reuse_nonempty_dir_exact_message(monkeypatch, sample_script, tmp_path):
    """The defense-in-depth guard against overwriting an existing non-empty entry dir must raise
    with the exact path it refused — verifying both the message text and that the real entry_dir
    (not a str(None)) is interpolated in."""
    entry = store.add_python(sample_script, name="first")
    monkeypatch.setattr(store, "_unique_slug", lambda base, existing: entry.slug)

    other = tmp_path / "other.py"
    other.write_text("print(1)\n", encoding="utf-8")
    with pytest.raises(store.StoreError) as exc:
        store.add_python(other, name="second")
    assert str(exc.value) == (
        f"Refusing to reuse the existing, non-empty entry directory: {entry.dir}"
    )


def test_load_registry_corrupt_suppresses_replace_failure(monkeypatch):
    """A corrupt registry.toml degrades to an empty registry even if the best-effort rename of
    the bad file fails (e.g. a permission error): the OSError from os.replace is suppressed, so
    _load_registry still returns {} rather than propagating. contextlib.suppress(None) would
    instead let the error escape (a TypeError from its __exit__)."""
    reg = registry_path()
    reg.parent.mkdir(parents=True, exist_ok=True)
    reg.write_text("not valid toml [[[", encoding="utf-8")

    def boom(_src, _dst):
        raise OSError("cannot rename")

    monkeypatch.setattr(os, "replace", boom)
    assert store._load_registry() == {}


def test_add_python_strict_decode_requests_utf8(tmp_path, monkeypatch):
    """The copy-injection safety gate re-decodes the source STRICTLY as UTF-8 (a lossy
    errors='replace' round-trip back to disk would corrupt a non-UTF-8 byte). Pin the exact
    codec name that gate asks for."""
    src = tmp_path / "s.py"
    src.write_text("print(1)\n", encoding="utf-8")

    seen: list[tuple[tuple[object, ...], dict[str, object]]] = []

    class _RecBytes(bytes):
        @override
        def decode(self, *a, **kw):
            seen.append((a, kw))
            return bytes.decode(self, *a, **kw)

    real_read_bytes = Path.read_bytes
    monkeypatch.setattr(Path, "read_bytes", lambda self: _RecBytes(real_read_bytes(self)))
    store.add_python(src, dependencies=["httpx"])

    # Exactly one decode, positional "utf-8", NO errors= handler: a lossy errors="replace"
    # (or any other handler) must fail this test, since that is the corruption the gate prevents.
    assert seen == [(("utf-8",), {})]


def test_add_script_unknown_kind_exact_message(tmp_path):
    src = tmp_path / "x.sh"
    src.write_text("echo hi\n", encoding="utf-8")
    with pytest.raises(store.StoreError) as exc:
        store.add_script(src, kind="martian")
    assert str(exc.value) == "Unknown entry kind: martian"


def test_add_script_missing_file_exact_message(tmp_path):
    ghost = tmp_path / "ghost.sh"
    with pytest.raises(store.StoreError) as exc:
        store.add_script(ghost, kind="shell")
    assert str(exc.value) == f"File not found: {ghost.resolve()}"


def test_add_script_reads_source_as_utf8_replace(tmp_path, monkeypatch):
    """add_script must read its source with encoding="utf-8", errors="replace" — same lenient
    decode add_python uses. Pins the exact kwargs (dropping either, nulling either, or passing a
    differently-cased/typo'd literal are all distinct surviving mutants)."""
    src = tmp_path / "deploy.sh"
    src.write_text("#!/bin/bash\n# Deploy\necho hi\n", encoding="utf-8")

    calls: list[tuple[Path, dict[str, object]]] = []
    real_read_text = Path.read_text

    def spy(self, *a, **kw):
        calls.append((self, kw))
        return real_read_text(self, *a, **kw)

    monkeypatch.setattr(Path, "read_text", spy)
    store.add_script(src, kind="shell")

    matched = [kw for p, kw in calls if p == src.resolve()]
    assert matched, "expected a read_text call on the source file"
    assert matched[0].get("encoding") == "utf-8"
    assert matched[0].get("errors") == "replace"


def test_add_script_records_added_at_timestamp(tmp_path):
    src = tmp_path / "deploy.sh"
    src.write_text("#!/bin/bash\necho hi\n", encoding="utf-8")
    entry = store.add_script(src, kind="shell")
    assert entry.meta.added_at != ""
    datetime.fromisoformat(entry.meta.added_at)  # must parse


def test_remove_forgets_the_removed_slugs_saved_values(sample_script):
    """remove() drops the entry's remembered parameter values too, keyed by the entry's OWN slug
    (argstate.forget(entry.slug)). A wrong key (None) would leave the values file orphaned."""
    entry = store.add_python(sample_script, name="hi")
    argstate.save_last(entry.slug, values={"A": "1"})
    values_file = values_dir() / f"{entry.slug}.toml"
    assert values_file.exists()

    store.remove("hi")
    assert not values_file.exists()


def test_rename_to_taken_name_exact_message():
    store.add_command("echo a", name="alpha")
    e2 = store.add_command("echo b", name="beta")
    with pytest.raises(store.StoreError) as exc:
        store.rename(e2.slug, "alpha")
    assert str(exc.value) == "The name alpha is already taken."


def test_rename_empty_name_exact_message():
    entry = store.add_command("echo a", name="alpha")
    with pytest.raises(store.StoreError) as exc:
        store.rename(entry.slug, "   ")
    assert str(exc.value) == "A name is required."


def test_rename_to_another_entrys_slug_string_is_taken():
    """The uniqueness predicate restates resolve()'s matching against the LOCKED registry
    snapshot: another entry's SLUG key counts as taken, not only its display name. Renaming B
    to A's slug string is refused with the exact message."""
    a = store.add_command("echo a", name="Alpha Name")  # slug "alpha-name" != display name
    b = store.add_command("echo b", name="beta")
    assert a.slug != a.meta.name
    with pytest.raises(store.StoreError) as exc:
        store.rename(b.slug, a.slug)
    assert str(exc.value) == f"The name {a.slug} is already taken."


def test_rename_to_its_own_slug_string_is_allowed():
    """Renaming an entry to a string equal to its OWN slug is allowed — the predicate excludes
    the entry's own slug key (`new_name != entry.slug`), even though that slug is a registry key."""
    entry = store.add_command("echo hi", name="Some Name")  # slug "some-name" != display name
    assert entry.slug != entry.meta.name
    renamed = store.rename(entry.slug, entry.slug)
    assert renamed.meta.name == entry.slug
    assert renamed.slug == entry.slug  # slug is immutable
    assert store.resolve(entry.slug).meta.name == entry.slug


def test_rename_updates_meta_and_registry_and_preserves_slug_dir_argstate(sample_script):
    """A normal rename writes the new name to meta.toml AND the registry row, while the slug,
    the entry directory, and the argstate values file (both keyed by the immutable slug) all
    survive untouched."""
    entry = store.add_python(sample_script, name="before")
    argstate.save_preset(entry.slug, "p1", {"A": "1"})
    slug, entry_dir = entry.slug, entry.dir

    renamed = store.rename(slug, "after")

    assert renamed.slug == slug  # slug immutable
    assert renamed.dir == entry_dir
    assert renamed.meta.name == "after"
    assert store._read_meta(entry_dir).name == "after"  # meta.toml on disk carries the new name
    assert store._load_registry()[slug]["name"] == "after"  # registry row updated
    assert argstate.load_state(slug)["presets"] == {"p1": {"A": "1"}}  # argstate preserved


def test_rename_race_exactly_one_of_two_concurrent_claims_wins():
    """The uniqueness check sits INSIDE the registry lock: two entries renaming to the SAME new
    name concurrently must not both pass (each holds only its own entry lock). The registry lock
    serializes the check+write, so exactly one rename lands and the other is refused."""
    a = store.add_command("echo a", name="aaa")
    b = store.add_command("echo b", name="bbb")

    barrier = threading.Barrier(2)
    results: dict[str, str] = {}

    def do(slug: str, key: str) -> None:
        barrier.wait()
        try:
            store.rename(slug, "shared-name")
            results[key] = "ok"
        except store.StoreError as exc:
            results[key] = str(exc)

    threads = [
        threading.Thread(target=do, args=(a.slug, "a")),
        threading.Thread(target=do, args=(b.slug, "b")),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    outcomes = list(results.values())
    assert outcomes.count("ok") == 1  # exactly one succeeded
    assert any("already taken" in v for v in outcomes if v != "ok")  # the other was refused
    names = [e.meta.name for e in store.list_entries()]
    assert names.count("shared-name") == 1  # only one entry ended up with the shared name


def test_remove_leftover_dir_raises_but_drops_registry_row_and_keeps_values(
    sample_script, monkeypatch
):
    """When the best-effort rmtree leaves the directory behind, remove() raises — but the
    registry row was already popped (durable removal happens BEFORE rmtree), and the values
    file is deliberately KEPT so a `doctor --rebuild`-restored entry keeps its presets."""
    entry = store.add_python(sample_script, name="hi")
    argstate.save_preset(entry.slug, "p1", {"A": "1"})
    values_file = values_dir() / f"{entry.slug}.toml"
    assert values_file.exists()

    monkeypatch.setattr(shutil, "rmtree", lambda *a, **k: None)  # no-op → directory survives
    with pytest.raises(store.StoreError):
        store.remove("hi")

    assert entry.slug not in store._load_registry()  # registry row gone (pop happened first)
    assert values_file.exists()  # values kept for a doctor-restored entry


def test_remove_real_delete_removes_dir_and_values_and_returns_name(sample_script):
    """The success path (no monkeypatch): the entry directory is gone, the argstate values file
    is forgotten, and the entry's display name is returned."""
    entry = store.add_python(sample_script, name="hi")
    argstate.save_last(entry.slug, values={"A": "1"})
    values_file = values_dir() / f"{entry.slug}.toml"
    assert values_file.exists()

    name = store.remove("hi")
    assert name == "hi"
    assert not entry.dir.exists()
    assert not values_file.exists()


def test_resolve_corrupt_meta_exact_message():
    import tomllib

    entry = store.add_command("echo hi", name="x")
    bad = "[[[bad"
    (entry.dir / "meta.toml").write_text(bad, encoding="utf-8")

    with pytest.raises(tomllib.TOMLDecodeError) as parse_exc:
        tomllib.loads(bad)
    expected_error = str(parse_exc.value)

    with pytest.raises(store.NotFoundError) as exc:
        store.resolve(entry.slug)
    assert str(exc.value) == (
        f"{entry.slug}: metadata is corrupt ({expected_error}); run skit doctor --rebuild"
    )


def test_update_dependencies_python_constraint_on_npm_kind_exact_message(tmp_path):
    src = tmp_path / "s.js"
    src.write_text("console.log(1)\n", encoding="utf-8")
    entry = store.add_script(src, kind="js")
    with pytest.raises(store.StoreUsageError) as exc:
        store.update_dependencies(entry.slug, [], requires_python=">=3.11")
    assert str(exc.value) == "A Python constraint doesn't apply to js scripts."


def test_update_dependencies_reference_npm_exact_message(tmp_path):
    src = tmp_path / "s.js"
    src.write_text("console.log(1)\n", encoding="utf-8")
    entry = store.add_script(src, kind="js", mode="reference", name="refjs")
    with pytest.raises(store.StoreUsageError) as exc:
        store.update_dependencies(entry.slug, ["chalk"])
    assert str(exc.value) == (
        "refjs is a reference-mode entry: it runs from its own project, which already "
        "provides its packages. Dependency management applies to copies."
    )


def test_update_dependencies_python_empty_deps_does_not_sweep_node_modules(sample_script):
    """The npm node_modules sweep runs ONLY for npm-flavor entries with an empty dep list. A
    python entry (uv flavor) must never have its dir swept, even when its deps are cleared."""
    entry = store.add_python(sample_script)
    nm = entry.dir / "node_modules"
    nm.mkdir()
    (nm / "keep.txt").write_text("x", encoding="utf-8")

    store.update_dependencies(entry.slug, [])  # clear deps on a python entry
    assert nm.exists()


def test_update_dependencies_python_nonempty_deps_does_not_sweep_node_modules(sample_script):
    """Same npm-sweep gate, from the other side: a python entry with a non-empty dep list must
    also never be swept (the guard's first `and`, not the last, is what excludes it)."""
    entry = store.add_python(sample_script)
    nm = entry.dir / "node_modules"
    nm.mkdir()
    (nm / "keep.txt").write_text("x", encoding="utf-8")

    store.update_dependencies(entry.slug, ["httpx"])  # set deps on a python entry
    assert nm.exists()


def test_update_description_returns_entry_with_correct_dir(sample_script):
    entry = store.add_python(sample_script, name="hi")
    updated = store.update_description(entry.slug, "new desc")
    assert updated.dir == entry.dir
    assert updated.meta.description == "new desc"


def test_update_needs_returns_entry_with_correct_slug_and_dir(sample_script):
    entry = store.add_python(sample_script, name="hi")
    updated = store.update_needs(entry.slug, ["ffmpeg"])
    assert updated.slug == entry.slug
    assert updated.dir == entry.dir
    assert updated.meta.needs == ["ffmpeg"]


def test_write_parameters_returns_entry_with_correct_dir(sample_script):
    entry = store.add_python(sample_script, name="hi")
    decls = [ParamDecl(name="a", binding="const", type="str", default=None, secret=False)]
    updated = store.write_parameters(entry.slug, decls)
    assert updated.dir == entry.dir
    assert updated.slug == entry.slug
