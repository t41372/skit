use std::{fs, io::{Read as _, Write as _}, path::PathBuf, thread, time::Duration};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tempfile::TempDir;

#[test]
fn test_add_prompt_editor_lane_reports_store_errors() {
    let data=TempDir::new().unwrap(); let state=TempDir::new().unwrap(); let config=TempDir::new().unwrap(); let home=TempDir::new().unwrap();
    let mut seed=assert_cmd::cargo::cargo_bin_cmd!("skit");
    seed.env("SKIT_DATA_DIR",data.path()).env("SKIT_STATE_DIR",state.path()).env("SKIT_CONFIG_DIR",config.path()).env("SKIT_LANG","en").env("HOME",home.path()).env("USERPROFILE",home.path()).current_dir(home.path()).args(["add","--cmd","echo hi","--name","taken"]).assert().success();
    let mut plain=assert_cmd::cargo::cargo_bin_cmd!("skit");
    plain.env("SKIT_DATA_DIR",data.path()).env("SKIT_STATE_DIR",state.path()).env("SKIT_CONFIG_DIR",config.path()).env("SKIT_LANG","en").env("HOME",home.path()).env("USERPROFILE",home.path()).current_dir(home.path()).args(["config","form","plain"]).assert().success();

    let pair=native_pty_system().openpty(PtySize{rows:24,cols:100,pixel_width:0,pixel_height:0}).unwrap();
    let mut command=CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(["add","--prompt"]).cwd(home.path()).env("TERM","xterm-256color").env("SKIT_DATA_DIR",data.path()).env("SKIT_STATE_DIR",state.path()).env("SKIT_CONFIG_DIR",config.path()).env("SKIT_LANG","en").env("HOME",home.path()).env("USERPROFILE",home.path());
    let mut child=pair.slave.spawn_command(command).unwrap(); drop(pair.slave);
    let mut reader=pair.master.try_clone_reader().unwrap(); let drain=thread::spawn(move||{let mut b=Vec::new();reader.read_to_end(&mut b).unwrap();b});
    let mut writer=pair.master.take_writer().unwrap(); thread::sleep(Duration::from_millis(120)); let _=writer.write_all(b"\x1b[1;1R"); let _=writer.flush(); thread::sleep(Duration::from_millis(160)); writer.write_all(b"taken\r").unwrap(); writer.flush().unwrap();
    let status=child.wait().unwrap(); drop(writer); let output=String::from_utf8_lossy(&drain.join().unwrap()).replace("\r\n","\n").replace('\r',"");
    assert_eq!(status.exit_code(),1,"{output}");
    assert!(output.contains("already taken"),"{output}");
    assert!(data.path().join("scripts/taken/meta.toml").is_file(),"existing entry was damaged");
    assert_eq!(fs::read_to_string(data.path().join("scripts/taken/meta.toml")).unwrap().contains("kind = \"command\""),true);
}
