//! Who holds a TCP connection on this machine's own address, by the kernel's word (ADR-052, ADR-057): the model
//! proxy names its caller so, and the broker names the caller of the secrets daemon. One implementation for both.
//!
//! The kernel is asked for the one socket with exactly these two ends, 127.0.0.1:`local` and 127.0.0.1:`remote`,
//! through its socket diagnostics (`NETLINK_SOCK_DIAG`, `inet_diag`): one question and one answer, whatever else is
//! open on the machine. (Reading its whole table, `/proc/net/tcp`, cost as much as the table is long, for each
//! connection: an account that opened many could stall every caller; and it went by the ports, so another
//! loopback address with the same ports made the answer no one's. The security reader, 2026-11-17.)
//!
//! The kernel holds at most one socket for one pair of ends; its answer is taken only if it is that socket,
//! established, and 127.0.0.1 at both ends (IPv4, or IPv6's way of writing 127.0.0.1 for a program that reached it
//! from an IPv6 socket). The lookup falls back to a socket listening on `local` where there is no connection:
//! that is no connection, and no one's.

use rustix::net::{netlink, AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType};
use std::io;

/// `SOCK_DIAG_BY_FAMILY`, the request and its answer's message type.
const SOCK_DIAG_BY_FAMILY: u16 = 20;
/// `NLM_F_REQUEST`, alone: one socket asked for, not a list of them.
const NLM_F_REQUEST: u16 = 1;
const IPPROTO_TCP: u8 = 6;
const TCP_ESTABLISHED: u8 = 1;
const AF_INET: u8 = 2;
const AF_INET6: u8 = 10;
/// The netlink header, and `struct inet_diag_req_v2` after it.
const HEADER: usize = 16;
const REQUEST: usize = HEADER + 56;
/// `struct inet_diag_msg` after the header: family, state, timer, retransmits, the socket's ends (48 bytes), its
/// expiry and queues, then its uid and inode.
const ANSWER: usize = HEADER + 72;
const LOOPBACK: [u8; 4] = [127, 0, 0, 1];
/// 127.0.0.1 as an IPv6 socket has it (`::ffff:127.0.0.1`).
const LOOPBACK_MAPPED: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 127, 0, 0, 1];

/// The question for the socket whose own end is 127.0.0.1:`local` and whose other end is 127.0.0.1:`remote`.
pub fn question(local: u16, remote: u16) -> [u8; REQUEST] {
    let mut q = [0u8; REQUEST];
    q[0..4].copy_from_slice(&(REQUEST as u32).to_ne_bytes());
    q[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
    q[6..8].copy_from_slice(&NLM_F_REQUEST.to_ne_bytes());
    // inet_diag_req_v2: family, protocol, extensions, padding, the states (every one: the answer's is checked).
    q[16] = AF_INET;
    q[17] = IPPROTO_TCP;
    q[20..24].copy_from_slice(&u32::MAX.to_ne_bytes());
    // inet_diag_sockid: the ports and addresses in the network's order, no interface, no cookie (all ones).
    q[24..26].copy_from_slice(&local.to_be_bytes());
    q[26..28].copy_from_slice(&remote.to_be_bytes());
    q[28..32].copy_from_slice(&LOOPBACK);
    q[44..48].copy_from_slice(&LOOPBACK);
    q[64..72].copy_from_slice(&[0xff; 8]);
    q
}

/// The kernel's answer read: the socket's uid, where it is the connection asked for (established, these ports,
/// 127.0.0.1 at both ends); no one otherwise, a refusal or "no such socket" included.
pub fn answer(bytes: &[u8], local: u16, remote: u16) -> Option<u32> {
    let u16_at = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_ne_bytes([b[0], b[1]]));
    let u32_at = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]));
    let len = u32_at(0)? as usize;
    if len < ANSWER || len > bytes.len() || u16_at(4)? != SOCK_DIAG_BY_FAMILY {
        return None;
    }
    let m = &bytes[HEADER..len];
    let (src, dst) = (&m[8..24], &m[24..40]);
    let ours = match m[0] {
        AF_INET => src[..4] == LOOPBACK && dst[..4] == LOOPBACK,
        AF_INET6 => src == LOOPBACK_MAPPED && dst == LOOPBACK_MAPPED,
        _ => false,
    };
    let ports = m[4..6] == local.to_be_bytes() && m[6..8] == remote.to_be_bytes();
    (ours && ports && m[1] == TCP_ESTABLISHED).then(|| u32_at(HEADER + 64)).flatten()
}

/// The uid of the established TCP connection whose own end is 127.0.0.1:`local` and whose other end is
/// 127.0.0.1:`remote`, as the kernel has it now. None: there is no such connection.
pub fn uid_of_loopback(local: u16, remote: u16) -> io::Result<Option<u32>> {
    let fd = rustix::net::socket_with(AddressFamily::NETLINK, SocketType::DGRAM, SocketFlags::CLOEXEC, Some(netlink::SOCK_DIAG))?;
    rustix::net::sendto(&fd, &question(local, remote), SendFlags::empty(), &netlink::SocketAddrNetlink::new(0, 0))?;
    let mut buf = [0u8; 1024];
    let (got, _) = rustix::net::recv(&fd, &mut buf, RecvFlags::empty())?;
    Ok(answer(&buf[..got.min(buf.len())], local, remote))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, TcpListener, TcpStream};

    /// `NLMSG_ERROR`: the kernel's refusal, or "no such socket" (`ENOENT`).
    const NLMSG_ERROR: u16 = 2;

    fn me() -> u32 {
        std::fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m)).unwrap()
    }

    /// An answer as the kernel writes one: `family`, `state`, the ends, the uid.
    fn made(family: u8, state: u8, local: u16, remote: u16, src: &[u8], dst: &[u8], uid: u32) -> Vec<u8> {
        let mut a = vec![0u8; ANSWER];
        a[0..4].copy_from_slice(&(ANSWER as u32).to_ne_bytes());
        a[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
        let m = &mut a[HEADER..];
        m[0] = family;
        m[1] = state;
        m[4..6].copy_from_slice(&local.to_be_bytes());
        m[6..8].copy_from_slice(&remote.to_be_bytes());
        m[8..8 + src.len()].copy_from_slice(src);
        m[24..24 + dst.len()].copy_from_slice(dst);
        m[64..68].copy_from_slice(&uid.to_ne_bytes());
        a
    }

    #[test]
    fn only_the_connection_asked_for_is_anyones() {
        assert_eq!(answer(&made(AF_INET, TCP_ESTABLISHED, 50000, 7801, &LOOPBACK, &LOOPBACK, 994), 50000, 7801), Some(994));
        assert_eq!(answer(&made(AF_INET6, TCP_ESTABLISHED, 50000, 7801, &LOOPBACK_MAPPED, &LOOPBACK_MAPPED, 994), 50000, 7801), Some(994), "from an IPv6 socket");
        // A socket listening on the port asked for: where there is no connection, the kernel's lookup gives that.
        assert_eq!(answer(&made(AF_INET, 10, 50000, 0, &LOOPBACK, &[0; 4], 0), 50000, 7801), None, "a listener is no connection");
        assert_eq!(answer(&made(AF_INET, 8, 50000, 7801, &LOOPBACK, &LOOPBACK, 994), 50000, 7801), None, "established only");
        assert_eq!(answer(&made(AF_INET, TCP_ESTABLISHED, 50000, 7801, &[127, 0, 0, 2], &LOOPBACK, 1000), 50000, 7801), None, "another loopback address");
        assert_eq!(answer(&made(AF_INET, TCP_ESTABLISHED, 50000, 7801, &LOOPBACK, &[10, 0, 0, 15], 1000), 50000, 7801), None);
        assert_eq!(answer(&made(AF_INET, TCP_ESTABLISHED, 50001, 7801, &LOOPBACK, &LOOPBACK, 994), 50000, 7801), None, "other ports");
        let mut refused = made(AF_INET, TCP_ESTABLISHED, 50000, 7801, &LOOPBACK, &LOOPBACK, 994);
        refused[4..6].copy_from_slice(&NLMSG_ERROR.to_ne_bytes());
        assert_eq!(answer(&refused, 50000, 7801), None, "the kernel's refusal, or no such socket");
        assert_eq!(answer(&[0; 8], 50000, 7801), None, "too short");
    }

    /// Against the kernel: a connection this test makes is this test's account's, at both ends; a listening socket is
    /// no connection; no connection, no one.
    #[test]
    fn the_kernel_names_a_real_connection() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (_s, peer) = l.accept().unwrap();
        let server = l.local_addr().unwrap().port();
        assert_eq!(uid_of_loopback(peer.port(), server).unwrap(), Some(me()));
        assert_eq!(uid_of_loopback(server, peer.port()).unwrap(), Some(me()), "the other end");
        assert_eq!(uid_of_loopback(server, 1).unwrap(), None, "the listener, asked as a connection's end");
        drop(c);
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(uid_of_loopback(free, server).unwrap(), None);
    }

    /// Someone else on 127.0.0.2 with the same client port, to the same server: the connection asked for is still
    /// found, and theirs is never taken for it (the lookup went by the ports, and made both no one's).
    #[test]
    fn the_same_ports_from_another_loopback_address_change_nothing() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let server = l.local_addr().unwrap();
        let mine = TcpStream::connect(server).unwrap();
        let (_a, peer) = l.accept().unwrap();
        let other = connect_from(SocketAddr::from(([127, 0, 0, 2], peer.port())), server);
        let (_b, theirs) = l.accept().unwrap();
        assert_eq!((theirs.ip().to_string(), theirs.port()), ("127.0.0.2".to_string(), peer.port()));
        assert_eq!(uid_of_loopback(peer.port(), server.port()).unwrap(), Some(me()));
        drop((mine, other));
    }

    /// A connection to `to` from exactly `from` (std cannot bind a socket before it connects).
    fn connect_from(from: SocketAddr, to: SocketAddr) -> rustix::fd::OwnedFd {
        let s = rustix::net::socket(AddressFamily::INET, SocketType::STREAM, None).unwrap();
        rustix::net::bind(&s, &from).unwrap();
        rustix::net::connect(&s, &to).unwrap();
        s
    }
}
