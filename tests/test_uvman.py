"""uvman tests: download consent (no network) + pinned-version URL liveness (opt-in network).

When bumping UV_VERSION, set SKIT_NET_TESTS=1 and run this file to confirm the pinned
version's download URLs are live on all three platforms before committing.
"""

from __future__ import annotations

import hashlib
import io
import os
import urllib.request
from pathlib import Path

import pytest

from skit import uvman

net = pytest.mark.skipif(
    not os.environ.get("SKIT_NET_TESTS"),
    reason="network liveness test; set SKIT_NET_TESTS=1 when bumping UV_VERSION",
)

TRIPLES = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
]


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
    monkeypatch.setattr("platform.machine", lambda: "mips")
    monkeypatch.setattr("sys.platform", "linux")
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
    assert uvman._triple() == "aarch64-unknown-linux-gnu"


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
    exe = bin_dir / "uv"
    exe.touch()
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: bin_dir)
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
    {x86_64, aarch64} x {apple-darwin, unknown-linux-gnu, pc-windows-msvc} — so no reachable
    platform is left without a hash to verify against."""
    produced: set[str] = set()
    for machine in ("x86_64", "arm64"):
        for plat in ("darwin", "win32", "linux"):
            monkeypatch.setattr("platform.machine", lambda m=machine: m)
            monkeypatch.setattr("sys.platform", plat)
            produced.add(uvman._triple())
    assert len(produced) == 6
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
        return dest_dir / "uv"

    monkeypatch.setattr(uvman, "_extract_uv", _fake_extract)

    result = uvman.ensure_uv_downloaded(quiet=True)
    assert seen, "extraction was not reached — the checksum gate did not pass"
    assert result == str(tmp_path / "bin" / "uv")
    assert seen["archive_bytes"] == data


def test_checksum_mismatch_raises_checksum_error_not_generic(monkeypatch, tmp_path) -> None:
    """A tampered/corrupt archive (hash != pinned) fails closed with the checksum message — NOT the
    generic 'Failed to download' wrapper — and extraction is never reached."""
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
