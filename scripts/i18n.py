"""Maintainer i18n workflow (Babel — dev-only; the runtime uses stdlib gettext).

Message catalogs use source-string msgids: the English text in gettext()/ngettext() calls is the
msgid, so English needs no catalog. Translations live in src/skit/locales/<locale>/LC_MESSAGES/skit.po
and are compiled to .mo (both committed; .mo is what ships and what tests load).

Usage (through uv, so Babel is on hand):
  uv run python scripts/i18n.py extract        # refresh locales/skit.pot from the source strings
  uv run python scripts/i18n.py update         # merge the .pot into every locale's .po
  uv run python scripts/i18n.py compile        # compile .po -> .mo (run after editing any .po)
  uv run python scripts/i18n.py add <locale>   # scaffold a new locale, e.g. `add ja` or `add fr`
  uv run python scripts/i18n.py coverage       # gate: .pot fresh, every locale 100%, nothing unwrapped

Adding UI strings:   extract -> update -> translate the new msgids in each .po -> compile
Adding a language:   add <locale> -> translate -> compile
"""

from __future__ import annotations

import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCALES = ROOT / "src" / "skit" / "locales"
POT = LOCALES / "skit.pot"
DOMAIN = "skit"


def _pybabel(*args: str) -> int:
    return subprocess.run(
        [sys.executable, "-m", "babel.messages.frontend", *args], check=False, cwd=ROOT
    ).returncode


def _project_version() -> str:
    with (ROOT / "pyproject.toml").open("rb") as f:
        return tomllib.load(f)["project"]["version"]


def extract() -> int:
    return _pybabel(
        "extract",
        "-F",
        "babel.cfg",
        "--project",
        "skit",
        "--version",
        _project_version(),
        "--sort-by-file",
        "--no-wrap",
        "-o",
        str(POT),
        "src/skit",
    )


def update() -> int:
    return _pybabel("update", "-i", str(POT), "-d", str(LOCALES), "-D", DOMAIN, "--no-wrap")


def compile_() -> int:
    return _pybabel("compile", "-d", str(LOCALES), "-D", DOMAIN)


def add(locale: str) -> int:
    return _pybabel(
        "init", "-i", str(POT), "-d", str(LOCALES), "-D", DOMAIN, "-l", locale, "--no-wrap"
    )


def main(argv: list[str]) -> int:
    if not argv:
        print(__doc__)
        return 2
    cmd, *rest = argv
    if cmd == "extract":
        return extract()
    if cmd == "update":
        return update()
    if cmd == "compile":
        return compile_()
    if cmd == "add" and rest:
        return add(rest[0])
    if cmd == "coverage":
        from i18n_coverage import main as coverage_main

        return coverage_main()
    print(__doc__)
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
