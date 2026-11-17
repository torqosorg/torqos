//! Who is connected, by the kernel's word (ADR-057): never by what the caller says.
//!
//! The proxy listens on 127.0.0.1. For each connection it reads the kernel's own table of TCP connections
//! (`/proc/net/tcp`, and `tcp6` for a program that reached 127.0.0.1 from an IPv6 socket) and takes the uid of
//! the socket at the other end. The broker names the caller of the secrets daemon the same way (ADR-052,
//! `broker/src/secrets.rs`). The connection is held open by the proxy while it reads, so the line it finds is
//! this connection's: the kernel gives the same two ports to no other while one end is open.

use std::net::SocketAddr;

/// The uid of the socket whose own end is 127.0.0.1:`client` and whose other end is 127.0.0.1:`server`,
/// established, in the kernel's table as text (IPv4's and IPv6's together). Only if it is the one established
/// connection with those two ports, whatever its addresses: a second one (another account on 127.0.0.2, say,
/// that took the same client port to the same server port) makes the answer no one, never the first one's.
pub fn uid_of_connection(table: &str, client: u16, server: u16) -> Option<u32> {
    // 127.0.0.1 as the table writes it: in IPv4's, and as IPv6's maps it.
    const LOOPBACK: [&str; 2] = ["0100007F", "0000000000000000FFFF00000100007F"];
    let split = |field: &str| -> Option<(bool, u16)> {
        let (addr, port) = field.split_once(':')?;
        Some((LOOPBACK.contains(&addr), u16::from_str_radix(port, 16).ok()?))
    };
    // Each socket once: the kernel writes the table a page at a time, and one that changes between two pages can
    // show a socket twice. A socket is its two ends' addresses, its uid and its inode together.
    let found: std::collections::BTreeSet<(bool, &str, &str, Option<u32>, &str)> = table.lines().filter_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        // sl local remote st tx:rx tr:tm retrnsmt uid timeout inode …; 01 is ESTABLISHED.
        let ((here, local), (there, remote)) = (split(f.get(1)?)?, split(f.get(2)?)?);
        (f.len() > 9 && local == client && remote == server && f[3] == "01").then(|| (here && there, f[1], f[2], f[7].parse::<u32>().ok(), f[9]))
    }).collect();
    let mut found = found.into_iter();
    match (found.next(), found.next()) {
        (Some((true, _, _, uid, _)), None) => uid,
        _ => None,
    }
}

/// The uid at the other end of a connection this program accepted on `local` from `peer`. Only a connection
/// whose two ends are this machine's own address (127.0.0.1) is found in the table at all: any other is no one's.
pub fn caller(peer: SocketAddr, local: SocketAddr) -> Option<u32> {
    // Exactly 127.0.0.1 at both ends: every other loopback address (127.0.0.2, …) is anyone's to bind.
    let one27 = |a: SocketAddr| a.ip().to_canonical() == std::net::IpAddr::from([127, 0, 0, 1]);
    if !one27(peer) || !one27(local) {
        return None;
    }
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    // Read again where it was not found: a table that changed while it was written can leave a socket out. (Two
    // connections with the same ports are found again each time, and are no one's each time.)
    (0..3).find_map(|_| uid_of_connection(&format!("{}{}", read("/proc/net/tcp"), read("/proc/net/tcp6")), peer.port(), local.port()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/proc/net/tcp` as the kernel writes it: the proxy listening, the two ends of one connection, and one
    /// that is not from this machine.
    const TABLE: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1E79 00000000:0000 0A 00000000:00000000 00:00000000 00000000   989        0 1 1 0000000000000000 100 0 0 10 0
   1: 0100007F:C350 0100007F:1E79 01 00000000:00000000 00:00000000 00000000   994        0 2 1 0000000000000000 20 4 30 10 -1
   2: 0100007F:1E79 0100007F:C350 01 00000000:00000000 00:00000000 00000000   989        0 3 1 0000000000000000 20 4 30 10 -1
   3: 0A00000F:C351 0100007F:1E79 01 00000000:00000000 00:00000000 00000000  1000        0 4 1 0000000000000000 20 4 30 10 -1
";

    #[test]
    fn the_caller_is_the_kernels_answer() {
        assert_eq!(uid_of_connection(TABLE, 0xC350, 0x1E79), Some(994), "the caller's end, not the proxy's");
        assert_eq!(uid_of_connection(TABLE, 0x1E79, 0xC350), Some(989), "read the other way, the proxy's own");
        assert_eq!(uid_of_connection(TABLE, 0xC351, 0x1E79), None, "not from this machine's own address");
        assert_eq!(uid_of_connection(TABLE, 0xC352, 0x1E79), None, "no such connection");
        let six = "  sl  local_address                         remote_address                        st\n   0: 0000000000000000FFFF00000100007F:C350 0000000000000000FFFF00000100007F:1E79 01 00000000:00000000 00:00000000 00000000   994 0 9\n";
        assert_eq!(uid_of_connection(six, 0xC350, 0x1E79), Some(994), "from an IPv6 socket");
        let closing = TABLE.replace(" 01 00000000:00000000 00:00000000 00000000   994", " 08 00000000:00000000 00:00000000 00000000   994");
        assert_eq!(uid_of_connection(&closing, 0xC350, 0x1E79), None, "established only");
    }

    /// Another socket on 127.0.0.2 with a live agent's client port, to the same server port, makes the lookup
    /// ambiguous: refused, never taken for the agent (the security reader's finding, 2026-11-17).
    #[test]
    fn a_second_connection_from_another_loopback_address_is_no_ones() {
        let spoof = TABLE.to_string() + "   4: 0200007F:C350 0100007F:1E79 01 00000000:00000000 00:00000000 00000000  1000        0 5 1 0000000000000000 20 4 30 10 -1\n";
        assert_eq!(uid_of_connection(&spoof, 0xC350, 0x1E79), None, "two connections with these ports: no one");
        let six = "  sl  local_address                         remote_address                        st\n   0: 0000000000000000FFFF00000200007F:C350 0000000000000000FFFF00000100007F:1E79 01 00000000:00000000 00:00000000 00000000  1000 0 9\n";
        assert_eq!(uid_of_connection(&format!("{TABLE}{six}"), 0xC350, 0x1E79), None, "in the IPv6 table too");
    }

    /// The kernel writes its table a page at a time, and a table that changes between two pages can show one
    /// socket twice (seen: one run in forty of tests/proxy.rs, with many connections opening and closing). The
    /// same socket twice is still one connection; two sockets are two.
    #[test]
    fn one_socket_shown_twice_is_one_connection() {
        let twice = TABLE.to_string() + "   4: 0100007F:C350 0100007F:1E79 01 00000000:00000000 00:00000000 00000000   994        0 2 1 0000000000000000 20 4 30 10 -1\n";
        assert_eq!(uid_of_connection(&twice, 0xC350, 0x1E79), Some(994));
    }

    /// A caller on another loopback address (127.0.0.2) is no one, whoever it is.
    #[tokio::test]
    async fn a_caller_not_at_127_0_0_1_is_no_one() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let s = tokio::net::TcpSocket::new_v4().unwrap();
        s.bind("127.0.0.2:0".parse().unwrap()).unwrap();
        let _c = s.connect(l.local_addr().unwrap()).await.unwrap();
        let (accepted, peer) = l.accept().await.unwrap();
        assert_eq!(peer.ip().to_string(), "127.0.0.2");
        assert_eq!(caller(peer, accepted.local_addr().unwrap()), None);
    }

    /// Against the real table: a connection this test makes is this test's own account's.
    #[test]
    fn a_real_connection_is_this_accounts() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let c = std::net::TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (s, peer) = l.accept().unwrap();
        let me = std::fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m)).unwrap();
        assert_eq!(caller(peer, s.local_addr().unwrap()), Some(me));
        assert_eq!(caller("192.0.2.1:5".parse().unwrap(), s.local_addr().unwrap()), None, "not this machine: no one");
        drop(c);
    }
}
