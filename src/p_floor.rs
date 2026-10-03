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
//	Floor animation: raising stairs.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_floor.c`. Floor animation: raising stairs
//! (the original's own description, preserved).
//!
//! # `T_MovePlane`
//!
//! [`t_move_plane`] moves either a floor or a ceiling — the original
//! dispatches on a `floorOrCeiling: int` (`0`/`1`); this port keeps that
//! same `bool`-like split via [`PlaneKind`] instead of a raw `int`, for
//! a little extra type safety at call sites (`p_ceilng.rs`, Phase 7b3,
//! calls this same function for ceilings). `sector->floorheight`/
//! `ceilingheight` mutation and `P_ChangeSector`'s crush check are both
//! reproduced exactly, including the original's own `//return crushed`
//! dead code (commented out in the source — preserved as a comment here
//! too, not resurrected).
//!
//! # Representation
//!
//! [`FloorMove`] drops the embedded `thinker_t` header (as with every
//! other mover in this phase — [`crate::p_tick::Thinkers`] supplies list
//! linkage). `sector_t*` becomes a `usize` sector index. `texture:
//! short` (originally a raw flat-lump index) becomes `i16`, matching
//! [`crate::r_defs::Sector::floorpic`]'s type.
//!
//! Sounds (`S_StartSound`, `T_MoveFloor`'s stone-move/stop cues) are
//! queued via [`crate::s_sound`].

use crate::doomtype::MAXINT;
use crate::m_fixed::{Fixed, FRACUNIT};
use crate::p_map::p_change_sector;
use crate::p_setup::Level;
use crate::p_spec::{
    get_side, p_find_highest_floor_surrounding, p_find_lowest_ceiling_surrounding,
    p_find_lowest_floor_surrounding, p_find_next_highest_floor, p_find_sector_from_line_tag,
    two_sided,
};
use crate::p_tick::{ThinkFn, ThinkerData, ThinkerId, Thinkers};
use crate::r_data::RData;
use crate::r_defs::Line;
use crate::s_sound::{s_start_sound, sector_origin};
use crate::sounds::Sfx;

pub const FLOORSPEED: Fixed = FRACUNIT;

/// (`floor_e`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorType {
    /// lower floor to highest surrounding floor (the original's own
    /// comment, preserved).
    LowerFloor,
    /// lower floor to lowest surrounding floor (the original's own
    /// comment, preserved).
    LowerFloorToLowest,
    /// lower floor to highest surrounding floor VERY FAST (the
    /// original's own comment, preserved).
    TurboLower,
    /// raise floor to lowest surrounding CEILING (the original's own
    /// comment, preserved).
    RaiseFloor,
    /// raise floor to next highest surrounding floor (the original's
    /// own comment, preserved).
    RaiseFloorToNearest,
    /// raise floor to shortest height texture around it (the original's
    /// own comment, preserved).
    RaiseToTexture,
    /// lower floor to lowest surrounding floor and change floorpic (the
    /// original's own comment, preserved).
    LowerAndChange,
    RaiseFloor24,
    RaiseFloor24AndChange,
    RaiseFloorCrush,
    /// raise to next highest floor, turbo-speed (the original's own
    /// comment, preserved).
    RaiseFloorTurbo,
    DonutRaise,
    RaiseFloor512,
}

/// (`stair_e`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StairType {
    /// slowly build by 8 (the original's own comment, preserved).
    Build8,
    /// quickly build by 16 (the original's own comment, preserved).
    Turbo16,
}

/// (`floormove_t`)
#[derive(Debug, Clone, Copy)]
pub struct FloorMove {
    pub floor_type: FloorType,
    pub crush: bool,
    pub sector: usize,
    pub direction: i32,
    pub newspecial: i16,
    pub texture: i16,
    pub floordestheight: Fixed,
    pub speed: Fixed,
}

/// (`result_e`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultE {
    Ok,
    Crushed,
    Pastdest,
}

/// (`floorOrCeiling` in `T_MovePlane`) — which plane of the sector is
/// being moved. See module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneKind {
    Floor,
    Ceiling,
}

/// Port of `T_MovePlane`. Move a plane (floor or ceiling) and check for
/// crushing (the original's own comment, preserved).
#[allow(clippy::too_many_arguments)]
pub fn t_move_plane(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut crate::r_main::RMain,
    validcount: i32,
    leveltime: i32,
    sector: usize,
    speed: Fixed,
    dest: Fixed,
    crush: bool,
    plane: PlaneKind,
    direction: i32,
) -> ResultE {
    match plane {
        PlaneKind::Floor => match direction {
            -1 => {
                // DOWN
                if level.sectors[sector].floorheight - speed < dest {
                    let lastpos = level.sectors[sector].floorheight;
                    level.sectors[sector].floorheight = dest;
                    let flag = p_change_sector(
                        thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                    );
                    if flag {
                        level.sectors[sector].floorheight = lastpos;
                        p_change_sector(
                            thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                        );
                        // return crushed; (the original's own dead code, preserved)
                    }
                    return ResultE::Pastdest;
                }
                let lastpos = level.sectors[sector].floorheight;
                level.sectors[sector].floorheight -= speed;
                let flag = p_change_sector(
                    thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                );
                if flag {
                    level.sectors[sector].floorheight = lastpos;
                    p_change_sector(
                        thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                    );
                    return ResultE::Crushed;
                }
            }
            1 => {
                // UP
                if level.sectors[sector].floorheight + speed > dest {
                    let lastpos = level.sectors[sector].floorheight;
                    level.sectors[sector].floorheight = dest;
                    let flag = p_change_sector(
                        thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                    );
                    if flag {
                        level.sectors[sector].floorheight = lastpos;
                        p_change_sector(
                            thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                        );
                        // return crushed; (the original's own dead code, preserved)
                    }
                    return ResultE::Pastdest;
                }
                // COULD GET CRUSHED (the original's own comment,
                // preserved).
                let lastpos = level.sectors[sector].floorheight;
                level.sectors[sector].floorheight += speed;
                let flag = p_change_sector(
                    thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                );
                if flag {
                    if crush {
                        return ResultE::Crushed;
                    }
                    level.sectors[sector].floorheight = lastpos;
                    p_change_sector(
                        thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                    );
                    return ResultE::Crushed;
                }
            }
            _ => {}
        },
        PlaneKind::Ceiling => match direction {
            -1 => {
                // DOWN
                if level.sectors[sector].ceilingheight - speed < dest {
                    let lastpos = level.sectors[sector].ceilingheight;
                    level.sectors[sector].ceilingheight = dest;
                    let flag = p_change_sector(
                        thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                    );
                    if flag {
                        level.sectors[sector].ceilingheight = lastpos;
                        p_change_sector(
                            thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                        );
                        // return crushed; (the original's own dead code, preserved)
                    }
                    return ResultE::Pastdest;
                }
                // COULD GET CRUSHED (the original's own comment,
                // preserved).
                let lastpos = level.sectors[sector].ceilingheight;
                level.sectors[sector].ceilingheight -= speed;
                let flag = p_change_sector(
                    thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                );
                if flag {
                    if crush {
                        return ResultE::Crushed;
                    }
                    level.sectors[sector].ceilingheight = lastpos;
                    p_change_sector(
                        thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                    );
                    return ResultE::Crushed;
                }
            }
            1 => {
                // UP
                if level.sectors[sector].ceilingheight + speed > dest {
                    let lastpos = level.sectors[sector].ceilingheight;
                    level.sectors[sector].ceilingheight = dest;
                    let flag = p_change_sector(
                        thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                    );
                    if flag {
                        level.sectors[sector].ceilingheight = lastpos;
                        p_change_sector(
                            thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                        );
                        // return crushed; (the original's own dead code, preserved)
                    }
                    return ResultE::Pastdest;
                }
                level.sectors[sector].ceilingheight += speed;
                // UNUSED: the original's own `#if 0` crush-check on
                // ceiling-up is left out entirely, same as the source.
                let _ = p_change_sector(
                    thinkers, level, players, rmain, validcount, leveltime, sector, crush,
                );
            }
            _ => {}
        },
    }
    ResultE::Ok
}

/// Port of `T_MoveFloor`. MOVE A FLOOR TO IT'S DESTINATION (UP OR DOWN)
/// (the original's own comment, preserved).
pub fn t_move_floor(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut crate::r_main::RMain,
    validcount: i32,
    leveltime: i32,
    id: ThinkerId,
) {
    let floor = *thinkers.floor_move(id).unwrap();
    let res = t_move_plane(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        leveltime,
        floor.sector,
        floor.speed,
        floor.floordestheight,
        floor.crush,
        PlaneKind::Floor,
        floor.direction,
    );

    if leveltime & 7 == 0 {
        s_start_sound(sector_origin(floor.sector), Sfx::SfxStnmov);
    }

    if res == ResultE::Pastdest {
        level.sectors[floor.sector].specialdata = None;

        if floor.direction == 1 {
            if floor.floor_type == FloorType::DonutRaise {
                level.sectors[floor.sector].special = floor.newspecial;
                level.sectors[floor.sector].floorpic = floor.texture;
            }
        } else if floor.direction == -1 && floor.floor_type == FloorType::LowerAndChange {
            level.sectors[floor.sector].special = floor.newspecial;
            level.sectors[floor.sector].floorpic = floor.texture;
        }

        thinkers.remove(id);

        s_start_sound(sector_origin(floor.sector), Sfx::SfxPstop);
    }
}

/// Port of `EV_DoFloor`. HANDLE FLOOR TYPES (the original's own
/// comment, preserved).
pub fn ev_do_floor(
    level: &mut Level,
    thinkers: &mut Thinkers,
    rdata: &RData,
    line: &Line,
    floortype: FloorType,
) -> bool {
    let mut rtn = false;
    let mut secnum = -1;
    loop {
        secnum = p_find_sector_from_line_tag(level, line, secnum);
        if secnum < 0 {
            break;
        }
        let sec = secnum as usize;

        // ALREADY MOVING? IF SO, KEEP GOING... (the original's own
        // comment, preserved).
        if level.sectors[sec].specialdata.is_some() {
            continue;
        }

        rtn = true;
        let mut floor = FloorMove {
            floor_type: floortype,
            crush: false,
            sector: sec,
            direction: 0,
            newspecial: 0,
            texture: 0,
            floordestheight: 0,
            speed: FLOORSPEED,
        };

        match floortype {
            FloorType::LowerFloor => {
                floor.direction = -1;
                floor.speed = FLOORSPEED;
                floor.floordestheight = p_find_highest_floor_surrounding(level, sec);
            }
            FloorType::LowerFloorToLowest => {
                floor.direction = -1;
                floor.speed = FLOORSPEED;
                floor.floordestheight = p_find_lowest_floor_surrounding(level, sec);
            }
            FloorType::TurboLower => {
                floor.direction = -1;
                floor.speed = FLOORSPEED * 4;
                floor.floordestheight = p_find_highest_floor_surrounding(level, sec);
                if floor.floordestheight != level.sectors[sec].floorheight {
                    floor.floordestheight += 8 * FRACUNIT;
                }
            }
            FloorType::RaiseFloorCrush | FloorType::RaiseFloor => {
                floor.crush = floortype == FloorType::RaiseFloorCrush;
                floor.direction = 1;
                floor.speed = FLOORSPEED;
                floor.floordestheight = p_find_lowest_ceiling_surrounding(level, sec);
                if floor.floordestheight > level.sectors[sec].ceilingheight {
                    floor.floordestheight = level.sectors[sec].ceilingheight;
                }
                if floortype == FloorType::RaiseFloorCrush {
                    floor.floordestheight -= 8 * FRACUNIT;
                }
            }
            FloorType::RaiseFloorTurbo => {
                floor.direction = 1;
                floor.speed = FLOORSPEED * 4;
                floor.floordestheight =
                    p_find_next_highest_floor(level, sec, level.sectors[sec].floorheight);
            }
            FloorType::RaiseFloorToNearest => {
                floor.direction = 1;
                floor.speed = FLOORSPEED;
                floor.floordestheight =
                    p_find_next_highest_floor(level, sec, level.sectors[sec].floorheight);
            }
            FloorType::RaiseFloor24 => {
                floor.direction = 1;
                floor.speed = FLOORSPEED;
                floor.floordestheight = level.sectors[sec].floorheight + 24 * FRACUNIT;
            }
            FloorType::RaiseFloor512 => {
                floor.direction = 1;
                floor.speed = FLOORSPEED;
                floor.floordestheight = level.sectors[sec].floorheight + 512 * FRACUNIT;
            }
            FloorType::RaiseFloor24AndChange => {
                floor.direction = 1;
                floor.speed = FLOORSPEED;
                floor.floordestheight = level.sectors[sec].floorheight + 24 * FRACUNIT;
                let front = line.frontsector.unwrap();
                level.sectors[sec].floorpic = level.sectors[front].floorpic;
                level.sectors[sec].special = level.sectors[front].special;
            }
            FloorType::RaiseToTexture => {
                let mut minsize = MAXINT;
                floor.direction = 1;
                floor.speed = FLOORSPEED;
                for i in 0..level.sectors[sec].lines.len() {
                    if two_sided(level, sec, i) {
                        let side0 = get_side(level, sec, i, 0);
                        if side0.bottomtexture >= 0 {
                            let h = rdata.textureheight[side0.bottomtexture as usize];
                            if h < minsize {
                                minsize = h;
                            }
                        }
                        let side1 = get_side(level, sec, i, 1);
                        if side1.bottomtexture >= 0 {
                            let h = rdata.textureheight[side1.bottomtexture as usize];
                            if h < minsize {
                                minsize = h;
                            }
                        }
                    }
                }
                floor.floordestheight = level.sectors[sec].floorheight + minsize;
            }
            FloorType::LowerAndChange => {
                floor.direction = -1;
                floor.speed = FLOORSPEED;
                floor.floordestheight = p_find_lowest_floor_surrounding(level, sec);
                floor.texture = level.sectors[sec].floorpic;

                for i in 0..level.sectors[sec].lines.len() {
                    if two_sided(level, sec, i) {
                        let other = if get_side(level, sec, i, 0).sector == sec {
                            crate::p_spec::get_sector(level, sec, i, 1)
                        } else {
                            crate::p_spec::get_sector(level, sec, i, 0)
                        };

                        if level.sectors[other].floorheight == floor.floordestheight {
                            floor.texture = level.sectors[other].floorpic;
                            floor.newspecial = level.sectors[other].special;
                            break;
                        }
                    }
                }
            }
            FloorType::DonutRaise => {}
        }

        level.sectors[sec].specialdata =
            Some(thinkers.add_thinker(ThinkFn::FloorMove, ThinkerData::FloorMove(floor)));
    }
    rtn
}

/// Port of `EV_BuildStairs`. BUILD A STAIRCASE! (the original's own
/// comment, preserved).
pub fn ev_build_stairs(
    level: &mut Level,
    thinkers: &mut Thinkers,
    line: &Line,
    stair_type: StairType,
) -> bool {
    let mut rtn = false;
    let mut secnum = -1;
    loop {
        secnum = p_find_sector_from_line_tag(level, line, secnum);
        if secnum < 0 {
            break;
        }
        let mut sec = secnum as usize;

        // ALREADY MOVING? IF SO, KEEP GOING... (the original's own
        // comment, preserved).
        if level.sectors[sec].specialdata.is_some() {
            continue;
        }

        rtn = true;

        let (speed, stairsize) = match stair_type {
            StairType::Build8 => (FLOORSPEED / 4, 8 * FRACUNIT),
            StairType::Turbo16 => (FLOORSPEED * 4, 16 * FRACUNIT),
        };

        let mut height = level.sectors[sec].floorheight + stairsize;
        let texture = level.sectors[sec].floorpic;

        let mut floor = FloorMove {
            floor_type: FloorType::RaiseFloor, // (unused by T_MoveFloor for anything but donutRaise/lowerAndChange)
            crush: false,
            sector: sec,
            direction: 1,
            newspecial: 0,
            texture: 0,
            floordestheight: height,
            speed,
        };
        level.sectors[sec].specialdata =
            Some(thinkers.add_thinker(ThinkFn::FloorMove, ThinkerData::FloorMove(floor)));

        // Find next sector to raise. 1. Find 2-sided line with same
        // sector side[0]. 2. Other side is the next sector to raise
        // (the original's own comment, preserved).
        loop {
            let mut ok = false;
            for i in 0..level.sectors[sec].lines.len() {
                let line_idx = level.sectors[sec].lines[i];
                if level.lines[line_idx].flags & crate::doomdata::ML_TWOSIDED == 0 {
                    continue;
                }

                let front = level.lines[line_idx].frontsector.unwrap();
                if secnum != front as i32 {
                    continue;
                }

                let back = level.lines[line_idx].backsector.unwrap();

                if level.sectors[back].floorpic != texture {
                    continue;
                }

                height += stairsize;

                if level.sectors[back].specialdata.is_some() {
                    continue;
                }

                sec = back;
                secnum = back as i32;

                floor = FloorMove {
                    floor_type: FloorType::RaiseFloor,
                    crush: false,
                    sector: sec,
                    direction: 1,
                    newspecial: 0,
                    texture: 0,
                    floordestheight: height,
                    speed,
                };
                level.sectors[sec].specialdata =
                    Some(thinkers.add_thinker(ThinkFn::FloorMove, ThinkerData::FloorMove(floor)));
                ok = true;
                break;
            }
            if !ok {
                break;
            }
        }
    }
    rtn
}

impl FloorType {
    /// Every variant in declaration order (the original's enum values), so
    /// `ALL[v as usize] == v`.
    pub const ALL: [FloorType; 13] = [
        FloorType::LowerFloor,
        FloorType::LowerFloorToLowest,
        FloorType::TurboLower,
        FloorType::RaiseFloor,
        FloorType::RaiseFloorToNearest,
        FloorType::RaiseToTexture,
        FloorType::LowerAndChange,
        FloorType::RaiseFloor24,
        FloorType::RaiseFloor24AndChange,
        FloorType::RaiseFloorCrush,
        FloorType::RaiseFloorTurbo,
        FloorType::DonutRaise,
        FloorType::RaiseFloor512,
    ];

    /// The safe inverse of `as usize` (used by the savegame reader).
    pub fn from_index(i: usize) -> Option<FloorType> {
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
    fn move_plane_floor_up_stops_at_dest() {
        let mut level = level_with_sectors(&[(0, 128)]);
        let mut thinkers = Thinkers::new();
        let mut rmain = crate::r_main::RMain::new();
        let mut players: Vec<crate::d_player::Player> = Vec::new();

        let res = t_move_plane(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            1,
            0,
            0,
            FRACUNIT,
            10 * FRACUNIT,
            false,
            PlaneKind::Floor,
            1,
        );

        assert_eq!(level.sectors[0].floorheight, FRACUNIT);
        assert_eq!(
            res,
            ResultE::Ok,
            "still short of dest after one FRACUNIT step"
        );
    }

    #[test]
    fn move_plane_floor_up_reaches_pastdest_when_close_enough() {
        let mut level = level_with_sectors(&[(0, 128)]);
        level.sectors[0].floorheight = 9 * FRACUNIT;
        let mut thinkers = Thinkers::new();
        let mut rmain = crate::r_main::RMain::new();
        let mut players: Vec<crate::d_player::Player> = Vec::new();

        let res = t_move_plane(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            1,
            0,
            0,
            FRACUNIT * 2,
            10 * FRACUNIT,
            false,
            PlaneKind::Floor,
            1,
        );

        assert_eq!(level.sectors[0].floorheight, 10 * FRACUNIT);
        assert_eq!(res, ResultE::Pastdest);
    }

    #[test]
    fn move_plane_floor_down_stops_at_dest() {
        let mut level = level_with_sectors(&[(20, 128)]);
        let mut thinkers = Thinkers::new();
        let mut rmain = crate::r_main::RMain::new();
        let mut players: Vec<crate::d_player::Player> = Vec::new();

        let res = t_move_plane(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            1,
            0,
            0,
            FRACUNIT * 2,
            0,
            false,
            PlaneKind::Floor,
            -1,
        );

        assert_eq!(level.sectors[0].floorheight, 18 * FRACUNIT);
        assert_eq!(res, ResultE::Ok);
    }

    #[test]
    fn t_move_floor_removes_thinker_on_pastdest() {
        let mut level = level_with_sectors(&[(0, 128)]);
        let mut thinkers = Thinkers::new();
        let mut rmain = crate::r_main::RMain::new();
        let mut players: Vec<crate::d_player::Player> = Vec::new();

        let floor = FloorMove {
            floor_type: FloorType::RaiseFloor,
            crush: false,
            sector: 0,
            direction: 1,
            newspecial: 0,
            texture: 0,
            floordestheight: FRACUNIT,
            speed: FRACUNIT * 2,
        };
        let id = thinkers.add_thinker(ThinkFn::FloorMove, ThinkerData::FloorMove(floor));

        t_move_floor(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            1,
            0,
            id,
        );

        assert!(
            !thinkers.is_live(id),
            "thinker should self-remove at pastdest"
        );
        assert_eq!(level.sectors[0].specialdata, None);
    }

    #[test]
    fn ev_do_floor_raise_floor_24_targets_24_units_up() {
        let mut level = level_with_sectors(&[(0, 128)]);
        level.sectors[0].tag = 1;
        let mut thinkers = Thinkers::new();
        let rdata = RData::default();
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let rtn = ev_do_floor(
            &mut level,
            &mut thinkers,
            &rdata,
            &line,
            FloorType::RaiseFloor24,
        );

        assert!(rtn);
        let mover_id = level.sectors[0].specialdata.expect("floor mover spawned");
        let floor = thinkers.floor_move(mover_id).unwrap();
        assert_eq!(floor.floordestheight, 24 * FRACUNIT);
        assert_eq!(floor.direction, 1);
    }

    #[test]
    fn ev_do_floor_skips_sector_already_moving() {
        let mut level = level_with_sectors(&[(0, 128)]);
        level.sectors[0].tag = 1;
        let mut thinkers = Thinkers::new();
        let rdata = RData::default();
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let fake_id = thinkers.add_thinker(
            ThinkFn::FloorMove,
            ThinkerData::FloorMove(FloorMove {
                floor_type: FloorType::RaiseFloor,
                crush: false,
                sector: 0,
                direction: 1,
                newspecial: 0,
                texture: 0,
                floordestheight: 0,
                speed: FRACUNIT,
            }),
        );
        level.sectors[0].specialdata = Some(fake_id);

        let rtn = ev_do_floor(
            &mut level,
            &mut thinkers,
            &rdata,
            &line,
            FloorType::RaiseFloor24,
        );

        assert!(!rtn, "already-moving sector should be skipped");
        assert_eq!(level.sectors[0].specialdata, Some(fake_id));
    }
}
