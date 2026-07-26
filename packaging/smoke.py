"""Smoke-test a frozen skit binary: `python packaging/smoke.py dist/skit`.

Run against every release binary before it ships. Frozen-app failure modes are silent by
design elsewhere in skit — a mis-bundled tree-sitter grammar degrades analyzers to None
instead of crashing, and a missing wheel metadata falls back to version "0.0.0+unknown" —
so each check here asserts the POSITIVE outcome, never just "it didn't crash".

Stdlib-only and cross-platform: the pty-based TUI probe and the bash payload run are
skipped on Windows, everything else runs everywhere. State is isolated through the
SKIT_*_DIR env vars (the same contract the test suite uses), so a run never touches the
invoking user's real library.
"""

from __future__ import annotations

import io
import json
import os
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

CJK = tuple(chr(c) for c in range(0x4E00, 0x4E00 + 40))  # spot-check range is plenty
FAILURES: list[str] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    print(f"{'PASS' if ok else 'FAIL'}: {name}" + (f" — {detail}" if detail and not ok else ""))
    if not ok:
        FAILURES.append(name)


def run(
    binary: str, args: list[str], env: dict[str, str], timeout: int = 60
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(  # noqa: S603 — fixed argv against the binary under test
        [binary, *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        env=env,
        timeout=timeout,
        check=False,
    )


def probe_tui(binary: str, env: dict[str, str]) -> None:
    """POSIX only: launch the real TUI on a pty, expect the alternate screen, quit with Esc."""
    if sys.platform == "win32":
        print("SKIP: TUI pty probe (no pty on Windows)")
        return
    import pty
    import select

    pid, fd = pty.fork()  # ty: ignore[possibly-missing-attribute] — POSIX-only, guarded above
    if pid == 0:  # child: become the binary under test
        os.environ.update(env)
        os.environ["TERM"] = "xterm-256color"
        os.execv(binary, [binary])
    out = b""
    sent = False
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.5)
        if ready:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
        elif out and not sent:
            time.sleep(1.5)  # let the first paint settle before quitting
            os.write(fd, b"\x1b")  # Esc on the library table = quit
            sent = True
    os.close(fd)
    os.waitpid(pid, 0)
    text = out.decode("utf-8", "replace")
    check("tui enters the alternate screen", "\x1b[?1049h" in text)
    check(
        "tui exits without a traceback",
        "Traceback" not in text and "ModuleNotFoundError" not in text,
        text[-800:],
    )


def main() -> int:
    # FAIL details echo raw child output (CJK, box drawing); a cp1252 console must never
    # be able to crash the reporter itself mid-report.
    for stream in (sys.stdout, sys.stderr):
        if isinstance(stream, io.TextIOWrapper):
            stream.reconfigure(encoding="utf-8", errors="replace")
    binary = str(Path(sys.argv[1]).resolve())
    work = Path(tempfile.mkdtemp(prefix="skit-smoke-"))
    env: dict[str, str] = {
        **os.environ,
        "SKIT_CONFIG_DIR": str(work / "config"),
        "SKIT_DATA_DIR": str(work / "data"),
        "SKIT_STATE_DIR": str(work / "state"),
        "NO_COLOR": "1",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
    }
    # Any loader-path var already in the invoking environment would make the child-env
    # pollution check ambiguous; the smoke contract is a clean slate.
    for var in ("LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH"):
        env.pop(var, None)
    size_mb = Path(binary).stat().st_size / 1e6
    print(f"binary: {binary} ({size_mb:.1f} MB)")

    out = run(binary, ["--version"], env)
    check(
        "--version reports the real wheel version",
        out.returncode == 0 and "skit" in out.stdout and "0.0.0+unknown" not in out.stdout,
        out.stdout + out.stderr,
    )

    out = run(binary, ["config", "lang", "zh-CN"], env)
    check("config lang zh-CN accepted", out.returncode == 0, out.stderr)
    out = run(binary, ["--help"], env)
    check(
        "zh help renders CJK (gettext .mo catalogs bundled)",
        any(ch in out.stdout for ch in CJK),
        out.stdout[:400],
    )
    run(binary, ["config", "lang", "en"], env)

    # tree-sitter grammars: analysis is static, so this works on every platform even
    # where bash isn't installed. Silent degradation is exactly what this catches.
    script = work / "hello.sh"
    script.write_text(
        '#!/usr/bin/env bash\nNAME=${NAME:-world}\necho "hi $NAME"\n', encoding="utf-8"
    )
    out = run(binary, ["add", str(script), "--no-input"], env)
    check("add a shell script", out.returncode == 0, out.stdout + out.stderr)
    out = run(binary, ["params", "hello", "--json"], env)
    check(
        "shell analyzer alive (tree-sitter grammar bundled)",
        '"NAME"' in out.stdout,
        out.stdout + out.stderr,
    )

    # Child processes must not inherit the frozen bundle's loader path (childenv scrub):
    # dump the child's environment through a command entry and look for bundle markers.
    # The smoke env itself carries none of these vars (scrubbed below), so ANY sighting
    # in the child is pollution introduced by the binary.
    dump = "set" if sys.platform == "win32" else "env"
    run(binary, ["add", "--cmd", dump, "-n", "envdump"], env)
    out = run(binary, ["run", "envdump", "--no-input"], env)

    def polluted(line: str) -> bool:
        name, _, value = line.partition("=")
        if name.startswith("_PYI_"):
            return True
        if name in ("LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH"):
            return True
        return name == "PATH" and "_MEI" in value

    bad = [line for line in out.stdout.splitlines() if polluted(line)]
    check(
        "child env carries no frozen loader paths",
        out.returncode == 0 and not bad,
        "\n".join(bad) or out.stderr,
    )

    if sys.platform != "win32":
        out = run(binary, ["run", "hello", "--no-input"], env)
        check(
            "run a shell entry end-to-end",
            out.returncode == 0 and "hi world" in out.stdout,
            out.stdout + out.stderr,
        )
    run(binary, ["add", "--cmd", "exit 7", "-n", "exit7"], env)
    out = run(binary, ["run", "exit7", "--no-input"], env)
    check("script exit code passes through untouched", out.returncode == 7, str(out.returncode))

    out = run(binary, ["agent", "install", "--to", str(work / "skills")], env)
    check(
        "agent skill installs (importlib.resources works frozen)",
        out.returncode == 0 and (work / "skills" / "skit" / "SKILL.md").is_file(),
        out.stdout + out.stderr,
    )

    out = run(binary, ["list", "--json"], env)
    try:
        names = {e.get("name") for e in json.loads(out.stdout)}
    except json.JSONDecodeError:
        names = set()
    check("list --json is valid and complete", {"hello", "envdump", "exit7"} <= names, out.stdout)

    probe_tui(binary, env)

    timings = []
    for _ in range(5):
        t0 = time.monotonic()
        run(binary, ["--version"], env)
        timings.append((time.monotonic() - t0) * 1000)
    print(f"startup --version median: {statistics.median(timings):.0f} ms ({size_mb:.1f} MB)")

    if FAILURES:
        print(f"\nsmoke FAILED: {len(FAILURES)} check(s): {', '.join(FAILURES)}")
        return 1
    print("\nsmoke OK: all checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
