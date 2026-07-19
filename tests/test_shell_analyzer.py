"""Shell analyzer unit pins: const/envdefault/read detection, the suppression rule, the
data-read exclusion ancestry walk, demotions, hints, type inference, secret certainty, order
keys, last-write-wins, the has_error degradation path, the reconcile drift matrix (incl. the
envdefault loud line), the registry import guard, and the `skit params` shell integration.
"""

from __future__ import annotations

import sys

import pytest
from typer.testing import CliRunner

from skit import analysis, cli, flows, store
from skit.langs import registry
from skit.langs.shell import analyzer as shell
from skit.params import ParamDecl

runner = CliRunner()


def cands(src: str) -> list[analysis.Candidate]:
    return shell.analyze(src).candidates


def by_name(src: str) -> dict[str, analysis.Candidate]:
    return {c.name: c for c in cands(src)}


# ---------------------------------------------------------------- const


def _read_flags_of(src: str):
    import tree_sitter
    import tree_sitter_bash

    from skit.langs.shell import analyzer as a

    lang = tree_sitter.Language(tree_sitter_bash.language())
    for n in a._walk(tree_sitter.Parser(lang).parse(src.encode()).root_node):
        if n.type == "command":
            return a._read_flags(n)
    return None


def test_const_word_number_raw_double_quoted():
    b = by_name("A=plain\nB=42\nC='raw text'\nD=\"double q\"\n")
    assert (b["A"].type, b["A"].default) == ("str", "plain")
    assert (b["B"].type, b["B"].default) == ("int", 42)
    assert (b["C"].type, b["C"].default) == ("str", "raw text")
    assert (b["D"].type, b["D"].default) == ("str", "double q")


def test_const_excludes_empty_array_concat_expansion_cmdsub():
    src = "EMPTY=\nQUOTED_EMPTY=''\nARR=(1 2 3)\nCONCAT=a$B\nSUBBED=$(date)\nEXPANDED=${OTHER}\n"
    assert cands(src) == []  # none is a plain literal


def test_const_leading_underscore_skipped():
    assert [c.name for c in cands("_HIDDEN=1\nSHOWN=2\n")] == ["SHOWN"]


def test_const_last_write_wins_keeps_first_slot():
    b = by_name("X=1\nY=5\nX=2\n")
    assert b["X"].default == 2  # last value wins
    names = [c.name for c in cands("X=1\nY=5\nX=2\n")]
    assert names.index("X") < names.index("Y")  # first slot kept


def test_const_plus_equals_is_not_a_literal_const():
    # A bare += with no prior literal assignment yields no const candidate (it's an accumulator).
    assert [c.name for c in cands("N+=1\n")] == []


def test_declaration_export_declare_typeset_included_local_excluded():
    src = (
        "export A=1\n"
        "declare -i B=2\n"
        "typeset C=three\n"
        "local D=4\n"  # function scope — never a top-level const
    )
    assert {c.name for c in cands(src)} == {"A", "B", "C"}


def test_readonly_and_declare_r_excluded():
    src = "readonly LOCKED=1\ndeclare -r FROZEN=2\ntypeset -rx ALSO=3\nOPEN=4\n"
    assert [c.name for c in cands(src)] == ["OPEN"]


# ---------------------------------------------------------------- envdefault


def test_envdefault_all_four_operators():
    b = by_name(': "${A:-x}"\n: "${B:=y}"\n: "${C-z}"\n: "${D=w}"\n')
    assert set(b) == {"A", "B", "C", "D"}
    assert all(c.binding == "envdefault" for c in b.values())
    assert b["A"].default == "x"


def test_envdefault_non_default_operators_ignored():
    # ${VAR:?err}, ${#VAR}, ${VAR#pat}, ${VAR/a/b} are not defaults.
    assert cands(': "${VAR:?missing}"\necho "${#LIST}"\n') == []


def test_envdefault_type_inference_on_default():
    b = by_name(': "${PORT:-8080}"\n: "${RATIO:-1.5}"\n: "${NAME:-guest}"\n')
    assert (b["PORT"].type, b["PORT"].default) == ("int", 8080)
    assert (b["RATIO"].type, b["RATIO"].default) == ("float", 1.5)
    assert b["NAME"].type == "str"


def test_envdefault_empty_default():
    (c,) = cands(': "${OPT:-}"\n')
    assert (c.type, c.default) == ("str", "")


def test_envdefault_subscript_skipped():
    assert cands('echo "${ARR[0]:-x}"\n') == []


def test_envdefault_dedupes_by_name_first_default_wins():
    (c,) = cands('echo "${MODE:-first}"\necho "${MODE:-second}"\n')
    assert c.default == "first"


def test_envdefault_carries_env_name():
    (c,) = cands(': "${TOKEN_URL:-http://x}"\n')
    assert c.env_name == "TOKEN_URL" == c.name


def test_self_idiom_is_envdefault_not_suppressed():
    b = by_name('PORT="${PORT:-8080}"\nNAME=${NAME:-guest}\n')
    assert b["PORT"].binding == "envdefault"
    assert b["NAME"].binding == "envdefault"


# ---------------------------------------------------------------- suppression (risk #1)


def test_suppression_bare_literal_assignment_wins():
    b = by_name('PORT=8080\necho "${PORT:-9090}"\n')
    assert b["PORT"].binding == "const"
    assert "PORT" not in [
        c.name for c in cands('PORT=8080\necho "${PORT:-9090}"\n') if c.binding == "envdefault"
    ]


def test_suppression_cmdsub_assignment_shadows_envdefault():
    # A non-literal clobbering assignment isn't a const candidate, but still suppresses the env.
    assert cands('HOST=$(hostname)\necho "${HOST:-local}"\n') == []


def test_suppression_only_targets_the_shadowed_name():
    b = by_name('PORT=8080\necho "${PORT:-9090}"\necho "${MODE:-auto}"\n')
    assert b["MODE"].binding == "envdefault"


# ---------------------------------------------------------------- read


def _reads(src: str) -> list[analysis.Candidate]:
    return [c for c in cands(src) if c.binding == "input"]


def test_read_prompt_and_order_keys():
    rs = _reads('read -p "Name: " NAME\nread -p "Age: " AGE\n')
    assert [(c.name, c.order, c.prompt) for c in rs] == [
        ("input-1", 0, "Name: "),
        ("input-2", 1, "Age: "),
    ]


def test_read_secret_certainty_via_dash_s():
    (c,) = _reads('read -s -p "Enter value: " V\n')
    assert c.secret is True  # -s is certainty, not a name heuristic


def test_read_clustered_sp():
    (c,) = _reads('read -sp "PIN: " PIN\n')
    assert (c.secret, c.prompt) == (True, "PIN: ")


def test_read_clustered_rp_not_secret():
    (c,) = _reads('read -rp "Confirm: " C\n')
    assert (c.secret, c.prompt) == (False, "Confirm: ")


def test_read_multiple_varnames_share_prompt():
    rs = _reads('read -p "Two: " FIRST LAST\n')
    assert [c.name for c in rs] == ["input-1", "input-2"]
    assert all(c.prompt == "Two: " for c in rs)


def test_read_dynamic_prompt_collapses_to_empty():
    (c,) = _reads('read -p "$MSG" V\n')
    assert c.prompt == ""


def test_read_prompt_from_bare_word():
    (c,) = _reads("read -p Enter: V\n")
    assert c.prompt == "Enter:"


def test_read_attached_prompt():
    (c,) = _reads("read -pHello V\n")
    assert c.prompt == "Hello"


def test_read_value_flags_skip_their_argument():
    # -t 5 and -u 0 consume their value; only V is a varname. (-n/-N/-d also consume theirs, but
    # they REFRAME the input, so such a read is excluded outright — see the reframing test below.)
    (c,) = _reads("read -t 5 -u 0 V\n")
    assert c.name == "input-1"


def test_read_attached_value_flag_not_consumed():
    # -t5 attaches its value; W is still the varname.
    (c,) = _reads("read -t5 W\n")
    assert c.name == "input-1"


def test_reframing_reads_are_excluded_from_candidacy():
    # -n/-N/-d make the read stop early or on another delimiter, so the single line skit feeds it is
    # not the value the script would end up with (`read -n 3 X` on "abcdefgh" yields "abc"). Such a
    # read cannot be delivered faithfully, so it is never offered — as `read -a` already isn't.
    for src in ("read -n 3 X\n", "read -N 5 X\n", "read -d : X\n", "read -n3 X\n"):
        assert _reads(src) == [], src


def test_custom_ifs_reads_are_excluded_from_candidacy():
    # skit joins a multi-var read's values with a SPACE and relies on default $IFS to split them
    # back. A custom IFS breaks that in both directions: `IFS=: read A B` would hand the whole line
    # to A, and `IFS= read -r LINE` does no splitting or edge-stripping at all (so a value skit
    # would refuse as unsafe actually arrives intact). Neither is offered.
    assert _reads("IFS=: read A B\n") == []
    assert _reads("IFS= read -r LINE\n") == []
    assert len(_reads('read -p "p: " A B\n')) == 2  # an ordinary read still is


def test_read_end_of_options_marker():
    # After --, a dash-led token is a varname, not a flag.
    rs = _reads("read -- -weird\n")
    assert [c.prompt for c in rs] == [""]


def test_read_single_dash_is_a_varname():
    assert len(_reads("read -\n")) == 1


def test_read_non_word_argument_skipped():
    # A string arg between flags/varnames isn't a plain word varname.
    rs = _reads('read "$dyn" REAL\n')
    assert [c.name for c in rs] == ["input-1"]


def test_read_dash_p_at_end_no_argument():
    assert _reads("read -p\n") == []  # no varname, no prompt source; exercises the branch


def test_builtin_and_command_read_recognized():
    assert len(_reads("builtin read X\n")) == 1
    assert len(_reads("command read Y\n")) == 1


def test_non_read_command_ignored():
    assert _reads("echo hello\nprintf '%s' x\n") == []


def test_builtin_without_read_is_not_a_read():
    assert _reads("builtin pwd\ncommand ls\n") == []


def test_bare_builtin_is_not_a_read():
    assert _reads("builtin\n") == []


def test_read_secret_by_varname_and_prompt():
    assert _reads("read PASSWORD\n")[0].secret is True  # is_secret_name(varname)
    assert _reads('read -p "API key: " K\n')[0].secret is True  # is_secret_name(prompt)


# ---------------------------------------------------------------- data-read exclusion (risk #5)


def test_data_read_pipe_right_operand_excluded():
    assert _reads("cat f | while read -r line; do echo $line; done\n") == []


def test_data_read_pipe_three_stage_excluded():
    assert _reads("a | b | read Z\n") == []


def test_read_first_pipe_operand_is_interactive():
    assert len(_reads("read X | cat\n")) == 1  # head of a pipe reads the terminal


def test_data_read_loop_fed_by_file_redirect_excluded():
    assert _reads("while read -r x; do echo $x; done < f\n") == []


def test_data_read_own_stdin_redirect_excluded():
    assert _reads("read -r x < input.txt\n") == []


def test_data_read_herestring_excluded():
    assert _reads('read -r x <<< "$data"\n') == []


def test_data_read_heredoc_loop_excluded():
    assert _reads("while read -r x; do :; done <<EOF\na\nEOF\n") == []


def test_read_with_output_redirect_is_still_interactive():
    # `> out` is stdout, not stdin — the read still prompts.
    assert len(_reads("read -r x > out.log\n")) == 1


# ---------------------------------------------------------------- demotions


def _demoted(src: str) -> set[str]:
    return {c.name for c in cands(src) if c.demoted}


def test_demote_plus_equals():
    assert _demoted("N=0\nN+=1\n") == {"N"}


def test_demote_arithmetic_self_reference():
    assert _demoted("TOTAL=100\nTOTAL=$((TOTAL - 1))\n") == {"TOTAL"}


def test_demote_postfix_increment():
    assert _demoted("N=0\n((N++))\n") == {"N"}


def test_demote_arithmetic_compound_assignment():
    assert _demoted("N=0\n((N += 5))\n") == {"N"}


def test_demote_let_target():
    assert _demoted("M=1\nlet M=M+1\n") == {"M"}


def test_demote_loop_body_reassignment():
    assert _demoted("SUM=0\nfor i in 1 2; do SUM=$((SUM + i)); done\n") == {"SUM"}


def test_non_mutated_const_not_demoted():
    (c,) = cands("STABLE=7\n")
    assert (c.demoted, c.demotion) == (False, "")


def test_arithmetic_read_only_does_not_demote():
    # `(( n > 5 ))` reads n; it must not be mistaken for a mutation.
    assert _demoted("N=3\n(( N > 5 )) && echo big\n") == set()


def test_subscript_assignment_is_not_a_const_or_mutation():
    # arr[0]=5 has a subscript name (not a plain variable_name): never a const, never a suppressor.
    assert cands("arr[0]=5\n") == []


def test_subscript_loop_reassignment_ignored():
    assert cands("arr[0]=1\nfor i in 1 2; do arr[i]=$i; done\n") == []


def test_arithmetic_subscript_mutation_has_no_named_target():
    assert cands("(( arr[0] += 1 ))\n") == []


def test_let_with_non_identifier_argument():
    # `let COUNT=1 999` — COUNT is a target, the bare number 999 contributes nothing.
    (c,) = cands("COUNT=0\nlet COUNT=1 999\n")
    assert (c.name, c.demoted) == ("COUNT", True)


def test_postfix_on_subscript_marks_the_base_name():
    # ((arr[0]++)) mutates an element, but demotes the base name arr (which is a scalar const here).
    assert _demoted("arr=1\n((arr[0]++))\n") == {"arr"}


# ---------------------------------------------------------------- hints


def test_uses_self_location_dollar_zero():
    assert shell.analyze('D=$(dirname "$0")\n').uses_self_location is True


def test_uses_self_location_bash_source_and_subscript():
    assert shell.analyze('echo "$BASH_SOURCE ${BASH_SOURCE[0]}"\n').uses_self_location is True


def test_no_self_location():
    assert shell.analyze("X=1\n").uses_self_location is False


def test_uses_argv_positional():
    assert shell.analyze('echo "$1 $2"\n').uses_argv is True


def test_uses_argv_special_at_hash_star():
    assert shell.analyze('echo "$@ $# $*"\n').uses_argv is True


def test_uses_argv_getopts_and_shift():
    assert shell.analyze('getopts "ab" o\n').uses_argv is True
    assert shell.analyze("shift\n").uses_argv is True


def test_dollar_zero_is_not_argv():
    assert shell.analyze('echo "$0"\n').uses_argv is False


def test_other_special_variables_are_not_argv():
    # $? $$ $! are special variables, but not positional-argument markers.
    assert shell.analyze("echo $? $$ $!\n").uses_argv is False


def test_no_argv():
    assert shell.analyze("X=1\n").uses_argv is False


# ---------------------------------------------------------------- type inference edges


def test_type_leading_zeros_read_as_int():
    (c,) = cands("Z=007\n")
    assert (c.type, c.default) == ("int", 7)  # leading zeros not preserved (documented)


def test_type_negative_int():
    (c,) = cands("N=-3\n")
    assert (c.type, c.default) == ("int", -3)


def test_type_negative_float():
    (c,) = cands("F=-2.5\n")
    assert (c.type, c.default) == ("float", -2.5)


def test_type_dotted_version_is_str():
    (c,) = cands("V=1.5.2\n")
    assert (c.type, c.default) == ("str", "1.5.2")


def test_type_never_bool():
    b = by_name("FLAG=true\nOTHER=false\n")
    assert (b["FLAG"].type, b["FLAG"].default) == ("str", "true")
    assert b["OTHER"].type == "str"


# ---------------------------------------------------------------- degradation


def test_has_error_returns_empty_syntax_error():
    result = shell.analyze("if [[ -n $x ]] { echo hi }\nCONFIG=1\n")
    assert result.syntax_error is True
    assert result.candidates == []


def test_empty_script():
    result = shell.analyze("")
    assert result.candidates == []
    assert result.syntax_error is False


# ---------------------------------------------------------------- reconcile parity + envdefault matrix


def _spec(name: str) -> ParamDecl:
    return ParamDecl(name=name, binding="envdefault", delivery="env", type="str")


def test_reconcile_const_and_input_parity():
    # Shell reconcile handles const (by name) and input (by prompt/order) just like Python.
    text = 'CITY=Taipei\nread -p "Name: " NAME\n'
    specs = [
        ParamDecl(name="CITY", binding="const", type="str"),
        ParamDecl(name="input-1", binding="input", order=0, prompt="Name: "),
    ]
    report = shell.reconcile(text, specs)
    assert not report.has_drift
    assert {s.name for s in report.ok} == {"CITY", "input-1"}


def test_reconcile_envdefault_ok():
    report = shell.reconcile('echo "${PORT:-8080}"\n', [_spec("PORT")])
    assert not report.has_drift
    assert [s.name for s in report.ok] == ["PORT"]


def test_reconcile_envdefault_default_change_is_still_ok():
    # The default text changed (8080 -> 9090); env delivery still works, so no drift.
    report = shell.reconcile('echo "${PORT:-9090}"\n', [_spec("PORT")])
    assert not report.has_drift
    assert [s.name for s in report.ok] == ["PORT"]


def test_reconcile_envdefault_gone_is_missing():
    report = shell.reconcile("echo hello\n", [_spec("PORT")])
    assert report.has_drift
    assert [s.name for s in report.missing] == ["PORT"]


def test_reconcile_envdefault_bare_assignment_shadow_is_missing():
    # A plain assignment now clobbers PORT — the env value would be silently ignored.
    report = shell.reconcile('PORT=8080\necho "${PORT:-9090}"\n', [_spec("PORT")])
    assert report.has_drift
    assert [s.name for s in report.missing] == ["PORT"]


def test_envdefault_loud_drift_line():
    report = shell.reconcile("echo hello\n", [_spec("PORT")])
    lines = analysis.drift_lines(report, "deploy")
    joined = "\n".join(lines)
    assert "no longer read from the environment" in joined
    assert "PORT" in joined
    # It's the loud env line, not the generic "injection target" one.
    assert "injection target no longer exists" not in joined


def test_envdefault_unmanaged_is_new_not_drift():
    report = shell.reconcile('echo "${LOG_LEVEL:-info}"\n', [])
    assert not report.has_drift
    assert [c.name for c in report.new] == ["LOG_LEVEL"]


# ---------------------------------------------------------------- registry import guard


def _break_shell_import(monkeypatch: pytest.MonkeyPatch) -> None:
    """Make `from .shell import analyzer` inside the registry raise ImportError: drop the cached
    submodule attribute off the package AND null out its sys.modules entry (both are needed — an
    already-imported submodule lingers as a package attribute that `from … import` would reuse)."""
    import skit.langs.shell as shell_pkg

    registry.spec_for.cache_clear()
    monkeypatch.delattr(shell_pkg, "analyzer", raising=False)
    monkeypatch.setitem(sys.modules, "skit.langs.shell.analyzer", None)  # type: ignore[arg-type]


def test_import_guard_degrades_analyzer_to_none(monkeypatch):
    try:
        _break_shell_import(monkeypatch)
        spec = registry.spec_for("shell")
        assert spec is not None
        assert spec.analyzer is None  # degraded
        assert spec.params_io is not None  # everything else still works
    finally:
        registry.spec_for.cache_clear()


def test_plan_degrades_to_none_when_analyzer_missing(monkeypatch, tmp_path):
    src = tmp_path / "s.sh"
    src.write_text("#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n")
    entry = store.add_script(src, kind="shell", name="degraded")
    try:
        _break_shell_import(monkeypatch)
        plan = flows.plan_for_entry(entry)
        assert plan.source == "none"  # no analyzer -> no inject plan, entry still launchable
    finally:
        registry.spec_for.cache_clear()


# ---------------------------------------------------------------- `skit params` shell integration


def test_params_manage_writes_block_into_shell_copy(tmp_path):
    src = tmp_path / "deploy.sh"
    src.write_text("#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n")
    entry = store.add_script(src, kind="shell", name="sh1")
    result = runner.invoke(cli.app, ["params", "sh1", "--manage", "CITY"])
    assert result.exit_code == 0, result.output
    copy_text = entry.script_path.read_text(encoding="utf-8")
    assert "[tool.skit]" in copy_text
    assert 'name = "CITY"' in copy_text
    # The shebang stays on line 1 and the block opens after it.
    assert copy_text.startswith("#!/usr/bin/env bash\n")
    assert copy_text.index("#!") < copy_text.index("# /// script")


def test_params_show_lists_shell_const_and_unmanaged(tmp_path):
    src = tmp_path / "show.sh"
    src.write_text('#!/usr/bin/env bash\nCITY=Taipei\necho "${MODE:-auto}"\n')
    store.add_script(src, kind="shell", name="sh2")
    result = runner.invoke(cli.app, ["params", "sh2"])
    assert result.exit_code == 0, result.output
    # Both the const and the envdefault are detected as manageable candidates.
    assert "CITY" in result.output
    assert "MODE" in result.output


def test_params_show_getopts_shell_stops_advertising_manage(tmp_path):
    """A getopts shell drives its OWN CLI (uses_cli_framework): the read view must NOT
    advertise --manage — offering to manage a bare constant there would shadow the getopts
    form (the same gate the add panel and CLI onboarding fire). Reader-driven non-python
    entries stop advertising it; python argparse is unchanged."""
    src = tmp_path / "g.sh"
    src.write_text(
        '#!/usr/bin/env bash\nOUT=hello\nwhile getopts "n:v" o; do :; done\necho "$OUT"\n'
    )
    store.add_script(src, kind="shell", name="gsh")
    result = runner.invoke(cli.app, ["params", "gsh"])
    assert result.exit_code == 0, result.output
    assert "--manage" not in result.output  # its own parser IS the interface
    assert "Detected but not yet managed" not in result.output
    assert "gsh has no managed parameters." in result.output  # the plain, --manage-free line


def test_params_resync_reports_drift_after_edit(tmp_path):
    src = tmp_path / "drift.sh"
    src.write_text("#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n")
    entry = store.add_script(src, kind="shell", name="sh3")
    assert runner.invoke(cli.app, ["params", "sh3", "--manage", "CITY"]).exit_code == 0
    copy = entry.script_path
    text = copy.read_text(encoding="utf-8")
    # Rename the constant in the copy: CITY is gone from the script, but still managed.
    copy.write_text(text.replace("CITY=Taipei", "TOWN=Taipei").replace("$CITY", "$TOWN"))
    result = runner.invoke(cli.app, ["params", "sh3", "--resync"])
    assert result.exit_code == 0, result.output
    assert "CITY" in result.output  # the drop is reported
    assert 'name = "CITY"' not in copy.read_text(encoding="utf-8")  # and applied


def test_analyzer_and_injector_share_one_read_enumeration():
    # The invariant that kept breaking: the analyzer (numbering candidates) and the injector
    # (numbering rewrite sites) must agree on WHICH reads count and in what order, or a value lands
    # on the wrong `read`. They now consume the same injectable_reads(), so they cannot diverge.
    from skit.langs.shell import inject
    from skit.langs.shell.analyzer import injectable_reads

    for src in (
        "read -n 3 CODE\nread NAME\n",  # a reframing read the analyzer drops
        "IFS=: read A B\nread NAME\n",  # an IFS-prefixed read the analyzer drops
        "read P\nread Q\nread R\n",
        "cmd | while read x; do :; done\nread TOP\n",  # a data-read the analyzer drops
    ):
        root = inject._root(src)
        n_reads = sum(len(flags.varnames) for _node, flags in injectable_reads(root))
        assert len([c for c in shell.analyze(src).candidates if c.binding == "input"]) == n_reads
        assert len(inject._read_sites(root)) == n_reads


def test_read_flags_do_not_read_letters_from_an_attached_value():
    # -pSure? has an 'r' in "Sure?"; -pEnter an 'n'; -idefault a 'd'. None must flip raw/reframing —
    # those letters are the prompt/default TEXT, not option letters.
    assert not _read_flags_of("read -pSure? X").raw
    assert not _read_flags_of("read -pEnter X").reframing
    assert not _read_flags_of("read -idefault X").reframing
    assert _read_flags_of("read -r X").raw  # a real -r still registers
    # an ATTACHED reframing value (`-n3`) is still detected and still excludes the read
    assert _read_flags_of("read -n3 X").reframing
    assert shell.analyze("read -n3 X\n").candidates == []


def test_read_cluster_keeps_scanning_past_an_unknown_flag_letter():
    # `-er`: 'e' (readline edit, no value) is unknown to the value-flag set, so the scan continues
    # to 'r', which registers raw. The value still delivers as a normal read varname.
    flags = _read_flags_of("read -er X")
    assert flags.raw is True
    assert flags.varnames == ["X"]
    assert shell.analyze("read -er X\n").candidates[0].name == "input-1"
