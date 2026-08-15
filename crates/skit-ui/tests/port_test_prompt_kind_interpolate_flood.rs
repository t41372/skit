use std::path::PathBuf;

use skit_application::SourcePermissions;
use skit_domain::{EntrySettings, parameters::synthesized_placeholder};
use skit_form::{FormSource, form_plan};
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

fn review(body:&str,defaults:ReviewDefaults)->ReviewState{
    ReviewState::from_source(SourceSnapshot{path:PathBuf::from("/work/p.prompt.md"),source_record:"/work/p.prompt.md".to_owned(),bytes:body.as_bytes().to_vec(),permissions:SourcePermissions::default(),is_regular:true,is_directory:false,is_draft:false},KnownEntryKind::Prompt,defaults)
}

#[test]
fn test_add_prompt_interpolate_off_scans_and_manages_nothing() {
    let review=review("{{a}} {{b}}\n",ReviewDefaults{interpolate:Some(false),..ReviewDefaults::default()});
    let create=review.create_entry().unwrap();
    assert!(!create.settings.interpolate);
    assert!(create.settings.params.is_empty());
    assert!(create.settings.parameters.is_empty());
}

#[test]
fn test_add_prompt_auto_manage_flood_cap() {
    const FROZEN_AUTO_LIMIT:usize=30;
    let many=(0..=FROZEN_AUTO_LIMIT).map(|i|format!("{{{{h{i}}}}}")).collect::<Vec<_>>().join(" ");
    let mut review=review(&many,ReviewDefaults::default()); assert!(review.prompt_is_flooded()); assert!(review.selected_prompt_names().is_empty());
    let auto=review.create_entry().unwrap(); assert!(auto.settings.params.is_empty()); assert!(auto.settings.interpolate);
    review.set_prompt_selection(&["h0".to_owned(),"h3".to_owned()]); let explicit=review.create_entry().unwrap(); assert_eq!(explicit.settings.params,["h0","h3"]);
}

#[test]
fn test_plan_for_an_insertion_off_prompt_is_fieldless_and_driftless() {
    let settings=EntrySettings{params:vec!["a".to_owned()],parameters:vec![synthesized_placeholder("a")],interpolate:false,..EntrySettings::default()};
    let plan=form_plan("prompt","no holes anymore\n",&settings); assert_eq!(plan.source,FormSource::Command); assert!(plan.fields.is_empty()); assert!(plan.drift.is_empty());
}

#[test]
fn test_preview_names_caps_the_list() {
    const FROZEN_PREVIEW_LIMIT:usize=20;
    let short=review("{{n0}} {{n1}} {{n2}}",ReviewDefaults::default()); assert_eq!(short.prompt_preview().iter().map(|c|c.name.as_str()).collect::<Vec<_>>(),["n0","n1","n2"]);
    let body=(0..FROZEN_PREVIEW_LIMIT+5).map(|i|format!("{{{{n{i}}}}}")).collect::<Vec<_>>().join(" "); let long=review(&body,ReviewDefaults::default());
    assert_eq!(long.prompt_preview().len(),FROZEN_PREVIEW_LIMIT); assert_eq!(long.prompt_preview().last().unwrap().name,"n19"); assert_eq!(long.prompt_candidates().len(),25); assert_eq!(long.prompt_candidates()[FROZEN_PREVIEW_LIMIT].name,"n20");
}
