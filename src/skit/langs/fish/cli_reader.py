"""Static fish `argparse` reader: turn the builtin ``argparse`` spec strings into form fields.

fish's own option parser is the builtin ``argparse 'h/help' 'n/name=' … -- $argv``. Each quoted
token before the ``--`` is an OPTION SPEC; skit reads them statically and assembles the long
flag (fish always accepts ``--long`` when a long name exists). The fish analogue of the argparse
/ parseArgs / param() readers.

Spec grammar (fish docs), read here:
- ``x/long`` short+long, ``long`` long-only, ``x`` short-only, ``x-long`` "dummy short" (the
  short is reserved but not usable on the command line — only ``--long``). The separator between
  a single short char and the long name is one of ``/`` ``-`` ``#`` (at position 1); a name whose
  second character is none of those is a plain long name (so ``dry-run`` stays long-only).
- value suffixes: ``=`` (required value → str), ``=?`` (optional attached value → str),
  ``=+`` / ``=*`` (repeat → multiple str). No value suffix ⇒ a boolean store_true flag.
- ``!validator`` is stripped (skit doesn't run validators). ``#`` (implicit-integer flag) is not
  modeled — the field degrades to free-text.
- argparse's OWN options (``-n``/``--name``, ``-x``, ``-i``, ``-s``, ``-N``/``--min-args`` …)
  precede the specs and are skipped. Only the FIRST ``argparse`` command is read; none ⇒ None.

Headless, stdlib-only — reuses the analyzer's total scanner.
"""

from __future__ import annotations

from ...params import ParamDecl, is_secret_name
from ..python.argspec import ArgSpec
from . import analyzer

# argparse's own options that consume a following value (so their value is not a spec). Attached
# forms (``--name=foo``) carry the value in the same token, so they never consume the next one.
_VALUE_OWN_OPTS = frozenset(
    {"-n", "--name", "-x", "--exclusive", "-N", "--min-args", "-X", "--max-args"}
)

# The single-char short separators inside a spec name (`h/help`, `x-long`, `m#max`).
_NAME_SEPARATORS = ("/", "-", "#")


def read_cli(text: str) -> ArgSpec | None:
    """Read the first ``argparse … -- $argv`` command's spec strings into flag-delivery params.
    None when the script has no argparse command at all (callers fall back to the other sources);
    an argparse with no specs is a readable zero-field surface (`ArgSpec(fields=[])`)."""
    words = _find_argparse(text)
    if words is None:
        return None
    fields: list[ParamDecl] = []
    for token in _spec_tokens(words):
        decl = _read_spec(token)
        if decl is not None:
            fields.append(decl)
    return ArgSpec(fields=fields)


def _find_argparse(text: str) -> list[str] | None:
    """The tokens AFTER the first ``argparse`` command word (skipping leading and/or/not), or
    None when no statement is an argparse call."""
    for words, _lineno in analyzer._statements(text):
        j = 0
        while j < len(words) and words[j] in analyzer._CONDITIONAL_PREFIXES:
            j += 1
        rest = words[j:]
        if rest and rest[0] == "argparse":
            return rest[1:]
    return None


def _spec_tokens(words: list[str]) -> list[str]:
    """The option-spec tokens: argparse's own leading options skipped (consuming a value for the
    value-taking ones), then every token up to the ``--`` end-of-specs marker."""
    i = 0
    n = len(words)
    while i < n:
        w = words[i]
        if w == "--":
            return []  # `argparse -- $argv` — no specs
        if not w.startswith("-"):
            break  # the first non-flag token is the first spec
        i += 2 if (w in _VALUE_OWN_OPTS and i + 1 < n) else 1
    specs: list[str] = []
    while i < n and words[i] != "--":
        specs.append(words[i])
        i += 1
    return specs


def _read_spec(raw: str) -> ParamDecl | None:
    """One option spec → a flag-delivery ParamDecl, or None for an unparseable/garbage spec."""
    spec = analyzer._dequote(raw).split("!", 1)[0].strip()  # dequote, then drop a !validator
    if not spec:
        return None
    multiple = False
    has_value = False
    if spec.endswith(("=+", "=*")):
        has_value, multiple, core = True, True, spec[:-2]
    elif spec.endswith("=?"):
        has_value, core = True, spec[:-2]
    elif spec.endswith("="):
        has_value, core = True, spec[:-1]
    else:
        core = spec
    parsed = _parse_name(core)
    if parsed is None:
        return None
    name, flag, numeric = parsed
    decl = ParamDecl(
        name=name, binding="none", delivery="flag", flag=flag, secret=is_secret_name(name)
    )
    if numeric:
        decl.degraded = True  # `#` implicit-integer flag — not modeled; free-text fallback
    elif has_value:
        decl.type = "str"
        decl.multiple = multiple
    else:
        decl.type = "bool"
        decl.action = "store_true"
        decl.default = False
    return decl


def _parse_name(core: str) -> tuple[str, str, bool] | None:
    """(name, assembled flag, is-numeric) for a spec's name part. The flag prefers the long name
    (`x/long`, `x-long`, `m#max` → ``--long``/``--max``); a single-char spec is short-only
    (``-x``); a multi-char spec with no separator at position 1 is a plain long name
    (``dry-run`` → ``--dry-run``). A bare/leading separator is garbage → None."""
    if not core:
        return None
    if len(core) >= 2 and core[1] in _NAME_SEPARATORS:
        short, long, numeric = core[0], core[2:], core[1] == "#"
        if long:
            return long, f"--{long}", numeric
        return short, f"-{short}", numeric  # `x/` — empty long, fall back to the short
    if core[0] in _NAME_SEPARATORS:
        return None  # a leading separator (`#max`, `/x`) — not a nameable flag
    if len(core) == 1:
        return core, f"-{core}", False
    return core, f"--{core}", False
