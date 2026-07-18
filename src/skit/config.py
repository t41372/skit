"""User configuration (config.toml) and mirror settings.

skit keeps its own settings in its config dir (`config.toml`, next to the i18n `language` key) and
**never** writes to global state (`~/.config/uv/`, the shell). Mirror settings are applied only by
overlaying environment variables onto the `uv` child processes skit spawns — and only when the user
hasn't already set the corresponding variable themselves (their explicit env always wins).

Four GFW-facing download vectors are covered, grouped into three *independent* axes — each
ecosystem has its own mirror-vendor landscape, so no single vendor choice may span them:
- pypi axis:   `pypi`           -> `UV_DEFAULT_INDEX`          (PEP 723 script deps resolved by `uv run`)
- github axis: `python_install` -> `UV_PYTHON_INSTALL_MIRROR`  (CPython fetched by uv for a script's requires-python)
               `uv_binary`      -> skit's own uv bootstrap download (see uvman)
- npm axis:    `npm`            -> `NPM_CONFIG_REGISTRY`       (js/ts per-script deps, see langs/javascript/deps)
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

# Each axis has its own preset table because each ecosystem has its own mirror vendors: the
# PyPI providers don't run npm registries, the npm mirror (npmmirror, Alibaba's successor to
# the taobao registry) isn't a PyPI vendor, and github-release mirroring is a third, separate
# service class. New vendors join the axis they actually serve — never another axis's table.

# PyPI index presets (Python deps resolved by `uv run`).
PYPI_PRESETS: dict[str, str] = {
    "tsinghua": "https://pypi.tuna.tsinghua.edu.cn/simple",
    "aliyun": "https://mirrors.aliyun.com/pypi/simple",
    "ustc": "https://pypi.mirrors.ustc.edu.cn/simple",
}

# GitHub-release mirror presets: a base prefix that swaps for github.com's download prefix.
# One choice covers both github-release vectors (Python builds and the uv binary) because
# they are the same service, not because "everything follows the PyPI vendor".
GITHUB_RELEASE_PRESETS: dict[str, str] = {
    "nju": "https://mirror.nju.edu.cn/github-release",
}

# npm registry presets (js/ts per-script deps).
NPM_PRESETS: dict[str, str] = {
    "npmmirror": "https://registry.npmmirror.com",
}


def github_release_urls(base: str) -> tuple[str, str]:
    """The concrete (python_install, uv_binary) URLs a github-release mirror base expands to."""
    base = base.removesuffix("/")
    return (f"{base}/astral-sh/python-build-standalone/", f"{base}/astral-sh/uv")


def is_url_token(value: str) -> bool:
    """A pastable one-token http(s) URL: has a scheme, no whitespace, no "·" (the display
    separator). THE shared gate for every custom-URL entrance — CLI, TUI, and wizard must
    reject the same garbage, or a value one door refuses walks in through another and
    surfaces later as a mysteriously broken `UV_DEFAULT_INDEX`."""
    return (
        value.startswith(("https://", "http://"))
        and not any(ch.isspace() for ch in value)
        and "·" not in value
    )


# The recommended per-axis defaults (first-run wizard prompts, custom-URL placeholders).
PYTHON_INSTALL_MIRROR, UV_BINARY_MIRROR = github_release_urls(GITHUB_RELEASE_PRESETS["nju"])
NPM_REGISTRY_MIRROR = NPM_PRESETS["npmmirror"]

# uv env vars that REPLACE uv's default index — if the user set one of these (to a non-empty value)
# they've already chosen their PyPI vector, so skit defers. UV_INDEX / UV_EXTRA_INDEX_URL are
# deliberately excluded: they're *additive* (they don't replace the default index), so deferring on
# them would leave the GFW-blocked default index in place.
_INDEX_ENV = ("UV_DEFAULT_INDEX", "UV_INDEX_URL")
_PYTHON_MIRROR_ENV = "UV_PYTHON_INSTALL_MIRROR"
_NPM_REGISTRY_ENV = ("NPM_CONFIG_REGISTRY", "npm_config_registry")


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
    npm: str = ""  # npm registry (js/ts per-script deps + anything npm-ish the script spawns)


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
        npm=_url("npm"),
    )


def save_mirror(mirror: MirrorConfig) -> None:
    """Persist the [mirror] section, preserving every other key (e.g. language)."""
    doc = _load_config_for_save()
    doc["mirror"] = {
        "enabled": mirror.enabled,
        "pypi": mirror.pypi,
        "python_install": mirror.python_install,
        "uv_binary": mirror.uv_binary,
        "npm": mirror.npm,
    }
    save_config(doc)


def compose(
    *, pypi: str = "", python_install: str = "", uv_binary: str = "", npm: str = ""
) -> MirrorConfig:
    """Build a MirrorConfig from per-axis URLs ("" = that axis off); enabled iff any axis is on.

    This is the only constructor the UI surfaces go through: each axis (pypi / github-release /
    npm) is chosen independently — there is deliberately no "vendor" that spans axes, because
    the mirror-provider landscape of each ecosystem is its own.
    """
    return MirrorConfig(
        enabled=bool(pypi or python_install or uv_binary or npm),
        pypi=pypi,
        python_install=python_install,
        uv_binary=uv_binary,
        npm=npm,
    )


def pypi_choice(m: MirrorConfig) -> str:
    """How the STORED PyPI axis reads: a PYPI_PRESETS name, "custom", or "off".

    Deliberately blind to the master switch: the stored config has three states
    (on / paused-with-URLs / empty), and the per-axis readers report the stored axis so
    a paused config stays *visible* — whether it is applied is the master's business
    (`m.enabled`, folded in by mirror_env / mirrors_line, never hidden here).
    """
    if not m.pypi:
        return "off"
    return next((k for k, v in PYPI_PRESETS.items() if v == m.pypi), "custom")


def github_choice(m: MirrorConfig) -> str:
    """How the STORED github-release axis (Python builds + the uv binary) reads."""
    if not (m.python_install or m.uv_binary):
        return "off"
    urls = (m.python_install, m.uv_binary)
    return next(
        (k for k, base in GITHUB_RELEASE_PRESETS.items() if github_release_urls(base) == urls),
        "custom",
    )


def npm_choice(m: MirrorConfig) -> str:
    """How the STORED npm axis reads: an NPM_PRESETS name, "custom", or "off"."""
    if not m.npm:
        return "off"
    return next((k for k, v in NPM_PRESETS.items() if v == m.npm), "custom")


def github_base(m: MirrorConfig) -> str:
    """The single base prefix the stored github pair derives from, or "" when the pair
    was hand-edited into URLs no base expands to (skit's own UIs only ever write
    base-derived pairs).

    Recovering the base from the uv-binary suffix alone is sufficient: a pair is
    base-derivable only when BOTH vectors share that base, and the round-trip equality
    below pins the python-install half — so a second, python-install-derived candidate
    would be redundant (it can only ever agree with this one or fail alongside it)."""
    base = m.uv_binary.removesuffix("/astral-sh/uv")
    if base and github_release_urls(base) == (m.python_install, m.uv_binary):
        return base
    return ""


def pypi_display(m: MirrorConfig) -> str:
    """The stored PyPI axis for display: its preset name, "off", or (custom) the URL."""
    choice = pypi_choice(m)
    return m.pypi if choice == "custom" else choice


def github_display(m: MirrorConfig) -> str:
    """The stored github-release axis for display: preset name, "off", the custom base
    URL, or — only for a hand-edited underivable pair — both URLs joined with " + "
    (never " · ", which is the axes_summary separator and must stay unambiguous)."""
    choice = github_choice(m)
    if choice != "custom":
        return choice
    base = github_base(m)
    if base:
        return base
    return f"{m.python_install or 'off'} + {m.uv_binary or 'off'}"


def npm_display(m: MirrorConfig) -> str:
    """The stored npm axis for display: its preset name, "off", or (custom) the URL."""
    choice = npm_choice(m)
    return m.npm if choice == "custom" else choice


def axes_summary(m: MirrorConfig) -> str:
    """The STORED per-axis state as data tokens (`pypi=… · github=… · npm=…`), or "off"
    when nothing is saved. Blind to the master switch by design — mirrors_line() is the
    one place that folds it in, so a paused config never becomes invisible."""
    if all(c == "off" for c in (pypi_choice(m), github_choice(m), npm_choice(m))):
        return "off"
    return f"pypi={pypi_display(m)} · github={github_display(m)} · npm={npm_display(m)}"


def mirrors_line(m: MirrorConfig) -> str:
    """The one-line human mirror status (doctor, TUI health): stored axes + the master
    switch, three states honestly told apart — a paused config shows what `mirror on`
    would bring back instead of masquerading as unconfigured."""
    body = axes_summary(m)
    if body == "off":
        return gettext("Mirrors: off")
    if m.enabled:
        return gettext("Mirrors: %(axes)s") % {"axes": body}
    return gettext("Mirrors: off (saved: %(axes)s)") % {"axes": body}


def update_mirror_axes(
    *,
    pypi: str | None = None,
    python_install: str | None = None,
    uv_binary: str | None = None,
    npm: str | None = None,
) -> MirrorConfig:
    """Edit the STORED per-axis URLs and save (None = leave that URL alone, "" = clear
    it). Returns what was saved.

    A writer never destroys state it wasn't asked to change, so the other axes' URLs
    always survive, and the master switch follows three rules:
    - already on: stays on while any URL remains (clearing the last one turns it off);
    - fresh (off, nothing saved): a first URL turns it on — one-command setup;
    - paused (off with URLs saved, i.e. `mirror off`): stays paused — silently flipping
      the master would resurrect every other saved axis behind the user's back, so the
      caller *tells* the user how to re-enable instead of skit guessing.
    """
    m = load_mirror()
    paused = not m.enabled and any((m.pypi, m.python_install, m.uv_binary, m.npm))
    updates = {
        key: value
        for key, value in {
            "pypi": pypi,
            "python_install": python_install,
            "uv_binary": uv_binary,
            "npm": npm,
        }.items()
        if value is not None
    }
    updated = replace(m, **updates)
    any_urls = any((updated.pypi, updated.python_install, updated.uv_binary, updated.npm))
    updated = replace(updated, enabled=any_urls if m.enabled else (any_urls and not paused))
    save_mirror(updated)
    return updated


def disable() -> None:
    """Turn mirrors off (e.g. travelling abroad) without discarding the saved URLs."""
    save_mirror(replace(load_mirror(), enabled=False))


def enable() -> bool:
    """Re-enable the saved mirror URLs (the return trip after disable()).

    False when no URLs are saved — there is nothing to enable, and pretending otherwise
    would report an "on" state that overlays no environment at all.
    """
    m = load_mirror()
    if not (m.pypi or m.python_install or m.uv_binary or m.npm):
        return False
    save_mirror(replace(m, enabled=True))
    return True


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
    # npm reads its config env vars case-insensitively; bun and deno read the uppercase form.
    # Setting the uppercase one reaches all three installers (and the script's own npm calls).
    if mirror.npm and not any(base_env.get(v) for v in _NPM_REGISTRY_ENV):
        overlay["NPM_CONFIG_REGISTRY"] = mirror.npm
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


JS_RUNNERS = ("deno", "bun", "node")


def load_js_runner() -> str:
    """The preferred JS/TS runner (config.toml `[js] runner`), or "" for the built-in
    detection order (deno > bun > node). Unknown values normalize to "" — a hand-edited
    `runner = "carrier-pigeon"` must not poison every js run."""
    section = load_config().get("js", {})  # pragma: no mutate — isinstance normalizes
    value = section.get("runner", "") if isinstance(section, dict) else ""
    return value if value in JS_RUNNERS else ""


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
