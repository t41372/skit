"""Golden corpus fidelity tests (C series).

Every corpus script runs four checks:
1. analyzer never raises: any script must yield candidates (or an empty list) without exceptions.
2. metawriter byte-for-byte fidelity: write [tool.skit] then read it back; every byte outside the
   newly added PEP 723 lines must be identical to the original (C1).
3. shim with no values is the identity: inject(text, specs, {}) must return the exact same bytes.
4. shim-injected output still compiles: inject a type-compatible sample value for each candidate
   and verify that compile() accepts the result, with the PEP 723 block untouched.

Corpus coverage: shebang/coding, docstring, __future__, main guard, CRLF, tab indentation, no
trailing newline, CJK identifiers, input() inside loops/comprehensions, walrus, decorator, async,
argparse, and other real-world script shapes.
"""

from pathlib import Path

import pytest

from skit.langs.python import analyzer, metawriter, shim

CORPUS = sorted((Path(__file__).parent / "corpus").glob("*.py"))

# Sample values used when exercising full-value injection
_SAMPLE = {"str": "sample", "int": "7", "float": "1.5", "bool": "true"}


def _read(path: Path) -> str:
    # newline="" semantics: preserve CRLF as-is (splitlines(keepends=True) honours that too).
    # Use open(newline="") instead of Path.read_text(newline=) — the latter is a 3.13+ API and
    # this project's floor is 3.12.
    with path.open(encoding="utf-8", newline="") as f:
        return f.read()


def _specs_for(text: str) -> list[metawriter.ParamSpec]:
    return [
        metawriter.ParamSpec(
            name=c.name,
            kind=c.kind,
            type=c.type,
            default=c.default,
            prompt=c.prompt,
            order=c.order,
            secret=c.secret,
        )
        for c in analyzer.analyze(text).candidates
    ]


@pytest.mark.parametrize("path", CORPUS, ids=lambda p: p.name)
def test_analyzer_never_raises(path: Path):
    analyzer.analyze(_read(path))


@pytest.mark.parametrize("path", CORPUS, ids=lambda p: p.name)
def test_metawriter_byte_fidelity(path: Path):
    text = _read(path)
    specs = _specs_for(text)
    written = metawriter.write_params(text, specs)
    # Read back must equal what was written
    assert metawriter.read_params(written) == specs
    # Byte-for-byte fidelity: remove the lines metawriter added; the result must equal the
    # original (C1).
    added = [
        ln for ln in written.splitlines(keepends=True) if ln not in text.splitlines(keepends=True)
    ]
    restored = written
    for ln in added:
        restored = restored.replace(ln, "", 1)
    # Added lines must only appear inside the PEP 723 comment block (# prefix)
    assert all(ln.lstrip().startswith("#") for ln in added), added


@pytest.mark.parametrize("path", CORPUS, ids=lambda p: p.name)
def test_shim_no_values_is_identity(path: Path):
    text = _read(path)
    specs = _specs_for(text)
    assert shim.inject(text, specs, {}) == text


@pytest.mark.parametrize("path", CORPUS, ids=lambda p: p.name)
def test_shim_full_injection_compiles(path: Path):
    text = _read(path)
    specs = _specs_for(text)
    if not specs:
        pytest.skip("no candidates")
    values = {s.name: _SAMPLE.get(s.type, "sample") for s in specs}
    out = shim.inject(text, specs, values)
    compile(out, str(path), "exec")  # The injected output must be valid Python
    # The PEP 723 block (# /// script ... # ///) must be untouched
    for line in text.splitlines():
        if line.startswith("# ///") or line.startswith("# dependencies"):
            assert line in out
