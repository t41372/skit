"""Reconcile the `[tool.skit]` definitions with a Python script's current content (Phase 3).

The machinery is language-neutral and lives in `skit.analysis`; this module is the thin wiring
that binds the Python analyzer's `analyze` and re-exports the shared pieces, so the many existing
imports (`reconcile.reconcile`, `reconcile.edit_specs`, `reconcile.drift_lines`,
`reconcile.render_warning`, `reconcile.Report`, `reconcile.Candidate`, …) keep resolving with no
churn. See `skit.analysis` for the reconcile rules and the drift categories.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from ... import analysis
from ...analysis import Candidate, EditResult, Report, drift_lines, render_warning
from .analyzer import analyze

if TYPE_CHECKING:
    from ...params import ParamDecl

__all__ = [
    "Candidate",
    "EditResult",
    "Report",
    "drift_lines",
    "edit_specs",
    "reconcile",
    "render_warning",
]


def reconcile(text: str, specs: list[ParamDecl]) -> Report:
    """Reconcile the definitions with the script's current content (Python analyzer)."""
    return analysis.reconcile(text, specs, analyze=analyze)


def edit_specs(
    text: str,
    specs: list[ParamDecl],
    *,
    resync: bool = False,
    add: list[str] | tuple[str, ...] = (),
    remove: list[str] | tuple[str, ...] = (),
    secret: list[str] | tuple[str, ...] = (),
    no_secret: list[str] | tuple[str, ...] = (),
    prompts: dict[str, str] | None = None,
) -> EditResult:
    """Apply parameter-definition edit operations (Python analyzer). See analysis.edit_specs."""
    return analysis.edit_specs(
        text,
        specs,
        resync=resync,
        add=add,
        remove=remove,
        secret=secret,
        no_secret=no_secret,
        prompts=prompts,
        analyze=analyze,
    )
