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


# ---------- extract_comment_description (the comment-language docstring analogue) ----------


def test_extract_comment_description_first_comment_line_wins():
    from skit import store

    text = "#!/bin/bash\n# Ship the current build\n# more\necho hi\n"
    assert store.extract_comment_description(text, "#") == "Ship the current build"


def test_extract_comment_description_skips_shebang_and_blank_lines():
    from skit import store

    text = "#!/bin/sh\n\n# real desc\necho hi\n"  # shebang, then a blank, then the comment
    assert store.extract_comment_description(text, "#") == "real desc"


def test_extract_comment_description_skips_metadata_fence():
    from skit import store

    text = "#!/bin/bash\n# /// script\n# actual desc\ncode\n"  # the /// fence is machinery
    assert store.extract_comment_description(text, "#") == "actual desc"


def test_extract_comment_description_empty_comment_line_continues():
    from skit import store

    text = "#!/bin/sh\n#\n# after empty\necho\n"  # a bare `#` has no content; keep scanning
    assert store.extract_comment_description(text, "#") == "after empty"


def test_extract_comment_description_code_first_is_empty():
    from skit import store

    text = "NAME=1\n# a comment below code\n"  # first non-blank line is code -> no description
    assert store.extract_comment_description(text, "#") == ""


def test_extract_comment_description_only_shebang_is_empty():
    from skit import store

    assert store.extract_comment_description("#!/bin/sh\n\n", "#") == ""


def test_extract_comment_description_lua_double_dash_prefix():
    from skit import store

    assert store.extract_comment_description("-- Lua tool\nprint('x')\n", "--") == "Lua tool"


# ---------- add_script (the generic Tier-0 interpreted add) ----------


def _sh(tmp_path: Path, body: str = "#!/bin/bash\n# Deploy it\necho hi\n") -> Path:
    p = tmp_path / "deploy.sh"
    p.write_text(body, encoding="utf-8")
    return p


def test_add_script_copy_is_byte_identical_and_records_hash(tmp_path: Path):
    from skit import store

    src = _sh(tmp_path)
    entry = store.add_script(src, kind="shell")
    assert entry.meta.kind == "shell"
    assert entry.meta.mode == "copy"
    assert entry.script_path.name == "script.sh"  # copy under the kind's stored name
    assert entry.script_path.read_bytes() == src.read_bytes()  # verbatim
    assert entry.meta.source_hash.startswith("sha256:")
    assert entry.meta.workdir == "invoke"  # copy mode decouples from origin
    assert entry.meta.description == "Deploy it"  # comment-extracted


def test_add_script_reference_points_to_origin(tmp_path: Path):
    from skit import store

    src = _sh(tmp_path)
    entry = store.add_script(src, kind="shell", mode="reference")
    assert entry.meta.workdir == "origin"
    assert entry.script_path == src.resolve()
    assert not (entry.dir / "script.sh").exists()  # never copied


def test_add_script_explicit_workdir_override(tmp_path: Path):
    from skit import store

    entry = store.add_script(_sh(tmp_path), kind="shell", workdir="store")
    assert entry.meta.workdir == "store"


def test_add_script_explicit_name_and_description(tmp_path: Path):
    from skit import store

    entry = store.add_script(_sh(tmp_path), kind="shell", name="ship", description="custom")
    assert entry.meta.name == "ship"
    assert entry.meta.description == "custom"  # explicit wins over comment extraction


def test_add_script_records_interpreter(tmp_path: Path):
    from skit import store

    entry = store.add_script(_sh(tmp_path), kind="shell", interpreter="zsh")
    assert entry.meta.interpreter == "zsh"


def test_add_script_unknown_kind_raises(tmp_path: Path):
    from skit import store

    with pytest.raises(store.StoreError, match="Unknown entry kind"):
        store.add_script(_sh(tmp_path), kind="martian")


def test_add_script_non_interpreted_kind_raises(tmp_path: Path):
    from skit import store

    # exe is a real kind but not an interpreted, copyable one — add_script refuses it.
    with pytest.raises(store.StoreError, match="Unknown entry kind"):
        store.add_script(_sh(tmp_path), kind="exe")


def test_add_script_missing_file_raises(tmp_path: Path):
    from skit import store

    with pytest.raises(store.StoreError, match="not found"):
        store.add_script(tmp_path / "ghost.sh", kind="shell")


def test_add_script_lua_uses_double_dash_description(tmp_path: Path):
    from skit import store

    p = tmp_path / "tool.lua"
    p.write_text("-- Resize things\nprint('x')\n", encoding="utf-8")
    entry = store.add_script(p, kind="lua")
    assert entry.meta.kind == "lua"
    assert entry.meta.description == "Resize things"  # '--' comment prefix honoured
    assert entry.script_path.name == "script.lua"


# ---------------------------------------------------------------------------
# list_summaries — the listing view, served from the index
# ---------------------------------------------------------------------------


def test_summaries_match_full_entries_field_for_field(sample_script: Path, tmp_path: Path):
    """The index projection and the meta must agree on every field a listing renders.
    If they can drift, `skit list` shows something `skit show` contradicts."""
    from skit import store

    store.add_python(sample_script, name="copied", description="a copy")
    store.add_python(sample_script, name="linked", mode="reference")
    store.add_command("echo hi", name="templated", description="no file")
    exe = tmp_path / "tool"
    exe.touch(mode=0o755)
    store.add_exe(exe, name="binary")

    by_slug = {e.slug: e for e in store.list_entries()}
    summaries = store.list_summaries()
    assert [s.slug for s in summaries] == sorted(by_slug)
    for summary in summaries:
        entry = by_slug[summary.slug]
        meta = entry.meta
        assert (summary.name, summary.kind, summary.mode, summary.description) == (
            meta.name,
            meta.kind,
            meta.mode,
            meta.description,
        )
        # `target` is the LINKED original, not meta.source: a copied entry's script
        # lives in the store, and carrying its provenance path would double the index.
        assert summary.target == (meta.source if meta.mode == "reference" else "")
        assert summary.script_path == entry.script_path


def test_summaries_serve_from_the_index_without_reading_metas(sample_script: Path):
    """The whole point: one file read for the whole library. Proven by deleting every
    meta.toml — a listing that still answers cannot have opened one."""
    from skit import store

    store.add_python(sample_script, name="one", description="first")
    store.add_command("echo hi", name="two", description="second")
    expected = [(s.slug, s.name, s.description) for s in store.list_summaries()]
    for entry in store.list_entries():
        (entry.dir / "meta.toml").unlink()

    assert [(s.slug, s.name, s.description) for s in store.list_summaries()] == expected
    assert store.list_entries() == []  # the full read has nothing left to read


def test_a_row_an_older_skit_wrote_falls_back_to_its_meta(sample_script: Path):
    """Registries written before the index carried mode/source still list correctly —
    the row can't supply a summary, so that one entry is read from its meta."""
    from skit import store

    entry = store.add_python(sample_script, name="old", mode="reference")
    # An older document: no version key, and rows that cannot say what mode they are.
    from skit.atomic import atomic_write_toml
    from skit.paths import registry_path

    atomic_write_toml(
        registry_path(),
        {"entries": {entry.slug: {"name": "old", "kind": "python", "description": ""}}},
    )
    assert store._load_index()[0] == 1

    (summary,) = store.list_summaries()
    assert summary.mode == "reference"  # NOT the "copy" a v2 row would have implied
    assert summary.script_path == entry.script_path
    # ...and the document is rewritten, so the fallback is paid once.
    version, rows = store._load_index()
    assert version == store.INDEX_VERSION
    assert rows[entry.slug] == store._registry_row(entry.meta)


@pytest.mark.parametrize(
    "row",
    [
        {"name": "x", "kind": "python", "description": 7},
        {"name": "x", "kind": "python", "mode": "sideways", "description": ""},
        {"kind": "python", "description": ""},
        {"name": "x", "kind": "python", "description": "", "mode": "reference", "target": 7},
    ],
    ids=["non-string-field", "unknown-mode", "missing-field", "non-string-target"],
)
def test_a_hand_broken_row_falls_back_instead_of_inventing_a_summary(
    sample_script: Path, row: dict[str, object]
):
    """registry.toml is a plain file a user can edit. A row that a newer skit could not
    have written is never coerced into a summary — the meta answers instead."""
    from skit import store

    entry = store.add_python(sample_script, name="real", description="the truth")
    store._save_registry({entry.slug: row})

    (summary,) = store.list_summaries()
    assert summary.name == "real"
    assert summary.description == "the truth"


def test_a_broken_row_over_a_corrupt_meta_is_skipped_like_list_entries(sample_script: Path):
    """Fallback reaches a meta that is itself corrupt: skip the entry and leave it for
    doctor — the same answer list_entries gives, never a crash."""
    from skit import store

    entry = store.add_python(sample_script, name="doomed")
    store._save_registry({entry.slug: {"name": "doomed"}})
    (entry.dir / "meta.toml").write_text("not [ toml", encoding="utf-8")

    assert store.list_summaries() == []
    assert store.list_entries() == []


def test_rename_and_describe_keep_the_index_in_step(sample_script: Path):
    """The two fields that change after add. If a mutator forgets the row, `skit list`
    shows a stale name until someone runs doctor --rebuild."""
    from skit import store

    entry = store.add_python(sample_script, name="before", description="old text")
    store.rename(entry.slug, "after")
    store.update_description(entry.slug, "new text")

    (summary,) = store.list_summaries()
    assert (summary.name, summary.description) == ("after", "new text")


def test_an_older_registry_is_widened_the_first_time_it_is_listed(sample_script: Path):
    """Self-healing: a library added by an older skit would otherwise fall back to
    reading every meta.toml forever, because the index only gains the new fields when
    an entry is added, renamed or re-described. The first listing rewrites it, so
    nobody has to know to run `doctor --rebuild` after upgrading."""
    from skit import store

    entry = store.add_python(sample_script, name="legacy", description="old row")
    store._save_registry(
        {entry.slug: {"name": "legacy", "kind": "python", "description": "old row"}}
    )

    store.list_summaries()

    row = store._load_registry()[entry.slug]
    assert row == store._registry_row(entry.meta)
    assert store._summary_from_row(entry.slug, row, entry.dir) is not None


def test_widening_never_drops_an_entry_added_meanwhile(sample_script: Path, tmp_path: Path):
    """The rewrite re-reads under the registry lock and only fills in the slugs it has
    metas for, so an entry that landed between the read and the write survives."""
    from skit import store

    old = store.add_python(sample_script, name="legacy")
    store._save_registry({old.slug: {"name": "legacy", "kind": "python", "description": ""}})

    other = tmp_path / "other.py"
    other.write_text("print(2)\n", encoding="utf-8")
    raced = store.add_python(other, name="raced")
    # Roll the index back to the pre-add snapshot the listing read, then widen with it:
    # exactly the interleaving where a blind overwrite would erase `raced`.
    store._save_registry({old.slug: {"name": "legacy", "kind": "python", "description": ""}})
    stale = {old.slug: store._registry_row(old.meta)}

    def restore_the_race(_entries: object = None) -> dict[str, object]:
        return {
            old.slug: {"name": "legacy", "kind": "python", "description": ""},
            raced.slug: store._registry_row(raced.meta),
        }

    with pytest.MonkeyPatch.context() as mp:
        mp.setattr(store, "_load_registry", restore_the_race)
        store._upgrade_rows(stale)

    rows = store._load_registry()
    assert set(rows) == {old.slug, raced.slug}
    assert rows[old.slug] == store._registry_row(old.meta)


def test_widening_skips_an_entry_removed_meanwhile(sample_script: Path):
    """The other side of re-reading under the lock: a slug the listing saw but that a
    concurrent `skit remove` has since deleted must not be written back in — the
    widening fills rows in, it never resurrects them."""
    from skit import store

    entry = store.add_python(sample_script, name="legacy")
    stale = {entry.slug: store._registry_row(entry.meta), "vanished": {"name": "ghost"}}

    store._upgrade_rows(stale)

    assert set(store._load_registry()) == {entry.slug}


def test_a_store_that_cannot_be_written_still_lists(sample_script: Path, monkeypatch):
    """The widening is a side effect of a READ. A read-only store, or one another
    process is mid-write on, must still answer `skit list` — never fail on an index it
    does not depend on."""
    from skit import store

    entry = store.add_python(sample_script, name="legacy", description="old row")
    store._save_registry(
        {entry.slug: {"name": "legacy", "kind": "python", "description": "old row"}}
    )

    def refuse(_entries: object) -> None:
        raise OSError("read-only file system")

    monkeypatch.setattr(store, "_save_registry", refuse)
    (summary,) = store.list_summaries()
    assert summary.name == "legacy"
    assert summary.mode == entry.meta.mode


def test_a_corrupt_index_lists_nothing_and_preserves_the_bad_bytes(sample_script: Path):
    """registry.toml is a rebuildable index, so a listing degrades exactly as every
    other reader does: empty, with the unparseable bytes moved aside for inspection
    rather than discarded, and the metas untouched for `doctor --rebuild`."""
    from skit import store
    from skit.paths import registry_path

    store.add_python(sample_script, name="doomed")
    registry_path().write_text("entries = [ this is not toml", encoding="utf-8")

    assert store.list_summaries() == []
    assert registry_path().with_name("registry.toml.corrupt").exists()
    assert store.doctor_rebuild()[0] == 1  # the meta survived; the index comes back
    assert [s.name for s in store.list_summaries()] == ["doomed"]


def test_exe_is_always_reference_mode(tmp_path: Path):
    """`DirectLaunch.target` returns `script_path` rather than `Path(meta.source)`,
    which is the same answer ONLY because an exe entry is never copied. If that ever
    changes, a copy-mode exe would resolve its target to the entry directory (exe has
    no stored_name) and the missing-target mark would go quietly wrong."""
    from skit import store
    from skit.langs.registry import spec_for

    exe = tmp_path / "tool"
    exe.touch(mode=0o755)
    entry = store.add_exe(exe, name="binary")
    assert entry.meta.mode == "reference"
    spec = spec_for("exe")
    assert spec is not None
    assert spec.launch.target(entry) == Path(entry.meta.source) == entry.script_path
    # ...and asked of the narrow shape a listing holds, the same answer.
    (summary,) = [s for s in store.list_summaries() if s.slug == entry.slug]
    assert spec.launch.target(summary) == entry.script_path
