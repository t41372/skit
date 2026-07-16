"""Programmatic i18n coverage — the check you can't do by eye.

Independent measures, each exact enough to gate a commit. Two guard catalog integrity directly:

0.  CATALOG SYNTAX — every msgid/msgstr/msgctxt keyword (and every continuation line) must be a
    quoted string. Babel's lenient parser silently mis-recovers a lost quote — a `msgstr foo"`
    still "compiles" and counts as translated — so pin it at the line level.

0b. PLACEHOLDER PARITY — every translated msgstr must carry the same %-format placeholders its
    msgid does. A fuzzy `update` grafting an unrelated translation (msgstr `%(detail)s` under a
    `%(error)s` msgid) sails past every other check and then crashes with KeyError at format time.

And four measure coverage of the pipeline itself:

1. EXTRACTION FRESHNESS — re-extract the .pot from source and diff its msgid set against
   the committed .pot. A string newly wrapped in gettext() but not re-extracted, or one
   removed from source but left in the catalog, shows up here. (Catches "I added a UI
   string and forgot to run extract/update/compile.")

2. CATALOG COMPLETENESS — for every shipped locale, count msgids with a non-empty,
   non-fuzzy msgstr. Coverage % = translated / total. Lists every untranslated or fuzzy
   entry. English is the identity (msgid == source), so it is 100% by construction and is
   not counted.

3. UNWRAPPED UI LITERALS — AST-scan src/skit for string literals sitting in user-facing
   sinks (Static/Label/RadioButton/Checkbox/Option first arg, .notify/.print/Prompt.ask,
   help=/placeholder=/title=, *border_title/*placeholder/TITLE assignments) that are NOT
   wrapped in gettext()/ngettext(). These never enter the pipeline at all — invisible when
   you test in a single language, which is exactly why eyeballing misses them.

4. DYNAMIC GETTEXT — flag gettext()/ngettext() calls whose message is not a string literal
   (a variable or dict lookup, e.g. gettext(LABELS[kind])). Babel only extracts literals, so
   such a message never becomes a msgid at all: it escapes checks 1 and 2 and silently ships
   the English source in every locale.

Exit code 0 iff every measure is clean. Usage: `uv run python scripts/i18n_coverage.py`.
"""

from __future__ import annotations

import ast
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from babel.messages.pofile import read_po

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src" / "skit"
LOCALES = SRC / "locales"
POT = LOCALES / "skit.pot"
DOMAIN = "skit"


# --------------------------------------------------------------------------- catalogs


def _read_catalog(path: Path):
    with path.open("rb") as f:
        return read_po(f)


def _ids(catalog) -> set[object]:
    return {m.id for m in catalog if m.id}


def _shipped_locales() -> list[Path]:
    return sorted(p.parent.parent for p in LOCALES.glob("*/LC_MESSAGES/skit.po"))


def check_po_syntax() -> list[str]:
    """Flag structurally broken catalog lines pybabel's lenient parser silently mis-recovers.

    A hand-edited `msgstr --ref …"` (opening quote lost) parses "successfully" under Babel and
    even compiles — into a mangled runtime string — while counting as translated for the
    completeness check. msgfmt --check would refuse it, but gettext-tools isn't a dev
    dependency, so pin the invariant at the line level: every msgid/msgstr keyword must be
    followed by a double-quoted string, and every continuation line must be one."""
    problems: list[str] = []
    keyword = re.compile(r"(msgid(?:_plural)?|msgstr(?:\[\d+\])?|msgctxt)\s+(\S.*)$")
    for po in sorted(LOCALES.glob("*/LC_MESSAGES/skit.po")):
        shown = po.relative_to(ROOT) if po.is_relative_to(ROOT) else po
        for lineno, line in enumerate(po.read_text(encoding="utf-8").splitlines(), start=1):
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue  # blank separators and comments (incl. #~ obsolete entries)
            m = keyword.match(stripped)
            if m:
                if not (m.group(2).startswith('"') and m.group(2).endswith('"')):
                    problems.append(f"{shown}:{lineno}: unquoted {m.group(1)}")
            elif not (stripped.startswith('"') and stripped.endswith('"')):
                # A continuation line of a wrapped msgid/msgstr — babel silently TRUNCATES
                # the entry at an unquoted one, which still counts as "translated" for the
                # completeness check, so it must fail here.
                problems.append(f"{shown}:{lineno}: unquoted continuation line")
    return problems


def check_freshness() -> list[str]:
    """Re-extract and diff msgids against the committed .pot."""
    with tempfile.TemporaryDirectory() as td:
        fresh_pot = Path(td) / "fresh.pot"
        rc = subprocess.run(
            [
                sys.executable,
                "-m",
                "babel.messages.frontend",
                "extract",
                "-F",
                "babel.cfg",
                "--no-wrap",
                "-o",
                str(fresh_pot),
                "src/skit",
            ],
            check=False,
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        if rc.returncode != 0:
            return [f"extract failed: {rc.stderr.strip()}"]
        fresh = _ids(_read_catalog(fresh_pot))
    committed = _ids(_read_catalog(POT))
    return [
        f"wrapped in source but MISSING from skit.pot (run i18n.py extract): {mid!r}"
        for mid in sorted(fresh - committed, key=str)
    ] + [
        f"in skit.pot but no longer in source (stale): {mid!r}"
        for mid in sorted(committed - fresh, key=str)
    ]


def check_completeness() -> tuple[list[str], dict[str, float]]:
    """Per-locale translated/untranslated/fuzzy accounting."""
    problems: list[str] = []
    coverage: dict[str, float] = {}
    for locale_dir in _shipped_locales():
        po = locale_dir / "LC_MESSAGES" / "skit.po"
        cat = _read_catalog(po)
        tag = locale_dir.name.replace("_", "-")
        total = untranslated = fuzzy = 0
        for m in cat:
            if not m.id:
                continue  # the header
            total += 1
            strings = m.string if isinstance(m.string, tuple) else (m.string,)
            if not all(strings):
                untranslated += 1
                problems.append(f"[{tag}] untranslated: {m.id!r}")
            elif m.fuzzy:
                fuzzy += 1
                problems.append(f"[{tag}] fuzzy (needs review): {m.id!r}")
        done = total - untranslated - fuzzy
        coverage[tag] = 100.0 * done / total if total else 100.0
    return problems, coverage


# --------------------------------------------------------------- placeholder parity

# Named %-format placeholders — %(installer)s, %(error)s — captured by name (the conversion
# letter/flags after the ")" don't affect which dict key the code must supply).
_NAMED_PLACEHOLDER = re.compile(r"%\((\w+)\)")
# A positional %-conversion (%s, %d) once %% escapes and %(name)s named refs are removed.
_POSITIONAL_PLACEHOLDER = re.compile(r"%[a-zA-Z]")


def _named_placeholders(text: str) -> set[str]:
    return set(_NAMED_PLACEHOLDER.findall(text))


def _positional_conversions(text: str) -> list[str]:
    """The positional %-conversions (%s, %d, …) in source order, once %% escapes and %(name)s
    named refs are removed. The conversion LETTER matters, not just the count: msgstr "%d" under
    a "%s" msgid is a TypeError at format time just as a count mismatch is, so compare the
    ordered tokens, not len()."""
    bare = _NAMED_PLACEHOLDER.sub("", text).replace("%%", "")
    return _POSITIONAL_PLACEHOLDER.findall(bare)


def check_placeholder_parity() -> list[str]:
    """Every translated (non-fuzzy) msgstr must carry the SAME %-format placeholders its msgid
    does. The trap AGENTS.md warns about is a fuzzy `update` grafting an unrelated translation —
    a msgstr referencing `%(detail)s` under a msgid that supplies `%(error)s` — which the
    completeness, freshness and syntax gates all wave through (they never look inside the string),
    then crashes with `KeyError: 'error'` at `%`-format time on the very run that hits it. A
    positional `%s` count mismatch is the same failure as a `TypeError`. This is the check that
    catches both, before the next translation change is where it ships."""
    problems: list[str] = []
    for locale_dir in _shipped_locales():
        po = locale_dir / "LC_MESSAGES" / "skit.po"
        cat = _read_catalog(po)
        tag = locale_dir.name.replace("_", "-")
        for m in cat:
            if not m.id or m.fuzzy:
                continue  # header, and fuzzy entries the completeness gate already flags
            ids = m.id if isinstance(m.id, tuple) else (m.id,)
            id_named = [_named_placeholders(i) for i in ids]
            id_positional = [_positional_conversions(i) for i in ids]
            strings = m.string if isinstance(m.string, tuple) else (m.string,)
            for form in strings:
                if not form:
                    continue  # an untranslated plural form — the completeness gate owns it
                if _named_placeholders(form) not in id_named:
                    problems.append(
                        f"[{tag}] placeholder mismatch: {m.id!r} — msgstr names "
                        f"{sorted(_named_placeholders(form))}, msgid names "
                        f"{[sorted(s) for s in id_named]}"
                    )
                elif _positional_conversions(form) not in id_positional:
                    problems.append(
                        f"[{tag}] positional-placeholder mismatch: {m.id!r} — msgstr has "
                        f"{_positional_conversions(form)}, msgid has {id_positional}"
                    )
    return problems


# ------------------------------------------------------------------- unwrapped literals

# Call sinks: callee simple-name -> positional arg indices that render as UI text.
_CALL_SINKS = {
    "Static": (0,),
    "Label": (0,),
    "RadioButton": (0,),
    "Checkbox": (0,),
    "Option": (0,),
    "Button": (0,),
}
# Method sinks: method name -> positional arg indices.
_METHOD_SINKS = {
    "notify": (0,),
    "print": (0,),
    "ask": (0,),
}
# Keyword args whose value is user-facing text, on any call.
_KW_SINKS = {"help", "placeholder", "title"}
# Assignment targets (attribute/name) whose value is user-facing text.
_ASSIGN_SINKS = ("border_title", "placeholder", "TITLE", "tooltip", "border_subtitle")

# Rich console markup tags ([red], [/dim], [bold $accent], …). These wrap interpolated
# values in f-strings and carry no translatable prose of their own; strip them before the
# prose test so `f"[red]{escape(exc)}[/red]"` (variable content, no gettext) isn't flagged.
_MARKUP_TAG = re.compile(r"\[/?[^\]]*\]")

# Strings that are structurally NOT translatable UI prose: ids, CSS, format tokens, glyphs,
# encodings, single glyphs/punctuation. Kept deliberately tight so real prose can't hide here.
_NON_UI = re.compile(
    r"""^(
        [\W\d_]* |                       # no letters at all (glyphs, punctuation, numbers)
        [a-z][a-z0-9_]*(-[a-z0-9_]+)* |  # lowercase identifier / css-class / slug (single token)
        [#.][\w\-]+ |                    # css id/class selector
        utf-8 | UTF-8 | ascii |          # encodings
        https?://.* )$""",
    re.VERBOSE,
)

# Reviewed, intentionally-untranslated literals: input placeholders that show a universal
# INPUT FORMAT (a path shape, a shell command), not prose. Translating them would be wrong.
_ALLOWED = {
    "~/scripts/tool.py",
    "ffmpeg -i {input} {output}",
    "ffmpeg -i {input}",
}


def _is_ui_prose(s: str) -> bool:
    if s in _ALLOWED:
        return False
    s = _MARKUP_TAG.sub("", s).strip()  # drop rich markup tags, keep the prose between them
    if len(s) < 2 or not any(c.isalpha() for c in s):
        return False
    return not _NON_UI.match(s)


def _wraps_gettext(node: ast.AST) -> bool:
    """True if the node is a gettext()/ngettext() call, or an f-string / concat that contains
    one (so the literal text is produced by translation, not a bare literal)."""
    for n in ast.walk(node):
        if isinstance(n, ast.Call):
            fn = n.func
            name = (
                fn.id
                if isinstance(fn, ast.Name)
                else (fn.attr if isinstance(fn, ast.Attribute) else "")
            )
            if name in ("gettext", "ngettext", "_"):
                return True
    return False


def _bare_ui_strings(node: ast.AST) -> list[str]:
    """The user-facing string literals directly under `node` that are NOT gettext-wrapped."""
    if _wraps_gettext(node):
        return []
    out: list[str] = []
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        if _is_ui_prose(node.value):
            out.append(node.value)
    elif isinstance(node, ast.JoinedStr):  # f-string with no gettext inside
        for part in node.values:
            if (
                isinstance(part, ast.Constant)
                and isinstance(part.value, str)
                and _is_ui_prose(part.value)
            ):
                out.append(part.value)
    return out


def scan_unwrapped(src: Path = SRC) -> list[str]:
    """Report user-facing string literals in UI sinks not wrapped in gettext, under `src`.
    `src` is a parameter so a test can point it at the real repo tree even when the test
    itself runs from a copied (e.g. mutmut) tree whose .py bodies are mutated."""
    problems: list[str] = []
    anchor = src.parent.parent  # repo root, for tidy relative paths
    for py in sorted(src.rglob("*.py")):
        tree = ast.parse(py.read_text(encoding="utf-8"), filename=str(py))
        rel = py.relative_to(anchor) if anchor in py.parents else py.name
        for node in ast.walk(tree):
            hits: list[tuple[int, str]] = []
            if isinstance(node, ast.Call):
                fn = node.func
                cname = fn.id if isinstance(fn, ast.Name) else None
                mname = fn.attr if isinstance(fn, ast.Attribute) else None
                idxs = _CALL_SINKS.get(cname or "", ()) or _METHOD_SINKS.get(mname or "", ())
                for i in idxs:
                    if i < len(node.args):
                        for s in _bare_ui_strings(node.args[i]):
                            hits.append((node.lineno, s))
                for kw in node.keywords:
                    if kw.arg in _KW_SINKS:
                        for s in _bare_ui_strings(kw.value):
                            hits.append((node.lineno, s))
            elif isinstance(node, (ast.Assign, ast.AnnAssign)):
                targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                names = {
                    (
                        t.attr
                        if isinstance(t, ast.Attribute)
                        else t.id
                        if isinstance(t, ast.Name)
                        else ""
                    )
                    for t in targets
                }
                if names & set(_ASSIGN_SINKS) and node.value is not None:
                    for s in _bare_ui_strings(node.value):
                        hits.append((node.lineno, s))
            for lineno, s in hits:
                short = s if len(s) <= 60 else s[:57] + "..."
                problems.append(f"{rel}:{lineno}: unwrapped UI string: {short!r}")
    return problems


# ---------------------------------------------------------------- dynamic gettext calls


def _is_str_literal(node: ast.AST) -> bool:
    return isinstance(node, ast.Constant) and isinstance(node.value, str)


def scan_dynamic_gettext(src: Path = SRC) -> list[str]:
    """Report gettext()/ngettext()/_() calls whose message argument is NOT a string literal.

    Babel can only extract *literal* msgids, so `gettext(some_var)`, `gettext(LOOKUP[k])`, or
    `gettext(f"...")` silently escapes the catalog and falls back to the English source in every
    locale. This failure mode is invisible to the freshness and completeness checks (the strings
    never become msgids at all), which is exactly how the form's type labels and the library's
    kind labels reverted to English. i18n.py's own wrappers legitimately forward a variable to
    the translations object, but do so as attribute calls (`_translations.gettext(msg)`), which
    are not the bare-name calls flagged here."""
    problems: list[str] = []
    anchor = src.parent.parent
    for py in sorted(src.rglob("*.py")):
        tree = ast.parse(py.read_text(encoding="utf-8"), filename=str(py))
        rel = py.relative_to(anchor) if anchor in py.parents else py.name
        for node in ast.walk(tree):
            if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)):
                continue
            fn = node.func.id
            if fn not in ("gettext", "ngettext", "_"):
                continue
            # gettext(msg) / _(msg): arg 0 is the msgid. ngettext(sing, plur, n): args 0 and 1.
            positions = (0, 1) if fn == "ngettext" else (0,)
            if any(i < len(node.args) and not _is_str_literal(node.args[i]) for i in positions):
                problems.append(
                    f"{rel}:{node.lineno}: {fn}() called with a non-literal message — "
                    f"Babel can't extract it, so it is never translated"
                )
    return problems


# --------------------------------------------------------------------------------- main


def main() -> int:
    syntax = check_po_syntax()
    parity = check_placeholder_parity()
    fresh = check_freshness()
    completeness, coverage = check_completeness()
    unwrapped = scan_unwrapped()
    dynamic = scan_dynamic_gettext()

    print("=== i18n coverage ===\n")
    print("0. Catalog syntax (unquoted msgid/msgstr lines):")
    print("   OK — every catalog line is well-quoted" if not syntax else "")
    for p in syntax:
        print(f"   ✗ {p}")
    print("0b. Placeholder parity (msgstr %-placeholders match msgid):")
    print("   OK — every translated string keeps its msgid's placeholders" if not parity else "")
    for p in parity:
        print(f"   ✗ {p}")
    print("1. Extraction freshness (source ↔ skit.pot):")
    print("   OK — .pot matches the wrapped source strings" if not fresh else "")
    for p in fresh:
        print(f"   ✗ {p}")
    print("\n2. Catalog completeness (per shipped locale):")
    for tag, pct in sorted(coverage.items()):
        print(f"   {tag}: {pct:.1f}%")
    for p in completeness:
        print(f"   ✗ {p}")
    print("\n3. Unwrapped UI literals (not in the gettext pipeline):")
    print("   OK — every scanned UI sink routes through gettext" if not unwrapped else "")
    for p in unwrapped:
        print(f"   ✗ {p}")
    print("\n4. Dynamic gettext calls (message not a literal Babel can extract):")
    print(
        "   OK — every gettext()/ngettext() message is an extractable literal"
        if not dynamic
        else ""
    )
    for p in dynamic:
        print(f"   ✗ {p}")

    failed = bool(syntax or parity or fresh or completeness or unwrapped or dynamic)
    print("\n" + ("FAIL — i18n coverage is incomplete" if failed else "PASS — i18n fully covered"))
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
