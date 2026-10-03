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
//	DOOM Network game communication and protocol,
//	all OS independend parts.
//
//-----------------------------------------------------------------------------

//! Rust port of `d_net.h` (data structures only — see note below).
//!
//! Networking stuff: network play related data. There is a data struct
//! that stores network communication related stuff, and another one
//! that defines the actual packets to be transmitted.
//!
//! Phase 11b adds the `d_net.c` protocol on top of those shapes:
//! [`NetGame`] holds the original's file-scope state (`nettics[]`,
//! `netcmds[][]`, `maketic`, `resendto[]`, ...) and ports `NetUpdate`,
//! `GetPackets`, `HSendPacket`/`HGetPacket`, `D_ArbitrateNetStart`,
//! `D_CheckNetGame`, `D_QuitNetGame` and `TryRunTics`. The two things the
//! original reaches for through globals/drivers become traits:
//! [`NetTransport`] (`I_NetCmd`'s `CMD_SEND`/`CMD_GET` — the UDP driver,
//! Phase 11c, or an in-memory loopback for tests) and [`NetHost`] (the
//! clock, input polling, and running a game tic).

use crate::d_ticcmd::TicCmd;
use crate::doomstat;
use crate::doomtype::Byte;

/// (`DOOMCOM_ID`)
pub const DOOMCOM_ID: i32 = 0x12345678;

/// Max computers/players in a game (`MAXNETNODES`).
pub const MAXNETNODES: usize = 8;

/// Networking and tick handling related (`BACKUPTICS`).
pub const BACKUPTICS: usize = 12;

/// (`command_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Send = 1,
    Get = 2,
}

/// (`doomdata_t`) — network packet data.
#[derive(Debug, Clone, Copy)]
pub struct DoomData {
    /// High bit is retransmit request.
    pub checksum: u32,
    /// Only valid if NCMD_RETRANSMIT.
    pub retransmitfrom: Byte,
    pub starttic: Byte,
    pub player: Byte,
    pub numtics: Byte,
    pub cmds: [TicCmd; BACKUPTICS],
}

/// (`doomcom_t`)
#[derive(Debug, Clone, Copy)]
pub struct DoomCom {
    /// Supposed to be `DOOMCOM_ID`?
    pub id: i32,
    /// DOOM executes an int to execute commands.
    pub intnum: i16,
    /// Communication between DOOM and the driver. Is `CMD_SEND` or
    /// `CMD_GET`.
    pub command: i16,
    /// Is dest for send, set by get (-1 = no packet).
    pub remotenode: i16,
    /// Number of bytes in doomdata to be sent.
    pub datalength: i16,
    /// Info common to all nodes. Console is always node 0.
    pub numnodes: i16,
    /// Flag: 1 = no duplication, 2-5 = dup for slow nets.
    pub ticdup: i16,
    /// Flag: 1 = send a backup tic in every packet.
    pub extratics: i16,
    /// Flag: 1 = deathmatch.
    pub deathmatch: i16,
    /// Flag: -1 = new game, 0-5 = load savegame.
    pub savegame: i16,
    /// 1-3
    pub episode: i16,
    /// 1-9
    pub map: i16,
    /// 1-5
    pub skill: i16,

    // Info specific to this node.
    pub consoleplayer: i16,
    pub numplayers: i16,

    /// These are related to the 3-display mode, in which two drones
    /// looking left and right were used to render two additional views
    /// on two additional computers. Probably not operational anymore.
    /// 1 = left, 0 = center, -1 = right.
    pub angleoffset: i16,
    /// 1 = drone.
    pub drone: i16,

    /// The packet data to be sent.
    pub data: DoomData,
}

// ---------------------------------------------------------------------
// Phase 11b: the d_net.c protocol
// ---------------------------------------------------------------------

/// Flag bits in `doomdata_t.checksum` (`NCMD_*`).
pub const NCMD_EXIT: u32 = 0x8000_0000;
pub const NCMD_RETRANSMIT: u32 = 0x4000_0000;
pub const NCMD_SETUP: u32 = 0x2000_0000;
/// Kill game.
pub const NCMD_KILL: u32 = 0x1000_0000;
pub const NCMD_CHECKSUM: u32 = 0x0fff_ffff;

/// (`RESENDCOUNT`)
pub const RESENDCOUNT: i32 = 10;
/// Bit flag in `doomdata.player` (`PL_DRONE`).
pub const PL_DRONE: u8 = 0x80;

/// Bytes of a packet before its ticcmds (`checksum` .. `numtics`).
const HEADER_SIZE: usize = 8;
/// Bytes of one ticcmd on the wire (`ticcmd_t`).
const TICCMD_SIZE: usize = 8;

impl Default for DoomData {
    fn default() -> Self {
        DoomData {
            checksum: 0,
            retransmitfrom: 0,
            starttic: 0,
            player: 0,
            numtics: 0,
            cmds: [TicCmd::default(); BACKUPTICS],
        }
    }
}

impl DoomData {
    /// `NetbufferSize`: the bytes a packet with `numtics` ticcmds takes.
    pub fn size(&self) -> usize {
        HEADER_SIZE + TICCMD_SIZE * self.numtics as usize
    }

    /// The packet as sent on the wire: the original's `doomdata_t`
    /// layout with the byte order `i_net.c` gives it (`htonl` on the
    /// checksum, `htons` on each ticcmd's `angleturn`/`consistancy`).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.size());
        out.extend_from_slice(&self.checksum.to_be_bytes());
        out.extend_from_slice(&[
            self.retransmitfrom,
            self.starttic,
            self.player,
            self.numtics,
        ]);
        for c in &self.cmds[..self.numtics as usize] {
            out.push(c.forwardmove as u8);
            out.push(c.sidemove as u8);
            out.extend_from_slice(&c.angleturn.to_be_bytes());
            out.extend_from_slice(&c.consistancy.to_be_bytes());
            out.push(c.chatchar);
            out.push(c.buttons);
        }
        out
    }

    /// Parses a received packet. `None` when it is too short, claims more
    /// than [`BACKUPTICS`] ticcmds, or its length isn't exactly
    /// `NetbufferSize` (HGetPacket's "bad packet length").
    pub fn from_bytes(b: &[u8]) -> Option<DoomData> {
        if b.len() < HEADER_SIZE {
            return None;
        }
        let mut d = DoomData {
            checksum: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            retransmitfrom: b[4],
            starttic: b[5],
            player: b[6],
            numtics: b[7],
            ..Default::default()
        };
        if d.numtics as usize > BACKUPTICS || b.len() != d.size() {
            return None;
        }
        for i in 0..d.numtics as usize {
            let c = &b[HEADER_SIZE + i * TICCMD_SIZE..][..TICCMD_SIZE];
            d.cmds[i] = TicCmd {
                forwardmove: c[0] as i8,
                sidemove: c[1] as i8,
                angleturn: i16::from_be_bytes([c[2], c[3]]),
                consistancy: i16::from_be_bytes([c[4], c[5]]),
                chatchar: c[6],
                buttons: c[7],
            };
        }
        Some(d)
    }
}

/// `NetbufferChecksum`: `0` under `NORMALUNIX` ("byte order problems" —
/// the original's own comment), which is what this build defines.
pub fn netbuffer_checksum(_d: &DoomData) -> u32 {
    0
}

/// The driver half of `I_NetCmd`: `CMD_SEND` and `CMD_GET`. Node 0 is
/// always the local machine (it never goes through the transport — see
/// `HSendPacket`'s rebound); nodes `1..numnodes` are the other machines.
pub trait NetTransport {
    /// `CMD_SEND`: sends `packet` to `node`.
    fn send(&mut self, node: usize, packet: &[u8]);
    /// `CMD_GET`: the next waiting packet and the node it came from, or
    /// `None` (`remotenode == -1`).
    fn recv(&mut self) -> Option<(usize, Vec<u8>)>;
}

/// What the protocol needs from the rest of the game: the clock, input,
/// and running one game tic.
pub trait NetHost {
    /// `I_GetTime`: 35 Hz tics.
    fn get_time(&mut self) -> i32;
    /// `I_StartTic(); D_ProcessEvents(); G_BuildTiccmd(...)`: polls input
    /// and builds the console player's next ticcmd.
    /// `maketic` is the tic the command is for (`G_BuildTiccmd` stamps
    /// it with the consistency value of `maketic % BACKUPTICS`).
    fn build_ticcmd(&mut self, maketic: i32) -> TicCmd;
    /// `gametic`: the tic about to (or currently being) run.
    fn gametic(&self) -> i32;
    /// One iteration of `TryRunTics`' inner loop body: `D_DoAdvanceDemo`
    /// if pending, `M_Ticker`, `G_Ticker` with `cmds` (one per player),
    /// then `gametic++`.
    fn run_tic(&mut self, cmds: &[TicCmd; MAXPLAYERS]);
    /// `M_Ticker`.
    fn menu_ticker(&mut self);
    /// `players[consoleplayer].message = "Player N left the game"`.
    fn player_left(&mut self, player: usize);
    /// Called from busy-wait loops so the host can sleep a moment.
    fn idle(&mut self) {}
    /// `CheckAbort`: true if the user pressed ESC (aborting the sync).
    fn abort_requested(&mut self) -> bool {
        false
    }
}

/// `MAXPLAYERS`, as a `usize` for array sizes.
pub const MAXPLAYERS: usize = crate::doomdef::MAXPLAYERS as usize;

/// What `I_InitNetwork` leaves in `doomcom` for `D_CheckNetGame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetInfo {
    pub netgame: bool,
    /// Machines in the game; node 0 is this one.
    pub numnodes: usize,
    pub numplayers: usize,
    pub consoleplayer: usize,
    /// 1 = no duplication, 2-5 = dup for slow nets.
    pub ticdup: i32,
    /// 1 = send a backup tic in every packet.
    pub extratics: i32,
}

impl NetInfo {
    /// What `I_InitNetwork` sets for a game without `-net`.
    pub fn single_player() -> NetInfo {
        NetInfo {
            netgame: false,
            numnodes: 1,
            numplayers: 1,
            consoleplayer: 0,
            ticdup: 1,
            extratics: 0,
        }
    }
}

/// The game settings the key player (console 0) hands the others in
/// `D_ArbitrateNetStart` (the original's `startskill`, `deathmatch`,
/// `nomonsters`, `respawnparm`, `startmap`, `startepisode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetStart {
    pub skill: i32,
    /// 0 = coop, 1 = deathmatch, 2 = altdeath.
    pub deathmatch: i32,
    pub nomonsters: bool,
    pub respawn: bool,
    pub map: i32,
    pub episode: i32,
}

/// The `d_net.c` state and protocol.
pub struct NetGame {
    transport: Box<dyn NetTransport>,
    info: NetInfo,
    /// `netbuffer` (`doomcom->data`).
    netbuffer: DoomData,
    localcmds: [TicCmd; BACKUPTICS],
    netcmds: [[TicCmd; BACKUPTICS]; MAXPLAYERS],
    nettics: [i32; MAXNETNODES],
    nodeingame: [bool; MAXNETNODES],
    /// Set when local needs tics.
    remoteresend: [bool; MAXNETNODES],
    /// Set when remote needs tics.
    resendto: [i32; MAXNETNODES],
    resendcount: [i32; MAXNETNODES],
    nodeforplayer: [usize; MAXPLAYERS],
    /// `playeringame[]` as the protocol sees it (published to
    /// `doomstat` by [`NetGame::d_check_net_game`]; the host mirrors
    /// leaves through [`NetHost::player_left`]).
    playeringame: [bool; MAXPLAYERS],
    /// The tick that hasn't had control made for it yet.
    maketic: i32,
    skiptics: i32,
    pub ticdup: i32,
    /// `BACKUPTICS/(2*ticdup)-1`.
    pub maxsend: i32,
    gametime: i32,
    singletics: bool,
    reboundpacket: bool,
    reboundstore: DoomData,
    frameon: i32,
    frameskip: [bool; 4],
    oldnettics: i32,
    oldentertics: i32,
}

impl NetGame {
    /// `D_CheckNetGame`'s per-node reset plus what `I_InitNetwork`
    /// decided: call [`NetGame::d_check_net_game`] next.
    pub fn new(info: NetInfo, transport: Box<dyn NetTransport>) -> NetGame {
        NetGame {
            transport,
            info,
            netbuffer: DoomData::default(),
            localcmds: [TicCmd::default(); BACKUPTICS],
            netcmds: [[TicCmd::default(); BACKUPTICS]; MAXPLAYERS],
            nettics: [0; MAXNETNODES],
            nodeingame: [false; MAXNETNODES],
            remoteresend: [false; MAXNETNODES],
            resendto: [0; MAXNETNODES],
            resendcount: [0; MAXNETNODES],
            nodeforplayer: [0; MAXPLAYERS],
            playeringame: [false; MAXPLAYERS],
            maketic: 0,
            skiptics: 0,
            ticdup: info.ticdup,
            maxsend: 1,
            gametime: 0,
            singletics: false,
            reboundpacket: false,
            reboundstore: DoomData::default(),
            frameon: 0,
            frameskip: [false; 4],
            oldnettics: 0,
            oldentertics: 0,
        }
    }

    pub fn info(&self) -> NetInfo {
        self.info
    }

    /// `playeringame[]`.
    pub fn playeringame(&self) -> [bool; MAXPLAYERS] {
        self.playeringame
    }

    /// `maketic`: the next tic to build a command for.
    pub fn maketic(&self) -> i32 {
        self.maketic
    }

    /// `netcmds[player][tic % BACKUPTICS]`.
    pub fn netcmd(&self, player: usize, tic: i32) -> TicCmd {
        self.netcmds[player][(tic as usize) % BACKUPTICS]
    }

    /// `ExpandTics`: to save bytes only the low byte of tic numbers is
    /// sent; works out the rest from `maketic`.
    pub fn expand_tics(&self, low: i32) -> i32 {
        let delta = low - (self.maketic & 0xff);
        if (-64..=64).contains(&delta) {
            (self.maketic & !0xff) + low
        } else if delta > 64 {
            (self.maketic & !0xff) - 256 + low
        } else {
            (self.maketic & !0xff) + 256 + low
        }
    }

    /// `HSendPacket`.
    fn h_send_packet(&mut self, node: usize, flags: u32) {
        self.netbuffer.checksum = netbuffer_checksum(&self.netbuffer) | flags;

        if node == 0 {
            self.reboundstore = self.netbuffer;
            self.reboundpacket = true;
            return;
        }

        if !self.info.netgame {
            panic!("Tried to transmit to another node");
        }
        let bytes = self.netbuffer.to_bytes();
        self.transport.send(node, &bytes);
    }

    /// `HGetPacket`: the next valid packet into `netbuffer`, returning
    /// the node it came from; `None` if none is waiting.
    fn h_get_packet(&mut self) -> Option<usize> {
        if self.reboundpacket {
            self.netbuffer = self.reboundstore;
            self.reboundpacket = false;
            return Some(0);
        }
        if !self.info.netgame {
            return None;
        }
        let (node, bytes) = self.transport.recv()?;
        // bad packet length / bad packet checksum (the original's debug
        // file messages) are dropped silently
        let d = DoomData::from_bytes(&bytes)?;
        if netbuffer_checksum(&d) != (d.checksum & NCMD_CHECKSUM) {
            return None;
        }
        self.netbuffer = d;
        Some(node)
    }

    /// `GetPackets`.
    pub fn get_packets(&mut self, host: &mut dyn NetHost) {
        while let Some(netnode) = self.h_get_packet() {
            if self.netbuffer.checksum & NCMD_SETUP != 0 {
                continue; // extra setup packet
            }

            let netconsole = (self.netbuffer.player & !PL_DRONE) as usize;
            if netconsole >= MAXPLAYERS || netnode >= MAXNETNODES {
                continue; // garbage from the network
            }

            // to save bytes, only the low byte of tic numbers are sent;
            // figure out what the rest of the bytes are
            let realstart = self.expand_tics(self.netbuffer.starttic as i32);
            let realend = realstart + self.netbuffer.numtics as i32;

            // check for exiting the game
            if self.netbuffer.checksum & NCMD_EXIT != 0 {
                if !self.nodeingame[netnode] {
                    continue;
                }
                self.nodeingame[netnode] = false;
                self.playeringame[netconsole] = false;
                host.player_left(netconsole);
                continue;
            }

            // check for a remote game kill
            if self.netbuffer.checksum & NCMD_KILL != 0 {
                panic!("Killed by network driver");
            }

            self.nodeforplayer[netconsole] = netnode;

            // check for retransmit request
            if self.resendcount[netnode] <= 0 && self.netbuffer.checksum & NCMD_RETRANSMIT != 0 {
                self.resendto[netnode] = self.expand_tics(self.netbuffer.retransmitfrom as i32);
                self.resendcount[netnode] = RESENDCOUNT;
            } else {
                self.resendcount[netnode] -= 1;
            }

            // check for out of order / duplicated packet
            if realend == self.nettics[netnode] {
                continue;
            }
            if realend < self.nettics[netnode] {
                continue;
            }

            // check for a missed packet: stop processing until the other
            // system resends the missed tics
            if realstart > self.nettics[netnode] {
                self.remoteresend[netnode] = true;
                continue;
            }

            // update command store from the packet
            self.remoteresend[netnode] = false;
            let start = (self.nettics[netnode] - realstart) as usize;
            let mut src = start;
            while self.nettics[netnode] < realend {
                let dest = (self.nettics[netnode] as usize) % BACKUPTICS;
                self.nettics[netnode] += 1;
                self.netcmds[netconsole][dest] = self.netbuffer.cmds[src];
                src += 1;
            }
        }
    }

    /// `NetUpdate`: builds ticcmds for the console player, sends out a
    /// packet, listens for the others'.
    pub fn net_update(&mut self, host: &mut dyn NetHost) {
        // check time
        let nowtime = host.get_time() / self.ticdup;
        let mut newtics = nowtime - self.gametime;
        self.gametime = nowtime;

        if newtics <= 0 {
            // nothing new to update
            self.get_packets(host);
            return;
        }

        if self.skiptics <= newtics {
            newtics -= self.skiptics;
            self.skiptics = 0;
        } else {
            self.skiptics -= newtics;
            newtics = 0;
        }

        self.netbuffer.player = self.info.consoleplayer as u8;

        // build new ticcmds for console player
        let gameticdiv = host.gametic() / self.ticdup;
        for _ in 0..newtics {
            if self.maketic - gameticdiv >= (BACKUPTICS as i32) / 2 - 1 {
                break; // can't hold any more
            }
            self.localcmds[(self.maketic as usize) % BACKUPTICS] = host.build_ticcmd(self.maketic);
            self.maketic += 1;
        }

        if self.singletics {
            return; // singletic update is syncronous
        }

        // send the packet to the other nodes
        for i in 0..self.info.numnodes {
            if !self.nodeingame[i] {
                continue;
            }
            let realstart = self.resendto[i];
            self.netbuffer.starttic = realstart as u8;
            let numtics = self.maketic - realstart;
            if numtics > BACKUPTICS as i32 {
                panic!("NetUpdate: netbuffer->numtics > BACKUPTICS");
            }
            self.netbuffer.numtics = numtics as u8;

            self.resendto[i] = self.maketic - self.info.extratics;

            for j in 0..numtics as usize {
                self.netbuffer.cmds[j] = self.localcmds[(realstart as usize + j) % BACKUPTICS];
            }

            if self.remoteresend[i] {
                self.netbuffer.retransmitfrom = self.nettics[i] as u8;
                self.h_send_packet(i, NCMD_RETRANSMIT);
            } else {
                self.netbuffer.retransmitfrom = 0;
                self.h_send_packet(i, 0);
            }
        }

        // listen for other packets
        self.get_packets(host);
    }

    /// `D_ArbitrateNetStart`: console 0 (the key player) sends the game
    /// settings to every node and waits until all have answered; the
    /// others wait for them. Returns the settings to play with.
    pub fn d_arbitrate_net_start(&mut self, host: &mut dyn NetHost, mine: NetStart) -> NetStart {
        let mut start = mine;
        if self.info.consoleplayer != 0 {
            // listen for setup info from key player
            loop {
                if host.abort_requested() {
                    panic!("Network game synchronization aborted.");
                }
                if self.h_get_packet().is_none() {
                    host.idle();
                    continue;
                }
                if self.netbuffer.checksum & NCMD_SETUP != 0 {
                    if self.netbuffer.player as i32 != crate::doomdef::VERSION {
                        panic!("Different DOOM versions cannot play a net game!");
                    }
                    start.skill = (self.netbuffer.retransmitfrom & 15) as i32;
                    start.deathmatch = ((self.netbuffer.retransmitfrom & 0xc0) >> 6) as i32;
                    start.nomonsters = self.netbuffer.retransmitfrom & 0x20 != 0;
                    start.respawn = self.netbuffer.retransmitfrom & 0x10 != 0;
                    start.map = (self.netbuffer.starttic & 0x3f) as i32;
                    start.episode = (self.netbuffer.starttic >> 6) as i32;
                    return start;
                }
            }
        }

        // key player, send the setup info
        let mut gotinfo = [false; MAXNETNODES];
        loop {
            if host.abort_requested() {
                panic!("Network game synchronization aborted.");
            }
            for i in 0..self.info.numnodes {
                let mut b = start.skill as u8;
                if start.deathmatch != 0 {
                    b |= (start.deathmatch as u8) << 6;
                }
                if start.nomonsters {
                    b |= 0x20;
                }
                if start.respawn {
                    b |= 0x10;
                }
                self.netbuffer.retransmitfrom = b;
                self.netbuffer.starttic = (start.episode * 64 + start.map) as u8;
                self.netbuffer.player = crate::doomdef::VERSION as u8;
                self.netbuffer.numtics = 0;
                self.h_send_packet(i, NCMD_SETUP);
            }

            let mut budget = 10;
            while budget > 0 && self.h_get_packet().is_some() {
                let n = (self.netbuffer.player & 0x7f) as usize;
                if n < MAXNETNODES {
                    gotinfo[n] = true;
                }
                budget -= 1;
            }

            if (1..self.info.numnodes).all(|i| gotinfo[i]) {
                return start;
            }
            host.idle();
        }
    }

    /// `D_CheckNetGame` without the `doomstat` publishing: resets the
    /// per-node state, runs the start arbitration in a netgame (so the
    /// returned settings are the key player's), and marks the players and
    /// nodes in the game.
    pub fn check_net_game(&mut self, host: &mut dyn NetHost, start: NetStart) -> NetStart {
        for i in 0..MAXNETNODES {
            self.nodeingame[i] = false;
            self.nettics[i] = 0;
            self.remoteresend[i] = false; // set when local needs tics
            self.resendto[i] = 0; // which tic to start sending
        }

        let start = if self.info.netgame {
            self.d_arbitrate_net_start(host, start)
        } else {
            start
        };

        // read values out of doomcom
        self.ticdup = self.info.ticdup;
        self.maxsend = (BACKUPTICS as i32 / (2 * self.ticdup) - 1).max(1);

        for i in 0..self.info.numplayers {
            self.playeringame[i] = true;
        }
        for i in 0..self.info.numnodes {
            self.nodeingame[i] = true;
        }
        start
    }

    /// `D_CheckNetGame`: works out player numbers among the net
    /// participants and publishes the result in `doomstat`. `start` is
    /// replaced by the key player's settings in a netgame.
    pub fn d_check_net_game(&mut self, host: &mut dyn NetHost, start: &mut NetStart) {
        {
            let st = doomstat::state_mut();
            st.netgame = self.info.netgame;
            st.consoleplayer = self.info.consoleplayer as i32;
            st.displayplayer = self.info.consoleplayer as i32;
        }
        *start = self.check_net_game(host, *start);

        let st = doomstat::state_mut();
        st.deathmatch = start.deathmatch != 0;
        st.altdeath = start.deathmatch == 2;
        st.nomonsters = start.nomonsters;
        st.respawnparm = start.respawn;
        st.playeringame = self.playeringame;
        st.ticdup = self.ticdup;
    }

    /// `D_QuitNetGame`: called before quitting to leave a net game
    /// without hanging the other players.
    pub fn d_quit_net_game(&mut self, usergame: bool) {
        if !self.info.netgame || !usergame {
            return;
        }
        // send a bunch of packets for security
        self.netbuffer.player = self.info.consoleplayer as u8;
        self.netbuffer.numtics = 0;
        for _ in 0..4 {
            for j in 1..self.info.numnodes {
                if self.nodeingame[j] {
                    self.h_send_packet(j, NCMD_EXIT);
                }
            }
            crate::i_system::i_wait_vbl(1);
        }
    }

    /// The lowest `nettics` over the nodes in the game, and how many.
    fn lowtic(&self) -> (i32, usize) {
        let mut low = i32::MAX;
        let mut playing = 0;
        for i in 0..self.info.numnodes {
            if self.nodeingame[i] {
                playing += 1;
                low = low.min(self.nettics[i]);
            }
        }
        (low, playing)
    }

    /// `TryRunTics`: runs as many game tics as the network allows (at
    /// least one is attempted every call).
    pub fn try_run_tics(&mut self, host: &mut dyn NetHost) {
        // get real tics
        let entertic = host.get_time() / self.ticdup;
        let realtics = entertic - self.oldentertics;
        self.oldentertics = entertic;

        // get available tics
        self.net_update(host);

        let (mut lowtic, _numplaying) = self.lowtic();
        let availabletics = lowtic - host.gametic() / self.ticdup;

        // decide how many tics to run
        let mut counts = if realtics < availabletics - 1 {
            realtics + 1
        } else if realtics < availabletics {
            realtics
        } else {
            availabletics
        };
        if counts < 1 {
            counts = 1;
        }

        self.frameon += 1;

        // ideally nettics[0] should be 1 - 3 tics above lowtic; if we are
        // consistantly slower, speed up time
        let first = (0..MAXPLAYERS).find(|&i| self.playeringame[i]).unwrap_or(0);
        if self.info.consoleplayer == first {
            // the key player does not adapt
        } else {
            let remote = self.nettics[self.nodeforplayer[first]];
            if self.nettics[0] <= remote {
                self.gametime -= 1;
            }
            self.frameskip[(self.frameon & 3) as usize] = self.oldnettics > remote;
            self.oldnettics = self.nettics[0];
            if self.frameskip.iter().all(|&f| f) {
                self.skiptics = 1;
            }
        }

        // wait for new tics if needed
        while lowtic < host.gametic() / self.ticdup + counts {
            self.net_update(host);
            lowtic = self.lowtic().0;

            if lowtic < host.gametic() / self.ticdup {
                panic!("TryRunTics: lowtic < gametic");
            }

            // don't stay in here forever -- give the menu a chance to
            // work (the original's own comment)
            if host.get_time() / self.ticdup - entertic >= 20 {
                host.menu_ticker();
                return;
            }
            host.idle();
        }

        // run the count * ticdup dics
        while counts > 0 {
            counts -= 1;
            for i in 0..self.ticdup {
                if host.gametic() / self.ticdup > lowtic {
                    panic!("gametic>lowtic");
                }
                let buf = ((host.gametic() / self.ticdup) as usize) % BACKUPTICS;
                let mut cmds = [TicCmd::default(); MAXPLAYERS];
                for (j, c) in cmds.iter_mut().enumerate() {
                    *c = self.netcmds[j][buf];
                }
                host.run_tic(&cmds);

                // modify command for duplicated tics
                if i != self.ticdup - 1 {
                    let buf = ((host.gametic() / self.ticdup) as usize) % BACKUPTICS;
                    for j in 0..MAXPLAYERS {
                        let cmd = &mut self.netcmds[j][buf];
                        cmd.chatchar = 0;
                        if cmd.buttons & (crate::d_event::BT_SPECIAL as u8) != 0 {
                            cmd.buttons = 0;
                        }
                    }
                }
            }
            self.net_update(host); // check for new console commands
        }
    }
}

/// The transport of a game without `-net`: there is nobody to talk to
/// (node 0 is local and never goes through a transport).
pub struct NullTransport;

impl NetTransport for NullTransport {
    fn send(&mut self, _node: usize, _packet: &[u8]) {}
    fn recv(&mut self) -> Option<(usize, Vec<u8>)> {
        None
    }
}

/// An in-memory [`NetTransport`] between machines in one process (each
/// on its own thread): what the tests use in place of UDP.
pub struct LoopbackTransport {
    me: usize,
    machines: usize,
    peers: Vec<std::sync::mpsc::Sender<(usize, Vec<u8>)>>,
    rx: std::sync::mpsc::Receiver<(usize, Vec<u8>)>,
}

/// `n` machines fully connected; machine `k` numbers the others as nodes
/// `1..n` (node `j` is machine `(k + j) % n`), as every machine's node 0
/// is itself.
pub fn loopback_mesh(n: usize) -> Vec<LoopbackTransport> {
    let (txs, rxs): (Vec<_>, Vec<_>) = (0..n).map(|_| std::sync::mpsc::channel()).unzip();
    rxs.into_iter()
        .enumerate()
        .map(|(me, rx)| LoopbackTransport {
            me,
            machines: n,
            peers: txs.clone(),
            rx,
        })
        .collect()
}

impl NetTransport for LoopbackTransport {
    fn send(&mut self, node: usize, packet: &[u8]) {
        let target = (self.me + node) % self.machines;
        // the sender is node `(me - target) mod n` in the target's numbering
        let from = (self.me + self.machines - target) % self.machines;
        let _ = self.peers[target].send((from, packet.to_vec()));
    }

    fn recv(&mut self) -> Option<(usize, Vec<u8>)> {
        self.rx.try_recv().ok()
    }
}

/// Shared by the protocol tests here and the UDP driver's (`i_net`).
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A host with an (accelerated) real-time clock that records every
    /// tic it is asked to run.
    pub(crate) struct TestHost {
        pub(crate) start: std::time::Instant,
        pub(crate) rate: i64,
        pub(crate) me: usize,
        pub(crate) gametic: i32,
        pub(crate) built: i32,
        pub(crate) log: Vec<[TicCmd; MAXPLAYERS]>,
        pub(crate) left: Vec<usize>,
    }

    impl TestHost {
        pub(crate) fn new(me: usize, rate: i64) -> TestHost {
            TestHost {
                start: std::time::Instant::now(),
                rate,
                me,
                gametic: 0,
                built: 0,
                log: Vec::new(),
                left: Vec::new(),
            }
        }
    }

    impl NetHost for TestHost {
        fn get_time(&mut self) -> i32 {
            (self.start.elapsed().as_micros() as i64 * self.rate / 1_000_000) as i32
        }
        fn build_ticcmd(&mut self, _maketic: i32) -> TicCmd {
            self.built += 1;
            TicCmd {
                forwardmove: ((self.me as i32 + 1) * 10 + self.built % 7) as i8,
                buttons: (self.built % 3) as u8,
                ..Default::default()
            }
        }
        fn gametic(&self) -> i32 {
            self.gametic
        }
        fn run_tic(&mut self, cmds: &[TicCmd; MAXPLAYERS]) {
            self.log.push(*cmds);
            self.gametic += 1;
        }
        fn menu_ticker(&mut self) {}
        fn player_left(&mut self, player: usize) {
            self.left.push(player);
        }
        fn idle(&mut self) {
            std::thread::sleep(std::time::Duration::from_micros(50));
        }
    }

    /// Runs `n` machines for `tics` tics each (a thread per machine) over
    /// `transports`, returning each machine's recorded tics and the
    /// settings it ended up with.
    pub(crate) fn play(
        transports: Vec<Box<dyn NetTransport + Send>>,
        tics: i32,
        key_start: NetStart,
    ) -> Vec<(Vec<[TicCmd; MAXPLAYERS]>, NetStart)> {
        let n = transports.len();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handles: Vec<_> = transports
            .into_iter()
            .enumerate()
            .map(|(k, t)| {
                let done = done.clone();
                std::thread::spawn(move || {
                    let info = NetInfo {
                        netgame: true,
                        numnodes: n,
                        numplayers: n,
                        consoleplayer: k,
                        ticdup: 1,
                        extratics: 1,
                    };
                    let mut g = NetGame::new(info, t);
                    let mut host = TestHost::new(k, 700);
                    let got = g.check_net_game(
                        &mut host,
                        if k == 0 {
                            key_start
                        } else {
                            NetStart::default()
                        },
                    );
                    while host.gametic < tics {
                        g.try_run_tics(&mut host);
                    }
                    // keep answering retransmit requests until every
                    // machine has finished (a real game never stops
                    // talking mid-level)
                    done.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    while done.load(std::sync::atomic::Ordering::SeqCst) < n {
                        g.net_update(&mut host);
                        host.idle();
                    }
                    (host.log, got)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    }

    pub(crate) fn assert_lockstep(results: &[(Vec<[TicCmd; MAXPLAYERS]>, NetStart)], tics: usize) {
        let first = &results[0].0;
        for (k, (log, _)) in results.iter().enumerate() {
            assert!(log.len() >= tics, "machine {k} ran {} tics", log.len());
            for t in 0..tics {
                assert_eq!(log[t], first[t], "machine {k} diverged at tic {t}");
            }
        }
        // each player's commands in the shared log are the ones that
        // player's machine built (`build_ticcmd`: 10*(player+1) + n % 7)
        for (p, _) in results.iter().enumerate() {
            for (t, cmds) in first.iter().take(tics).enumerate() {
                assert_eq!(
                    cmds[p].forwardmove as i32,
                    (p as i32 + 1) * 10 + (t as i32 + 1) % 7,
                    "player {p} tic {t}"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn constants_match_original() {
        assert_eq!(DOOMCOM_ID, 0x12345678);
        assert_eq!(MAXNETNODES, 8);
        assert_eq!(BACKUPTICS, 12);
    }

    #[test]
    fn command_variants_match_original_values() {
        assert_eq!(Command::Send as i32, 1);
        assert_eq!(Command::Get as i32, 2);
    }

    #[test]
    fn doomdata_cmds_array_is_backuptics_long() {
        let d = DoomData {
            checksum: 0,
            retransmitfrom: 0,
            starttic: 0,
            player: 0,
            numtics: 0,
            cmds: [TicCmd::default(); BACKUPTICS],
        };
        assert_eq!(d.cmds.len(), 12);
    }

    // ---- Phase 11b ----

    #[test]
    fn packets_round_trip_in_the_original_wire_format() {
        let mut d = DoomData {
            checksum: NCMD_RETRANSMIT,
            retransmitfrom: 7,
            starttic: 200,
            player: 2,
            numtics: 2,
            ..Default::default()
        };
        d.cmds[0] = TicCmd {
            forwardmove: -25,
            sidemove: 40,
            angleturn: -1234,
            consistancy: 0x1234,
            chatchar: b'x',
            buttons: 3,
        };
        d.cmds[1].forwardmove = 50;
        let bytes = d.to_bytes();
        assert_eq!(bytes.len(), 8 + 2 * 8, "NetbufferSize");
        assert_eq!(&bytes[..4], &[0x40, 0, 0, 0], "checksum is big-endian");
        assert_eq!(&bytes[10..12], &(-1234i16).to_be_bytes(), "angleturn BE");
        let back = DoomData::from_bytes(&bytes).unwrap();
        assert_eq!(back.checksum, d.checksum);
        assert_eq!(back.cmds[..2], d.cmds[..2]);
        assert_eq!(
            (
                back.retransmitfrom,
                back.starttic,
                back.player,
                back.numtics
            ),
            (7, 200, 2, 2)
        );
    }

    #[test]
    fn malformed_packets_are_rejected() {
        let d = DoomData {
            numtics: 1,
            ..Default::default()
        };
        let good = d.to_bytes();
        assert!(DoomData::from_bytes(&good).is_some());
        assert!(
            DoomData::from_bytes(&good[..good.len() - 1]).is_none(),
            "short"
        );
        let mut long = good.clone();
        long.push(0);
        assert!(DoomData::from_bytes(&long).is_none(), "long");
        assert!(DoomData::from_bytes(&[0; 4]).is_none(), "no header");
        let mut many = good;
        many[7] = (BACKUPTICS + 1) as u8;
        assert!(DoomData::from_bytes(&many).is_none(), "too many tics");
    }

    /// Drops every `n`th packet it is asked to send.
    struct Lossy {
        inner: LoopbackTransport,
        n: usize,
        sent: usize,
    }
    impl NetTransport for Lossy {
        fn send(&mut self, node: usize, packet: &[u8]) {
            self.sent += 1;
            if !self.sent.is_multiple_of(self.n) {
                self.inner.send(node, packet);
            }
        }
        fn recv(&mut self) -> Option<(usize, Vec<u8>)> {
            self.inner.recv()
        }
    }

    #[test]
    fn expand_tics_recovers_the_high_bytes() {
        let mut g = NetGame::new(
            NetInfo::single_player(),
            Box::new(loopback_mesh(1).remove(0)),
        );
        g.maketic = 300; // low byte 44
        assert_eq!(g.expand_tics(50), 256 + 50);
        assert_eq!(g.expand_tics(10), 256 + 10);
        g.maketic = 250;
        assert_eq!(g.expand_tics(2), 256 + 2, "wrapped past 255");
        g.maketic = 258;
        assert_eq!(g.expand_tics(255), 255, "slightly behind, before the wrap");
    }

    #[test]
    fn single_player_runs_each_tic_with_its_own_command() {
        let mut g = NetGame::new(
            NetInfo::single_player(),
            Box::new(loopback_mesh(1).remove(0)),
        );
        let mut host = TestHost::new(0, 10_000); // fast clock
        let start = NetStart::default();
        g.check_net_game(&mut host, start);
        assert_eq!(g.playeringame(), [true, false, false, false]);
        for _ in 0..50 {
            g.try_run_tics(&mut host);
        }
        assert!(host.gametic >= 50, "at least one tic per call");
        // every tic ran with the console player's own built command
        for (t, cmds) in host.log.iter().enumerate() {
            assert_eq!(
                cmds[0].forwardmove as i32,
                10 + (t as i32 + 1) % 7,
                "tic {t}"
            );
        }
    }

    #[test]
    fn two_machines_agree_on_the_settings_and_run_in_lockstep() {
        let start = NetStart {
            skill: 3,
            deathmatch: 2,
            nomonsters: true,
            respawn: true,
            map: 5,
            episode: 2,
        };
        let ts: Vec<Box<dyn NetTransport + Send>> = loopback_mesh(2)
            .into_iter()
            .map(|t| Box::new(t) as Box<dyn NetTransport + Send>)
            .collect();
        let results = play(ts, 80, start);
        assert_eq!(
            results[1].1, start,
            "the other machine got the key player's settings"
        );
        assert_lockstep(&results, 80);
    }

    #[test]
    fn three_machines_stay_in_lockstep() {
        let ts: Vec<Box<dyn NetTransport + Send>> = loopback_mesh(3)
            .into_iter()
            .map(|t| Box::new(t) as Box<dyn NetTransport + Send>)
            .collect();
        let results = play(ts, 60, NetStart::default());
        assert_lockstep(&results, 60);
    }

    #[test]
    fn lost_packets_are_recovered_by_retransmission() {
        let ts: Vec<Box<dyn NetTransport + Send>> = loopback_mesh(2)
            .into_iter()
            .map(|t| {
                Box::new(Lossy {
                    inner: t,
                    n: 3,
                    sent: 0,
                }) as Box<dyn NetTransport + Send>
            })
            .collect();
        // arbitration also loses packets, but it re-sends until answered
        let results = play(ts, 80, NetStart::default());
        assert_lockstep(&results, 80);
    }

    #[test]
    fn a_leaving_player_is_dropped_from_the_game() {
        let mut ts = loopback_mesh(2);
        let t1 = ts.remove(1);
        let t0 = ts.remove(0);
        let info = |k| NetInfo {
            netgame: true,
            numnodes: 2,
            numplayers: 2,
            consoleplayer: k,
            ticdup: 1,
            extratics: 1,
        };
        let mut a = NetGame::new(info(0), Box::new(t0));
        let mut b = NetGame::new(info(1), Box::new(t1));
        // no arbitration here: mark everyone in the game by hand
        for g in [&mut a, &mut b] {
            g.playeringame = [true, true, false, false];
            g.nodeingame[0] = true;
            g.nodeingame[1] = true;
        }
        let mut ha = TestHost::new(0, 700);
        usergame_quit(&mut b);
        a.get_packets(&mut ha);
        assert_eq!(ha.left, vec![1]);
        assert_eq!(a.playeringame(), [true, false, false, false]);
        assert!(!a.nodeingame[1]);
    }

    fn usergame_quit(g: &mut NetGame) {
        g.d_quit_net_game(true);
    }
}
