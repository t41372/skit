use std::{collections::BTreeMap, env, io::Write as _, path::PathBuf, process::{Command, Stdio}};
use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{LanguageError, inject_values};

pub fn spec(name:&str,binding:ParameterBinding,ty:ParameterType,order:i64,secret:bool,prompt:&str)->ParamDecl{
 let mut d=ParamDecl::new(name); d.binding=binding; d.delivery=ParameterDelivery::Inject; d.parameter_type=ty; d.order=order; d.secret=secret; d.prompt=prompt.to_owned(); d
}
pub fn constant(name:&str,ty:ParameterType)->ParamDecl{spec(name,ParameterBinding::Const,ty,-1,false,"")}
pub fn input(name:&str,order:i64,secret:bool,prompt:&str)->ParamDecl{spec(name,ParameterBinding::Input,ParameterType::Str,order,secret,prompt)}
pub fn values(pairs:&[(&str,&str)])->BTreeMap<String,String>{pairs.iter().map(|(k,v)|((*k).into(),(*v).into())).collect()}
pub fn inject(source:&str,declarations:&[ParamDecl],pairs:&[(&str,&str)])->Result<String,LanguageError>{inject_values("python",source,declarations,&values(pairs))}

fn find_python()->PathBuf{
 for name in ["python3","python"]{
  if let Some(paths)=env::var_os("PATH"){
   for dir in env::split_paths(&paths){let p=dir.join(name); if p.is_file(){return p;}}
  }
 }
 panic!("a Python runtime is required by frozen test_shim.py behavior contracts")
}

pub fn run_python(source:&str,stdin:&str)->String{
 let mut child=Command::new(find_python()).args(["-c",source]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
 if !stdin.is_empty(){child.stdin.as_mut().unwrap().write_all(stdin.as_bytes()).unwrap();}
 drop(child.stdin.take());
 let out=child.wait_with_output().unwrap();
 assert!(out.status.success(),"Python child failed: {}",String::from_utf8_lossy(&out.stderr));
 String::from_utf8(out.stdout).unwrap()
}
