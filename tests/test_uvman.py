"""uvman tests: download consent (no network) + pinned-version URL liveness (opt-in network).

When bumping UV_VERSION, set SKIT_NET_TESTS=1 and run this file to confirm the pinned
version's download URLs are live on all three platforms before committing.
"""

from __future__ import annotations

import hashlib
import io
import os
import sys
import urllib.request
from pathlib import Path

import pytest

from skit import uvman

# The installed binary's name is platform-native: `_extract_uv`/`ensure_uv_downloaded` look for
# (and stage) `uv.exe` on Windows, `uv` elsewhere. Tests that build real archives and assert on the
# staged path must mirror that, or they'd hunt for `uv` inside a Windows install and find nothing.
EXE = "uv.exe" if sys.platform == "win32" else "uv"

net = pytest.mark.skipif(
    not os.environ.get("SKIT_NET_TESTS"),
    reason="network liveness test; set SKIT_NET_TESTS=1 when bumping UV_VERSION",
)

TRIPLES = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
]


def _producible_triples(monkeypatch: pytest.MonkeyPatch) -> set[str]:
    """Every triple that _triple() can actually emit -- {x86_64, aarch64} x {apple-darwin,
    unknown-linux-gnu, unknown-linux-musl, pc-windows-msvc} -- computed by exhaustively
    monkeypatching platform.machine/sys.platform/_is_musl rather than hardcoded, so this stays
    correct if _triple()'s branching ever changes (e.g. a new arch or libc flavor is added)."""
    produced: set[str] = set()
    for machine in ("x86_64", "arm64"):
        for plat in ("darwin", "win32", "linux"):
            monkeypatch.setattr("platform.machine", lambda m=machine: m)
            monkeypatch.setattr("sys.platform", plat)
            if plat == "linux":
                # linux additionally branches on libc flavor; cover both to reach the musl triples.
                for is_musl in (False, True):
                    monkeypatch.setattr(uvman, "_is_musl", lambda v=is_musl: v)
                    produced.add(uvman._triple())
            else:
                produced.add(uvman._triple())
    return produced


def test_triples_covers_every_pinned_and_producible_triple(monkeypatch: pytest.MonkeyPatch) -> None:
    """TRIPLES is exactly what the two network tests below (test_pinned_uv_release_exists and
    test_pinned_sha256_matches_live_sidecar -- the only automated mechanism that cross-checks a
    pinned hash against Astral's live `.sha256` sidecar) iterate over. If TRIPLES ever drifts from
    the full set of pinned (_UV_SHA256) / producible (_triple()) triples -- exactly what happened
    when the two musl triples were pinned but never added to TRIPLES -- that live cross-check
    would silently stop covering the missing triple(s), and a bad pin could ship undetected. This
    test is offline (no network touched) but proves the drift itself can't happen silently: any
    future triple added to _UV_SHA256 or made producible by _triple() must also be added here."""
    produced = _producible_triples(monkeypatch)
    assert set(TRIPLES) == set(uvman._UV_SHA256) == produced


@net
@pytest.mark.parametrize("triple", TRIPLES)
def test_pinned_uv_release_exists(triple: str) -> None:
    url = uvman.download_url(triple)
    req = urllib.request.Request(url, method="HEAD")
    with urllib.request.urlopen(req, timeout=30) as resp:
        assert resp.status == 200


@net
@pytest.mark.parametrize("triple", TRIPLES)
def test_pinned_sha256_matches_live_sidecar(triple: str) -> None:
    """A future UV_VERSION bump that forgets to refresh _UV_SHA256 must fail loudly here: every
    pinned hash must equal the official `.sha256` sidecar for that release archive. Built from the
    canonical GitHub release base (not download_url) so a configured mirror can't skew the check."""
    ext = "zip" if "windows" in triple else "tar.gz"
    sidecar = f"{uvman._UV_RELEASES}/{uvman.UV_VERSION}/uv-{triple}.{ext}.sha256"
    with urllib.request.urlopen(sidecar, timeout=30) as resp:
        official = resp.read().decode().split()[0].strip()
    assert official == uvman._UV_SHA256[triple]


# ---- Download consent (_ask_consent), no network ----


def _tty(monkeypatch, *, stdin: bool, stderr: bool) -> None:
    monkeypatch.setattr("sys.stdin", io.StringIO(""), raising=False)
    monkeypatch.setattr("sys.stdin.isatty", lambda: stdin, raising=False)
    monkeypatch.setattr("sys.stderr.isatty", lambda: stderr, raising=False)


def test_consent_non_interactive_auto_yes(monkeypatch) -> None:
    """Pipe / CI context: honour A9 zero-friction, don't block waiting for input."""
    _tty(monkeypatch, stdin=False, stderr=False)
    assert uvman._ask_consent(Path("/tmp/x")) is True


@pytest.mark.parametrize(
    ("answer", "expected"),
    [
        ("", True),  # bare Enter = default Y
        ("y", True),
        ("Y", True),
        ("yes", True),
        ("n", False),
        ("N", False),
        ("no", False),
        ("  n  ", False),  # leading/trailing whitespace stripped
    ],
)
def test_consent_interactive_answers(monkeypatch, answer: str, expected: bool) -> None:
    _tty(monkeypatch, stdin=True, stderr=True)
    monkeypatch.setattr("builtins.input", lambda: answer)
    assert uvman._ask_consent(Path("/tmp/x")) is expected


def test_consent_eof_is_yes(monkeypatch) -> None:
    """Semi-interactive context (isatty is True but no input is readable): EOF counts as
    consent so the first run doesn't hang."""
    _tty(monkeypatch, stdin=True, stderr=True)

    def _raise() -> str:
        raise EOFError

    monkeypatch.setattr("builtins.input", _raise)
    assert uvman._ask_consent(Path("/tmp/x")) is True


def test_declined_raises_with_guidance(monkeypatch, tmp_path) -> None:
    """Declining the download raises UvDeclinedError (a UvDownloadError subclass, handled
    uniformly by the caller), and the message includes self-install guidance."""
    monkeypatch.setattr(uvman, "_ask_consent", lambda _: False)
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: tmp_path / "bin")
    with pytest.raises(uvman.UvDeclinedError) as exc_info:
        uvman.ensure_uv_downloaded()
    assert "uv" in str(exc_info.value)


def test_quiet_skips_consent(monkeypatch, tmp_path) -> None:
    """quiet=True (programmatic call): consent is bypassed. The test interrupts execution at
    the URL-construction stage to avoid touching the network."""
    called = {"consent": False}

    def _spy(_dest: Path) -> bool:
        called["consent"] = True
        return True

    monkeypatch.setattr(uvman, "_ask_consent", _spy)
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: tmp_path / "bin")

    def _boom(*a, **k):
        raise RuntimeError("stop before network")

    monkeypatch.setattr(uvman, "download_url", _boom)
    # download_url is called outside the consent guard, so RuntimeError propagates unchanged —
    # the test only cares that consent was never invoked.
    with pytest.raises(RuntimeError, match="stop before network"):
        uvman.ensure_uv_downloaded(quiet=True)
    assert called["consent"] is False


# ---- _triple: unsupported architecture ----------


def test_triple_unsupported_arch_raises(monkeypatch) -> None:
    monkeypatch.setenv("SKIT_LANG", "en")
    monkeypatch.setattr("platform.machine", lambda: "mips")
    monkeypatch.setattr("sys.platform", "linux")
    monkeypatch.setattr(uvman, "_is_musl", lambda: False)
    with pytest.raises(uvman.UvDownloadError, match=r"(?i)unsupported"):
        uvman._triple()


def test_triple_darwin_aarch64(monkeypatch) -> None:
    monkeypatch.setattr("platform.machine", lambda: "arm64")
    monkeypatch.setattr("sys.platform", "darwin")
    assert uvman._triple() == "aarch64-apple-darwin"


def test_triple_windows_x86_64(monkeypatch) -> None:
    monkeypatch.setattr("platform.machine", lambda: "AMD64")
    monkeypatch.setattr("sys.platform", "win32")
    assert uvman._triple() == "x86_64-pc-windows-msvc"


def test_triple_linux_aarch64(monkeypatch) -> None:
    monkeypatch.setattr("platform.machine", lambda: "aarch64")
    monkeypatch.setattr("sys.platform", "linux")
    monkeypatch.setattr(uvman, "_is_musl", lambda: False)
    assert uvman._triple() == "aarch64-unknown-linux-gnu"


# ---- _triple / _is_musl: musl (Alpine) detection ----------


def test_is_musl_true_when_ld_musl_present(monkeypatch, tmp_path) -> None:
    """_is_musl() detects musl by the presence of the fixed-path dynamic linker
    /lib/ld-musl-<arch>.so.1 that every musl-linked binary is built to expect."""
    fake_lib = tmp_path / "lib"
    fake_lib.mkdir()
    (fake_lib / "ld-musl-x86_64.so.1").touch()
    monkeypatch.setattr(uvman, "_MUSL_LD_DIR", fake_lib)
    assert uvman._is_musl() is True


def test_is_musl_false_when_ld_musl_absent(monkeypatch, tmp_path) -> None:
    fake_lib = tmp_path / "lib"
    fake_lib.mkdir()  # empty: no ld-musl-*.so.1 inside
    monkeypatch.setattr(uvman, "_MUSL_LD_DIR", fake_lib)
    assert uvman._is_musl() is False


def test_is_musl_false_when_lib_dir_missing(monkeypatch, tmp_path) -> None:
    """A missing /lib (e.g. a minimal container layout) must not raise — just means "not musl"."""
    monkeypatch.setattr(uvman, "_MUSL_LD_DIR", tmp_path / "no-such-lib-dir")
    assert uvman._is_musl() is False


def test_triple_linux_musl_x86_64(monkeypatch) -> None:
    """On a musl userland (e.g. Alpine), _triple() must select the musl target, not gnu —
    a gnu uv binary cannot exec without glibc's dynamic loader."""
    monkeypatch.setattr("platform.machine", lambda: "x86_64")
    monkeypatch.setattr("sys.platform", "linux")
    monkeypatch.setattr(uvman, "_is_musl", lambda: True)
    assert uvman._triple() == "x86_64-unknown-linux-musl"


def test_triple_linux_musl_aarch64(monkeypatch) -> None:
    monkeypatch.setattr("platform.machine", lambda: "aarch64")
    monkeypatch.setattr("sys.platform", "linux")
    monkeypatch.setattr(uvman, "_is_musl", lambda: True)
    assert uvman._triple() == "aarch64-unknown-linux-musl"


def test_download_url_musl_triple_targz(monkeypatch) -> None:
    """The musl triples are Linux, so they must still map to the tar.gz archive extension (only
    the windows triples use .zip)."""
    url = uvman.download_url("x86_64-unknown-linux-musl")
    assert url.endswith(".tar.gz")
    assert f"{uvman.UV_VERSION}/uv-x86_64-unknown-linux-musl.tar.gz" in url


# ---- download_url ----------


def test_download_url_structure() -> None:
    url = uvman.download_url("x86_64-unknown-linux-gnu")
    assert uvman.UV_VERSION in url
    assert url.endswith(".tar.gz")
    url_win = uvman.download_url("x86_64-pc-windows-msvc")
    assert url_win.endswith(".zip")


# ---- ensure_uv_downloaded: binary already exists ----------


def test_ensure_uv_already_exists(monkeypatch, tmp_path) -> None:
    """If the binary is already present, skip download and return the path immediately."""
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    exe = bin_dir / EXE
    exe.touch()
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: bin_dir)

    def _boom(*a, **k):
        raise RuntimeError("network should never be touched when the binary already exists")

    monkeypatch.setattr("urllib.request.urlopen", _boom)
    result = uvman.ensure_uv_downloaded(quiet=True)
    assert result == str(exe)


# ---- _extract_uv: missing executable in archive raises ----------


def test_extract_uv_no_exe_in_archive_raises(tmp_path) -> None:
    """An archive that contains no 'uv' executable must raise UvDownloadError."""
    import tarfile

    archive = tmp_path / "empty.tar.gz"
    # Create a tar.gz with an unrelated file
    member = tmp_path / "README.txt"
    member.write_text("nothing here\n", encoding="utf-8")
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(member, arcname="README.txt")
    with pytest.raises(uvman.UvDownloadError):
        uvman._extract_uv(archive, tmp_path / "dest")


# ---- ensure_uv_downloaded: network error wrapped as UvDownloadError ----------


def test_ensure_uv_network_error_wrapped(monkeypatch, tmp_path) -> None:
    import urllib.error

    monkeypatch.setattr(uvman, "_ask_consent", lambda _: True)
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: tmp_path / "bin")

    def _fail(*a, **kw):
        raise urllib.error.URLError("connection refused")

    monkeypatch.setattr("urllib.request.urlopen", _fail)
    with pytest.raises(uvman.UvDownloadError):
        uvman.ensure_uv_downloaded(quiet=True)


def test_download_url_uses_configured_mirror(monkeypatch, tmp_path):
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
    from skit import config

    config.save_mirror(config.preset("tsinghua"))
    url = uvman.download_url("aarch64-apple-darwin")
    assert url.startswith(config.UV_BINARY_MIRROR)
    assert f"{uvman.UV_VERSION}/uv-aarch64-apple-darwin.tar.gz" in url


def test_download_url_defaults_to_github_without_mirror(monkeypatch, tmp_path):
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
    url = uvman.download_url("x86_64-unknown-linux-gnu")
    assert url.startswith("https://github.com/astral-sh/uv/releases/download")
    assert url.endswith(".tar.gz")


def test_download_url_github_when_uv_binary_blank(monkeypatch, tmp_path):
    """(e) Mirror enabled but uv_binary left blank -> fall back to the GitHub release base."""
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
    from skit import config

    config.save_mirror(config.MirrorConfig(enabled=True, pypi="https://x/simple", uv_binary=""))
    url = uvman.download_url("aarch64-apple-darwin")
    assert url.startswith("https://github.com/astral-sh/uv/releases/download")
    assert f"{uvman.UV_VERSION}/uv-aarch64-apple-darwin.tar.gz" in url


# ---- SHA256 pinning + checksum verification (F3, no network) ----------


def _fake_urlopen(data: bytes):
    """A urllib.request.urlopen stand-in that serves fixed bytes (as a BytesIO context manager),
    so ensure_uv_downloaded's shutil.copyfileobj writes exactly `data` to the archive."""

    def _open(url, timeout=None):  # signature match for urlopen(url, timeout=...)
        return io.BytesIO(data)

    return _open


def test_uv_sha256_covers_every_producible_triple(monkeypatch) -> None:
    """The pinned table must key on exactly the triples _triple() can emit —
    {x86_64, aarch64} x {apple-darwin, unknown-linux-gnu, unknown-linux-musl, pc-windows-msvc} —
    so no reachable platform is left without a hash to verify against."""
    produced = _producible_triples(monkeypatch)
    assert len(produced) == 8
    assert set(uvman._UV_SHA256) == produced
    # Each pinned value is a 64-char lowercase-hex SHA256 digest.
    assert all(
        len(h) == 64 and all(c in "0123456789abcdef" for c in h) for h in uvman._UV_SHA256.values()
    )


def test_checksum_pass_proceeds_to_extraction(monkeypatch, tmp_path) -> None:
    """When the archive's SHA256 equals the pinned hash, the checksum gate opens and control
    reaches extraction (stubbed to a sentinel); ensure_uv_downloaded returns the extracted path."""
    data = b"known-good-uv-archive-bytes"
    triple = "x86_64-unknown-linux-gnu"
    monkeypatch.setattr(uvman, "_triple", lambda: triple)
    monkeypatch.setattr(uvman, "_UV_SHA256", {triple: hashlib.sha256(data).hexdigest()})
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: tmp_path / "bin")
    monkeypatch.setattr("urllib.request.urlopen", _fake_urlopen(data))

    seen: dict[str, bytes] = {}

    def _fake_extract(archive: Path, dest_dir: Path) -> Path:
        # Capture the archive bytes now, while the download's TemporaryDirectory still exists,
        # to prove the verified bytes are exactly what reached extraction.
        seen["archive_bytes"] = archive.read_bytes()
        return dest_dir / EXE

    monkeypatch.setattr(uvman, "_extract_uv", _fake_extract)

    result = uvman.ensure_uv_downloaded(quiet=True)
    assert seen, "extraction was not reached — the checksum gate did not pass"
    assert result == str(tmp_path / "bin" / EXE)
    assert seen["archive_bytes"] == data


def test_checksum_mismatch_raises_checksum_error_not_generic(monkeypatch, tmp_path) -> None:
    """A tampered/corrupt archive (hash != pinned) fails closed with the checksum message — NOT the
    generic 'Failed to download' wrapper — and extraction is never reached."""
    monkeypatch.setenv("SKIT_LANG", "en")
    data = b"tampered-bytes-from-a-hostile-mirror"
    pinned = "00" * 32  # a valid-shaped but wrong digest
    triple = "x86_64-unknown-linux-gnu"
    monkeypatch.setattr(uvman, "_triple", lambda: triple)
    monkeypatch.setattr(uvman, "_UV_SHA256", {triple: pinned})
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: tmp_path / "bin")
    monkeypatch.setattr("urllib.request.urlopen", _fake_urlopen(data))

    extracted = {"called": False}
    monkeypatch.setattr(uvman, "_extract_uv", lambda *a: extracted.__setitem__("called", True))

    with pytest.raises(uvman.UvDownloadError) as exc_info:
        uvman.ensure_uv_downloaded(quiet=True)

    msg = str(exc_info.value)
    assert "checksum" in msg.lower()  # distinguishes it from the generic download failure
    assert "Failed to download" not in msg
    assert pinned in msg  # expected digest surfaced
    assert hashlib.sha256(data).hexdigest() in msg  # actual digest surfaced
    assert extracted["called"] is False  # a mismatched archive is never extracted


def _tar_gz_with_uv(
    tmp_path: Path, exe_name: str = EXE, content: bytes = b"genuine-uv-bytes"
) -> Path:
    """Build a real tar.gz archive (named uniquely per call) containing a single executable member
    called `exe_name`, for tests that exercise the real _extract_uv extraction/copy pipeline."""
    import tarfile

    src_dir = tmp_path / f"src-{os.urandom(4).hex()}"
    src_dir.mkdir()
    member = src_dir / exe_name
    member.write_bytes(content)
    archive = tmp_path / f"uv-{os.urandom(4).hex()}.tar.gz"
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(member, arcname=f"uv-1.0/{exe_name}")
    return archive


# ---- _extract_uv: atomic install (no partial binary survives a mid-copy failure) ----------


def test_extract_uv_failed_copy_leaves_no_partial_binary(monkeypatch, tmp_path) -> None:
    """A copy interrupted partway (simulated: SIGKILL/ENOSPC in production) must not leave a
    corrupt file at the final `uv` path, and must not leave any stray tmp file behind either —
    dest_dir ends up exactly as if the install never started."""
    archive = _tar_gz_with_uv(tmp_path)
    dest_dir = tmp_path / "dest"

    def _boom(_src, _dst, *a, **kw):
        raise OSError("simulated disk-full mid-copy")

    monkeypatch.setattr(uvman.shutil, "copy2", _boom)

    with pytest.raises(OSError, match="simulated disk-full"):
        uvman._extract_uv(archive, dest_dir)

    assert not (dest_dir / EXE).exists()
    assert list(dest_dir.iterdir()) == []  # the staged tmp file was cleaned up — nothing poisoned


def test_extract_uv_self_heals_after_interrupted_install(monkeypatch, tmp_path) -> None:
    """After a failed install leaves no binary at dest, a fresh (unpatched) extraction attempt
    must succeed cleanly — proving the failure didn't poison dest_dir for next time."""
    archive = _tar_gz_with_uv(tmp_path, content=b"the-real-uv-binary")
    dest_dir = tmp_path / "dest"

    monkeypatch.setattr(
        uvman.shutil, "copy2", lambda *a, **kw: (_ for _ in ()).throw(OSError("boom"))
    )
    with pytest.raises(OSError, match="boom"):
        uvman._extract_uv(archive, dest_dir)
    assert not (dest_dir / EXE).exists()

    monkeypatch.undo()  # restore the real shutil.copy2
    dest = uvman._extract_uv(archive, dest_dir)
    assert dest == dest_dir / EXE
    assert dest.read_bytes() == b"the-real-uv-binary"


# ---- _extract_uv: staged-file fsync (durability across power loss, not just SIGKILL/ENOSPC) ----


def test_extract_uv_fsyncs_staged_file_before_replace(monkeypatch, tmp_path) -> None:
    """os.replace() alone only guarantees atomicity wrt concurrent readers -- it says nothing
    about whether the staged file's bytes have reached stable storage. The staged file must be
    fsync'd BEFORE the rename commits, so a crash between the rename and the page-cache writeback
    can't leave `dest` looking installed but holding zero-length/garbage bytes. Spy on the call
    order of os.fsync vs os.replace (real fsync/replace still run; only the order is observed)."""
    archive = _tar_gz_with_uv(tmp_path)
    dest_dir = tmp_path / "dest"

    calls: list[str] = []
    real_fsync = os.fsync
    real_replace = os.replace

    def _spy_fsync(fd):
        calls.append("fsync")
        return real_fsync(fd)

    def _spy_replace(src, dst):
        calls.append("replace")
        return real_replace(src, dst)

    monkeypatch.setattr(uvman.os, "fsync", _spy_fsync)
    monkeypatch.setattr(uvman.os, "replace", _spy_replace)

    dest = uvman._extract_uv(archive, dest_dir)

    assert dest == dest_dir / EXE
    assert dest.read_bytes() == b"genuine-uv-bytes"
    assert "fsync" in calls
    assert "replace" in calls
    assert calls.index("fsync") < calls.index("replace")  # staged data synced before the rename


def test_extract_uv_dir_fsync_failure_is_swallowed(monkeypatch, tmp_path) -> None:
    """The post-replace directory fsync is best-effort (not every filesystem/platform supports
    fsync on a directory fd): a failure there must not fail the install or leak past the
    contextlib.suppress guarding it, since the durability guarantee for dest's *contents* was
    already secured by the staged-file fsync before the rename."""
    archive = _tar_gz_with_uv(tmp_path)
    dest_dir = tmp_path / "dest"
    real_fsync_path = uvman._fsync_path

    def _selective(path):
        if path == dest_dir:
            raise OSError("simulated: fsync not supported for directories on this filesystem")
        return real_fsync_path(path)

    monkeypatch.setattr(uvman, "_fsync_path", _selective)

    dest = uvman._extract_uv(archive, dest_dir)
    assert dest == dest_dir / EXE
    assert dest.read_bytes() == b"genuine-uv-bytes"


def test_extract_uv_staged_fsync_failure_triggers_existing_cleanup(monkeypatch, tmp_path) -> None:
    """Unlike the best-effort directory fsync, a failure fsync'ing the staged file's *data* is not
    swallowed: it must propagate, and it must compose with the pre-existing cleanup-on-failure
    (except BaseException: unlink the staged tmp file) exactly like a copy2/chmod failure does --
    dest_dir is left exactly as if the install never started."""
    archive = _tar_gz_with_uv(tmp_path)
    dest_dir = tmp_path / "dest"

    def _boom(path):
        raise OSError("simulated: fsync EIO")

    monkeypatch.setattr(uvman, "_fsync_path", _boom)

    with pytest.raises(OSError, match="simulated: fsync EIO"):
        uvman._extract_uv(archive, dest_dir)

    assert not (dest_dir / EXE).exists()
    assert list(dest_dir.iterdir()) == []  # the staged tmp file was still cleaned up


def test_extract_uv_skips_dir_fsync_on_windows(monkeypatch, tmp_path) -> None:
    """Directories can't be opened via os.open on Windows, so the best-effort directory fsync must
    not even be attempted there — only the staged-file fsync (which works fine on Windows) runs."""
    archive = _tar_gz_with_uv(tmp_path, exe_name="uv.exe")
    dest_dir = tmp_path / "dest"
    monkeypatch.setattr("sys.platform", "win32")

    fsynced: list[Path] = []
    real_fsync_path = uvman._fsync_path

    def _spy(path: Path) -> None:
        fsynced.append(path)
        real_fsync_path(path)

    monkeypatch.setattr(uvman, "_fsync_path", _spy)

    dest = uvman._extract_uv(archive, dest_dir)
    assert dest == dest_dir / "uv.exe"
    assert len(fsynced) == 1  # only the staged file was fsync'd, never the directory
    assert fsynced[0].parent == dest_dir
    assert dest_dir not in fsynced


def test_ensure_uv_downloaded_atomic_install_self_heals(monkeypatch, tmp_path) -> None:
    """End-to-end (mocked network): an interrupted install must not poison the private bin dir —
    ensure_uv_downloaded raises but leaves no binary behind — and the *next* call re-downloads and
    installs successfully, exactly the self-healing behavior the atomic install is meant to give."""
    archive = _tar_gz_with_uv(tmp_path, content=b"end-to-end-uv-bytes")
    data = archive.read_bytes()
    triple = "x86_64-unknown-linux-gnu"
    bin_dir = tmp_path / "bin"

    monkeypatch.setattr(uvman, "_triple", lambda: triple)
    monkeypatch.setattr(uvman, "_UV_SHA256", {triple: hashlib.sha256(data).hexdigest()})
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: bin_dir)
    monkeypatch.setattr(uvman, "_ask_consent", lambda _: True)
    monkeypatch.setattr("urllib.request.urlopen", _fake_urlopen(data))

    real_copy2 = uvman.shutil.copy2
    calls = {"n": 0}

    def _flaky_copy2(src, dst, *a, **kw):
        calls["n"] += 1
        if calls["n"] == 1:
            Path(dst).write_bytes(b"\x7f\x00partial-garbage")  # simulate a torn write
            raise OSError("simulated ENOSPC mid-copy")
        return real_copy2(src, dst, *a, **kw)

    monkeypatch.setattr(uvman.shutil, "copy2", _flaky_copy2)

    with pytest.raises(uvman.UvDownloadError):
        uvman.ensure_uv_downloaded(quiet=True)
    assert not (bin_dir / EXE).exists()
    assert calls["n"] == 1

    # Second attempt (network re-mocked the same way): the corrupt-cache bug would have short
    # circuited here via dest.exists(); the fix means dest is absent, so this re-downloads.
    result = uvman.ensure_uv_downloaded(quiet=True)
    assert result == str(bin_dir / EXE)
    assert (bin_dir / EXE).read_bytes() == b"end-to-end-uv-bytes"
    assert calls["n"] == 2


def test_checksum_fail_closed_when_triple_unpinned(monkeypatch, tmp_path) -> None:
    """If the platform triple has no pinned hash, refuse rather than run an unverified binary:
    raise UvDownloadError (not the generic wrapper) and never extract."""
    triple = "x86_64-unknown-linux-gnu"
    monkeypatch.setattr(uvman, "_triple", lambda: triple)
    monkeypatch.setattr(uvman, "_UV_SHA256", {})  # no pin for anything
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: tmp_path / "bin")
    monkeypatch.setattr("urllib.request.urlopen", _fake_urlopen(b"whatever"))

    extracted = {"called": False}
    monkeypatch.setattr(uvman, "_extract_uv", lambda *a: extracted.__setitem__("called", True))

    with pytest.raises(uvman.UvDownloadError) as exc_info:
        uvman.ensure_uv_downloaded(quiet=True)

    msg = str(exc_info.value)
    assert triple in msg
    assert "Failed to download" not in msg
    assert extracted["called"] is False  # never extract an unverifiable binary
