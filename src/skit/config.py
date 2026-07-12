"""User configuration (config.toml) and mirror settings.

skit keeps its own settings in its config dir (`config.toml`, next to the i18n `language` key) and
**never** writes to global state (`~/.config/uv/`, the shell). Mirror settings are applied only by
overlaying environment variables onto the `uv` child processes skit spawns — and only when the user
hasn't already set the corresponding variable themselves (their explicit env always wins).

Three GFW-facing download vectors are covered:
- `pypi`           -> `UV_DEFAULT_INDEX`          (PEP 723 script deps resolved by `uv run`)
- `python_install` -> `UV_PYTHON_INSTALL_MIRROR`  (CPython fetched by uv for a script's requires-python)
- `uv_binary`      -> skit's own uv bootstrap download (see uvman)
"""

from __future__ import annotations

import socket
import sys
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass, replace
from typing import Any

from .atomic import atomic_write_toml, load_toml_recoverable
from .i18n import gettext
from .paths import config_dir

# NJU mirrors GitHub release assets; uv/skit just swap the github.com download prefix for these.
_GITHUB_RELEASE = "https://mirror.nju.edu.cn/github-release"
PYTHON_INSTALL_MIRROR = f"{_GITHUB_RELEASE}/astral-sh/python-build-standalone/"
UV_BINARY_MIRROR = f"{_GITHUB_RELEASE}/astral-sh/uv"

# PyPI index presets (the part users pick between; the GitHub mirrors above are shared).
PYPI_PRESETS: dict[str, str] = {
    "tsinghua": "https://pypi.tuna.tsinghua.edu.cn/simple",
    "aliyun": "https://mirrors.aliyun.com/pypi/simple",
    "ustc": "https://pypi.mirrors.ustc.edu.cn/simple",
}

# uv env vars that REPLACE uv's default index — if the user set one of these (to a non-empty value)
# they've already chosen their PyPI vector, so skit defers. UV_INDEX / UV_EXTRA_INDEX_URL are
# deliberately excluded: they're *additive* (they don't replace the default index), so deferring on
# them would leave the GFW-blocked default index in place.
_INDEX_ENV = ("UV_DEFAULT_INDEX", "UV_INDEX_URL")
_PYTHON_MIRROR_ENV = "UV_PYTHON_INSTALL_MIRROR"


def _config_path():
    return config_dir() / "config.toml"


def load_config() -> dict[str, Any]:
    path = _config_path()
    if not path.is_file():
        return {}
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        return {}


def save_config(doc: Mapping[str, Any]) -> None:
    atomic_write_toml(_config_path(), dict(doc))


def _load_config_for_save() -> dict[str, Any]:
    """Like load_config(), but for the read-modify-write savers (save_editor / save_mirror).

    load_config() treats a present-but-corrupt config.toml the same as an absent one (returns {}),
    which is fine for read-only callers. But a saver that started from that {} would then overwrite
    the file with just the one key it just set — silently destroying every other saved setting with
    no trace, contradicting the "preserving every other key" docstring promise. So here: if the file
    exists but fails to parse, back it up first (config.toml.bak) and warn on stderr, so the save can
    still proceed but nothing is lost without a trace.

    The backup mechanics (is-file / try-parse / copy2-on-failure) live in atomic.load_toml_recoverable
    so i18n.set_language's own read-modify-write can share them: i18n can't import this module (config
    already imports gettext from i18n, so the reverse would be an import cycle), but both safely
    import the neutral atomic module.
    """
    path = _config_path()
    recovery = load_toml_recoverable(path)
    if recovery.corrupt:
        if recovery.backup_path is not None:
            print(
                gettext(
                    "%(path)s is corrupt and could not be parsed. It has been backed up to "
                    "%(backup)s before this change; recover any lost settings from that file."
                )
                % {"path": str(path), "backup": str(recovery.backup_path)},
                file=sys.stderr,
            )
        else:
            print(
                gettext(
                    "%(path)s is corrupt and could not be parsed, and it could not be backed up "
                    "either; the settings it contained will be lost when this change is saved."
                )
                % {"path": str(path)},
                file=sys.stderr,
            )
    return recovery.doc


def is_configured() -> bool:
    """True once the user has been through setup (config.toml exists), so first-run setup runs once."""
    return _config_path().is_file()


def mirror_configured() -> bool:
    """True once a [mirror] section has been written — the marker that the first-run mirror offer
    has already happened. Distinct from is_configured(): setting a language also writes config.toml,
    but must NOT suppress the mirror offer (that's what would happen if the gate keyed on the file's
    mere existence)."""
    return "mirror" in load_config()


def load_editor() -> str:
    """The user's configured editor command (config.toml `editor`), or "" when unset. editor.py
    falls back to $VISUAL / $EDITOR / a platform default when this is empty."""
    value = load_config().get("editor", "")  # pragma: no mutate — default guarded by isinstance
    return value if isinstance(value, str) else ""


def save_editor(command: str) -> None:
    """Persist (or clear, when empty) the editor command, preserving every other key."""
    doc = _load_config_for_save()
    if command.strip():
        doc["editor"] = command.strip()
    else:
        doc.pop("editor", None)
    save_config(doc)


FORM_STYLES = ("tui", "plain")


def load_form() -> str:
    """Interactive-form style: "tui" (inline mini-form / review panel, the default) or
    "plain" (line prompts). Consumed by the CLI's interactive flows — `skit run`
    parameter collection and `skit add`'s review panel; the TUI workbench always uses
    its own full screens."""
    value = load_config().get("form", "")  # pragma: no mutate — normalized below
    return value if value in FORM_STYLES else "tui"


def save_form(style: str) -> None:
    """Persist the form style, preserving every other key."""
    doc = _load_config_for_save()
    doc["form"] = style
    save_config(doc)


AFTER_RUN_MODES = ("exit", "stay")


def load_after_run() -> str:
    """What the TUI does when a launched script finishes: "exit" (the default — skit is
    a launcher, so it quits and passes the script's exit code through) or "stay"
    (banner + Enter returns to the Library, the workbench loop)."""
    value = load_config().get("after_run", "")  # pragma: no mutate — normalized below
    return value if value in AFTER_RUN_MODES else "exit"


def save_after_run(mode: str) -> None:
    """Persist the after-run behavior, preserving every other key."""
    doc = _load_config_for_save()
    doc["after_run"] = mode
    save_config(doc)


@dataclass(frozen=True)
class MirrorConfig:
    enabled: bool = False
    pypi: str = ""
    python_install: str = ""
    uv_binary: str = ""


def load_mirror() -> MirrorConfig:
    section = load_config().get(
        "mirror", {}
    )  # pragma: no mutate — isinstance check below normalizes any default
    if not isinstance(section, dict):
        return MirrorConfig()

    def _url(key: str) -> str:
        # Type-harden a hand-edited config: only a real string is a URL; anything else (int, bool,
        # ...) is treated as blank rather than str()-coerced into a bogus value like "123".
        value = section.get(
            key, ""
        )  # pragma: no mutate — isinstance check below normalizes any default
        return value if isinstance(value, str) else ""

    def _https_url(key: str) -> str:
        # uv_binary names an executable skit downloads, chmod +x's, and runs, so it MUST be https —
        # a plain-http mirror would let a MITM swap in a trojaned uv. The wizard rejects non-https
        # interactively; a hand-edited non-https value is silently blanked here so uv_binary_base()
        # falls back to the GitHub default (checksum verification in uvman is the further backstop).
        url = _url(key)
        return url if url.startswith("https://") else ""

    return MirrorConfig(
        # Require a genuine bool: a stray `enabled = "false"` would otherwise be truthy and silently
        # invert the user's intent (bool("false") is True).
        enabled=section.get("enabled") is True,
        pypi=_url("pypi"),
        python_install=_url("python_install"),
        uv_binary=_https_url("uv_binary"),
    )


def save_mirror(mirror: MirrorConfig) -> None:
    """Persist the [mirror] section, preserving every other key (e.g. language)."""
    doc = _load_config_for_save()
    doc["mirror"] = {
        "enabled": mirror.enabled,
        "pypi": mirror.pypi,
        "python_install": mirror.python_install,
        "uv_binary": mirror.uv_binary,
    }
    save_config(doc)


def preset(name: str) -> MirrorConfig:
    """Build an enabled MirrorConfig for a PyPI provider, sharing the NJU GitHub-release mirrors."""
    return MirrorConfig(
        enabled=True,
        pypi=PYPI_PRESETS[name],
        python_install=PYTHON_INSTALL_MIRROR,
        uv_binary=UV_BINARY_MIRROR,
    )


def disable() -> None:
    """Turn mirrors off (e.g. travelling abroad) without discarding the saved URLs."""
    save_mirror(replace(load_mirror(), enabled=False))


def mirror_env(base_env: Mapping[str, str]) -> dict[str, str]:
    """The env overlay to inject into uv child processes.

    Returns only the variables the user has NOT already set themselves (their env wins — the "defer"
    rule). Empty when mirrors are disabled.
    """
    mirror = load_mirror()
    if not mirror.enabled:
        return {}
    overlay: dict[str, str] = {}
    # Defer on a *truthy* user value only: an empty `UV_INDEX_URL=""` means "unset", so it must not
    # suppress the mirror (presence-based defer would wrongly leave the blocked default in place).
    if mirror.pypi and not any(base_env.get(v) for v in _INDEX_ENV):
        overlay["UV_DEFAULT_INDEX"] = mirror.pypi
    if mirror.python_install and not base_env.get(_PYTHON_MIRROR_ENV):
        overlay[_PYTHON_MIRROR_ENV] = mirror.python_install
    return overlay


def uv_binary_base() -> str:
    """The base URL for skit's own uv-binary bootstrap download, or "" to use the GitHub default."""
    mirror = load_mirror()
    return mirror.uv_binary if mirror.enabled else ""


def looks_blocked(timeout: float = 2.5) -> bool:
    """Heuristic for "is this network likely behind the GFW?" — True if PyPI or GitHub can't be
    reached within `timeout` seconds. Used only to *offer* mirror setup on first run; never decides
    anything on its own."""
    for host in ("pypi.org", "github.com"):
        try:
            with socket.create_connection((host, 443), timeout=timeout):
                pass
        except OSError:
            return True
    return False


def load_bash_path() -> str:
    """Windows escape hatch: an explicit bash to run shell entries with (config.toml
    `[shell] bash_path`). Empty when unset; POSIX systems never need it."""
    section = load_config().get("shell", {})  # pragma: no mutate — isinstance normalizes
    value = section.get("bash_path", "") if isinstance(section, dict) else ""
    return value if isinstance(value, str) else ""


def save_bash_path(path: str) -> None:
    """Persist (or clear, when empty) the Windows bash path, preserving every other key."""
    doc = _load_config_for_save()
    section = doc.get("shell")
    if not isinstance(section, dict):
        section = {}
    if path.strip():
        section["bash_path"] = path.strip()
    else:
        section.pop("bash_path", None)
    if section:
        doc["shell"] = section
    else:
        doc.pop("shell", None)
    save_config(doc)


_JS_RUNNERS = ("deno", "bun", "node")


def load_js_runner() -> str:
    """The preferred JS/TS runner (config.toml `[js] runner`), or "" for the built-in
    detection order (deno > bun > node). Unknown values normalize to "" — a hand-edited
    `runner = "carrier-pigeon"` must not poison every js run."""
    section = load_config().get("js", {})  # pragma: no mutate — isinstance normalizes
    value = section.get("runner", "") if isinstance(section, dict) else ""
    return value if value in _JS_RUNNERS else ""


def save_js_runner(name: str) -> None:
    """Persist (or clear, when empty) the preferred JS runner, preserving every other key."""
    doc = _load_config_for_save()
    section = doc.get("js")
    if not isinstance(section, dict):
        section = {}
    if name.strip():
        section["runner"] = name.strip()
    else:
        section.pop("runner", None)
    if section:
        doc["js"] = section
    else:
        doc.pop("js", None)
    save_config(doc)
