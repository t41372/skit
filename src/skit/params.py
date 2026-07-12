"""ParamDecl: the one universal parameter model (docs/design/multilang.md, D3).

Every parameter skit knows about — an injected Python constant, a shell env-default, a
declared flag on an exe, a command-template placeholder — is one ParamDecl. Two
orthogonal axes carry the semantics:

- **binding** — how the parameter anchors in source, which decides how drift
  reconciliation matches it: ``const`` (a named literal assignment), ``input`` (an
  interactive prompt call site, keyed by order + prompt text), ``envdefault`` (an
  ``${NAME:-default}``-style expansion, keyed by variable name), or ``none`` (no source
  anchor at all — a hand-declared parameter, or one reflected from the script's own
  CLI parser).
- **delivery** — how the value reaches the program at run time: ``inject`` (rewrite a
  temporary copy / intercept the prompt), ``env`` (set an environment variable on the
  child process), ``flag`` (assemble real argv; ``flag == ""`` means positional), or
  ``placeholder`` (fill a command template).

The two are not independent — a source-anchored binding implies its delivery — and
``validate_invariants`` states the rule. Two serialized homes exist, chosen solely by
the kind's capability (never merged, so there is no precedence to get wrong):

- in-file ``[tool.skit]`` blocks (kinds with ``params_io``): ``to_block_dict`` /
  ``from_block_dict``. The block shape is FROZEN — it is what every existing user file
  already carries (the ``kind`` key with ``const``/``input`` values) — and shared
  verbatim by every ``#``-comment language.
- ``meta.toml [[parameters]]`` (exe / command / anything without a text body):
  ``to_meta_dict`` / ``from_meta_dict``, the full model.

Headless, stdlib-only.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Literal

if TYPE_CHECKING:
    from .langs.python.analyzer import Candidate

Binding = Literal["const", "input", "envdefault", "none"]
Delivery = Literal["inject", "env", "flag", "placeholder"]
ParamType = Literal["str", "int", "float", "bool", "choice"]

# Secret pre-check heuristic (matched against the upper-cased name / prompt). Universal:
# python candidates, shell variables, command placeholders, and declared params all run
# their names through the same rule, so "what counts as secret-looking" can never fork.
_SECRET_HINTS = ("KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD")


def is_secret_name(text: str) -> bool:
    up = text.upper()
    return any(h in up for h in _SECRET_HINTS)


_BINDINGS: tuple[Binding, ...] = ("const", "input", "envdefault", "none")
_DELIVERIES: tuple[Delivery, ...] = ("inject", "env", "flag", "placeholder")
_TYPES: tuple[ParamType, ...] = ("str", "int", "float", "bool", "choice")

# The delivery each source-anchored binding implies; "none" is the free axis.
_BINDING_DELIVERY: dict[str, Delivery] = {
    "const": "inject",
    "input": "inject",
    "envdefault": "env",
}


@dataclass
class ParamDecl:
    """One parameter. Field-aligned with analyzer.Candidate (inter-convertible)."""

    name: str
    binding: Binding = "none"
    delivery: Delivery = "flag"
    type: ParamType = "str"
    default: str | int | float | bool | None = None
    required: bool = False
    multiple: bool = False  # flag delivery: shlex-split + glob-expand each piece
    choices: tuple[str, ...] = ()
    prompt: str = ""  # form label; for input bindings, the literal call prompt
    help: str = ""  # field help text (shown under the form field)
    secret: bool = False  # C3: the value never lands in a state file
    env_source: str = ""  # secret VALUE read from this env var (name only, never a value)
    flag: str = ""  # delivery=flag: "--output"; "" = positional
    action: str = ""  # bool flags: "store_true" | "store_false"
    order: int = -1  # binding=input: the call-order key (B1)
    env_target: str = ""  # delivery=env: variable to SET; "" = the param's own name
    degraded: bool = False  # static read couldn't fully model it; omit when left empty

    @property
    def env_var(self) -> str:
        """The environment variable an env-delivered value sets."""
        return self.env_target or self.name

    # ---------------------------------------------------------------- from a source candidate

    @classmethod
    def from_candidate(cls, c: Candidate) -> ParamDecl:
        """Build a decl from an analyzer Candidate — the two are field-aligned by design
        (A2), so the CLI, TUI add panel, TUI settings, and reconcile can't drift on which
        fields carry over. The one place this conversion lives. binding/type come off a
        Candidate typed ``str``; both are coerced through the closed literal sets (a no-op
        for real analyzer output, which only ever emits const/input and INJECTABLE_TYPES),
        and delivery is derived from the binding exactly like ``from_block_dict``."""
        binding = _coerce_literal(c.binding, _BINDINGS, "none")
        return cls(
            name=c.name,
            binding=binding,
            delivery=_BINDING_DELIVERY.get(binding, "flag"),
            type=_coerce_literal(c.type, _TYPES, "str"),
            default=c.default,
            prompt=c.prompt,
            order=c.order,
            secret=c.secret,
        )

    # ---------------------------------------------------------------- block (in-file)

    def to_block_dict(self) -> dict[str, str | int | float | bool]:
        """The FROZEN ``[tool.skit]`` table shape (key ``kind``, values const/input —
        exactly what existing user files carry; changing a key here orphans them)."""
        d: dict[str, str | int | float | bool] = {
            "name": self.name,
            "kind": self.binding,
            "type": self.type,
        }
        if self.default is not None:
            d["default"] = self.default
        if self.prompt:
            d["prompt"] = self.prompt
        if self.order >= 0:
            d["order"] = self.order
        if self.secret:
            d["secret"] = True
        if self.env_source:
            d["env_source"] = self.env_source
        return d

    @classmethod
    def from_block_dict(cls, d: dict[str, Any]) -> ParamDecl:
        """Total: a hand-edited block can hold any TOML scalar anywhere; degrade to
        defaults rather than raising out of every caller (TUI load, params/run/edit)."""
        try:
            order = int(d.get("order", -1))
        except (TypeError, ValueError):
            order = -1
        binding = _coerce_literal(str(d.get("kind", "const")), _BINDINGS, "const")
        return cls(
            name=str(d.get("name", "")),
            binding=binding,
            delivery=_BINDING_DELIVERY.get(binding, "flag"),
            type=_coerce_literal(str(d.get("type", "str")), _TYPES, "str"),
            default=_scalar_or_none(d.get("default")),
            prompt=str(d.get("prompt", "")),
            order=order,
            secret=bool(d.get("secret", False)),
            env_source=str(d.get("env_source", "")),
        )

    # ---------------------------------------------------------------- meta [[parameters]]

    def to_meta_dict(self) -> dict[str, Any]:
        """The meta.toml ``[[parameters]]`` row: the full model, empty/default values
        omitted (additive-only forward contract; old skit ignores unknown keys)."""
        d: dict[str, Any] = {"name": self.name, "delivery": self.delivery, "type": self.type}
        if self.binding != "none":
            d["binding"] = self.binding
        if self.default is not None:
            d["default"] = self.default
        if self.choices:
            d["choices"] = list(self.choices)
        if self.order >= 0:
            d["order"] = self.order
        # Truthiness-gated tail: every remaining optional field is falsy at its default,
        # so "set it iff truthy" is exactly "omit defaults".
        tail: tuple[tuple[str, str | bool], ...] = (
            ("required", self.required),
            ("multiple", self.multiple),
            ("prompt", self.prompt),
            ("help", self.help),
            ("secret", self.secret),
            ("env_source", self.env_source),
            ("flag", self.flag),
            ("action", self.action),
            ("env_target", self.env_target),
        )
        d.update({key: value for key, value in tail if value})
        return d

    @classmethod
    def from_meta_dict(cls, d: dict[str, Any]) -> ParamDecl:
        """Total, like from_block_dict: meta.toml is user-editable TOML."""
        try:
            order = int(d.get("order", -1))
        except (TypeError, ValueError):
            order = -1
        raw_choices = d.get("choices")
        choices = tuple(str(c) for c in raw_choices) if isinstance(raw_choices, list) else ()
        return cls(
            name=str(d.get("name", "")),
            binding=_coerce_literal(str(d.get("binding", "none")), _BINDINGS, "none"),
            delivery=_coerce_literal(str(d.get("delivery", "flag")), _DELIVERIES, "flag"),
            type=_coerce_literal(str(d.get("type", "str")), _TYPES, "str"),
            default=_scalar_or_none(d.get("default")),
            required=bool(d.get("required", False)),
            multiple=bool(d.get("multiple", False)),
            choices=choices,
            prompt=str(d.get("prompt", "")),
            help=str(d.get("help", "")),
            secret=bool(d.get("secret", False)),
            env_source=str(d.get("env_source", "")),
            flag=str(d.get("flag", "")),
            action=str(d.get("action", "")),
            order=order,
            env_target=str(d.get("env_target", "")),
        )


def synthesized_placeholder(name: str) -> ParamDecl:
    """The default schema of an undeclared command-template placeholder — exactly the
    historical form behavior: required (an empty placeholder silently assembles a broken
    command, which the non-interactive contract forbids), free-text, secret by the name
    heuristic (C3 applies to every source)."""
    return ParamDecl(
        name=name,
        binding="none",
        delivery="placeholder",
        required=True,
        secret=is_secret_name(name),
    )


def declared_from_meta(parameters: list[dict[str, Any]] | None) -> list[ParamDecl]:
    """The declared rows of a meta [[parameters]] array, nameless rows dropped (a
    hand-edited row without a name can't key a form field, a value, or an edit op)."""
    return [d for row in parameters or [] if (d := ParamDecl.from_meta_dict(row)).name]


def declared_for_template(
    parameters: list[dict[str, Any]] | None, placeholders: list[str]
) -> list[ParamDecl]:
    """The form decls for a command template: the template's placeholder list IS the
    field list (in template order — the template is the source of truth for WHICH
    parameters exist), and a declared row supplies a placeholder's schema when present
    (type/default/optional/secret override — the fix for the auto-secret-no-override
    defect). Declared env-delivery params ride along after the placeholders (an env
    variable is a legitimate second channel into a shell template's child process);
    any other declared delivery is ignored here — argv is not a template's interface
    (takes_argv=False), so a flag row can only be a hand-edit mistake, and dropping it
    from the form beats assembling arguments the template never reads."""
    declared = {d.name: d for d in declared_from_meta(parameters)}
    out: list[ParamDecl] = []
    for name in placeholders:
        decl = declared.get(name)
        if decl is not None and decl.delivery == "placeholder":
            out.append(decl)
        else:
            out.append(synthesized_placeholder(name))
    out.extend(
        d for d in declared.values() if d.delivery == "env" and d.name not in set(placeholders)
    )
    return out


def validate_invariants(decl: ParamDecl) -> str | None:
    """The binding→delivery rule, as a symbolic reason id (None = consistent). The UI
    owns the human wording. Deliberately a check, not a constructor guard: hand-edited
    TOML must degrade at the boundary (from_*_dict is total), and callers that MUTATE
    a decl (edit ops) re-check before persisting."""
    implied = _BINDING_DELIVERY.get(decl.binding)
    if implied is not None and decl.delivery != implied:
        return "binding-delivery-mismatch"
    if decl.type == "choice" and not decl.choices:
        return "choice-without-choices"
    return None


def normalize(decl: ParamDecl) -> ParamDecl:
    """Repair what validate_invariants flags where a safe repair exists: a
    source-anchored binding always wins over a hand-edited delivery."""
    implied = _BINDING_DELIVERY.get(decl.binding)
    if implied is not None and decl.delivery != implied:
        decl = field_replace(decl, delivery=implied)
    return decl


def field_replace(decl: ParamDecl, **changes: Any) -> ParamDecl:
    """dataclasses.replace, re-exported so edit-op callers don't import dataclasses."""
    import dataclasses

    return dataclasses.replace(decl, **changes)


def _coerce_literal[T: str](value: str, allowed: tuple[T, ...], fallback: T) -> T:
    for a in allowed:
        if value == a:
            return a
    return fallback


def _scalar_or_none(value: Any) -> str | int | float | bool | None:
    """Only the injectable scalar domain survives; anything else (a TOML table, an
    array) degrades to None — never crashes a reader."""
    if isinstance(value, (str, int, float, bool)):
        return value
    return None
