//! Which sites an agent may reach through the proxy (ADR-057), decided from the name it asks for and then
//! from the address that name turns out to be.

use std::net::IpAddr;
use torq_wire::activity::Why;

/// The one port an agent reaches a site on: HTTPS.
pub const HTTPS: u16 = 443;

/// What signing in to the engine's service, and keeping the sign-in fresh, need (Claude Code 2.1.316: the
/// address its sign-in is exchanged and renewed at; the page a person signs in on is opened in their own
/// browser, not from here).
pub const SIGN_IN: [&str; 1] = ["platform.claude.com"];

/// What the owner set.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// The sites on the owner's list (`torq agent network hosts`): a name, or `*.name` for every name under it.
    pub hosts: Vec<String>,
    /// Whether the web is open to the agents (`torq agent network web`).
    pub web: bool,
    /// The model's own service: reached through the proxy's record, and directly only where the owner lists it by name.
    pub model: String,
}

/// A site's name as it is compared: lower case, without a final dot. Nothing that is not a name or an address
/// in numbers.
pub fn site(host: &str) -> Option<String> {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    let h = h.strip_prefix('[').and_then(|x| x.strip_suffix(']')).map(String::from).unwrap_or(h);
    let named = !h.is_empty() && h.len() <= 253 && h.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    (named || h.parse::<IpAddr>().is_ok()).then_some(h)
}

/// Whether `entry` of the owner's list names `site`.
fn names(entry: &str, site: &str) -> bool {
    let entry = entry.trim_end_matches('.').to_ascii_lowercase();
    match entry.strip_prefix("*.") {
        Some(under) => site.len() > under.len() + 1 && site.ends_with(under) && site.as_bytes()[site.len() - under.len() - 1] == b'.',
        None => entry == site,
    }
}

impl Policy {
    /// Whether the owner's list names `site`.
    pub fn listed(&self, site: &str) -> bool {
        self.hosts.iter().any(|e| names(e, site))
    }

    /// Whether an agent may ask for `host` on `port`, and why. (The address the name turns out to be is
    /// judged after: `address_ok`.)
    pub fn decide(&self, host: &str, port: u16) -> (bool, Why) {
        let Some(site) = site(host) else { return (false, Why::NotAName) };
        if port != HTTPS {
            return (false, Why::NotHttps);
        }
        if self.listed(&site) {
            return (true, Why::Listed);
        }
        // Before the web's rule: an engine that lost its setting fails here, in words, and does not go on unrecorded.
        if site == self.model {
            return (false, Why::TheModel);
        }
        if SIGN_IN.contains(&site.as_str()) {
            return (true, Why::SignIn);
        }
        if self.web {
            return (true, Why::Web);
        }
        (false, Why::NotListed)
    }
}

/// Whether the proxy connects to `ip` for an agent. Never to this machine itself, by any of its names for
/// itself, nor to an address that only means something on one wire. An address of a private network (the
/// machine's own LAN: a router's page, a printer, another machine's service) only where the owner listed the
/// site by name: opening the web opens the internet, not the network the machine sits on.
pub fn address_ok(ip: IpAddr, listed: bool) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            let never = a.is_loopback() || a.is_unspecified() || a.is_multicast() || a.is_broadcast() || a.is_link_local() || o[0] == 0;
            // 10/8, 172.16/12, 192.168/16, and 100.64/10 (addresses a provider shares).
            let private = a.is_private() || (o[0] == 100 && (o[1] & 0xC0) == 64);
            !never && (listed || !private)
        }
        IpAddr::V6(a) => {
            if let Some(four) = a.to_ipv4_mapped() {
                return address_ok(IpAddr::V4(four), listed);
            }
            let s = a.segments();
            let never = a.is_loopback() || a.is_unspecified() || a.is_multicast() || (s[0] & 0xFFC0) == 0xFE80;
            // fc00::/7: addresses of a network's own.
            let private = (s[0] & 0xFE00) == 0xFC00;
            !never && (listed || !private)
        }
    }
}

/// What a refused agent is told, in plain words, naming the setting that would allow it.
pub fn refusal(why: Why, host: &str, port: u16, proxy_port: u16) -> String {
    match why {
        Why::NotListed => format!("TorqOS: your agent may not reach {host}. It is not one of the sites this machine's agents may reach. An administrator can allow it in torqcli: `set torq agent network hosts {host}`, then `commit`. (`set torq agent network web true` lets the agents reach every site.)\n"),
        Why::TheModel => format!("TorqOS: your agent reaches its model ({host}) only through TorqOS's model proxy, which keeps a record of each request: http://127.0.0.1:{proxy_port}, set as ANTHROPIC_BASE_URL. A connection past it is refused. An administrator can allow one in torqcli (`set torq agent network hosts {host}`); what goes that way is not recorded.\n"),
        Why::NotHttps => format!("TorqOS: your agent reaches sites as HTTPS only (port {HTTPS}). {host} on port {port} is refused.\n"),
        Why::Address => format!("TorqOS: {host} is an address of this machine itself, or of the network it sits on. The agents do not reach those through the proxy (one on the network, only where an administrator lists the site by name: `set torq agent network hosts {host}`).\n"),
        Why::NotAName => "TorqOS: that is not a site's name.\n".to_string(),
        Why::Listed | Why::SignIn | Why::Web => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(hosts: &[&str], web: bool) -> Policy {
        Policy { hosts: hosts.iter().map(|s| s.to_string()).collect(), web, model: "api.anthropic.com".into() }
    }

    #[test]
    fn closed_by_default_but_for_signing_in() {
        let p = policy(&[], false);
        assert_eq!(p.decide("example.org", 443), (false, Why::NotListed));
        assert_eq!(p.decide("platform.claude.com", 443), (true, Why::SignIn));
        assert_eq!(p.decide("PLATFORM.claude.com.", 443), (true, Why::SignIn), "a name is a name however it is written");
        assert_eq!(p.decide("mcp-proxy.anthropic.com", 443), (false, Why::NotListed), "the claude.ai connectors' host is not allowed unasked");
        assert_eq!(p.decide("evil.platform.claude.com", 443), (false, Why::NotListed));
        assert_eq!(p.decide("platform.claude.com.evil.example", 443), (false, Why::NotListed));
    }

    #[test]
    fn the_owners_list_names_sites_and_everything_under_a_name() {
        let p = policy(&["example.org", "*.github.com"], false);
        assert_eq!(p.decide("example.org", 443), (true, Why::Listed));
        assert_eq!(p.decide("www.example.org", 443), (false, Why::NotListed), "a name is that name only");
        assert_eq!(p.decide("api.github.com", 443), (true, Why::Listed));
        assert_eq!(p.decide("a.b.github.com", 443), (true, Why::Listed));
        assert_eq!(p.decide("github.com", 443), (false, Why::NotListed), "`*.name` is what is under it");
        assert_eq!(p.decide("notgithub.com", 443), (false, Why::NotListed));
        assert_eq!(p.decide("github.com.evil.example", 443), (false, Why::NotListed));
    }

    #[test]
    fn the_web_opens_every_site_but_not_the_model_past_the_record() {
        let p = policy(&[], true);
        assert_eq!(p.decide("example.org", 443), (true, Why::Web));
        assert_eq!(p.decide("203.0.113.9", 443), (true, Why::Web));
        assert_eq!(p.decide("api.anthropic.com", 443), (false, Why::TheModel));
        assert_eq!(p.decide("API.Anthropic.com.", 443), (false, Why::TheModel));
        assert_eq!(policy(&[], false).decide("api.anthropic.com", 443), (false, Why::TheModel));
        assert_eq!(policy(&["api.anthropic.com"], false).decide("api.anthropic.com", 443), (true, Why::Listed), "the owner's own word allows it");
    }

    #[test]
    fn https_only_and_names_only() {
        let p = policy(&["example.org"], true);
        assert_eq!(p.decide("example.org", 80), (false, Why::NotHttps));
        assert_eq!(p.decide("example.org", 22), (false, Why::NotHttps));
        assert_eq!(p.decide("", 443), (false, Why::NotAName));
        assert_eq!(p.decide("exa mple.org", 443), (false, Why::NotAName));
        assert_eq!(p.decide("example.org/path", 443), (false, Why::NotAName));
        assert_eq!(p.decide("a..b", 443), (false, Why::NotAName));
    }

    #[test]
    fn never_this_machine_and_its_own_network_only_when_listed() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        for never in ["127.0.0.1", "127.0.0.53", "0.0.0.0", "169.254.169.254", "224.0.0.1", "255.255.255.255", "::1", "::", "fe80::1", "ff02::1", "::ffff:127.0.0.1"] {
            assert!(!address_ok(ip(never), false) && !address_ok(ip(never), true), "{never}: never, listed or not");
        }
        for private in ["10.0.0.1", "172.16.5.5", "192.168.1.1", "100.64.0.1", "fd00::1", "::ffff:192.168.1.1"] {
            assert!(!address_ok(ip(private), false), "{private}: not by the web's rule");
            assert!(address_ok(ip(private), true), "{private}: where the owner listed the site");
        }
        for public in ["203.0.113.9", "8.8.8.8", "172.32.0.1", "100.128.0.1", "2001:db8::1"] {
            assert!(address_ok(ip(public), false), "{public}");
        }
    }

    #[test]
    fn a_refusal_names_the_setting() {
        assert!(refusal(Why::NotListed, "example.org", 443, 7801).contains("set torq agent network hosts example.org"));
        assert!(refusal(Why::NotListed, "example.org", 443, 7801).contains("set torq agent network web true"));
        assert!(refusal(Why::TheModel, "api.anthropic.com", 443, 7801).contains("http://127.0.0.1:7801"));
        assert!(refusal(Why::NotHttps, "example.org", 80, 7801).contains("port 80"));
    }
}
