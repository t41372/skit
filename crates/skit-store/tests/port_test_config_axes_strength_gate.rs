use std::collections::BTreeSet;
use syn::{Attribute,Item};
fn is_test(a:&[Attribute])->bool{a.iter().any(|x|x.path().is_ident("test"))}
#[test]
fn config_axis_port_may_only_use_frozen_test_names(){
 let file=syn::parse_file(include_str!("port_test_config_mirror_axes.rs")).unwrap();
 let actual=file.items.into_iter().filter_map(|i|match i{Item::Fn(f) if is_test(&f.attrs)&&f.sig.ident.to_string().starts_with("test_")=>Some(f.sig.ident.to_string()),_=>None}).collect::<BTreeSet<_>>();
 let allowed=["test_axes_are_independent","test_github_release_urls_expand_from_one_base"].into_iter().map(str::to_owned).collect::<BTreeSet<_>>();
 assert!(actual.is_subset(&allowed),"self-invented parity-shaped names must be renamed rust_additive_*: {:?}",actual.difference(&allowed).collect::<Vec<_>>());
}
