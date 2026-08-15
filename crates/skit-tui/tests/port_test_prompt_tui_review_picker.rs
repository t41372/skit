use std::path::PathBuf;

use ratatui_core::{backend::TestBackend,layout::Rect,terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event,KeyCode,KeyEvent,KeyModifiers,MouseButton,MouseEvent,MouseEventKind,
};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::screens::picker::{
    ChoicePickerHit,PromptCandidatePickerEvent,PromptCandidatePickerSession,
    render_prompt_candidate_picker,
};
use skit_ui::{KnownEntryKind,ReviewDefaults,ReviewState,SourceSnapshot};

const FROZEN_AUTO_MANAGE_LIMIT:usize=30;
const FROZEN_LIST_PREVIEW_LIMIT:usize=20;

fn body(count:usize)->String{(0..count).map(|index|format!("{{{{h{index}}}}}")).collect::<Vec<_>>().join(" ")}
fn review(count:usize)->ReviewState{ReviewState::from_source(SourceSnapshot{path:PathBuf::from("/work/big.prompt.md"),source_record:"/work/big.prompt.md".to_owned(),bytes:body(count).into_bytes(),permissions:SourcePermissions::default(),is_regular:true,is_directory:false,is_draft:false},KnownEntryKind::Prompt,ReviewDefaults::default())}
fn key(code:KeyCode,modifiers:KeyModifiers)->Event{Event::Key(KeyEvent::new(code,modifiers))}
fn mouse(x:u16,y:u16)->Event{Event::Mouse(MouseEvent{kind:MouseEventKind::Down(MouseButton::Left),column:x,row:y,modifiers:KeyModifiers::NONE})}
fn render(session:&mut PromptCandidatePickerSession,width:u16,height:u16)->(Terminal<TestBackend>,skit_tui::screens::picker::ChoicePickerGeometry){let mut terminal=Terminal::new(TestBackend::new(width,height)).unwrap();let mut geometry=Default::default();terminal.draw(|frame|geometry=render_prompt_candidate_picker(frame,Rect::new(0,0,width,height),session,Locale::En)).unwrap();(terminal,geometry)}

#[test]
fn test_review_candidate_picker_keyboard_reaches_a_hidden_name(){
 let names=(0..FROZEN_AUTO_MANAGE_LIMIT+4).map(|i|format!("h{i}")).collect::<Vec<_>>();let mut review=review(names.len());assert!(review.prompt_is_flooded());assert_eq!(review.prompt_preview().len(),FROZEN_LIST_PREVIEW_LIMIT);assert!(review.selected_prompt_names().is_empty());
 let mut picker=PromptCandidatePickerSession::new(review.prompt_picker());let(_,geometry)=render(&mut picker,80,28);assert_eq!(picker.visible_names().len(),names.len());
 picker.handle_event(key(KeyCode::End,KeyModifiers::NONE),&geometry);picker.handle_event(key(KeyCode::Char(' '),KeyModifiers::NONE),&geometry);let accepted=picker.handle_event(key(KeyCode::Char('s'),KeyModifiers::CONTROL),&geometry);let Some(PromptCandidatePickerEvent::Accepted(selected))=accepted else{panic!("hidden-name picker did not accept")};assert_eq!(selected,[names.last().unwrap().clone()]);review.set_prompt_selection(&selected);assert_eq!(review.create_entry().unwrap().settings.params,selected);
}

#[test]
fn test_review_candidate_picker_select_all_and_done_are_mouse_operable(){
 let names=(0..FROZEN_AUTO_MANAGE_LIMIT+1).map(|i|format!("h{i}")).collect::<Vec<_>>();let review=review(names.len());let mut picker=PromptCandidatePickerSession::new(review.prompt_picker());let(_,geometry)=render(&mut picker,90,30);
 let select_all=geometry.hits.iter().find(|hit|matches!(hit.target,ChoicePickerHit::SelectAll)).expect("select-all mouse hit").area;assert_eq!(picker.handle_event(mouse(select_all.x,select_all.y),&geometry),Some(PromptCandidatePickerEvent::Changed));
 let done=geometry.hits.iter().find(|hit|matches!(hit.target,ChoicePickerHit::Done)).expect("Done mouse hit").area;let accepted=picker.handle_event(mouse(done.x,done.y),&geometry);let Some(PromptCandidatePickerEvent::Accepted(selected))=accepted else{panic!("Done mouse hit did not accept")};assert_eq!(selected,names);
}

#[test]
fn test_review_candidate_picker_keeps_search_and_footer_usable_on_tiny_screen(){
 let names=(0..FROZEN_AUTO_MANAGE_LIMIT+1).map(|i|format!("h{i}")).collect::<Vec<_>>();let review=review(names.len());let mut picker=PromptCandidatePickerSession::new(review.prompt_picker());let(_,geometry)=render(&mut picker,42,8);
 assert!(geometry.search.height>=1,"tiny picker lost search row");assert!(geometry.hits.iter().any(|hit|matches!(hit.target,ChoicePickerHit::Done)),"tiny picker lost Done footer hit");assert!(geometry.hits.iter().any(|hit|matches!(hit.target,ChoicePickerHit::Cancel)),"tiny picker lost Cancel footer hit");
 assert_eq!(picker.handle_event(Event::Paste(names.last().unwrap().clone()),&geometry),Some(PromptCandidatePickerEvent::Changed));assert_eq!(picker.visible_names(),[names.last().unwrap().as_str()]);
}

#[test]
fn test_review_candidate_picker_empty_search_and_cancel_are_keyboard_operable(){
 let review=review(FROZEN_AUTO_MANAGE_LIMIT+1);let original=review.selected_prompt_names();assert!(original.is_empty());let mut picker=PromptCandidatePickerSession::new(review.prompt_picker());let(_,geometry)=render(&mut picker,80,24);
 assert_eq!(picker.handle_event(Event::Paste("zzzz".to_owned()),&geometry),Some(PromptCandidatePickerEvent::Changed));assert!(picker.visible_names().is_empty());assert_eq!(picker.handle_event(key(KeyCode::Enter,KeyModifiers::NONE),&geometry),Some(PromptCandidatePickerEvent::Changed));assert_eq!(picker.handle_event(key(KeyCode::Esc,KeyModifiers::NONE),&geometry),Some(PromptCandidatePickerEvent::Cancelled));assert_eq!(review.selected_prompt_names(),original,"cancelled empty-search picker changed the underlying review selection");
}

#[test]
fn test_review_candidate_picker_tolerates_preview_recompose(){
 let names=(0..FROZEN_AUTO_MANAGE_LIMIT+1).map(|i|format!("h{i}")).collect::<Vec<_>>();let mut review=review(names.len());assert_eq!(review.prompt_preview().len(),FROZEN_LIST_PREVIEW_LIMIT);let mut picker=PromptCandidatePickerSession::new(review.prompt_picker());let(_,geometry)=render(&mut picker,90,28);
 // Recompose/change the capped inline preview state behind the modal. The picker owns a complete
 // snapshot and must remain able to publish all names without touching preview widget identities.
 review.set_prompt_selected("h0",true);review.set_prompt_selected("h1",true);
 let select_all=geometry.hits.iter().find(|hit|matches!(hit.target,ChoicePickerHit::SelectAll)).unwrap().area;picker.handle_event(mouse(select_all.x,select_all.y),&geometry);let accepted=picker.handle_event(key(KeyCode::Char('s'),KeyModifiers::CONTROL),&geometry);let Some(PromptCandidatePickerEvent::Accepted(selected))=accepted else{panic!("recomposed picker did not accept")};assert_eq!(selected,names);review.set_prompt_selection(&selected);assert_eq!(review.selected_prompt_names(),names);
}
