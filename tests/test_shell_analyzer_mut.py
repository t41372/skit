"""Mutation-kill pins for src/skit/langs/shell/analyzer.py.

These tests isolate behaviours the broader suite (tests/test_shell_analyzer.py) exercises only
indirectly: exact source line numbers on every Candidate, secret certainty carried onto const and
envdefault candidates, the precise `read`-flag parse shape (ReadFlags), the per-`continue`
loop-control in each scanning loop, the single-path loop-body demotion, and the argv special-var
membership one variable at a time. Everything runs the real tree-sitter-bash analyzer on real
shell snippets (English catalog for any message assertions).
"""

from __future__ import annotations

import tree_sitter
import tree_sitter_bash

from skit import analysis
from skit.langs.shell import analyzer as shell
from skit.langs.shell.analyzer import ReadFlags

_LANG = tree_sitter.Language(tree_sitter_bash.language())


def cands(src: str) -> list[analysis.Candidate]:
    return shell.analyze(src).candidates


def by_name(src: str) -> dict[str, analysis.Candidate]:
    return {c.name: c for c in cands(src)}


def reads(src: str) -> list[analysis.Candidate]:
    return [c for c in cands(src) if c.binding == "input"]


def demoted(src: str) -> set[str]:
    return {c.name for c in cands(src) if c.demoted}


def bindings(src: str, name: str) -> list[str]:
    return [c.binding for c in cands(src) if c.name == name]


def read_flags_of(src: str) -> ReadFlags:
    """The parsed ReadFlags of the first `command` node in src (which is always a read here)."""
    root = tree_sitter.Parser(_LANG).parse(src.encode()).root_node
    for node in shell._walk(root):
        if node.type == "command":
            flags = shell._read_flags(node)
            assert flags is not None
            return flags
    raise AssertionError("no command node found in src")


# ---------------------------------------------------------------- exact linenos + secret carry


def test_const_candidate_carries_exact_lineno_and_secret():
    # API_TOKEN sits on line 2 (a leading blank line), column 0. The candidate's lineno is the
    # 1-based *row* (2), never the column (0->1), never row-1/row+2, never a dropped 0, never None;
    # and is_secret_name(API_TOKEN) is carried through as secret=True (not dropped to the False
    # default).
    c = by_name("\nAPI_TOKEN=abcdef\n")["API_TOKEN"]
    assert c.lineno == 2
    assert c.secret is True


def test_const_non_secret_lineno_deeper_in_file():
    # A third-line const pins lineno against the +2 / column mutants from a different row.
    c = by_name("X=1\nY=2\nCITY=Taipei\n")["CITY"]
    assert c.lineno == 3
    assert c.secret is False


def test_envdefault_candidate_carries_exact_lineno_and_secret():
    # ${API_TOKEN:-x} on line 2; the expansion node starts at column 3, so a column-based lineno
    # would read 4, not 2. secret certainty from the name is carried onto the envdefault candidate.
    c = by_name('\n: "${API_TOKEN:-x}"\n')["API_TOKEN"]
    assert c.lineno == 2
    assert c.binding == "envdefault"
    assert c.secret is True


def test_read_candidate_carries_exact_lineno_and_str_type():
    # read on line 2, column 0; lineno is 2 and every read candidate is typed "str".
    (c,) = reads("\nread NAME\n")
    assert c.lineno == 2
    assert c.type == "str"


# ---------------------------------------------------------------- demotion reason string


def test_demoted_const_reports_accumulator_reason():
    # A += accumulator is demoted with the exact symbolic reason "accumulator" (not None, not a
    # cased/garbled variant) — the UI keys its wording off this id.
    c = next(x for x in cands("N=0\nN+=1\n") if x.name == "N")
    assert c.demoted is True
    assert c.demotion == "accumulator"


# ---------------------------------------------------------------- loop-body demotion (single path)


def test_plain_loop_body_reassignment_is_the_only_demotion_path():
    # X=5 inside the loop is a *plain* reassignment: it is not a += and does not self-reference, so
    # the only thing that can demote X is _collect_loop_reassignments walking the loop body. This
    # isolates that walk (the SUM=$((SUM+i)) idiom the broader suite uses is also caught by the
    # arithmetic self-reference path, so it can't pin this one).
    assert demoted("X=1\nfor i in 1 2; do X=5; done\n") == {"X"}


def test_plain_while_loop_reassignment_demotes():
    assert demoted("Y=1\nwhile true; do Y=9; done\n") == {"Y"}


# ---------------------------------------------------------------- per-continue loop control


def test_bare_assigned_skips_subscript_then_records_later_clobber():
    # arr[0]=1 (a subscript target) is skipped by `continue`; the loop must go on to record PORT=8080
    # as a bare clobbering assignment, which suppresses the ${PORT:-9090} envdefault (const wins). A
    # `break` there would stop before PORT and wrongly resurrect the envdefault.
    assert bindings('arr[0]=1\nPORT=8080\necho "${PORT:-9090}"\n', "PORT") == ["const"]


def test_bare_assigned_self_read_continues_to_later_clobber():
    # PORT=${PORT:-8080} self-reads (skipped via `continue`, so it does not suppress itself); the
    # loop must continue to MODE=production, which *does* clobber ${MODE:-dev}. A `break` at the
    # self-read would let MODE wrongly gain an envdefault.
    src = 'PORT=${PORT:-8080}\nMODE=production\necho "${MODE:-dev}"\n'
    assert bindings(src, "MODE") == ["const"]


def test_const_scan_skips_plus_equals_then_finds_later_const():
    # N+=1 is not a literal const (`continue`); the scan must continue to CITY=Taipei. A `break`
    # would abandon the rest of the file and lose CITY.
    assert [c.name for c in cands("N+=1\nCITY=Taipei\n")] == ["CITY"]


def test_envdefault_scan_skips_nondefault_operator_then_finds_default():
    # ${VAR:?err} is not a default operator (`continue`); the walk must reach ${PORT:-8080}. A
    # `break` would stop at the :? expansion and drop PORT.
    src = 'echo "${VAR:?err}"\necho "${PORT:-8080}"\n'
    assert [c.name for c in cands(src) if c.binding == "envdefault"] == ["PORT"]


def test_envdefault_scan_skips_subscript_then_finds_scalar():
    # ${ARR[0]:-x} has a subscript target (`continue`); the walk must reach ${PORT:-8080}.
    src = 'echo "${ARR[0]:-x}"\necho "${PORT:-8080}"\n'
    assert [c.name for c in cands(src) if c.binding == "envdefault"] == ["PORT"]


def test_toplevel_skips_local_then_yields_later_const():
    # A top-level `local` declaration is skipped (`continue`); the walk must reach CITY. A `break`
    # would stop the whole top-level scan at the local.
    assert [c.name for c in cands("local X=1\nCITY=Taipei\n")] == ["CITY"]


def test_injectable_reads_skip_reframing_then_include_normal_read():
    # read -n 3 (reframing) is skipped (`continue`); the walk must still reach `read NAME`. A `break`
    # would drop every read after the reframing one.
    assert len(reads("read -n 3 CODE\nread NAME\n")) == 1


# ---------------------------------------------------------------- read-flag parse shape (ReadFlags)


def test_plain_read_flag_shape():
    # No flags: every boolean is exactly False (not None), prompt is "", the single varname is kept.
    assert read_flags_of("read X") == ReadFlags(
        secret=False, prompt="", varnames=["X"], raw=False, reframing=False
    )


def test_secret_read_flag_shape():
    # -s sets secret; a cluster with no r/n/N/d leaves raw and reframing exactly False (not None),
    # and no -p leaves prompt exactly "".
    assert read_flags_of("read -s X") == ReadFlags(
        secret=True, prompt="", varnames=["X"], raw=False, reframing=False
    )


def test_raw_read_flag_shape():
    # -r sets raw; secret stays exactly False (the cluster init flowing through _scan_read_cluster
    # and _parse_read_args must stay a real bool, not None).
    assert read_flags_of("read -r X") == ReadFlags(
        secret=False, prompt="", varnames=["X"], raw=True, reframing=False
    )


def test_scan_cluster_no_prompt_leaves_prompt_empty():
    # A -s cluster returns an empty prompt so _parse_read_args keeps prompt "" (a non-empty init in
    # the cluster scanner would leak "" -> some literal onto the read).
    assert read_flags_of("read -s X").prompt == ""


def test_dash_p_prompt_consumes_only_a_real_next_arg():
    # `read -p Enter` (Enter is the LAST arg): -p consumes it as the prompt, leaving no varname, so
    # there is no candidate. An off-by-one that required a further arg would instead treat Enter as a
    # varname and emit input-1.
    assert reads("read -p Enter\n") == []


def test_dash_p_prompt_then_varname():
    (c,) = reads('read -p "Question: " ANSWER')
    assert (c.prompt, c.name) == ("Question: ", "input-1")


# ---------------------------------------------------------------- builtin/command read gating


def test_command_prefix_without_read_is_not_a_read():
    # `command ls FILE` is not a read: it must yield no candidate. Loosening the builtin/command
    # guard to an `or` would treat args[1:] (=["FILE"]) as read varnames and emit a bogus input-1.
    assert reads("command ls FILE\n") == []


def test_builtin_prefix_without_read_is_not_a_read():
    assert reads("builtin echo VALUE\n") == []


def test_builtin_read_still_recognized():
    (c,) = reads("builtin read TOWN\n")
    assert c.name == "input-1"


# ---------------------------------------------------------------- IFS prefix detection


def test_non_ifs_var_prefix_does_not_exclude_the_read():
    # `FOO=bar read NAME` has a variable_assignment prefix that is NOT IFS, so the read is still an
    # interactive candidate. Collapsing the IFS check to an `or` would treat any var-prefix as IFS
    # and drop the read.
    assert len(reads("FOO=bar read NAME\n")) == 1


def test_ifs_prefix_still_excludes_the_read():
    assert reads("IFS=: read A B\n") == []


# ---------------------------------------------------------------- let-target gating


def test_non_let_command_arguments_are_not_scanned_for_targets():
    # `echo COUNT=99` is not `let`: its argument must not be mined for assignment targets, so COUNT
    # stays a clean const. Flipping the let-name guard to `and` would scan every command's args and
    # wrongly demote COUNT as a let target.
    assert demoted("COUNT=0\necho COUNT=99\n") == set()


def test_let_command_targets_are_demoted():
    assert demoted("M=1\nlet M=M+1\n") == {"M"}


# ---------------------------------------------------------------- has-readonly-flag long options


def test_long_option_containing_r_is_not_treated_as_readonly():
    # `--xr` is a long option (starts with --); its 'r' must NOT be read as the -r readonly flag, so
    # Y stays a const candidate. A mangled `--` prefix check would treat --xr as readonly and drop Y.
    assert [c.name for c in cands("declare --xr Y=1\n")] == ["Y"]


def test_short_r_flag_is_readonly():
    assert cands("declare -r LOCKED=1\n") == []


# ---------------------------------------------------------------- uses_argv special vars (one each)


def test_uses_argv_at_alone():
    assert shell.analyze('echo "$@"\n').uses_argv is True


def test_uses_argv_star_alone():
    assert shell.analyze('echo "$*"\n').uses_argv is True


def test_uses_argv_hash_alone():
    assert shell.analyze('echo "$#"\n').uses_argv is True


# ---------------------------------------------------------------- j-counter loops (divergence)


def test_secret_flag_after_another_cluster_letter_terminates():
    # `read -rs X`: the scanner must advance past 'r' then 's'. A reset-to-1 on the 's' branch would
    # spin forever on index 1; reaching a result at all (and the right one) pins the increment.
    (c,) = reads("read -rs X")
    assert c.secret is True


def test_unknown_flag_letter_after_another_terminates():
    # `read -re X`: 'r' then the unknown 'e' must both be consumed and scanning must terminate with X
    # as the varname. A reset-to-1 on the unknown-letter branch would loop forever on 'e'.
    (c,) = reads("read -re X")
    assert c.name == "input-1"
