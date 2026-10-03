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
//	Plats (i.e. elevator platforms) code, raising/lowering.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_plats.c`. Plats (i.e. elevator platforms)
//! code, raising/lowering (the original's own description, preserved).
//!
//! # Representation
//!
//! [`Plat`] drops the embedded `thinker_t` header, same as every other
//! mover in this phase. `activeplats[MAXPLATS]` becomes [`ActivePlats`]
//! — an owned, level-local `[Option<ThinkerId>; MAXPLATS]` threaded
//! explicitly through call sites, same convention as `p_switch.rs`'s
//! [`crate::p_switch::SwitchState`] (not a `GlobalCell`: this resets
//! every `P_SpawnSpecials`, so it isn't genuinely game-wide state).
//!
//! Sounds (`S_StartSound`) are queued via [`crate::s_sound`], origin
//! the platform sector's `soundorg`.

use crate::p_setup::Level;
use crate::p_spec::{
    p_find_highest_floor_surrounding, p_find_lowest_floor_surrounding, p_find_sector_from_line_tag,
};
use crate::p_tick::{ThinkFn, ThinkerData, ThinkerId, Thinkers};
use crate::r_defs::Line;
use crate::s_sound::{s_start_sound, sector_origin};
use crate::sounds::Sfx;
use crate::{
    m_fixed::{Fixed, FRACUNIT},
    m_random::p_random,
};

/// (`PLATWAIT`)
pub const PLATWAIT: i32 = 3;
/// (`PLATSPEED`)
pub const PLATSPEED: Fixed = FRACUNIT;
/// (`MAXPLATS`)
pub const MAXPLATS: usize = 30;

/// (`plat_e`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatStatus {
    Up,
    Down,
    Waiting,
    InStasis,
}

/// (`plattype_e`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatType {
    PerpetualRaise,
    DownWaitUpStay,
    RaiseAndChange,
    RaiseToNearestAndChange,
    BlazeDwus,
}

/// (`plat_t`)
#[derive(Debug, Clone, Copy)]
pub struct Plat {
    pub sector: usize,
    pub speed: Fixed,
    pub low: Fixed,
    pub high: Fixed,
    pub wait: i32,
    pub count: i32,
    pub status: PlatStatus,
    pub oldstatus: PlatStatus,
    pub crush: bool,
    pub tag: i16,
    pub plat_type: PlatType,
}

/// (`activeplats[MAXPLATS]`) — see module docs.
#[derive(Debug)]
pub struct ActivePlats([Option<ThinkerId>; MAXPLATS]);

impl Default for ActivePlats {
    fn default() -> Self {
        ActivePlats([None; MAXPLATS])
    }
}

impl ActivePlats {
    /// Whether `id` is in the active list (the archiver's `activeXXX[i] == th` scan).
    pub fn contains(&self, id: ThinkerId) -> bool {
        self.0.contains(&Some(id))
    }

    /// Port of `P_AddActivePlat`.
    pub fn p_add_active_plat(&mut self, plat: ThinkerId) {
        for slot in self.0.iter_mut() {
            if slot.is_none() {
                *slot = Some(plat);
                return;
            }
        }
        panic!("P_AddActivePlat: no more plats!");
    }

    /// Port of `P_RemoveActivePlat`. Also clears the sector's
    /// `specialdata` and removes the thinker (the original does this
    /// via `(activeplats[i])->sector->specialdata = NULL` and
    /// `P_RemoveThinker` inline — reproduced here rather than split
    /// across two call sites, since every caller in this port always
    /// wants both, same as the original always did both together).
    pub fn p_remove_active_plat(
        &mut self,
        thinkers: &mut Thinkers,
        level: &mut Level,
        plat: ThinkerId,
    ) {
        for slot in self.0.iter_mut() {
            if *slot == Some(plat) {
                let sector = thinkers.plat(plat).unwrap().sector;
                level.sectors[sector].specialdata = None;
                thinkers.remove(plat);
                *slot = None;
                return;
            }
        }
        panic!("P_RemoveActivePlat: can't find plat!");
    }

    /// Port of `P_ActivateInStasis`.
    pub fn p_activate_in_stasis(&mut self, thinkers: &mut Thinkers, tag: i16) {
        for slot in self.0.iter().flatten() {
            let plat = thinkers.plat_mut(*slot).unwrap();
            if plat.tag == tag && plat.status == PlatStatus::InStasis {
                plat.status = plat.oldstatus;
            }
        }
    }

    /// Port of `EV_StopPlat`.
    pub fn ev_stop_plat(&mut self, thinkers: &mut Thinkers, line: &Line) {
        for slot in self.0.iter().flatten() {
            let plat = thinkers.plat_mut(*slot).unwrap();
            if plat.status != PlatStatus::InStasis && plat.tag == line.tag {
                plat.oldstatus = plat.status;
                plat.status = PlatStatus::InStasis;
            }
        }
    }
}

/// Port of `T_PlatRaise`. Move a plat up and down (the original's own
/// comment, preserved).
#[allow(clippy::too_many_arguments)]
pub fn t_plat_raise(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut crate::r_main::RMain,
    active: &mut ActivePlats,
    validcount: i32,
    leveltime: i32,
    id: ThinkerId,
) {
    let plat = *thinkers.plat(id).unwrap();

    match plat.status {
        PlatStatus::Up => {
            let res = crate::p_floor::t_move_plane(
                thinkers,
                level,
                players,
                rmain,
                validcount,
                leveltime,
                plat.sector,
                plat.speed,
                plat.high,
                plat.crush,
                crate::p_floor::PlaneKind::Floor,
                1,
            );

            if (plat.plat_type == PlatType::RaiseAndChange
                || plat.plat_type == PlatType::RaiseToNearestAndChange)
                && leveltime & 7 == 0
            {
                s_start_sound(sector_origin(plat.sector), Sfx::SfxStnmov);
            }

            if res == crate::p_floor::ResultE::Crushed && !plat.crush {
                let p = thinkers.plat_mut(id).unwrap();
                p.count = p.wait;
                p.status = PlatStatus::Down;
                s_start_sound(sector_origin(plat.sector), Sfx::SfxPstart);
            } else if res == crate::p_floor::ResultE::Pastdest {
                let p = thinkers.plat_mut(id).unwrap();
                p.count = p.wait;
                p.status = PlatStatus::Waiting;
                s_start_sound(sector_origin(plat.sector), Sfx::SfxPstop);

                match plat.plat_type {
                    PlatType::BlazeDwus
                    | PlatType::DownWaitUpStay
                    | PlatType::RaiseAndChange
                    | PlatType::RaiseToNearestAndChange => {
                        active.p_remove_active_plat(thinkers, level, id);
                    }
                    PlatType::PerpetualRaise => {}
                }
            }
        }
        PlatStatus::Down => {
            let res = crate::p_floor::t_move_plane(
                thinkers,
                level,
                players,
                rmain,
                validcount,
                leveltime,
                plat.sector,
                plat.speed,
                plat.low,
                false,
                crate::p_floor::PlaneKind::Floor,
                -1,
            );

            if res == crate::p_floor::ResultE::Pastdest {
                let p = thinkers.plat_mut(id).unwrap();
                p.count = p.wait;
                p.status = PlatStatus::Waiting;
                s_start_sound(sector_origin(plat.sector), Sfx::SfxPstop);
            }
        }
        PlatStatus::Waiting => {
            let p = thinkers.plat_mut(id).unwrap();
            p.count -= 1;
            if p.count == 0 {
                let sector_floorheight = level.sectors[p.sector].floorheight;
                p.status = if sector_floorheight == p.low {
                    PlatStatus::Up
                } else {
                    PlatStatus::Down
                };
                s_start_sound(sector_origin(plat.sector), Sfx::SfxPstart);
            }
        }
        PlatStatus::InStasis => {}
    }
}

/// Port of `EV_DoPlat`. Do Platforms — "amount" is only used for SOME
/// platforms (the original's own comment, preserved).
pub fn ev_do_plat(
    level: &mut Level,
    thinkers: &mut Thinkers,
    active: &mut ActivePlats,
    line: &Line,
    plat_type: PlatType,
    amount: i32,
) -> bool {
    let mut rtn = false;

    // Activate all <type> plats that are in_stasis (the original's own
    // comment, preserved).
    if plat_type == PlatType::PerpetualRaise {
        active.p_activate_in_stasis(thinkers, line.tag);
    }

    let mut secnum = -1;
    loop {
        secnum = p_find_sector_from_line_tag(level, line, secnum);
        if secnum < 0 {
            break;
        }
        let sec = secnum as usize;

        if level.sectors[sec].specialdata.is_some() {
            continue;
        }

        // Find lowest & highest floors around sector (the original's
        // own comment, preserved).
        rtn = true;
        let mut plat = Plat {
            sector: sec,
            speed: PLATSPEED,
            low: 0,
            high: 0,
            wait: 0,
            count: 0,
            status: PlatStatus::Up,
            oldstatus: PlatStatus::Up,
            crush: false,
            tag: line.tag,
            plat_type,
        };

        match plat_type {
            PlatType::RaiseToNearestAndChange => {
                plat.speed = PLATSPEED / 2;
                let front_sector = line.sidenum[0].map(|s| level.sides[s].sector).unwrap();
                level.sectors[sec].floorpic = level.sectors[front_sector].floorpic;
                plat.high = crate::p_spec::p_find_next_highest_floor(
                    level,
                    sec,
                    level.sectors[sec].floorheight,
                );
                plat.wait = 0;
                plat.status = PlatStatus::Up;
                // NO MORE DAMAGE, IF APPLICABLE (the original's own
                // comment, preserved).
                level.sectors[sec].special = 0;
                s_start_sound(sector_origin(sec), Sfx::SfxStnmov);
            }
            PlatType::RaiseAndChange => {
                plat.speed = PLATSPEED / 2;
                let front_sector = line.sidenum[0].map(|s| level.sides[s].sector).unwrap();
                level.sectors[sec].floorpic = level.sectors[front_sector].floorpic;
                plat.high = level.sectors[sec].floorheight + amount * FRACUNIT;
                plat.wait = 0;
                plat.status = PlatStatus::Up;
                s_start_sound(sector_origin(sec), Sfx::SfxStnmov);
            }
            PlatType::DownWaitUpStay => {
                plat.speed = PLATSPEED * 4;
                plat.low = p_find_lowest_floor_surrounding(level, sec);
                if plat.low > level.sectors[sec].floorheight {
                    plat.low = level.sectors[sec].floorheight;
                }
                plat.high = level.sectors[sec].floorheight;
                plat.wait = 35 * PLATWAIT;
                plat.status = PlatStatus::Down;
                s_start_sound(sector_origin(sec), Sfx::SfxPstart);
            }
            PlatType::BlazeDwus => {
                plat.speed = PLATSPEED * 8;
                plat.low = p_find_lowest_floor_surrounding(level, sec);
                if plat.low > level.sectors[sec].floorheight {
                    plat.low = level.sectors[sec].floorheight;
                }
                plat.high = level.sectors[sec].floorheight;
                plat.wait = 35 * PLATWAIT;
                plat.status = PlatStatus::Down;
                s_start_sound(sector_origin(sec), Sfx::SfxPstart);
            }
            PlatType::PerpetualRaise => {
                plat.speed = PLATSPEED;
                plat.low = p_find_lowest_floor_surrounding(level, sec);
                if plat.low > level.sectors[sec].floorheight {
                    plat.low = level.sectors[sec].floorheight;
                }
                plat.high = p_find_highest_floor_surrounding(level, sec);
                if plat.high < level.sectors[sec].floorheight {
                    plat.high = level.sectors[sec].floorheight;
                }
                plat.wait = 35 * PLATWAIT;
                plat.status = if p_random() & 1 != 0 {
                    PlatStatus::Up
                } else {
                    PlatStatus::Down
                };
                s_start_sound(sector_origin(sec), Sfx::SfxPstart);
            }
        }

        let id = thinkers.add_thinker(ThinkFn::PlatRaise, ThinkerData::Plat(plat));
        level.sectors[sec].specialdata = Some(id);
        active.p_add_active_plat(id);
    }

    rtn
}

impl PlatStatus {
    /// Every variant in declaration order (the original's enum values), so
    /// `ALL[v as usize] == v`.
    pub const ALL: [PlatStatus; 4] = [
        PlatStatus::Up,
        PlatStatus::Down,
        PlatStatus::Waiting,
        PlatStatus::InStasis,
    ];

    /// The safe inverse of `as usize` (used by the savegame reader).
    pub fn from_index(i: usize) -> Option<PlatStatus> {
        Self::ALL.get(i).copied()
    }
}

impl PlatType {
    /// Every variant in declaration order (the original's enum values), so
    /// `ALL[v as usize] == v`.
    pub const ALL: [PlatType; 5] = [
        PlatType::PerpetualRaise,
        PlatType::DownWaitUpStay,
        PlatType::RaiseAndChange,
        PlatType::RaiseToNearestAndChange,
        PlatType::BlazeDwus,
    ];

    /// The safe inverse of `as usize` (used by the savegame reader).
    pub fn from_index(i: usize) -> Option<PlatType> {
        Self::ALL.get(i).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r_defs::Sector;

    fn level_with_sectors(heights: &[(i32, i32)]) -> Level {
        let mut level = Level::default();
        level.sectors = heights
            .iter()
            .map(|&(floor, ceiling)| Sector {
                floorheight: floor * FRACUNIT,
                ceilingheight: ceiling * FRACUNIT,
                ..Default::default()
            })
            .collect();
        level
    }

    #[test]
    fn ev_do_plat_down_wait_up_stay_spawns_a_plat_going_down() {
        let mut level = level_with_sectors(&[(20, 128), (0, 128)]);
        level.sectors[0].tag = 1;
        level.lines.push(Line {
            flags: crate::doomdata::ML_TWOSIDED,
            frontsector: Some(0),
            backsector: Some(1),
            ..Default::default()
        });
        level.sectors[0].lines = vec![0];

        let mut thinkers = Thinkers::new();
        let mut active = ActivePlats::default();
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let rtn = ev_do_plat(
            &mut level,
            &mut thinkers,
            &mut active,
            &line,
            PlatType::DownWaitUpStay,
            0,
        );

        assert!(rtn);
        let id = level.sectors[0].specialdata.expect("plat spawned");
        let plat = thinkers.plat(id).unwrap();
        assert_eq!(plat.status, PlatStatus::Down);
        assert_eq!(plat.low, 0, "lowest surrounding floor");
        assert_eq!(plat.high, 20 * FRACUNIT, "starting floor height");
        assert!(active.0.contains(&Some(id)));
    }

    #[test]
    fn ev_do_plat_skips_sector_already_moving() {
        let mut level = level_with_sectors(&[(0, 128)]);
        level.sectors[0].tag = 1;
        let mut thinkers = Thinkers::new();
        let mut active = ActivePlats::default();
        let fake = thinkers.add_thinker(
            ThinkFn::PlatRaise,
            ThinkerData::Plat(Plat {
                sector: 0,
                speed: PLATSPEED,
                low: 0,
                high: 0,
                wait: 0,
                count: 0,
                status: PlatStatus::Down,
                oldstatus: PlatStatus::Down,
                crush: false,
                tag: 1,
                plat_type: PlatType::DownWaitUpStay,
            }),
        );
        level.sectors[0].specialdata = Some(fake);
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let rtn = ev_do_plat(
            &mut level,
            &mut thinkers,
            &mut active,
            &line,
            PlatType::DownWaitUpStay,
            0,
        );

        assert!(!rtn);
        assert_eq!(level.sectors[0].specialdata, Some(fake));
    }

    #[test]
    fn t_plat_raise_waiting_switches_to_up_when_at_low() {
        let mut level = level_with_sectors(&[(0, 128)]);
        let mut thinkers = Thinkers::new();
        let mut rmain = crate::r_main::RMain::new();
        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut active = ActivePlats::default();

        let id = thinkers.add_thinker(
            ThinkFn::PlatRaise,
            ThinkerData::Plat(Plat {
                sector: 0,
                speed: PLATSPEED,
                low: 0,
                high: 10 * FRACUNIT,
                wait: 5,
                count: 1,
                status: PlatStatus::Waiting,
                oldstatus: PlatStatus::Waiting,
                crush: false,
                tag: 0,
                plat_type: PlatType::DownWaitUpStay,
            }),
        );

        t_plat_raise(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            &mut active,
            1,
            0,
            id,
        );

        assert_eq!(thinkers.plat(id).unwrap().status, PlatStatus::Up);
    }

    #[test]
    fn remove_active_plat_clears_sector_specialdata_and_frees_thinker() {
        let mut level = level_with_sectors(&[(0, 128)]);
        let mut thinkers = Thinkers::new();
        let mut active = ActivePlats::default();
        let id = thinkers.add_thinker(
            ThinkFn::PlatRaise,
            ThinkerData::Plat(Plat {
                sector: 0,
                speed: PLATSPEED,
                low: 0,
                high: 0,
                wait: 0,
                count: 0,
                status: PlatStatus::Waiting,
                oldstatus: PlatStatus::Waiting,
                crush: false,
                tag: 0,
                plat_type: PlatType::DownWaitUpStay,
            }),
        );
        level.sectors[0].specialdata = Some(id);
        active.p_add_active_plat(id);

        active.p_remove_active_plat(&mut thinkers, &mut level, id);

        assert_eq!(level.sectors[0].specialdata, None);
        assert!(!thinkers.is_live(id));
        assert!(!active.0.contains(&Some(id)));
    }
}
