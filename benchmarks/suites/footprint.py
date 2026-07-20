"""footprint — what installing skit costs: wheel/sdist bytes on every profile, and on
the full profile a clean-venv dependency closure with per-distribution sizes (the
input the "optionalize tree-sitter?" decision has been waiting for)."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import TYPE_CHECKING

from ..results import Metric, Skip, SuiteOutput
from ._env import RunCtx

if TYPE_CHECKING:
    from ..pipeline import SuitePlan

_INSTALL_ATTEMPTS = 3  # closure installs touch the network; retry flakes

# Per-distribution installed sizes, computed inside the probe venv.
_DIST_SIZES = """\
import importlib.metadata, json, os
sizes = {}
for dist in importlib.metadata.distributions():
    total = 0
    for f in dist.files or []:
        try:
            total += os.path.getsize(dist.locate_file(f))
        except OSError:
            pass
    sizes[dist.metadata["Name"]] = total
print(json.dumps(sizes))
"""


def run(ctx: RunCtx, plan: SuitePlan) -> SuiteOutput:
    if ctx.uv is None:
        return SuiteOutput(
            suite="footprint",
            skipped=[Skip(suite="footprint", case="all", reason="uv not found")],
        )
    uv = ctx.uv
    output = SuiteOutput(suite="footprint")
    dist_dir = ctx.workdir / "dist"
    subprocess.run(  # noqa: S603 — fixed-shape uv argv
        [uv, "build", "--out-dir", str(dist_dir)],
        cwd=ctx.repo_root,
        check=True,
        capture_output=True,
    )
    wheels = sorted(dist_dir.glob("*.whl"))
    sdists = sorted(dist_dir.glob("*.tar.gz"))
    if len(wheels) != 1 or len(sdists) != 1:
        raise RuntimeError(f"expected one wheel and one sdist in {dist_dir}")
    output.metrics["footprint.wheel_bytes"] = Metric(
        value=float(wheels[0].stat().st_size), unit="bytes", n=1
    )
    output.metrics["footprint.sdist_bytes"] = Metric(
        value=float(sdists[0].stat().st_size), unit="bytes", n=1
    )
    from skit.uvman import UV_VERSION

    output.raw["uv_pinned_version"] = UV_VERSION

    if plan.closure:
        _closure(ctx, uv, wheels[0], output)
    return output


def _closure(ctx: RunCtx, uv: str, wheel: Path, output: SuiteOutput) -> None:
    venv = ctx.workdir / "footprint-venv"
    subprocess.run(  # noqa: S603 — fixed-shape uv argv
        [uv, "venv", str(venv)], check=True, capture_output=True
    )
    venv_python = venv / "bin" / "python"
    for attempt in range(1, _INSTALL_ATTEMPTS + 1):
        proc = subprocess.run(  # noqa: S603 — fixed-shape uv argv
            [uv, "pip", "install", "--python", str(venv_python), str(wheel)],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode == 0:
            break
        if attempt == _INSTALL_ATTEMPTS:
            raise RuntimeError(f"closure install failed {attempt}x: {proc.stderr[-2000:]}")
    purelib = subprocess.run(  # noqa: S603 — fixed-shape probe argv
        [str(venv_python), "-c", "import sysconfig; print(sysconfig.get_paths()['purelib'])"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    site = Path(purelib)
    output.metrics["footprint.closure_bytes"] = Metric(
        value=float(_tree_bytes(site)), unit="bytes", n=1
    )
    skit_bytes = sum(
        _tree_bytes(p) if p.is_dir() else p.stat().st_size
        for p in site.iterdir()
        if p.name == "skit" or p.name.startswith("skit_cli")
    )
    output.metrics["footprint.skit_installed_bytes"] = Metric(
        value=float(skit_bytes), unit="bytes", n=1
    )
    sizes: dict[str, int] = json.loads(
        subprocess.run(  # noqa: S603 — fixed-shape probe argv
            [str(venv_python), "-c", _DIST_SIZES], capture_output=True, text=True, check=True
        ).stdout
    )
    output.metrics["footprint.distributions"] = Metric(value=float(len(sizes)), unit="count", n=1)
    output.raw["largest_distributions"] = dict(sorted(sizes.items(), key=lambda kv: -kv[1])[:10])


def _tree_bytes(root: Path) -> int:
    return sum(p.stat().st_size for p in root.rglob("*") if p.is_file())
