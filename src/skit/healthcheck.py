"""One health pipeline for both faces: `skit doctor` and the TUI Health screen.

The sweeps were previously implemented twice and drifted apart — the TUI skipped
prompt drift and invalid runner rows entirely, so the two surfaces disagreed about
what "healthy" means, and the user who trusted the TUI never learned what doctor
knew. Every check lives HERE once; the CLI and the screen only render (each may add
its own remedy hint — a key chord there, a command here — but never its own facts).

Headless: no CLI/TUI imports.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from . import config, launcher, store
from .langs.registry import spec_for


@dataclass
class HealthReport:
    """The library-wide sweep result. Names are entry display names; every list keeps
    library order so the two faces render identically."""

    missing: list[store.Entry] = field(default_factory=list)
    drifted: list[store.Entry] = field(default_factory=list)
    needs_missing: dict[str, list[str]] = field(default_factory=dict)  # name → tools
    # Entries preflight refuses beyond the categories above: an uninstalled
    # interpreter/JS runtime, a pinned agent whose binary is gone, a vanished custom
    # working directory. name → the exact refusal message a run would show.
    launch_blocked: dict[str, str] = field(default_factory=dict)
    invalid_runner_rows: list[str] = field(default_factory=list)
    # Kept alongside for renderers that need slugs (the TUI's jump-to-script list).
    needs_entries: list[store.Entry] = field(default_factory=list)
    blocked_entries: list[store.Entry] = field(default_factory=list)


def entry_drifted(entry: store.Entry) -> bool:
    """Whether the stored definitions no longer match the script — through the entry's
    OWN analyzer, plus the prompt kind's fresh body scan (no analyzer, by design)."""
    spec = spec_for(entry.meta.kind)
    if spec is not None and spec.kind == "prompt":
        # An insertion-off prompt can't drift — nothing is filled at run time.
        if not entry.meta.interpolate or not entry.script_path.exists():
            return False
        from .langs.prompt import analyzer as prompt_analyzer
        from .langs.prompt import text as prompt_text

        try:
            text = prompt_text.read(entry.script_path)
        except (OSError, prompt_text.PromptEncodingError):
            return False  # unreadable bodies belong to the target/preflight sweeps
        fresh = set(prompt_analyzer.placeholder_names(text))
        return any(name not in fresh for name in entry.meta.params or [])
    if (
        spec is None
        or spec.analyzer is None
        or spec.params_io is None
        or not entry.script_path.exists()
    ):
        return False
    text = entry.script_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
    specs = spec.params_io.read(text)
    return bool(specs) and spec.analyzer.reconcile(text, specs).has_drift


def collect(entries: list[store.Entry]) -> HealthReport:
    """The whole-library sweep both faces consume."""
    report = HealthReport()
    for entry in entries:
        if launcher.target_missing(entry):
            report.missing.append(entry)
    for entry in entries:
        if entry_drifted(entry):
            report.drifted.append(entry)
    for entry in entries:
        tools = launcher.missing_needs(entry)
        if tools:
            report.needs_missing[entry.meta.name] = tools
            report.needs_entries.append(entry)
    for entry in entries:
        # Everything else preflight would refuse a run over (runtime binaries, pinned
        # agents, workdir existence) — the SAME code path a real run takes, so the two
        # can never disagree. Entries already reported above are skipped: their
        # preflight failure would just repeat the same issue in different words.
        if (
            entry in report.missing
            or entry.meta.name in report.needs_missing
            or spec_for(entry.meta.kind) is None
        ):
            continue
        try:
            launcher.preflight(entry)
        except launcher.LaunchError as exc:
            report.launch_blocked[entry.meta.name] = str(exc)
            report.blocked_entries.append(entry)
    report.invalid_runner_rows = config.invalid_prompt_runners()
    return report
