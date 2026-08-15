use std::fs;

use skit_store::{FileConfigStore,PromptRunner};
use tempfile::TempDir;

fn store(root:&TempDir)->FileConfigStore{FileConfigStore::new(root.path())}
fn path(root:&TempDir)->std::path::PathBuf{root.path().join("config.toml")}
fn write(root:&TempDir,text:&str){fs::create_dir_all(root.path()).unwrap();fs::write(path(root),text).unwrap();}
fn text(root:&TempDir)->String{fs::read_to_string(path(root)).unwrap_or_default()}
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
 assert!(config.replace_runner_row_if_unchanged(runner("valuable",&["valuable-agent","--model","x","{{prompt}}"]),&expected).unwrap());let raw=text(&root);assert!(raw.contains("name = \"valuable\""));assert!(raw.contains("valuable-agent"));assert!(raw.contains("untouched"));
}

#[test]
fn test_exact_row_repair_refuses_a_stale_snapshot_or_colliding_new_name(){
 let root=TempDir::new().unwrap();write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ argv = [\"valuable\", \"{{prompt}}\"] }, { name = \"taken\", argv = [\"taken\", \"{{prompt}}\"] }]\n");let config=store(&root);let expected=config.runner_rows().unwrap().remove(0);
 write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ argv = [\"valuable\", \"{{prompt}}\"], future = true }, { name = \"taken\", argv = [\"taken\", \"{{prompt}}\"] }]\n");assert!(!config.replace_runner_row_if_unchanged(runner("fresh",&["valuable","{{prompt}}"]),&expected).unwrap());
 let expected=config.runner_rows().unwrap().remove(0);assert!(config.replace_runner_row_if_unchanged(runner("taken",&["valuable","{{prompt}}"]),&expected).is_err());
}

#[test]
fn test_runner_remove_helpers_report_absent_targets_and_bad_shapes_without_writing(){
 let root=TempDir::new().unwrap();write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"kept\", argv = [\"kept\", \"{{prompt}}\"] }]\n");let config=store(&root);let before=text(&root);assert!(!config.remove_runner("ghost").unwrap());assert!(!config.remove_runner_row(99).unwrap());assert_eq!(text(&root),before);
 write(&root,"prompt = \"scalar\"\n");assert!(!config.remove_runner_row(0).unwrap());
 write(&root,"[prompt]\nrunners = \"before\"\n");let expected=config.runner_rows().unwrap().remove(0);write(&root,"[prompt]\nrunners = \"after\"\n");assert!(!config.remove_runner_row_if_unchanged(&expected).unwrap());assert!(text(&root).contains("runners = \"after\""));
}

#[test]
fn test_name_remove_snapshot_checks_only_target_key(){
 let root=TempDir::new().unwrap();write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"victim\", argv = [\"old\", \"{{prompt}}\"] }, { name = \"other\", argv = [\"other\", \"{{prompt}}\"] }]\n");let config=store(&root);let expected=config.runner_rows().unwrap().into_iter().filter(|r|r.name.as_deref()==Some("victim")).collect::<Vec<_>>();
 write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"unrelated\", argv = [\"unrelated\", \"{{prompt}}\"] }, { name = \"victim\", argv = [\"old\", \"{{prompt}}\"] }, { name = \"other\", argv = [\"other\", \"{{prompt}}\"] }]\n");assert!(config.remove_runner_if_unchanged("victim",&expected).unwrap());assert_eq!(config.runners().unwrap(),[runner("unrelated",&["unrelated","{{prompt}}"]),runner("other",&["other","{{prompt}}"])]);
}

#[test]
fn test_save_prompt_runners_preserves_other_keys(){
 let root=TempDir::new().unwrap();write(&root,"editor = \"vi\"\n[prompt]\nother = 1\nrunners_seeded = true\nrunners = []\n");let config=store(&root);config.set_runner(runner("x",&["x","{{prompt}}"]),false).unwrap();let raw=text(&root);assert!(raw.contains("editor = \"vi\""));assert!(raw.contains("other = 1"));assert!(raw.contains("runners_seeded = true"));assert_eq!(config.runners().unwrap(),[runner("x",&["x","{{prompt}}"])]);
}
