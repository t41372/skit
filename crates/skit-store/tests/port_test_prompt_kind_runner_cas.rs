use std::fs;

use skit_store::{FileConfigStore,PromptRunner};
use tempfile::TempDir;
use toml::Value;

fn store(root:&TempDir)->FileConfigStore{FileConfigStore::new(root.path())}
fn path(root:&TempDir)->std::path::PathBuf{root.path().join("config.toml")}
fn write(root:&TempDir,text:&str){fs::create_dir_all(root.path()).unwrap();fs::write(path(root),text).unwrap();}
fn text(root:&TempDir)->String{fs::read_to_string(path(root)).unwrap_or_default()}
fn doc(root:&TempDir)->Value{toml::from_str(&text(root)).unwrap()}
fn runner(name:&str,argv:&[&str])->PromptRunner{PromptRunner{name:name.to_owned(),argv:argv.iter().map(|v|(*v).to_owned()).collect()}}

#[test]
fn test_runner_edit_snapshot_checks_only_the_target_key(){
 let root=TempDir::new().unwrap();write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"victim\", argv = [\"old\", \"{{prompt}}\"] }, { name = \"other\", argv = [\"other\", \"{{prompt}}\"] }]\n");let config=store(&root);
 let expected=config.runner_rows().unwrap().into_iter().filter(|r|r.name.as_deref()==Some("victim")).collect::<Vec<_>>();
 write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"victim\", argv = [\"old\", \"{{prompt}}\"] }, { name = \"other\", argv = [\"unrelated\", \"{{prompt}}\"] }]\n");
 assert!(config.set_runner_if_unchanged(runner("victim",&["mine","{{prompt}}"]),&expected).unwrap());
 assert_eq!(config.runners().unwrap(),[runner("victim",&["mine","{{prompt}}"]),runner("other",&["unrelated","{{prompt}}"])]);
 let expected=config.runner_rows().unwrap().into_iter().filter(|r|r.name.as_deref()==Some("victim")).collect::<Vec<_>>();config.set_runner(runner("victim",&["external","{{prompt}}"]),true).unwrap();
 assert!(!config.set_runner_if_unchanged(runner("victim",&["old","{{prompt}}"]),&expected).unwrap());assert_eq!(config.runners().unwrap()[0],runner("victim",&["external","{{prompt}}"]));
}

#[test]
fn test_exact_row_repair_can_name_a_recognizable_anonymous_command(){
 let root=TempDir::new().unwrap();write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ argv = [\"valuable-agent\", \"--model\", \"x\", \"{{prompt}}\"] }, \"untouched\"]\n");let config=store(&root);let expected=config.runner_rows().unwrap().remove(0);
 assert!(config.replace_runner_row_if_unchanged(runner("valuable",&["valuable-agent","--model","x","{{prompt}}"]),&expected).unwrap());
 let document=doc(&root);let rows=document["prompt"]["runners"].as_array().unwrap();assert_eq!(rows.len(),2);
 assert_eq!(rows[0]["name"].as_str(),Some("valuable"));assert_eq!(rows[0]["argv"].as_array().unwrap().iter().map(|v|v.as_str().unwrap()).collect::<Vec<_>>(),["valuable-agent","--model","x","{{prompt}}"]);assert_eq!(rows[1].as_str(),Some("untouched"));
}

#[test]
fn test_exact_row_repair_refuses_a_stale_snapshot_or_colliding_new_name(){
 let root=TempDir::new().unwrap();write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ argv = [\"valuable\", \"{{prompt}}\"] }, { name = \"taken\", argv = [\"taken\", \"{{prompt}}\"] }]\n");let config=store(&root);let expected=config.runner_rows().unwrap().remove(0);
 write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ argv = [\"valuable\", \"{{prompt}}\"], future = true }, { name = \"taken\", argv = [\"taken\", \"{{prompt}}\"] }]\n");assert!(!config.replace_runner_row_if_unchanged(runner("fresh",&["valuable","{{prompt}}"]),&expected).unwrap());let current=doc(&root);assert_eq!(current["prompt"]["runners"][0]["future"].as_bool(),Some(true));
 let expected=config.runner_rows().unwrap().remove(0);assert!(config.replace_runner_row_if_unchanged(runner("taken",&["valuable","{{prompt}}"]),&expected).is_err());let current=doc(&root);assert_eq!(current["prompt"]["runners"].as_array().unwrap().len(),2);assert_eq!(current["prompt"]["runners"][1]["name"].as_str(),Some("taken"));
}

#[test]
fn test_runner_remove_helpers_report_absent_targets_and_bad_shapes_without_writing(){
 let root=TempDir::new().unwrap();write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"kept\", argv = [\"kept\", \"{{prompt}}\"] }]\n");let config=store(&root);let before=text(&root);assert!(!config.remove_runner("ghost").unwrap());assert!(!config.remove_runner_row(99).unwrap());assert_eq!(text(&root),before);
 write(&root,"prompt = \"scalar\"\n");assert!(!config.remove_runner_row(0).unwrap());
 write(&root,"[prompt]\nrunners = \"before\"\n");let expected=config.runner_rows().unwrap().remove(0);write(&root,"[prompt]\nrunners = \"after\"\n");assert!(!config.remove_runner_row_if_unchanged(&expected).unwrap());assert_eq!(doc(&root)["prompt"]["runners"].as_str(),Some("after"));
}

#[test]
fn test_name_remove_snapshot_checks_only_target_key(){
 let root=TempDir::new().unwrap();write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"victim\", argv = [\"old\", \"{{prompt}}\"] }, { name = \"other\", argv = [\"other\", \"{{prompt}}\"] }]\n");let config=store(&root);let expected=config.runner_rows().unwrap().into_iter().filter(|r|r.name.as_deref()==Some("victim")).collect::<Vec<_>>();
 write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"unrelated\", argv = [\"unrelated\", \"{{prompt}}\"] }, { name = \"victim\", argv = [\"old\", \"{{prompt}}\"] }, { name = \"other\", argv = [\"other\", \"{{prompt}}\"] }]\n");assert!(config.remove_runner_if_unchanged("victim",&expected).unwrap());assert_eq!(config.runners().unwrap(),[runner("unrelated",&["unrelated","{{prompt}}"]),runner("other",&["other","{{prompt}}"])]);
}

#[test]
fn test_save_prompt_runners_preserves_other_keys(){
 let root=TempDir::new().unwrap();write(&root,"editor = \"vi\"\n[prompt]\nother = 1\nrunners_seeded = true\nrunners = []\n");let config=store(&root);config.set_runner(runner("x",&["x","{{prompt}}"]),false).unwrap();let current=doc(&root);assert_eq!(current["editor"].as_str(),Some("vi"));assert_eq!(current["prompt"]["other"].as_integer(),Some(1));assert_eq!(current["prompt"]["runners_seeded"].as_bool(),Some(true));assert_eq!(config.runners().unwrap(),[runner("x",&["x","{{prompt}}"])]);assert!(!config.runners().unwrap().iter().any(|r|r.name=="ghost"));
}

#[test]
fn test_runner_raw_snapshots_are_recursively_type_sensitive(){
 let root=TempDir::new().unwrap();
 write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"bad\", argv = [\"bad\"], future = { nested = [1, { flag = 0 }] } }]\n");let config=store(&root);let expected=config.runner_rows().unwrap().remove(0);
 write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"bad\", argv = [\"bad\"], future = { nested = [true, { flag = false }] } }]\n");assert!(!config.remove_runner_row_if_unchanged(&expected).unwrap());let current=doc(&root);let nested=current["prompt"]["runners"][0]["future"]["nested"].as_array().unwrap();assert_eq!(nested[0].as_bool(),Some(true));assert_eq!(nested[1]["flag"].as_bool(),Some(false));
 write(&root,"prompt = 1\n");let expected=config.runner_rows().unwrap().remove(0);write(&root,"prompt = true\n");assert!(!config.remove_runner_row_if_unchanged(&expected).unwrap());assert_eq!(doc(&root)["prompt"].as_bool(),Some(true));
}
