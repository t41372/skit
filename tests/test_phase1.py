"""Phase 1: PEP 723 completion, parameter persistence, command placeholders, uv download URL."""

from __future__ import annotations

import os
import tomllib

import pytest

from skit import argstate, launcher, pep723, store, uvman


@pytest.fixture(autouse=True)
def isolated_dirs(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


# ---------- pep723 ----------

BLOCK = """# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "requests",
# ]
# ///
import requests
print(requests.__version__)
"""


def test_parse_block():
    meta = pep723.parse_block(BLOCK)
    assert meta is not None
    assert meta["dependencies"] == ["requests"]
    assert meta["requires-python"] == ">=3.11"


def test_parse_no_block():
    assert pep723.parse_block("print('hi')\n") is None
    assert not pep723.has_block("print('hi')\n")


def test_suggest_dependencies():
    text = "import requests\nimport os\nfrom rich.table import Table\nimport mymod.sub\n"
    got = pep723.suggest_dependencies(text)
    assert "requests" in got
    assert "rich" in got
    assert "os" not in got  # stdlib excluded


def test_suggest_syntax_error_returns_empty():
    assert pep723.suggest_dependencies("def broken(:\n") == []


def test_inject_block_roundtrip():
    src = "#!/usr/bin/env python3\nimport requests\n"
    out = pep723.inject_block(src, ["requests"], ">=3.10")
    assert out.startswith("#!/usr/bin/env python3\n# /// script\n")
    meta = pep723.parse_block(out)
    assert meta is not None
    assert meta["dependencies"] == ["requests"]
    assert meta["requires-python"] == ">=3.10"
    # Idempotent when a block already exists
    assert pep723.inject_block(out, ["other"], "") == out


def test_inject_preserves_body():
    src = "import requests\nprint('x')\n"
    out = pep723.inject_block(src, ["requests"])
    assert out.endswith("import requests\nprint('x')\n")


# ---------- store: copy injection vs reference meta ----------


def test_add_python_copy_injects_pep723(tmp_path):
    script = tmp_path / "s.py"
    script.write_text("import requests\nprint('hi')\n", encoding="utf-8")
    entry = store.add_python(script, dependencies=["requests"], requires_python=">=3.11")
    stored = (entry.dir / "script.py").read_text(encoding="utf-8")
    meta_in_copy = pep723.parse_block(stored)
    assert meta_in_copy is not None
    assert meta_in_copy["dependencies"] == ["requests"]
    # After injecting into the copy, meta.toml doesn't duplicate the info (single source of truth)
    assert entry.meta.dependencies is None
    assert entry.meta.requires_python == ""
    # The original file must never be touched
    assert script.read_text(encoding="utf-8") == "import requests\nprint('hi')\n"


def test_add_python_reference_records_in_meta(tmp_path):
    script = tmp_path / "s.py"
    script.write_text("import requests\n", encoding="utf-8")
    entry = store.add_python(
        script, mode="reference", dependencies=["requests"], requires_python=">=3.11"
    )
    assert entry.meta.dependencies == ["requests"]
    assert entry.meta.requires_python == ">=3.11"
    assert script.read_text(encoding="utf-8") == "import requests\n"  # original untouched
    with open(entry.dir / "meta.toml", "rb") as f:
        doc = tomllib.load(f)
    assert doc["dependencies"] == ["requests"]


def test_add_python_existing_block_not_touched(tmp_path):
    script = tmp_path / "s.py"
    script.write_text(BLOCK, encoding="utf-8")
    entry = store.add_python(script, dependencies=["other"], requires_python=">=3.12")
    stored = (entry.dir / "script.py").read_text(encoding="utf-8")
    block_meta = pep723.parse_block(stored)
    assert block_meta is not None
    assert block_meta["dependencies"] == ["requests"]  # original block preserved


# ---------- launcher: --with / --python passthrough ----------


def test_build_command_reference_deps(tmp_path, monkeypatch):
    script = tmp_path / "s.py"
    script.write_text("import requests\n", encoding="utf-8")
    entry = store.add_python(
        script, mode="reference", dependencies=["requests", "rich"], requires_python=">=3.11"
    )
    monkeypatch.setattr(launcher, "find_uv", lambda: "/fake/uv")
    cmd = launcher.build_command(entry)
    assert cmd[:3] == ["/fake/uv", "run", "--python"]
    assert ">=3.11" in cmd
    assert cmd.count("--with") == 2
    assert "--script" in cmd


# ---------- command placeholders ----------


def test_extract_placeholders():
    assert store.extract_placeholders("ffmpeg -i {input} -o {output} {input}") == [
        "input",
        "output",
    ]
    assert store.extract_placeholders("echo {{literal}} {x}") == ["x"]
    assert store.extract_placeholders("echo plain") == []


def test_command_params_fill_and_escape():
    entry = store.add_command("convert {src} to {dst} keep {{braces}}", name="conv")
    assert entry.meta.params == ["src", "dst"]
    cmd = launcher.build_command(entry, values={"src": "a.png", "dst": "b.jpg"})
    assert cmd == "convert a.png to b.jpg keep {braces}"


def test_command_missing_values_raises():
    entry = store.add_command("echo {x}", name="e")
    with pytest.raises(launcher.LaunchError):
        launcher.build_command(entry, values={})


# ---------- argstate ----------


def test_argstate_roundtrip_and_forget():
    assert argstate.load_last("nope") == {"values": {}, "extra_args": []}
    argstate.save_last("s1", values={"x": "1"}, extra_args=["--fast"])
    got = argstate.load_last("s1")
    assert got["values"] == {"x": "1"}
    assert got["extra_args"] == ["--fast"]
    argstate.forget("s1")
    assert argstate.load_last("s1") == {"values": {}, "extra_args": []}
    argstate.forget("s1")  # idempotent


def test_remove_clears_argstate(tmp_path):
    script = tmp_path / "s.py"
    script.write_text("print('hi')\n", encoding="utf-8")
    entry = store.add_python(script)
    argstate.save_last(entry.slug, extra_args=["--x"])
    store.remove(entry.meta.name)
    assert argstate.load_last(entry.slug) == {"values": {}, "extra_args": []}


# ---------- uvman (no network) ----------


def test_uv_download_url_shape():
    url = uvman.download_url("x86_64-unknown-linux-gnu")
    assert url.startswith("https://github.com/astral-sh/uv/releases/download/")
    assert uvman.UV_VERSION in url
    assert url.endswith("uv-x86_64-unknown-linux-gnu.tar.gz")
    assert uvman.download_url("x86_64-pc-windows-msvc").endswith(".zip")


def test_uv_triple_current_platform():
    # The current CI / sandbox platform must be resolvable (no UvDownloadError)
    triple = uvman._triple()
    assert any(k in triple for k in ("linux", "darwin", "windows"))


def test_ensure_uv_downloaded_skips_when_present(tmp_path, monkeypatch):
    from skit.paths import private_bin_dir

    bin_dir = private_bin_dir()
    bin_dir.mkdir(parents=True, exist_ok=True)
    fake = bin_dir / ("uv.exe" if os.name == "nt" else "uv")
    fake.write_text("#!/bin/sh\n", encoding="utf-8")
    assert uvman.ensure_uv_downloaded(quiet=True) == str(fake)
