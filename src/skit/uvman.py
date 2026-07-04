"""UvManager: when uv is missing, auto-download a managed copy into a private bin (A9, the pattern
rye validated).

- PIN a known-good version rather than chasing latest (reproducible, testable).
- Download/extract with pure stdlib (urllib + tarfile/zipfile); no extra dependencies.
- Download progress goes to stderr (stdout is reserved for the script's output).
"""

from __future__ import annotations

import platform
import shutil
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path

from .i18n import gettext
from .paths import private_bin_dir

# Pinned rather than chasing latest: the download URL is built from the version, so users get
# exactly what CI tested (reproducible); chasing latest would turn the fallback path into untested
# code on every upstream release and add an API lookup as a failure point. This is only the
# fallback for "the system has no uv" — if find_uv() locates a system uv, the system one wins.
UV_VERSION = "0.11.26"  # bumped 2026-07-04 (latest at the time)


class UvDownloadError(Exception):
    pass


class UvDeclinedError(UvDownloadError):
    """The user explicitly declined the download. The message includes self-install guidance."""


def _ask_consent(dest_dir: Path) -> bool:
    """Ask once before downloading on an interactive terminal; non-interactive (pipe/CI) keeps A9's
    zero-action behavior but has already been told via stderr.

    - Pulling an executable from the network shouldn't be entirely silent, but the default is Y: the
      target user is someone who "grabbed a script and just wants to run it".
    - The prompt goes to stderr (stdout is reserved for the script's output); EOF counts as consent
      (common in semi-interactive environments).
    """
    if not (sys.stdin.isatty() and sys.stderr.isatty()):
        return True
    print(
        gettext(
            "skit needs Astral's uv to run Python scripts, but it wasn't found on this system. Download uv %(version)s into skit's private directory (%(path)s)? This won't touch your PATH or global environment. [Y/n]"
        )
        % {"version": UV_VERSION, "path": str(dest_dir)},
        file=sys.stderr,
        flush=True,
        end=" ",
    )
    try:
        answer = input()
    except EOFError:
        return True
    return answer.strip().lower() not in ("n", "no")


def _triple() -> str:
    machine = platform.machine().lower()
    arch = {
        "x86_64": "x86_64",
        "amd64": "x86_64",
        "arm64": "aarch64",
        "aarch64": "aarch64",
    }.get(machine)
    if arch is None:
        raise UvDownloadError(
            gettext("Unsupported platform: %(platform)s")
            % {"platform": f"{sys.platform}/{machine}"}
        )
    if sys.platform == "darwin":
        return f"{arch}-apple-darwin"
    if sys.platform == "win32":
        return f"{arch}-pc-windows-msvc"
    return f"{arch}-unknown-linux-gnu"


def download_url(triple: str | None = None) -> str:
    triple = triple or _triple()
    ext = "zip" if "windows" in triple else "tar.gz"
    return f"https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/uv-{triple}.{ext}"


def _extract_uv(archive: Path, dest_dir: Path) -> Path:
    """Extract the uv executable from the archive into dest_dir and return the final path."""
    exe_name = "uv.exe" if sys.platform == "win32" else "uv"
    dest = dest_dir / exe_name
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as zf:
                zf.extractall(tmp_path)  # noqa: S202 — official release zip, extracted to a temp dir
        else:
            with tarfile.open(archive, "r:gz") as tf:
                tf.extractall(tmp_path, filter="data")
        candidates = list(tmp_path.rglob(exe_name))
        if not candidates:
            raise UvDownloadError(
                gettext("No uv binary found inside the archive: %(path)s") % {"path": str(archive)}
            )
        dest_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(candidates[0], dest)
    dest.chmod(dest.stat().st_mode | 0o755)
    return dest


def ensure_uv_downloaded(*, quiet: bool = False) -> str:
    """Download the pinned uv into the private bin and return the path. If it already exists, return
    it directly."""
    exe_name = "uv.exe" if sys.platform == "win32" else "uv"
    dest = private_bin_dir() / exe_name
    if dest.exists():
        return str(dest)
    if not quiet and not _ask_consent(private_bin_dir()):
        raise UvDeclinedError(
            gettext(
                "Download declined. Install uv yourself (https://docs.astral.sh/uv/getting-started/installation/) and skit will pick it up automatically."
            )
        )
    triple = _triple()
    url = download_url(triple)
    if not quiet:
        print(
            gettext("First run — downloading uv %(version)s…") % {"version": UV_VERSION},
            file=sys.stderr,
            flush=True,
        )
    try:
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / url.rsplit("/", 1)[-1]
            # A timeout is mandatory: urlretrieve has no timeout parameter, and a hung network would
            # stall the first run forever.
            with urllib.request.urlopen(url, timeout=60) as resp, open(archive, "wb") as f:  # noqa: S310 — https URL, fixed construction
                shutil.copyfileobj(resp, f)
            path = _extract_uv(archive, private_bin_dir())
    except UvDownloadError:
        raise
    except Exception as exc:  # wrap network/extraction failures uniformly
        raise UvDownloadError(
            gettext("Failed to download uv: %(error)s") % {"error": str(exc)}
        ) from exc
    if not quiet:
        print(
            gettext("uv installed at: %(path)s") % {"path": str(path)}, file=sys.stderr, flush=True
        )
    return str(path)
