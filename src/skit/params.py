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
from typing import Any, Literal

Binding = Literal["const", "input", "envdefault", "none"]
Delivery = Literal["inject", "env", "flag", "placeholder"]
ParamType = Literal["str", "int", "float", "bool", "choice"]

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
