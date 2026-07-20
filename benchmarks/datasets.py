"""Deterministic benchmark libraries, generated through skit's PUBLIC store API only.

Format fidelity by construction: every entry is added the way real entries are added
(`store.add_*`), so the generator cannot drift from the store format. The per-add
registry rewrite makes this O(N²) — fine at the N ≤ 1000 this pipeline uses by design
(the 10k tier + a bulk writer is an explicit non-goal; see docs/design/benchmarks.md).

Discontinuity clause: the kind mix, `state_fraction`, and the missing-target fraction
are load-bearing inputs to every scale/tui metric. Changing ANY of them bumps
GENERATOR_VERSION and is a history discontinuity.
"""

from __future__ import annotations

import json
import os
import shutil
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from pathlib import Path
from random import Random
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Callable, Iterator

    from skit.models import Entry

GENERATOR_VERSION = 1
DEFAULT_SEED = 20260720
DEFAULT_STATE_FRACTION = 0.5

# The tui probe filters on this character and asserts real row movement. The generator
# guarantees BOTH sides of that assertion: entry 0's searchable text never contains it
# (so a full match can't mask a dead filter), and — for n >= 3 — at least one long
# description does contain it (the word "description"), so the filtered subset is
# never empty either. Changing it is a probe+generator change, made here once.
SEARCH_PROBE_CHAR = "o"

# Sums to exactly 100; expanded into a shuffled 100-slot pattern so any N gets the mix.
KIND_MIX: tuple[tuple[str, int], ...] = (
    ("python", 30),
    ("shell", 20),
    ("js", 10),
    ("ts", 5),
    ("command", 10),
    ("prompt", 10),
    ("fish", 5),
    ("exe", 6),
    ("ruby", 1),
    ("perl", 1),
    ("lua", 1),
    ("r", 1),
)

# Every 10th reference-mode entry (in add order) gets its target deleted, driving the
# TUI/list `target_missing` path. Part of the discontinuity clause.
MISSING_TARGET_STRIDE = 10

_WORDS = ("alpha", "bravo", "delta", "gamma", "kilo", "lima", "omega", "sigma")
_EXT = {
    "python": "py",
    "shell": "sh",
    "js": "js",
    "ts": "ts",
    "fish": "fish",
    "exe": "sh",
    "ruby": "rb",
    "perl": "pl",
    "lua": "lua",
    "r": "r",
}


class DatasetError(RuntimeError):
    """Generation refused or failed its own verification."""


@dataclass(frozen=True)
class Manifest:
    root: Path
    n: int
    seed: int
    state_fraction: float
    generator_version: int
    skit_version: str
    slugs: tuple[str, ...]
    kinds: dict[str, str]

    @property
    def mid_slug(self) -> str:
        if not self.slugs:
            raise DatasetError("empty dataset has no mid entry")
        return self.slugs[len(self.slugs) // 2]

    def to_json(self) -> str:
        return (
            json.dumps(
                {
                    "generator_version": self.generator_version,
                    "skit_version": self.skit_version,
                    "n": self.n,
                    "seed": self.seed,
                    "state_fraction": self.state_fraction,
                    "slugs": list(self.slugs),
                    "kinds": self.kinds,
                },
                sort_keys=True,
                indent=2,
            )
            + "\n"
        )

    @classmethod
    def load(cls, root: Path) -> Manifest:
        doc = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
        return cls(
            root=root,
            n=doc["n"],
            seed=doc["seed"],
            state_fraction=doc["state_fraction"],
            generator_version=doc["generator_version"],
            skit_version=doc.get("skit_version", ""),
            slugs=tuple(doc["slugs"]),
            kinds=doc["kinds"],
        )


def check_reusable(manifest: Manifest, n: int) -> None:
    """Whether an on-disk dataset may be reused for this run. Every generation input
    must match — including the skit version that WROTE the store, so a
    version-changing branch switch can't reuse a stale store layout. (Same-version
    branches share the stamp; when switching between such branches locally, delete
    .bench/datasets — benchmarks/README.md says so.) Mismatch is an error to fix,
    never a silent apples-to-oranges comparison."""
    import skit

    inputs = (
        manifest.n,
        manifest.seed,
        manifest.state_fraction,
        manifest.generator_version,
        manifest.skit_version,
    )
    wanted = (n, DEFAULT_SEED, DEFAULT_STATE_FRACTION, GENERATOR_VERSION, skit.__version__)
    if inputs != wanted:
        raise DatasetError(
            f"dataset {manifest.root} was generated with different inputs "
            f"{inputs} (wanted {wanted}) — delete it and rerun"
        )


def skit_dirs(root: Path) -> dict[str, str]:
    """The SKIT_*_DIR values that point skit at this dataset — the same three
    variables tests/conftest.py isolates with. ALWAYS absolute: benchmarked children
    run from a scratch cwd, and a relative override would silently resolve against
    it — every read then sees an empty library and produces wrong-but-plausible
    numbers (this actually happened; see docs/design/benchmarks.md's failure-class
    list)."""
    resolved = root.resolve()
    return {
        "SKIT_DATA_DIR": str(resolved / "data"),
        "SKIT_STATE_DIR": str(resolved / "state"),
        "SKIT_CONFIG_DIR": str(resolved / "config"),
    }


@contextmanager
def scoped_skit_dirs(root: Path) -> Iterator[None]:
    """Temporarily point THIS process's skit at the dataset (store reads the env live
    per call). Save/restore so the generator never leaks redirection to its caller."""
    saved = {k: os.environ.get(k) for k in skit_dirs(root)}
    os.environ.update(skit_dirs(root))
    try:
        yield
    finally:
        for key, value in saved.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


def kind_slots(rng: Random) -> list[str]:
    slots = [kind for kind, share in KIND_MIX for _ in range(share)]
    rng.shuffle(slots)
    return slots


def entry_name(i: int, rng: Random) -> str:
    """Deterministic name variety: short/long ASCII with CJK and emoji sprinkled.
    Entry 0 is the search-probe invariant: its searchable text never contains
    SEARCH_PROBE_CHAR (the tui probe filters on it and asserts the row count drops).
    """
    if i == 0:
        return "alpha-seed-0"
    if i % 7 == 3:
        return f"測試腳本-{i}"
    if i % 7 == 5:
        return f"🚀-tool-{i}"
    if i % 2 == 0:
        return f"{rng.choice(_WORDS)}-{rng.choice(_WORDS)}-{rng.choice(_WORDS)}-{i}"
    return f"{rng.choice(_WORDS)}-{i}"


def entry_description(i: int, rng: Random) -> str:
    if i == 0:
        return ""  # keeps entry 0's searchable text SEARCH_PROBE_CHAR-free by construction
    phase = i % 3
    if phase == 0:
        return ""
    if phase == 1:
        return f"runs the {rng.choice(_WORDS)} task"
    return (
        "a long description that tells what this entry does, why it was added, and "
        "when to reach for it during daily work, in enough words to wrap a line"
    )


_SOURCE_TEMPLATES: dict[str, Callable[[int], str]] = {
    "python": lambda i: (
        f"# a small generated benchmark entry\nimport sys\n\nprint('entry {i}', len(sys.argv))\n"
    ),
    "shell": lambda i: (
        f'#!/usr/bin/env bash\nGREETING="${{GREETING:-hello-{i}}}"\necho "$GREETING"\n'
    ),
    "exe": lambda i: (
        f'#!/usr/bin/env bash\nGREETING="${{GREETING:-hello-{i}}}"\necho "$GREETING"\n'
    ),
    "js": lambda i: f"console.log('entry {i}', process.argv.length);\n",
    "ts": lambda i: f"const n: number = {i};\nconsole.log('entry', n);\n",
    "fish": lambda i: f"echo entry-{i}\n",
    "ruby": lambda i: f"puts 'entry {i}'\n",
    "perl": lambda i: f'print "entry {i}\\n";\n',
    "lua": lambda i: f"print('entry {i}')\n",
    "r": lambda i: f"cat('entry {i}\\n')\n",
}


def _source_text(kind: str, i: int) -> str:
    template = _SOURCE_TEMPLATES.get(kind)
    if template is None:
        raise DatasetError(f"no source template for kind {kind!r}")
    return template(i)


def _prompt_text(i: int) -> str:
    return (
        f"Review the file {{{{path}}}} and summarize finding {i} in one paragraph.\n"
        "Focus on {{topic}}.\n"
    )


def _require_empty(root: Path) -> None:
    if root.exists() and any(root.iterdir()):
        raise DatasetError(f"refusing to generate into non-empty {root} — clean it first")


def generate(
    root: Path,
    n: int,
    *,
    seed: int = DEFAULT_SEED,
    state_fraction: float = DEFAULT_STATE_FRACTION,
) -> Manifest:
    """Build a full skit home (data/state/config) with n entries under `root`."""
    if n < 0:
        raise DatasetError("n must be >= 0")
    if not 0 <= state_fraction <= 1:
        raise DatasetError("state_fraction must be within [0, 1]")
    _require_empty(root)
    root.mkdir(parents=True, exist_ok=True)
    src_dir = root / "srcfiles"
    src_dir.mkdir()

    rng = Random(f"{seed}:{n}:{state_fraction}")  # noqa: S311 — deterministic fixtures, not crypto
    slots = kind_slots(rng)

    with scoped_skit_dirs(root):
        # Import inside the redirected scope by convention (harmless either way: the
        # store reads SKIT_* live per call), keeping every store touch inside it.
        from skit import argstate, store
        from skit.params import ParamDecl

        slugs: list[str] = []
        kinds: dict[str, str] = {}
        reference_slugs: list[tuple[str, Path]] = []
        for i in range(n):
            kind = slots[i % 100]
            entry = _add_entry(src_dir, i, kind, entry_name(i, rng), entry_description(i, rng))
            slugs.append(entry.slug)
            kinds[entry.slug] = kind
            if kind == "exe":
                reference_slugs.append((entry.slug, Path(entry.meta.source)))
            # Realistic param blocks on a third of the script kinds — the form/launch
            # paths then have declared parameters to chew on.
            if kind in ("python", "shell", "js", "ts", "fish") and i % 3 == 0:
                store.write_parameters(
                    entry.slug,
                    [
                        ParamDecl(name="count", delivery="flag", flag="--count"),
                        ParamDecl(name="verbose", type="bool", action="store_true"),
                    ],
                )

        # Deliberately missing targets: every MISSING_TARGET_STRIDE-th reference entry.
        for j, (_slug, path) in enumerate(reference_slugs):
            if j % MISSING_TARGET_STRIDE == 0:
                path.unlink()

        # Last-run state for state_fraction of entries, timestamps spread over a fixed
        # synthetic range (drives the TUI's activity sort).
        base = datetime(2026, 1, 1, tzinfo=UTC)
        for i, slug in enumerate(slugs):
            if rng.random() < state_fraction:
                argstate.save_last(slug, values={"count": str(i % 7)})
                at = (base + timedelta(hours=i)).replace(microsecond=0).isoformat()
                argstate.record_run(slug, 0, at=at)

        found = len(store.list_entries())
        if found != n:
            raise DatasetError(f"generated {found} entries, expected {n}")

    import skit

    manifest = Manifest(
        root=root,
        n=n,
        seed=seed,
        state_fraction=state_fraction,
        generator_version=GENERATOR_VERSION,
        skit_version=skit.__version__,
        slugs=tuple(slugs),
        kinds=kinds,
    )
    (root / "manifest.json").write_text(manifest.to_json(), encoding="utf-8")
    return manifest


def _add_entry(src_dir: Path, i: int, kind: str, name: str, description: str) -> Entry:
    """One entry through the public store API — the same call a real `skit add` makes."""
    from skit import store

    if kind == "command":
        return store.add_command(f"echo {{msg}} entry-{i}", name=name, description=description)
    if kind == "prompt":
        path = src_dir / f"src_{i}.md"
        path.write_text(_prompt_text(i), encoding="utf-8")
        return store.add_prompt(path, name=name, description=description)
    path = src_dir / f"src_{i}.{_EXT[kind]}"
    path.write_text(_source_text(kind, i), encoding="utf-8")
    if kind == "exe":
        path.chmod(0o755)
        return store.add_exe(path, name=name, description=description)
    if kind == "python":
        return store.add_python(path, name=name, description=description)
    return store.add_script(path, kind=kind, name=name, description=description)


def generate_runover(root: Path, fixtures_dir: Path) -> Manifest:
    """The run_overhead suite's dedicated minimal library: exactly the three noop
    entries in an otherwise-empty home, so lane C's resolve cost never rides on the
    scale grid's N."""
    _require_empty(root)
    root.mkdir(parents=True, exist_ok=True)
    src_dir = root / "srcfiles"
    src_dir.mkdir()
    for name in ("noop.py", "noop.sh", "noop.js"):
        shutil.copy(fixtures_dir / name, src_dir / name)
    (src_dir / "noop.sh").chmod(0o755)

    with scoped_skit_dirs(root):
        from skit import store

        slugs = [
            store.add_python(src_dir / "noop.py", name="noop-py").slug,
            store.add_script(src_dir / "noop.sh", kind="shell", name="noop-sh").slug,
            store.add_script(src_dir / "noop.js", kind="js", name="noop-js").slug,
        ]
        found = len(store.list_entries())
        if found != 3:
            raise DatasetError(f"runover library has {found} entries, expected 3")

    import skit

    manifest = Manifest(
        root=root,
        n=3,
        seed=0,
        state_fraction=0.0,
        generator_version=GENERATOR_VERSION,
        skit_version=skit.__version__,
        slugs=tuple(slugs),
        kinds={slugs[0]: "python", slugs[1]: "shell", slugs[2]: "js"},
    )
    (root / "manifest.json").write_text(manifest.to_json(), encoding="utf-8")
    return manifest
