r"""Shell injector: run-time value delivery for bash/sh/zsh (the correctness-critical half of A2).

Three deliveries, one entry point (`inject`), mirroring `langs/python/shim.py`'s discipline
(ShimError vs ShimValueError, per-call-site binding via the shared `callmatch`, temp-file secret
handling) but with shell's own hazards:

- **envdefault → environment overlay.** Zero source rewrite, no temp copy, `$0` untouched. The
  preferred channel wherever it exists (and `skit params … --normalize NAME` exists to *create* it).
- **const → byte-span rewrite of the assignment's VALUE node**, on a temp copy. **Quoting is
  normalized, never preserved**: an int/float that coerces becomes a bare word, everything else
  becomes a POSIX single-quoted literal (`'` → `'\\''`). A literal has no expansion to preserve, so
  normalizing loses nothing — and it closes shell injection outright (risk #3): inside single quotes
  no character is special to any POSIX shell, so `$(…)`, backticks and `;` are inert text. Every
  same-name top-level occurrence is rewritten (mirrors python's `_const_targets`).
- **read → per-CALL-SITE rewrite.** Only the `read` command-name token is replaced, by
  `_skit_read <K> '<value>' <0|1> '<prompt>'`; every original flag, varname and redirect survives
  untouched after it. `K` is the call site's index in the CURRENT source, resolved through
  `callmatch.match_calls` (prompt first, position only as a fallback — shared with reconcile), so a
  value can never silently land on the wrong prompt (risk #2), and a second claimant on one site is
  reported as drift (risk #7) instead of splicing two replacements over one span.

The preamble (inserted after the shebang line, only when a read is actually intercepted) is
**POSIX-portable by necessity**, and every line of it was verified on real shells:

- macOS `/bin/bash` is **3.2**: no associative arrays, no `[[ -v ]]`. Per-site one-shot state
  therefore lives in plain `_skit_used_<K>` variables, set and read through `eval` (with a `-`
  default so `set -u` is safe).
- `command read` **fails in zsh** (zsh's `command` bypasses builtins, so the read never runs and the
  variable comes back empty). `builtin read` works in bash 3.2/5, zsh and macOS `/bin/sh` — but
  **dash has no `builtin`**. So the fall-through keyword is dialect-selected: `builtin` for
  bash/zsh, `command` for sh/dash/anything else (POSIX guarantees `command`).
- The queued value arrives as an ARGUMENT (single-quote-escaped exactly like a const value) and is
  fed to the real `read` through an unquoted heredoc whose body is just `$_sv` — a parameter
  expansion result, so it is never re-scanned for quotes or escapes.
- The prompt+value echo is skit's own `printf` (masked to `***` when secret), because `read -p`
  prints its prompt only when stdin is a terminal — and under the heredoc it isn't. On the *second*
  call to the same site the wrapper falls through to the real, unpatched `read`, so a read inside a
  loop keeps consuming real stdin exactly as it always did.

**The delivery contract: what is in the form is what the script gets — byte for byte, or nothing.**
A `read` line is not a value channel; it is parsed. So a value is either escaped so it survives that
parsing intact, or refused outright (`InjectSplitError`) — never silently mangled:

- Without `-r`, `read` UNESCAPES backslashes in the line it consumes, and a backslash before skit's
  own join separator would escape it, merging two fields (form values `C:\` + `Doe` arrived as
  FIRST="C: Doe", LAST=""). Each backslash is therefore doubled for a non-`-r` read, so the shell's
  own unescaping reproduces the value exactly. `-r` reads take the line literally and are left alone.
- A newline ENDS the line, so no variable can carry one — refused everywhere, single-variable reads
  included. Space/tab split the line and are stripped from its edges — refused in any non-last
  variable, and at the edges of the last. A carriage return is neither, and is delivered intact.
- A read that REFRAMES its input (`-n`/`-N`/`-d`) or redefines `IFS` cannot receive a value through
  one line at all, so the analyzer never offers it as a candidate (`read -a`'s existing degradation).

**Dual syntax gates** run before anything launches (risks #3/#4): a mandatory offline tree-sitter
re-parse (`has_error` ⇒ InjectSyntaxError, temp copy removed), and — when the interpreter is
installed — `<interpreter> -n <tmp>`, which parses without executing. A missing interpreter never
fails the run here (preflight owns that error).
"""

from __future__ import annotations

import math
import shutil
import subprocess
from dataclasses import dataclass
from typing import TYPE_CHECKING

from tree_sitter import Parser

from ...callmatch import match_calls
from ...i18n import gettext
from ...rewrite import ByteSpan, apply_byte_spans, line_start_table, write_injected
from ..base import (
    InjectError,
    InjectGapError,
    InjectRequest,
    InjectResult,
    InjectSplitError,
    InjectSyntaxError,
    InjectValueError,
)
from .analyzer import (
    _LANGUAGE,
    _arguments,
    _assignment_operator,
    _literal_text,
    _text,
    _toplevel_assignments,
    _uses_self_location,
    injectable_reads,
)

if TYPE_CHECKING:
    from pathlib import Path

    from tree_sitter import Node

    from ...params import ParamDecl

# Shells whose `command` builtin would bypass the `read` builtin (zsh) or that ship `builtin`
# anyway (bash): for these the wrapper falls through with `builtin read`. Everything else —
# sh, dash, ash, ksh, an unrecognized name — gets POSIX `command read`, which dash requires.
_BUILTIN_SHELLS = ("bash", "zsh")

# The interpreter syntax gate is a parse-only run (`-n`), so it is fast; the timeout is a
# liveness guard, not a policy — a hung gate must never wedge a run.
_GATE_TIMEOUT = 30.0

# What a `read` line does to a value, measured on bash 3.2/5, sh, zsh and dash — three DIFFERENT
# behaviours, so one refusal set would be wrong in both directions:
#
# - A newline **ends the line**. `read` consumes one line, so everything after the first newline is
#   silently discarded — for EVERY variable, including a single-variable `read NAME`. No variable
#   can absorb it, so a newline is refused everywhere.
# - Space and tab are the default $IFS field separators: they split the line across the variables,
#   and `read` also STRIPS them from the line's leading and trailing edges. So they are refused in
#   any non-last variable (the value would spill into the next one), and refused at the EDGES of the
#   last variable (they would be silently trimmed). Interior spaces in the last variable are fine —
#   it takes the remainder of the line verbatim ("de Lovelace" arrives whole).
# - A carriage return is neither: it is not in the default $IFS and does not end the line — every
#   shell delivers `a\rb` intact (verified with od). Refusing it was a false positive; it is not
#   in either set.
_LINE_BREAK = frozenset("\n")
_FIELD_SPLIT = frozenset(" \t")


def inject(request: InjectRequest) -> InjectResult:
    """Deliver `request.values` for a shell entry.

    Returns the injected temp copy (or None when every value went out through the environment —
    the `$0`-safe path), the environment overlay, and any warnings the caller must emit.

    Raises `InjectError` when an injection target no longer exists or two definitions claim one
    read site (drift), `InjectValueError` when a value doesn't fit its declared int/float type,
    `InjectGapError` on a positional gap in a multi-variable read, and `InjectSyntaxError` when a
    post-injection syntax gate rejects the result (nothing is launched; the temp copy is removed).
    """
    text = request.text
    root = _root(text)
    env: dict[str, str] = {}
    spans: list[ByteSpan] = []
    missing: list[str] = []
    sites = _read_sites(root)
    queue: dict[int, str] = {}  # resolved call-site order -> the value to feed it
    secret_orders: set[int] = set()
    stored = [
        (spec.order, spec.prompt)
        for spec in request.specs
        if spec.binding == "input" and spec.name in request.values
    ]
    bindings = match_calls(stored, [(site.order, site.prompt) for site in sites])

    for spec in request.specs:
        if spec.name not in request.values:
            continue  # no value received: leave it alone (preserve the script's own behavior)
        raw = request.values[spec.name]
        if spec.binding == "envdefault":
            # Zero rewrite: the script's own ${NAME:-default} reads it back out of the child's
            # environment. Nothing about the source changes, so $0 stays the real file.
            env[spec.env_var] = raw
            continue
        if spec.binding == "input":
            order = _claim_site(spec, bindings, queue, missing)
            if order is None:
                continue
            queue[order] = raw
            if spec.secret:
                secret_orders.add(order)
            continue
        # const (the only remaining source-anchored binding): every same-name top-level
        # occurrence takes the new value, so the name holds it however the script ends up
        # running — exactly what python's shim does for a module-level constant.
        targets = _const_targets(root, spec.name)
        if not targets:
            missing.append(spec.name)
            continue
        literal = _const_literal(raw, spec.type, spec.name)
        spans.extend(ByteSpan(node.start_byte, node.end_byte, literal) for node in targets)

    if missing:
        raise InjectError(", ".join(missing))
    spans.extend(_read_spans(sites, queue, secret_orders))
    if not spans:
        # Nothing to rewrite (env-only delivery, or no values at all): no temp copy is written
        # AT ALL — the run launches the original file, and $0 is whatever it always was.
        return InjectResult(env=env)

    out = apply_byte_spans(text, spans)
    if queue:
        out = _insert_preamble(out, text, _fallthrough_keyword(request.interpreter))
    _gate_reparse(out)  # gate 1 (mandatory, offline): never launch text we corrupted
    path = write_injected(request.entry_dir, out, suffix=".sh")
    try:
        _gate_interpreter(request.interpreter, path)  # gate 2 (hardening): `<shell> -n`
    except BaseException:
        path.unlink(
            missing_ok=True
        )  # a refused copy must never be left behind (it may hold secrets)
        raise
    return InjectResult(path=path, env=env, warnings=_warnings(root))


# ---------------------------------------------------------------- parsing


def _root(text: str) -> Node:
    data = text.encode("utf-8")  # pragma: no mutate — "utf-8"/"UTF-8" are the same codec alias
    return Parser(_LANGUAGE).parse(data).root_node


# ---------------------------------------------------------------- const


def _const_targets(root: Node, name: str) -> list[Node]:
    """The VALUE nodes of every top-level literal assignment to `name` — the exact set the
    analyzer counted as this const's candidate (same predicate: plain `=`, non-empty literal RHS,
    never a readonly one), so what is offered as a form field is what gets rewritten."""
    out: list[Node] = []
    for node, readonly in _toplevel_assignments(root):
        if readonly:
            continue  # a readonly can't be reassigned; the analyzer never offered it either
        name_node = node.child_by_field_name("name")
        value_node = node.child_by_field_name("value")
        if name_node is None or name_node.type != "variable_name" or value_node is None:
            continue
        if _text(name_node) != name or _assignment_operator(node) != "=":
            continue  # a different name, or `+=` (an accumulator, never a const target)
        if _literal_text(value_node):
            out.append(value_node)
    return out


def _const_literal(raw: str, type_name: str, param_name: str) -> str:
    """The source text to splice in place of a const's value. Quoting is NORMALIZED, never
    preserved: a coercing int/float becomes a bare word; anything else becomes a single-quoted
    POSIX literal, inside which no character is special to any shell (this is what closes the
    injection hole — a payload like `$(touch pwned)` stays inert text).

    "Anything else" includes a hand-declared `type = "bool"` (the analyzer never infers one — shell
    has no boolean type): it delivers the value's TEXT, `'true'` / `'false'`, because text is all a
    shell variable can hold. Python's `repr(True)` -> `True` would mean nothing here."""
    if type_name == "int":
        try:
            return str(int(raw))
        except ValueError as exc:
            raise InjectValueError(raw, type_name, param_name) from exc
    if type_name == "float":
        try:
            number = float(raw)
        except ValueError as exc:
            raise InjectValueError(raw, type_name, param_name) from exc
        if math.isnan(number) or math.isinf(number):
            # `X=inf` is a perfectly valid shell word, but it is not the number the user meant —
            # refuse it explicitly, exactly like python's shim refuses a non-finite repr().
            raise InjectValueError(raw, type_name, param_name)
        return repr(number)
    return quote(raw)


def quote(value: str) -> str:
    """POSIX single-quoting: close, escape, reopen (`'` → `'\\''`). The one escaper — const values,
    read values and read prompts all go through it, so there is a single place quoting can be right
    (or wrong), and a single place a test can monkeypatch to prove the syntax gate bites."""
    return "'" + value.replace("'", "'\\''") + "'"


# ---------------------------------------------------------------- read


@dataclass(frozen=True)
class _ReadSite:
    """One managed `read` VARIABLE: the analyzer numbers candidates per varname (`read FIRST LAST`
    is two), while the rewrite is per COMMAND — so a site carries both keys."""

    order: int  # the candidate's B1 order key (unique across the file)
    command: int  # which read command, in source order (the call-site identity K)
    node: Node  # that command's node
    prompt: str
    secret: bool  # the analyzer's certainty (`read -s`), before the spec's own override
    raw: bool  # this read's -r: whether backslashes in the fed line are literal (see _feed_value)


def _read_sites(root: Node) -> list[_ReadSite]:
    """Every interactive read varname, numbered exactly like `analyzer._read_candidates` — the
    numbering IS the match key, so the two must not be able to disagree. They share the SAME
    enumerator (`analyzer.injectable_reads`) precisely so they cannot: every past divergence (an
    excluded read the injector still counted, shifting every later site) was a silent-wrong-value
    bug."""
    out: list[_ReadSite] = []
    order = 0
    for command, (node, flags) in enumerate(injectable_reads(root)):
        for _varname in flags.varnames:
            out.append(
                _ReadSite(
                    order=order,
                    command=command,
                    node=node,
                    prompt=flags.prompt,
                    secret=flags.secret,
                    raw=flags.raw,
                )
            )
            order += 1
    return out


def _claim_site(
    spec: ParamDecl,
    bindings: dict[int, tuple[int, bool]],
    queue: dict[int, str],
    missing: list[str],
) -> int | None:
    """Resolve one input spec to a call site in the CURRENT source, or record it as missing.

    Mirrors the python shim exactly: no match at all is drift (never drop a value into a black
    hole), and a site a previous spec already claimed is ALSO drift (risk #7) — two replacements
    over one command-name span would corrupt the injected copy into unparsable text, so a second
    claimant must never reach apply_byte_spans."""
    binding = bindings.get(spec.order)
    if binding is None:
        missing.append(spec.name)
        return None
    resolved_order, _ambiguous = binding
    if resolved_order in queue:
        missing.append(spec.name)
        return None
    return resolved_order


def _read_spans(
    sites: list[_ReadSite], queue: dict[int, str], secret_orders: set[int]
) -> list[ByteSpan]:
    """One span per intercepted read COMMAND: replace just its command-name token.

    A `read` consumes one LINE and splits it on IFS, so a multi-variable read's values are joined
    with a single space, in variable order — which is precisely what typing them at the prompt
    would have produced. They must form a contiguous prefix (see InjectGapError)."""
    grouped: dict[int, list[_ReadSite]] = {}
    for site in sites:
        grouped.setdefault(site.command, []).append(site)
    spans: list[ByteSpan] = []
    for command, group in grouped.items():
        supplied = [queue.get(site.order) for site in group]
        if all(value is None for value in supplied):
            continue  # nothing managed on this call: it keeps reading real stdin
        _check_prefix(group, supplied)
        line = " ".join(
            _feed_value(value, raw=group[0].raw) for value in supplied if value is not None
        )
        # `strict=True` is a defensive no-op: supplied is built one-per-site from group just above,
        # so the two are provably equal-length and strict True/False/None behave identically. Pin
        # the zip to its own line so only that expression is suppressed — the `any(...)` predicate
        # below (whose `or` IS a real, tested mutation) stays mutated.
        # pragma: no mutate start
        pairs = zip(group, supplied, strict=True)
        # pragma: no mutate end
        secret = any(
            site.secret or site.order in secret_orders for site, value in pairs if value is not None
        )
        start, end = _command_name_span(group[0].node)
        replacement = f"_skit_read {command} {quote(line)} {int(secret)} {quote(group[0].prompt)}"
        spans.append(ByteSpan(start, end, replacement))
    return spans


def _check_prefix(group: list[_ReadSite], supplied: list[str | None]) -> None:
    """Refuse any value a `read` line would deliver DIFFERENTLY from what the user typed.

    The one contract this guard exists for: skit never silently hands the script a value other than
    the one in the form. `read` re-splits the single line it is fed (see the _LINE_BREAK /
    _FIELD_SPLIT note above), so three distinct refusals fall out — and the last variable is only
    exempt from the FIELD-SPLIT rule, never from the line-break one.

    The last-variable exemption is keyed on the read's last VARIABLE (len-1), never on the last
    *supplied* value: `supplied` always has one slot per variable, so a trailing None is an UNMANAGED
    variable the shell still binds from the same line. Keying it on the last supplied value instead
    let `read FIRST LAST` with only input-1 managed accept "John Paul" and silently deliver
    FIRST="John", LAST="Paul".
    """
    last_index = len(supplied) - 1
    for i, value in enumerate(supplied):
        # `value != ""` vs any other sentinel is unobservable: the only values it re-routes are ""
        # and the sentinel itself, and _split_reason returns "" (no hazard) for both.
        if value is not None and value != "":  # pragma: no mutate
            reason = _split_reason(value, is_last=i == last_index)
            if reason:
                raise InjectSplitError(_display_name(group[i]), reason)
    # A gap is an unmanaged variable (None) OR a managed empty string that isn't the last variable:
    # an empty non-last value contributes nothing to the joined line, so the next field's value would
    # shift up into it — the same wrong binding as a None gap. flows never sends an empty value (it
    # omits them), so this only bites a direct inject() caller, but the injector must be correct on
    # its own, not by relying on a downstream layer's invariant. A trailing empty is fine — a short
    # line, which read handles.
    for i, value in enumerate(supplied):
        if value is not None and (value != "" or i == last_index):
            continue
        later = next(
            (j for j in range(i + 1, len(supplied)) if supplied[j] not in (None, "")), None
        )
        if later is not None:
            raise InjectGapError(_display_name(group[i]), _display_name(group[later]))
        return  # nothing filled after this gap: a short line, which read handles


def _feed_value(value: str, *, raw: bool) -> str:
    """One value, escaped for the line this `read` will parse.

    Without `-r` (the textbook form the analyzer offers), `read` PROCESSES backslashes in the line it
    consumes: `\\x` becomes `x`, and — worse — a backslash before skit's own join separator escapes
    it, merging two fields and emptying the next (form values C:\\ + Doe silently arrived as
    FIRST="C: Doe", LAST=""). Doubling each backslash makes `read`'s own unescaping reproduce the
    value byte-for-byte, which is the whole contract: what is in the form is what the script gets.

    With `-r` the line is taken literally, so the value is already byte-exact and must NOT be
    doubled."""
    return value if raw else value.replace("\\", "\\\\")


def _split_reason(value: str, *, is_last: bool) -> str:
    """Why a `read` line would mangle this value — "" when it arrives intact.

    "line-break": a newline ends the line, so everything after it is discarded — true for EVERY
    variable, a single-variable read included.
    "field-split": a space/tab in a non-last variable spills the remainder into the next one.
    "edge-space": a leading/trailing space or tab, which `read` strips from the line's edges (only
    reachable on the last variable; interior spaces there arrive whole).
    """
    if any(ch in _LINE_BREAK for ch in value):
        return "line-break"
    if not is_last:
        return "field-split" if any(ch in _FIELD_SPLIT for ch in value) else ""
    if value[:1] in _FIELD_SPLIT or value[-1:] in _FIELD_SPLIT:
        return "edge-space"
    return ""


def _display_name(site: _ReadSite) -> str:
    """The form key the analyzer gives this read variable (`input-1`, …) — how the user knows it."""
    return f"input-{site.order + 1}"


def _command_name_span(node: Node) -> tuple[int, int]:
    """The byte span of the command NAME to replace: `read`, or the whole `builtin read` /
    `command read` pair (the analyzer accepts both spellings, so the injector must rewrite the
    keyword too — `builtin _skit_read` would call the wrapper as a builtin and fail)."""
    name_node = node.child_by_field_name("name")
    if name_node is None:  # pragma: no cover — a `command` node always has a command_name
        raise InjectError("read")  # pragma: no mutate — unreachable guard (see no cover above)
    if _text(name_node) in ("builtin", "command"):
        return name_node.start_byte, _arguments(node)[0].end_byte
    return name_node.start_byte, name_node.end_byte


# ---------------------------------------------------------------- preamble


def _fallthrough_keyword(interpreter: str) -> str:
    """`builtin` for bash/zsh, `command` for everything else (see the module docstring: `command
    read` silently does nothing in zsh, and dash has no `builtin` at all)."""
    name = interpreter.rsplit("/", 1)[-1].removesuffix(".exe")  # pragma: no mutate (maxsplit inert)
    return "builtin" if name in _BUILTIN_SHELLS else "command"


def _preamble(keyword: str) -> str:
    """The `_skit_read` wrapper: one-shot per call site, then the real read.

    POSIX-only constructs (bash 3.2 / dash / zsh all run this verbatim). The heredoc body and its
    `EOF` terminator MUST stay unindented — a heredoc delimiter is only matched at column 0 unless
    `<<-` is used, and `<<-` would strip tabs from the VALUE too."""
    return (
        "_skit_read() {\n"
        "  _sk=$1; _sv=$2; _ss=$3; _sp=$4; shift 4\n"
        '  eval "_su=\\${_skit_used_$_sk-}"\n'
        '  if [ -z "$_su" ]; then\n'
        '    eval "_skit_used_$_sk=1"\n'
        "    if [ \"$_ss\" = 1 ]; then printf '%s%s\\n' \"$_sp\" '***'; "
        'else printf \'%s%s\\n\' "$_sp" "$_sv"; fi\n'
        f'    {keyword} read "$@" <<EOF\n'
        "$_sv\n"
        "EOF\n"
        "  else\n"
        f'    {keyword} read "$@"\n'
        "  fi\n"
        "}  # skit:shim\n"
    )


def _insert_preamble(out: str, original: str, keyword: str) -> str:
    """Insert the wrapper after the shebang line (or at the very top when there is none).

    The offset is computed on the ORIGINAL text and applied to the already-spliced one: every
    const/read span starts at or after it (line 1 of a shell script is either a shebang comment or
    the insertion point itself), so the bytes before it are identical in both. And a preamble is
    only ever inserted when a read was intercepted, which guarantees a line exists after the
    shebang — so `line_start_table(...)[1]` is a real line start, never the end-of-file sentinel."""
    offset = line_start_table(original)[1] if original.startswith("#!") else 0
    return apply_byte_spans(out, [ByteSpan(offset, offset, _preamble(keyword))])


# ---------------------------------------------------------------- gates & warnings


def _gate_reparse(out: str) -> None:
    """Gate 1 (mandatory, offline): the injected text must still parse. This is what stands
    between a quoting bug and a corrupted script running with the user's real arguments."""
    if _root(out).has_error:
        raise InjectSyntaxError(
            gettext("the injected copy no longer parses as a shell script (nothing was run)")
        )


def _gate_interpreter(interpreter: str, path: Path) -> None:
    """Gate 2 (hardening): `<interpreter> -n <file>` — bash/sh/zsh all parse without executing a
    single line. Skipped when the interpreter isn't installed: that is preflight's error to raise,
    not a reason to refuse an injection the offline gate already accepted."""
    program = shutil.which(interpreter) if interpreter else None
    if program is None:
        return
    try:
        proc = subprocess.run(  # noqa: S603 — argv list, interpreter resolved from PATH; `-n` never executes the script
            [program, "-n", str(path)],
            capture_output=True,
            check=False,
            timeout=_GATE_TIMEOUT,
        )
    except (OSError, subprocess.SubprocessError):
        return  # the gate itself couldn't run; gate 1 already vouched for the text
    if proc.returncode != 0:
        detail = proc.stderr.decode(errors="replace").strip().splitlines()
        raise InjectSyntaxError(
            gettext("%(shell)s rejected the injected copy: %(detail)s")
            % {"shell": interpreter, "detail": detail[0] if detail else ""}
        )


def _warnings(root: Node) -> list[str]:
    """The `$0` caveat: a rewritten script runs from a temp copy, so a script that asks where it
    lives gets a different answer. Only ever reached when a temp copy is actually written — the
    env-delivery path returns before this, which is exactly why normalization is worth offering."""
    if not _uses_self_location(root):
        return []
    # The advice must be true in BOTH storage modes without this layer knowing the
    # mode (injection is language machinery, not entry policy): the manual
    # ${NAME:-default} spelling works everywhere, and --normalize is named as the
    # shortcut that performs it — on a stored copy; a reference entry's refusal
    # already redirects to the manual edit.
    return [
        gettext(
            "⚠ This script reads its own location ($0 / $BASH_SOURCE), and the injected values "
            "run from a temporary copy — so it sees the copy's path, not the original's. "
            'Rewriting a constant as NAME="${NAME:-value}" delivers the value through the '
            "environment instead, with no copy at all (`skit params <script> --normalize NAME` "
            "does the rewrite for you on a stored copy)."
        )
    ]
