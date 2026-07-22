"""Concrete launch strategies (one per family; see base.LaunchStrategy).

The bodies here moved verbatim from launcher.py's _build_python/_build_exe/_build_shell
so behavior is pinned by the existing launcher tests; launcher.py keeps the public
build/describe/preflight/run surface and dispatches through the registry.
"""

from __future__ import annotations

import os
import re
import shlex
import shutil
import subprocess
import sys
from pathlib import Path
from typing import TYPE_CHECKING, ClassVar

from ..i18n import gettext
from ..paths import private_bin_dir
from .base import (
    ArgvLaunch,
    LaunchError,
    LaunchPayload,
    NotExecutableError,
    ShellLaunch,
    TargetMissingError,
)

if TYPE_CHECKING:
    from ..config import PromptRunner
    from ..models import Entry


def find_uv() -> str | None:
    """Detection order: PATH -> skit's private bin (A9/§5.6)."""
    found = shutil.which("uv")
    if found:
        return found
    private = private_bin_dir() / "uv"
    if private.exists():
        return str(private)
    private_exe = private_bin_dir() / "uv.exe"
    if private_exe.exists():
        return str(private_exe)
    return None


def ensure_uv() -> str:
    """Find uv or auto-download a managed copy (first-run experience: zero user action). Raises
    LaunchError on failure."""
    found = find_uv()
    if found:
        return found
    from .. import uvman

    try:
        return uvman.ensure_uv_downloaded()
    except uvman.UvDownloadError as exc:
        raise LaunchError(
            f"{gettext('uv not found. Install it (https://docs.astral.sh/uv/) or run skit doctor for guidance.')} ({exc})"
        ) from exc


def _check_script_exists(script: Path) -> None:
    if not script.exists():
        raise TargetMissingError(
            gettext("The script file doesn't exist: %(path)s") % {"path": str(script)}
        )


def _check_exe_exists(source: str) -> None:
    path = Path(source)
    if not path.exists():
        raise TargetMissingError(
            gettext("The executable doesn't exist: %(path)s") % {"path": source}
        )
    if not path.is_file():
        # A directory (a macOS .app bundle, a typo'd path) would reach subprocess.run and
        # die with a raw PermissionError traceback; refuse it as the usual clean 126.
        raise NotExecutableError(
            gettext("%(path)s isn't a runnable file (it's a directory or special file).")
            % {"path": source}
        )
    if sys.platform != "win32" and not os.access(path, os.X_OK):
        raise NotExecutableError(
            gettext("%(path)s exists but isn't executable (chmod +x it?).") % {"path": source}
        )


def join_for_display(argv: list[str]) -> str:
    if sys.platform == "win32":
        return subprocess.list2cmdline(argv)
    return shlex.join(argv)


class UvLaunch:
    """python entries: always `uv run --no-project --script <path>` (C2)."""

    def _argv_tail(self, entry: Entry) -> list[str]:
        """The flags after `uv run --no-project`, shared by build and describe.

        C2: unconditional isolation. Without --no-project, `uv run --script` attaches a
        block-less script to whatever uv project encloses the cwd (empirically verified) —
        and copy-mode entries default to workdir="invoke", so "run it from inside any
        project directory" was a live hijack path. Scripts with a PEP 723 block and
        reference-mode --with deps are unaffected by the flag.
        """
        cmd: list[str] = []
        # In reference mode, dependencies are recorded in meta (the original file can't take a
        # PEP 723 block), so pass them via --with/--python.
        if entry.meta.requires_python:
            cmd += ["--python", entry.meta.requires_python]  # pragma: no mutate — cmd is [] here
        for dep in entry.meta.dependencies or []:
            cmd += ["--with", dep]
        return cmd

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> LaunchPayload:
        # Check the cheap, local condition (does the script exist?) before the potentially-
        # network-bound one (is uv installed, or does it need downloading?) — mirrors
        # preflight's ordering, and spares a user with a missing script a pointless uv
        # download/error first.
        script = script_override or entry.script_path
        _check_script_exists(script)
        uv = ensure_uv()
        cmd = [uv, "run", "--no-project", *self._argv_tail(entry)]
        return ArgvLaunch([*cmd, "--script", str(script), *extra])

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> str:
        uv = find_uv() or "uv"  # when uv isn't installed yet the literal "uv" stands in
        cmd = [uv, "run", "--no-project", *self._argv_tail(entry)]
        script = script_override or entry.script_path
        return join_for_display([*cmd, "--script", str(script), *extra])

    def target(self, entry: Entry) -> Path | None:
        return entry.script_path

    def preflight(self, entry: Entry, *, runner: PromptRunner | None = None) -> None:
        _check_script_exists(entry.script_path)


class DirectLaunch:
    """exe entries: run the referenced file directly."""

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> LaunchPayload:
        exe = entry.meta.source
        _check_exe_exists(exe)
        return ArgvLaunch([exe, *extra])

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> str:
        return join_for_display([entry.meta.source, *extra])

    def target(self, entry: Entry) -> Path | None:
        return Path(entry.meta.source)

    def preflight(self, entry: Entry, *, runner: PromptRunner | None = None) -> None:
        _check_exe_exists(entry.meta.source)


def quote_for_shell(value: str) -> str:
    """Quote a single substituted value for the platform shell TemplateLaunch executes under,
    mirroring how `extra` args are already quoted below (shlex on POSIX, list2cmdline on Windows) —
    otherwise a value with spaces or shell metacharacters reshapes the command's argument
    structure or, worse, injects extra shell syntax."""
    if sys.platform == "win32":
        return subprocess.list2cmdline([value])
    return shlex.quote(value)


# Matches, left to right: a `{{` escape, a `}}` escape, or a `{name}` placeholder (the same
# identifier rule as store.extract_placeholders). Substitution and escape-restoration run together
# in ONE pass over the ORIGINAL template via this pattern so replacement text is never re-scanned —
# doing it as two sequential passes (substitute placeholders, then str.replace "{{"/"}}") would
# corrupt any substituted value that itself contains "{{" or "}}".
_TEMPLATE_TOKEN_RE = re.compile(r"\{\{|\}\}|(?<!\{)\{([a-zA-Z_][a-zA-Z0-9_]*)\}(?!\})")


def _posix_quote_state(text: str, state: str) -> str:
    """Advance the POSIX shell quote context ("" unquoted, "'" single, '"' double) across a
    chunk of template text. Backslash consumes the next character outside single quotes —
    inside double quotes that over-approximates (`\\x` leaves the backslash literal for most
    x), but the skipped character is never one that could change the state ('`'' is literal
    in double quotes anyway, and `\\\"` correctly stays inside). A chunk ENDING on an
    unconsumed backslash returns the state with "\\" appended ('\\\\' or '\"\\\\'): that
    escape would otherwise silently apply to the first character of whatever the caller
    emits next — exactly how `\"foo\\{name}\"` once ate the escape guarding a substituted
    value's `$`. Callers resolve the pending escape (see _substitute_posix); a resumed
    chunk consumes its first character as the escaped one. ANSI-C `$'…'` quoting is
    tracked as ordinary single quotes; its `\\'` escape is the one known mis-track."""
    i = 0
    n = len(text)
    if state.endswith("\\"):
        # Resuming after a dangling backslash: the first character is the escaped one.
        state = state.removesuffix("\\")
        if n == 0:
            return state + "\\"
        i = 1
    while i < n:
        ch = text[i]
        if state == "'":
            if ch == "'":
                state = ""
        elif ch == "\\":
            if i + 1 >= n:
                return state + "\\"  # dangling: the escape pends across the boundary
            i += 1  # pragma: no mutate — escaped-char skip (decrement/reset would hang)
        elif state == '"':
            if ch == '"':
                state = ""
        elif ch in ("'", '"'):
            state = ch
        i += 1  # pragma: no mutate — loop cursor (decrement/reset never reaches n)
    return state


def _posix_quote_value(value: str, state: str) -> str:
    """Quote one substituted value FOR the quote context its placeholder sits in. shlex.quote
    alone is only right in unquoted position: inside "double quotes" it leaves $(…)/backticks
    live and adds literal apostrophes, and inside 'single quotes' it nests quotes wrongly —
    the reproduced way a value like `$(cmd)` stopped arriving literally. Every branch is
    state-neutral: the template's own context resumes exactly where it left off."""
    if state == "'":
        return value.replace("'", "'\\''")
    if state == '"':
        out = value.replace("\\", "\\\\").replace('"', '\\"')
        return out.replace("$", "\\$").replace("`", "\\`")
    return shlex.quote(value)


class TemplateLaunch:
    """command entries: template + placeholder fill-in, executed through the shell."""

    @staticmethod
    def _substitute_posix(template: str, vals: dict[str, str]) -> str:
        """The POSIX twin of the Windows regex pass below: same one-pass token walk
        (substituted values are never re-scanned), but the quote context is tracked
        across the template's OWN text between tokens, so each value is escaped for
        the position it lands in. Brace escapes and untouched placeholders emit
        literal braces — no quote characters, so the state carries straight through."""
        out: list[str] = []
        # state: "XXXX" (the only surviving mutant of "") behaves identically — state is
        # only ever compared to "'"/'"' or tested with .endswith, never truthy-checked, so
        # any non-quote sentinel is the unquoted context. pos: only pos=None survives, and
        # None is the slice-start of 0 (template[None:x] == template[0:x]); pos=1 is killed.
        state = ""  # pragma: no mutate — see above: "XXXX" is an equivalent sentinel
        pos = 0  # pragma: no mutate — see above: None-start ≡ 0 (the only surviving mutant)
        for m in _TEMPLATE_TOKEN_RE.finditer(template):
            chunk = template[pos : m.start()]
            out.append(chunk)
            state = _posix_quote_state(chunk, state)
            matched = m.group(0)
            if state.endswith("\\"):
                # The template ends this chunk on a dangling backslash that would escape
                # the FIRST character we emit next — for a substituted value, eating the
                # `\\` that guards its `$`. Emitting one more backslash completes an
                # escaped `\\\\` pair (a literal backslash in both unquoted and double-
                # quoted sh, which is what the template's own text meant), and the value's
                # quoting below stays intact. A brace emission (m.group(1) is None) absorbs
                # the escape instead — `\\{` is the same literal two characters either way —
                # so the neutralizer is added ONLY before a substituted value.
                state = state.removesuffix("\\")
                if m.group(1) in vals:
                    out.append("\\")
            if matched == "{{":
                out.append("{")
            elif matched == "}}":
                out.append("}")
            else:
                name = m.group(1)
                if name is not None and name in vals:
                    out.append(_posix_quote_value(vals[name], state))
                else:
                    out.append(matched)  # unfilled placeholder: travels as-is, quote-neutral
            pos = m.end()
        out.append(template[pos:])
        return "".join(out)

    def _render(self, entry: Entry, extra: list[str], values: dict[str, str] | None) -> str:
        template = entry.meta.template
        vals = values or {}
        if entry.meta.params:
            missing = [p for p in entry.meta.params if p not in vals]
            if missing:
                raise LaunchError(
                    gettext("Missing parameter values: %(names)s") % {"names": ", ".join(missing)}
                )

        def repl(m: re.Match[str]) -> str:
            matched = m.group(0)
            if matched == "{{":
                return "{"
            if matched == "}}":
                return "}"
            name = m.group(1)
            if name is None or name not in vals:
                return matched
            return quote_for_shell(vals[name])

        if sys.platform == "win32":
            # cmd.exe has no tractable universal escape (`%` expands even inside double
            # quotes); keep the historical list2cmdline pass rather than pretend otherwise.
            cmd = _TEMPLATE_TOKEN_RE.sub(repl, template)
        else:
            cmd = self._substitute_posix(template, vals)
        if extra:
            # shell=True execution: quoting must follow that platform's shell (POSIX uses shlex,
            # Windows cmd uses list2cmdline), or arguments containing $ or backticks would be
            # expanded.
            if sys.platform == "win32":
                cmd = cmd + " " + subprocess.list2cmdline(extra)
            else:
                cmd = cmd + " " + shlex.join(extra)
        return cmd

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> LaunchPayload:
        return ShellLaunch(self._render(entry, extra, values))

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> str:
        try:
            return self._render(entry, extra, values)
        except LaunchError:
            return entry.meta.template

    def target(self, entry: Entry) -> Path | None:
        return None  # command entries have no file target

    def preflight(self, entry: Entry, *, runner: PromptRunner | None = None) -> None:
        return None  # nothing to check before values are collected


def _which(name: str) -> str | None:
    """PATH lookup, isolated so tests patch one seam for every interpreter strategy."""
    return shutil.which(name)


def resolve_interpreter(name: str) -> str:
    """The absolute path of an interpreter binary, or a clean refusal.

    Windows bash policy (approved in docs/design/multilang.md): PATH first (Git for
    Windows puts bash there), then the config key `shell.bash_path`, then an honest
    NotExecutableError (exit 126) that names both escape hatches — never a silent
    reroute through WSL (principle 6: skit doesn't decide for the user's environment).
    On POSIX a missing interpreter gets the same 126 with an install hint.
    """
    found = _which(name)
    if found:
        return found
    if name in ("bash", "sh", "zsh") and sys.platform == "win32":
        from .. import config

        configured = config.load_bash_path()
        if configured and Path(configured).exists():
            return configured
        raise NotExecutableError(
            gettext(
                "%(name)s isn't available on this system. Install Git for Windows (its bash "
                "works) or WSL, or point skit at one with: skit config shell.bash_path <path>"
            )
            % {"name": name}
        )
    raise NotExecutableError(
        gettext("The interpreter %(name)s isn't installed (or isn't on PATH).") % {"name": name}
    )


class InterpreterLaunch:
    """Interpreted kinds (shell/fish/ruby/…): `<interpreter> <script> <args>`.

    Running through the interpreter — instead of exec'ing the file like DirectLaunch —
    is what removes the +x requirement and makes copy mode possible: the execute bit
    and the shebang stay the file's business, not the launch contract's."""

    def __init__(self, default_interpreter: str, *, prefix: tuple[str, ...] = ()) -> None:
        self._default = default_interpreter
        self._prefix = prefix  # extra argv between interpreter and script (e.g. Rscript flags)

    def _interpreter_name(self, entry: Entry) -> str:
        return entry.meta.interpreter or self._default

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> LaunchPayload:
        script = script_override or entry.script_path
        _check_script_exists(script)
        interpreter = resolve_interpreter(self._interpreter_name(entry))
        return ArgvLaunch([interpreter, *self._prefix, str(script), *extra])

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> str:
        # Side-effect-free by design: the bare interpreter name stands in (no PATH
        # lookup — resolve_interpreter may raise, and describe must never). UvLaunch
        # differs deliberately: find_uv() is a cached, non-raising lookup.
        script = script_override or entry.script_path
        return join_for_display([self._interpreter_name(entry), *self._prefix, str(script), *extra])

    def target(self, entry: Entry) -> Path | None:
        return entry.script_path

    def preflight(self, entry: Entry, *, runner: PromptRunner | None = None) -> None:
        # Both checks are cheap and local (no network, unlike uv), so preflight can
        # afford them — the TUI gets "zsh isn't installed" before the terminal suspends.
        _check_script_exists(entry.script_path)
        resolve_interpreter(self._interpreter_name(entry))


class RunnerLaunch:
    """JS/TS: the first installed runner wins — deno > bun > node (deno's inline
    npm:/jsr: specifiers are the closest thing JS has to PEP 723 self-containment,
    bun auto-installs imports, node is the universal fallback). skit never downloads
    a runtime (approved decision: assume the user's tooling); meta.interpreter or the
    config key `js.runner` overrides the order outright."""

    ORDER: ClassVar[tuple[str, ...]] = ("deno", "bun", "node")

    # How each runner is invoked for a single script file. deno gets --allow-all: skit picks
    # the runner automatically, so the SAME script must behave the same under all three — node
    # and bun have no sandbox, and deno's would otherwise deny env/fs probes (auto-deny when
    # stdin isn't a TTY: exactly the agent/CI path). skit is a launcher, not a sandbox.
    _INVOKE: ClassVar[dict[str, tuple[str, ...]]] = {
        "deno": ("run", "--allow-all"),
        "bun": ("run",),
        "node": (),
    }

    def _resolve(self, entry: Entry) -> tuple[str, str]:
        """(absolute path, runner name); raises NotExecutableError when nothing is installed."""
        from .. import config

        override = entry.meta.interpreter or config.load_js_runner()
        candidates = (override,) if override else self.ORDER
        for name in candidates:
            found = _which(name)
            if found:
                return found, name
        raise NotExecutableError(
            gettext(
                "No JavaScript runtime found (looked for: %(names)s). Install deno, bun, or "
                "node — or pick one with: skit config js.runner <name>"
            )
            % {"names": ", ".join(candidates)}
        )

    def _preferred_name(self, entry: Entry) -> str:
        """The runner name describe shows without touching PATH (side-effect-free)."""
        from .. import config

        return entry.meta.interpreter or config.load_js_runner() or self.ORDER[0]

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> LaunchPayload:
        script = script_override or entry.script_path
        _check_script_exists(script)
        path, name = self._resolve(entry)
        # Swept on EVERY launch (not just deps-managed ones): a crash-stranded injected copy in
        # entry_dir may carry secret values, and it must not outlive the deps declaration that
        # put it there — clearing the deps must not also disable the cleanup.
        from .javascript import deps as js_deps

        js_deps.sweep_stale_injected(entry.dir)
        self._ensure_deps(entry, name)
        return ArgvLaunch([path, *self._INVOKE.get(name, ()), str(script), *extra])

    def _ensure_deps(self, entry: Entry, runner: str) -> None:
        """Make the copy-mode entry's package.json right before launch (copy mode only — a
        reference entry lives in its own project, whose node_modules already serves it). With
        declared deps, materialize node_modules next to the stored copy; with none but a
        module-typed origin, write just the "type" so the flattened stored copy isn't misread.
        Runs in build, not preflight: like UvLaunch's ensure_uv, a first install may hit the
        network, and by build time the terminal is the script's."""
        if entry.meta.mode != "copy":
            return
        from .javascript import deps as js_deps

        # The original filename's explicit module flavor (.mjs/.cjs/.mts/.cts) — the store flattens
        # sources to script.js/.ts, so this is the only surviving signal.
        module_type = js_deps.module_type_for(entry.meta.source)
        if not entry.meta.dependencies:
            # No managed deps, but a module-typed origin still needs its "type": without a
            # package.json, deno reads a bare .js/.ts as ESM (a CommonJS script throws
            # `require is not defined`) and node <22.7 reads an ESM one as CommonJS.
            js_deps.ensure_module_manifest(entry.dir, module_type)
            return
        from .. import config

        env = {**os.environ, **config.mirror_env(os.environ)}
        js_deps.ensure_installed(
            entry.dir, list(entry.meta.dependencies), runner, env, module_type=module_type
        )

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> str:
        name = self._preferred_name(entry)
        script = script_override or entry.script_path
        return join_for_display([name, *self._INVOKE.get(name, ()), str(script), *extra])

    def target(self, entry: Entry) -> Path | None:
        return entry.script_path

    def preflight(self, entry: Entry, *, runner: PromptRunner | None = None) -> None:
        _check_script_exists(entry.script_path)
        _, name = self._resolve(entry)
        if entry.meta.dependencies and entry.meta.mode == "copy":
            # Cheap, offline check only (preflight must not touch the network): surface "npm is
            # missing" before the TUI suspends, instead of mid-launch. But only when an install
            # is actually pending — a fresh marker means build short-circuits without the
            # installer, so demanding it here would refuse a run the CLI completes fine.
            from .javascript import deps as js_deps

            if js_deps.needs_install(
                entry.dir,
                list(entry.meta.dependencies),
                name,
                module_type=js_deps.module_type_for(entry.meta.source),
            ):
                js_deps.require_installer(name)


class PromptLaunch:
    """prompt entries: two-stage render → the chosen runner's argv, no shell.

    Stage 1 fills the body's managed `{{placeholder}}` holes with this run's values, raw;
    stage 2 substitutes the rendered text into the runner argv's one `{{prompt}}` token —
    a single execve argument, so there is no quoting and no injection surface (see
    langs/prompt/render.py). The runner arrives from the CLI/TUI layer via the
    `runner=` keyword (an explicit per-run pick); with none given, the entry's pinned
    `meta.runner` resolves here — so a pinned prompt runs through every existing call
    path with zero extra threading. No pin and no pick is an honest 126, never a guess.
    """

    def _resolve_runner(self, entry: Entry, runner: PromptRunner | None) -> PromptRunner:
        if runner is not None:
            return runner
        from .. import config

        pin = entry.meta.runner
        if not pin:
            if not config.load_prompt_runners():
                raise NotExecutableError(
                    gettext(
                        "No agents are configured. Add one with: "
                        "skit runner add mycli -- mycli run {{prompt}}"
                    )
                )
            raise NotExecutableError(
                gettext(
                    "No runner selected for %(name)s. Pass --runner NAME, or pin one with: "
                    "skit params %(name)s --runner NAME"
                )
                % {"name": entry.meta.name}
            )
        found = config.find_prompt_runner(pin)
        if found is None:
            raise NotExecutableError(
                gettext(
                    "The runner %(runner)s isn't configured (known: %(names)s). Manage "
                    "runners with: skit runner list"
                )
                % {
                    "runner": pin,
                    "names": ", ".join(r.name for r in config.load_prompt_runners()) or "—",
                }
            )
        return found

    def _require_binary(self, runner: PromptRunner) -> str:
        """The runner binary's absolute path, or the usual exit-126 refusal."""
        found = _which(runner.argv[0])
        if found is None:
            raise NotExecutableError(
                gettext(
                    "The runner %(runner)s needs %(binary)s, which isn't installed (or isn't "
                    "on PATH)."
                )
                % {"runner": runner.name, "binary": runner.argv[0]}
            )
        return found

    @staticmethod
    def _read_body(script: Path) -> str:
        """Read a prompt without universal-newline translation."""
        from .prompt import text as prompt_text

        try:
            return prompt_text.read(script)
        except prompt_text.PromptEncodingError as exc:
            raise LaunchError(str(exc)) from exc
        except OSError as exc:
            raise LaunchError(
                gettext("Can't read %(path)s: %(error)s")
                % {"path": str(script), "error": exc.strerror or str(exc)}
            ) from exc

    def _render_argv(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        argv: list[str],
    ) -> list[str]:
        """Render and validate argv from an already chosen runner command."""
        from .prompt import render

        script = script_override or entry.script_path
        _check_script_exists(script)
        text = self._read_body(script)
        managed = list(entry.meta.params or []) if entry.meta.interpolate else []
        rendered = render.render_body(text, values or {}, managed)
        filled = render.fill_runner_argv(argv, rendered, extra)
        render.check_argv_length(filled)
        return filled

    def validate_argv(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
        display_values: dict[str, str] | None = None,
    ) -> str:
        """Render the exact configured argv once and return its validated display.

        This is the dry-run/normal-transparency gate: prompt rendering, NUL bytes and
        platform argv limits fail before anything is printed, while a dry run remains
        side-effect-free and does not require the agent CLI to be installed.
        """
        from .prompt import render

        chosen = self._resolve_runner(entry, runner)
        script = script_override or entry.script_path
        _check_script_exists(script)
        text = self._read_body(script)
        managed = list(entry.meta.params or []) if entry.meta.interpolate else []
        rendered = render.render_body(text, values or {}, managed)
        argv = render.fill_runner_argv(list(chosen.argv), rendered, extra)
        render.check_argv_length(argv)
        if display_values is not None:
            # Render the masked twin from the SAME body snapshot that was validated.
            # A second read could expose a concurrently edited secret or print bytes
            # that never passed the argv limits above.
            display_body = render.render_body(text, display_values, managed)
            argv = render.fill_runner_argv(list(chosen.argv), display_body, extra)
        return join_for_display(argv)

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> LaunchPayload:
        payload, _safe_display, _chosen = self.build_snapshot(
            entry,
            extra,
            values,
            script_override,
            runner=runner,
        )
        return payload

    def build_snapshot(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> tuple[ArgvLaunch, str, PromptRunner]:
        """Build one executable argv plus its body-safe transparency display.

        Both derive from the same resolved runner row. This matters for an unpinned
        caller: a concurrent config edit must not make transparency describe a
        different runner than the already-prepared argv.
        """
        from .prompt import render

        chosen = self._resolve_runner(entry, runner)
        # Preserve preflight/old execute refusal order: body/render errors precede a
        # missing runner binary. Render against the configured command first, then
        # replace only argv[0] with the resolved executable and recheck its real size.
        argv = self._render_argv(entry, extra, values, script_override, list(chosen.argv))
        binary = self._require_binary(chosen)
        argv[0] = binary
        render.check_argv_length(argv)
        omitted = gettext("<rendered prompt omitted; use --dry-run to inspect it>")
        safe_display = join_for_display(render.fill_runner_argv(list(chosen.argv), omitted, extra))
        return ArgvLaunch(argv), safe_display, chosen

    def describe_compact(
        self,
        entry: Entry,
        extra: list[str],
        *,
        runner: PromptRunner | None = None,
    ) -> str:
        """The normal-run command line without copying the prompt into scrollback."""
        from .prompt import render

        chosen = self._resolve_runner(entry, runner)
        omitted = gettext("<rendered prompt omitted; use --dry-run to inspect it>")
        return join_for_display(render.fill_runner_argv(list(chosen.argv), omitted, extra))

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
        *,
        runner: PromptRunner | None = None,
    ) -> str:
        # Side-effect-free: no PATH lookup and no writes. Reading config IS safe here —
        # load_prompt_runners never seeds (materializing is ensure_prompt_runners_seeded,
        # a management-surface act) — and it's what keeps the transparency line honest
        # for a PINNED run that arrives without an explicit runner (the TUI rerun path):
        # a multi-token runner like the opencode seed must show its real flags, not a
        # two-token stub. Only an unresolvable pin degrades to the name + "{{prompt}}".
        from .prompt import render

        if runner is None and entry.meta.runner:
            from .. import config

            runner = config.find_prompt_runner(entry.meta.runner)
        argv = list(runner.argv) if runner is not None else [entry.meta.runner or "?", "{{prompt}}"]
        script = script_override or entry.script_path
        try:
            text = self._read_body(script)
        except LaunchError:
            return join_for_display([*argv, *extra])
        managed = list(entry.meta.params or []) if entry.meta.interpolate else []
        try:
            rendered = render.render_body(text, values or {}, managed)
        except LaunchError:
            return join_for_display([*argv, *extra])
        return join_for_display(render.fill_runner_argv(argv, rendered, extra))

    def target(self, entry: Entry) -> Path | None:
        return entry.script_path

    def preflight(self, entry: Entry, *, runner: PromptRunner | None = None) -> None:
        """Validate the body and the runner this launch will actually use.

        Headless callers and the form-free rerun path omit ``runner`` and therefore
        validate the entry's pin.  A run form passes its resolved picker choice so a
        stale or broken pin cannot veto an explicit per-run override.
        """
        _check_script_exists(entry.script_path)
        # Body validity is independent of runner resolution.  Read it even for an
        # unpinned prompt so doctor/Health and the TUI's post-picker preflight report
        # the same launch-blocking corruption as build/--dry-run.
        self._read_body(entry.script_path)
        if runner is not None or entry.meta.runner:
            self._require_binary(self._resolve_runner(entry, runner))
