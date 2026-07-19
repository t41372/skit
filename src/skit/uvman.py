"""UvManager: when uv is missing, auto-download a managed copy into a private bin (A9, the pattern
rye validated).

- PIN a known-good version rather than chasing latest (reproducible, testable).
- Download/extract with pure stdlib (urllib + tarfile/zipfile); no extra dependencies.
- Download progress goes to stderr (stdout is reserved for the script's output).
"""

from __future__ import annotations

import contextlib
import hashlib
import os
import platform
import shutil
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path

from . import config
from .i18n import gettext
from .paths import private_bin_dir

# Pinned rather than chasing latest: the download URL is built from the version, so users get
# exactly what CI tested (reproducible); chasing latest would turn the fallback path into untested
# code on every upstream release and add an API lookup as a failure point. This is only the
# fallback for "the system has no uv" — if find_uv() locates a system uv, the system one wins.
# Bump: change this line -> REFRESH _UV_SHA256 below from the official `.sha256` sidecars for the new
# version (the pinned hashes are version-specific — a stale table would reject every download) -> run
# tests/test_uvman.py (URL liveness + the SKIT_NET_TESTS sidecar cross-check) -> three-platform CI green.
UV_VERSION = "0.11.26"  # bumped 2026-07-04 (latest at the time)

# Official SHA256 of each release archive, from Astral's per-asset `.sha256` sidecars for UV_VERSION.
# The downloaded archive is verified against this table BEFORE extraction, so a hostile or compromised
# mirror (or a corrupt transfer) can never get skit to extract and execute a trojaned uv — the fetch
# fails closed on any mismatch, and equally if a triple is missing here. These 8 triples are exactly
# what _triple() can emit: {x86_64, aarch64} x {apple-darwin, unknown-linux-gnu, unknown-linux-musl,
# pc-windows-msvc} — the musl pair covers Alpine and other musl-libc Linux userlands (see _is_musl).
# MUST be refreshed alongside UV_VERSION (see the bump note above).
_UV_SHA256: dict[str, str] = {
    "aarch64-apple-darwin": "8f7fbf1708399b921857bce71e1d60f0d3ccf52a30caebc1c1a2f175dce13ab6",
    "x86_64-apple-darwin": "922b460202707dd5f4ccacbadbe7f6a546cc46e82a99bf50ca99a7977a78eddd",
    "aarch64-unknown-linux-gnu": "befa1a59c91e96eb601b0fd9a97c03dd666f17baba644b2b4db9c59a767e387e",
    "x86_64-unknown-linux-gnu": "6426a73c3837e6e2483ee344cbc00f36394d179afcba6183cb77437e67db4af0",
    "aarch64-unknown-linux-musl": "47418cfdb34b1ca42e503da72631ac8c475602e2411ac6c39aa84c2373fe6324",
    "x86_64-unknown-linux-musl": "62bf1a53501adf4083224b69b33737450ac516935f5a5e483e9dfaf2665084de",
    "aarch64-pc-windows-msvc": "98246149741f558e25e45ecf2b0b20f34de0634269f2bf0dcb4012d4b6ba289a",
    "x86_64-pc-windows-msvc": "4e1278ede866be6c0bf32d2f466cc6de7a9fb399ecf20c9ce2d186e52424be47",
}


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


_MUSL_LD_DIR = Path("/lib")
_MUSL_LD_GLOB = "ld-musl-*.so.1"


def _is_musl() -> bool:
    """Best-effort detection of a musl libc userland (Alpine Linux and similar).

    musl's dynamic linker always installs at the fixed path /lib/ld-musl-<arch>.so.1 — baked
    into the PT_INTERP field of every musl-linked ELF binary, unlike glibc's version- and
    distro-dependent ld-linux naming — so this file's presence is a reliable, dependency-free
    signal. Deliberately avoids shelling out to `ldd`/`getconf` or parsing ELF headers ourselves:
    a plain path check can't fail in a way that blocks the download. Only meaningful (and only
    called) under sys.platform == "linux"; harmless to call elsewhere since musl never installs
    at this path outside Linux.
    """
    return any(_MUSL_LD_DIR.glob(_MUSL_LD_GLOB))


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
    libc = "musl" if _is_musl() else "gnu"
    return f"{arch}-unknown-linux-{libc}"


_UV_RELEASES = "https://github.com/astral-sh/uv/releases/download"


def download_url(triple: str | None = None) -> str:
    triple = triple or _triple()
    ext = "zip" if "windows" in triple else "tar.gz"
    base = config.uv_binary_base() or _UV_RELEASES
    return f"{base}/{UV_VERSION}/uv-{triple}.{ext}"


def _fsync_path(path: Path) -> None:
    """Fsync `path` (a file or a directory) so its data — or, for a directory, the rename/link
    entries within it — is durable on stable storage, not just sitting in the page cache.

    Flag choice is platform-split: on POSIX, fsync() flushes the inode's dirty pages regardless of
    the fd's access mode, and directories can never be opened for writing, so O_RDONLY works for
    both the staged file and (only on POSIX) the directory fsync. On Windows, os.fsync maps to the
    CRT `_commit`, which requires a *writable* fd — a read-only handle fails with EBADF — and the
    directory fsync isn't attempted there anyway (os.open can't open a directory), so this is only
    ever called on the staged file, which we can safely open O_RDWR.
    """
    # The Windows arm can't be exercised on the POSIX mutation runner: there O_RDONLY and O_RDWR are
    # equivalent for fsync, and a flipped condition only opens the dir fd O_RDWR (EISDIR, swallowed
    # by the caller's contextlib.suppress) — so no mutant is observable. Hence: no mutate.
    flags = os.O_RDWR if sys.platform == "win32" else os.O_RDONLY  # pragma: no mutate
    fd = os.open(path, flags)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _extract_uv(archive: Path, dest_dir: Path) -> Path:
    """Extract the uv executable from the archive into dest_dir and return the final path.

    Installs atomically (mirrors atomic.py's tmp + os.replace pattern, C7): the executable is
    copied into a throwaway file inside dest_dir, fsync'd, and only os.replace()'d onto the real
    `dest` path once the copy, chmod, and fsync have all fully succeeded. If the process is killed
    or the disk fills up partway through that copy, the only thing left behind is the throwaway
    tmp file — `dest` itself either holds the complete, verified, executable binary or does not
    exist at all. This matters because ensure_uv_downloaded and launcher.find_uv both gate purely
    on dest.exists(): without this, a truncated file at `dest` would look "already installed"
    forever and skit would never self-heal by re-downloading. The same-directory tmp file also
    makes concurrent first runs safe (last os.replace wins, never a torn file).

    The fsync of the staged file's data BEFORE os.replace is what makes the "complete or absent"
    guarantee hold across power loss, not just SIGKILL/ENOSPC: os.replace() is atomic with respect
    to concurrent readers (they see either the old or the new dest, never a torn file), but that
    says nothing about whether the staged file's bytes have actually reached disk yet — without
    the fsync, a crash right after the rename commits but before the page cache writes back could
    leave `dest` renamed-in with zero-length/garbage contents. The directory fsync afterward is a
    best-effort extra: it durably persists the rename's directory-entry update too, but isn't
    required for the dest-content guarantee above (and isn't attempted on Windows, where
    directories can't be opened via os.open).
    """
    exe_name = "uv.exe" if sys.platform == "win32" else "uv"
    dest = dest_dir / exe_name
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as zf:
                zf.extractall(tmp_path)  # noqa: S202 — official release zip, extracted to a temp dir
        else:
            with tarfile.open(
                archive, "r:gz"
            ) as tf:  # pragma: no mutate — "r" mode auto-detects gzip compression
                tf.extractall(tmp_path, filter="data")
        candidates = list(tmp_path.rglob(exe_name))
        if not candidates:
            raise UvDownloadError(
                gettext("No uv binary found inside the archive: %(path)s") % {"path": str(archive)}
            )
        dest_dir.mkdir(parents=True, exist_ok=True)
        fd, staged_name = tempfile.mkstemp(dir=dest_dir, prefix=f".{exe_name}.", suffix=".tmp")
        os.close(fd)
        staged = Path(staged_name)
        try:
            shutil.copy2(candidates[0], staged)
            staged.chmod(staged.stat().st_mode | 0o755)
            _fsync_path(staged)  # durable on disk BEFORE the rename, not just BEFORE this return
            os.replace(staged, dest)
            if sys.platform != "win32":  # os.open can't open a directory on Windows
                with contextlib.suppress(OSError):
                    _fsync_path(dest_dir)  # best-effort: persist the rename's directory entry too
        except BaseException:
            with contextlib.suppress(OSError):
                staged.unlink()
            raise
    return dest


def _verify_checksum(archive: Path, triple: str) -> None:
    """Verify the downloaded archive against the pinned official SHA256 for its platform triple.

    Fails closed: an unknown triple (no pinned hash) or any digest mismatch raises UvDownloadError, so
    skit never extracts or runs a uv binary it could not authenticate. This is the defense that hardens
    BOTH the GitHub path and the China-mirror / custom-host path against a compromised or corrupt
    download — the pinned hashes come from Astral's official release, not from wherever we fetched.
    """
    expected = _UV_SHA256.get(triple)
    if expected is None:  # no pin for this triple -> refuse rather than trust an unverified binary
        raise UvDownloadError(
            gettext("No pinned checksum for platform %(triple)s; refusing to run an unverified uv.")
            % {"triple": triple}
        )
    actual = hashlib.sha256(archive.read_bytes()).hexdigest()
    if actual != expected:
        raise UvDownloadError(
            gettext(
                "Downloaded uv failed its checksum (the mirror may be compromised or the file corrupt). Expected %(expected)s, got %(actual)s."
            )
            % {"expected": expected, "actual": actual}
        )


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
            archive = Path(tmp) / url.rsplit("/", 1)[-1]  # pragma: no mutate — [1]==[-1]
            # A timeout is mandatory: urlretrieve has no timeout parameter, and a hung network would
            # stall the first run forever.
            with urllib.request.urlopen(url, timeout=60) as resp, open(archive, "wb") as f:  # noqa: S310 — url is always https: the GitHub default/presets are https, every custom entrance rejects a non-https github-release base (cli._prompt_github_base, cli._set_mirror_github, tui_prefs._resolve_mirror), and config.load_mirror blanks a non-https hand-edited uv_binary (falling back to the GitHub default)
                shutil.copyfileobj(resp, f)
            # Verify integrity BEFORE extraction/execution: a compromised mirror or corrupt transfer
            # must fail closed here, never reach _extract_uv + chmod +x. The UvDownloadError raised on
            # mismatch propagates as-is (below), so the user sees the checksum error, not "Failed to
            # download".
            _verify_checksum(archive, triple)
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
