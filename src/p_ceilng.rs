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
// DESCRIPTION:  Ceiling aninmation (lowering, crushing, raising)
//
//-----------------------------------------------------------------------------

//! Rust port of `p_ceilng.c`. Ceiling animation (lowering,
//! crushing, raising) (the original's own description, preserved).
//!
//! # Representation
//!
//! [`Ceiling`] drops the embedded `thinker_t` header, same as every
//! other mover in this phase. `activeceilings[MAXCEILINGS]` becomes
//! [`ActiveCeilings`], same `[Option<ThinkerId>; N]` convention as
//! `p_plats.rs`'s `ActivePlats`.
//!
//! One divergence from a literal transliteration: the original's
//! `direction` field triples as "in stasis" (`0`), "down" (`-1`), and
//! "up" (`1`) — [`t_move_ceiling`]/[`Ceiling`] keep that same `i32`
//! encoding (rather than splitting stasis into a separate bool) because
//! `P_ActivateInStasisCeiling`/`EV_CeilingCrushStop` swap `direction`
//! with `olddirection` directly, and re-deriving that dance through a
//! richer enum would obscure the original's own logic rather than
//! clarify it.
//!
//! Sounds (`S_StartSound`) are queued via [`crate::s_sound`], origin
//! the ceiling sector's `soundorg`.

use crate::p_floor::{t_move_plane, PlaneKind, ResultE};
use crate::p_setup::Level;
use crate::p_spec::{p_find_highest_ceiling_surrounding, p_find_sector_from_line_tag};
use crate::p_tick::{ThinkFn, ThinkerData, ThinkerId, Thinkers};
use crate::r_defs::Line;
use crate::s_sound::{s_start_sound, sector_origin};
use crate::sounds::Sfx;
use crate::{
    d_player::Player,
    m_fixed::{Fixed, FRACUNIT},
    r_main::RMain,
};

pub const CEILSPEED: Fixed = FRACUNIT;
pub const CEILWAIT: i32 = 150;
pub const MAXCEILINGS: usize = 30;

/// (`ceiling_e`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeilingType {
    LowerToFloor,
    RaiseToHighest,
    LowerAndCrush,
    CrushAndRaise,
    FastCrushAndRaise,
    SilentCrushAndRaise,
}

/// (`ceiling_t`)
#[derive(Debug, Clone, Copy)]
pub struct Ceiling {
    pub ceiling_type: CeilingType,
    pub sector: usize,
    pub bottomheight: Fixed,
    pub topheight: Fixed,
    pub speed: Fixed,
    pub crush: bool,

    /// 1 = up, 0 = waiting (in stasis), -1 = down (the original's own
    /// comment, preserved — see module docs on why this stays an
    /// `i32`).
    pub direction: i32,

    pub tag: i16,
    pub olddirection: i32,
}

/// (`activeceilings[MAXCEILINGS]`) — see module docs.
#[derive(Debug)]
pub struct ActiveCeilings([Option<ThinkerId>; MAXCEILINGS]);

impl Default for ActiveCeilings {
    fn default() -> Self {
        ActiveCeilings([None; MAXCEILINGS])
    }
}

impl ActiveCeilings {
    /// Whether `id` is in the active list (the archiver's `activeXXX[i] == th` scan).
    pub fn contains(&self, id: ThinkerId) -> bool {
        self.0.contains(&Some(id))
    }

    /// Port of `P_AddActiveCeiling`.
    pub fn p_add_active_ceiling(&mut self, c: ThinkerId) {
        for slot in self.0.iter_mut() {
            if slot.is_none() {
                *slot = Some(c);
                return;
            }
        }
        // The original silently drops the ceiling if the table is full
        // (no `I_Error`, unlike `P_AddActivePlat`) — reproduced as a
        // silent no-op, not a panic.
    }

    /// Port of `P_RemoveActiveCeiling`.
    pub fn p_remove_active_ceiling(
        &mut self,
        thinkers: &mut Thinkers,
        level: &mut Level,
        c: ThinkerId,
    ) {
        for slot in self.0.iter_mut() {
            if *slot == Some(c) {
                let sector = thinkers.ceiling(c).unwrap().sector;
                level.sectors[sector].specialdata = None;
                thinkers.remove(c);
                *slot = None;
                return;
            }
        }
    }

    /// Port of `P_ActivateInStasisCeiling`. Restart a ceiling that's
    /// in-stasis (the original's own comment, preserved).
    pub fn p_activate_in_stasis_ceiling(&mut self, thinkers: &mut Thinkers, line: &Line) {
        for slot in self.0.iter().flatten() {
            let c = thinkers.ceiling_mut(*slot).unwrap();
            if c.tag == line.tag && c.direction == 0 {
                c.direction = c.olddirection;
            }
        }
    }

    /// Port of `EV_CeilingCrushStop`. Stop a ceiling from crushing!
    /// (the original's own comment, preserved).
    pub fn ev_ceiling_crush_stop(&mut self, thinkers: &mut Thinkers, line: &Line) -> bool {
        let mut rtn = false;
        for slot in self.0.iter().flatten() {
            let c = thinkers.ceiling_mut(*slot).unwrap();
            if c.tag == line.tag && c.direction != 0 {
                c.olddirection = c.direction;
                c.direction = 0; // in-stasis
                rtn = true;
            }
        }
        rtn
    }
}

/// Port of `T_MoveCeiling`.
#[allow(clippy::too_many_arguments)]
pub fn t_move_ceiling(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    active: &mut ActiveCeilings,
    validcount: i32,
    leveltime: i32,
    id: ThinkerId,
) {
    let ceiling = *thinkers.ceiling(id).unwrap();

    match ceiling.direction {
        0 => {
            // IN STASIS
        }
        1 => {
            // UP
            let res = t_move_plane(
                thinkers,
                level,
                players,
                rmain,
                validcount,
                leveltime,
                ceiling.sector,
                ceiling.speed,
                ceiling.topheight,
                false,
                PlaneKind::Ceiling,
                ceiling.direction,
            );

            if leveltime & 7 == 0 && ceiling.ceiling_type != CeilingType::SilentCrushAndRaise {
                s_start_sound(sector_origin(ceiling.sector), Sfx::SfxStnmov);
            }

            if res == ResultE::Pastdest {
                match ceiling.ceiling_type {
                    CeilingType::RaiseToHighest => {
                        active.p_remove_active_ceiling(thinkers, level, id);
                    }
                    CeilingType::SilentCrushAndRaise => {
                        s_start_sound(sector_origin(ceiling.sector), Sfx::SfxPstop);
                        // falls through in the original
                        thinkers.ceiling_mut(id).unwrap().direction = -1;
                    }
                    CeilingType::FastCrushAndRaise | CeilingType::CrushAndRaise => {
                        thinkers.ceiling_mut(id).unwrap().direction = -1;
                    }
                    _ => {}
                }
            }
        }
        -1 => {
            // DOWN
            let res = t_move_plane(
                thinkers,
                level,
                players,
                rmain,
                validcount,
                leveltime,
                ceiling.sector,
                ceiling.speed,
                ceiling.bottomheight,
                ceiling.crush,
                PlaneKind::Ceiling,
                ceiling.direction,
            );

            if leveltime & 7 == 0 && ceiling.ceiling_type != CeilingType::SilentCrushAndRaise {
                s_start_sound(sector_origin(ceiling.sector), Sfx::SfxStnmov);
            }

            if res == ResultE::Pastdest {
                match ceiling.ceiling_type {
                    CeilingType::SilentCrushAndRaise => {
                        s_start_sound(sector_origin(ceiling.sector), Sfx::SfxPstop);
                        thinkers.ceiling_mut(id).unwrap().speed = CEILSPEED;
                        thinkers.ceiling_mut(id).unwrap().direction = 1;
                    }
                    CeilingType::CrushAndRaise => {
                        thinkers.ceiling_mut(id).unwrap().speed = CEILSPEED;
                        thinkers.ceiling_mut(id).unwrap().direction = 1;
                    }
                    CeilingType::FastCrushAndRaise => {
                        thinkers.ceiling_mut(id).unwrap().direction = 1;
                    }
                    CeilingType::LowerAndCrush | CeilingType::LowerToFloor => {
                        active.p_remove_active_ceiling(thinkers, level, id);
                    }
                    _ => {}
                }
            } else if res == ResultE::Crushed {
                match ceiling.ceiling_type {
                    CeilingType::SilentCrushAndRaise
                    | CeilingType::CrushAndRaise
                    | CeilingType::LowerAndCrush => {
                        thinkers.ceiling_mut(id).unwrap().speed = CEILSPEED / 8;
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Port of `EV_DoCeiling`. Move a ceiling up/down and all around! (the
/// original's own comment, preserved).
pub fn ev_do_ceiling(
    level: &mut Level,
    thinkers: &mut Thinkers,
    active: &mut ActiveCeilings,
    line: &Line,
    ceiling_type: CeilingType,
) -> bool {
    let mut rtn = false;

    // Reactivate in-stasis ceilings...for certain types (the original's
    // own comment, preserved).
    if matches!(
        ceiling_type,
        CeilingType::FastCrushAndRaise
            | CeilingType::SilentCrushAndRaise
            | CeilingType::CrushAndRaise
    ) {
        active.p_activate_in_stasis_ceiling(thinkers, line);
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

        rtn = true;
        let mut ceiling = Ceiling {
            ceiling_type,
            sector: sec,
            bottomheight: 0,
            topheight: 0,
            speed: CEILSPEED,
            crush: false,
            direction: 0,
            tag: level.sectors[sec].tag,
            olddirection: 0,
        };

        match ceiling_type {
            CeilingType::FastCrushAndRaise => {
                ceiling.crush = true;
                ceiling.topheight = level.sectors[sec].ceilingheight;
                ceiling.bottomheight = level.sectors[sec].floorheight + 8 * FRACUNIT;
                ceiling.direction = -1;
                ceiling.speed = CEILSPEED * 2;
            }
            CeilingType::SilentCrushAndRaise | CeilingType::CrushAndRaise => {
                ceiling.crush = true;
                ceiling.topheight = level.sectors[sec].ceilingheight;
                ceiling.bottomheight = level.sectors[sec].floorheight + 8 * FRACUNIT;
                ceiling.direction = -1;
                ceiling.speed = CEILSPEED;
            }
            // `crush` is left `false` here — the original's switch only
            // sets `ceiling->crush = true` in the silentCrushAndRaise/
            // crushAndRaise case above, which *falls through* into this
            // one for the bottomheight/direction/speed assignment;
            // lowerAndCrush's own name notwithstanding, it never gets
            // `crush = true` when reached directly. Preserved as-is.
            CeilingType::LowerAndCrush | CeilingType::LowerToFloor => {
                ceiling.bottomheight = level.sectors[sec].floorheight;
                if ceiling_type != CeilingType::LowerToFloor {
                    ceiling.bottomheight += 8 * FRACUNIT;
                }
                ceiling.direction = -1;
                ceiling.speed = CEILSPEED;
            }
            CeilingType::RaiseToHighest => {
                ceiling.topheight = p_find_highest_ceiling_surrounding(level, sec);
                ceiling.direction = 1;
                ceiling.speed = CEILSPEED;
            }
        }

        let id = thinkers.add_thinker(ThinkFn::MoveCeiling, ThinkerData::Ceiling(ceiling));
        level.sectors[sec].specialdata = Some(id);
        active.p_add_active_ceiling(id);
    }
    rtn
}

impl CeilingType {
    /// Every variant in declaration order (the original's enum values), so
    /// `ALL[v as usize] == v`.
    pub const ALL: [CeilingType; 6] = [
        CeilingType::LowerToFloor,
        CeilingType::RaiseToHighest,
        CeilingType::LowerAndCrush,
        CeilingType::CrushAndRaise,
        CeilingType::FastCrushAndRaise,
        CeilingType::SilentCrushAndRaise,
    ];

    /// The safe inverse of `as usize` (used by the savegame reader).
    pub fn from_index(i: usize) -> Option<CeilingType> {
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
    fn ev_do_ceiling_lower_and_crush_targets_floor_plus_8() {
        let mut level = level_with_sectors(&[(0, 128)]);
        level.sectors[0].tag = 1;
        let mut thinkers = Thinkers::new();
        let mut active = ActiveCeilings::default();
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let rtn = ev_do_ceiling(
            &mut level,
            &mut thinkers,
            &mut active,
            &line,
            CeilingType::LowerAndCrush,
        );

        assert!(rtn);
        let id = level.sectors[0].specialdata.expect("ceiling mover spawned");
        let ceiling = thinkers.ceiling(id).unwrap();
        assert_eq!(ceiling.bottomheight, 8 * FRACUNIT);
        assert_eq!(ceiling.direction, -1);
        assert!(
            !ceiling.crush,
            "lowerAndCrush falls through from the crush-setting cases in the original \
             without itself setting crush=true — only crushAndRaise/silentCrushAndRaise/\
             fastCrushAndRaise do"
        );
        assert!(active.0.contains(&Some(id)));
    }

    #[test]
    fn ev_do_ceiling_lower_to_floor_does_not_add_the_8_unit_gap() {
        let mut level = level_with_sectors(&[(0, 128)]);
        level.sectors[0].tag = 1;
        let mut thinkers = Thinkers::new();
        let mut active = ActiveCeilings::default();
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        ev_do_ceiling(
            &mut level,
            &mut thinkers,
            &mut active,
            &line,
            CeilingType::LowerToFloor,
        );

        let id = level.sectors[0].specialdata.unwrap();
        assert_eq!(thinkers.ceiling(id).unwrap().bottomheight, 0);
    }

    #[test]
    fn t_move_ceiling_crush_and_raise_reverses_at_bottom() {
        let mut level = level_with_sectors(&[(0, 128)]);
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: Vec<Player> = Vec::new();
        let mut active = ActiveCeilings::default();

        level.sectors[0].ceilingheight = 10 * FRACUNIT;
        let ceiling = Ceiling {
            ceiling_type: CeilingType::CrushAndRaise,
            sector: 0,
            bottomheight: 10 * FRACUNIT,
            topheight: 128 * FRACUNIT,
            speed: 100 * FRACUNIT,
            crush: true,
            direction: -1,
            tag: 0,
            olddirection: 0,
        };
        let id = thinkers.add_thinker(ThinkFn::MoveCeiling, ThinkerData::Ceiling(ceiling));

        t_move_ceiling(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            &mut active,
            1,
            0,
            id,
        );

        let c = thinkers.ceiling(id).unwrap();
        assert_eq!(
            c.direction, 1,
            "should reverse to raise once past bottomheight"
        );
        assert_eq!(c.speed, CEILSPEED);
    }

    #[test]
    fn ev_ceiling_crush_stop_puts_matching_tag_in_stasis() {
        let mut thinkers = Thinkers::new();
        let mut active = ActiveCeilings::default();
        let id = thinkers.add_thinker(
            ThinkFn::MoveCeiling,
            ThinkerData::Ceiling(Ceiling {
                ceiling_type: CeilingType::CrushAndRaise,
                sector: 0,
                bottomheight: 0,
                topheight: 0,
                speed: CEILSPEED,
                crush: true,
                direction: -1,
                tag: 5,
                olddirection: 0,
            }),
        );
        active.p_add_active_ceiling(id);
        let line = Line {
            tag: 5,
            ..Default::default()
        };

        let rtn = active.ev_ceiling_crush_stop(&mut thinkers, &line);

        assert!(rtn);
        let c = thinkers.ceiling(id).unwrap();
        assert_eq!(c.direction, 0);
        assert_eq!(c.olddirection, -1);
    }
}
