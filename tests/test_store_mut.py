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
from datetime import UTC, datetime
from pathlib import Path

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
    """remove() must call shutil.rmtree with ignore_errors=True (a best-effort delete):
    the entry is already gone from the registry/meta by the time rmtree runs, so a
    filesystem hiccup there must not turn a successful logical removal into a crash."""
    store.add_python(sample_script, name="hi")

    def fake_rmtree(_path, ignore_errors=False, **_kw):
        if not ignore_errors:
            raise FileNotFoundError("simulated: directory already gone")

    monkeypatch.setattr(shutil, "rmtree", fake_rmtree)
    name = store.remove("hi")
    assert name == "hi"


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
    entry = store.add_python(sample_script)

    read_calls: list[tuple[Path, dict[str, object]]] = []
    write_calls: list[tuple[Path, dict[str, object]]] = []
    real_read_text = Path.read_text
    real_write_text = Path.write_text

    def spy_read(self, *a, **kw):
        read_calls.append((self, kw))
        return real_read_text(self, *a, **kw)

    def spy_write(self, *a, **kw):
        write_calls.append((self, kw))
        return real_write_text(self, *a, **kw)

    monkeypatch.setattr(Path, "read_text", spy_read)
    monkeypatch.setattr(Path, "write_text", spy_write)
    store.update_dependencies(entry.slug, ["httpx"], ">=3.11")

    script_reads = [kw for p, kw in read_calls if p.name == "script.py"]
    script_writes = [kw for p, kw in write_calls if p.name == "script.py"]
    assert script_reads, "expected a read_text call on script.py"
    assert script_reads[0].get("encoding") == "utf-8"
    assert script_reads[0].get("errors") == "replace"
    assert script_writes, "expected a write_text call on script.py"
    assert script_writes[0].get("encoding") == "utf-8"


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
