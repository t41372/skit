"""Regression tests for bugs caught during the 2026-07-04 code review (one test per real bug)."""

from __future__ import annotations

import sys

import pytest

from skit import launcher, pep723, store, uvman
from skit.langs import launch
from skit.langs.python import metawriter, reconcile, shim
from skit.params import ParamDecl


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))


# ---------- launcher: {{name}} escapes must not be replaced by a same-named placeholder ----------


def test_escaped_placeholder_not_substituted():
    entry = store.add_command("echo {{name}} {name}", name="esc")
    cmd = launcher.build_command(entry, values={"name": "X"})
    assert cmd == "echo {name} X"


def test_escape_unescaped_even_without_params():
    entry = store.add_command("echo {{literal}}", name="noparams")
    assert entry.meta.params is None
    cmd = launcher.build_command(entry)
    assert cmd == "echo {literal}"


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell quoting")
def test_extra_args_quoted_for_posix_shell():
    entry = store.add_command("echo hi", name="quoting")
    cmd = launcher.build_command(entry, extra_args=["$HOME", "a b", "`whoami`"])
    # shlex quoting must preserve $, backticks, and spaces as literals
    assert "'$HOME'" in cmd
    assert "'a b'" in cmd
    assert "'`whoami`'" in cmd


# ---------- shim: non-finite floats must be explicitly rejected ----------
# (X = inf is not valid Python)


@pytest.mark.parametrize("bad", ["inf", "-inf", "nan", "Infinity"])
def test_inject_rejects_non_finite_float(bad):
    text = "RATE = 1.5\nprint(RATE)\n"
    specs = [ParamDecl(name="RATE", binding="const", type="float")]
    with pytest.raises(shim.ShimError):
        shim.inject(text, specs, {"RATE": bad})


def test_inject_accepts_normal_float():
    text = "RATE = 1.5\nprint(RATE)\n"
    specs = [ParamDecl(name="RATE", binding="const", type="float")]
    out = shim.inject(text, specs, {"RATE": "2.75"})
    assert "RATE = 2.75" in out


# ---------- shim.write_injected: unique filename + private permissions ----------


def test_write_injected_unique_and_private(tmp_path):
    a = shim.write_injected(tmp_path, "print(1)\n")
    b = shim.write_injected(tmp_path, "print(2)\n")
    assert a != b
    assert a.name.startswith(".injected-")
    assert a.suffix == ".py"
    assert a.read_text(encoding="utf-8") == "print(1)\n"
    if sys.platform != "win32":
        assert (a.stat().st_mode & 0o777) == 0o600


# ---------- metawriter: a prompt containing control characters must round-trip cleanly ----------


def test_write_params_prompt_with_newline_roundtrips():
    text = 'CITY = "Taipei"\nprint(CITY)\n'
    specs = [ParamDecl(name="CITY", binding="const", type="str", prompt="City:\nwith newline\t!")]
    out = metawriter.write_params(text, specs)
    back = metawriter.read_params(out)
    assert len(back) == 1
    assert back[0].prompt == "City:\nwith newline\t!"


# ---------- reconcile.edit_specs: pure function must not mutate the caller's specs ----------


def test_edit_specs_does_not_mutate_input():
    text = 'CITY = "Taipei"\n'
    original = [ParamDecl(name="CITY", binding="const", type="str", secret=False, prompt="")]
    reconcile.edit_specs(text, original, secret=["CITY"], prompts={"CITY": "changed"})
    assert original[0].secret is False
    assert original[0].prompt == ""


# ---------- pep723: multi-line dependency array with inline comment ----------
# must not leave orphan lines


def test_set_dependencies_multiline_array_with_comment():
    text = '# /// script\n# dependencies = [  # my deps\n#     "requests",\n# ]\n# ///\nprint(1)\n'
    out = pep723.set_dependencies(text, ["httpx"])
    meta = pep723.parse_block(out)
    assert meta is not None
    assert meta["dependencies"] == ["httpx"]


# ---------- i18n.is_supported: garbage tags must be rejected ----------


def test_is_supported_rejects_junk():
    from skit import i18n

    assert i18n.is_supported("zh-TW")
    assert i18n.is_supported("zh_TW.UTF-8")
    assert i18n.is_supported("en-US")
    assert i18n.is_supported("x-pseudo")
    assert not i18n.is_supported("ent")
    assert not i18n.is_supported("english")
    assert not i18n.is_supported("fr")
    assert not i18n.is_supported("")


# ---------- models.slugify: all-special input falls back to "script" ----------


def test_slugify_all_special_chars_fallback():
    from skit.models import slugify

    assert slugify("---") == "script"
    assert slugify("!!!") == "script"
    assert slugify("  ") == "script"


# ---------- metawriter.write_params: no block + no params -> text unchanged ----------


def test_write_params_no_block_no_params():
    """If the source has no PEP 723 block and there are no params to write, return unchanged."""
    text = "print(1)\n"
    result = metawriter.write_params(text, [])
    assert result == text


# ---------- pep723.parse_block: corrupt block body returns None ----------


def test_parse_block_corrupt_body_returns_none():
    bad = "# /// script\n# not: valid: toml: [\n# ///\nprint(1)\n"
    assert pep723.parse_block(bad) is None


# ---------- atomic: exception cleanup deletes the temp file ----------


def test_atomic_write_bytes_cleanup_on_error(tmp_path):
    """If the write raises mid-way, the temp file must be silently removed."""
    import os

    from skit.atomic import atomic_write_bytes

    target = tmp_path / "out.bin"

    def bad_fdopen(fd, *a, **kw):
        os.close(fd)
        raise OSError("disk full")

    import unittest.mock

    with unittest.mock.patch("skit.atomic.os.fdopen", side_effect=bad_fdopen):
        with pytest.raises(OSError, match="disk full"):
            atomic_write_bytes(target, b"data")
    # No temp file should remain
    temps = list(tmp_path.glob(".out.bin.*.tmp"))
    assert temps == []


# ---------- argstate: corrupt state file falls back to empty dict ----------


def test_argstate_corrupt_file_fallback(tmp_path, monkeypatch):
    from skit import argstate
    from skit.paths import values_dir

    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    values_dir().mkdir(parents=True, exist_ok=True)
    (values_dir() / "myscript.toml").write_text("[[[invalid", encoding="utf-8")
    # Must not raise; corrupt file falls back to an empty state
    state = argstate.load_state("myscript")
    assert state["values"] == {}


# ---------- i18n: available_locales when locales dir absent ----------


def test_available_locales_missing_dir(monkeypatch):
    import pathlib

    from skit import i18n

    monkeypatch.setattr(i18n, "_LOCALES_DIR", pathlib.Path("/nonexistent/__skit_locales__"))
    locales = i18n.available_locales()
    assert locales == [i18n.DEFAULT_LOCALE]


# ---------- i18n: 4-char subtag is capitalized ----------


def test_normalize_four_char_subtag():
    from skit import i18n

    # zh-Hant-TW: 'Hant' is a 4-char script subtag → title-cased
    result = i18n._normalize("zh-hant-tw")
    assert result == "zh-Hant-TW"


# ---------- i18n: detect_locale ValueError/TypeError from locale.getlocale ----------


def test_detect_locale_locale_module_error(monkeypatch):
    import locale as _locale

    from skit import i18n

    monkeypatch.setenv("SKIT_LANG", "")
    monkeypatch.setenv("LC_ALL", "")
    monkeypatch.setenv("LC_MESSAGES", "")
    monkeypatch.setenv("LANG", "")

    def _bad_getlocale():
        raise ValueError("no locale")

    monkeypatch.setattr(_locale, "getlocale", _bad_getlocale)
    # Must not raise; falls back to empty string
    assert i18n.detect_locale() == ""


# ---------- i18n: _config_language with corrupt config file ----------


def test_config_language_corrupt_file(tmp_path, monkeypatch):
    from skit import i18n
    from skit.paths import config_dir

    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "cfg"))
    config_dir().mkdir(parents=True, exist_ok=True)
    (config_dir() / "config.toml").write_text("[[[bad", encoding="utf-8")
    # Corrupt file must return empty string silently
    assert i18n._config_language() == ""


# ---------- i18n: set_language OSError on corrupt existing config ----------


def test_set_language_with_existing_corrupt_config(tmp_path, monkeypatch):
    from skit import i18n
    from skit.paths import config_dir

    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "cfg"))
    config_dir().mkdir(parents=True, exist_ok=True)
    (config_dir() / "config.toml").write_text("[[[bad", encoding="utf-8")
    # Should not crash; falls back to empty doc and writes the new language
    i18n.set_language("en-US")
    assert (config_dir() / "config.toml").exists()


# ---------- models: slugify leading/trailing dashes stripped ----------


def test_slugify_leading_trailing_special():
    from skit.models import slugify

    # A name starting with a non-alnum char: no leading dash in output
    assert slugify("-hello-") == "hello"
    # A name where non-alnum appears mid-word: dash injected once only
    assert slugify("hello  world") == "hello-world"


# ---------- shim: AnnAssign (annotated assignment) is a const target ----------


def test_inject_annotated_assignment():
    src = "CITY: str = 'Taipei'\nprint(CITY)\n"
    from skit.langs.python import shim
    from skit.params import ParamDecl

    spec = ParamDecl(name="CITY", binding="const", type="str")
    out = shim.inject(src, [spec], {"CITY": "Kaohsiung"})
    assert "'Kaohsiung'" in out


# ---------- shim: preamble appended when body is only __future__ imports ----------


def test_preamble_appended_when_only_future_imports():
    """A module with only __future__ imports has no viable insertion point; preamble goes at EOF."""
    from skit.langs.python import shim
    from skit.params import ParamDecl

    src = "from __future__ import annotations\nx = input()\nprint(x)\n"
    spec = ParamDecl(name="input-1", binding="input", order=0)
    out = shim.inject(src, [spec], {"input-1": "v"})
    assert "# skit:shim" in out
    # The preamble must appear after the __future__ line (not before it)
    lines = out.splitlines()
    future_idx = next(i for i, ln in enumerate(lines) if "__future__" in ln)
    shim_idx = next(i for i, ln in enumerate(lines) if "# skit:shim" in ln)
    assert shim_idx > future_idx


# ---------- shim: _insert_preamble appends newline when last line has none ----------


def test_preamble_appends_newline_when_missing():
    """A missing trailing newline must not cause the preamble to merge with the last line."""
    # A script with only a __future__ import and no other statements forces idx=None (append path).
    # No trailing newline on the last line exercises the newline-insertion guard.
    src = "from __future__ import annotations"  # no trailing newline, no input() call to inject
    # We need at least one input() call for inject to do anything; add it inline.
    src_with_input = src + "\nresult = input('val: ')\nprint(result)"
    spec = ParamDecl(name="input-1", binding="input", order=0)
    out = shim.inject(src_with_input, [spec], {"input-1": "v"})
    # The preamble line must stand alone (not merged onto a preceding line without a newline)
    for line in out.splitlines():
        if "# skit:shim" in line:
            # The preamble line must contain only the shim marker (no extra source code prefix)
            stripped = line.strip()
            assert stripped.startswith("import") or stripped.startswith("_skit")
            break
    else:
        pytest.fail("No shim preamble line found in output")


# ---------- store: _unique_slug with multiple collisions ----------


def test_unique_slug_multiple_collisions(tmp_path, monkeypatch):
    """When base and base-2 are both taken, _unique_slug must try base-3."""
    from skit.store import _unique_slug

    existing = {"hello", "hello-2"}
    assert _unique_slug("hello", existing) == "hello-3"


# ---------- store: update_dependencies reference mode (no PEP 723 sync) ----------


def test_update_dependencies_reference_mode(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    from skit import store

    script = tmp_path / "tool.py"
    script.write_text("print('hi')\n", encoding="utf-8")
    entry = store.add_python(script, mode="reference")
    updated = store.update_dependencies(entry.slug, ["httpx"])
    assert updated.meta.dependencies == ["httpx"]
    # The original file must not be touched
    assert "httpx" not in script.read_text(encoding="utf-8")


# ---------- store: update_dependencies exe entry (not python, no PEP 723 sync) ----------


def test_update_dependencies_exe_entry(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    from skit import store

    exe = tmp_path / "tool"
    exe.touch()
    entry = store.add_exe(exe)
    updated = store.update_dependencies(entry.slug, ["libssl"])
    assert updated.meta.dependencies == ["libssl"]


# ---------- launcher: find_uv private-bin .exe variant (Windows path) ----------


def test_find_uv_private_bin_exe_variant(tmp_path, monkeypatch):

    monkeypatch.setattr("shutil.which", lambda _: None)
    monkeypatch.setattr("skit.langs.launch.private_bin_dir", lambda: tmp_path / "bin")
    (tmp_path / "bin").mkdir()
    (tmp_path / "bin" / "uv.exe").touch()
    assert launch.find_uv() == str(tmp_path / "bin" / "uv.exe")


# ---------- launcher: _build_python with only requires_python (no deps) ----------


def test_build_python_only_requires_python(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    from skit import launcher, store

    script = tmp_path / "s.py"
    script.write_text("print('ok')\n", encoding="utf-8")
    entry = store.add_python(script)
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/uv")
    entry.meta.requires_python = ">=3.11"
    entry.meta.dependencies = None
    cmd = launcher.build_command(entry)
    assert "--python" in cmd
    assert ">=3.11" in cmd
    assert "--with" not in cmd


# ---------- uvman: ensure_uv_downloaded full success path (mocked network) ----------


def test_ensure_uv_downloaded_success(monkeypatch, tmp_path):
    """Simulate a successful download using stubs that avoid real network and disk I/O.

    Covers: progress print (L118), the download/extract call (L125-128), and the
    success-print + return path (L131-133).
    """
    import sys

    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    exe_name = "uv.exe" if sys.platform == "win32" else "uv"

    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: bin_dir)
    monkeypatch.setattr(uvman, "_ask_consent", lambda _: True)
    monkeypatch.setattr(uvman, "download_url", lambda triple=None: "https://fake/uv.zip")

    def _fake_extract(archive, dest_dir):
        # Simulate extraction: create the exe in dest_dir without touching the archive
        dest_dir.mkdir(parents=True, exist_ok=True)
        exe = dest_dir / exe_name
        exe.write_bytes(b"fake")
        return exe

    monkeypatch.setattr(uvman, "_extract_uv", _fake_extract)
    # This test exercises the success-path plumbing (progress print -> download/extract -> return),
    # not integrity checking, and its stubbed download writes no bytes; the SHA256 gate has its own
    # dedicated coverage in tests/test_uvman.py, so no-op it here.
    monkeypatch.setattr(uvman, "_verify_checksum", lambda archive, triple: None)

    # Patch urlopen + shutil.copyfileobj so no real download or temp-file write occurs
    import shutil as _shutil

    class _FakeResp:
        def __enter__(self):
            return self

        def __exit__(self, *_):
            pass

    monkeypatch.setattr("urllib.request.urlopen", lambda url, timeout=None: _FakeResp())
    monkeypatch.setattr(_shutil, "copyfileobj", lambda src, dst: None)

    result = uvman.ensure_uv_downloaded(quiet=False)
    assert exe_name in result
