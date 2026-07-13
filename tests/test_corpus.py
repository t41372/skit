"""Golden corpus fidelity tests (C series), across every analyzable language.

Language-blind checks run over both the Python (`corpus/*.py`) and shell (`corpus/shell/*.sh`)
corpora — each parametrized as (analyze, path):

1. analyzer never raises: any script yields candidates (or an empty list) without exceptions.
2. metawriter byte fidelity: write [tool.skit] then read it back; every line the writer ADDS is a
   comment line (the `#`-block engine is language-blind — it never touches code), and the read-back
   round-trips the specs (C1).
3. block round-trip preserves shebang position: an injected `# /// script` block lands AFTER the
   shebang, and every code (non-comment) line survives verbatim.

Python injector checks:

4. shim with no values is the identity: inject(text, specs, {}) returns the exact same bytes.
5. shim-injected output still compiles: inject a type-compatible sample value for each candidate and
   verify compile() accepts the result, with the PEP 723 block untouched.

Shell injector checks (the same two claims, in shell's terms):

6. no values ⇒ no temp copy at all (path None, empty env): the injector never rewrites for free.
7. full injection re-parses: inject a type-compatible sample value for every candidate and verify
   the result has no tree-sitter error (the mandatory gate, re-asserted here on the golden set) —
   and that a const-only file's [tool.skit]-style comment lines survive untouched. Files whose
   candidates are ALL envdefaults deliver purely by environment and correctly write no copy.

Corpus coverage — Python: shebang/coding, docstring, __future__, main guard, CRLF, tab indentation,
no trailing newline, CJK, input() in loops/comprehensions, walrus, decorator, async, argparse.
Shell: word/number/raw/double-quoted consts, export/readonly/declare/local, envdefault :-/:=/-/=,
the both-assigned-and-defaulted suppression, self-idiom, reads (clustered/multi-var/dynamic/retry),
data-reads (pipe/redirect-fed), heredoc, demotions, argv/self-location hints, CJK+emoji, CRLF, no
trailing newline, a zsh dialect tree-sitter-bash can't parse (honest empty), a function-read defined
above but invoked after a top-level read (risk #2), $0 next to an injectable const, and every
quoting shape a const RHS can take (risk #3).
"""

from collections.abc import Callable
from pathlib import Path

import pytest

from skit.analysis import Analysis
from skit.langs.base import InjectRequest
from skit.langs.javascript import analyzer as js_analyzer
from skit.langs.javascript import inject as js_inject
from skit.langs.javascript import io as js_io
from skit.langs.python import analyzer as py_analyzer
from skit.langs.python import metawriter, shim
from skit.langs.shell import analyzer as sh_analyzer
from skit.langs.shell import inject as sh_inject
from skit.params import ParamDecl

PY_CORPUS = sorted((Path(__file__).parent / "corpus").glob("*.py"))
SH_CORPUS = sorted((Path(__file__).parent / "corpus" / "shell").glob("*.sh"))
JS_CORPUS = sorted((Path(__file__).parent / "corpus" / "js").glob("*.mjs"))
TS_CORPUS = sorted((Path(__file__).parent / "corpus" / "ts").glob("*.ts"))

# (id, lang, path) triples for the JS/TS corpus checks — each file parsed under its kind's grammar.
_JS_TS = [(f"js:{p.name}", "js", p) for p in JS_CORPUS] + [
    (f"ts:{p.name}", "ts", p) for p in TS_CORPUS
]
_JS_TS_IDS = [entry[0] for entry in _JS_TS]
_JS_TS_ARGS = [(entry[1], entry[2]) for entry in _JS_TS]

# (id, analyze, path) triples for the language-blind checks.
_NEUTRAL = [(f"py:{p.name}", py_analyzer.analyze, p) for p in PY_CORPUS] + [
    (f"sh:{p.name}", sh_analyzer.analyze, p) for p in SH_CORPUS
]
_NEUTRAL_IDS = [entry[0] for entry in _NEUTRAL]
_NEUTRAL_ARGS = [(entry[1], entry[2]) for entry in _NEUTRAL]

# Sample values used when exercising full-value injection
_SAMPLE = {"str": "sample", "int": "7", "float": "1.5", "bool": "true"}


def _read(path: Path) -> str:
    # newline="" semantics: preserve CRLF as-is (splitlines(keepends=True) honours that too).
    # Use open(newline="") instead of Path.read_text(newline=) — the latter is a 3.13+ API and
    # this project's floor is 3.12.
    with path.open(encoding="utf-8", newline="") as f:
        return f.read()


def _specs_for(text: str, analyze: Callable[[str], Analysis]) -> list[ParamDecl]:
    # ParamDecl.from_candidate is the field-aligned conversion this test used to spell out
    # by hand; it carries exactly the same fields, so byte fidelity is unchanged.
    return [ParamDecl.from_candidate(c) for c in analyze(text).candidates]


@pytest.mark.parametrize(("analyze", "path"), _NEUTRAL_ARGS, ids=_NEUTRAL_IDS)
def test_analyzer_never_raises(analyze: Callable[[str], Analysis], path: Path):
    analyze(_read(path))


@pytest.mark.parametrize(("analyze", "path"), _NEUTRAL_ARGS, ids=_NEUTRAL_IDS)
def test_metawriter_byte_fidelity(analyze: Callable[[str], Analysis], path: Path):
    text = _read(path)
    specs = _specs_for(text, analyze)
    written = metawriter.write_params(text, specs)
    # Read back must equal what was written
    assert metawriter.read_params(written) == specs
    # Byte-for-byte fidelity: every line the writer ADDED must be inside the comment block (# prefix)
    # — the '#'-comment block engine is language-blind, so this holds for shell exactly as for python.
    added = [
        ln for ln in written.splitlines(keepends=True) if ln not in text.splitlines(keepends=True)
    ]
    assert all(ln.lstrip().startswith("#") for ln in added), added


@pytest.mark.parametrize(("analyze", "path"), _NEUTRAL_ARGS, ids=_NEUTRAL_IDS)
def test_block_roundtrip_preserves_shebang(analyze: Callable[[str], Analysis], path: Path):
    text = _read(path)
    specs = _specs_for(text, analyze)
    if not specs:
        pytest.skip("no candidates → no block written")
    written = metawriter.write_params(text, specs)
    lines = text.splitlines(keepends=True)
    if lines and lines[0].startswith("#!"):
        # The shebang stays on line 1 and the injected block opens strictly after it.
        assert written.splitlines(keepends=True)[0] == lines[0]
        assert written.index("#!") < written.index("# /// script")
    # metawriter only ever touches comment lines: every code (non-comment) line survives verbatim.
    for ln in lines:
        if not ln.lstrip().startswith("#"):
            assert ln in written


@pytest.mark.parametrize("path", PY_CORPUS, ids=lambda p: p.name)
def test_shim_no_values_is_identity(path: Path):
    text = _read(path)
    specs = _specs_for(text, py_analyzer.analyze)
    assert shim.inject(text, specs, {}) == text


@pytest.mark.parametrize("path", PY_CORPUS, ids=lambda p: p.name)
def test_shim_full_injection_compiles(path: Path):
    text = _read(path)
    specs = _specs_for(text, py_analyzer.analyze)
    if not specs:
        pytest.skip("no candidates")
    values = {s.name: _SAMPLE.get(s.type, "sample") for s in specs}
    out = shim.inject(text, specs, values)
    compile(out, str(path), "exec")  # The injected output must be valid Python
    # The PEP 723 block (# /// script ... # ///) must be untouched
    for line in text.splitlines():
        if line.startswith("# ///") or line.startswith("# dependencies"):
            assert line in out


@pytest.mark.parametrize("path", SH_CORPUS, ids=lambda p: p.name)
def test_shell_inject_no_values_writes_nothing(path: Path, tmp_path: Path):
    text = _read(path)
    specs = _specs_for(text, sh_analyzer.analyze)
    result = sh_inject.inject(
        InjectRequest(text=text, specs=specs, values={}, entry_dir=tmp_path, interpreter="bash")
    )
    assert result.path is None  # no values -> no rewrite, no temp copy, nothing to clean up
    assert result.env == {}
    assert not list(tmp_path.iterdir())


@pytest.mark.parametrize("path", SH_CORPUS, ids=lambda p: p.name)
def test_shell_full_injection_reparses(path: Path, tmp_path: Path):
    text = _read(path)
    specs = _specs_for(text, sh_analyzer.analyze)
    if not specs:
        pytest.skip("no candidates")
    values = {s.name: _SAMPLE.get(s.type, "sample") for s in specs}
    result = sh_inject.inject(
        InjectRequest(text=text, specs=specs, values=values, entry_dir=tmp_path, interpreter="bash")
    )
    env_names = {s.env_var for s in specs if s.binding == "envdefault"}
    assert set(result.env) == env_names  # envdefaults go out by environment, never by rewrite
    if result.path is None:
        assert env_names == {s.name for s in specs}  # nothing left to rewrite: an env-only script
        return
    try:
        out = result.path.read_text(encoding="utf-8")
    finally:
        result.path.unlink()
    # The injected output must still parse — the same gate inject() itself enforces (it also ran
    # `bash -n` on this very file before returning), re-asserted here over the golden set.
    assert not sh_analyzer.analyze(out).syntax_error
    # Only code bytes change: every comment line of the original survives verbatim.
    for line in text.splitlines():
        if line.lstrip().startswith("#"):
            assert line in out


# ---------------------------------------------------------------- JS/TS corpus


def _js_specs(text: str, lang: str) -> list[ParamDecl]:
    return [ParamDecl.from_candidate(c) for c in js_analyzer.analyze(text, lang=lang).candidates]


@pytest.mark.parametrize(("lang", "path"), _JS_TS_ARGS, ids=_JS_TS_IDS)
def test_js_analyzer_never_raises(lang: str, path: Path):
    js_analyzer.analyze(_read(path), lang=lang)


@pytest.mark.parametrize(("lang", "path"), _JS_TS_ARGS, ids=_JS_TS_IDS)
def test_js_block_byte_fidelity(lang: str, path: Path):
    text = _read(path)
    specs = _js_specs(text, lang)
    written = js_io.write_params(text, specs)
    # Read back must equal what was written (the `//` block engine round-trips).
    assert js_io.read_params(written) == specs
    # Byte-for-byte fidelity: every line the writer ADDED must be a `//`-comment line — the block
    # engine is language-blind, so this holds for JS/TS exactly as the `#`-engine does for python.
    added = [
        ln for ln in written.splitlines(keepends=True) if ln not in text.splitlines(keepends=True)
    ]
    assert all(ln.lstrip().startswith("//") for ln in added), added


@pytest.mark.parametrize(("lang", "path"), _JS_TS_ARGS, ids=_JS_TS_IDS)
def test_js_inject_no_values_is_identity(lang: str, path: Path, tmp_path: Path):
    text = _read(path)
    specs = _js_specs(text, lang)
    result = js_inject.inject(
        InjectRequest(text=text, specs=specs, values={}, entry_dir=tmp_path, interpreter=""),
        lang=lang,
    )
    assert result.path is None  # no values -> no rewrite, no temp copy, nothing to clean up
    assert result.env == {}
    assert not list(tmp_path.iterdir())


@pytest.mark.parametrize(("lang", "path"), _JS_TS_ARGS, ids=_JS_TS_IDS)
def test_js_full_injection_reparses(lang: str, path: Path, tmp_path: Path):
    text = _read(path)
    specs = _js_specs(text, lang)
    if not specs:
        pytest.skip("no candidates")
    values = {s.name: _SAMPLE.get(s.type, "sample") for s in specs}
    result = js_inject.inject(
        InjectRequest(text=text, specs=specs, values=values, entry_dir=tmp_path, interpreter=""),
        lang=lang,
    )
    assert result.path is not None  # JS delivers every value by temp-copy rewrite (no env channel)
    assert result.env == {}
    try:
        out = result.path.read_text(encoding="utf-8")
    finally:
        result.path.unlink()
    # The injected output must still parse — the same mandatory gate inject() itself enforces.
    assert not js_analyzer.analyze(out, lang=lang).syntax_error
    # Only value bytes change: every `//`-comment line of the original survives verbatim.
    for line in text.splitlines():
        if line.lstrip().startswith("//"):
            assert line in out
