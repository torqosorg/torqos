//! Who is connected, by the kernel's word (ADR-057): never by what the caller says.
//!
//! The proxy listens on 127.0.0.1. For each connection it asks the kernel for the one socket with exactly the
//! connection's two ends and takes that socket's uid (`torq_wire::peer`, through the kernel's socket diagnostics:
//! one question, whatever else is open). The broker names the caller of the secrets daemon the same way, with the
//! same code (ADR-052). The connection is held open by the proxy while it asks, so the socket found is this
//! connection's: the kernel gives the same two ends to no other while one is open.

use std::net::SocketAddr;

/// The uid at the other end of a connection this program accepted on `local` from `peer`. Only a connection whose
/// two ends are this machine's own address exactly (127.0.0.1) is anyone's: every other loopback address
/// (127.0.0.2, …) is anyone's to bind.
pub fn caller(peer: SocketAddr, local: SocketAddr) -> Option<u32> {
    let one27 = |a: SocketAddr| a.ip().to_canonical() == std::net::IpAddr::from([127, 0, 0, 1]);
    if !one27(peer) || !one27(local) {
        return None;
    }
    torq_wire::peer::uid_of_loopback(peer.port(), local.port()).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Against the kernel: a connection this test makes is this test's own account's; and someone else on
    /// 127.0.0.2 with the same client port, to the same server, changes nothing (the lookup by ports alone made the
    /// first no one's: the security reader, 2026-11-17).
    #[tokio::test]
    async fn a_real_connection_is_this_accounts_whatever_else_has_its_ports() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _c = tokio::net::TcpStream::connect(l.local_addr().unwrap()).await.unwrap();
        let (s, peer) = l.accept().await.unwrap();
        let me = std::fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m)).unwrap();
        assert_eq!(caller(peer, s.local_addr().unwrap()), Some(me));
        let other = tokio::net::TcpSocket::new_v4().unwrap();
        other.bind(SocketAddr::from(([127, 0, 0, 2], peer.port()))).unwrap();
        let _o = other.connect(l.local_addr().unwrap()).await.unwrap();
        let (_t, theirs) = l.accept().await.unwrap();
        assert_eq!(theirs.port(), peer.port());
        assert_eq!(caller(peer, s.local_addr().unwrap()), Some(me), "still found, and still mine");
        assert_eq!(caller("192.0.2.1:5".parse().unwrap(), s.local_addr().unwrap()), None, "not this machine: no one");
    }
}
