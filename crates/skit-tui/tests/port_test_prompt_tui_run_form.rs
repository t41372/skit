use std::collections::BTreeMap;

use crossterm::event::{Event,KeyCode,KeyEvent,KeyModifiers,MouseButton,MouseEvent,MouseEventKind};
use ratatui::{Terminal,backend::TestBackend,buffer::Buffer};
use skit_domain::parameters::ParamDecl;
use skit_tui::{EventHandling,TuiSession,render_state};
use skit_ui::{Action,Effect,FieldValue,FormControl,FormPurpose,LibraryState,RunFormView,Screen,TypedValue};

fn form(declarations:&[ParamDecl], runners:&[&str], default:&str)->RunFormView{
    RunFormView::from_declarations(
        "p","Prompt",declarations,&BTreeMap::new(),
        &runners.iter().map(|value|(*value).to_owned()).collect::<Vec<_>>(),
        default,&BTreeMap::new(),"",
    )
}
fn state_with_form(form:RunFormView)->LibraryState{let mut state=LibraryState::default();state.update(Action::Present(Screen::Run(Box::new(form))));state}
fn draw(session:&mut TuiSession,state:&LibraryState,width:u16,height:u16)->(Terminal<TestBackend>,skit_tui::FormGeometry){let backend=TestBackend::new(width,height);let mut terminal=Terminal::new(backend).unwrap();let mut geometry=None;terminal.draw(|frame|geometry=Some(render_state(frame,state,session))).unwrap();(terminal,geometry.unwrap())}
fn key(code:KeyCode)->Event{Event::Key(KeyEvent::new(code,KeyModifiers::NONE))}
fn mouse(x:u16,y:u16)->Event{Event::Mouse(MouseEvent{kind:MouseEventKind::Down(MouseButton::Left),column:x,row:y,modifiers:KeyModifiers::NONE})}
fn drive(session:&mut TuiSession,state:&mut LibraryState,geometry:&skit_tui::FormGeometry,event:Event)->EventHandling{let handling=session.handle_event(event,state,geometry);if let EventHandling::Action(action)=handling.clone(){let _=state.update(action);}handling}
fn submitted_runner(effect:Effect)->String{let Effect::Submit{purpose:FormPurpose::Run,values,..}=effect else{panic!("run form did not submit: {effect:?}")};match values.get("_skit_runner").expect("runner field"){FieldValue::Explicit(TypedValue::Choice(value))=>value.clone(),other=>panic!("runner lost typed choice semantics: {other:?}")}}
fn buffer_text(buffer:&Buffer)->String{let mut output=String::new();for y in 0..buffer.area.height{for x in 0..buffer.area.width{output.push_str(buffer[(x,y)].symbol());}output.push('\n');}output}

#[test]
fn test_form_picker_defaults_to_the_pin_and_submits_it(){
    let mut state=state_with_form(form(&[ParamDecl::new("a")],&["claude","codex"],"codex"));
    let run=state.run_form().unwrap();assert_eq!(run.fields()[0].control.value(),"codex");
    state.update(Action::SetFieldValue{field:1,value:"1".to_owned()});
    assert_eq!(submitted_runner(state.update(Action::Submit)),"codex");
}

#[test]
fn test_form_picker_keyboard_pick_runs_and_remembers(){
    let mut state=state_with_form(form(&[ParamDecl::new("a")],&["claude","codex"],"claude"));state.update(Action::SetFieldValue{field:1,value:"1".to_owned()});
    let mut session=TuiSession::default();let(_,geometry)=draw(&mut session,&state,90,24);state.update(Action::FocusField(0));let(_,geometry)=draw(&mut session,&state,90,24);
    assert_eq!(drive(&mut session,&mut state,&geometry,key(KeyCode::Enter)),EventHandling::Consumed);
    drive(&mut session,&mut state,&geometry,key(KeyCode::Down));drive(&mut session,&mut state,&geometry,key(KeyCode::Enter));
    assert_eq!(state.run_form().unwrap().fields()[0].control.value(),"codex");assert_eq!(submitted_runner(state.update(Action::Submit)),"codex");
}

#[test]
fn test_form_picker_mouse_click_picks_a_runner(){
    let mut state=state_with_form(form(&[ParamDecl::new("a")],&["claude","codex"],"claude"));state.update(Action::FocusField(0));let mut session=TuiSession::default();let(_,geometry)=draw(&mut session,&state,90,24);
    drive(&mut session,&mut state,&geometry,key(KeyCode::Enter));let(terminal,geometry)=draw(&mut session,&state,90,24);let rendered=buffer_text(terminal.backend().buffer());assert!(rendered.contains("codex"),"runner option not rendered: {rendered}");
    // The active picker exposes option hit regions through the shared form geometry; click the last
    // runner option rather than hard-coding terminal coordinates.
    let hit=geometry.hits.iter().rev().find(|hit|matches!(hit.action,skit_tui::HitTarget::SelectOption{field:0,ref value} if value=="codex")).expect("codex mouse hit");
    drive(&mut session,&mut state,&geometry,mouse(hit.rect.x,hit.rect.y));
    assert_eq!(state.run_form().unwrap().fields()[0].control.value(),"codex");
}

#[test]
fn test_prompt_with_no_placeholders_still_shows_the_form_for_the_picker(){
    let state=state_with_form(form(&[],&["claude","codex"],"claude"));let run=state.run_form().unwrap();assert!(!run.has_parameters());assert!(run.has_runner_picker());assert!(run.fields().iter().any(|field|matches!(field.control,FormControl::Choice(_))&&field.key=="_skit_runner"));
    let mut session=TuiSession::default();let(terminal,_)=draw(&mut session,&state,90,20);let rendered=buffer_text(terminal.backend().buffer());assert!(rendered.contains("Runner")&&rendered.contains("claude"),"promptless prompt lost runner form: {rendered}");
}

#[test]
fn test_unicode_placeholder_is_a_working_tui_field(){
    let declaration=ParamDecl::new("目标");let mut state=state_with_form(form(&[declaration],&["claude"],"claude"));let index=state.run_form().unwrap().fields().iter().position(|field|field.key=="value:目标").expect("unicode field");state.update(Action::SetFieldValue{field:index,value:"src/主程式.py".to_owned()});
    let Effect::Submit{values,..}=state.update(Action::Submit) else{panic!("unicode prompt form did not submit")};assert_eq!(values.get("value:目标").unwrap().as_text(),"src/主程式.py");
}
