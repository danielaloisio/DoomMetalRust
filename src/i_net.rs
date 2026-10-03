//-----------------------------------------------------------------------------
//
// Copyright (C) 1993-1996 by id Software, Inc.
//
// This source is available for distribution and/or modification
// only under the terms of the DOOM Source Code License as
// published by id Software. All rights reserved.
//
// The source is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// FITNESS FOR A PARTICULAR PURPOSE. See the DOOM Source Code License
// for more details.
//
// DESCRIPTION:
//
//-----------------------------------------------------------------------------

//! Rust port of `i_net.c`: the UDP network driver.
//!
//! `I_InitNetwork` parses the command line (`-net <player> <host>...`,
//! `-port`, `-dup`, `-extratic`) and, for a netgame, opens the sockets;
//! `I_NetCmd`'s `CMD_SEND`/`CMD_GET` are [`UdpTransport`]'s
//! [`NetTransport`] methods (the byte swapping lives in
//! [`crate::d_net::DoomData::to_bytes`]/`from_bytes`, same wire format).
//!
//! Two small extensions over the original, both only for running several
//! copies on one machine (the original needs one machine per player):
//!
//! - a host may carry its own port, `host:port` (default: `-port`'s value,
//!   `DOOMPORT`, 5029);
//! - packets go out from the *bound* socket (the original sends from a
//!   second, unbound one), so a receiver can tell two copies on the same
//!   IP apart by source port. Incoming packets are matched to a node by
//!   IP and source port first, then by IP alone when exactly one node has
//!   that IP — which is all the original (and any original-protocol
//!   peer, whose source port is an ephemeral one) ever needed.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

use crate::d_net::{NetInfo, NetTransport, MAXNETNODES};

/// (`DOOMPORT`): `IPPORT_USERRESERVED + 0x1d`.
pub const DOOMPORT: u16 = 5000 + 0x1d;

/// Largest datagram we accept (`sizeof(doomdata_t)` is 8 + 12 * 8 bytes;
/// anything longer is rejected by the packet parser anyway).
const RECV_BUFFER: usize = 2048;

/// The UDP side of `I_NetCmd`: `sendaddress[]`, `insocket`.
pub struct UdpTransport {
    socket: UdpSocket,
    /// `sendaddress[]`: index 0 is this machine (unused).
    nodes: Vec<Option<SocketAddr>>,
}

impl UdpTransport {
    /// Binds `port` on every interface (`BindToLocalPort`) in
    /// non-blocking mode (`FIONBIO`), with `peers` as nodes `1..`.
    pub fn bind(port: u16, peers: Vec<SocketAddr>) -> std::io::Result<UdpTransport> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_nonblocking(true)?;
        let mut nodes = vec![None];
        nodes.extend(peers.into_iter().map(Some));
        Ok(UdpTransport { socket, nodes })
    }

    /// The port actually bound (useful with port 0 in tests).
    pub fn local_port(&self) -> u16 {
        self.socket.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// Finds which node `from` is (the original compares IPs only).
    fn node_for(&self, from: SocketAddr) -> Option<usize> {
        let same_ip: Vec<usize> = (1..self.nodes.len())
            .filter(|&i| self.nodes[i].is_some_and(|a| a.ip() == from.ip()))
            .collect();
        same_ip
            .iter()
            .copied()
            .find(|&i| self.nodes[i].is_some_and(|a| a.port() == from.port()))
            .or(if same_ip.len() == 1 {
                Some(same_ip[0])
            } else {
                None
            })
    }
}

impl NetTransport for UdpTransport {
    /// `PacketSend`. Errors are ignored, like the original (its
    /// `I_Error` on a failed send is commented out): a lost packet is
    /// just a lost packet.
    fn send(&mut self, node: usize, packet: &[u8]) {
        if let Some(Some(addr)) = self.nodes.get(node) {
            let _ = self.socket.send_to(packet, addr);
        }
    }

    /// `PacketGet`: the next datagram from one of the players. Packets
    /// from anyone else (a new game's broadcast) are dropped.
    fn recv(&mut self) -> Option<(usize, Vec<u8>)> {
        let mut buf = [0u8; RECV_BUFFER];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((n, from)) => {
                    if let Some(node) = self.node_for(from) {
                        return Some((node, buf[..n].to_vec()));
                    }
                    // not from a player: look at the next one
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return None,
                // a previous send bounced (ICMP port unreachable on some
                // platforms): nothing to read, not a failure
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
                Err(e) => panic!("GetPacket: {e}"),
            }
        }
    }
}

/// Resolves one `-net` host argument like the original: a leading `.`
/// marks a literal IP address (`inet_addr`), anything else is a host
/// name; an optional `:port` overrides `default_port`. IPv4 only
/// (`AF_INET`).
pub fn resolve_host(arg: &str, default_port: u16) -> Result<SocketAddr, String> {
    let spec = arg.strip_prefix('.').unwrap_or(arg);
    let (host, port) = match spec.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => (
            h,
            p.parse::<u16>().map_err(|_| format!("bad port in {arg}"))?,
        ),
        _ => (spec, default_port),
    };
    (host, port)
        .to_socket_addrs()
        .map_err(|_| format!("gethostbyname: couldn't find {arg}"))?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| format!("gethostbyname: couldn't find {arg}"))
}

/// Port of `I_InitNetwork` over `args` (`myargv`, `args[0]` the program
/// name): the [`NetInfo`] for `D_CheckNetGame` and the transport to talk
/// through. Without `-net` that's a single player game with no transport.
///
/// `-net <consoleplayer> <host> <host> ...` lists the *other* machines
/// (in the order every machine agrees on is not required: each machine
/// numbers its peers itself, only the player numbers must be distinct).
pub fn i_init_network(args: &[String]) -> Result<(NetInfo, Box<dyn NetTransport>), String> {
    let find = |name: &str| {
        args.iter()
            .enumerate()
            .skip(1)
            .find(|(_, a)| a.eq_ignore_ascii_case(name))
            .map(|(i, _)| i)
    };

    // set up for network
    let ticdup = match find("-dup") {
        Some(i) if i + 1 < args.len() => {
            let c = args[i + 1]
                .bytes()
                .next()
                .map_or(0, |b| b as i32 - '0' as i32);
            c.clamp(1, 9)
        }
        _ => 1,
    };
    let extratics = i32::from(find("-extratic").is_some());

    let mut port = DOOMPORT;
    if let Some(p) = find("-port") {
        if p + 1 < args.len() {
            port = args[p + 1]
                .parse()
                .map_err(|_| format!("bad -port {}", args[p + 1]))?;
            eprintln!("using alternate port {port}");
        }
    }

    // parse network game options, -net <consoleplayer> <host> <host> ...
    let Some(i) = find("-net") else {
        // single player game
        return Ok((
            NetInfo {
                ticdup,
                extratics,
                ..NetInfo::single_player()
            },
            Box::new(crate::d_net::NullTransport),
        ));
    };

    let consoleplayer = args
        .get(i + 1)
        .and_then(|a| a.bytes().next())
        .map(|b| b as i32 - '1' as i32)
        .ok_or("-net needs a player number")?;

    let mut peers = Vec::new();
    for a in args.iter().skip(i + 2).take_while(|a| !a.starts_with('-')) {
        peers.push(resolve_host(a, port)?);
    }
    let numnodes = peers.len() + 1; // this node for sure
    if numnodes > MAXNETNODES {
        return Err(format!(
            "too many hosts ({numnodes} nodes, max {MAXNETNODES})"
        ));
    }
    if consoleplayer < 0 || consoleplayer as usize >= numnodes {
        return Err(format!(
            "-net player {} is outside the {numnodes} players",
            consoleplayer + 1
        ));
    }

    // build message to receive
    let transport =
        UdpTransport::bind(port, peers).map_err(|e| format!("BindToPort: bind: {e}"))?;

    Ok((
        NetInfo {
            netgame: true,
            numnodes,
            numplayers: numnodes,
            consoleplayer: consoleplayer as usize,
            ticdup,
            extratics,
        },
        Box::new(transport),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d_net::test_support::{assert_lockstep, play};
    use crate::d_net::NetStart;

    fn args(s: &str) -> Vec<String> {
        std::iter::once("doom")
            .chain(s.split_whitespace())
            .map(String::from)
            .collect()
    }

    #[test]
    fn no_net_is_single_player_with_the_original_defaults() {
        let (info, _) = i_init_network(&args("-skill 3")).unwrap();
        assert_eq!(info, NetInfo::single_player());
        assert_eq!((info.ticdup, info.extratics), (1, 0));
    }

    #[test]
    fn dup_is_clamped_and_extratic_enables_a_backup_tic() {
        let (info, _) = i_init_network(&args("-dup 7 -extratic")).unwrap();
        assert_eq!((info.ticdup, info.extratics), (7, 1));
        assert_eq!(i_init_network(&args("-dup 0")).unwrap().0.ticdup, 1);
        assert_eq!(i_init_network(&args("-dup 9")).unwrap().0.ticdup, 9);
    }

    #[test]
    fn hosts_resolve_with_an_optional_port_and_the_dot_prefix() {
        assert_eq!(
            resolve_host(".127.0.0.1", 5029).unwrap(),
            "127.0.0.1:5029".parse().unwrap()
        );
        assert_eq!(
            resolve_host("127.0.0.1:7000", 5029).unwrap(),
            "127.0.0.1:7000".parse().unwrap()
        );
        assert_eq!(
            resolve_host("localhost", 5029).unwrap().port(),
            5029,
            "name resolved, default port"
        );
        assert!(resolve_host("127.0.0.1:nope", 5029).is_err());
    }

    #[test]
    fn net_arguments_name_the_player_and_peers() {
        let (info, _) = i_init_network(&args("-net 2 127.0.0.1:41999 -port 0")).unwrap();
        assert!(info.netgame);
        assert_eq!(
            (info.consoleplayer, info.numnodes, info.numplayers),
            (1, 2, 2)
        );
        // player number out of range
        assert!(i_init_network(&args("-net 3 127.0.0.1:41999 -port 0")).is_err());
        assert!(i_init_network(&args("-net")).is_err());
    }

    #[test]
    fn senders_are_matched_by_ip_and_port() {
        let a = UdpTransport {
            socket: UdpSocket::bind("127.0.0.1:0").unwrap(),
            nodes: vec![
                None,
                Some("127.0.0.1:6001".parse().unwrap()),
                Some("127.0.0.1:6002".parse().unwrap()),
                Some("10.9.8.7:5029".parse().unwrap()),
            ],
        };
        assert_eq!(a.node_for("127.0.0.1:6002".parse().unwrap()), Some(2));
        assert_eq!(a.node_for("127.0.0.1:6001".parse().unwrap()), Some(1));
        // two nodes share the IP and the port is unknown: ambiguous
        assert_eq!(a.node_for("127.0.0.1:9".parse().unwrap()), None);
        // one node with that IP: any source port will do (the original's
        // sockets sent from an ephemeral port)
        assert_eq!(a.node_for("10.9.8.7:33333".parse().unwrap()), Some(3));
        assert_eq!(a.node_for("10.9.8.8:5029".parse().unwrap()), None);
    }

    /// Two machines over real UDP sockets on localhost: the setup
    /// arbitration and then lockstep play, 3 machines' worth of packets
    /// between 2 ports.
    #[test]
    fn two_machines_play_in_lockstep_over_udp() {
        // bind both first so each knows the other's port
        let s0 = UdpSocket::bind("127.0.0.1:0").unwrap();
        let s1 = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (p0, p1) = (
            s0.local_addr().unwrap().port(),
            s1.local_addr().unwrap().port(),
        );
        s0.set_nonblocking(true).unwrap();
        s1.set_nonblocking(true).unwrap();
        let addr = |p| -> SocketAddr { format!("127.0.0.1:{p}").parse().unwrap() };
        let t0 = UdpTransport {
            socket: s0,
            nodes: vec![None, Some(addr(p1))],
        };
        let t1 = UdpTransport {
            socket: s1,
            nodes: vec![None, Some(addr(p0))],
        };
        let start = NetStart {
            skill: 4,
            deathmatch: 1,
            nomonsters: false,
            respawn: true,
            map: 7,
            episode: 3,
        };
        let results = play(vec![Box::new(t0), Box::new(t1)], 70, start);
        assert_eq!(results[1].1, start, "settings travelled over UDP");
        assert_lockstep(&results, 70);
    }
}
