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
"""

from __future__ import annotations

import contextlib
import tomllib
from collections.abc import Iterable
from typing import Any

from .atomic import atomic_write_toml
from .paths import state_dir, values_dir


def _load_doc(slug: str) -> dict[str, Any]:
    path = values_dir() / f"{slug}.toml"
    if not path.exists():
        return {}
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        return {}


def _save_doc(slug: str, doc: dict[str, Any]) -> None:
    doc = {k: v for k, v in doc.items() if v}  # don't persist empty sections
    atomic_write_toml(values_dir() / f"{slug}.toml", doc)


def _strip_secrets(values: dict[str, str], secret_names: Iterable[str]) -> dict[str, str]:
    banned = set(secret_names)
    return {k: v for k, v in values.items() if k not in banned}


def load_state(slug: str) -> dict[str, Any]:
    """Return {"values": {…}, "extra_args": […], "presets": {name: {…}}, "last_run": {…}}.

    last_run is {"at": ISO-8601 str, "exit": int} after the first recorded run, else {}.
    """
    doc = _load_doc(slug)
    return {
        "values": dict(doc.get("values", {})),
        "extra_args": list(doc.get("extra_args", [])),
        "presets": {k: dict(v) for k, v in doc.get("presets", {}).items()},
        "last_run": dict(doc.get("last_run", {})),
    }


def save_last(
    slug: str,
    *,
    values: dict[str, str] | None = None,
    extra_args: list[str] | None = None,
    secret_names: Iterable[str] = (),
) -> None:
    """Remember last-used (read-modify-write, keeping presets). Secret keys are stripped (C3).

    None means "no new data — leave the stored value alone"; an EMPTY dict/list means
    "the user cleared it" and erases the stored value. (Folding those two into one falsy
    check made cleared extra args resurrect forever: the form saved nothing, the next
    run re-read the old value, reused it, and wrote it back.)

    Even on a call that carries no new values, any name in secret_names is dropped from
    the previously-stored values — a value saved while a parameter was public must not
    survive on disk after it becomes secret.
    """
    doc = _load_doc(slug)
    banned = set(secret_names)
    if values is not None:
        doc["values"] = _strip_secrets(values, banned)
    elif banned:
        doc["values"] = _strip_secrets(doc.get("values", {}), banned)
    if extra_args is not None:
        doc["extra_args"] = extra_args
    _save_doc(slug, doc)


def save_preset(
    slug: str,
    preset: str,
    values: dict[str, str],
    *,
    secret_names: Iterable[str] = (),
) -> None:
    """Save one named preset. Secret keys are stripped (C3)."""
    doc = _load_doc(slug)
    presets = dict(doc.get("presets", {}))
    presets[preset] = _strip_secrets(values, secret_names)
    doc["presets"] = presets
    _save_doc(slug, doc)


def delete_preset(slug: str, preset: str) -> bool:
    doc = _load_doc(slug)
    presets = dict(doc.get("presets", {}))
    if preset not in presets:
        return False
    del presets[preset]
    doc["presets"] = presets
    _save_doc(slug, doc)
    return True


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
    doc = _load_doc(slug)
    removed: set[str] = set()

    values = dict(doc.get("values", {}))
    # `removed` is still empty here, so |= and = are equivalent; pragma only the accumulation and
    # keep the intersection on its own line so its &→| mutant stays mutation-tested.
    value_hits = banned & values.keys()
    removed |= value_hits  # pragma: no mutate
    doc["values"] = _strip_secrets(values, banned)

    presets = dict(doc.get("presets", {}))
    new_presets: dict[str, dict[str, str]] = {}
    for name, preset_values in presets.items():
        removed |= banned & preset_values.keys()
        cleaned = _strip_secrets(preset_values, banned)
        # Drop a preset that held only the now-secret param, mirroring delete_preset — an empty
        # [presets.<name>] table would otherwise linger and still validate for `run --preset`.
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
    atomic_write_toml(state_dir() / "prompt.toml", {"last_runner": name})


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
    top-level key would be dropped by _save_doc's empty-section pruning (0 is falsy)."""
    doc = _load_doc(slug)
    last_run: dict[str, Any] = {"at": at, "exit": exit_code}
    if values is not None:
        # Unlike last-used [values], this is the exact accepted invocation: values
        # equal to defaults and delivered empty strings stay so --from-last can pin
        # what actually ran instead of reconstructing it from a later source version.
        last_run["values"] = _strip_secrets(values, secret_names)
    doc["last_run"] = last_run
    _save_doc(slug, doc)


def forget(slug: str) -> None:
    path = values_dir() / f"{slug}.toml"
    with contextlib.suppress(FileNotFoundError):
        path.unlink()
