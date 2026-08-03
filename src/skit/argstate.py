"""Persistence of parameter values (state layer, separate from the data layer).

- Stored at state_dir()/values/<slug>.toml, removed together with the script.
- File shape: [values] (last-used), extra_args, [presets.<name>] (named presets).
- **C3 is enforced structurally here**: every write entry point requires secret_names, and any key
  in that list is stripped before it hits disk, so a secret value can never appear in a state file
  (there are tests for this). This includes the complete values snapshot nested under last_run,
  which powers `preset save --from-last`. This holds for *new* writes; it says nothing about a value
  that was written while the parameter was still public. purge_secret() retroactively scrubs that
  plaintext once a parameter transitions to secret, and save_last() also drops any now-secret key
  left over from before, even on calls that carry no new value for it — so nothing written while a
  parameter was public can outlive it becoming secret.
- Value resolution (this run's input > preset > last-used > definition default) lives in
  flows.prefill; this module only stores and strips.
- Writers fail typed: an OSError under any write (the lock, the atomic replacement) is
  re-raised as StateWriteError — an OSError subclass, mirroring config.ConfigWriteError —
  so the CLI's root boundary maps it to the operational exit and the TUI can notify
  instead of dying, while every existing `except OSError` (flows.post_run_persistence_error)
  keeps catching it unchanged. Readers stay untyped on purpose: they already degrade.
"""

from __future__ import annotations

import contextlib
import tomllib
from collections.abc import Iterable
from pathlib import Path
from typing import Any

from .atomic import advisory_file_lock, atomic_write_toml
from .paths import state_dir, values_dir


class StateWriteError(OSError):
    """An expected filesystem failure while updating skit's per-entry state."""


def _rewrap(exc: OSError) -> StateWriteError:
    """The one OSError→StateWriteError spelling (config._config_lock's shape): errno,
    strerror and filename survive, so _fail_operational renders the same sentence it
    renders for a config write."""
    return StateWriteError(exc.errno, exc.strerror or str(exc), exc.filename)


def _values_lock_path(slug: str) -> Path:
    # Outside values/ — forget() unlinks the values file itself, and a lock file must
    # never be a thing another process is about to unlink (store._entry_lock_path's
    # rule). Every read-modify-write below holds this lock: atomic_write_toml alone
    # stops torn TOML but not last-writer-wins — two processes saving different
    # presets from the same stale snapshot would silently drop one of them.
    return state_dir() / ".locks" / f"{slug}.values.lock"


def _load_doc(slug: str) -> dict[str, Any]:
    # One syscall, not two: the missing-file case is the common one for an entry that
    # has never run, and it already lands in the OSError branch below. A preceding
    # exists() only bought a second stat per entry — a thousand of them on `skit list`.
    try:
        with open(values_dir() / f"{slug}.toml", "rb") as f:
            doc = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        return {}
    # Chokepoint shape guard (store._load_registry's rule): a values file is TOML a
    # person can edit, so a section may hold a scalar where every reader expects a
    # table (or array). Dropping the malformed section HERE — not in each reader —
    # keeps one degradation rule for all of them: the listing's last_run stamp, the
    # run form's prefill, preset commands. Without it, `last_run = "x"` crashed
    # `skit list --json` whole. Only the shapes readers subscript are guarded; leaf
    # values round-trip as-is.
    for key, shape in (
        ("values", dict),
        ("extra_args", list),
        ("presets", dict),
        ("last_run", dict),
    ):
        if key in doc and not isinstance(doc[key], shape):
            del doc[key]
    if "presets" in doc:
        doc["presets"] = {k: v for k, v in doc["presets"].items() if isinstance(v, dict)}
    last_run = doc.get("last_run")
    if isinstance(last_run, dict) and not isinstance(last_run.get("values", {}), dict):
        # The nested snapshot is subscripted too (purge_secret, --from-last).
        doc["last_run"] = {k: v for k, v in last_run.items() if k != "values"}
    return doc


def _save_doc(slug: str, doc: dict[str, Any]) -> None:
    doc = {k: v for k, v in doc.items() if v}  # don't persist empty sections
    atomic_write_toml(values_dir() / f"{slug}.toml", doc)


def _strip_secrets(values: dict[str, str], secret_names: Iterable[str]) -> dict[str, str]:
    banned = set(secret_names)
    return {k: v for k, v in values.items() if k not in banned}


def load_state(slug: str) -> dict[str, Any]:
    """Return {"values": {…}, "extra_args": […], "extra_args_raw": bool,
    "presets": {name: {…}}, "last_run": {…}}.

    last_run is {"at": ISO-8601 str, "exit": int} after the first recorded run, else {}.
    extra_args_raw says HOW the remembered tail was captured: True = raw intent text
    (the TUI's extra field — tokens/globs expand on replay), False/absent = already
    shell-processed (the CLI's `-- args` — replays literally, in both faces). Without
    it, one stored tail replayed under two different expansion regimes depending on
    which face happened to rerun it.
    """
    doc = _load_doc(slug)
    return {
        "values": dict(doc.get("values", {})),
        "extra_args": list(doc.get("extra_args", [])),
        # `is True`, not bool(): the house rule for hand-editable bools (models.py's
        # interpolate, config.py's enabled). A hand-edited `extra_args_raw = "no"`
        # must degrade to the safe literal-replay default, never coerce truthy toward
        # re-expansion — the exact direction the provenance marker exists to prevent.
        "extra_args_raw": doc.get("extra_args_raw") is True,
        "presets": {k: dict(v) for k, v in doc.get("presets", {}).items()},
        "last_run": dict(doc.get("last_run", {})),
    }


def last_run(slug: str) -> dict[str, Any]:
    """Just the last-run stamp — {"at": ISO-8601 str, "exit": int}, or {} before the
    first recorded run.

    `load_state` reads the same file but also copies out values, extra args and every
    preset; a listing needs none of that, and pays for all of it once per entry.
    """
    return dict(_load_doc(slug).get("last_run", {}))


def save_last(
    slug: str,
    *,
    values: dict[str, str] | None = None,
    extra_args: list[str] | None = None,
    extra_args_raw: bool = False,
    secret_names: Iterable[str] = (),
) -> None:
    """Remember last-used (read-modify-write, keeping presets). Secret keys are stripped (C3).

    None means "no new data — leave the stored value alone"; an EMPTY dict/list means
    "the user cleared it" and erases the stored value. (Folding those two into one falsy
    check made cleared extra args resurrect forever: the form saved nothing, the next
    run re-read the old value, reused it, and wrote it back.)

    extra_args_raw records the tail's provenance (see load_state) and travels WITH the
    tail: it is written or cleared exactly when extra_args is, so a marker can never
    describe a tail it didn't come with.

    Even on a call that carries no new values, any name in secret_names is dropped from
    the previously-stored values — a value saved while a parameter was public must not
    survive on disk after it becomes secret.
    """
    try:
        with advisory_file_lock(_values_lock_path(slug)):
            doc = _load_doc(slug)
            banned = set(secret_names)
            if values is not None:
                doc["values"] = _strip_secrets(values, banned)
            elif banned:
                doc["values"] = _strip_secrets(doc.get("values", {}), banned)
            if extra_args is not None:
                doc["extra_args"] = extra_args
                if extra_args and extra_args_raw:
                    doc["extra_args_raw"] = True
                else:
                    # _save_doc prunes falsy values, so False is stored as absence; pop so a
                    # cleared/processed tail never inherits a stale raw marker.
                    doc.pop("extra_args_raw", None)
            _save_doc(slug, doc)
    except OSError as exc:
        raise _rewrap(exc) from exc


def save_preset(
    slug: str,
    preset: str,
    values: dict[str, str],
    *,
    secret_names: Iterable[str] = (),
) -> None:
    """Save one named preset. Secret keys are stripped (C3)."""
    try:
        with advisory_file_lock(_values_lock_path(slug)):
            doc = _load_doc(slug)
            presets = dict(doc.get("presets", {}))
            presets[preset] = _strip_secrets(values, secret_names)
            doc["presets"] = presets
            _save_doc(slug, doc)
    except OSError as exc:
        raise _rewrap(exc) from exc


def delete_preset(slug: str, preset: str) -> bool:
    try:
        with advisory_file_lock(_values_lock_path(slug)):
            doc = _load_doc(slug)
            presets = dict(doc.get("presets", {}))
            if preset not in presets:
                return False
            del presets[preset]
            doc["presets"] = presets
            _save_doc(slug, doc)
            return True
    except OSError as exc:
        raise _rewrap(exc) from exc


def purge_secret(slug: str, names: Iterable[str]) -> set[str]:
    """Retroactively scrub plaintext for parameters that have just become secret.

    C3 (see module docstring) only stops *new* writes; a value stored while a parameter was still
    public stays on disk until something removes it. Call this once, at the moment a parameter
    transitions to secret, to purge that name from last-used [values] and from every
    [presets.*] entry for this slug.

    Returns the subset of names that actually had a stored value removed (from either [values] or
    any preset), so callers can tell the user what was cleaned up. Passing an empty names is a
    no-op that touches nothing on disk.
    """
    banned = set(names)
    if not banned:
        return set()
    try:
        return _purge_secret_locked(slug, banned)
    except OSError as exc:
        raise _rewrap(exc) from exc


def _purge_secret_locked(slug: str, banned: set[str]) -> set[str]:
    with advisory_file_lock(_values_lock_path(slug)):
        doc = _load_doc(slug)
        removed: set[str] = set()

        values = dict(doc.get("values", {}))
        # `removed` is still empty here, so |= and = are equivalent; pragma only the accumulation
        # and keep the intersection on its own line so its &→| mutant stays mutation-tested.
        value_hits = banned & values.keys()
        removed |= value_hits  # pragma: no mutate
        doc["values"] = _strip_secrets(values, banned)

        presets = dict(doc.get("presets", {}))
        new_presets: dict[str, dict[str, str]] = {}
        for name, preset_values in presets.items():
            removed |= banned & preset_values.keys()
            cleaned = _strip_secrets(preset_values, banned)
            # Drop a preset that held only the now-secret param, mirroring delete_preset — an
            # empty [presets.<name>] table would otherwise linger and still validate for
            # `run --preset`.
            if cleaned:
                new_presets[name] = cleaned
        doc["presets"] = new_presets

        # The exact last-run snapshot is another value-bearing surface. A parameter that
        # becomes secret after it ran publicly must be scrubbed here too, or --from-last
        # could copy the old plaintext back into a preset.
        last_run = dict(doc.get("last_run", {}))
        if "values" in last_run:
            last_values = dict(last_run.get("values", {}))
            removed |= banned & last_values.keys()
            last_run["values"] = _strip_secrets(last_values, banned)
            doc["last_run"] = last_run

        _save_doc(slug, doc)
        return removed


def load_last_runner() -> str:
    """The most recently PICKED prompt-runner name (state, not config). One job only:
    prefill the next picker — it never resolves a non-interactive run (a --no-input run
    must be provably unaffected by it). Corrupt/absent state degrades to "" (no prefill),
    never an error."""
    path = state_dir() / "prompt.toml"
    if not path.exists():
        return ""
    try:
        with open(path, "rb") as f:
            doc = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        return ""
    value = doc.get("last_runner", "")  # pragma: no mutate — isinstance normalizes
    return value if isinstance(value, str) else ""


def save_last_runner(name: str) -> None:
    """Remember an explicit runner pick (add-time picker, `--runner`, the run form's
    picker). Using a PIN is not a pick and never lands here."""
    try:
        atomic_write_toml(state_dir() / "prompt.toml", {"last_runner": name})
    except OSError as exc:
        raise _rewrap(exc) from exc


def record_run(
    slug: str,
    exit_code: int,
    *,
    at: str,
    values: dict[str, str] | None = None,
    secret_names: Iterable[str] = (),
) -> None:
    """Remember when the entry last ran and how it exited (Library sort order, detail pane,
    and the r-rerun context key all read this). Stored as a table — a bare `last_exit = 0`
    top-level key would be dropped by _save_doc's empty-section pruning (0 is falsy).

    `values=None` follows the convention save_last states one screen up: "no new data —
    leave the stored value alone". It used to REPLACE the whole table, so `skit run --raw`
    — whose call site promises the escape hatch "leaves no fingerprints … values survive
    for the next real run" — deleted the snapshot on its way past. What it left behind was
    exactly the shape `preset save --from-last` documents as legacy state, so that command
    then refused with "no remembered values yet — run it once first" about an entry whose
    values were sitting in the same file and which had just run twice.
    """
    try:
        with advisory_file_lock(_values_lock_path(slug)):
            doc = _load_doc(slug)
            last_run: dict[str, Any] = {"at": at, "exit": exit_code}
            if values is not None:
                # Unlike last-used [values], this is the exact accepted invocation: values
                # equal to defaults and delivered empty strings stay so --from-last can pin
                # what actually ran instead of reconstructing it from a later source version.
                last_run["values"] = _strip_secrets(values, secret_names)
            else:
                kept = doc.get("last_run")
                if isinstance(kept, dict) and isinstance(snapshot := kept.get("values"), dict):
                    last_run["values"] = snapshot
            doc["last_run"] = last_run
            _save_doc(slug, doc)
    except OSError as exc:
        raise _rewrap(exc) from exc


def forget(slug: str) -> None:
    path = values_dir() / f"{slug}.toml"
    with contextlib.suppress(FileNotFoundError):
        path.unlink()
