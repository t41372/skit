use std::{fs, path::PathBuf, process::{Command, Output}};

use tempfile::TempDir;

struct Sandbox { data: TempDir, state: TempDir, config: TempDir, home: TempDir }
impl Sandbox {
    fn new() -> Self { Self { data: TempDir::new().unwrap(), state: TempDir::new().unwrap(), config: TempDir::new().unwrap(), home: TempDir::new().unwrap() } }
    fn configure(&self, c: &mut Command) {
        c.env("SKIT_DATA_DIR", self.data.path()).env("SKIT_STATE_DIR", self.state.path()).env("SKIT_CONFIG_DIR", self.config.path()).env("SKIT_LANG", "en").env("HOME", self.home.path()).env("USERPROFILE", self.home.path()).current_dir(self.home.path());
    }
    fn run(&self, args: &[&str]) -> Output { let mut c=Command::new(env!("CARGO_BIN_EXE_skit")); self.configure(&mut c); c.args(args).output().unwrap() }
    fn source(&self) -> PathBuf { let p=self.home.path().join("p.prompt.md"); fs::write(&p,"Do {{a}}\n").unwrap(); p }
    fn add(&self) {
        let p=self.source(); let out=self.run(&["add",p.to_str().unwrap(),"--name","p","--no-input"]); assert_eq!(out.status.code(),Some(0),"{}",text(&out));
        let seeded=self.run(&["runner","list"]); assert_eq!(seeded.status.code(),Some(0),"{}",text(&seeded));
    }
    fn meta(&self)->PathBuf { self.data.path().join("scripts/p/meta.toml") }
    fn entry_dir(&self)->PathBuf { self.data.path().join("scripts/p") }
}
fn text(o:&Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}

#[cfg(unix)]
fn make_write_fail(s:&Sandbox)->Option<(u32,u32)> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(s.entry_dir(),fs::Permissions::from_mode(0o555)).unwrap();
    let id=Command::new("id").arg("-u").output().ok().and_then(|o|String::from_utf8(o.stdout).ok()).and_then(|v|v.trim().parse::<u32>().ok()).unwrap();
    if id==0 {
        for root in [s.data.path(),s.state.path(),s.config.path(),s.home.path(),s.data.path().join("scripts").as_path()] { fs::set_permissions(root,fs::Permissions::from_mode(0o777)).unwrap(); }
        Some((65_534,65_534))
    } else { None }
}

fn failing_run(s:&Sandbox,args:&[&str])->Output {
    #[cfg(unix)] {
        use std::os::unix::process::CommandExt as _;
        let drop=make_write_fail(s); let mut c=Command::new(env!("CARGO_BIN_EXE_skit")); s.configure(&mut c); c.args(args); if let Some((uid,gid))=drop { c.uid(uid).gid(gid); } c.output().unwrap()
    }
    #[cfg(not(unix))] {
        // Force a genuine repository adapter failure without a fake exception: replace the entry
        // directory with a file so the metadata operation cannot descend to meta.toml.
        let saved=s.data.path().join("scripts/p.saved"); fs::rename(s.entry_dir(),&saved).unwrap(); fs::write(s.entry_dir(),b"not a directory").unwrap(); s.run(args)
    }
}

#[test]
fn test_params_runner_pin_reports_store_errors() {
    let s=Sandbox::new(); s.add(); let before=fs::read(s.meta()).unwrap();
    let out=failing_run(&s,&["params","p","--runner","claude"]);
    assert_eq!(out.status.code(),Some(1),"{}",text(&out));
    let shown=text(&out); assert!(!shown.trim().is_empty(),"repository failure was swallowed");
    #[cfg(unix)] assert!(shown.to_ascii_lowercase().contains("permission denied"),"{shown}");
    #[cfg(unix)] assert_eq!(fs::read(s.meta()).unwrap(),before,"failed runner pin partially rewrote metadata");
}

#[test]
fn test_params_interpolate_reports_store_errors() {
    let s=Sandbox::new(); s.add(); let before=fs::read(s.meta()).unwrap();
    let out=failing_run(&s,&["params","p","--no-interpolate"]);
    assert_eq!(out.status.code(),Some(1),"{}",text(&out));
    let shown=text(&out); assert!(!shown.trim().is_empty(),"repository failure was swallowed");
    #[cfg(unix)] assert!(shown.to_ascii_lowercase().contains("permission denied"),"{shown}");
    #[cfg(unix)] assert_eq!(fs::read(s.meta()).unwrap(),before,"failed interpolation edit partially rewrote metadata");
}
