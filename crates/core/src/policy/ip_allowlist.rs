use std::net::IpAddr;

use ipnet::IpNet;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum IpAllowlistError {
    #[error("Invalid IP/CIDR rule `{0}`")]
    InvalidRule(String),
}

/// Returns `Ok(true)` if `client_ip` matches any rule in `rules` (either an
/// exact IP match or a CIDR containment). Returns `Ok(false)` if no rule
/// matches. Returns `Err` if a rule is unparseable.
///
/// An empty `rules` slice returns `Ok(false)` (no rules match).
pub fn ip_allowed(client_ip: &IpAddr, rules: &[String]) -> Result<bool, IpAllowlistError> {
    for rule in rules {
        let rule = rule.trim();
        if rule.is_empty() {
            continue;
        }

        if let Ok(net) = rule.parse::<IpNet>() {
            if net.contains(client_ip) {
                return Ok(true);
            }
            continue;
        }

        if let Ok(ip) = rule.parse::<IpAddr>() {
            if &ip == client_ip {
                return Ok(true);
            }
            continue;
        }

        return Err(IpAllowlistError::InvalidRule(rule.to_string()));
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn empty_rules_means_nothing_matches() {
        assert!(!ip_allowed(&ip("10.0.0.1"), &[]).unwrap());
    }

    #[test]
    fn matches_exact_ipv4() {
        let rules = vec!["10.0.0.5".to_string()];
        assert!(ip_allowed(&ip("10.0.0.5"), &rules).unwrap());
        assert!(!ip_allowed(&ip("10.0.0.6"), &rules).unwrap());
    }

    #[test]
    fn matches_cidr_ipv4() {
        let rules = vec!["10.0.0.0/24".to_string()];
        assert!(ip_allowed(&ip("10.0.0.5"), &rules).unwrap());
        assert!(ip_allowed(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 255)), &rules).unwrap());
        assert!(!ip_allowed(&ip("10.0.1.0"), &rules).unwrap());
    }

    #[test]
    fn matches_cidr_ipv6() {
        let rules = vec!["2001:db8::/32".to_string()];
        assert!(ip_allowed(&ip("2001:db8::1"), &rules).unwrap());
        assert!(!ip_allowed(&ip("2001:db9::1"), &rules).unwrap());
    }

    #[test]
    fn ignores_blank_entries() {
        let rules = vec!["  ".to_string(), "10.0.0.5".to_string()];
        assert!(ip_allowed(&ip("10.0.0.5"), &rules).unwrap());
    }

    #[test]
    fn rejects_garbage_rule() {
        let rules = vec!["not-an-ip".to_string()];
        assert!(matches!(
            ip_allowed(&ip("10.0.0.5"), &rules).unwrap_err(),
            IpAllowlistError::InvalidRule(_)
        ));
    }
}
