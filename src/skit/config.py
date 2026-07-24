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

import contextlib
import socket
import sys
import tomllib
from collections.abc import Iterator, Mapping
from copy import deepcopy
from dataclasses import dataclass, replace
from typing import Any

from .atomic import advisory_file_lock, atomic_write_toml, load_toml_recoverable
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


_CONFIG_LOCK_POLL_SECONDS = 0.05


@contextlib.contextmanager
def _config_lock() -> Iterator[None]:
    """Serialize config transactions across processes and threads.

    Atomic replacement keeps one write parseable, but it cannot make a read-modify-write
    transaction atomic: two runner mutations can read the same document and the later
    replace then silently erase the earlier one. The persistent OS-backed lock is also
    used by i18n's language writer, store metadata, the registry, and JS installs.
    """
    with advisory_file_lock(
        _config_path().with_suffix(".lock"),
        poll_seconds=_CONFIG_LOCK_POLL_SECONDS,
    ):
        yield


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


@contextlib.contextmanager
def _config_update() -> Iterator[dict[str, Any]]:
    """Yield the latest config under its cross-process read-modify-write lock."""
    with _config_lock():
        doc = _load_config_for_save()
        yield doc
        save_config(doc)


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
    with _config_update() as doc:
        if command.strip():
            doc["editor"] = command.strip()
        else:
            doc.pop("editor", None)


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
    with _config_update() as doc:
        doc["form"] = style


AFTER_RUN_MODES = ("exit", "stay")


def load_after_run() -> str:
    """What the TUI does when a launched script finishes: "exit" (the default — skit is
    a launcher, so it quits, passes the script's exit code through, and leaves its output
    visible) or "stay" (return immediately to the Library workbench with a status line)."""
    value = load_config().get("after_run", "")  # pragma: no mutate — normalized below
    return value if value in AFTER_RUN_MODES else "exit"


def save_after_run(mode: str) -> None:
    """Persist the after-run behavior, preserving every other key."""
    with _config_update() as doc:
        doc["after_run"] = mode


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
    with _config_update() as doc:
        doc["mirror"] = {
            "enabled": mirror.enabled,
            "pypi": mirror.pypi,
            "python_install": mirror.python_install,
            "uv_binary": mirror.uv_binary,
            "npm": mirror.npm,
        }


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
    """The env overlay for skit's child processes (uv reads the index/install vars,
    npm/bun read the registry var — one overlay, every ecosystem it names).

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
    # The default here is dead weight: the isinstance(value, str) guard below maps any non-str
    # default (None from a bare/removed .get) back to "", so mutating it is unobservable.
    value = section.get("bash_path", "") if isinstance(section, dict) else ""  # pragma: no mutate
    return value if isinstance(value, str) else ""


def save_bash_path(path: str) -> None:
    """Persist (or clear, when empty) the Windows bash path, preserving every other key."""
    with _config_update() as doc:
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


# --------------------------------------------------------------------------
# prompt runners ([[prompt.runners]] — the agent CLIs `skit run <prompt>` fires)
#
# Deliberately scoped under `prompt.` and named PromptRunner in code: "runner" already
# means a JS runtime elsewhere (RunnerLaunch, `js.runner`), and the two vocabularies
# must never blur. A runner is a NAME plus an ARGV TOKEN LIST — one token list element
# per execve argument, `{{prompt}}` marking where the rendered prompt lands. No shell is
# ever involved (see langs/prompt/render.py), which is what makes multi-line prompts
# safe on every platform.
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class PromptRunner:
    """One configured agent CLI a prompt entry can be fired at."""

    name: str
    argv: tuple[str, ...]


@dataclass(frozen=True)
class PromptRunnerRow:
    """One raw configured row as the management UI needs to see it.

    Valid launchers carry ``invalid_reason=None``.  Invalid rows stay addressable by
    their raw list index so the user can remove exactly that row without a filtered
    read followed by a whole-list rewrite destroying every other malformed row.
    ``index=None`` identifies a malformed ``[prompt]`` / ``prompt.runners`` container.
    """

    index: int | None
    name: str
    argv: tuple[str, ...] | None
    invalid_reason: str | None
    descriptor: str
    # Deep-copied raw TOML value used as a compare-and-swap snapshot by row removal.
    # It includes unknown keys and non-table values that the typed projection cannot.
    raw: Any = None


class PromptRunnerConfigError(Exception):
    """A targeted runner mutation cannot preserve a malformed enclosing container."""

    def __init__(self, code: str) -> None:
        self.code = code
        message = {
            "prompt-section-not-table": gettext(
                "The [prompt] value in config.toml isn't a table. Remove or repair it "
                "before managing agents."
            ),
            "runners-not-list": gettext(
                "The prompt.runners value in config.toml isn't a list. Remove or repair it "
                "before managing agents."
            ),
        }[code]
        super().__init__(message)


class PromptRunnerExistsError(Exception):
    """The requested stable runner-name key already has at least one raw row."""


class PromptRunnerChangedError(Exception):
    """The runner row(s) being edited no longer match the management snapshot."""


def prompt_runner_row_reason(row: PromptRunnerRow) -> str:
    """Localized human wording for one raw runner-row status.

    ``invalid_reason`` remains the stable English machine code used by JSON and doctor;
    human CLI/TUI surfaces must never leak those implementation tokens as UI copy.
    Every gettext call stays on a literal so extraction and completeness checks see the
    whole closed set.
    """
    reason = row.invalid_reason
    if reason is None:
        return gettext("valid")
    if reason == "prompt-section-not-table":
        return str(PromptRunnerConfigError("prompt-section-not-table"))
    if reason == "runners-not-list":
        return str(PromptRunnerConfigError("runners-not-list"))
    return {
        "empty": gettext("Type the agent's command, e.g. mycli run {{prompt}}"),
        "prompt-slot-count": gettext(
            "The command needs the {{prompt}} slot exactly once — that's where the "
            "rendered prompt lands."
        ),
        "prompt-in-binary": gettext(
            "{{prompt}} can't be the command itself — the first word must be the program to run."
        ),
        "stray-hole": gettext(
            "Runner commands take only the {{prompt}} slot — single-brace text is literal, "
            "and other {{holes}} aren't supported."
        ),
        "name": gettext("A name is required."),
        "argv-type": gettext("The command must be a list of text arguments."),
        "row-not-table": gettext("This runner row isn't a table."),
        "duplicate": gettext("Another row already uses this runner name."),
    }.get(reason, gettext("This runner row is malformed."))


# The seven seeds (docs/design/prompt.md; gemini-cli deliberately excluded). Interactive
# invocations — each opens the agent's own session with the prompt as opening message.
# amp has no interactive-with-initial-prompt form, so its seed is the closest equivalent
# (`amp -x` executes the prompt). Antigravity installs as `agy`; its
# `--prompt-interactive` flag supplies the opening prompt and keeps the session open.
# Copilot binds its interactive prompt with `=` so a leading-dash prompt remains data.
# Cursor's fixed `agent` subcommand plus the root delimiter protects both option-looking
# prompts and prompts such as `status` that collide with another root subcommand.
# All of it is user-editable data, not code.
PROMPT_RUNNER_SEEDS: tuple[PromptRunner, ...] = (
    # Positional prompts need the end-of-options delimiter: without it, a prompt
    # beginning with `--help`/`--model` is parsed as runner flags instead of text.
    PromptRunner("claude", ("claude", "--", "{{prompt}}")),
    PromptRunner("codex", ("codex", "--", "{{prompt}}")),
    # OpenCode's yargs parser has the same ambiguity when the value is a separate
    # token. Binding it with `=` preserves prompts such as `--help` and `--version`.
    PromptRunner("opencode", ("opencode", "--prompt={{prompt}}")),
    PromptRunner("amp", ("amp", "-x", "{{prompt}}")),
    PromptRunner("antigravity", ("agy", "--prompt-interactive", "{{prompt}}")),
    PromptRunner("copilot", ("copilot", "--interactive={{prompt}}")),
    PromptRunner("cursor", ("cursor-agent", "--", "agent", "{{prompt}}")),
)


def validate_prompt_runner_argv(argv: list[str]) -> str | None:
    """Whether an argv token list is a well-formed runner template, as a symbolic reason
    id (None = valid; the CLI owns the human wording). Rules: non-empty, all strings,
    exactly one `{{prompt}}` token across all tokens, never in argv[0] (the binary), and
    no other `{{holes}}` (per-runner parameters are explicitly out of v1). Single-brace
    text is a literal on this surface — a tool's own `{x}` syntax passes untouched."""
    if not argv or not all(isinstance(t, str) and t for t in argv):
        return "empty"
    from .langs.prompt.analyzer import RESERVED_NAME, TOKEN_RE

    prompt_slots = 0
    for i, token in enumerate(argv):
        for m in TOKEN_RE.finditer(token):
            if not m.group(1).isidentifier() or m.group(1) != RESERVED_NAME:
                return "stray-hole"
            if i == 0:
                return "prompt-in-binary"
            prompt_slots += 1
    if prompt_slots != 1:
        return "prompt-slot-count"
    return None


def _parse_prompt_runner_rows(rows: list[Any]) -> list[PromptRunnerRow]:
    """Classify every raw row without discarding any of them.

    The launch path consumes only the valid subset, while management consumes this
    complete view.  Keeping the raw index here is the crucial distinction: a later
    targeted edit/removal can change one requested row without reconstructing the
    array from a lossy, valid-only projection.
    """
    parsed: list[PromptRunnerRow] = []
    seen_names: set[str] = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            parsed.append(
                PromptRunnerRow(index, "", None, "row-not-table", str(row), deepcopy(row))
            )
            continue
        name = row.get("name")
        argv = row.get("argv")
        normalized_argv = (
            tuple(argv)
            if isinstance(argv, list) and all(isinstance(token, str) for token in argv)
            else None
        )
        descriptor = str(name) if isinstance(name, str) and name.strip() else str(row)
        if not isinstance(name, str) or not name.strip():
            # Name and command are independent raw fields.  A missing stable key makes
            # the row unusable, but must not erase a perfectly recognizable argv from
            # management/JSON — the user needs to inspect it before exact-row repair.
            parsed.append(
                PromptRunnerRow(index, "", normalized_argv, "name", descriptor, deepcopy(row))
            )
            continue
        normalized_name = name.strip()
        if normalized_argv is None:
            parsed.append(
                PromptRunnerRow(
                    index, normalized_name, None, "argv-type", descriptor, deepcopy(row)
                )
            )
            continue
        reason = validate_prompt_runner_argv(list(normalized_argv))
        if reason is not None:
            parsed.append(
                PromptRunnerRow(
                    index,
                    normalized_name,
                    normalized_argv,
                    reason,
                    descriptor,
                    deepcopy(row),
                )
            )
            continue
        # Runner names are persistent keys (prompt pins store the name), and the TUI
        # also uses them as OptionList ids.  A hand-edited duplicate is therefore not
        # two usable rows: lookup would silently choose one while the management screen
        # could not represent both.  Keep the first valid definition deterministically
        # and report every later duplicate through the existing doctor channel.
        if normalized_name in seen_names:
            parsed.append(
                PromptRunnerRow(
                    index,
                    normalized_name,
                    normalized_argv,
                    "duplicate",
                    normalized_name,
                    deepcopy(row),
                )
            )
            continue
        seen_names.add(normalized_name)
        parsed.append(
            PromptRunnerRow(
                index,
                normalized_name,
                normalized_argv,
                None,
                normalized_name,
                deepcopy(row),
            )
        )
    return parsed


def _parse_prompt_runners(rows: Any) -> tuple[list[PromptRunner], list[str]]:
    """(valid runners, descriptors of skipped rows), retained for read-only callers."""
    if not isinstance(rows, list):
        return [], ["prompt.runners"]
    parsed = _parse_prompt_runner_rows(rows)
    return (
        [
            PromptRunner(row.name, row.argv)
            for row in parsed
            if row.invalid_reason is None and row.argv
        ],
        [row.descriptor for row in parsed if row.invalid_reason is not None],
    )


def _prompt_section() -> dict[str, Any]:
    section = load_config().get("prompt", {})  # pragma: no mutate — isinstance normalizes
    return section if isinstance(section, dict) else {}


def prompt_runners_seeded() -> bool:
    """True once the seed presets have been materialized (or the user authored their own
    rows). The marker is what lets a deliberately EMPTIED runner list stay empty instead
    of resurrecting the seeds on the next read."""
    doc = load_config()
    if "prompt" not in doc:
        return False
    section = doc["prompt"]
    # A malformed enclosing value is user data too.  Treat it as already configured
    # so merely opening a management surface cannot replace it with the built-in seeds.
    if not isinstance(section, dict):
        return True
    return section.get("runners_seeded") is True or "runners" in section


def load_prompt_runners() -> list[PromptRunner]:
    """The effective runner list, read-only: the configured rows once seeded (malformed
    rows skipped), the built-in seeds before then. Reading never writes — materializing
    the seeds into the user's config is ensure_prompt_runners_seeded(), called by the
    `skit runner` management surface (the moment the user goes looking for the data)."""
    if not prompt_runners_seeded():
        return list(PROMPT_RUNNER_SEEDS)
    valid, _invalid = _parse_prompt_runners(_prompt_section().get("runners", []))
    return valid


def invalid_prompt_runners() -> list[str]:
    """Descriptors of configured runner rows the loader skips (doctor's report)."""
    if not prompt_runners_seeded():
        return []
    return [row.descriptor for row in prompt_runner_rows() if row.invalid_reason is not None]


def prompt_runner_rows() -> list[PromptRunnerRow]:
    """Every effective/configured row, including invalid rows and their reason codes.

    This is the management view.  Launch and picker surfaces intentionally keep using
    ``load_prompt_runners()`` so a malformed row can never become executable merely
    because it is now visible and repairable.
    """
    if not prompt_runners_seeded():
        return [
            PromptRunnerRow(
                index,
                runner.name,
                runner.argv,
                None,
                runner.name,
                {"name": runner.name, "argv": list(runner.argv)},
            )
            for index, runner in enumerate(PROMPT_RUNNER_SEEDS)
        ]
    doc = load_config()
    section = doc.get("prompt")
    if not isinstance(section, dict):
        return [
            PromptRunnerRow(
                None,
                "",
                None,
                "prompt-section-not-table",
                "prompt",
                deepcopy(section),
            )
        ]
    rows = section.get("runners", [])
    if not isinstance(rows, list):
        return [
            PromptRunnerRow(
                None,
                "",
                None,
                "runners-not-list",
                "prompt.runners",
                deepcopy(rows),
            )
        ]
    return _parse_prompt_runner_rows(rows)


def find_prompt_runner(name: str) -> PromptRunner | None:
    for runner in load_prompt_runners():
        if runner.name == name:
            return runner
    return None


def _save_prompt_runners_locked(runners: list[PromptRunner]) -> None:
    """Full-list replacement while the caller holds ``_config_lock``."""
    doc = _load_config_for_save()
    section = doc.get("prompt")
    if not isinstance(section, dict):
        section = {}
    section["runners_seeded"] = True
    section["runners"] = [{"name": r.name, "argv": list(r.argv)} for r in runners]
    doc["prompt"] = section
    save_config(doc)


def save_prompt_runners(runners: list[PromptRunner]) -> None:
    """Explicitly replace the whole runner list, preserving non-runner config keys.

    Management mutations must use the targeted helpers below.  This full-replacement
    API remains for callers that intentionally own the complete list (tests, reset,
    and first-time seed materialization).
    """
    with _config_lock():
        _save_prompt_runners_locked(runners)


def _ensure_prompt_runners_seeded_locked() -> None:
    """Materialize seeds while the caller holds ``_config_lock`` (never re-lock)."""
    if not prompt_runners_seeded():
        _save_prompt_runners_locked(list(PROMPT_RUNNER_SEEDS))


def ensure_prompt_runners_seeded() -> None:
    """Materialize the seed presets into the user's config on first management need, so
    they are visible and editable — never a hidden built-in list. A no-op once seeded."""
    with _config_lock():
        _ensure_prompt_runners_seeded_locked()


def _prompt_runner_rows_for_targeted_save() -> tuple[dict[str, Any], dict[str, Any], list[Any]]:
    """Load the raw list for a lossless targeted mutation, refusing bad containers."""
    doc = _load_config_for_save()
    section_value = doc.get("prompt")
    if section_value is None:
        section: dict[str, Any] = {}
    elif not isinstance(section_value, dict):
        raise PromptRunnerConfigError("prompt-section-not-table")
    else:
        section = section_value
    rows_value = section.get("runners", [])
    if not isinstance(rows_value, list):
        raise PromptRunnerConfigError("runners-not-list")
    return doc, section, rows_value


def _raw_prompt_runner_name(row: Any) -> str:
    """The stable key carried by a recognizable raw row, normalized like the loader."""
    if not isinstance(row, dict):
        return ""
    name = row.get("name")
    return name.strip() if isinstance(name, str) else ""


def _toml_values_equal(left: Any, right: Any) -> bool:
    """Deep TOML equality that keeps bool and int distinct.

    Python's ordinary equality treats ``True == 1`` and ``False == 0``.  That is wrong
    for compare-and-swap snapshots of user configuration: changing an unknown field (or
    a whole malformed row/container) from an integer to a boolean is a real TOML edit and
    must invalidate the confirmation/edit snapshot.
    """
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(
            _toml_values_equal(value, right[key]) for key, value in left.items()
        )
    if isinstance(left, list):
        return len(left) == len(right) and all(
            _toml_values_equal(left_item, right_item)
            for left_item, right_item in zip(left, right, strict=True)
        )
    return bool(left == right)


def set_prompt_runner(
    runner: PromptRunner,
    *,
    replace_existing: bool = False,
    expected: list[PromptRunnerRow] | None = None,
) -> bool:
    """Add or explicitly replace one stable name while preserving unrelated raw rows.

    Returns whether one or more rows with that name existed.  Explicit replacement
    coalesces duplicate rows for the targeted key into the new valid definition; rows
    for every other key, including malformed/unknown values, stay byte-for-data intact.
    """
    with _config_lock():
        _ensure_prompt_runners_seeded_locked()
        return _set_prompt_runner_locked(
            runner, replace_existing=replace_existing, expected=expected
        )


def _set_prompt_runner_locked(
    runner: PromptRunner,
    *,
    replace_existing: bool,
    expected: list[PromptRunnerRow] | None,
) -> bool:
    """Targeted stable-key mutation while the caller holds ``_config_lock``."""
    doc, section, rows = _prompt_runner_rows_for_targeted_save()
    matches = [i for i, row in enumerate(rows) if _raw_prompt_runner_name(row) == runner.name]
    matching_rows = [rows[i] for i in matches]
    if expected is not None:
        expected_rows = [row.raw for row in expected if row.name == runner.name]
        if not _toml_values_equal(matching_rows, expected_rows):
            raise PromptRunnerChangedError(runner.name)
    if matches and not replace_existing:
        raise PromptRunnerExistsError(runner.name)
    new_row = {"name": runner.name, "argv": list(runner.argv)}
    if matches:
        first = matches[0]
        matching = set(matches)
        rows = [
            new_row if i == first else row
            for i, row in enumerate(rows)
            if i not in matching or i == first
        ]
    else:
        rows = [*rows, new_row]
    section["runners_seeded"] = True
    section["runners"] = rows
    doc["prompt"] = section
    save_config(doc)
    return bool(matches)


def replace_prompt_runner_row(
    index: int, runner: PromptRunner, *, expected: PromptRunnerRow
) -> bool:
    """CAS-repair one recognizable raw row whose stable name is missing.

    This is deliberately index-targeted rather than a stable-key replacement: a nameless
    row has no key to address.  The complete raw snapshot protects the repair window, and
    the newly supplied name must not collide with any other raw row.
    """
    with _config_lock():
        return _replace_prompt_runner_row_locked(index, runner, expected=expected)


def _replace_prompt_runner_row_locked(
    index: int, runner: PromptRunner, *, expected: PromptRunnerRow
) -> bool:
    """Exact raw-row repair while the caller holds ``_config_lock``."""
    doc, section, rows = _prompt_runner_rows_for_targeted_save()
    if (
        index < 0
        or index >= len(rows)
        or expected.index != index
        or not _toml_values_equal(rows[index], expected.raw)
    ):
        raise PromptRunnerChangedError(str(index))
    if any(
        current_index != index and _raw_prompt_runner_name(row) == runner.name
        for current_index, row in enumerate(rows)
    ):
        raise PromptRunnerExistsError(runner.name)
    rows = [
        {"name": runner.name, "argv": list(runner.argv)} if i == index else row
        for i, row in enumerate(rows)
    ]
    section["runners_seeded"] = True
    section["runners"] = rows
    doc["prompt"] = section
    save_config(doc)
    return True


def remove_prompt_runner(name: str, *, expected: list[PromptRunnerRow] | None = None) -> bool:
    """Remove every raw row for one stable key, preserving all other rows.

    When ``expected`` is supplied, compare only the complete raw rows for this key.
    Unrelated config edits do not block removal, but a replacement/addition under the
    confirmed name does — the confirmation must never authorize deleting a newer value.
    """
    name = name.strip()
    if not name:
        # Nameless/scalar rows all project to the empty key.  They are repairable only
        # through their exact raw index; a stable-key delete must never batch-delete them.
        return False
    with _config_lock():
        _ensure_prompt_runners_seeded_locked()
        return _remove_prompt_runner_locked(name, expected=expected)


def _remove_prompt_runner_locked(name: str, *, expected: list[PromptRunnerRow] | None) -> bool:
    """Targeted stable-key removal while the caller holds ``_config_lock``."""
    doc, section, rows = _prompt_runner_rows_for_targeted_save()
    matching_rows = [row for row in rows if _raw_prompt_runner_name(row) == name]
    if expected is not None and not _toml_values_equal(
        matching_rows, [row.raw for row in expected if row.name == name]
    ):
        return False
    kept = [row for row in rows if _raw_prompt_runner_name(row) != name]
    if len(kept) == len(rows):
        return False
    section["runners_seeded"] = True
    section["runners"] = kept
    doc["prompt"] = section
    save_config(doc)
    return True


def _remove_prompt_runner_container(
    doc: dict[str, Any], section_value: Any, expected: PromptRunnerRow | None
) -> bool:
    """CAS-remove the one malformed enclosing value represented by ``index=None``."""
    if not isinstance(section_value, dict):
        if expected is not None and (
            expected.index is not None
            or expected.invalid_reason != "prompt-section-not-table"
            or not _toml_values_equal(section_value, expected.raw)
        ):
            return False
        doc["prompt"] = {"runners_seeded": True, "runners": []}
        save_config(doc)
        return True
    rows_value = section_value.get("runners", [])
    if isinstance(rows_value, list):
        return False
    if expected is not None and (
        expected.index is not None
        or expected.invalid_reason != "runners-not-list"
        or not _toml_values_equal(rows_value, expected.raw)
    ):
        return False
    section_value["runners_seeded"] = True
    section_value["runners"] = []
    doc["prompt"] = section_value
    save_config(doc)
    return True


def remove_prompt_runner_row(index: int | None, *, expected: PromptRunnerRow | None = None) -> bool:
    """Remove the exact raw row selected in the TUI, or a malformed container.

    ``None`` is deliberately accepted only for the single container diagnostic exposed
    by ``prompt_runner_rows``.  Confirming that removal is the honest repair path for a
    non-list ``prompt.runners`` or a non-table ``prompt`` value.
    """
    with _config_lock():
        return _remove_prompt_runner_row_locked(index, expected=expected)


def _remove_prompt_runner_row_locked(
    index: int | None, *, expected: PromptRunnerRow | None
) -> bool:
    """Exact raw-row/container removal while the caller holds ``_config_lock``."""
    doc = _load_config_for_save()
    section_value = doc.get("prompt")
    if index is None:
        return _remove_prompt_runner_container(doc, section_value, expected)
    if not isinstance(section_value, dict):
        return False
    rows = section_value.get("runners", [])
    if not isinstance(rows, list) or index < 0 or index >= len(rows):
        return False
    # Compare the complete raw TOML value, not just name/argv. Unknown future keys and
    # malformed scalar rows are part of the selected row's identity too. This CAS closes
    # the user-visible confirmation window: an insert/removal before this index can never
    # make us delete whichever different row now occupies it.
    if expected is not None and (
        expected.index != index or not _toml_values_equal(rows[index], expected.raw)
    ):
        return False
    rows = [row for i, row in enumerate(rows) if i != index]
    section_value["runners_seeded"] = True
    section_value["runners"] = rows
    doc["prompt"] = section_value
    save_config(doc)
    return True


JS_RUNNERS = ("deno", "bun", "node")


def load_js_runner() -> str:
    """The preferred JS/TS runner (config.toml `[js] runner`), or "" for the built-in
    detection order (deno > bun > node). Unknown values normalize to "" — a hand-edited
    `runner = "carrier-pigeon"` must not poison every js run."""
    section = load_config().get("js", {})  # pragma: no mutate — isinstance normalizes
    # The default here is dead weight: the `value in JS_RUNNERS` guard below maps any non-runner
    # default (None, "", "XXXX", or the else-branch value) back to "", so mutating it is unobservable.
    value = section.get("runner", "") if isinstance(section, dict) else ""  # pragma: no mutate
    return value if value in JS_RUNNERS else ""


def save_js_runner(name: str) -> None:
    """Persist (or clear, when empty) the preferred JS runner, preserving every other key."""
    with _config_update() as doc:
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
