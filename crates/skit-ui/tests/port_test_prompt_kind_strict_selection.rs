use std::path::PathBuf;

use skit_application::SourcePermissions;
use skit_ui::{KnownEntryKind,ReviewDefaults,ReviewState,SourceSnapshot};

fn review(body:&str)->ReviewState{ReviewState::from_source(SourceSnapshot{path:PathBuf::from("/work/p.prompt.md"),source_record:"/work/p.prompt.md".to_owned(),bytes:body.as_bytes().to_vec(),permissions:SourcePermissions::default(),is_regular:true,is_directory:false,is_draft:false},KnownEntryKind::Prompt,ReviewDefaults::default())}

#[test]
fn test_add_prompt_refuses_unknown_managed_name(){
 let mut state=review("{{a}}\n");state.set_prompt_selection(&["ghost".to_owned()]);assert!(state.create_entry().is_err(),"an explicit unknown managed name was silently discarded instead of refused");
}
