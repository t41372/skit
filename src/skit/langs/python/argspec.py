"""Static argparse reader: turn literal add_argument(...) calls into form fields.

The unified-form model's third source (UX spec: "any script becomes a pokeable form"):
scripts that parse their own CLI don't get injection — skit reads their argument
declarations statically and renders the same form, then assembles real flags.

Honesty rules (mirrors the analyzer's A4/C4 stance — never execute the user's script):
- Only LITERAL arguments to add_argument are trusted. A field whose type/choices/default
  can't be read statically degrades to a free-text field that is omitted when left empty
  (the script's own default then applies).
- A parser that can't be modeled at all — add_subparsers, add_argument inside a loop —
  degrades the whole spec: the form keeps only the passthrough-args escape field, and the
  UI says so instead of pretending.

Headless, stdlib-only.
"""

from __future__ import annotations

import ast
from dataclasses import dataclass, field

from .analyzer import _is_secret_name, _literal_value

# Actions that add no form field at all (argparse handles them internally).
_NON_FIELD_ACTIONS = ("help", "version")
# Actions we model as a checkbox.
_BOOL_ACTIONS = ("store_true", "store_false")


@dataclass
class ArgField:
    """One add_argument call, as a form field."""

    dest: str  # display/name key (flag name without dashes, or the positional name)
    flag: str = ""  # "--output" (longest declared flag); "" for a positional
    required: bool = False
    kind: str = "str"  # "str" | "int" | "float" | "bool" | "choice"
    choices: list[str] = field(default_factory=list)
    default: str | int | float | bool | None = None
    help: str = ""
    multiple: bool = False  # nargs "+" / "*"
    degraded: bool = False  # free-text fallback; omit from the command when left empty
    secret: bool = False
    action: str = ""  # "store_true" / "store_false" when kind == "bool"
    order: int = 0


@dataclass
class ArgSpec:
    fields: list[ArgField] = field(default_factory=list)
    ok: bool = True  # False -> whole-parser degradation (passthrough escape only)
    reason: str = ""  # symbolic: "subparsers" | "dynamic" (UI owns the wording)


def read_cli(text: str) -> ArgSpec | None:
    """The unified entry point: the first CLI surface that reads statically wins.
    argparse first (the overwhelmingly common case in AI-written scripts), then click,
    then typer. None when the script has no readable CLI surface at all."""
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return None
    for reader in (read_argparse, _read_click, _read_typer):
        spec = reader(text) if reader is read_argparse else reader(tree)
        if spec is not None:
            return spec
    return None


def read_argparse(text: str) -> ArgSpec | None:
    """Read the script's argparse surface. None when there's nothing argparse-shaped
    (no add_argument calls at all) — callers then fall back to the other form sources."""
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return None
    calls = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.Call)
        and isinstance(node.func, ast.Attribute)
        and node.func.attr == "add_argument"
    ]
    if not calls:
        return None
    if any(
        isinstance(node, ast.Call)
        and isinstance(node.func, ast.Attribute)
        and node.func.attr == "add_subparsers"
        for node in ast.walk(tree)
    ):
        return ArgSpec(ok=False, reason="subparsers")
    if _any_call_inside_loop(tree):
        return ArgSpec(ok=False, reason="dynamic")
    fields: list[ArgField] = []
    for i, call in enumerate(sorted(calls, key=lambda c: (c.lineno, c.col_offset))):
        f = _read_call(call, order=i)
        if f is not None:
            fields.append(f)
    return ArgSpec(fields=fields)


def _any_call_inside_loop(tree: ast.Module) -> bool:
    """add_argument under a for/while: the argument list is data-driven; we can't model it."""
    for node in ast.walk(tree):
        if isinstance(node, (ast.For, ast.While)):
            for sub in ast.walk(node):
                if (
                    isinstance(sub, ast.Call)
                    and isinstance(sub.func, ast.Attribute)
                    and sub.func.attr == "add_argument"
                ):
                    return True
    return False


def _read_call(call: ast.Call, order: int) -> ArgField | None:
    """One add_argument call -> ArgField, or None for non-field actions (--help/--version)."""
    names = [a.value for a in call.args if isinstance(a, ast.Constant) and isinstance(a.value, str)]
    if not names or len(names) != len(call.args):
        # A non-literal name (or none at all): we can't even label the field — skip it and
        # let the passthrough escape field carry it.
        return None
    kwargs = {kw.arg: kw.value for kw in call.keywords if kw.arg}
    action = _literal_str(kwargs.get("action"))
    if action in _NON_FIELD_ACTIONS:
        return None
    positional = not names[0].startswith("-")
    long_flags = [n for n in names if n.startswith("--")]
    flag = long_flags[0] if long_flags else ("" if positional else names[0])
    dest = _literal_str(kwargs.get("dest")) or (
        names[0] if positional else flag.lstrip("-").replace("-", "_")
    )
    nargs = _literal_value(kwargs["nargs"])[1] if "nargs" in kwargs else None
    multiple = nargs in ("+", "*")
    if positional:
        required = nargs not in ("*", "?")
    else:
        req_node = kwargs.get("required")
        required = isinstance(req_node, ast.Constant) and req_node.value is True

    f = ArgField(
        dest=dest,
        flag=flag,
        required=required,
        help=_literal_str(kwargs.get("help")),
        multiple=multiple,
        secret=_is_secret_name(dest),
        order=order,
    )
    if action in _BOOL_ACTIONS:
        f.kind = "bool"
        f.action = action
        f.default = action == "store_false"  # store_false means "on unless flagged"
    elif action:
        # append / count / extend / custom Action classes: real but unmodelable — degrade.
        f.degraded = True
    else:
        _apply_value_kwargs(f, kwargs)
    return f


def _apply_value_kwargs(f: ArgField, kwargs: dict[str, ast.expr]) -> None:
    """Fill kind/choices/default from literal kwargs; degrade the field on anything opaque."""
    if "choices" in kwargs:
        choices = _literal_str_list(kwargs["choices"])
        if choices is None:
            f.degraded = True
            return
        f.kind = "choice"
        f.choices = choices
    if "type" in kwargs and not _apply_type(f, kwargs["type"]):
        # A conversion function we can't run (e.g. parse_color): free-text fallback.
        f.degraded = True
        return
    if "default" in kwargs:
        ok, value = _literal_value(kwargs["default"])
        if ok:
            f.default = value
        elif not (isinstance(kwargs["default"], ast.Constant) and kwargs["default"].value is None):
            # A computed default (tuple, call, attribute): show the field, but leave it
            # empty and omit the flag when untouched so the script's own default applies.
            f.degraded = True


def _apply_type(f: ArgField, node: ast.expr) -> bool:
    """Apply a literal type= kwarg. True when the type is form-representable."""
    if isinstance(node, ast.Name) and node.id in ("int", "float", "str"):
        if f.kind != "choice":  # choices win: the selector already constrains input
            f.kind = node.id
        return True
    # Path renders as text; anything else is an arbitrary callable we won't execute.
    return isinstance(node, ast.Name) and node.id == "Path"


# --------------------------------------------------------------------------
# click
# --------------------------------------------------------------------------


def _imports_module(tree: ast.Module, root: str) -> bool:
    """Whether the script imports `root` in any form (import x / import x.y / from x[.y]
    import z). The root comparison is dot-split so `import click.testing` counts."""
    for n in ast.walk(tree):
        if isinstance(n, ast.Import) and any(a.name.split(".")[0] == root for a in n.names):
            return True
        if (
            isinstance(n, ast.ImportFrom)
            and n.module is not None
            and n.module.split(".")[0] == root
        ):
            return True
    return False


def _decorator_call(node: ast.expr) -> ast.Call | None:
    return node if isinstance(node, ast.Call) else None


def _decorator_name(node: ast.expr) -> str:
    """The trailing attribute/name of a decorator callable: click.option -> "option"."""
    target = node.func if isinstance(node, ast.Call) else node
    if isinstance(target, ast.Attribute):
        return target.attr
    if isinstance(target, ast.Name):
        return target.id
    return ""


def _read_click(tree: ast.Module) -> ArgSpec | None:
    """Static click reading: @click.option/@click.argument stacks on a command function.

    click applies decorators bottom-up, so the BOTTOM decorator declares the first
    parameter — fields are collected in reversed decorator order to match runtime.
    A @click.group (or several commands) is subcommand territory: whole-spec degrade."""
    if not _imports_module(tree, "click"):
        return None  # a bare @app.command() without click is somebody else's (typer's) surface
    decorated = [
        (node, [_decorator_name(d) for d in node.decorator_list])
        for node in ast.walk(tree)
        if isinstance(node, ast.FunctionDef)
    ]
    has_group = any("group" in names for _fn, names in decorated)
    commands = [fn for fn, names in decorated if "command" in names]
    if not commands and not has_group:
        return None
    if has_group or len(commands) > 1:
        return ArgSpec(ok=False, reason="subparsers")
    fields: list[ArgField] = []
    order = 0
    for deco in reversed(commands[0].decorator_list):
        call = _decorator_call(deco)
        if call is None:
            continue
        kind = _decorator_name(deco)
        if kind not in ("option", "argument"):
            continue
        f = _read_click_param(call, positional=(kind == "argument"), order=order)
        if f is not None:
            fields.append(f)
            order += 1
    return ArgSpec(fields=fields)


def _is_true_kwarg(node: ast.expr | None) -> bool:
    return isinstance(node, ast.Constant) and node.value is True


def _read_click_param(call: ast.Call, positional: bool, order: int) -> ArgField | None:
    names = [a.value for a in call.args if isinstance(a, ast.Constant) and isinstance(a.value, str)]
    if not names or len(names) != len(call.args):
        return None
    kwargs = {kw.arg: kw.value for kw in call.keywords if kw.arg}
    long_flags = [n for n in names if n.startswith("--")]
    flag = "" if positional else (long_flags[0] if long_flags else names[0])
    dest = names[0] if positional else (flag.lstrip("-").replace("-", "_"))
    nargs = _literal_value(kwargs["nargs"])[1] if "nargs" in kwargs else None
    f = ArgField(
        dest=dest,
        flag=flag,
        # click arguments are required by default; nargs=-1 (variadic) is not.
        required=(positional and nargs != -1) or _is_true_kwarg(kwargs.get("required")),
        help=_literal_str(kwargs.get("help")),
        multiple=nargs == -1 or _is_true_kwarg(kwargs.get("multiple")),
        secret=_is_secret_name(dest),
        order=order,
    )
    is_flag = kwargs.get("is_flag")
    if isinstance(is_flag, ast.Constant) and is_flag.value is True:
        if _is_true_kwarg(kwargs.get("default")):
            # An is_flag that DEFAULTS to on needs the --flag/--no-flag pairing we
            # can't assemble faithfully — degrade honestly (typer's rule, mirrored).
            f.degraded = True
            return f
        f.kind = "bool"
        f.action = "store_true"
        f.default = False
        return f
    type_node = kwargs.get("type")
    if type_node is not None and not _apply_click_type(f, type_node):
        f.degraded = True
        return f
    if "default" in kwargs:
        ok, value = _literal_value(kwargs["default"])
        if ok:
            f.default = value
        elif not (isinstance(kwargs["default"], ast.Constant) and kwargs["default"].value is None):
            f.degraded = True
    if "count" in kwargs:
        f.degraded = True
    return f


def _apply_click_type(f: ArgField, node: ast.expr) -> bool:
    """click type=: bare int/float/str, click.INT/FLOAT/STRING, click.Choice([...])."""
    if isinstance(node, ast.Name) and node.id in ("int", "float", "str"):
        f.kind = node.id
        return True
    if isinstance(node, ast.Attribute) and node.attr in ("INT", "FLOAT", "STRING"):
        f.kind = {"INT": "int", "FLOAT": "float", "STRING": "str"}[node.attr]
        return True
    if isinstance(node, ast.Call) and _decorator_name(node) == "Choice" and node.args:
        choices = _literal_str_list(node.args[0])
        if choices is None:
            return False
        f.kind = "choice"
        f.choices = choices
        return True
    return False


# --------------------------------------------------------------------------
# typer
# --------------------------------------------------------------------------

_ANNOTATION_KINDS = {"int": "int", "float": "float", "str": "str", "bool": "bool", "Path": "str"}


def _read_typer(tree: ast.Module) -> ArgSpec | None:
    """Static typer reading: the command function's signature IS the CLI surface.

    Finds @<app>.command() functions or the function handed to typer.run(); more than
    one command means subcommands (whole-spec degrade). Parameters map by annotation
    (int/float/str/bool/Path) with typer.Option/typer.Argument defaults; both the legacy
    `x: int = typer.Option(...)` form and the modern `x: Annotated[int, typer.Option(...)]`
    form (what AI-written typer scripts overwhelmingly use) are read. A bool that defaults
    to True would need paired --x/--no-x flags we can't assemble faithfully, so it degrades
    instead of guessing."""
    if not _imports_module(tree, "typer"):
        return None
    commands = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.FunctionDef)
        and any(_decorator_name(d) == "command" for d in node.decorator_list)
    ]
    if not commands:
        run_targets = {
            node.args[0].id
            for node in ast.walk(tree)
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr == "run"
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == "typer"
            and node.args
            and isinstance(node.args[0], ast.Name)
        }
        commands = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.FunctionDef) and node.name in run_targets
        ]
    if not commands:
        return None
    if len(commands) > 1:
        return ArgSpec(ok=False, reason="subparsers")
    fn = commands[0]
    args = fn.args.args + fn.args.kwonlyargs
    defaults: list[ast.expr | None] = list(fn.args.defaults) + list(fn.args.kw_defaults)
    # Positional-args defaults align to the TAIL of fn.args.args.
    pad = len(fn.args.args) - len(fn.args.defaults)
    aligned: list[ast.expr | None] = [None] * pad + defaults
    fields: list[ArgField] = []
    # len(aligned) == len(args) by construction: pad + positional defaults covers
    # args.args, and kw_defaults is exactly one entry per kwonly arg (None when absent).
    for i, arg in enumerate(args):
        fields.append(_read_typer_param(arg, aligned[i], order=i))
    return ArgSpec(fields=fields)


def _annotated_parts(annotation: ast.expr | None) -> tuple[ast.expr | None, ast.Call | None]:
    """Unwrap `Annotated[T, typer.Option/Argument(...), ...]` (the modern typer style,
    and what AI-generated typer scripts overwhelmingly use). Returns (T, the typer
    metadata call or None). A non-Annotated annotation passes straight through as
    (annotation, None), so the legacy `x: int = typer.Option(...)` path is untouched."""
    if not (
        isinstance(annotation, ast.Subscript)
        and (
            (isinstance(annotation.value, ast.Name) and annotation.value.id == "Annotated")
            or (
                isinstance(annotation.value, ast.Attribute) and annotation.value.attr == "Annotated"
            )
        )
    ):
        return annotation, None
    elts = annotation.slice.elts if isinstance(annotation.slice, ast.Tuple) else [annotation.slice]
    base = elts[0] if elts else None
    meta = next(
        (
            e
            for e in elts[1:]
            if isinstance(e, ast.Call) and _decorator_name(e) in ("Option", "Argument")
        ),
        None,
    )
    return base, meta


def _read_typer_param(arg: ast.arg, default: ast.expr | None, order: int) -> ArgField:
    name = arg.arg
    annotation, annotated_meta = _annotated_parts(arg.annotation)
    looked_up = _ANNOTATION_KINDS.get(annotation.id) if isinstance(annotation, ast.Name) else None
    f = ArgField(
        dest=name,
        flag=f"--{name.replace('_', '-')}",
        kind=looked_up if looked_up is not None else "str",
        secret=_is_secret_name(name),
        order=order,
        # An annotation we can't model (Enum, Optional[...], List[...]) degrades the
        # field; NO annotation at all is fine — typer treats it as str, and so do we.
        degraded=annotation is not None and looked_up is None,
    )
    if annotated_meta is not None:
        # Annotated style: the Option/Argument metadata lives in the annotation, and the
        # default (if any) is the parameter's own `= value`.
        # pragma: has_positional_default=False vs None is equivalent (both falsy in every
        # branch of _apply_typer_meta); the killable True variant is behaviorally pinned by
        # test_annotated_option_positional_decl_is_a_flag_not_a_default.
        _apply_typer_meta(f, annotated_meta, has_positional_default=False)  # pragma: no mutate
        _apply_typer_signature_default(f, default)
        return _typer_finish_bool(f)
    if default is None:
        # No default: a positional, required argument (typer's own rule).
        f.flag = ""
        f.required = True
        return _typer_finish_bool(f)
    if isinstance(default, ast.Call) and _decorator_name(default) in ("Option", "Argument"):
        # Legacy style: the Option/Argument call IS the parameter default, and its first
        # positional is the value default (ellipsis = required).
        _apply_typer_meta(f, default, has_positional_default=True)
        return _typer_finish_bool(f)
    _apply_typer_signature_default(f, default)
    return _typer_finish_bool(f)


def _apply_typer_meta(f: ArgField, call: ast.Call, *, has_positional_default: bool) -> None:
    """Read flag/help (and, in the legacy style, the value default) from a typer
    Option/Argument call. In the Annotated style the call carries no positional default,
    so every string positional is a flag declaration."""
    kwargs = {kw.arg: kw.value for kw in call.keywords if kw.arg}
    decl_args = call.args[1:] if has_positional_default else call.args
    decls = [a.value for a in decl_args if isinstance(a, ast.Constant) and isinstance(a.value, str)]
    long_flags = [d for d in decls if d.startswith("--")]
    if _decorator_name(call) == "Argument":
        f.flag = ""
    elif long_flags:
        f.flag = long_flags[0]
    f.help = _literal_str(kwargs.get("help"))
    if has_positional_default and call.args:
        first = call.args[0]
        if isinstance(first, ast.Constant) and first.value is ...:
            f.required = True
        else:
            ok, value = _literal_value(first)
            if ok:
                f.default = value
            elif not (isinstance(first, ast.Constant) and first.value is None):
                f.degraded = True


def _apply_typer_signature_default(f: ArgField, default: ast.expr | None) -> None:
    """Apply the parameter's own `= value` default (the Annotated style keeps it there,
    and a bare `x: int = 5` too). None means required."""
    if default is None:
        f.required = True
        return
    ok, value = _literal_value(default)
    if ok:
        f.default = value
    else:
        f.degraded = True


def _typer_finish_bool(f: ArgField) -> ArgField:
    """typer bools become --x/--no-x pairs; only the default-False case assembles as a
    plain store_true flag. Default-True (or required) would need the --no-x spelling —
    degrade rather than emit a flag that means the opposite."""
    if f.kind != "bool":
        return f
    if f.default in (None, False) and not f.required:
        f.action = "store_true"
        f.default = False
        return f
    f.degraded = True
    f.kind = "str"
    return f


def _literal_str(node: ast.expr | None) -> str:
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    return ""


def _literal_str_list(node: ast.expr) -> list[str] | None:
    """A literal list/tuple of scalars, rendered as strings (choices are typed at the form
    edge anyway); None when any element isn't a literal."""
    if not isinstance(node, (ast.List, ast.Tuple)):
        return None
    out: list[str] = []
    for elt in node.elts:
        ok, value = _literal_value(elt)
        if not ok:
            return None
        out.append(str(value))
    return out
