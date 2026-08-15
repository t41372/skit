use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

use skit_i18n::Locale;
use skit_store::{FileConfigStore, PromptRunner};
use tempfile::TempDir;

fn store(root:&TempDir)->FileConfigStore { FileConfigStore::new(root.path()) }
fn path(root:&TempDir)->std::path::PathBuf { root.path().join("config.toml") }
fn write(root:&TempDir,text:&str){fs::create_dir_all(root.path()).unwrap();fs::write(path(root),text).unwrap();}
fn text(root:&TempDir)->String{fs::read_to_string(path(root)).unwrap_or_default()}
fn runner(name:&str,argv:&[&str])->PromptRunner{PromptRunner{name:name.to_owned(),argv:argv.iter().map(|v|(*v).to_owned()).collect()}}
fn names(rows:&[PromptRunner])->Vec<&str>{rows.iter().map(|r|r.name.as_str()).collect()}

#[test]
fn test_load_prompt_runners_is_read_only_before_seeding() {
    let root=TempDir::new().unwrap(); let config=store(&root);
    assert!(!path(&root).exists());
    let rows=config.runner_rows().unwrap();
    assert_eq!(rows.iter().filter_map(|row|row.name.as_deref()).collect::<Vec<_>>(),["claude","codex","opencode","amp","antigravity","copilot","cursor","pi"]);
    assert!(rows.iter().all(|row|row.reason.is_none()));
    let runners=config.runners().unwrap();
    assert_eq!(names(&runners),["claude","codex","opencode","amp","antigravity","copilot","cursor","pi"]);
    assert_eq!(runners.iter().find(|r|r.name=="antigravity").unwrap().argv,["agy","--prompt-interactive","{{prompt}}"]);
    assert_eq!(runners.iter().find(|r|r.name=="opencode").unwrap().argv,["opencode","--prompt={{prompt}}"]);
    assert_eq!(runners.iter().find(|r|r.name=="copilot").unwrap().argv,["copilot","--interactive={{prompt}}"]);
    assert_eq!(runners.iter().find(|r|r.name=="cursor").unwrap().argv,["cursor-agent","--","agent","{{prompt}}"]);
    assert_eq!(runners.iter().find(|r|r.name=="pi").unwrap().argv,["pi","{{prompt}}"]);
    assert!(!path(&root).exists(),"read-only runner discovery materialized config");
}

#[test]
fn test_ensure_seeded_materializes_once_and_empty_stays_empty() {
    let root=TempDir::new().unwrap(); let config=store(&root);
    config.ensure_runners_seeded().unwrap();
    assert!(text(&root).contains("runners_seeded = true"));
    for name in ["claude","codex","opencode","amp","antigravity","copilot","cursor","pi"] { assert!(names(&config.runners().unwrap()).contains(&name)); }
    for name in ["claude","codex","opencode","amp","antigravity","copilot","cursor","pi"] { assert!(config.remove_runner(name).unwrap()); }
    assert!(config.runners().unwrap().is_empty());
    config.ensure_runners_seeded().unwrap();
    assert!(config.runners().unwrap().is_empty(),"explicitly empty runner list resurrected built-in seeds");
}

#[test]
fn test_marker_alone_counts_as_seeded_and_stays_empty() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\n"); let config=store(&root);
    assert!(config.runners().unwrap().is_empty());
    config.ensure_runners_seeded().unwrap();
    assert!(config.runners().unwrap().is_empty());
}

#[test]
fn test_hand_authored_rows_without_marker_count_as_seeded() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners = [{ name = \"mine\", argv = [\"m\", \"{{prompt}}\"] }]\n"); let config=store(&root);
    assert_eq!(config.runners().unwrap(),[runner("mine",&["m","{{prompt}}"])]);
    config.ensure_runners_seeded().unwrap();
    assert_eq!(config.runners().unwrap(),[runner("mine",&["m","{{prompt}}"])]);
}

#[test]
fn test_malformed_runner_rows_are_skipped_and_reported() {
    let root=TempDir::new().unwrap(); write(&root,concat!("[prompt]\nrunners_seeded = true\nrunners = [", "{ name = \"good\", argv = [\"g\", \"{{prompt}}\"] },", "{ name = \"bad-no-slot\", argv = [\"g\"] },", "{ name = \"\", argv = [\"g\", \"{{prompt}}\"] },", "{ name = \"bad-argv\", argv = \"not-a-list\" },", "{ name = \"bad-token-type\", argv = [\"g\", 3] },", "\"not-a-table\"", "]\n"));
    let config=store(&root); assert_eq!(config.runners().unwrap(),[runner("good",&["g","{{prompt}}"])]);
    let rows=config.runner_rows().unwrap(); assert_eq!(rows[2].name.as_deref(),Some("")); assert_eq!(rows[2].argv.as_deref(),Some(&["g".to_owned(),"{{prompt}}".to_owned()][..])); assert!(rows[2].descriptor.starts_with('{'));
    let invalid=config.invalid_runner_rows().unwrap(); assert!(invalid.iter().any(|v|v=="bad-no-slot")); assert_eq!(invalid.len(),5);
}

#[test]
fn test_duplicate_normalized_runner_names_keep_first_and_are_reported() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"same\", argv = [\"first\", \"{{prompt}}\"] }, { name = \" same \", argv = [\"second\", \"{{prompt}}\"] }]\n"); let config=store(&root);
    assert_eq!(config.runners().unwrap(),[runner("same",&["first","{{prompt}}"])]);
    assert_eq!(config.invalid_runner_rows().unwrap(),["same"]);
}

#[test]
fn test_runners_section_of_wrong_type_degrades() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\nrunners = \"garbage\"\n"); let config=store(&root);
    assert!(config.runners().unwrap().is_empty());
    assert_eq!(config.invalid_runner_rows().unwrap(),["prompt.runners"]);

    // Frozen Python only performs the explicit seeding read after switching to a malformed
    // top-level prompt value. It must be a non-mutating no-op, not a new error contract.
    write(&root,"prompt = \"not-a-table\"\n");
    assert!(config.runners().unwrap().is_empty());
    assert_eq!(config.invalid_runner_rows().unwrap(),["prompt"]);
    let before=text(&root);
    config.ensure_runners_seeded().expect("frozen management read tolerates a malformed prompt container");
    assert_eq!(text(&root),before,"opening runner management rewrote a malformed prompt value");
}

#[test]
fn test_runner_container_rows_have_localized_human_recovery_reason() {
    for (doc,reason,needle) in [
        ("prompt = \"bad\"\n","prompt-section-not-table","isn't a table"),
        ("[prompt]\nrunners = \"bad\"\n","runners-not-list","isn't a list"),
    ] {
        let root=TempDir::new().unwrap();
        write(&root,doc);
        let row=store(&root).runner_rows().unwrap().remove(0);
        assert_eq!(row.reason.as_deref(),Some(reason));
        let shown=row.localized_reason(Locale::En).unwrap();
        assert!(shown.contains(needle),"frozen human recovery wording drifted: {shown}");
    }
}

#[test]
fn test_targeted_runner_mutations_preserve_unrelated_malformed_rows() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"typo\", argv = [\"mycli\", \"{{promt}}\"], future = 7 }, \"not-a-table\"]\n"); let config=store(&root);
    config.set_runner(runner("good",&["good","{{prompt}}"]),false).unwrap(); let after=text(&root); assert!(after.contains("future = 7")); assert!(after.contains("not-a-table")); assert!(after.contains("name = \"good\""));
    assert!(config.remove_runner("good").unwrap()); let after=text(&root); assert!(after.contains("future = 7")); assert!(after.contains("not-a-table")); assert!(!after.contains("name = \"good\""));
}

#[test]
fn test_explicit_runner_replace_repairs_same_name_malformed_rows_only() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \" typo \", argv = [\"old\"] }, { name = \"other\", argv = [\"other\"] }, { name = \"typo\", argv = \"also-bad\" }]\n"); let config=store(&root); let replacement=runner("typo",&["fixed","{{prompt}}"]);
    assert!(config.set_runner(replacement.clone(),false).is_err()); assert_eq!(config.set_runner(replacement.clone(),true).unwrap(),true); assert_eq!(config.runners().unwrap(),[replacement]); let raw=text(&root); assert!(raw.contains("name = \"other\"")); assert!(raw.contains("argv = [\"other\"]"));
}

#[test]
fn test_tui_targeted_row_removal_can_recover_bad_containers() {
    let root=TempDir::new().unwrap(); write(&root,"language = \"zh-TW\"\n[prompt]\nrunners = \"garbage\"\nother = 1\n"); let config=store(&root); let row=config.runner_rows().unwrap().remove(0); assert!(row.index.is_none()); assert!(config.remove_runner_row_if_unchanged(&row).unwrap()); let raw=text(&root); assert!(raw.contains("language = \"zh-TW\"")); assert!(raw.contains("other = 1")); assert!(raw.contains("runners_seeded = true")); assert!(raw.contains("runners = []"));
    write(&root,"language = \"zh-TW\"\nprompt = \"not-a-table\"\n"); let row=config.runner_rows().unwrap().remove(0); assert!(config.remove_runner_row_if_unchanged(&row).unwrap()); let raw=text(&root); assert!(raw.contains("language = \"zh-TW\"")); assert!(raw.contains("runners_seeded = true")); assert!(raw.contains("runners = []"));
}

#[test]
fn test_raw_row_remove_snapshot_includes_unknown_fields_and_container_value() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"bad\", argv = [\"bad\"], future = 1 }]\n"); let config=store(&root); let expected=config.runner_rows().unwrap().remove(0); write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"bad\", argv = [\"bad\"], future = 2 }]\n"); assert!(!config.remove_runner_row_if_unchanged(&expected).unwrap()); assert!(text(&root).contains("future = 2"));
    write(&root,"prompt = \"before\"\n"); let expected=config.runner_rows().unwrap().remove(0); write(&root,"prompt = \"after\"\n"); assert!(!config.remove_runner_row_if_unchanged(&expected).unwrap()); assert!(text(&root).contains("prompt = \"after\""));
}

#[test]
fn test_runner_raw_snapshots_are_recursively_type_sensitive() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"bad\", argv = [\"bad\"], future = { nested = [1, { flag = 0 }] } }]\n"); let config=store(&root); let expected=config.runner_rows().unwrap().remove(0); write(&root,"[prompt]\nrunners_seeded = true\nrunners = [{ name = \"bad\", argv = [\"bad\"], future = { nested = [1, { flag = false }] } }]\n"); assert!(!config.remove_runner_row_if_unchanged(&expected).unwrap()); assert!(text(&root).contains("flag = false"));
}

fn locked_file(path:&Path)->File{fs::create_dir_all(path.parent().unwrap()).unwrap();let file=OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).unwrap();if file.metadata().unwrap().len()==0{file.set_len(1).unwrap();}file.lock().unwrap();file}

#[test]
fn test_runner_targeted_transactions_do_not_lose_concurrent_distinct_adds() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\nrunners = []\n"); let lock=locked_file(&root.path().join("config.lock")); let barrier=Arc::new(Barrier::new(3)); let a=store(&root); let b=store(&root); let b1=barrier.clone(); let t1=thread::spawn(move||{b1.wait();a.set_runner(runner("one",&["one","{{prompt}}"]),false).unwrap();}); let b2=barrier.clone(); let t2=thread::spawn(move||{b2.wait();b.set_runner(runner("two",&["two","{{prompt}}"]),false).unwrap();}); barrier.wait(); thread::sleep(Duration::from_millis(30)); lock.unlock().unwrap(); t1.join().unwrap(); t2.join().unwrap(); let got=store(&root).runners().unwrap(); assert_eq!(got.iter().map(|r|r.name.as_str()).collect::<std::collections::BTreeSet<_>>(),["one","two"].into_iter().collect());
}

#[test]
fn test_runner_transaction_and_non_runner_config_update_preserve_each_other() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\nrunners = []\n"); let lock=locked_file(&root.path().join("config.lock")); let barrier=Arc::new(Barrier::new(3)); let a=store(&root); let b=store(&root); let b1=barrier.clone(); let t1=thread::spawn(move||{b1.wait();a.set_runner(runner("agent",&["agent","{{prompt}}"]),false).unwrap();}); let b2=barrier.clone(); let t2=thread::spawn(move||{b2.wait();b.set("editor","code --wait").unwrap();}); barrier.wait(); thread::sleep(Duration::from_millis(30)); lock.unlock().unwrap(); t1.join().unwrap(); t2.join().unwrap(); let config=store(&root); assert_eq!(config.runners().unwrap(),[runner("agent",&["agent","{{prompt}}"])]); assert_eq!(config.get("editor").unwrap(),"code --wait");
}

#[test]
fn test_runner_transaction_and_i18n_update_share_the_neutral_config_lock() {
    let root=TempDir::new().unwrap(); write(&root,"[prompt]\nrunners_seeded = true\nrunners = []\n"); let lock=locked_file(&root.path().join("config.lock")); let barrier=Arc::new(Barrier::new(3)); let a=store(&root); let b=store(&root); let b1=barrier.clone(); let t1=thread::spawn(move||{b1.wait();a.set_runner(runner("agent",&["agent","{{prompt}}"]),false).unwrap();}); let b2=barrier.clone(); let t2=thread::spawn(move||{b2.wait();b.set("lang","zh-TW").unwrap();}); barrier.wait(); thread::sleep(Duration::from_millis(30)); lock.unlock().unwrap(); t1.join().unwrap(); t2.join().unwrap(); let config=store(&root); assert_eq!(config.runners().unwrap(),[runner("agent",&["agent","{{prompt}}"])]); assert_eq!(config.get("lang").unwrap(),"zh-TW");
}

#[test]
fn rust_additive_config_lock_probe_has_no_timeout() {
    let root=TempDir::new().unwrap(); let lock=locked_file(&root.path().join("config.lock")); let start=Instant::now(); lock.unlock().unwrap(); assert!(start.elapsed()<Duration::from_secs(1));
}
