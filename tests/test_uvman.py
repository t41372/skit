"""uvman tests: download consent (no network) + pinned-version URL liveness (opt-in network).

When bumping UV_VERSION, set SKIT_NET_TESTS=1 and run this file to confirm the pinned
version's download URLs are live on all three platforms before committing.
"""

from __future__ import annotations

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
]


@net
@pytest.mark.parametrize("triple", TRIPLES)
def test_pinned_uv_release_exists(triple: str) -> None:
    url = uvman.download_url(triple)
    req = urllib.request.Request(url, method="HEAD")
    with urllib.request.urlopen(req, timeout=30) as resp:
        assert resp.status == 200


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
