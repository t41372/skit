"""Default that names a top-level literal constant resolves as if the literal were inline.

The sound extension the CLI readers make (argspec._constant_env / cli_reader._constant_env):
a `default=DEFAULT_HOST` naming a module-level literal const resolves through the analyzer's
own constant harvest, exactly as if the literal were written in place. Two rules keep that
inside the honesty bar, and both are pinned here: the name must be bound EXACTLY ONCE
file-wide — anything rebinding it (augmented assignment, a loop, an `if`/`try`/`with` body, a
function-local declaration, a function parameter, JS `let`/`var` or reassignment) degrades the
field honestly rather than resolving a literal the script may already have replaced — and it
must never be secret-looking, so a hardcoded key never escapes the script's own text through a
field default (C3). Calls/attributes/unknown names degrade too. Covered across every reader
that grew the capability: argparse, click, typer (legacy + Annotated + bare signature), and the
JS/TS parseArgs reader.
"""

from __future__ import annotations

from skit.langs.javascript import cli_reader
from skit.langs.python import argspec

# --------------------------------------------------------------------------
# argparse: default=CONST resolving through the top-level constant environment
# --------------------------------------------------------------------------


def test_argparse_string_constant_default_resolves():
    # A bare Name default referring to a top-level literal const resolves as if the string
    # literal were inline — clean field, not a degraded free-text box.
    spec = argspec.read_argparse(
        "import argparse\nHOST = 'example.com'\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--host', default=HOST)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "example.com"
    assert f.degraded is False


def test_argparse_int_and_bool_constant_defaults_resolve():
    # int and bool constant values resolve to their literal value, both clean.
    spec = argspec.read_argparse(
        "import argparse\nPORT = 8080\nDEBUG = True\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--port', type=int, default=PORT)\n"
        "ap.add_argument('--debug', default=DEBUG)\n"
    )
    assert spec is not None
    by = {f.name: f for f in spec.fields}
    assert by["port"].default == 8080
    assert by["port"].degraded is False
    assert by["debug"].default is True
    assert by["debug"].degraded is False


def test_argparse_augmented_assigned_name_does_not_resolve():
    # A name augmented-assigned anywhere is a working variable, not a knowable constant:
    # its value at parse time isn't a single literal, so the field degrades.
    spec = argspec.read_argparse(
        "import argparse\nHOST = 'a'\nHOST += 'b'\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--host', default=HOST)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_argparse_loop_reassigned_name_does_not_resolve():
    # Reassigned inside a loop body -> mutated -> excluded from the constant environment.
    spec = argspec.read_argparse(
        "import argparse\nHOST = 'a'\nfor i in range(3):\n    HOST = str(i)\n"
        "ap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_argparse_unknown_name_default_degrades():
    # A default naming something that isn't a top-level literal const at all: no resolution,
    # so the field degrades (shown, omitted when left empty so the script's own default applies).
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--host', default=MISSING)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_argparse_call_default_still_degrades():
    # Unchanged behavior: a computed default (a call) is never read as a value — it degrades.
    spec = argspec.read_argparse(
        "import argparse\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--host', default=str(f()))\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_argparse_constant_used_twice_resolves_in_both_fields():
    # The rule counts BINDINGS, not USES: one assignment read by two add_argument calls is
    # still provably one literal, so both fields resolve. (Were Load-context names counted
    # too, this constant would look "bound" three times and both fields would degrade —
    # over-tightening the fix into uselessness, since a constant exists to be referenced.)
    spec = argspec.read_argparse(
        "import argparse\nHOST = 'example.com'\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--host', default=HOST)\n"
        "ap.add_argument('--mirror', default=HOST)\n"
    )
    assert spec is not None
    by = {f.name: f for f in spec.fields}
    assert by["host"].default == "example.com"
    assert by["host"].degraded is False
    assert by["mirror"].default == "example.com"
    assert by["mirror"].degraded is False


def test_argparse_conditional_rebinding_does_not_resolve():
    # Rebound in an `if` body: the top-level harvest sees only `HOST = 'localhost'`, but the
    # script's own PROD branch may have replaced it by the time the parser is built.
    # Resolving would make skit pass `--host localhost` on EVERY run and silently defeat
    # that branch — so the name is bound twice module-wide and never resolves.
    spec = argspec.read_argparse(
        "import argparse, os\nHOST = 'localhost'\n"
        "if os.getenv('PROD'):\n    HOST = 'prod.example.com'\n"
        "ap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_argparse_try_except_rebinding_does_not_resolve():
    # The same rule inside a try/except: neither rebinding is a top-level statement, so the
    # harvest can't see them — the module-wide binding count is what catches this.
    spec = argspec.read_argparse(
        "import argparse\nHOST = 'localhost'\n"
        "try:\n    import prodcfg\n    HOST = 'prod.example.com'\n"
        "except ImportError:\n    HOST = 'fallback.example.com'\n"
        "ap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_argparse_with_block_rebinding_does_not_resolve():
    # And inside a `with` body — one more non-top-level statement position.
    spec = argspec.read_argparse(
        "import argparse\nHOST = 'localhost'\n"
        "with open('cfg') as fh:\n    HOST = 'from-config.example.com'\n"
        "ap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_argparse_function_local_assignment_blocks_resolution():
    # A function-local assignment to the same name is a second binding module-wide. skit
    # refuses to reason about which one the parser call sees, so the field degrades.
    spec = argspec.read_argparse(
        "import argparse\nHOST = 'localhost'\n"
        "def setup():\n    HOST = 'inner.example.com'\n    return HOST\n"
        "ap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_argparse_function_parameter_shadow_blocks_resolution():
    # A PARAMETER of the same name binds it too (_bound_names counts ast.arg), so the
    # top-level literal is no longer provably the only binding — degrade, don't guess.
    spec = argspec.read_argparse(
        "import argparse\nHOST = 'localhost'\n"
        "def connect(HOST):\n    return HOST\n"
        "ap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_argparse_secret_constant_never_resolves():
    # C3: a hardcoded API key resolved into a field default would be prefilled into the run
    # form, printed by `show --json` and written into preset TOML on disk — the literal
    # leaving the script's own text for the first time. The field degrades instead, and the
    # secret appears NOWHERE in the resulting declaration.
    spec = argspec.read_argparse(
        "import argparse\nAPI_KEY = 'sk-live-abc123'\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--auth', default=API_KEY)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None
    assert "sk-live-abc123" not in repr(f)


def test_argparse_password_and_token_constants_never_resolve():
    # The same C3 rule for the other secret-looking spellings the harvest flags.
    spec = argspec.read_argparse(
        "import argparse\nPASSWORD = 'hunter2'\nGH_TOKEN = 'ghp_xyz789'\n"
        "ap = argparse.ArgumentParser()\n"
        "ap.add_argument('--auth', default=PASSWORD)\n"
        "ap.add_argument('--creds', default=GH_TOKEN)\n"
    )
    assert spec is not None
    by = {f.name: f for f in spec.fields}
    assert by["auth"].degraded is True
    assert by["auth"].default is None
    assert by["creds"].degraded is True
    assert by["creds"].default is None
    assert "hunter2" not in repr(spec.fields)
    assert "ghp_xyz789" not in repr(spec.fields)


def test_argparse_constant_bound_twice_does_not_resolve():
    # Two top-level assignments to the same name: which one is in force at the
    # add_argument call depends on where that call sits between them, and skit refuses to
    # guess (A4/C4). The field degrades to free-text, so an untouched field is omitted and
    # the script's own default applies — the honest answer, not a coin flip.
    spec = argspec.read_argparse(
        "import argparse\nC = 1\nC = 2\nap = argparse.ArgumentParser()\n"
        "ap.add_argument('--x', type=int, default=C)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default is None
    assert f.degraded is True


# --------------------------------------------------------------------------
# click: @click.option(default=CONST)
# --------------------------------------------------------------------------


def test_click_constant_default_resolves():
    spec = argspec.read_cli(
        "import click\nCONST = 'prod'\n@click.command()\n"
        "@click.option('--n', default=CONST)\ndef m(n): pass\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "prod"
    assert f.degraded is False


def test_click_constant_also_read_inside_the_body_still_resolves():
    # The positive, guarded against over-tightening: the const is READ again inside the
    # command body. Reads are not bindings, so it stays single-bound and resolves.
    spec = argspec.read_cli(
        "import click\nCONST = 'prod'\n@click.command()\n"
        "@click.option('--n', default=CONST)\ndef m(n):\n    print(CONST, n)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "prod"
    assert f.degraded is False


def test_click_secret_constant_default_degrades():
    # C3 holds on click's surface too: the key never leaves the script's own text.
    spec = argspec.read_cli(
        "import click\nAPI_KEY = 'sk-live-abc123'\n@click.command()\n"
        "@click.option('--auth', default=API_KEY)\ndef m(auth): pass\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None
    assert "sk-live-abc123" not in repr(f)


def test_click_unknown_name_default_degrades():
    spec = argspec.read_cli(
        "import click\n@click.command()\n@click.option('--n', default=MISSING)\ndef m(n): pass\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True


# --------------------------------------------------------------------------
# typer: legacy Option positional default, Annotated signature default, bare signature default
# --------------------------------------------------------------------------


def test_typer_legacy_option_constant_default_resolves():
    # `x: str = typer.Option(CONST)` — the first positional of the Option call is the value
    # default, and a Name there resolves through the constant environment.
    spec = argspec.read_cli(
        "import typer\nCONST = 'prod'\n"
        "def main(x: str = typer.Option(CONST)):\n    pass\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "prod"
    assert f.degraded is False


def test_typer_annotated_signature_constant_default_resolves():
    # `x: Annotated[str, typer.Option()] = CONST` — the `= value` default is a Name and resolves.
    spec = argspec.read_cli(
        "import typer\nfrom typing import Annotated\nCONST = 'prod'\n"
        "def main(x: Annotated[str, typer.Option()] = CONST):\n    pass\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "prod"
    assert f.degraded is False


def test_typer_bare_signature_constant_default_resolves():
    # `x: int = CONST` — a plain signature default naming an int const resolves to its value.
    spec = argspec.read_cli(
        "import typer\nCONST = 42\ndef main(x: int = CONST):\n    pass\ntyper.run(main)\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.type == "int"
    assert f.default == 42
    assert f.degraded is False


def test_typer_unknown_signature_default_degrades():
    # A signature default naming an unknown constant degrades (unchanged non-resolvable behavior).
    spec = argspec.read_cli(
        "import typer\ndef main(x: int = MISSING):\n    pass\ntyper.run(main)\n"
    )
    assert spec is not None
    assert spec.fields[0].degraded is True


# --------------------------------------------------------------------------
# JS/TS parseArgs: default naming a top-level const
# --------------------------------------------------------------------------


def test_js_constant_default_resolves():
    # `default: DEFAULT_HOST` naming a top-level `const` resolves as if the literal were inline.
    spec = cli_reader.read_cli(
        'const DEFAULT_HOST = "example.com";\n'
        'parseArgs({options:{host:{type:"string", default: DEFAULT_HOST}}});\n'
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "example.com"
    assert f.degraded is False


def test_js_let_binding_default_does_not_resolve():
    # A `let` binding is demoted (reassignable), so it is excluded from the constant
    # environment and the field degrades.
    spec = cli_reader.read_cli(
        'let HOST = "example.com";\nparseArgs({options:{host:{type:"string", default: HOST}}});\n'
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_js_reassigned_const_default_does_not_resolve():
    # A const that is nonetheless reassigned is a working variable -> mutated -> excluded.
    spec = cli_reader.read_cli(
        'const HOST = "a";\nHOST = "b";\n'
        'parseArgs({options:{host:{type:"string", default: HOST}}});\n'
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_js_unknown_identifier_default_degrades():
    # An identifier that names nothing top-level and literal degrades the field.
    spec = cli_reader.read_cli('parseArgs({options:{host:{type:"string", default: UNKNOWN}}});\n')
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_js_function_local_const_shadow_does_not_resolve():
    # `_const_candidates` only sees the TOP-LEVEL `const HOST`, so without the file-wide
    # declaration count the field would resolve to the outer "localhost" — overriding the
    # inner value the script would actually have used, on every run. Two declarations of
    # the name means the harvested literal isn't provably the one in scope: degrade.
    spec = cli_reader.read_cli(
        'const HOST = "localhost";\n'
        "function main() {\n"
        '  const HOST = process.env.HOST ?? "prod.internal";\n'
        '  parseArgs({options:{host:{type:"string", default: HOST}}});\n'
        "}\nmain();\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_js_function_parameter_shadow_does_not_resolve():
    # A formal PARAMETER of the same name shadows the top-level const just as a local
    # declaration does — `_declared_names` counts parameters, so this degrades too.
    spec = cli_reader.read_cli(
        'const HOST = "localhost";\n'
        "function main(HOST) {\n"
        '  parseArgs({options:{host:{type:"string", default: HOST}}});\n'
        '}\nmain("prod.internal");\n'
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_js_constant_read_as_a_parameter_default_still_resolves():
    # `function main(a = HOST)` BINDS `a` and merely READS HOST. Counting that read as a
    # declaration would make the constant look bound twice and refuse to fold a value
    # that is provably still the one literal — a false negative that would quietly
    # degrade fields as soon as a script used its own constant as a parameter default.
    spec = cli_reader.read_cli(
        'const HOST = "localhost";\n'
        "function main(a = HOST) { return a; }\n"
        'parseArgs({options:{host:{type:"string", default: HOST}}});\n'
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "localhost"
    assert f.degraded is False


def test_ts_typed_parameter_default_reads_the_constant_without_declaring_it():
    # TypeScript's shape differs from JS's: `pattern`, `type` and `value` all hang off one
    # required_parameter, so `a: string = HOST` reaches HOST through a SIBLING of the
    # binding rather than through an assignment_pattern. Only `pattern` binds a name —
    # counting the sibling read would refuse to fold a constant that is still one literal.
    spec = cli_reader.read_cli(
        'const HOST = "localhost";\n'
        "function main(a: string = HOST) { return a; }\n"
        'parseArgs({options:{host:{type:"string", default: HOST}}});\n',
        lang="ts",
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "localhost"
    assert f.degraded is False


def test_ts_destructured_parameter_default_is_also_only_a_read():
    # Same rule through a destructuring pattern: `{x}: Opts = DEF` binds x, reads DEF.
    spec = cli_reader.read_cli(
        'const DEF = "d";\n'
        "function main({x}: Opts = DEF) { return x; }\n"
        'parseArgs({options:{host:{type:"string", default: DEF}}});\n',
        lang="ts",
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "d"
    assert f.degraded is False


def test_ts_typed_parameter_with_a_default_still_shadows_by_its_bound_name():
    # The other half: the typed parameter's own name still shadows the top-level const,
    # default or no default — `pattern` is exactly what gets counted.
    spec = cli_reader.read_cli(
        'const HOST = "localhost";\n'
        'function main(HOST: string = "inner.example.com") {\n'
        '  parseArgs({options:{host:{type:"string", default: HOST}}});\n'
        "}\nmain();\n",
        lang="ts",
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_js_parameter_with_a_default_still_shadows_by_its_bound_name():
    # The other half of the same branch: in `function main(HOST = "x")` the LEFT of the
    # default IS the bound name, so it shadows the top-level const and blocks folding.
    spec = cli_reader.read_cli(
        'const HOST = "localhost";\n'
        'function main(HOST = "inner.example.com") {\n'
        '  parseArgs({options:{host:{type:"string", default: HOST}}});\n'
        "}\nmain();\n"
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_ts_typed_function_parameter_shadow_does_not_resolve():
    # The TypeScript grammar wraps a parameter in a required_parameter pattern rather than
    # exposing a bare identifier, so the parameter walk has to reach INSIDE it — the typed
    # spelling is a distinct shape and gets its own pin.
    spec = cli_reader.read_cli(
        'const HOST: string = "localhost";\n'
        "function main(HOST: string) {\n"
        '  parseArgs({options:{host:{type:"string", default: HOST}}});\n'
        '}\nmain("prod.internal");\n',
        lang="ts",
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None


def test_js_secret_constant_never_resolves():
    # C3 on the JS surface: a hardcoded key must not escape the script's own text through a
    # resolved field default (prefill, `show --json`, preset TOML on disk).
    spec = cli_reader.read_cli(
        'const API_KEY = "sk-live-abc123";\n'
        'parseArgs({options:{auth:{type:"string", default: API_KEY}}});\n'
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.degraded is True
    assert f.default is None
    assert "sk-live-abc123" not in repr(f)


def test_ts_constant_default_resolves():
    # The same resolution holds under the TypeScript grammar (annotated const declaration).
    spec = cli_reader.read_cli(
        'const DEFAULT_HOST: string = "example.com";\n'
        'parseArgs({options:{host:{type:"string", default: DEFAULT_HOST}}});\n',
        lang="ts",
    )
    assert spec is not None
    f = spec.fields[0]
    assert f.default == "example.com"
    assert f.degraded is False
