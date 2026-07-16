"""Per-script npm dependencies for JS/TS entries — the PEP 723 + uv analogue.

A copy-mode entry's declared dependencies are materialized as a real `node_modules` **next to the
stored copy** (`scripts/<slug>/`): node's ESM resolution walks up from the importing file, so
adjacency is the one mechanism all three runners honor (bun and deno also prefer a local
node_modules when present). skit generates a minimal private `package.json` from the declared list
and lets the resolved runner's own installer do the fetching — `npm install` under node (npm ships
with node), `bun install` under bun, `deno install` under deno — so a bun-only or deno-only user
never needs npm.

Lifecycle scripts are **disabled** on every runner (`--ignore-scripts` for npm and bun; deno skips
them by default), so a declared dependency can never execute code at install time — deps can arrive
from an import scan under `--no-input`, so install must stay inert. The flag matters for bun too:
left to its default, bun runs the postinstall scripts of a built-in allow-list of popular packages,
so `--ignore-scripts` is what actually closes that door and keeps the three runners consistent. A
package that genuinely requires its postinstall step (native-addon downloads) won't work; that
trade is deliberate and documented.

This module is **stdlib-only** (A2: launch paths may not import a parser); it runs inside
`RunnerLaunch.build`, the same moment `UvLaunch.build` calls `ensure_uv()` — the terminal is
already handed over. The installer's own output is captured (not streamed), so an install
announces itself with one stderr line first — an install "can legitimately take minutes", and
a silent minutes-long gap is indistinguishable from a hang (the same visibility discipline as
uvman's "downloading uv" notice). Install failures raise `NotExecutableError` (exit 126: the
target exists, its prerequisites don't), carrying the most informative line of the installer's
stderr.

Staleness is a content hash: `node_modules/.skit-deps-ok` records the sha256 of the generated
manifest plus the installer's name. Deps edited → hash mismatch → reinstall; node_modules deleted
→ marker gone → reinstall; nothing changed → one file read, no subprocess. The marker lives
*inside* node_modules so wiping the tree can never leave a stale "ok" behind. The whole
check-clean-install-stamp cycle runs under a per-entry advisory lock (same O_CREAT|O_EXCL +
stale-reclaim discipline as store._registry_lock), so two concurrent runs of one entry can't
race their installers over the same directory.
"""

from __future__ import annotations

import contextlib
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from typing import TYPE_CHECKING

from ...i18n import gettext
from ..base import NotExecutableError

if TYPE_CHECKING:
    from collections.abc import Iterator, Mapping
    from pathlib import Path

_UTF8 = "utf-8"  # pragma: no mutate — "utf-8"/"UTF-8" codec alias
_MARKER = ".skit-deps-ok"
_LOCK_NAME = ".skit-deps.lock"
# An install holding the lock longer than this is presumed crashed and reclaimed. Generous:
# a cold-cache install of a heavy tree can legitimately take minutes. Accepted trade-off
# (documented, like the live-run race): an install genuinely exceeding this has its lock
# reclaimed by a concurrent run — the lock mtime is set once at creation, and a refresh
# heartbeat would need a thread the launch path doesn't otherwise carry.
_LOCK_STALE_SECONDS = 600.0
_LOCK_POLL_SECONDS = 0.1

# argv tail per installer (the program path is resolved separately). npm's flags cut the advisory
# noise (audit/fund chatter) and disable lifecycle scripts; bun needs its own --ignore-scripts
# because its default is to run a built-in allow-list of packages' postinstall scripts (see the
# module docstring). deno skips lifecycle scripts unless asked, so it needs no flag.
_INSTALL_ARGS: dict[str, tuple[str, ...]] = {
    "npm": ("install", "--no-audit", "--no-fund", "--ignore-scripts"),
    "bun": ("install", "--ignore-scripts"),
    "deno": ("install",),
}

# Which installer each runner brings along: node ships npm; bun and deno install themselves.
_INSTALLER_FOR_RUNNER = {"node": "npm", "bun": "bun", "deno": "deno"}

# The original extension's explicit module flavor, preserved into the generated manifest's
# "type" field. The store flattens every source to `script.js`/`script.ts`, so an author's
# `.mjs`/`.cjs` signal would otherwise be lost — and node would warn (MODULE_TYPELESS_PACKAGE_JSON)
# on every run of an ESM script next to a type-less package.json. A plain `.js`/`.ts` source
# carries no explicit signal, so the field is omitted and node's own detection decides.
_MODULE_TYPES = {".mjs": "module", ".mts": "module", ".cjs": "commonjs", ".cts": "commonjs"}


def split_requirements(text: str) -> list[str]:
    """Split a comma-separated npm requirement list. npm version ranges never contain commas
    (they use spaces and `||`), so — unlike PEP 508, where `pep723.split_requirements` must
    respect specifier internals — a plain comma split is exactly right. Using the PEP 508
    splitter here is the bug this function exists to prevent: it treats `, @scope/pkg` as a
    specifier continuation (names must start with a letter/digit) and silently merges a scoped
    package into its neighbor."""
    return [part.strip() for part in text.split(",") if part.strip()]


def split_requirement(req: str) -> tuple[str, str]:
    """(package name, version range) for one declared requirement string. "chalk" → ("chalk",
    "*"), "chalk@^5" → ("chalk", "^5"), "@scope/pkg@1.2" → ("@scope/pkg", "1.2") — the last "@"
    separates a range only when it isn't the scope's leading "@"."""
    at = req.rfind("@")
    if at <= 0:
        return req, "*"
    name, rng = req[:at], req[at + 1 :]
    return (name, rng or "*") if not name.endswith("/") else (req, "*")


def module_type_for(source: str) -> str:
    """The manifest "type" the entry's ORIGINAL filename pins ("module" for .mjs/.mts,
    "commonjs" for .cjs/.cts, "" when the source carried no explicit signal or is unknown)."""
    if not source:
        return ""
    dot = source.rfind(".")
    return _MODULE_TYPES.get(source[dot:].lower(), "") if dot >= 0 else ""


def manifest_text(dependencies: list[str], *, module_type: str = "") -> str:
    """The generated package.json for a declared dependency list: private (never publishable),
    dependencies only, declaration order preserved, plus the module "type" when the original
    filename pinned one. This exact text is the staleness-hash input, so it must be
    deterministic for a given (list, type) pair."""
    deps: dict[str, str] = {}
    for req in dependencies:
        name, rng = split_requirement(req.strip())
        if name:
            deps[name] = rng
    manifest: dict[str, object] = {"private": True}
    if module_type:
        manifest["type"] = module_type
    manifest["dependencies"] = deps
    return json.dumps(manifest, indent=2) + "\n"


def ensure_module_manifest(entry_dir: Path, module_type: str) -> None:
    """Pin just the module "type" in a package.json next to a copy-mode entry that has no managed
    deps but whose origin filename pinned a flavor (.mjs/.cjs/.mts/.cts). The store flattens the
    stored copy to script.js/script.ts, so without this the runner has no module signal: deno reads
    a bare .js/.ts as ESM (a CommonJS script then throws `require is not defined`) and node <22.7
    reads it as CommonJS (an ESM script throws on `import`/`export`). A flavorless origin writes
    nothing — its module type is the runner's own default. Idempotent; no install, no node_modules.
    When deps ARE declared, ensure_installed writes the full manifest (with this same "type")
    instead, so the two never fight over the file."""
    if not module_type:
        return
    manifest = json.dumps({"private": True, "type": module_type}, indent=2) + "\n"
    target = entry_dir / "package.json"
    try:
        if target.read_text(encoding=_UTF8) == manifest:
            return  # already correct — no needless rewrite on every run
    except (OSError, ValueError):
        # Absent, unreadable, or non-UTF-8 (UnicodeDecodeError is a ValueError) — (re)write it.
        # Same discipline as needs_install and the marker read, which also self-heal a corrupt file.
        pass
    target.write_text(manifest, encoding=_UTF8)


# Injected copies of deps-managed entries live in entry_dir (module-resolution adjacency), so a
# crash between write and the caller's finally-unlink can strand one — possibly carrying secret
# values. Anything older than this is certainly not a live run's copy and gets swept.
_STALE_INJECTED_SECONDS = 3600.0


def sweep_stale_injected(entry_dir: Path) -> None:
    """Unlink aged `.injected-*` leftovers in entry_dir. Age-gated so a concurrent run's live
    copy is never swept; every error is ignored (the sweep is hygiene, never a blocker). Called
    on every RunnerLaunch build AND from clean(), so clearing an entry's deps can't strand a
    secret-bearing leftover with nothing left to sweep it."""
    cutoff = time.time() - _STALE_INJECTED_SECONDS
    try:
        candidates = list(entry_dir.glob(".injected-*"))
    except OSError:  # pragma: no cover — entry_dir vanished mid-run
        return
    for path in candidates:
        try:
            if path.stat().st_mtime < cutoff:
                path.unlink()
        except OSError:  # pragma: no cover — raced with another run's cleanup
            continue


def clean(entry_dir: Path) -> None:
    """Remove everything the engine ever materialized for an entry (the declared list was
    emptied, or a stale env is about to be rebuilt): the generated manifest, the installers'
    lockfiles, and node_modules. A removal that silently fails would poison the next install
    (npm layers over remnants; the marker would then stamp the mess as good), so failures are
    LOUD: NotExecutableError naming the first path that wouldn't go."""
    sweep_stale_injected(entry_dir)
    failures: list[str] = []
    for name in ("package.json", "package-lock.json", "bun.lock", "bun.lockb", "deno.lock"):
        try:
            (entry_dir / name).unlink(missing_ok=True)
        except OSError as exc:
            failures.append(f"{name}: {exc.strerror or exc}")

    def _on_error(_func: object, path: str, exc: BaseException) -> None:
        # "Already gone" is success, not failure: rmtree can race an external remover (the lock
        # only serializes skit-vs-skit) and reach here with the tree already deleted.
        if not isinstance(exc, FileNotFoundError):
            failures.append(path)

    tree = entry_dir / "node_modules"
    try:
        if tree.is_symlink():
            # A symlinked node_modules (a shared-store layout): rmtree refuses to recurse a
            # symlink, so remove the link itself — the target isn't ours to delete.
            tree.unlink()
        elif tree.exists():
            shutil.rmtree(tree, onexc=_on_error)
    except FileNotFoundError:
        pass  # raced an external remover between the check and the unlink — already gone
    except OSError as exc:
        failures.append(f"node_modules: {exc.strerror or exc}")
    if failures:
        raise NotExecutableError(
            gettext("Couldn't clear the old dependency environment: %(detail)s")
            % {"detail": failures[0]}
        )


def require_installer(runner: str) -> str:
    """The absolute path of the installer `runner` implies, or NotExecutableError when it isn't
    on PATH. Cheap (a PATH scan), so preflight can call it before the TUI suspends."""
    installer = _INSTALLER_FOR_RUNNER.get(runner, "npm")
    program = shutil.which(installer)
    if program is None:
        raise NotExecutableError(
            gettext(
                "%(installer)s is needed to install this script's dependencies, but it isn't on "
                "your PATH."
            )
            % {"installer": installer}
        )
    return program


@contextlib.contextmanager
def _install_lock(entry_dir: Path) -> Iterator[None]:
    """Serialize install/clean for ONE entry across processes — same portable advisory-lock
    discipline as store._registry_lock (O_CREAT|O_EXCL lockfile, stale-age reclaim so a crashed
    holder can't wedge the entry forever). Without it, two concurrent first runs both see a
    stale marker and race two installers over one directory."""
    lock_path = entry_dir / _LOCK_NAME
    # A unique per-acquisition token, written into the lockfile, so release removes OUR lock and
    # not a successor's. Without it, a holder whose over-running install was stale-reclaimed would,
    # on its late release, blindly unlink the successor's fresh lockfile — admitting a THIRD
    # concurrent installer over one directory and a marker stamped atop the wreckage.
    token = f"{os.getpid()}-{os.urandom(8).hex()}".encode()
    while True:
        try:
            fd = os.open(lock_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            # Contention: a holder exists — reclaim if it looks crashed (age-based, not token: a
            # presumed-dead holder's token is meaningless), else wait our turn. Two waiters both
            # reclaiming inside the stale window can briefly double-install — the same accepted
            # >_LOCK_STALE_SECONDS reclaim trade-off documented above, not a wedge.
            try:
                age = time.time() - lock_path.stat().st_mtime
            except OSError:
                age = _LOCK_STALE_SECONDS + 1  # vanished mid-check; treat as reclaimable
            if age > _LOCK_STALE_SECONDS:
                try:
                    lock_path.unlink()
                except FileNotFoundError:
                    pass  # another waiter reclaimed it first — retry the acquisition
                except OSError as exc:
                    # A stale lock we CANNOT remove (read-only dir, permissions mishap):
                    # retrying would busy-loop forever, so fail with the same 126-family
                    # message the unwritable-dir creation path uses.
                    raise NotExecutableError(
                        gettext("Couldn't prepare the dependency environment: %(error)s")
                        % {"error": exc.strerror or str(exc)}
                    ) from exc
            else:
                time.sleep(_LOCK_POLL_SECONDS)
            continue
        except OSError as exc:
            # Not contention — the lock file can't be created at all (read-only entry dir,
            # permissions mishap). Report it as the 126 family every other dependency
            # prerequisite uses, not a raw traceback at the script's own exit code.
            raise NotExecutableError(
                gettext("Couldn't prepare the dependency environment: %(error)s")
                % {"error": exc.strerror or str(exc)}
            ) from exc
        else:
            with contextlib.suppress(OSError):  # a failed token write degrades to reclaim-only
                os.write(fd, token)
            os.close(fd)
            break
    try:
        yield
    finally:
        # Release only our own lock: if a stale-reclaim already replaced it with a successor's
        # fresh lockfile, the token won't match and we leave that one alone.
        with contextlib.suppress(OSError):
            if lock_path.read_bytes() == token:
                lock_path.unlink()


# CSI/SGR escape sequences: deno colorizes stderr even into a pipe, and raw escape bytes must
# never reach the user-facing error line (or rich's own markup).
_ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")

# Trailing hint/pointer boilerplate the installers append AFTER the actual cause. Calibrated
# against real captured output (see test fixtures): npm ends E404 runs with a "you can also
# install from a tarball…" hint plus a debug-log pointer, ERESOLVE runs with a report pointer,
# and network failures with a proxy hint — taking the raw last line would surface those
# instead of the line that names the package or the cause.
_NOISE_MARKERS = (
    "A complete log of this run",
    "Note that you can also install",
    "tarball, folder, http url",
    "For a full report see",
    "If you are behind a proxy",
)
# npm's per-line prefix ("npm error ", "npm error 404 ", older "npm ERR! ").
_NPM_PREFIX_RE = re.compile(r"npm (?:error|warn|ERR!)(?: \d+)? ?")
# A line whose cause words mark it as the headline of the failure. npm buries these between a
# code line and hint boilerplate; deno/bun put them last. Preferring the LAST match yields the
# line naming the package/conflict for every captured shape (E404, ERESOLVE, ECONNREFUSED,
# deno missing-package, bun missing-package).
_CAUSE_RE = re.compile(
    r"(?i)not found|does not exist|could not be found|failed|unable to|refused|denied|conflict"
)


def _npm_line_is_noise(remainder: str) -> bool:
    """Whether an npm line's post-prefix remainder carries no headline information: nothing at
    all, an indented continuation (dependency trees, error-object dumps), a stack frame, a
    lone brace from the object dump, or a bare report/log path."""
    return (
        not remainder
        or remainder.startswith((" ", "at ", "/", "{", "}"))
        or bool(re.match(r"[A-Za-z]:\\", remainder))  # a bare Windows path
    )


def _failure_detail(stderr: bytes) -> str:
    """The most informative line of an installer's stderr: ANSI-stripped, npm's prefix-only /
    continuation / stack-frame / bare-path lines and trailing hint boilerplate dropped, then
    the LAST cause-marked line (falling back to the last survivor). Calibrated against real
    captured output — npm E404 ("404 Not Found - GET …/pkg"), npm ERESOLVE ("Fix the upstream
    dependency conflict…"), npm ECONNREFUSED ("FetchError: request to … failed"), deno
    ("npm package 'pkg' does not exist.") and bun ("pkg@* failed to resolve")."""
    text = _ANSI_RE.sub("", stderr.decode(_UTF8, "replace"))
    informative: list[str] = []
    for raw in text.splitlines():
        prefix = _NPM_PREFIX_RE.match(raw)
        if prefix is not None and _npm_line_is_noise(raw[prefix.end() :]):
            continue
        line = raw.strip()
        if not line or any(marker in line for marker in _NOISE_MARKERS):
            continue
        informative.append(line)
    causes = [line for line in informative if _CAUSE_RE.search(line)]
    if causes:
        return causes[-1]
    return informative[-1] if informative else "?"


def clear(entry_dir: Path) -> None:
    """clean(), taken under the entry's install lock — the entry point for callers OUTSIDE the
    install cycle (`skit deps --clear` via store.update_dependencies). Without the lock, a
    concurrent run's installer could have the tree ripped out from under it mid-install and
    then stamp a marker over the wreckage."""
    with _install_lock(entry_dir):
        clean(entry_dir)


def _resolve_manifest(
    dependencies: list[str], runner: str, module_type: str
) -> tuple[str, str, str]:
    """The (installer name, manifest text, staleness stamp) a dependency set implies under
    `runner`. Shared by ensure_installed and needs_install so the two can never disagree on
    whether an install is pending."""
    installer = _INSTALLER_FOR_RUNNER.get(runner, "npm")
    manifest = manifest_text(dependencies, module_type=module_type)
    stamp = hashlib.sha256(f"{installer}\n{manifest}".encode()).hexdigest()
    return installer, manifest, stamp


def needs_install(
    entry_dir: Path, dependencies: list[str], runner: str, *, module_type: str = ""
) -> bool:
    """Whether an install is pending: the node_modules marker is missing, unreadable, or doesn't
    match the stamp the current (deps, installer, module_type) would produce. A cheap, offline,
    lock-free probe for preflight — it reuses ensure_installed's own stamp so preflight can't
    demand an installer for a run that build would complete without touching it."""
    _installer, _manifest, stamp = _resolve_manifest(dependencies, runner, module_type)
    try:
        return (entry_dir / "node_modules" / _MARKER).read_text(encoding=_UTF8) != stamp
    except (OSError, ValueError):
        return True


def ensure_installed(
    entry_dir: Path,
    dependencies: list[str],
    runner: str,
    env: Mapping[str, str],
    *,
    module_type: str = "",
) -> None:
    """Make `entry_dir/node_modules` satisfy `dependencies`, installing only when stale.

    `runner` is the resolved runner's name ("node"/"bun"/"deno" — anything else falls back to
    npm); `env` is the full environment for the installer subprocess (the caller has already
    overlaid the mirror registry); `module_type` is the original filename's explicit module
    flavor for the manifest (see module_type_for). Raises NotExecutableError when the installer
    is missing or exits nonzero — the 126 family, matching a missing `needs` command.
    """
    installer, manifest, stamp = _resolve_manifest(dependencies, runner, module_type)
    marker = entry_dir / "node_modules" / _MARKER
    with _install_lock(entry_dir):
        try:
            recorded = marker.read_text(encoding=_UTF8)  # pragma: no mutate — ASCII hex content
            if recorded == stamp:
                return  # same deps, same installer, node_modules intact: nothing to do
        except (OSError, ValueError):
            # No node_modules / no marker yet — or a corrupted one (invalid UTF-8 raises
            # UnicodeDecodeError, a ValueError). Either way the answer is the same: install.
            pass
        program = require_installer(runner)
        # Anything stale is REBUILT from scratch: installers layer garbage over a foreign tree
        # (npm walks a deno-layout node_modules and happily installs the store's
        # devDependencies), a removed dep would linger as an orphan, and a leftover lockfile
        # from another installer can steer resolution. Declarative semantics — the manifest is
        # the whole truth, like uv's per-script environments.
        clean(entry_dir)
        (entry_dir / "package.json").write_text(  # pragma: no mutate — see _UTF8
            manifest, encoding=_UTF8
        )
        # One visible line before a possibly minutes-long captured install: a silent gap is
        # indistinguishable from a hang. stderr, so a script's piped stdout stays clean.
        print(
            gettext("Installing dependencies (%(installer)s)…") % {"installer": installer},
            file=sys.stderr,
        )
        try:
            proc = subprocess.run(  # noqa: S603 — fixed argv, installer resolved from PATH
                [program, *_INSTALL_ARGS[installer]],
                cwd=entry_dir,
                env=dict(env),
                capture_output=True,
                check=False,
            )
        except OSError as exc:
            raise NotExecutableError(
                gettext("Couldn't run %(installer)s: %(error)s")
                % {"installer": installer, "error": str(exc)}
            ) from exc
        if proc.returncode != 0:
            raise NotExecutableError(
                gettext("Installing dependencies failed (%(installer)s): %(detail)s")
                % {"installer": installer, "detail": _failure_detail(proc.stderr)}
            )
        marker.parent.mkdir(exist_ok=True)  # a dep-less manifest may install nothing at all
        marker.write_text(stamp, encoding=_UTF8)  # pragma: no mutate — ASCII hex content
