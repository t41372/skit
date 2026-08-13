//! Exact-name completeness gate for Python v0.4 `tests/test_shell_analyzer.py` at `main@206f9ef`.
//!
//! Rust statically owns the Shell parser; only Python's two lazy-import failure seams are closed.
//! Every source-analysis, reconcile, public analyzer/injector invariant, and CLI contract remains
//! executable. The gate parses real `#[test] fn test_*` items and rejects missing or invented names.
use std::collections::BTreeSet;
use syn::{Attribute,Item};
const EXECUTABLE:&[&str]=&[
"test_const_word_number_raw_double_quoted","test_const_excludes_empty_array_concat_expansion_cmdsub","test_const_leading_underscore_skipped","test_const_last_write_wins_keeps_first_slot","test_const_plus_equals_is_not_a_literal_const","test_declaration_export_declare_typeset_included_local_excluded","test_readonly_and_declare_r_excluded","test_envdefault_all_four_operators","test_envdefault_non_default_operators_ignored","test_envdefault_type_inference_on_default","test_envdefault_empty_default","test_envdefault_subscript_skipped","test_envdefault_dedupes_by_name_first_default_wins","test_envdefault_carries_env_name","test_self_idiom_is_envdefault_not_suppressed","test_suppression_bare_literal_assignment_wins","test_suppression_cmdsub_assignment_shadows_envdefault","test_suppression_only_targets_the_shadowed_name","test_read_prompt_and_order_keys","test_read_secret_certainty_via_dash_s","test_read_clustered_sp","test_read_clustered_rp_not_secret","test_read_multiple_varnames_share_prompt","test_read_dynamic_prompt_collapses_to_empty","test_read_prompt_from_bare_word","test_read_attached_prompt","test_read_value_flags_skip_their_argument","test_read_attached_value_flag_not_consumed","test_reframing_reads_are_excluded_from_candidacy","test_custom_ifs_reads_are_excluded_from_candidacy","test_read_end_of_options_marker","test_read_single_dash_is_a_varname","test_read_non_word_argument_skipped","test_read_dash_p_at_end_no_argument","test_builtin_and_command_read_recognized","test_non_read_command_ignored","test_builtin_without_read_is_not_a_read","test_bare_builtin_is_not_a_read","test_read_secret_by_varname_and_prompt","test_data_read_pipe_right_operand_excluded","test_data_read_pipe_three_stage_excluded","test_read_first_pipe_operand_is_interactive","test_data_read_loop_fed_by_file_redirect_excluded","test_data_read_own_stdin_redirect_excluded","test_data_read_herestring_excluded","test_data_read_heredoc_loop_excluded","test_read_with_output_redirect_is_still_interactive","test_demote_plus_equals","test_demote_arithmetic_self_reference","test_demote_postfix_increment","test_demote_arithmetic_compound_assignment","test_demote_let_target","test_demote_loop_body_reassignment","test_non_mutated_const_not_demoted","test_arithmetic_read_only_does_not_demote","test_subscript_assignment_is_not_a_const_or_mutation","test_subscript_loop_reassignment_ignored","test_arithmetic_subscript_mutation_has_no_named_target","test_let_with_non_identifier_argument","test_postfix_on_subscript_marks_the_base_name","test_uses_self_location_dollar_zero","test_uses_self_location_bash_source_and_subscript","test_no_self_location","test_uses_argv_positional","test_uses_argv_special_at_hash_star","test_uses_argv_getopts_and_shift","test_dollar_zero_is_not_argv","test_other_special_variables_are_not_argv","test_no_argv","test_type_leading_zeros_read_as_int","test_type_negative_int","test_type_negative_float","test_type_dotted_version_is_str","test_type_never_bool","test_has_error_returns_empty_syntax_error","test_empty_script","test_reconcile_const_and_input_parity","test_reconcile_envdefault_ok","test_reconcile_envdefault_default_change_is_still_ok","test_reconcile_envdefault_gone_is_missing","test_reconcile_envdefault_bare_assignment_shadow_is_missing","test_envdefault_loud_drift_line","test_envdefault_unmanaged_is_new_not_drift","test_params_manage_writes_block_into_shell_copy","test_params_show_lists_shell_const_and_unmanaged","test_params_show_getopts_shell_stops_advertising_manage","test_params_resync_reports_drift_after_edit","test_analyzer_and_injector_share_one_read_enumeration","test_read_flags_do_not_read_letters_from_an_attached_value","test_read_cluster_keeps_scanning_past_an_unknown_flag_letter"];
const CLOSED:&[&str]=&["test_import_guard_degrades_analyzer_to_none","test_plan_degrades_to_none_when_analyzer_missing"];
fn has_test(a:&[Attribute])->bool{a.iter().any(|x|x.path().is_ident("test"))}
fn names(source:&str)->BTreeSet<String>{syn::parse_file(source).expect("shell analyzer port source must parse").items.into_iter().filter_map(|item|match item{Item::Fn(f)if has_test(&f.attrs)&&f.sig.ident.to_string().starts_with("test_")=>Some(f.sig.ident.to_string()),_=>None}).collect()}
#[test]
fn test_shell_analyzer_frozen_names_are_exactly_accounted(){
 assert_eq!(EXECUTABLE.len(),90);assert_eq!(CLOSED.len(),2);
 let expected=EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();let closed=CLOSED.iter().copied().collect::<BTreeSet<_>>();
 assert_eq!(expected.len(),90,"duplicate executable oracle");assert_eq!(closed.len(),2,"duplicate closed oracle");assert!(expected.is_disjoint(&closed));assert_eq!(expected.len()+closed.len(),92);
 let mut actual=BTreeSet::new();
 for source in [
  include_str!("../../skit-language/tests/port_test_shell_analyzer_bindings.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_demotions.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_hints.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_types.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_reads_options.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_reads_edges.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_reads_nonread.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_data_reads.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_reconcile.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_enumeration.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_attached_flags.rs"),
  include_str!("../../skit-language/tests/port_test_shell_analyzer_unknown_flag_cluster.rs"),
  include_str!("port_test_shell_analyzer_cli.rs")]{actual.extend(names(source));}
 let actual=actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
 assert_eq!(actual,expected,"Shell analyzer executable parity is incomplete or mislabeled");assert!(actual.is_disjoint(&closed));
}
