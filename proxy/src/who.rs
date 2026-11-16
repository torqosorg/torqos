//! Who is connected, by the kernel's word (ADR-057): never by what the caller says.
//!
//! The proxy listens on 127.0.0.1. For each connection it reads the kernel's own table of TCP connections
//! (`/proc/net/tcp`, and `tcp6` for a program that reached 127.0.0.1 from an IPv6 socket) and takes the uid of
//! the socket at the other end. The broker names the caller of the secrets daemon the same way (ADR-052,
//! `broker/src/secrets.rs`). The connection is held open by the proxy while it reads, so the line it finds is
//! this connection's: the kernel gives the same two ports to no other while one end is open.

use std::net::SocketAddr;

/// The uid of the socket whose own end is 127.0.0.1:`client` and whose other end is 127.0.0.1:`server`,
/// established, in the kernel's table as text.
pub fn uid_of_connection(table: &str, client: u16, server: u16) -> Option<u32> {
    // 127.0.0.1 as the table writes it: in IPv4's, and as IPv6's maps it.
    const LOOPBACK: [&str; 2] = ["0100007F", "0000000000000000FFFF00000100007F"];
    let at = |field: &str| -> Option<u16> {
        let (addr, port) = field.split_once(':')?;
        LOOPBACK.contains(&addr).then(|| u16::from_str_radix(port, 16).ok()).flatten()
    };
    table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        // sl local remote st tx:rx tr:tm retrnsmt uid …; 01 is ESTABLISHED.
        (f.len() > 7 && at(f[1]) == Some(client) && at(f[2]) == Some(server) && f[3] == "01").then(|| f[7].parse().ok()).flatten()
    })
}

/// The uid at the other end of a connection this program accepted on `local` from `peer`: both on this
/// machine's own address (127.0.0.1), or no one.
pub fn caller(peer: SocketAddr, local: SocketAddr) -> Option<u32> {
    if !peer.ip().is_loopback() || !local.ip().is_loopback() {
        return None;
    }
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    uid_of_connection(&read("/proc/net/tcp"), peer.port(), local.port()).or_else(|| uid_of_connection(&read("/proc/net/tcp6"), peer.port(), local.port()))
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
