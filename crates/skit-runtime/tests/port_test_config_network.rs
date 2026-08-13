use std::{cell::RefCell, time::Duration};
use skit_runtime::{NetworkProbe, REACHABILITY_TIMEOUT, network_looks_blocked};

#[derive(Debug)]
struct Probe { reachable: Vec<&'static str>, seen: RefCell<Vec<String>> }
impl NetworkProbe for Probe {
    fn can_connect(&self, host:&str, port:u16, timeout:Duration)->bool {
        assert_eq!(port,443); assert_eq!(timeout,REACHABILITY_TIMEOUT);
        self.seen.borrow_mut().push(host.to_owned()); self.reachable.contains(&host)
    }
}
fn probe(reachable:Vec<&'static str>)->Probe{Probe{reachable,seen:RefCell::new(Vec::new())}}

#[test]
fn test_looks_blocked_true_when_unreachable(){let p=probe(Vec::new());assert!(network_looks_blocked(&p));}
#[test]
fn test_looks_blocked_false_when_reachable(){let p=probe(vec!["pypi.org","github.com"]);assert!(!network_looks_blocked(&p));}
#[test]
fn test_looks_blocked_short_circuits_on_first_host(){let p=probe(vec!["github.com"]);assert!(network_looks_blocked(&p));assert_eq!(p.seen.borrow().as_slice(),["pypi.org"]);}
#[test]
fn test_looks_blocked_true_when_second_host_unreachable(){let p=probe(vec!["pypi.org"]);assert!(network_looks_blocked(&p));assert_eq!(p.seen.borrow().as_slice(),["pypi.org","github.com"]);}
