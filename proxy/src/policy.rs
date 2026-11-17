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

/// A network: an address and how many of its first bits name it (an address of its own is all of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Net {
    pub addr: IpAddr,
    pub bits: u8,
}

impl Net {
    fn contains(&self, ip: IpAddr) -> bool {
        let same = |bits: u8, width: u8, a: u128, b: u128| bits == 0 || (bits <= width && (a ^ b) >> (width - bits) == 0);
        match (self.addr, ip) {
            (IpAddr::V4(n), IpAddr::V4(a)) => same(self.bits, 32, u32::from(n).into(), u32::from(a).into()),
            (IpAddr::V6(n), IpAddr::V6(a)) => same(self.bits, 128, n.into(), a.into()),
            _ => false,
        }
    }
}

/// This machine as the network sees it, read each time an address is judged: its own addresses, and the
/// networks it is on directly (a route with no router on the way). Each is refused as a private network's
/// address is, unless the owner listed the site by name: on a machine whose network is a public one (a
/// server's, at a provider), opening the web must not open its neighbours either.
#[derive(Debug, Clone, Default)]
pub struct Local {
    pub nets: Vec<Net>,
}

impl Local {
    /// Read from the kernel's own tables (`/proc/net`). A table that cannot be read adds nothing; the rules of
    /// `address_ok` for private networks hold all the same.
    pub fn read() -> Local {
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        Local::parse(&read("/proc/net/route"), &read("/proc/net/fib_trie"), &read("/proc/net/if_inet6"), &read("/proc/net/ipv6_route"))
    }

    /// From the tables' text: IPv4's routes with no gateway, but a default one (`route`), and its own addresses
    /// (`fib_trie`: each `/32 host LOCAL`); IPv6's own addresses, each with its network (`if_inet6`), and its
    /// routes with no next hop, but a default one (`ipv6_route`).
    pub fn parse(route: &str, fib_trie: &str, if_inet6: &str, ipv6_route: &str) -> Local {
        let mut nets = Vec::new();
        let v4 = |hex: &str| u32::from_str_radix(hex, 16).ok().map(|n| std::net::Ipv4Addr::from(n.to_le_bytes()));
        let v6 = |hex: &str| if hex.len() == 32 { u128::from_str_radix(hex, 16).ok().map(std::net::Ipv6Addr::from) } else { None };
        let bits = |hex: &str| u8::from_str_radix(hex, 16).ok().filter(|b| (1..=128).contains(b));
        for l in route.lines().skip(1) {
            let f: Vec<&str> = l.split_whitespace().collect();
            if let (Some(dest), Some(gw), Some(mask)) = (f.get(1).and_then(|h| v4(h)), f.get(2).and_then(|h| v4(h)), f.get(7).and_then(|h| v4(h))) {
                let n = u32::from(mask).count_ones() as u8;
                if gw.is_unspecified() && n > 0 {
                    nets.push(Net { addr: IpAddr::V4(dest), bits: n });
                }
            }
        }
        let mut last = None;
        for l in fib_trie.lines() {
            let t = l.trim_start();
            if let Some(a) = t.strip_prefix("|-- ") {
                last = a.trim().parse::<std::net::Ipv4Addr>().ok();
            } else if t.starts_with("/32 host LOCAL") {
                if let Some(a) = last {
                    nets.push(Net { addr: IpAddr::V4(a), bits: 32 });
                }
            }
        }
        for l in if_inet6.lines() {
            let f: Vec<&str> = l.split_whitespace().collect();
            if let Some(a) = f.first().and_then(|h| v6(h)) {
                nets.push(Net { addr: IpAddr::V6(a), bits: 128 });
                if let Some(n) = f.get(2).and_then(|h| bits(h)) {
                    nets.push(Net { addr: IpAddr::V6(a), bits: n });
                }
            }
        }
        for l in ipv6_route.lines() {
            let f: Vec<&str> = l.split_whitespace().collect();
            let flags = f.get(8).and_then(|h| u32::from_str_radix(h, 16).ok()).unwrap_or(0);
            // Not RTF_GATEWAY (a router on the way) nor RTF_REJECT (a route that refuses).
            if let (Some(dest), Some(n), Some(hop)) = (f.first().and_then(|h| v6(h)), f.get(1).and_then(|h| bits(h)), f.get(4).and_then(|h| v6(h))) {
                if hop.is_unspecified() && flags & 0x0202 == 0 {
                    nets.push(Net { addr: IpAddr::V6(dest), bits: n });
                }
            }
        }
        Local { nets }
    }

    fn holds(&self, ip: IpAddr) -> bool {
        self.nets.iter().any(|n| n.contains(ip))
    }
}

/// Whether the proxy connects to `ip` for an agent. Never to this machine's loopback, by any of its names for
/// it, nor to an address that only means something on one wire, nor to one that is no one's. An address of a
/// private network (the machine's own LAN: a router's page, a printer, another machine's service), of this
/// machine itself, or of a network it is on directly (`local`), only where the owner listed the site by name:
/// opening the web opens the internet, not the network the machine sits on. An IPv6 address that carries an
/// IPv4 one (mapped, NAT64's, 6to4's, the old "compatible" form) is judged as the IPv4 address it carries.
pub fn address_ok(ip: IpAddr, listed: bool, local: &Local) -> bool {
    let own = local.holds(ip);
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            // And 240/4: kept for later use, no one's today.
            let never = a.is_loopback() || a.is_unspecified() || a.is_multicast() || a.is_broadcast() || a.is_link_local() || o[0] == 0 || o[0] >= 240;
            // 10/8, 172.16/12, 192.168/16, 100.64/10 (addresses a provider shares), 198.18/15 (networks for
            // testing: a lab's own).
            let private = a.is_private() || (o[0] == 100 && (o[1] & 0xC0) == 64) || (o[0] == 198 && (o[1] & 0xFE) == 18);
            !never && (listed || !(private || own))
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            let carried = |hi: u16, lo: u16| IpAddr::V4(std::net::Ipv4Addr::from(((hi as u32) << 16) | lo as u32));
            if let Some(four) = a.to_ipv4_mapped() {
                return address_ok(IpAddr::V4(four), listed, local);
            }
            // An address of this machine's or of a network it is on is that, whatever it carries.
            if own && !listed {
                return false;
            }
            // 64:ff9b::/96 (NAT64: the IPv4 address in its last 32 bits), and ::a.b.c.d (IPv4-compatible: all
            // but :: and ::1, which are judged as themselves, below).
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] || (s[..6] == [0; 6] && (s[6] != 0 || s[7] > 1)) {
                return address_ok(carried(s[6], s[7]), listed, local);
            }
            // 2002::/16 (6to4: the IPv4 address in the 32 bits after the first 16).
            if s[0] == 0x2002 {
                return address_ok(carried(s[1], s[2]), listed, local);
            }
            let never = a.is_loopback() || a.is_unspecified() || a.is_multicast() || (s[0] & 0xFFC0) == 0xFE80;
            // fc00::/7 (addresses of a network's own), fec0::/10 (the old site-local ones), 64:ff9b:1::/48
            // (NAT64 of a network's own).
            let private = (s[0] & 0xFE00) == 0xFC00 || (s[0] & 0xFFC0) == 0xFEC0 || s[..3] == [0x64, 0xff9b, 1];
            !never && (listed || !(private || own))
        }
    }
}

/// What a refused agent is told, in plain words: that it is refused and that its person decides. Never how the
/// fence would be opened (the setting's name, a command): that is for its person, who reads it with each refused
/// site in `show system agent activity` (the security reader, 2026-11-17: an agent told the command asks for it,
/// or writes it into a proposal).
pub fn refusal(why: Why, host: &str, port: u16, proxy_port: u16) -> String {
    match why {
        Why::NotListed => format!("TorqOS: your agent may not reach {host}: it is not one of the sites this machine's agents may reach, and the sites they reach are your person's to decide. This refusal is in your person's record of what you did.\n"),
        // Not how it could be opened: that is the owner's to know, not the agent's to ask for.
        Why::TheModel => format!("TorqOS: your agent reaches its model ({host}) only through TorqOS's model proxy, which keeps a record of each request: http://127.0.0.1:{proxy_port}, set as ANTHROPIC_BASE_URL. A connection past it is refused.\n"),
        Why::NotHttps => format!("TorqOS: your agent reaches sites as HTTPS only (port {HTTPS}). {host} on port {port} is refused.\n"),
        Why::Address => format!("TorqOS: {host} is an address of this machine itself, or of the network it sits on, and the agents do not reach those through the proxy. Whether they may is your person's to decide. This refusal is in your person's record of what you did.\n"),
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
        let none = Local::default();
        for never in ["127.0.0.1", "127.0.0.53", "0.0.0.0", "169.254.169.254", "224.0.0.1", "255.255.255.255", "240.0.0.1", "250.1.2.3", "::1", "::", "fe80::1", "ff02::1", "::ffff:127.0.0.1",
            // Loopback and the like, carried in IPv6: NAT64's, 6to4's, the "compatible" form.
            "64:ff9b::7f00:1", "64:ff9b::a9fe:a9fe", "2002:7f00:1::1", "2002:a9fe:a9fe::", "::127.0.0.1", "::169.254.169.254"] {
            assert!(!address_ok(ip(never), false, &none) && !address_ok(ip(never), true, &none), "{never}: never, listed or not");
        }
        for private in ["10.0.0.1", "172.16.5.5", "192.168.1.1", "100.64.0.1", "198.18.0.1", "198.19.255.255", "fd00::1", "fec0::1", "64:ff9b:1::a00:1", "::ffff:192.168.1.1",
            // A private address carried in IPv6.
            "64:ff9b::c0a8:101", "64:ff9b::10.0.0.1", "2002:c0a8:101::1", "2002:a00:1:1::5", "::192.168.1.1", "::10.1.2.3"] {
            assert!(!address_ok(ip(private), false, &none), "{private}: not by the web's rule");
            assert!(address_ok(ip(private), true, &none), "{private}: where the owner listed the site");
        }
        for public in ["203.0.113.9", "8.8.8.8", "172.32.0.1", "100.128.0.1", "198.20.0.1", "198.17.255.255", "223.255.255.254", "2001:db8::1", "64:ff9b::808:808", "2002:808:808::1", "::8.8.8.8"] {
            assert!(address_ok(ip(public), false, &none), "{public}");
        }
    }

    /// The machine's own addresses and the networks it is on directly, as the kernel's tables say them (here as
    /// a machine at a provider would have them: a public network, 203.0.113.0/24 and 2001:db8:5::/64): refused
    /// under the web's rule, reached where the owner listed the site by name. The tables' text is the kernel's.
    #[test]
    fn this_machines_own_addresses_and_networks_only_when_listed() {
        let route = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
eth0\t00000000\t017100CB\t0003\t0\t0\t1024\t00000000\t0\t0\t0\n\
eth0\t007100CB\t00000000\t0001\t0\t0\t1024\t00FFFFFF\t0\t0\t0\n\
eth0\t08080808\t017100CB\t0007\t0\t0\t1024\tFFFFFFFF\t0\t0\t0\n";
        let fib = "Main:\n  +-- 0.0.0.0/0 3 0 4\n     |-- 198.51.100.7\n        /32 host LOCAL\n     |-- 203.0.113.0\n        /24 link UNICAST\n";
        let if6 = "20010db8000500000000000000000009 02 40 00 80     eth0\n00000000000000000000000000000001 01 80 10 80       lo\n";
        let r6 = "20010db8000600000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001     eth0\n\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 20010db8000500000000000000000001 00000400 00000001 00000000 00000003     eth0\n\
20010db8000700000000000000000000 40 00000000000000000000000000000000 00 20010db8000500000000000000000001 00000400 00000001 00000000 00000003     eth0\n";
        let local = Local::parse(route, fib, if6, r6);
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        for own in ["203.0.113.5", "203.0.113.255", "198.51.100.7", "2001:db8:5::9", "2001:db8:5::77", "2001:db8:6::1", "::ffff:203.0.113.5", "64:ff9b::cb00:7105", "2002:cb00:7105::1"] {
            assert!(!address_ok(ip(own), false, &local), "{own}: this machine or a network it is on, not by the web's rule");
            assert!(address_ok(ip(own), true, &local), "{own}: where the owner listed the site");
        }
        for beyond in ["203.0.114.1", "198.51.100.8", "8.8.8.8", "2001:db8:7::1", "2001:db8:5:1::1"] {
            assert!(address_ok(ip(beyond), false, &local), "{beyond}: past a router, the internet");
        }
        // The tables of the machine this runs on are read without failing, whatever they hold.
        let _ = Local::read();
    }

    #[test]
    fn a_refusal_says_the_person_decides_and_never_how_to_open_the_fence() {
        for why in [Why::NotListed, Why::Address] {
            let said = refusal(why, "example.org", 443, 7801);
            assert!(said.contains("example.org") && said.contains("your person's to decide") && said.contains("record"), "{said}");
            assert!(!said.contains("set torq") && !said.contains("torqcli") && !said.contains("commit") && !said.contains("administrator") && !said.contains("web true"), "{said}");
        }
        assert!(refusal(Why::TheModel, "api.anthropic.com", 443, 7801).contains("http://127.0.0.1:7801"));
        // The model past the record: refused, and the agent is not told how it could be opened.
        let past = refusal(Why::TheModel, "api.anthropic.com", 443, 7801);
        assert!(past.contains("refused") && !past.contains("set torq") && !past.contains("administrator") && !past.contains("torqcli"), "{past}");
        assert!(refusal(Why::NotHttps, "example.org", 80, 7801).contains("port 80"));
    }
}
