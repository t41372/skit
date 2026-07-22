"""Static parseArgs reader: turn a literal `util.parseArgs({options:{…}})` into form fields.

The JS/TS analogue of the argparse reader (`langs/python/argspec.py`): a script that parses its own
CLI doesn't get injection — skit reads its `options` declaration statically and renders the same
form, then assembles real flags. Node's built-in `util.parseArgs` is the standard, dependency-free
CLI surface, so it is the one reader here.

Honesty rules (mirrors argspec's A4/C4 stance — never execute the user's script):
- Only a LITERAL, inline `options` object is trusted. Each option's `type`/`default` must be a
  literal — or, for `default`, an identifier naming a top-level literal const (resolved through
  the analyzer's own constant harvest) — or the field degrades to a free-text field that is
  omitted when left empty (the script's own default then applies).
- A surface that can't be modeled at all — `options` is an identifier reference, or a spread
  (`...common`) merges in options from elsewhere — degrades the WHOLE spec: the form keeps only the
  passthrough-args escape field, and the UI says so instead of pretending.
- A computed key (`[flag]: …`) skips just that one field (its name is dynamic — unnameable).

Headless; parses via tree-sitter (the analyzer's grammar handle), no query strings.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from tree_sitter import Parser

from ...params import ParamDecl, is_secret_name
from ..python.argspec import ArgSpec
from .analyzer import (
    _const_candidates,
    _literal_value,
    _mutated_names,
    _string_value,
    _text,
    _walk,
    language_for,
)

if TYPE_CHECKING:
    from tree_sitter import Node

# `default: DEFAULT_HOST` resolving through the module's top-level literal consts — the
# same sound extension the Python reader makes, under the same two rules
# (argspec._constant_env carries the full rationale):
#
# - **Declared exactly once, file-wide.** `_const_candidates` sees only top-level
#   declarations, so a `const HOST` local to the function that calls parseArgs is
#   invisible there and would silently resolve to the OUTER literal. Counting every
#   declaration of the name at any depth (plus function parameters) and requiring
#   exactly one makes the harvested top-level literal provably the one in scope.
#   Reassignment (`_mutated_names`) and reassignable `let`/`var` bindings (demoted)
#   stay excluded on top of that.
# - **Never a secret.** A hardcoded `const API_KEY = "sk-live-…"` must not escape the
#   script's own text through a resolved field default (C3).
ConstEnv = dict[str, str | int | float | bool]


def _declared_names(root: Node) -> dict[str, int]:  # noqa: PLR0915 — one branch per JS/TS binding shape
    """How many times each name is DECLARED anywhere in the file: the `name` identifier
    of every `variable_declarator` at any depth, plus every name a function parameter
    BINDS."""
    counts: dict[str, int] = {}

    def _bump(node: Node | None) -> None:
        if node is not None and node.type in (
            "identifier",
            "shorthand_property_identifier_pattern",
        ):
            text = _text(node)
            counts[text] = counts.get(text, 0) + 1

    def _bump_bound(node: Node) -> None:
        """Only the names a parameter BINDS — a plain identifier, the targets of a
        destructuring pattern, or the binding side of a default. `function f(a = HOST)`
        binds `a` and merely READS HOST; counting that read would make the HOST constant
        look declared twice and needlessly refuse to fold it.

        The default sits in a different place per grammar, so both spellings are named
        here rather than left to the generic walk: JS puts it in an `assignment_pattern`
        (binding on the left), while TS hangs `pattern`, `type` and `value` off one
        `required_parameter`/`optional_parameter` — so `function f(a: string = HOST)`
        would otherwise reach HOST through the sibling `value`. Taking only `pattern`
        there also means a type annotation is never walked into (types are a separate
        namespace that binds no value name), so no rule is needed for them.

        A destructuring key is a property name, not a binding (`{x: {y}}` binds y), and
        the grammar gives it its own node type — so the generic walk, which bumps
        `identifier` alone, already leaves it out."""
        kind = node.type
        if kind in ("identifier", "shorthand_property_identifier_pattern"):
            _bump(node)
            return
        if kind in ("required_parameter", "optional_parameter"):
            pattern = node.child_by_field_name("pattern")
            if pattern is not None:  # pragma: no branch — a TS parameter always has a pattern
                _bump_bound(pattern)
            return
        if kind == "assignment_pattern":
            left = node.child_by_field_name("left")
            if left is not None:  # pragma: no branch — an assignment_pattern always has a left
                _bump_bound(left)
            return
        if kind == "pair_pattern":
            value = node.child_by_field_name("value")
            if value is not None:  # pragma: no branch — pair_pattern always has a value
                _bump_bound(value)
            return
        for child in node.named_children:
            _bump_bound(child)

    def _bump_imports(node: Node) -> None:
        """Count only the value names introduced by one import statement."""
        clause = next((c for c in node.named_children if c.type == "import_clause"), None)
        if clause is None:
            return  # side-effect-only import
        for child in clause.named_children:
            if child.type == "identifier":
                _bump(child)  # default import
            elif child.type == "namespace_import":
                identifiers = [c for c in child.named_children if c.type == "identifier"]
                if identifiers:  # pragma: no branch — namespace_import always binds an identifier
                    _bump(identifiers[-1])
            elif child.type == "named_imports":
                for spec in child.named_children:
                    # Every named child of named_imports is an import_specifier; braces
                    # and commas are anonymous tree-sitter children.
                    bound = spec.child_by_field_name("alias") or spec.child_by_field_name("name")
                    _bump(bound)
            else:  # pragma: no cover — import_clause has only the three shapes above
                continue

    for node in _walk(root):
        if node.type == "variable_declarator":
            name = node.child_by_field_name("name")
            if name is not None:  # pragma: no branch — variable_declarator always has a name
                _bump_bound(name)
        elif node.type == "formal_parameters":
            # A parameter is an identifier itself (JS) or wraps one in a
            # required/optional_parameter pattern (TS) — the walk reaches both.
            for sub in node.named_children:
                _bump_bound(sub)
        elif node.type in (
            "function_declaration",
            "function_expression",
            "generator_function_declaration",
            "generator_function",
            "class_declaration",
            "class",
        ):
            _bump(node.child_by_field_name("name"))
        elif node.type == "catch_clause":
            parameter = node.child_by_field_name("parameter")
            if parameter is not None:
                _bump_bound(parameter)
        elif node.type == "import_statement":
            _bump_imports(node)
    return counts


def _constant_env(root: Node) -> ConstEnv:
    mutated = _mutated_names(root)
    declared = _declared_names(root)
    return {
        c.name: c.default
        for c in _const_candidates(root)
        if not c.demoted
        and c.default is not None
        and not c.secret
        and c.name not in mutated
        and declared.get(c.name) == 1
    }


def read_cli(text: str, *, lang: str = "js") -> ArgSpec | None:  # noqa: PLR0911  # pragma: no mutate — default-lang mutants equivalent: lang only feeds language_for(), which maps js/JS/XXjsXX all to _JS  # fmt: skip
    """Read the script's `util.parseArgs` surface. None when there's nothing parseArgs-shaped (so
    callers fall back to the other form sources); an ArgSpec with ok=False when the surface exists
    but can't be modeled (whole-spec degrade). One early return per unreadable/degrade path."""
    root = Parser(language_for(lang)).parse(text.encode("utf-8")).root_node  # pragma: no mutate — "utf-8"/"UTF-8" name the same codec (case-insensitive)  # fmt: skip
    if root.has_error:
        return None
    call = _find_parseargs(root)
    if call is None:
        return None
    config = _first_argument(call)
    if config is None or config.type != "object":
        return None  # parseArgs() with no config object at all — no readable surface
    options = _pair_value(config, "options")
    if options is None:
        return None  # no `options` key — nothing to model here
    if options.type != "object":
        # `options` is an identifier reference (`parseArgs({ options: opts })`): the real option
        # set lives elsewhere and can't be read statically — degrade the whole spec, honestly.
        return ArgSpec(ok=False, reason="dynamic")
    if any(child.type == "spread_element" for child in options.named_children):
        # A spread (`{ ...common, name: {…} }`) merges options we can't see — whole-spec degrade.
        return ArgSpec(ok=False, reason="dynamic")
    env = _constant_env(root)
    fields: list[ParamDecl] = []
    for pair in options.named_children:
        if pair.type != "pair":
            continue
        field = _read_option(pair, env)
        if field is not None:
            fields.append(field)
    return ArgSpec(fields=fields)


# ---------------------------------------------------------------- locating the call


def _find_parseargs(root: Node) -> Node | None:
    """The first `parseArgs(...)` / `*.parseArgs(...)` call in source order, or None. A member call
    (`util.parseArgs`) matches on the trailing property name, so `node:util`'s import alias — however
    the module was destructured or namespaced — doesn't matter."""
    for node in _walk(root):
        if node.type != "call_expression":
            continue
        fn = node.child_by_field_name("function")
        if fn is None:  # pragma: no cover — a call_expression always has a function
            continue  # pragma: no mutate — unreachable: fn is never None for a call_expression (see no-cover above), so continue/break can't differ
        if fn.type == "identifier" and _text(fn) == "parseArgs":
            return node
        if fn.type == "member_expression":
            prop = fn.child_by_field_name("property")
            if prop is not None and _text(prop) == "parseArgs":
                return node
    return None


def _first_argument(call: Node) -> Node | None:
    """The first positional argument node of a call (skipping the parens), or None."""
    arguments = call.child_by_field_name("arguments")
    if arguments is None:  # pragma: no cover — a call_expression always has an arguments node
        return None
    for child in arguments.named_children:
        return child
    return None


def _pair_value(obj: Node, key: str) -> Node | None:
    """The value node of the object's `key: value` pair (property_identifier or string key), or None
    when the object has no such key."""
    for child in obj.named_children:
        if child.type != "pair":
            continue
        key_node = child.child_by_field_name("key")
        if key_node is not None and _property_name(key_node) == key:
            return child.child_by_field_name("value")
    return None


def _property_name(key: Node) -> str:
    """A pair key's name: a bare `property_identifier` or a quoted `string` key. Anything else
    (a computed `[k]` key) yields "" — the caller treats that as no usable name."""
    if key.type == "property_identifier":
        return _text(key)
    if key.type == "string":
        return _string_value(key)
    return ""


# ---------------------------------------------------------------- reading one option


def _read_option(pair: Node, env: ConstEnv) -> ParamDecl | None:
    """One `name: { type, short, default, multiple }` option → a flag-delivery ParamDecl, or None
    for a computed (dynamic) key that can't name a field."""
    key = pair.child_by_field_name("key")
    if key is None or key.type == "computed_property_name":  # pragma: no mutate — surviving mutants equivalent: key is never None for a pair in an error-free tree, and a computed key yields _property_name()=="" so `if not name` below skips it anyway (the killable != / is-not-None mutations of this line stay covered by test_js_analyzer's computed-key + member-inline tests)  # fmt: skip
        return None  # `[flag]: …` — a dynamic key, unnameable; skip just this field
    name = _property_name(key)
    if not name:
        return None  # an empty-string key (`"": {…}`) can't name a field — skip it
    # binding "none" / delivery "flag" are the ParamDecl defaults; passing them explicitly would
    # only add equivalent "drop the kwarg" mutants (removed kwarg == default). Omit them — the
    # values are pinned by test_read_option_defaults_binding_none_delivery_flag.
    field = ParamDecl(
        name=name,
        flag=f"--{name}",
        secret=is_secret_name(name),
    )
    spec = pair.child_by_field_name("value")
    if spec is None or spec.type != "object":
        field.degraded = True  # `name: someVar` — the option spec isn't inline; free-text fallback
        return field
    _apply_option_spec(field, spec, env)
    return field


def _apply_option_spec(field: ParamDecl, spec: Node, env: ConstEnv) -> None:
    """Fill type/default/multiple from an inline option-spec object. `short` is display-only (skit
    always assembles the long `--name` flag), so it is read and ignored. Type is applied before
    default so an explicit `default` always wins over a boolean's implicit `false`."""
    props: dict[str, Node] = {}
    for pair in spec.named_children:
        if pair.type != "pair":
            continue
        key = pair.child_by_field_name("key")
        value = pair.child_by_field_name("value")
        if key is None or value is None or key.type == "computed_property_name":  # pragma: no mutate — surviving mutants equivalent: key/value are never None for a pair in an error-free tree, and a computed key yields _property_name()=="" so `if name` below drops it anyway (the killable != / is-not-None mutations of this line stay covered by test_js_analyzer's spec tests)  # fmt: skip
            continue
        name = _property_name(key)
        if name:
            props[name] = value
    if "type" in props:
        _apply_type(field, props["type"])
    if "default" in props:
        _apply_default(field, props["default"], env)
    if "multiple" in props and props["multiple"].type == "true":
        field.multiple = True
        # parseArgs collects one value per occurrence: the flag must be repeated
        # (`--tag a --tag b`); a bare second value is an unexpected-positional error.
        field.repeat = True


def _apply_type(field: ParamDecl, value: Node) -> None:
    """Apply a literal `type: "string" | "boolean"`. A boolean becomes a store_true checkbox; a
    string a text field; anything else (a non-literal or unknown type) degrades the field."""
    if value.type == "string":
        text = _string_value(value)
        if text == "boolean":
            field.type = "bool"
            field.action = "store_true"
            field.default = False
            return
        if text == "string":
            field.type = "str"
            return
    field.degraded = True


def _apply_default(field: ParamDecl, value: Node, env: ConstEnv) -> None:
    """Apply a literal `default:` — or an identifier that names a top-level literal const
    (resolved through _constant_env, exactly as if the literal were inline); anything else
    degrades the field (shown, but omitted when left untouched so the script's own default
    applies)."""
    literal = _literal_value(value)
    if literal is not None:
        _type, default = literal
        field.default = default
        return
    if value.type == "identifier" and _text(value) in env:
        field.default = env[_text(value)]
        return
    field.degraded = True
