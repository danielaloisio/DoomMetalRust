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
//	Archiving: SaveGame I/O.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_saveg.c`: archiving the game state to/from a byte buffer
//! (savegames).
//!
//! # Format divergence
//!
//! The original `memcpy`s its C structs (`player_t`, `mobj_t`,
//! `ceiling_t`, ...) into the file, with pointers swizzled to indices —
//! a layout that depends on the compiler, word size and padding
//! (`PADSAVEP` exists just for that), and on a 64-bit build is not even
//! the DOS file format. This port writes the same *sections in the same
//! order with the same semantic content*, but field by field in
//! little-endian with no padding, so the format is stable and
//! documented here. Original `.dsg` files are therefore not readable
//! (and neither are they by any 64-bit build of the original).
//!
//! Sections (after the header written by `g_game`): players
//! (`P_ArchivePlayers`), the world (`P_ArchiveWorld`: sectors and lines
//! as 16-bit values, heights `>> FRACBITS` — so fractional floor/ceiling
//! heights and texture offsets are lost, like the original), the mobj
//! thinkers (`tc_mobj` records ended by `tc_end`) and the special
//! thinkers (`tc_ceiling`..`tc_glow` ended by `tc_endspecials`).
//!
//! # Preserved quirks
//!
//! * `T_FireFlicker` is not archived (the original's `P_ArchiveSpecials`
//!   has no case for it): flickering lights freeze at their saved level
//!   after a load.
//! * A platform in stasis (`function == NULL`) is not archived either:
//!   the original's NULL-function branch only looks in `activeceilings`.
//!   A ceiling in stasis is, with its stasis state.
//! * `mobj.target` comes back `None` (as in the original); so do
//!   `player.attacker` and `player.message`.
//!
//! # Other differences
//!
//! * `mobj.tracer` also comes back `None` (the original keeps a stale
//!   pointer from the previous session).
//! * `sprite` is restored from the state table (`states[state].sprite`)
//!   rather than stored; it is always equal to it.
//! * Malformed data returns `Err(SaveError)`; the original calls
//!   `I_Error`.

use crate::d_player::{Player, PlayerState};
use crate::doomdata::MapThing;
use crate::doomdef::{WeaponType, MAXPLAYERS};
use crate::info::{MobjType, StateNum, MOBJINFO, STATES};
use crate::m_fixed::FRACBITS;
use crate::p_ceilng::{ActiveCeilings, Ceiling, CeilingType};
use crate::p_doors::{VlDoor, VlDoorType};
use crate::p_floor::{FloorMove, FloorType};
use crate::p_lights::{Glow, LightFlash, Strobe};
use crate::p_plats::{ActivePlats, Plat, PlatStatus, PlatType};
use crate::p_pspr::PSpr;
use crate::p_setup::Level;
use crate::p_tick::{ThinkFn, ThinkerData, Thinkers};
use crate::r_defs::Mobj;

/// A savegame that can't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveError(pub String);

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, SaveError> {
    Err(SaveError(msg.into()))
}

/// `thinkerclass_t`.
const TC_END: u8 = 0;
const TC_MOBJ: u8 = 1;

/// `specials_e`.
const TC_CEILING: u8 = 0;
const TC_DOOR: u8 = 1;
const TC_FLOOR: u8 = 2;
const TC_PLAT: u8 = 3;
const TC_FLASH: u8 = 4;
const TC_STROBE: u8 = 5;
const TC_GLOW: u8 = 6;
const TC_ENDSPECIALS: u8 = 7;

/// Little-endian byte sink (`save_p`, writing).
#[derive(Default)]
pub struct SaveWriter {
    pub buf: Vec<u8>,
}

impl SaveWriter {
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    pub fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }
}

/// Little-endian byte source (`save_p`, reading).
pub struct SaveReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SaveReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        SaveReader { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], SaveError> {
        let end = self.pos + n;
        if end > self.data.len() {
            return err("savegame is truncated");
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, SaveError> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool, SaveError> {
        Ok(self.u8()? != 0)
    }
    pub fn i16(&mut self) -> Result<i16, SaveError> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32, SaveError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, SaveError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], SaveError> {
        self.take(n)
    }
    /// Bytes not yet read.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

fn enum_from<T>(v: i32, from: impl Fn(usize) -> Option<T>, what: &str) -> Result<T, SaveError> {
    usize::try_from(v)
        .ok()
        .and_then(from)
        .map_or_else(|| err(format!("bad {what} {v} in savegame")), Ok)
}

// ---------------------------------------------------------------- players

/// Port of `P_ArchivePlayers`.
pub fn p_archive_players(w: &mut SaveWriter, players: &[Player], playeringame: &[bool]) {
    for (i, player) in players.iter().enumerate() {
        if !playeringame.get(i).copied().unwrap_or(false) {
            continue;
        }
        let p = player;
        w.u8(match p.playerstate {
            PlayerState::Live => 0,
            PlayerState::Dead => 1,
            PlayerState::Reborn => 2,
        });
        for v in [p.viewz, p.viewheight, p.deltaviewheight, p.bob] {
            w.i32(v);
        }
        for v in [p.health, p.armorpoints, p.armortype] {
            w.i32(v);
        }
        p.powers.iter().for_each(|&v| w.i32(v));
        p.cards.iter().for_each(|&v| w.bool(v));
        w.bool(p.backpack);
        p.frags.iter().for_each(|&v| w.i32(v));
        w.i32(p.readyweapon as i32);
        w.i32(p.pendingweapon as i32);
        p.weaponowned.iter().for_each(|&v| w.bool(v));
        p.ammo.iter().for_each(|&v| w.i32(v));
        p.maxammo.iter().for_each(|&v| w.i32(v));
        w.bool(p.attackdown);
        w.bool(p.usedown);
        for v in [
            p.cheats,
            p.refire,
            p.killcount,
            p.itemcount,
            p.secretcount,
            p.damagecount,
            p.bonuscount,
            p.extralight,
            p.fixedcolormap,
            p.colormap,
        ] {
            w.i32(v);
        }
        for psp in &p.psprites {
            // state pointer -> index (-1 for none)
            w.i32(psp.state.map_or(-1, |s| s as i32));
            w.i32(psp.tics);
            w.i32(psp.sx);
            w.i32(psp.sy);
        }
        w.bool(p.didsecret);
    }
}

/// Port of `P_UnArchivePlayers`. `mo`, `message` and `attacker` are
/// reset (`mo` is set when the mobj thinkers are read back).
pub fn p_unarchive_players(
    r: &mut SaveReader,
    players: &mut [Player],
    playeringame: &[bool],
) -> Result<(), SaveError> {
    for (i, p) in players.iter_mut().enumerate() {
        if !playeringame.get(i).copied().unwrap_or(false) {
            continue;
        }
        p.playerstate = match r.u8()? {
            0 => PlayerState::Live,
            1 => PlayerState::Dead,
            2 => PlayerState::Reborn,
            n => return err(format!("bad player state {n} in savegame")),
        };
        p.viewz = r.i32()?;
        p.viewheight = r.i32()?;
        p.deltaviewheight = r.i32()?;
        p.bob = r.i32()?;
        p.health = r.i32()?;
        p.armorpoints = r.i32()?;
        p.armortype = r.i32()?;
        for v in p.powers.iter_mut() {
            *v = r.i32()?;
        }
        for v in p.cards.iter_mut() {
            *v = r.bool()?;
        }
        p.backpack = r.bool()?;
        for v in p.frags.iter_mut() {
            *v = r.i32()?;
        }
        p.readyweapon = enum_from(r.i32()?, WeaponType::from_index, "weapon")?;
        p.pendingweapon = enum_from(r.i32()?, WeaponType::from_index, "weapon")?;
        for v in p.weaponowned.iter_mut() {
            *v = r.bool()?;
        }
        for v in p.ammo.iter_mut() {
            *v = r.i32()?;
        }
        for v in p.maxammo.iter_mut() {
            *v = r.i32()?;
        }
        p.attackdown = r.bool()?;
        p.usedown = r.bool()?;
        p.cheats = r.i32()?;
        p.refire = r.i32()?;
        p.killcount = r.i32()?;
        p.itemcount = r.i32()?;
        p.secretcount = r.i32()?;
        p.damagecount = r.i32()?;
        p.bonuscount = r.i32()?;
        p.extralight = r.i32()?;
        p.fixedcolormap = r.i32()?;
        p.colormap = r.i32()?;
        for psp in p.psprites.iter_mut() {
            let s = r.i32()?;
            let state = if s < 0 {
                None
            } else {
                Some(enum_from(s, StateNum::from_index, "state")?)
            };
            *psp = PSpr {
                state,
                tics: r.i32()?,
                sx: r.i32()?,
                sy: r.i32()?,
            };
        }
        p.didsecret = r.bool()?;

        // will be set when unarc thinker
        p.message = None;
        p.attacker = None;
        p.pending_pspr.clear();
    }
    Ok(())
}

// ------------------------------------------------------------------ world

/// Port of `P_ArchiveWorld`.
pub fn p_archive_world(w: &mut SaveWriter, level: &Level) {
    // do sectors
    for sec in &level.sectors {
        w.i16((sec.floorheight >> FRACBITS) as i16);
        w.i16((sec.ceilingheight >> FRACBITS) as i16);
        w.i16(sec.floorpic);
        w.i16(sec.ceilingpic);
        w.i16(sec.lightlevel);
        w.i16(sec.special); // needed?
        w.i16(sec.tag); // needed?
    }

    // do lines
    for li in &level.lines {
        w.i16(li.flags);
        w.i16(li.special);
        w.i16(li.tag);
        for side in li.sidenum {
            let Some(side) = side else { continue };
            let si = &level.sides[side];
            w.i16((si.textureoffset >> FRACBITS) as i16);
            w.i16((si.rowoffset >> FRACBITS) as i16);
            w.i16(si.toptexture);
            w.i16(si.bottomtexture);
            w.i16(si.midtexture);
        }
    }
}

/// Port of `P_UnArchiveWorld`.
pub fn p_unarchive_world(r: &mut SaveReader, level: &mut Level) -> Result<(), SaveError> {
    for sec in level.sectors.iter_mut() {
        sec.floorheight = (r.i16()? as i32) << FRACBITS;
        sec.ceilingheight = (r.i16()? as i32) << FRACBITS;
        sec.floorpic = r.i16()?;
        sec.ceilingpic = r.i16()?;
        sec.lightlevel = r.i16()?;
        sec.special = r.i16()?;
        sec.tag = r.i16()?;
        sec.specialdata = None;
        sec.soundtarget = None;
    }

    for i in 0..level.lines.len() {
        let li = &mut level.lines[i];
        li.flags = r.i16()?;
        li.special = r.i16()?;
        li.tag = r.i16()?;
        let sidenum = li.sidenum;
        for side in sidenum {
            let Some(side) = side else { continue };
            let si = &mut level.sides[side];
            si.textureoffset = (r.i16()? as i32) << FRACBITS;
            si.rowoffset = (r.i16()? as i32) << FRACBITS;
            si.toptexture = r.i16()?;
            si.bottomtexture = r.i16()?;
            si.midtexture = r.i16()?;
        }
    }
    Ok(())
}

// -------------------------------------------------------------- thinkers

/// Port of `P_ArchiveThinkers` (the mobjs).
pub fn p_archive_thinkers(w: &mut SaveWriter, thinkers: &Thinkers) {
    for (_, function, data) in thinkers.iter_list() {
        let (ThinkFn::MobjThinker, ThinkerData::Mobj(m)) = (function, data) else {
            // I_Error ("P_ArchiveThinkers: Unknown thinker function");
            // (commented out in the original: other kinds are skipped)
            continue;
        };
        w.u8(TC_MOBJ);
        for v in [m.x, m.y, m.z] {
            w.i32(v);
        }
        w.u32(m.angle);
        w.i32(m.frame);
        for v in [
            m.floorz, m.ceilingz, m.radius, m.height, m.momx, m.momy, m.momz,
        ] {
            w.i32(v);
        }
        w.i32(m.validcount);
        w.i32(m.mobj_type as i32);
        w.i32(m.tics);
        w.i32(m.state as i32);
        w.i32(m.flags);
        w.i32(m.health);
        w.i32(m.movedir);
        w.i32(m.movecount);
        w.i32(m.reactiontime);
        w.i32(m.threshold);
        // player pointer -> index + 1
        w.i32(m.player.map_or(0, |p| p as i32 + 1));
        w.i32(m.lastlook);
        for v in [
            m.spawnpoint.x,
            m.spawnpoint.y,
            m.spawnpoint.angle,
            m.spawnpoint.thing_type,
            m.spawnpoint.options,
        ] {
            w.i16(v);
        }
    }
    // add a terminating marker
    w.u8(TC_END);
}

/// Port of `P_UnArchiveThinkers`: removes every current thinker, then
/// reads the saved mobjs back in, relinking them into the world and
/// their players.
pub fn p_unarchive_thinkers(
    r: &mut SaveReader,
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
) -> Result<(), SaveError> {
    // remove all the current thinkers
    let ids: Vec<_> = thinkers.iter_list().map(|(id, f, _)| (id, f)).collect();
    for (id, function) in ids {
        if function == ThinkFn::MobjThinker {
            crate::p_mobj::p_remove_mobj(thinkers, level, id);
        }
    }
    *thinkers = Thinkers::new(); // P_InitThinkers

    // read in saved thinkers
    loop {
        match r.u8()? {
            TC_END => return Ok(()),
            TC_MOBJ => {
                let (x, y, z) = (r.i32()?, r.i32()?, r.i32()?);
                let angle = r.u32()?;
                let frame = r.i32()?;
                let (floorz, ceilingz, radius, height) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
                let (momx, momy, momz) = (r.i32()?, r.i32()?, r.i32()?);
                let validcount = r.i32()?;
                let mobj_type = enum_from(r.i32()?, MobjType::from_index, "mobj type")?;
                let tics = r.i32()?;
                let state = enum_from(r.i32()?, StateNum::from_index, "state")?;
                let flags = r.i32()?;
                let health = r.i32()?;
                let movedir = r.i32()?;
                let movecount = r.i32()?;
                let reactiontime = r.i32()?;
                let threshold = r.i32()?;
                let player = r.i32()?;
                let lastlook = r.i32()?;
                let spawnpoint = MapThing {
                    x: r.i16()?,
                    y: r.i16()?,
                    angle: r.i16()?,
                    thing_type: r.i16()?,
                    options: r.i16()?,
                };

                let mut mobj = Mobj::blank(mobj_type);
                mobj.x = x;
                mobj.y = y;
                mobj.z = z;
                mobj.angle = angle;
                mobj.sprite = STATES[state as usize].sprite;
                mobj.frame = frame;
                mobj.floorz = floorz;
                mobj.ceilingz = ceilingz;
                mobj.radius = radius;
                mobj.height = height;
                mobj.momx = momx;
                mobj.momy = momy;
                mobj.momz = momz;
                mobj.validcount = validcount;
                mobj.info = &MOBJINFO[mobj_type as usize];
                mobj.tics = tics;
                mobj.state = state;
                mobj.flags = flags;
                mobj.health = health;
                mobj.movedir = movedir;
                mobj.movecount = movecount;
                mobj.reactiontime = reactiontime;
                mobj.threshold = threshold;
                mobj.lastlook = lastlook;
                mobj.spawnpoint = spawnpoint;
                mobj.target = None;
                mobj.player = None;

                let id = thinkers.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(mobj));
                if player != 0 {
                    let idx = (player - 1) as usize;
                    let Some(p) = players.get_mut(idx) else {
                        return err(format!("bad player {player} in savegame"));
                    };
                    thinkers.mobj_mut(id).unwrap().player = Some(idx);
                    p.mo = id;
                }
                crate::p_mobj::p_set_thing_position(thinkers, level, id);
                let m = thinkers.mobj_mut(id).unwrap();
                let sector = &level.sectors[level.subsectors[m.subsector.unwrap()].sector];
                m.floorz = sector.floorheight;
                m.ceilingz = sector.ceilingheight;
            }
            n => return err(format!("Unknown tclass {n} in savegame")),
        }
    }
}

// -------------------------------------------------------------- specials

/// Port of `P_ArchiveSpecials`.
pub fn p_archive_specials(
    w: &mut SaveWriter,
    thinkers: &Thinkers,
    active_ceilings: &ActiveCeilings,
) {
    for (id, function, data) in thinkers.iter_list() {
        if function == ThinkFn::Null {
            // In stasis: only ceilings on the active list are saved (the
            // original's NULL-function branch only scans `activeceilings`).
            if let ThinkerData::Ceiling(c) = data {
                if active_ceilings.contains(id) {
                    w.u8(TC_CEILING);
                    write_ceiling(w, c, false);
                }
            }
            continue;
        }
        match (function, data) {
            (ThinkFn::MoveCeiling, ThinkerData::Ceiling(c)) => {
                w.u8(TC_CEILING);
                write_ceiling(w, c, true);
            }
            (ThinkFn::VerticalDoor, ThinkerData::VlDoor(d)) => {
                w.u8(TC_DOOR);
                w.i32(d.door_type as i32);
                w.i32(d.sector as i32);
                w.i32(d.topheight);
                w.i32(d.speed);
                w.i32(d.direction);
                w.i32(d.topwait);
                w.i32(d.topcountdown);
            }
            (ThinkFn::FloorMove, ThinkerData::FloorMove(f)) => {
                w.u8(TC_FLOOR);
                w.i32(f.floor_type as i32);
                w.bool(f.crush);
                w.i32(f.sector as i32);
                w.i32(f.direction);
                w.i16(f.newspecial);
                w.i16(f.texture);
                w.i32(f.floordestheight);
                w.i32(f.speed);
            }
            (ThinkFn::PlatRaise, ThinkerData::Plat(p)) => {
                w.u8(TC_PLAT);
                w.i32(p.sector as i32);
                w.i32(p.speed);
                w.i32(p.low);
                w.i32(p.high);
                w.i32(p.wait);
                w.i32(p.count);
                w.i32(p.status as i32);
                w.i32(p.oldstatus as i32);
                w.bool(p.crush);
                w.i16(p.tag);
                w.i32(p.plat_type as i32);
            }
            (ThinkFn::LightFlash, ThinkerData::LightFlash(f)) => {
                w.u8(TC_FLASH);
                for v in [
                    f.sector as i32,
                    f.count,
                    f.maxlight,
                    f.minlight,
                    f.maxtime,
                    f.mintime,
                ] {
                    w.i32(v);
                }
            }
            (ThinkFn::StrobeFlash, ThinkerData::StrobeFlash(s)) => {
                w.u8(TC_STROBE);
                for v in [
                    s.sector as i32,
                    s.count,
                    s.minlight,
                    s.maxlight,
                    s.darktime,
                    s.brighttime,
                ] {
                    w.i32(v);
                }
            }
            (ThinkFn::Glow, ThinkerData::Glow(g)) => {
                w.u8(TC_GLOW);
                for v in [g.sector as i32, g.minlight, g.maxlight, g.direction] {
                    w.i32(v);
                }
            }
            // FireFlicker: not archived (see module docs); mobjs were
            // saved by P_ArchiveThinkers.
            _ => {}
        }
    }
    // add a terminating marker
    w.u8(TC_ENDSPECIALS);
}

fn write_ceiling(w: &mut SaveWriter, c: &Ceiling, active: bool) {
    w.bool(active); // `thinker.function != NULL`
    w.i32(c.ceiling_type as i32);
    w.i32(c.sector as i32);
    w.i32(c.bottomheight);
    w.i32(c.topheight);
    w.i32(c.speed);
    w.bool(c.crush);
    w.i32(c.direction);
    w.i16(c.tag);
    w.i32(c.olddirection);
}

fn read_sector(r: &mut SaveReader, level: &Level) -> Result<usize, SaveError> {
    let s = r.i32()?;
    if s < 0 || s as usize >= level.sectors.len() {
        return err(format!("bad sector {s} in savegame"));
    }
    Ok(s as usize)
}

/// Port of `P_UnArchiveSpecials`.
pub fn p_unarchive_specials(
    r: &mut SaveReader,
    thinkers: &mut Thinkers,
    level: &mut Level,
    active_plats: &mut ActivePlats,
    active_ceilings: &mut ActiveCeilings,
) -> Result<(), SaveError> {
    loop {
        match r.u8()? {
            TC_ENDSPECIALS => return Ok(()),
            TC_CEILING => {
                let active = r.bool()?;
                let ceiling_type = enum_from(r.i32()?, CeilingType::from_index, "ceiling type")?;
                let sector = read_sector(r, level)?;
                let c = Ceiling {
                    ceiling_type,
                    sector,
                    bottomheight: r.i32()?,
                    topheight: r.i32()?,
                    speed: r.i32()?,
                    crush: r.bool()?,
                    direction: r.i32()?,
                    tag: r.i16()?,
                    olddirection: r.i32()?,
                };
                let function = if active {
                    ThinkFn::MoveCeiling
                } else {
                    ThinkFn::Null
                };
                let id = thinkers.add_thinker(function, ThinkerData::Ceiling(c));
                level.sectors[sector].specialdata = Some(id);
                active_ceilings.p_add_active_ceiling(id);
            }
            TC_DOOR => {
                let door_type = enum_from(r.i32()?, VlDoorType::from_index, "door type")?;
                let sector = read_sector(r, level)?;
                let d = VlDoor {
                    door_type,
                    sector,
                    topheight: r.i32()?,
                    speed: r.i32()?,
                    direction: r.i32()?,
                    topwait: r.i32()?,
                    topcountdown: r.i32()?,
                };
                let id = thinkers.add_thinker(ThinkFn::VerticalDoor, ThinkerData::VlDoor(d));
                level.sectors[sector].specialdata = Some(id);
            }
            TC_FLOOR => {
                let floor_type = enum_from(r.i32()?, FloorType::from_index, "floor type")?;
                let crush = r.bool()?;
                let sector = read_sector(r, level)?;
                let f = FloorMove {
                    floor_type,
                    crush,
                    sector,
                    direction: r.i32()?,
                    newspecial: r.i16()?,
                    texture: r.i16()?,
                    floordestheight: r.i32()?,
                    speed: r.i32()?,
                };
                let id = thinkers.add_thinker(ThinkFn::FloorMove, ThinkerData::FloorMove(f));
                level.sectors[sector].specialdata = Some(id);
            }
            TC_PLAT => {
                let sector = read_sector(r, level)?;
                let (speed, low, high, wait, count) =
                    (r.i32()?, r.i32()?, r.i32()?, r.i32()?, r.i32()?);
                let status = enum_from(r.i32()?, PlatStatus::from_index, "plat status")?;
                let oldstatus = enum_from(r.i32()?, PlatStatus::from_index, "plat status")?;
                let crush = r.bool()?;
                let tag = r.i16()?;
                let plat_type = enum_from(r.i32()?, PlatType::from_index, "plat type")?;
                let p = Plat {
                    sector,
                    speed,
                    low,
                    high,
                    wait,
                    count,
                    status,
                    oldstatus,
                    crush,
                    tag,
                    plat_type,
                };
                let id = thinkers.add_thinker(ThinkFn::PlatRaise, ThinkerData::Plat(p));
                level.sectors[sector].specialdata = Some(id);
                active_plats.p_add_active_plat(id);
            }
            TC_FLASH => {
                let sector = read_sector(r, level)?;
                let f = LightFlash {
                    sector,
                    count: r.i32()?,
                    maxlight: r.i32()?,
                    minlight: r.i32()?,
                    maxtime: r.i32()?,
                    mintime: r.i32()?,
                };
                thinkers.add_thinker(ThinkFn::LightFlash, ThinkerData::LightFlash(f));
            }
            TC_STROBE => {
                let sector = read_sector(r, level)?;
                let s = Strobe {
                    sector,
                    count: r.i32()?,
                    minlight: r.i32()?,
                    maxlight: r.i32()?,
                    darktime: r.i32()?,
                    brighttime: r.i32()?,
                };
                thinkers.add_thinker(ThinkFn::StrobeFlash, ThinkerData::StrobeFlash(s));
            }
            TC_GLOW => {
                let sector = read_sector(r, level)?;
                let g = Glow {
                    sector,
                    minlight: r.i32()?,
                    maxlight: r.i32()?,
                    direction: r.i32()?,
                };
                thinkers.add_thinker(ThinkFn::Glow, ThinkerData::Glow(g));
            }
            n => {
                return err(format!(
                    "P_UnarchiveSpecials: Unknown tclass {n} in savegame"
                ))
            }
        }
    }
}

/// `MAXPLAYERS` as a `usize` for callers sizing `playeringame`.
pub const MAXPLAYERS_USIZE: usize = MAXPLAYERS as usize;
