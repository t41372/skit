"""A child-process environment that undoes the frozen binary's loader-path surgery.

When skit ships as a PyInstaller binary, the bootloader points the dynamic-loader search
path at its private bundle directory before Python starts: on Linux it prepends the bundle
to ``LD_LIBRARY_PATH`` (stashing any pre-existing value in ``LD_LIBRARY_PATH_ORIG``), and
the equivalent DYLD variables are affected on macOS; on Windows the bundle directory can
end up on ``PATH``. That redirection is what lets the *parent* find its bundled libpython —
but skit's whole job is spawning children (user scripts, uv, editors, npm/bun installers,
``bash -n``/``node --check`` probes), and a child that inherits it may resolve the bundle's
libssl/libpython instead of the system's and crash in ways skit gets blamed for.

Every place skit assembles a child environment therefore starts from :func:`child_env`
instead of ``os.environ``. In a normal (unfrozen) install it is a plain copy; in a frozen
one it restores each loader variable from its ``*_ORIG`` stash when the bootloader saved
one, drops bundle-directory entries otherwise, and removes the bootloader's private
``_PYI_*`` bookkeeping so a child that happens to re-invoke skit boots fresh. The scrub is
deliberately NOT applied to ``os.environ`` itself: the parent still dlopens bundled C
extensions lazily (tree-sitter grammars load on first analysis) and must keep its own
loader path intact.
"""

from __future__ import annotations

import os
import sys
from collections.abc import Mapping
from pathlib import Path

# Variables the PyInstaller bootloader redirects at the bundle, per platform. All three are
# scrubbed unconditionally — a variable that doesn't exist on the current platform is simply
# absent from the environment, so there is no need to branch on sys.platform.
_LOADER_VARS = ("LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH")


def _bundle_dir() -> str | None:
    """The frozen bundle directory, or None when running from a normal install.

    PyInstaller sets both markers: ``sys.frozen`` and ``sys._MEIPASS`` (the extraction dir in
    onefile, the ``_internal`` dir in onedir). Requiring both keeps the helper a no-op under
    any other runtime that sets ``sys.frozen`` alone.
    """
    frozen = getattr(sys, "frozen", False)  # pragma: no mutate — absent-attribute default: False vs None are both falsy (a true equivalent); frozen-detection behavior itself is pinned by test_childenv's frozen/unfrozen suites  # fmt: skip
    if not frozen:
        return None
    bundle: str | None = getattr(sys, "_MEIPASS", None)
    return bundle


def _is_bundle_entry(entry: str, bundle: Path) -> bool:
    return bool(entry) and (Path(entry) == bundle or Path(entry).is_relative_to(bundle))


def _without_bundle_entries(value: str, bundle: Path, *, keep_empty: bool = False) -> str:
    """`value` (a pathsep-joined search path) minus entries inside the bundle directory.

    keep_empty=False also drops empty entries (which mean "current directory" to the
    dynamic loader) — right for the loader variables the bootloader rewrote wholesale,
    wrong for PATH, where an empty entry is the user's own POSIX-blessed cwd-on-PATH
    choice and must survive the scrub (PATH callers pass keep_empty=True).
    """
    kept = [
        entry
        for entry in value.split(os.pathsep)
        if (entry or keep_empty) and not _is_bundle_entry(entry, bundle)
    ]
    return os.pathsep.join(kept)


def child_env(base: Mapping[str, str] | None = None) -> dict[str, str]:
    """A copy of `base` (default: ``os.environ``) safe to hand to a child process.

    Unfrozen: an unmodified copy. Frozen: loader variables are restored from the
    bootloader's ``*_ORIG`` stash (or cleansed of bundle-directory entries when nothing was
    stashed — the bootloader saves no ``_ORIG`` when the variable didn't previously exist),
    ``PATH`` loses any bundle-directory entries, and ``_PYI_*`` bootloader internals are
    dropped.
    """
    env = dict(os.environ if base is None else base)
    bundle = _bundle_dir()
    if bundle is None:
        return env
    bundle_path = Path(bundle)
    for var in _LOADER_VARS:
        orig = env.pop(f"{var}_ORIG", None)
        if orig is not None:
            env[var] = orig
        elif var in env:
            cleansed = _without_bundle_entries(env[var], bundle_path)
            if cleansed:
                env[var] = cleansed
            else:
                del env[var]
    # Windows resolves DLLs (and executables) through PATH; a bundle entry there would leak
    # the frozen runtime into children exactly like LD_LIBRARY_PATH does on Linux. PATH has
    # no *_ORIG stash — the bootloader never replaces it wholesale — so filtering suffices.
    # Touch PATH only when a bundle entry is actually present: everything else about the
    # user's PATH (ordering, empty cwd entries) is their business, and a no-op here keeps
    # frozen behavior byte-identical to a pip install.
    if "PATH" in env and any(
        _is_bundle_entry(entry, bundle_path) for entry in env["PATH"].split(os.pathsep)
    ):
        cleansed = _without_bundle_entries(env["PATH"], bundle_path, keep_empty=True)
        if cleansed:
            env["PATH"] = cleansed
        else:
            del env["PATH"]
    for key in [k for k in env if k.startswith("_PYI_")]:
        del env[key]
    return env
