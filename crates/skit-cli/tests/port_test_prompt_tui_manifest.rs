//! Live exact-name audit for frozen `tests/test_prompt_tui.py`.
//!
//! This guard is intentionally not attached to the master inventory until all 83 frozen names are
//! accounted. Textual-vs-Ratatui is not an architecture closure by itself: equivalent public
//! reducer/session/event/store behavior must remain executable.

use std::{collections::{BTreeMap,BTreeSet},fs,path::Path};
use syn::{Attribute,Item};

const CLOSED:&[(&str,&str)]=&[];
const OWNERS:&[&str]=&[
    "crates/skit-tui/tests/port_test_prompt_tui_run_form.rs",
    "crates/skit-ui/tests/port_test_prompt_tui_settings_core.rs",
    "crates/skit-ui/tests/port_test_prompt_tui_review_core.rs",
    "crates/skit-tui/tests/port_test_prompt_tui_review_picker.rs",
    "crates/skit-ui/tests/port_test_prompt_tui_noop_capabilities.rs",
    "crates/skit-cli/tests/port_test_prompt_tui_runner_modal.rs",
    "crates/skit-tui/tests/port_test_prompt_tui_settings_candidates.rs",
];

fn has_test(attrs:&[Attribute])->bool{attrs.iter().any(|attr|attr.path().is_ident("test"))}
fn rust_tests(path:&Path)->Vec<String>{let source=fs::read_to_string(path).unwrap_or_else(|error|panic!("could not read {}: {error}",path.display()));let file=syn::parse_file(&source).unwrap_or_else(|error|panic!("could not parse {}: {error}",path.display()));file.items.iter().filter_map(|item|match item{Item::Fn(function) if has_test(&function.attrs)=>{let name=function.sig.ident.to_string();name.starts_with("test_").then_some(name)},_=>None}).collect()}
fn frozen_names(source:&str)->Vec<String>{source.lines().filter_map(|line|{let line=line.trim_start();let rest=line.strip_prefix("async def test_").or_else(||line.strip_prefix("def test_"))?;let tail=rest.split_once('(')?.0;Some(format!("test_{tail}"))}).collect()}

#[test]
fn prompt_tui_frozen_name_audit_is_complete(){
 let repo=Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("skit-cli lives under <repo>/crates/skit-cli");
 let python=fs::read_to_string(repo.join("tests/test_prompt_tui.py")).expect("preserved Prompt-TUI source");let frozen_list=frozen_names(&python);let frozen=frozen_list.iter().map(String::as_str).collect::<BTreeSet<_>>();
 assert_eq!(frozen_list.len(),83,"frozen Prompt-TUI denominator changed");assert_eq!(frozen.len(),83,"duplicate frozen Prompt-TUI name");
 for sentinel in ["test_form_picker_mouse_click_picks_a_runner","test_settings_candidate_picker_tolerates_preview_recompose","test_review_ctrl_e_keeps_placeholder_ticks_by_name_across_flood_transitions","test_library_edit_prompt_offers_picker_and_manages_the_selection"]{assert!(frozen.contains(sentinel),"preserved Prompt-TUI source lost sentinel {sentinel}");}
 let closed=CLOSED.iter().map(|(name,_)|*name).collect::<BTreeSet<_>>();assert_eq!(closed.len(),CLOSED.len(),"duplicate Prompt-TUI closure name");assert!(CLOSED.iter().all(|(_,reason)|!reason.trim().is_empty()));assert!(closed.is_subset(&frozen),"Prompt-TUI closure includes non-frozen name");
 let mut owners=BTreeMap::<String,String>::new();let mut duplicates=Vec::new();for relative in OWNERS{for name in rust_tests(&repo.join(relative)){if let Some(previous)=owners.insert(name.clone(),(*relative).to_owned()){duplicates.push(format!("{name}: {previous} and {relative}"));}}}assert!(duplicates.is_empty(),"duplicate Prompt-TUI owners:\n{}",duplicates.join("\n"));
 let expected=frozen.difference(&closed).copied().collect::<BTreeSet<_>>();let actual=owners.keys().map(String::as_str).collect::<BTreeSet<_>>();let missing=expected.difference(&actual).copied().collect::<Vec<_>>();let extras=actual.difference(&expected).copied().collect::<Vec<_>>();
 assert!(missing.is_empty()&&extras.is_empty(),"Prompt-TUI exact-name audit incomplete: executable={}/{} closed={} missing={missing:?} extras={extras:?}",actual.len(),frozen.len(),closed.len());
}
