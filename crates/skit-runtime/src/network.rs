//! Reachability probing for the first-run mirror offer.

use std::{
    net::{TcpStream, ToSocketAddrs as _},
    time::Duration,
};

/// The hosts the first-run offer probes, in order.
///
/// Version 0.4 probes the package index first and the release host second
/// (`src/skit/config.py:491`).
pub const REACHABILITY_HOSTS: &[&str] = &["pypi.org", "github.com"];

/// The port and per-host budget version 0.4 uses (`src/skit/config.py:487` and `:493`).
pub const REACHABILITY_PORT: u16 = 443;
/// Time allowed for one host.
pub const REACHABILITY_TIMEOUT: Duration = Duration::from_millis(2_500);

/// Ask whether one host answers on one port.
///
/// This is a port so the first-run decision is testable without a network.
pub trait NetworkProbe: std::fmt::Debug {
    /// Return true when a connection completes inside `timeout`.
    fn can_connect(&self, host: &str, port: u16, timeout: Duration) -> bool;
}

/// Probe the real network.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemNetworkProbe;

impl NetworkProbe for SystemNetworkProbe {
    fn can_connect(&self, host: &str, port: u16, timeout: Duration) -> bool {
        // Name resolution is part of the reachability question: a blocked resolver is a blocked
        // network for this purpose, exactly as a refused connection is.
        let Ok(addresses) = (host, port).to_socket_addrs() else {
            return false;
        };
        addresses.into_iter().any(|address| {
            TcpStream::connect_timeout(&address, timeout).is_ok_and(|stream| {
                drop(stream);
                true
            })
        })
    }
}

/// Report whether the network to the package hosts looks slow or blocked.
///
/// Version 0.4 answers true as soon as one host fails to answer inside the budget, and it only
/// ever *offers* mirror setup; it never decides anything on its own (`src/skit/config.py:487-497`).
pub fn network_looks_blocked(probe: &dyn NetworkProbe) -> bool {
    !REACHABILITY_HOSTS
        .iter()
        .all(|host| probe.can_connect(host, REACHABILITY_PORT, REACHABILITY_TIMEOUT))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{NetworkProbe, REACHABILITY_HOSTS, network_looks_blocked};

    #[derive(Debug)]
    struct ScriptedProbe {
        reachable: Vec<&'static str>,
        asked: RefCell<Vec<String>>,
    }

    impl NetworkProbe for ScriptedProbe {
        fn can_connect(&self, host: &str, port: u16, timeout: std::time::Duration) -> bool {
            assert_eq!(port, 443);
            assert_eq!(timeout, super::REACHABILITY_TIMEOUT);
            self.asked.borrow_mut().push(host.to_owned());
            self.reachable.contains(&host)
        }
    }

    fn probe(reachable: Vec<&'static str>) -> ScriptedProbe {
        ScriptedProbe {
            reachable,
            asked: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn both_hosts_must_answer_before_the_network_looks_open() {
        assert!(!network_looks_blocked(&probe(vec![
            "pypi.org",
            "github.com"
        ])));
        assert!(network_looks_blocked(&probe(vec!["pypi.org"])));
        assert!(network_looks_blocked(&probe(vec!["github.com"])));
        assert!(network_looks_blocked(&probe(Vec::new())));
    }

    /// A refused first host answers the question, so the second is never probed.
    #[test]
    fn the_probe_stops_at_the_first_unreachable_host() {
        let scripted = probe(vec!["github.com"]);
        assert!(network_looks_blocked(&scripted));
        assert_eq!(scripted.asked.borrow().as_slice(), ["pypi.org"]);

        let scripted = probe(vec!["pypi.org", "github.com"]);
        assert!(!network_looks_blocked(&scripted));
        assert_eq!(scripted.asked.borrow().as_slice(), REACHABILITY_HOSTS);
    }
}
