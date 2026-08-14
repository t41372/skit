use assert_cmd::Command;
use skit_store::{FileConfigStore, MirrorSettings};
use tempfile::TempDir;

pub struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir}
impl Sandbox{
 pub fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()}}
 pub fn command(&self)->Command{let mut c=assert_cmd::cargo::cargo_bin_cmd!("skit");c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path());c}
 pub fn run(&self,args:&[&str])->std::process::Output{self.command().args(args).output().unwrap()}
 pub fn mirror(&self)->MirrorSettings{FileConfigStore::new(self.config.path()).mirror().unwrap()}
 pub fn path(&self)->&std::path::Path{self.config.path()}
}
pub fn text(o:&std::process::Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}
