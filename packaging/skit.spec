# -*- mode: python ; coding: utf-8 -*-
# PyInstaller build recipe for the single-file skit binary (issue #16).
#
# Build (from the repo root, inside an environment holding the BUILT WHEEL — never an
# editable install, which would drag maintainer-only .po sources in via package data):
#   uv sync --group packaging --no-editable
#   uv run pyinstaller --noconfirm packaging/skit.spec
# The binary lands at dist/skit (dist/skit.exe on Windows); packaging/smoke.py must pass
# against it before it ships.
#
# Choices that are contracts, not defaults:
# - onefile, no UPX: UPX is the classic antivirus-false-positive amplifier and saves
#   little on a zstd-compressed archive.
# - collect_submodules("skit"): cli.py imports skit.inlineform via a string
#   (importlib.import_module), which static analysis cannot see. Collecting the whole
#   package also future-proofs any new dynamic import inside skit.
# - collect_submodules("textual.widgets"): Textual resolves widgets lazily through
#   __getattr__. Today's PyInstaller happens to follow textual's TYPE_CHECKING imports,
#   but that is an accident of textual's source layout — this line is the guarantee.
# - collect_submodules("shellingham"): typer's completion machinery picks a
#   shellingham backend at runtime (typer issue #101).
# - hiddenimports for the tree-sitter grammars: today they are static imports inside
#   try/except guards, which PyInstaller follows — but a mis-bundled grammar DEGRADES
#   SILENTLY (analyzers become None by design), so the grammars are pinned here and
#   positively asserted by packaging/smoke.py.
# - copy_metadata("skit-cli"): importlib.metadata.version("skit-cli") must not fall
#   back to "0.0.0+unknown" (src/skit/__init__.py).
# - "X utf8=1": frozen apps ignore PYTHONUTF8 (PyInstaller 6.x); legacy cp1252
#   Windows consoles would garble the TUI's box drawing and CJK without it. The X-flag
#   name is CPython's `-X utf8` (the bootloader recognizes only "utf8" and "dev" — the
#   sys.flags FIELD is called utf8_mode, the option is not).

import os

from PyInstaller.utils.hooks import collect_data_files, collect_submodules, copy_metadata

a = Analysis(
    [os.path.join(SPECPATH, "entry.py")],
    # collect_data_files: locales/**/skit.mo (gettext catalogs) + skills/skit/SKILL.md
    # (the packaged Agent Skill), laid out so Path(__file__).parent/"locales" and
    # importlib.resources.files("skit.skills") both resolve inside the bundle.
    datas=collect_data_files("skit") + copy_metadata("skit-cli"),
    hiddenimports=[
        "tree_sitter_bash",
        "tree_sitter_javascript",
        "tree_sitter_typescript",
        *collect_submodules("skit"),
        *collect_submodules("textual.widgets"),
        *collect_submodules("shellingham"),
    ],
    # Never imported by skit; excluding them keeps the binary a few MB smaller and, more
    # importantly, keeps pkg_resources' deprecation-warning-at-import out of the bundle.
    excludes=["setuptools", "pkg_resources", "pip", "wheel"],
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    # Frozen CPython reads -X options from here, not the environment (see header).
    [("X utf8=1", None, "OPTION")],
    name="skit",
    console=True,  # a TUI dies without a console; never --windowed
    upx=False,
    strip=False,
    # macOS: leaving codesign_identity unset makes PyInstaller ad-hoc-sign every Mach-O,
    # which is mandatory on arm64 and sufficient for curl/tar distribution (curl sets no
    # quarantine xattr; only browser downloads face Gatekeeper).
)
